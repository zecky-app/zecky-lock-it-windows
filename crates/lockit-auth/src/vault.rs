//! The sealed vault: the wallet's recovery phrase, locked with 1 to 3 keys.
//!
//! # How it is locked
//!
//! Every key gives a 32-byte secret. The vault key is derived from *all* of
//! them together, so every key the owner chose is needed to open the lock;
//! there is no "any one of them" mode.
//!
//! ```text
//! vault_key = HKDF-SHA256(
//!     salt = kdf_salt (32 random bytes, in the file),
//!     ikm  = for each key, in kind order:  tag(kind) || secret,
//!     info = "zecky-lockit/vault/v1/key" || vault_id )
//!
//! ciphertext = XChaCha20-Poly1305(vault_key, nonce, payload, aad = header)
//! ```
//!
//! The header (version, vault id, and the public description of every key)
//! is authenticated: removing a key from the file, swapping one, or changing
//! any of its fields makes the vault refuse to open.
//!
//! Changing the set of keys means opening the vault and sealing it again;
//! a fresh vault id, salt and nonce are used every time.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{
    keys::KeyDescriptor,
    secret::random_bytes,
    Error, Result, Secret,
};

/// The vault file format this build writes and reads.
pub const VAULT_VERSION: u32 = 1;

/// The most keys a lock can have.
pub const MAX_KEYS: usize = 3;

const KEY_INFO: &[u8] = b"zecky-lockit/vault/v1/key";

/// A sealed vault, as stored on disk (JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vault {
    /// File format version.
    pub version: u32,
    /// Random id of this sealing, base64 (16 bytes).
    pub vault_id: String,
    /// The keys that are all needed to open it, sorted by kind.
    pub keys: Vec<KeyDescriptor>,
    /// Salt for the vault key derivation, base64 (32 bytes).
    pub kdf_salt: String,
    /// Nonce, base64 (24 bytes).
    pub nonce: String,
    /// Encrypted payload with its authentication tag, base64.
    pub ciphertext: String,
}

/// The part of the file that is authenticated but not encrypted.
#[derive(Serialize)]
struct Header<'a> {
    version: u32,
    vault_id: &'a str,
    keys: &'a [KeyDescriptor],
    kdf_salt: &'a str,
}

