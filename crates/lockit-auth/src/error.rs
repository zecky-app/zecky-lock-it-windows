//! Errors for the lock.

use crate::keys::KeyKind;

/// Result type used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while enrolling a key or opening the vault.
///
/// Messages are written to be shown to the owner as they are: calm, and
/// without secrets in them.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A vault needs at least one key and at most three.
    #[error("a lock needs between 1 and {max} keys, but {given} were given")]
    KeyCount {
        /// How many keys were supplied.
        given: usize,
        /// The most keys a lock can have.
        max: usize,
    },

    /// Only security keys were chosen; they check, but give no secret.
    #[error("a security key or a Zecky code adds a check, but a lock also needs Windows Hello or a passphrase")]
    NeedsSecretKey,

    /// A Zecky code is six digits.
    #[error("a Zecky code is six digits")]
    CodeNotSixDigits,

    /// The same kind of key was added twice.
    #[error("each key of a lock must be of a different kind; {0} was chosen twice")]
    DuplicateKind(KeyKind),

    /// The keys did not open the vault: a wrong key, or a damaged file.
    #[error("the lock did not open: a key was wrong, or the vault file was changed")]
    WrongKeyOrDamaged,

    /// The vault file could not be read.
    #[error("the vault file is not valid: {0}")]
    Format(String),

    /// The vault was written by a newer version of Lock it.
    #[error("this vault uses format version {found}; this app understands version {supported}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
        /// Version this build can read.
        supported: u32,
    },

    /// The owner dismissed the prompt.
    #[error("the request was cancelled")]
    Cancelled,

    /// This kind of key cannot be used on this computer.
    #[error("{kind} is not available on this computer: {reason}")]
    Unavailable {
        /// Which kind of key.
        kind: KeyKind,
        /// Plain-language reason.
        reason: String,
    },

    /// The key answered, but not the way a usable key must.
    #[error("{kind} cannot be used for a lock: {reason}")]
    KeyUnusable {
        /// Which kind of key.
        kind: KeyKind,
        /// Plain-language reason.
        reason: String,
    },

    /// The passphrase is too short to protect anything.
    #[error("the passphrase must be at least {min} characters long")]
    PassphraseTooShort {
        /// Minimum length in characters.
        min: usize,
    },

    /// The operating system reported a failure.
    #[error("the system reported a problem: {0}")]
    Platform(String),

    /// The system's random number generator failed.
    #[error("the system could not provide random numbers")]
    Random,
}
