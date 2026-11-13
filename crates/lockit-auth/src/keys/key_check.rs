//! Checking a security key's answer, as Lock it Mac does (`SecurityKeys.swift`).
//!
//! A security key is a *check*, not a source of secret. When it is paired,
//! its public key is saved. To open the lock, the key signs a fresh random
//! challenge, and the signature is checked here against the saved public key.
//! A copy of the lock file without the key in hand cannot pass the check, and
//! the key cannot be removed from the file without breaking the vault, because
//! the vault's header (which lists it) is authenticated.
//!
//! This works with every version of Windows' security-key interface, so on
//! Windows 10 too. A lock with a security key must also hold a key that gives
//! a secret (Windows Hello or a passphrase).
//!
//! Everything in this file is plain parsing and arithmetic, so it is tested on
//! any system.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use p256::ecdsa::{signature::Verifier as _, Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{KeyDescriptor, KeyKind};
use crate::{Error, Result, Secret};

/// The relying party keys are registered under: the same as Lock it Mac.
pub const RELYING_PARTY: &str = "zecky.app";

const USER_PRESENT: u8 = 0x01;
const ATTESTED: u8 = 0x40;

fn unusable(reason: impl Into<String>) -> Error {
    Error::KeyUnusable { kind: KeyKind::SecurityKey, reason: reason.into() }
}

/// The fixed head of the authenticator data both answers start with.
struct AuthData<'a> {
    rp_id_hash: &'a [u8],
    flags: u8,
    rest: &'a [u8],
}

fn parse_auth_data(data: &[u8]) -> Result<AuthData<'_>> {
    if data.len() < 37 {
        return Err(unusable("its answer is too short"));
    }
    Ok(AuthData { rp_id_hash: &data[..32], flags: data[32], rest: &data[37..] })
}

fn check_rp(auth: &AuthData<'_>) -> Result<()> {
    if auth.rp_id_hash != Sha256::digest(RELYING_PARTY.as_bytes()).as_slice() {
        return Err(unusable("its answer is for another app or site"));
    }
    if auth.flags & USER_PRESENT == 0 {
        return Err(unusable("the key was not touched"));
    }
    Ok(())
}

/// What is saved when a key is paired: its credential id and public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedKey {
    /// The credential id the key gave.
    pub credential_id: Vec<u8>,
    /// The P-256 public key, SEC1 uncompressed (65 bytes).
    pub public_key: Vec<u8>,
}

impl PairedKey {
    /// The constant the vault mixes in for this key. It is public (derived
    /// from the public key), so it adds no secrecy; it only ties the key into
    /// the vault the same way as every other kind.
    pub fn vault_secret(&self) -> Secret {
        let mut hash = Sha256::new();
        hash.update(b"zecky-lockit/check-key/v1");
        hash.update(&self.public_key);
        Secret::new(hash.finalize().into())
    }
}

// ---------- The list of paired keys, kept in the lock's descriptor ----------
//
// One security-key entry in the lock holds every paired key (a main key and
// backups, as on Lock it Mac); any one of them passes. The list sits in the
// lock's authenticated header, so nobody can add a key of their own without
// opening the lock. Test builds 2 to 5 stored a single key in
// `credential_id` / `public_key`; such a lock keeps working and moves to the
// list the first time a key is added or removed.

const KEYS_PARAM: &str = "keys";
const LEGACY_ID: &str = "credential_id";
const LEGACY_PUBLIC_KEY: &str = "public_key";

/// A key's role, as on Lock it Mac.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The key used day to day.
    Main,
    /// Kept somewhere else, in case the main key is lost.
    Backup,
}

/// One paired key in the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEntry {
    /// Credential id, base64.
    pub id: String,
    /// P-256 public key (SEC1, uncompressed), base64.
    pub public_key: String,
    /// Main or backup.
    pub role: Role,
    /// When it was paired (Unix seconds; 0 for keys from test builds).
    pub added: i64,
}

impl KeyEntry {
    /// A new entry for a key just paired.
    pub fn new(paired: &PairedKey, role: Role, added: i64) -> Self {
        Self { id: B64.encode(&paired.credential_id), public_key: B64.encode(&paired.public_key), role, added }
    }

