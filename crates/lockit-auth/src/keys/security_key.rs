//! A FIDO2 security key (for example a YubiKey) as a key of the lock.
//!
//! # How it works
//!
//! Windows does not let a normal program talk to a security key directly;
//! everything goes through the system's WebAuthn API (`webauthn.dll`), which
//! shows the familiar "touch your security key" window.
//!
//! * **Setting up** creates a credential on the key with the `hmac-secret`
//!   extension switched on. The credential is not "discoverable": it uses no
//!   storage slot on the key, and the credential id is kept in the vault file.
//! * **Opening** asks the key for an assertion and hands it this key's salt.
//!   The key answers with a 32-byte value that only this physical key can
//!   compute for this credential and salt. That value is the secret.
//!
//! The salt is passed in the API's default (WebAuthn "PRF") form, the same
//! form browsers use, so Windows hashes it with a fixed prefix before it
//! reaches the key.
//!
//! # The key's PIN is always required
//!
//! A security key gives a *different* answer depending on whether the owner
//! was verified with the key's PIN (or fingerprint). To keep the answer the
//! same for the life of the lock, and so that a stolen key alone opens
//! nothing, verification is always required. Windows asks the owner to set a
//! PIN on a key that has none.
//!
//! # Status
//!
//! Compiles for Windows. **Not yet run on a real Windows PC or tested with a
//! real key.**

