//! Zecky Lock it: the keys that open the lock.
//!
//! This crate holds everything that decides *who may release funds*:
//!
//! * [`Vault`]: the sealed file that holds the wallet's recovery phrase. It can
//!   only be opened with every key the owner chose (one, two or three).
//! * [`KeySource`]: something that can produce a 32-byte secret after the owner
//!   proves presence. Three kinds exist:
//!   * [`KeyKind::DeviceHello`]: Windows Hello, backed by the PC's TPM
//!     (the Windows counterpart of the Mac's Secure Enclave key);
//!   * [`KeyKind::SecurityKey`]: a FIDO2 security key such as a YubiKey
//!     (hmac-secret extension, through the Windows WebAuthn API);
//!   * [`KeyKind::Passphrase`]: a passphrase stretched with Argon2id.
//!
//! Viewing balances does not need this crate at all: the lock is needed only
//! to release. Nothing here talks to the network.
//!
//! # Status
//!
//! The vault, the passphrase key and the policy code are plain Rust and are
//! covered by tests that run on any platform. The two Windows key sources
//! compile for Windows but have **not yet been run on a real Windows PC**.
//! Do not protect real funds with them until they have been tested on
//! hardware and reviewed.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod error;
pub mod keys;
pub mod protect;
mod secret;
mod vault;

pub use error::{Error, Result};
pub use keys::{KeyDescriptor, KeyKind, KeySource};
pub use secret::Secret;
pub use vault::{Contact, Policy, Vault, MAX_KEYS, VAULT_VERSION};