    /// Back to the form the check uses.
    pub fn paired(&self) -> Result<PairedKey> {
        let bad = || Error::Format("a saved security key is damaged".into());
        Ok(PairedKey {
            credential_id: B64.decode(&self.id).map_err(|_| bad())?,
            public_key: B64.decode(&self.public_key).map_err(|_| bad())?,
        })
    }
}

fn is_legacy(descriptor: &KeyDescriptor) -> bool {
    !descriptor.params.contains_key(KEYS_PARAM) && descriptor.params.contains_key(LEGACY_PUBLIC_KEY)
}

/// Every key paired in this lock's security-key entry.
pub fn keys_of(descriptor: &KeyDescriptor) -> Result<Vec<KeyEntry>> {
    if is_legacy(descriptor) {
        let paired = PairedKey {
            credential_id: descriptor.param_bytes(LEGACY_ID)?,
            public_key: descriptor.param_bytes(LEGACY_PUBLIC_KEY)?,
        };
        return Ok(vec![KeyEntry::new(&paired, Role::Main, 0)]);
    }
    serde_json::from_str(descriptor.param(KEYS_PARAM)?).map_err(|_| Error::Format("the security key list is damaged".into()))
}

/// Writes the list into the entry (moving a test-build entry to the list).
pub fn set_keys(descriptor: &mut KeyDescriptor, keys: &[KeyEntry]) -> Result<()> {
    descriptor.params.remove(LEGACY_ID);
    descriptor.params.remove(LEGACY_PUBLIC_KEY);
    let json = serde_json::to_string(keys).map_err(|e| Error::Format(e.to_string()))?;
    descriptor.params.insert(KEYS_PARAM.into(), json);
    Ok(())
}

/// The constant this entry mixes into the lock. Public: a security key is a
/// check, not a secret. That is also what lets "Lost my keys" take the keys
/// off after 48 hours without one of them.
pub fn vault_secret(descriptor: &KeyDescriptor) -> Result<Secret> {
    if is_legacy(descriptor) {
        let legacy = PairedKey { credential_id: vec![], public_key: descriptor.param_bytes(LEGACY_PUBLIC_KEY)? };
        return Ok(legacy.vault_secret());
    }
    Ok(Secret::new(Sha256::digest(b"zecky-lockit/check-keys/v2").into()))
}

/// Checks an answer against the whole list; returns the key that answered.
pub fn check_any(
    keys: &[KeyEntry],
    credential_id: &[u8],
    auth_data: &[u8],
    client_data_json: &[u8],
    signature_der: &[u8],
) -> Result<KeyEntry> {
    for entry in keys {
        let paired = entry.paired()?;
        if paired.credential_id == credential_id {
            check_assertion(&paired, credential_id, auth_data, client_data_json, signature_der)?;
            return Ok(entry.clone());
        }
    }
    Err(Error::WrongKeyOrDamaged)
}

/// Reads the pairing answer: the authenticator data from "make credential".
pub fn read_registration(auth_data: &[u8], credential_id: &[u8]) -> Result<PairedKey> {
    let auth = parse_auth_data(auth_data)?;
    check_rp(&auth)?;
    if auth.flags & ATTESTED == 0 {
        return Err(unusable("its answer holds no new key"));
    }
    // aaguid (16), credential id length (2), credential id, COSE public key.
    let rest = auth.rest;
    if rest.len() < 18 {
        return Err(unusable("its answer ends early"));
    }
    let id_len = u16::from_be_bytes([rest[16], rest[17]]) as usize;
    let id = rest.get(18..18 + id_len).ok_or_else(|| unusable("its answer ends early"))?;
    if id != credential_id {
        return Err(unusable("its answer does not match its credential"));
    }
    let (cose, _) = cbor::read(&rest[18 + id_len..], 0)?;
    let get = |key: i64| cose.get_int_key(key);
    let ok = matches!(get(1), Some(cbor::Value::Int(2)))
        && matches!(get(3), Some(cbor::Value::Int(-7)))
        && matches!(get(-1), Some(cbor::Value::Int(1)));
    let (Some(cbor::Value::Bytes(x)), Some(cbor::Value::Bytes(y))) = (get(-2), get(-3)) else {
        return Err(unusable("it did not give a P-256 key"));
    };
    if !ok || x.len() != 32 || y.len() != 32 {
        return Err(unusable("it did not give a P-256 key"));
    }
    let mut public_key = Vec::with_capacity(65);
    public_key.push(0x04);
    public_key.extend_from_slice(x);
    public_key.extend_from_slice(y);
    VerifyingKey::from_sec1_bytes(&public_key).map_err(|_| unusable("its public key is not valid"))?;
    Ok(PairedKey { credential_id: credential_id.to_vec(), public_key })
}