use std::{ffi::c_void, sync::OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use windows::{
    core::{w, BOOL, HRESULT, PCWSTR},
    Win32::{
        Foundation::{HMODULE, HWND},
        Networking::WindowsWebServices::*,
        System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
        UI::WindowsAndMessaging::GetForegroundWindow,
    },
};
use zeroize::Zeroizing;

use super::{KeyDescriptor, KeyKind, KeySource};
use crate::{secret::random_bytes, Error, Result, Secret};

/// Relying-party id of the lock's credentials. Part of the format: changing
/// it would make every enrolled security key stop opening its lock.
const RP_ID: &str = "lockit.zecky.app";
const RP_ID_W: PCWSTR = w!("lockit.zecky.app");
const RP_NAME_W: PCWSTR = w!("Zecky Lock it");
const ORIGIN: &str = "https://lockit.zecky.app";
const USER_NAME_W: PCWSTR = w!("Lock it");

const CREDENTIAL_ID_PARAM: &str = "credential_id";
const RP_ID_PARAM: &str = "rp_id";

/// How long the "touch your key" window waits, in milliseconds.
const TIMEOUT_MS: u32 = 120_000;

/// `NTE_USER_CANCELLED`: the owner closed the system window.
const NTE_USER_CANCELLED: HRESULT = HRESULT(0x8009_0036_u32 as i32);

/// The parts of `webauthn.dll` this file uses, loaded when first needed.
///
/// Loaded at run time, not linked at start-up: on a Windows without
/// `webauthn.dll` the app must still start, and simply report that security
/// keys are not available here.
struct WebAuthnApi {
    version: unsafe extern "system" fn() -> u32,
    make_credential: unsafe extern "system" fn(
        HWND,
        *const WEBAUTHN_RP_ENTITY_INFORMATION,
        *const WEBAUTHN_USER_ENTITY_INFORMATION,
        *const WEBAUTHN_COSE_CREDENTIAL_PARAMETERS,
        *const WEBAUTHN_CLIENT_DATA,
        *const WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS,
        *mut *mut WEBAUTHN_CREDENTIAL_ATTESTATION,
    ) -> HRESULT,
    get_assertion: unsafe extern "system" fn(
        HWND,
        PCWSTR,
        *const WEBAUTHN_CLIENT_DATA,
        *const WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
        *mut *mut WEBAUTHN_ASSERTION,
    ) -> HRESULT,
    free_attestation: unsafe extern "system" fn(*const WEBAUTHN_CREDENTIAL_ATTESTATION),
    free_assertion: unsafe extern "system" fn(*const WEBAUTHN_ASSERTION),
    error_name: unsafe extern "system" fn(HRESULT) -> PCWSTR,
}

impl WebAuthnApi {
    /// The loaded API, or `None` if this Windows has no usable `webauthn.dll`.
    fn get() -> Option<&'static Self> {
        static API: OnceLock<Option<WebAuthnApi>> = OnceLock::new();
        API.get_or_init(Self::load).as_ref()
    }

    fn load() -> Option<Self> {
        // SAFETY: loads a system DLL from System32 only, and looks up
        // functions whose signatures are fixed by webauthn.h.
        unsafe {
            let module: HMODULE =
                LoadLibraryExW(w!("webauthn.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
            macro_rules! function {
                ($name:literal) => {
                    std::mem::transmute(GetProcAddress(module, windows::core::s!($name))?)
                };
            }
            Some(Self {
                version: function!("WebAuthNGetApiVersionNumber"),
                make_credential: function!("WebAuthNAuthenticatorMakeCredential"),
                get_assertion: function!("WebAuthNAuthenticatorGetAssertion"),
                free_attestation: function!("WebAuthNFreeCredentialAttestation"),
                free_assertion: function!("WebAuthNFreeAssertion"),
                error_name: function!("WebAuthNGetErrorName"),
            })
        }
    }
}

/// The security key source.
pub struct SecurityKey {
    window: HWND,
}

impl SecurityKey {
    /// Creates the source. The system's window is shown above whichever
    /// window is in front when the key is asked.
    pub fn new() -> Self {
        Self { window: HWND::default() }
    }

    /// Creates the source with the app window the system's window should
    /// appear above (a raw `HWND`).
    pub fn for_window(raw_hwnd: isize) -> Self {
        Self { window: HWND(raw_hwnd as *mut c_void) }
    }

    /// Whether this Windows can ask a security key for an `hmac-secret`.
    pub fn is_available() -> bool {
        // SAFETY: takes no arguments and only reads a version number.
        WebAuthnApi::get().is_some_and(|api| unsafe { (api.version)() } >= WEBAUTHN_API_VERSION_4)
    }

    fn api() -> Result<&'static WebAuthnApi> {
        WebAuthnApi::get().ok_or_else(|| Error::Unavailable {
            kind: KeyKind::SecurityKey,
            reason: "this Windows has no security-key support (webauthn.dll is missing)".into(),
        })
    }

    fn window(&self) -> HWND {
        if self.window.is_invalid() {
            // SAFETY: no arguments; returns a handle or null.
            unsafe { GetForegroundWindow() }
        } else {
            self.window
        }
    }

    fn require_available() -> Result<()> {
        Self::api()?;
        if Self::is_available() {
            Ok(())
        } else {
            Err(Error::Unavailable {
                kind: KeyKind::SecurityKey,
                reason: "this version of Windows is too old to use a security key for a lock. Install the latest Windows updates.".into(),
            })
        }
    }

    /// Creates the credential on the key. Returns its credential id.
    fn make_credential(&self) -> Result<Vec<u8>> {
        let mut user_id: [u8; 16] = random_bytes()?;
        let mut client_data = client_data_json("webauthn.create")?;

        let rp = WEBAUTHN_RP_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
            pwszId: RP_ID_W,
            pwszName: RP_NAME_W,
            pwszIcon: PCWSTR::null(),
        };
        let user = WEBAUTHN_USER_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
            cbId: user_id.len() as u32,
            pbId: user_id.as_mut_ptr(),
            pwszName: USER_NAME_W,
            pwszIcon: PCWSTR::null(),
            pwszDisplayName: USER_NAME_W,
        };
        let mut algorithm = WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: WEBAUTHN_COSE_ALGORITHM_ECDSA_P256_WITH_SHA256,
        };
        let algorithms = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
            cCredentialParameters: 1,
            pCredentialParameters: &mut algorithm,
        };
        let client = WEBAUTHN_CLIENT_DATA {
            dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
            cbClientDataJSON: client_data.len() as u32,
            pbClientDataJSON: client_data.as_mut_ptr(),
            pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
        };

        // Ask the key to switch `hmac-secret` on for this credential.
        let mut enable = BOOL::from(true);
        let mut extension = WEBAUTHN_EXTENSION {
            pwszExtensionIdentifier: WEBAUTHN_EXTENSIONS_IDENTIFIER_HMAC_SECRET,
            cbExtension: std::mem::size_of::<BOOL>() as u32,
            pvExtension: (&mut enable as *mut BOOL).cast(),
        };
        let options = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
            // Version 4 holds every field used here and is understood by all
            // Windows versions that pass `require_available`.
            dwVersion: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_4,
            dwTimeoutMilliseconds: TIMEOUT_MS,
            Extensions: WEBAUTHN_EXTENSIONS { cExtensions: 1, pExtensions: &mut extension },
            dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            bRequireResidentKey: BOOL::from(false),
            dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
            dwAttestationConveyancePreference: WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
            ..Default::default()
        };

        let api = Self::api()?;
        let mut attestation: *mut WEBAUTHN_CREDENTIAL_ATTESTATION = std::ptr::null_mut();
        // SAFETY: every pointer refers to a local that outlives the call, and
        // the structures are filled in as `webauthn.h` documents.
        unsafe {
            (api.make_credential)(self.window(), &rp, &user, &algorithms, &client, &options, &mut attestation)
        }
        .ok()
        .map_err(|e| webauthn_error(api, e))?;
        if attestation.is_null() {
            return Err(Error::Platform("Windows returned no credential".into()));
        }
        let _free = FreeAttestation(api, attestation);

        // SAFETY: non-null and owned by us until `_free` drops; the API
        // guarantees the fields below are valid for their stated lengths.
        let (credential_id, hmac_secret_on) = unsafe {
            let attestation = &*attestation;
            (
                copy_bytes(attestation.pbCredentialId, attestation.cbCredentialId),
                extension_is_true(&attestation.Extensions, WEBAUTHN_EXTENSIONS_IDENTIFIER_HMAC_SECRET),
            )
        };
        if credential_id.is_empty() {
            return Err(Error::Platform("Windows returned an empty credential id".into()));
        }
        if !hmac_secret_on {
            return Err(Error::KeyUnusable {
                kind: KeyKind::SecurityKey,
                reason: "this security key does not support the hmac-secret feature a lock needs. Recent YubiKeys (5 series) do.".into(),
            });
        }
        Ok(credential_id)
    }

    /// Asks the key for the secret of one credential and salt.
    fn hmac_secret(&self, credential_id: &[u8], salt: &[u8; 32]) -> Result<Secret> {
        let mut client_data = client_data_json("webauthn.get")?;
        let client = WEBAUTHN_CLIENT_DATA {
            dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
            cbClientDataJSON: client_data.len() as u32,
            pbClientDataJSON: client_data.as_mut_ptr(),
            pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
        };

        let mut credential_id = credential_id.to_vec();
        let mut credential = WEBAUTHN_CREDENTIAL {
            dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
            cbId: credential_id.len() as u32,
            pbId: credential_id.as_mut_ptr(),
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
        };

        let mut salt_copy = *salt;
        let mut hmac_salt = WEBAUTHN_HMAC_SECRET_SALT {
            cbFirst: salt_copy.len() as u32,
            pbFirst: salt_copy.as_mut_ptr(),
            cbSecond: 0,
            pbSecond: std::ptr::null_mut(),
        };
        let mut salt_values = WEBAUTHN_HMAC_SECRET_SALT_VALUES {
            pGlobalHmacSalt: &mut hmac_salt,
            cCredWithHmacSecretSaltList: 0,
            pCredWithHmacSecretSaltList: std::ptr::null_mut(),
        };

        let options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
            // Version 6 is the first with `pHmacSecretSaltValues`.
            dwVersion: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_6,
            dwTimeoutMilliseconds: TIMEOUT_MS,
            CredentialList: WEBAUTHN_CREDENTIALS { cCredentials: 1, pCredentials: &mut credential },
            dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
            pHmacSecretSaltValues: &mut salt_values,
            ..Default::default()
        };

        let api = Self::api()?;
        let mut assertion: *mut WEBAUTHN_ASSERTION = std::ptr::null_mut();
        // SAFETY: every pointer refers to a local that outlives the call.
        unsafe { (api.get_assertion)(self.window(), RP_ID_W, &client, &options, &mut assertion) }
            .ok()
            .map_err(|e| webauthn_error(api, e))?;
        if assertion.is_null() {
            return Err(Error::Platform("Windows returned no answer from the key".into()));
        }
        let _free = FreeAssertion(api, assertion);

        // SAFETY: non-null and owned by us until `_free` drops. `pHmacSecret`
        // exists from structure version 3 on, which is checked first.
        let output = unsafe {
            let assertion = &*assertion;
            if assertion.dwVersion < WEBAUTHN_ASSERTION_VERSION_3 || assertion.pHmacSecret.is_null() {
                None
            } else {
                let hmac = &*assertion.pHmacSecret;
                Some(Zeroizing::new(copy_bytes(hmac.pbFirst, hmac.cbFirst)))
            }
        };
        let output = output.ok_or_else(|| Error::KeyUnusable {
            kind: KeyKind::SecurityKey,
            reason: "it did not return the secret a lock needs".into(),
        })?;
        let bytes: [u8; 32] = output.as_slice().try_into().map_err(|_| Error::KeyUnusable {
            kind: KeyKind::SecurityKey,
            reason: "it returned a secret of the wrong length".into(),
        })?;
        Ok(Secret::new(bytes))
    }
}

