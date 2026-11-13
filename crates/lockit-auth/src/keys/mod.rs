//! The kinds of key a lock can have.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::{Error, Result, Secret};

pub mod key_check;
pub mod passphrase;

#[cfg(windows)]
pub mod security_key;
#[cfg(windows)]
pub mod windows_hello;

/// The kind of a key. A lock holds at most one key of each kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    /// Windows Hello (face, fingerprint or PIN) with a key held by the TPM.
    DeviceHello,
    /// A FIDO2 security key, for example a YubiKey.
    SecurityKey,
    /// A passphrase the owner remembers.
    Passphrase,
}

impl std::fmt::Display for KeyKind {
    /// The name the owner sees.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeyKind::DeviceHello => "This PC (Windows Hello)",
            KeyKind::SecurityKey => "The security key",
            KeyKind::Passphrase => "The passphrase",
        })
    }
}

impl KeyKind {
    /// Whether this kind gives a secret. A security key is a check only (as
    /// on Lock it Mac), so a lock must also hold one of the others.
    pub fn gives_secret(self) -> bool {
        !matches!(self, KeyKind::SecurityKey)
    }

    /// Stable one-byte tag mixed into the vault key. Never change these.
    pub(crate) fn tag(self) -> u8 {
        match self {
            KeyKind::DeviceHello => 1,
            KeyKind::SecurityKey => 2,
            KeyKind::Passphrase => 3,
        }
    }
}

/// The public description of one enrolled key.
///
/// It is stored in the vault file in the clear and holds no secret: only what
/// is needed to ask the same key the same question again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyDescriptor {
    /// Which kind of key this is.
    pub kind: KeyKind,
    /// The name the owner gave it, for example "Blue YubiKey".
    pub label: String,
    /// Random 32 bytes the key mixes in, base64. Unique to this enrolment.
    pub salt: String,
    /// Extra public facts the key source needs (credential id, key name,
    /// passphrase-stretching cost). Sorted, so the file is stable.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl KeyDescriptor {
    /// Creates a descriptor with a fresh random salt.
    pub fn new(kind: KeyKind, label: &str) -> Result<Self> {
        let salt: [u8; 32] = crate::secret::random_bytes()?;
        Ok(Self {
            kind,
            label: label.to_owned(),
            salt: B64.encode(salt),
            params: BTreeMap::new(),
        })
    }

    /// Decodes the salt.
    pub fn salt_bytes(&self) -> Result<[u8; 32]> {
        let raw = B64
            .decode(&self.salt)
            .map_err(|_| Error::Format("a key's salt is not valid base64".into()))?;
        raw.try_into()
            .map_err(|_| Error::Format("a key's salt must be 32 bytes".into()))
    }

    /// Reads a required parameter.
    pub fn param(&self, name: &str) -> Result<&str> {
        self.params
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| Error::Format(format!("a {:?} key is missing its `{name}` field", self.kind)))
    }

    /// Reads a required base64 parameter.
    pub fn param_bytes(&self, name: &str) -> Result<Vec<u8>> {
        B64.decode(self.param(name)?)
            .map_err(|_| Error::Format(format!("a key's `{name}` field is not valid base64")))
    }

    /// Stores a base64 parameter.
    pub fn set_param_bytes(&mut self, name: &str, value: &[u8]) {
        self.params.insert(name.to_owned(), B64.encode(value));
    }
}

/// Something that can turn the owner's presence into a 32-byte secret.
///
/// Both calls may show a system prompt (Windows Hello, "touch your security
/// key") and block until the owner answers.
pub trait KeySource {
    /// The kind of key this source handles.
    fn kind(&self) -> KeyKind;

    /// Sets the key up for a new lock and returns its public description
    /// together with the secret it will give from now on.
    ///
    /// A key that answers differently each time would seal a vault nobody
    /// can open. Where the platform does not promise a repeatable answer
    /// (Windows Hello), the implementation asks twice and compares before
    /// returning. The app should still let the owner try every key once
    /// right after sealing, before any funds arrive.
    fn enroll(&mut self, label: &str) -> Result<(KeyDescriptor, Secret)>;

    /// Asks an enrolled key for its secret again.
    fn unlock(&mut self, descriptor: &KeyDescriptor) -> Result<Secret>;
}