impl Vault {
    /// Seals `payload` so that every one of `keys` is needed to open it.
    ///
    /// `keys` are the pairs returned by [`crate::KeySource::enroll`]. Between one and
    /// [`MAX_KEYS`] are accepted, each of a different kind.
    pub fn seal(payload: &[u8], mut keys: Vec<(KeyDescriptor, Secret)>) -> Result<Self> {
        if keys.is_empty() || keys.len() > MAX_KEYS {
            return Err(Error::KeyCount { given: keys.len(), max: MAX_KEYS });
        }
        keys.sort_by_key(|(descriptor, _)| descriptor.kind);
        for pair in keys.windows(2) {
            if pair[0].0.kind == pair[1].0.kind {
                return Err(Error::DuplicateKind(pair[0].0.kind));
            }
        }

        let vault_id = B64.encode(random_bytes::<16>()?);
        let kdf_salt = B64.encode(random_bytes::<32>()?);
        let nonce: [u8; 24] = random_bytes()?;

        let (descriptors, secrets): (Vec<_>, Vec<_>) = keys.into_iter().unzip();
        let header = header_bytes(VAULT_VERSION, &vault_id, &descriptors, &kdf_salt)?;
        let cipher = cipher_for(&descriptors, &secrets, &vault_id, &kdf_salt)?;
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: payload, aad: &header })
            .map_err(|_| Error::Platform("sealing the vault failed".into()))?;

        Ok(Self {
            version: VAULT_VERSION,
            vault_id,
            keys: descriptors,
            kdf_salt,
            nonce: B64.encode(nonce),
            ciphertext: B64.encode(ciphertext),
        })
    }

    /// Opens the vault with one secret per key, in the order of [`Vault::keys`].
    ///
    /// The returned bytes are wiped when dropped.
    pub fn open(&self, secrets: &[Secret]) -> Result<Zeroizing<Vec<u8>>> {
        self.check_shape()?;
        if secrets.len() != self.keys.len() {
            return Err(Error::KeyCount { given: secrets.len(), max: self.keys.len() });
        }
        let nonce: [u8; 24] = B64
            .decode(&self.nonce)
            .ok()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(|| Error::Format("the nonce is not valid".into()))?;
        let ciphertext = B64
            .decode(&self.ciphertext)
            .map_err(|_| Error::Format("the ciphertext is not valid base64".into()))?;

        let header = header_bytes(self.version, &self.vault_id, &self.keys, &self.kdf_salt)?;
        let cipher = cipher_for(&self.keys, secrets, &self.vault_id, &self.kdf_salt)?;
        cipher
            .decrypt(XNonce::from_slice(&nonce), Payload { msg: &ciphertext, aad: &header })
            .map(Zeroizing::new)
            .map_err(|_| Error::WrongKeyOrDamaged)
    }

    /// Opens the vault by asking for each key's secret in turn.
    ///
    /// `ask` is handed each key's description, in the order of
    /// [`Vault::keys`], and returns that key's secret: usually by calling
    /// [`crate::KeySource::unlock`] on the matching source, after showing the owner
    /// which key is next. If any call fails (for example the owner cancels a
    /// prompt), nothing is opened and that error is returned.
    pub fn open_with(
        &self,
        mut ask: impl FnMut(&KeyDescriptor) -> Result<Secret>,
    ) -> Result<Zeroizing<Vec<u8>>> {
        self.check_shape()?;
        let mut secrets = Vec::with_capacity(self.keys.len());
        for descriptor in &self.keys {
            secrets.push(ask(descriptor)?);
        }
        self.open(&secrets)
    }

    /// Reads a vault from its JSON form.
    pub fn from_json(json: &str) -> Result<Self> {
        let vault: Self = serde_json::from_str(json).map_err(|e| Error::Format(e.to_string()))?;
        vault.check_shape()?;
        Ok(vault)
    }

    /// Writes the vault as JSON, ready to be stored.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Format(e.to_string()))
    }

    fn check_shape(&self) -> Result<()> {
        if self.version != VAULT_VERSION {
            return Err(Error::UnsupportedVersion { found: self.version, supported: VAULT_VERSION });
        }
        if self.keys.is_empty() || self.keys.len() > MAX_KEYS {
            return Err(Error::KeyCount { given: self.keys.len(), max: MAX_KEYS });
        }
        for pair in self.keys.windows(2) {
            if pair[0].kind >= pair[1].kind {
                return Err(Error::Format("the vault's keys are not in order, or one kind appears twice".into()));
            }
        }
        Ok(())
    }
}

fn header_bytes(version: u32, vault_id: &str, keys: &[KeyDescriptor], kdf_salt: &str) -> Result<Vec<u8>> {
    // serde_json writes struct fields in declaration order and BTreeMap keys
    // sorted, so the same header always gives the same bytes.
    serde_json::to_vec(&Header { version, vault_id, keys, kdf_salt }).map_err(|e| Error::Format(e.to_string()))
}

fn cipher_for(
    descriptors: &[KeyDescriptor],
    secrets: &[Secret],
    vault_id: &str,
    kdf_salt: &str,
) -> Result<XChaCha20Poly1305> {
    let salt = B64
        .decode(kdf_salt)
        .map_err(|_| Error::Format("the vault's salt is not valid base64".into()))?;
    if salt.len() != 32 {
        return Err(Error::Format("the vault's salt must be 32 bytes".into()));
    }

    let mut ikm = Zeroizing::new(Vec::with_capacity(descriptors.len() * 33));
    for (descriptor, secret) in descriptors.iter().zip(secrets) {
        ikm.push(descriptor.kind.tag());
        ikm.extend_from_slice(secret.expose());
    }

    let mut info = Vec::with_capacity(KEY_INFO.len() + vault_id.len());
    info.extend_from_slice(KEY_INFO);
    info.extend_from_slice(vault_id.as_bytes());

    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(&info, key.as_mut())
        .map_err(|_| Error::Platform("deriving the vault key failed".into()))?;
    Ok(XChaCha20Poly1305::new(key.as_ref().into()))
}
