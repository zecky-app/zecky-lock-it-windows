//! A passphrase key, stretched with Argon2id.

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use super::{KeyDescriptor, KeyKind, KeySource};
use crate::{Error, Result, Secret};

/// Shortest passphrase accepted, in characters.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

/// Argon2id cost used for new locks: 256 MiB, 3 passes, 1 lane.
///
/// Stored with the key, so it can be raised later without breaking old locks.
const DEFAULT_MEMORY_KIB: u32 = 256 * 1024;
const DEFAULT_PASSES: u32 = 3;
const DEFAULT_LANES: u32 = 1;

/// Upper bounds accepted when reading a vault file.
const MAX_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_PASSES: u32 = 16;
const MAX_LANES: u32 = 8;

/// A passphrase the owner types.
///
/// The passphrase is held only for the duration of one call and wiped after.
pub struct PassphraseKey {
    passphrase: Zeroizing<String>,
    memory_kib: u32,
    passes: u32,
}

impl PassphraseKey {
    /// Wraps a passphrase the owner just typed.
    pub fn new(passphrase: impl Into<String>) -> Self {
        Self {
            passphrase: Zeroizing::new(passphrase.into()),
            memory_kib: DEFAULT_MEMORY_KIB,
            passes: DEFAULT_PASSES,
        }
    }

    /// Lowers the stretching cost. For tests only: real locks use the default.
    #[doc(hidden)]
    pub fn with_cost(mut self, memory_kib: u32, passes: u32) -> Self {
        self.memory_kib = memory_kib;
        self.passes = passes;
        self
    }

    fn stretch(&self, salt: &[u8; 32], memory_kib: u32, passes: u32, lanes: u32) -> Result<Secret> {
        let params = Params::new(memory_kib, passes, lanes, Some(32))
            .map_err(|e| Error::Format(format!("passphrase cost settings are not valid: {e}")))?;
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut out = Zeroizing::new([0u8; 32]);
        argon
            .hash_password_into(self.passphrase.as_bytes(), salt, out.as_mut())
            .map_err(|e| Error::Platform(format!("passphrase stretching failed: {e}")))?;
        Ok(Secret::new(*out))
    }
}

impl KeySource for PassphraseKey {
    fn kind(&self) -> KeyKind {
        KeyKind::Passphrase
    }

    fn enroll(&mut self, label: &str) -> Result<(KeyDescriptor, Secret)> {
        if self.passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(Error::PassphraseTooShort { min: MIN_PASSPHRASE_CHARS });
        }
        let mut descriptor = KeyDescriptor::new(KeyKind::Passphrase, label)?;
        descriptor.params.insert("kdf".into(), "argon2id-v19".into());
        descriptor.params.insert("memory_kib".into(), self.memory_kib.to_string());
        descriptor.params.insert("passes".into(), self.passes.to_string());
        descriptor.params.insert("lanes".into(), DEFAULT_LANES.to_string());
        let secret = self.stretch(&descriptor.salt_bytes()?, self.memory_kib, self.passes, DEFAULT_LANES)?;
        Ok((descriptor, secret))
    }

    fn unlock(&mut self, descriptor: &KeyDescriptor) -> Result<Secret> {
        if descriptor.param("kdf")? != "argon2id-v19" {
            return Err(Error::Format("this passphrase key uses an unknown stretching method".into()));
        }
        let number = |name: &str| -> Result<u32> {
            descriptor
                .param(name)?
                .parse()
                .map_err(|_| Error::Format(format!("a passphrase key's `{name}` field is not a number")))
        };
        let (memory_kib, passes, lanes) = (number("memory_kib")?, number("passes")?, number("lanes")?);
        // The file is not trusted until the vault opens, so refuse costs that
        // would only serve to stall or exhaust this computer.
        if memory_kib > MAX_MEMORY_KIB || passes > MAX_PASSES || lanes > MAX_LANES {
            return Err(Error::Format("this passphrase key asks for an unreasonable stretching cost".into()));
        }
        self.stretch(&descriptor.salt_bytes()?, memory_kib, passes, lanes)
    }
}