impl Default for SecurityKey {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySource for SecurityKey {
    fn kind(&self) -> KeyKind {
        KeyKind::SecurityKey
    }

    /// Shows two system windows: one to create the credential, one to read
    /// its secret for the first time.
    fn enroll(&mut self, label: &str) -> Result<(KeyDescriptor, Secret)> {
        Self::require_available()?;
        let mut descriptor = KeyDescriptor::new(KeyKind::SecurityKey, label)?;
        let credential_id = self.make_credential()?;
        descriptor.set_param_bytes(CREDENTIAL_ID_PARAM, &credential_id);
        descriptor.params.insert(RP_ID_PARAM.into(), RP_ID.into());
        let secret = self.hmac_secret(&credential_id, &descriptor.salt_bytes()?)?;
        Ok((descriptor, secret))
    }

    fn unlock(&mut self, descriptor: &KeyDescriptor) -> Result<Secret> {
        Self::require_available()?;
        if descriptor.param(RP_ID_PARAM)? != RP_ID {
            return Err(Error::Format("this security key was set up for a different app id".into()));
        }
        let credential_id = descriptor.param_bytes(CREDENTIAL_ID_PARAM)?;
        self.hmac_secret(&credential_id, &descriptor.salt_bytes()?)
    }
}

/// The client data the API signs over. For a native app nobody verifies the
/// signature, but the structure must still be well formed.
fn client_data_json(kind: &str) -> Result<Vec<u8>> {
    let challenge = B64URL.encode(random_bytes::<32>()?);
    Ok(format!(r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{ORIGIN}","crossOrigin":false}}"#)
        .into_bytes())
}

/// Copies `len` bytes from a buffer owned by `webauthn.dll`.
///
/// # Safety
/// `pointer` must be null or valid for `len` bytes.
unsafe fn copy_bytes(pointer: *const u8, len: u32) -> Vec<u8> {
    if pointer.is_null() || len == 0 {
        Vec::new()
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(pointer, len as usize) }.to_vec()
    }
}

/// Whether the returned extensions hold `identifier` with a TRUE value.
///
/// # Safety
/// `extensions` must come from a live structure returned by `webauthn.dll`.
unsafe fn extension_is_true(extensions: &WEBAUTHN_EXTENSIONS, identifier: PCWSTR) -> bool {
    if extensions.pExtensions.is_null() {
        return false;
    }
    // SAFETY: the API guarantees `cExtensions` valid entries.
    let list = unsafe { std::slice::from_raw_parts(extensions.pExtensions, extensions.cExtensions as usize) };
    list.iter().any(|extension| {
        // SAFETY: identifiers are null-terminated strings owned by the API;
        // the value of `hmac-secret` is a BOOL when its size says so.
        unsafe {
            !extension.pwszExtensionIdentifier.is_null()
                && extension.pwszExtensionIdentifier.as_wide() == identifier.as_wide()
                && extension.cbExtension as usize == std::mem::size_of::<BOOL>()
                && !extension.pvExtension.is_null()
                && (*(extension.pvExtension as *const BOOL)).as_bool()
        }
    })
}

fn webauthn_error(api: &WebAuthnApi, error: windows::core::Error) -> Error {
    if error.code() == NTE_USER_CANCELLED {
        return Error::Cancelled;
    }
    // SAFETY: returns a pointer to a static, null-terminated name.
    let name = unsafe { (api.error_name)(error.code()).to_string() }.unwrap_or_default();
    Error::Platform(format!("the security key request failed ({name}): {}", error.message()))
}

struct FreeAttestation(&'static WebAuthnApi, *mut WEBAUTHN_CREDENTIAL_ATTESTATION);

impl Drop for FreeAttestation {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `WebAuthNAuthenticatorMakeCredential`
        // and is freed exactly once.
        unsafe { (self.0.free_attestation)(self.1) }
    }
}

struct FreeAssertion(&'static WebAuthnApi, *mut WEBAUTHN_ASSERTION);

impl Drop for FreeAssertion {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `WebAuthNAuthenticatorGetAssertion`
        // and is freed exactly once.
        unsafe { (self.0.free_assertion)(self.1) }
    }
}