/// Checks an answer to "sign this": the right key, touched, for this app,
/// and a valid signature over `auth_data || SHA-256(client_data_json)`.
///
/// `client_data_json` is the exact JSON Lock it handed the key, which holds
/// the fresh challenge; the signature covers its hash, so an old answer
/// cannot be replayed.
pub fn check_assertion(
    paired: &PairedKey,
    credential_id: &[u8],
    auth_data: &[u8],
    client_data_json: &[u8],
    signature_der: &[u8],
) -> Result<()> {
    if credential_id != paired.credential_id {
        return Err(Error::WrongKeyOrDamaged);
    }
    let auth = parse_auth_data(auth_data)?;
    check_rp(&auth)?;
    let key = VerifyingKey::from_sec1_bytes(&paired.public_key).map_err(|_| Error::Format("a saved security key is damaged".into()))?;
    let signature = Signature::from_der(signature_der).map_err(|_| unusable("its signature is not readable"))?;
    let mut signed = auth_data.to_vec();
    signed.extend_from_slice(&Sha256::digest(client_data_json));
    key.verify(&signed, &signature).map_err(|_| Error::WrongKeyOrDamaged)
}

/// A tiny CBOR reader: just what a COSE public key uses.
mod cbor {
    use super::{unusable, Result};

    #[derive(Debug)]
    pub enum Value<'a> {
        Int(i64),
        Bytes(&'a [u8]),
        Text,
        Map(Vec<(Value<'a>, Value<'a>)>),
        Other,
    }

