//! Windows Hello key: face, fingerprint or PIN, with a key held by the TPM.
//!
//! This is the Windows counterpart of the Mac's Secure Enclave key.
//!
//! # How it works
//!
//! Windows Hello gives each app named RSA-2048 keys that live in the PC's TPM
//! and can only sign after the owner passes a Hello prompt. There is no
//! "decrypt" or "derive" operation on the classic API, so the secret is made
//! from a signature:
//!
//! ```text
//! signature = Hello key signs ( "zecky-lockit/hello/v1/challenge" || salt )
//! secret    = HKDF-SHA256( salt = salt, ikm = signature,
//!                          info = "zecky-lockit/hello/v1/secret" )
//! ```
//!
//! RSA PKCS#1 v1.5 signatures are deterministic, so the same key and salt
//! always give the same secret. That property is what the lock depends on, so
//! [`enroll`](KeySource::enroll) signs twice and refuses a key that answers
//! differently the second time.
//!
//! # What this does and does not protect
//!
//! * The private key never leaves the TPM and cannot be copied to another PC.
//! * Every use shows a Hello prompt. Without biometric hardware that prompt
//!   is the Hello PIN.
//! * For classic desktop apps Windows does not tie the key to *this* program:
//!   another program running as the same user could ask the same key to sign,
//!   and the owner would see a Hello prompt they did not expect. Packaging the
//!   app (MSIX) gives per-app separation.
//!
//! # Status
//!
//! Compiles for Windows. **Not yet run on a real Windows PC.**

use hkdf::Hkdf;
use sha2::Sha256;
use windows::{
    core::HSTRING,
    Security::{
        Credentials::{
            KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
        },
        Cryptography::CryptographicBuffer,
    },
};
use zeroize::Zeroizing;

use super::{KeyDescriptor, KeyKind, KeySource};
use crate::{secret::random_bytes, Error, Result, Secret};

const CHALLENGE_PREFIX: &[u8] = b"zecky-lockit/hello/v1/challenge";
const SECRET_INFO: &[u8] = b"zecky-lockit/hello/v1/secret";
const KEY_NAME_PARAM: &str = "hello_key_name";

/// The Windows Hello key source.
#[derive(Default)]
pub struct WindowsHelloKey;

impl WindowsHelloKey {
    /// Creates the source. Nothing is asked of the owner until it is used.
    pub fn new() -> Self {
        Self
    }

    /// Whether Windows Hello is set up for this user on this PC.
    pub fn is_available() -> bool {
        KeyCredentialManager::IsSupportedAsync()
            .and_then(|operation| operation.get())
            .unwrap_or(false)
    }

    /// Removes the TPM key of an enrolment that is no longer used.
    ///
    /// Call this after the vault has been sealed again without this key.
    pub fn forget(descriptor: &KeyDescriptor) -> Result<()> {
        let name = HSTRING::from(descriptor.param(KEY_NAME_PARAM)?);
        KeyCredentialManager::DeleteAsync(&name)
            .and_then(|action| action.get())
            .map_err(platform)
    }
}

impl KeySource for WindowsHelloKey {
    fn kind(&self) -> KeyKind {
        KeyKind::DeviceHello
    }

    fn enroll(&mut self, label: &str) -> Result<(KeyDescriptor, Secret)> {
        if !Self::is_available() {
            return Err(Error::Unavailable {
                kind: KeyKind::DeviceHello,
                reason: "Windows Hello is not set up. Add a PIN, fingerprint or face in Windows Settings first.".into(),
            });
        }

        let mut descriptor = KeyDescriptor::new(KeyKind::DeviceHello, label)?;
        let name = format!("zecky-lockit-{}", hex(&random_bytes::<8>()?));
        descriptor.params.insert(KEY_NAME_PARAM.into(), name.clone());

        let created = KeyCredentialManager::RequestCreateAsync(
            &HSTRING::from(&name),
            KeyCredentialCreationOption::FailIfExists,
        )
        .and_then(|operation| operation.get())
        .map_err(platform)?;
        check_status(created.Status().map_err(platform)?)?;
        let credential = created.Credential().map_err(platform)?;

        let salt = descriptor.salt_bytes()?;
        let first = sign(&credential, &salt)?;
        let second = sign(&credential, &salt)?;
        if first.as_slice() != second.as_slice() {
            // Leave nothing behind for a key that cannot be used.
            let _ = Self::forget(&descriptor);
            return Err(Error::KeyUnusable {
                kind: KeyKind::DeviceHello,
                reason: "it gave two different answers to the same question".into(),
            });
        }
        Ok((descriptor, secret_from(&first, &salt)?))
    }

    fn unlock(&mut self, descriptor: &KeyDescriptor) -> Result<Secret> {
        let name = HSTRING::from(descriptor.param(KEY_NAME_PARAM)?);
        let opened = KeyCredentialManager::OpenAsync(&name)
            .and_then(|operation| operation.get())
            .map_err(platform)?;
        check_status(opened.Status().map_err(platform)?)?;
        let credential = opened.Credential().map_err(platform)?;

        let salt = descriptor.salt_bytes()?;
        let signature = sign(&credential, &salt)?;
        secret_from(&signature, &salt)
    }
}

/// Asks the Hello key to sign the challenge for `salt`. Shows a Hello prompt.
fn sign(credential: &KeyCredential, salt: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>> {
    let mut challenge = Vec::with_capacity(CHALLENGE_PREFIX.len() + salt.len());
    challenge.extend_from_slice(CHALLENGE_PREFIX);
    challenge.extend_from_slice(salt);

    let buffer = CryptographicBuffer::CreateFromByteArray(&challenge).map_err(platform)?;
    let signed = credential
        .RequestSignAsync(&buffer)
        .and_then(|operation| operation.get())
        .map_err(platform)?;
    check_status(signed.Status().map_err(platform)?)?;

    let result = signed.Result().map_err(platform)?;
    let mut bytes = windows::core::Array::<u8>::new();
    CryptographicBuffer::CopyToByteArray(&result, &mut bytes).map_err(platform)?;
    if bytes.is_empty() {
        return Err(Error::KeyUnusable {
            kind: KeyKind::DeviceHello,
            reason: "it returned an empty signature".into(),
        });
    }
    Ok(Zeroizing::new(bytes.to_vec()))
}

fn secret_from(signature: &[u8], salt: &[u8; 32]) -> Result<Secret> {
    let mut out = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), signature)
        .expand(SECRET_INFO, out.as_mut())
        .map_err(|_| Error::Platform("deriving the Hello secret failed".into()))?;
    Ok(Secret::new(*out))
}

fn check_status(status: KeyCredentialStatus) -> Result<()> {
    match status {
        KeyCredentialStatus::Success => Ok(()),
        KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => Err(Error::Cancelled),
        KeyCredentialStatus::NotFound => Err(Error::Unavailable {
            kind: KeyKind::DeviceHello,
            reason: "this PC no longer has the Hello key of this lock. If Windows Hello was reset, the lock can only be restored from the recovery phrase.".into(),
        }),
        KeyCredentialStatus::SecurityDeviceLocked => Err(Error::Unavailable {
            kind: KeyKind::DeviceHello,
            reason: "the PC's security chip is locked after too many attempts. Wait, then try again.".into(),
        }),
        KeyCredentialStatus::CredentialAlreadyExists => {
            Err(Error::Platform("a Hello key with this name already exists".into()))
        }
        other => Err(Error::Platform(format!("Windows Hello reported status {}", other.0))),
    }
}

fn platform(error: windows::core::Error) -> Error {
    Error::Platform(error.message())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