    impl Value<'_> {
        pub fn get_int_key(&self, key: i64) -> Option<&Value<'_>> {
            match self {
                Value::Map(pairs) => pairs.iter().find(|(k, _)| matches!(k, Value::Int(n) if *n == key)).map(|(_, v)| v),
                _ => None,
            }
        }
    }

    pub fn read(bytes: &[u8], depth: u8) -> Result<(Value<'_>, usize)> {
        if depth > 4 {
            return Err(unusable("its key is nested too deep"));
        }
        let head = *bytes.first().ok_or_else(|| unusable("its key ends early"))?;
        let (major, info) = (head >> 5, head & 0x1f);
        let (arg, mut pos) = match info {
            0..=23 => (info as u64, 1),
            24 => (*bytes.get(1).ok_or_else(|| unusable("its key ends early"))? as u64, 2),
            25 => {
                let b = bytes.get(1..3).ok_or_else(|| unusable("its key ends early"))?;
                (u16::from_be_bytes([b[0], b[1]]) as u64, 3)
            }
            _ => return Err(unusable("its key uses an unsupported form")),
        };
        let value = match major {
            0 => Value::Int(arg as i64),
            1 => Value::Int(-1 - arg as i64),
            2 | 3 => {
                let data = bytes.get(pos..pos + arg as usize).ok_or_else(|| unusable("its key ends early"))?;
                pos += arg as usize;
                if major == 2 { Value::Bytes(data) } else { Value::Text }
            }
            5 => {
                let mut pairs = Vec::new();
                for _ in 0..arg {
                    let (k, used) = read(&bytes[pos..], depth + 1)?;
                    pos += used;
                    let (v, used) = read(&bytes[pos..], depth + 1)?;
                    pos += used;
                    pairs.push((k, v));
                }
                Value::Map(pairs)
            }
            _ => Value::Other,
        };
        Ok((value, pos))
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::{signature::Signer as _, SigningKey};

    use super::*;

    /// Builds the answers a real key gives, from a software key.
    fn auth_head(flags: u8) -> Vec<u8> {
        let mut data = Sha256::digest(RELYING_PARTY.as_bytes()).to_vec();
        data.push(flags);
        data.extend_from_slice(&7u32.to_be_bytes());
        data
    }

    fn registration(key: &SigningKey, id: &[u8]) -> Vec<u8> {
        let point = key.verifying_key().to_encoded_point(false);
        let mut data = auth_head(USER_PRESENT | ATTESTED);
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&(id.len() as u16).to_be_bytes());
        data.extend_from_slice(id);
        // {1: 2, 3: -7, -1: 1, -2: x, -3: y}
        data.extend_from_slice(&[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20]);
        data.extend_from_slice(point.x().unwrap());
        data.extend_from_slice(&[0x22, 0x58, 0x20]);
        data.extend_from_slice(point.y().unwrap());
        data
    }

    fn answer(key: &SigningKey, flags: u8, client: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let auth = auth_head(flags);
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(client));
        let signature: Signature = key.sign(&signed);
        (auth, signature.to_der().as_bytes().to_vec())
    }

    #[test]
    fn a_paired_key_passes_and_everything_else_fails() {
        let key = SigningKey::from_slice(&[7; 32]).unwrap();
        let id = b"credential-1";
        let paired = read_registration(&registration(&key, id), id).unwrap();
        assert_eq!(paired.public_key.len(), 65);

        let client = br#"{"type":"webauthn.get","challenge":"abc"}"#;
        let (auth, sig) = answer(&key, USER_PRESENT, client);
        check_assertion(&paired, id, &auth, client, &sig).unwrap();

        // Another key, an old answer for another challenge, an untouched key.
        let other = SigningKey::from_slice(&[9; 32]).unwrap();
        let (auth2, sig2) = answer(&other, USER_PRESENT, client);
        assert!(check_assertion(&paired, id, &auth2, client, &sig2).is_err());
        assert!(check_assertion(&paired, id, &auth, br#"{"challenge":"old"}"#, &sig).is_err());
        let (auth3, sig3) = answer(&key, 0, client);
        assert!(check_assertion(&paired, id, &auth3, client, &sig3).is_err());
        assert!(check_assertion(&paired, b"another-id", &auth, client, &sig).is_err());
    }

    #[test]
    fn any_key_in_the_list_passes_and_old_single_keys_still_read() {
        let main = SigningKey::from_slice(&[7; 32]).unwrap();
        let backup = SigningKey::from_slice(&[8; 32]).unwrap();
        let p_main = read_registration(&registration(&main, b"main"), b"main").unwrap();
        let p_backup = read_registration(&registration(&backup, b"backup"), b"backup").unwrap();

        let mut d = KeyDescriptor::new(KeyKind::SecurityKey, "Security key").unwrap();
        d.set_param_bytes(LEGACY_ID, b"main");
        d.set_param_bytes(LEGACY_PUBLIC_KEY, &p_main.public_key);
        let legacy = keys_of(&d).unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(vault_secret(&d).unwrap().expose(), p_main.vault_secret().expose());

        let list = vec![legacy[0].clone(), KeyEntry::new(&p_backup, Role::Backup, 1)];
        set_keys(&mut d, &list).unwrap();
        assert!(!d.params.contains_key(LEGACY_ID));
        assert_eq!(keys_of(&d).unwrap(), list);
        assert_ne!(vault_secret(&d).unwrap().expose(), p_main.vault_secret().expose());

        let client = br#"{"challenge":"x"}"#;
        let (auth, sig) = answer(&backup, USER_PRESENT, client);
        assert_eq!(check_any(&list, b"backup", &auth, client, &sig).unwrap().role, Role::Backup);
        assert!(check_any(&list, b"main", &auth, client, &sig).is_err(), "the backup's answer is not the main key's");
        assert!(check_any(&list[..1], b"backup", &auth, client, &sig).is_err(), "a removed key no longer passes");
    }

    #[test]
    fn a_registration_for_another_site_is_refused() {
        let key = SigningKey::from_slice(&[7; 32]).unwrap();
        let mut data = registration(&key, b"id");
        data[0] ^= 1;
        assert!(read_registration(&data, b"id").is_err());
        assert!(read_registration(&registration(&key, b"id"), b"other").is_err());
    }
}
