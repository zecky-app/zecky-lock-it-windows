//! A FIDO2 security key (for example a YubiKey) as a check on the lock.
//!
//! The same design as Lock it Mac: pairing saves the key's public key; to
//! open the lock, the key signs a fresh random challenge and the signature is
//! checked against that public key (see [`super::key_check`]). The key gives
//! no secret, so a lock with a security key also holds Windows Hello or a
//! passphrase.
//!
//! Windows does not let a normal program talk to a security key directly;
//! everything goes through the system's WebAuthn API (`webauthn.dll`), which
//! shows the familiar "touch your security key" window. Only the first
//! version of that interface is used, so this works on Windows 10 too.
//!
//! Like on the Mac, the key's PIN is not asked for ("discouraged"): touching
//! the key is the check.

use std::{ffi::c_void, sync::OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use windows::{
    core::{w, HRESULT, PCWSTR},
    Win32::{
        Foundation::{HMODULE, HWND},
        Networking::WindowsWebServices::*,
        System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32},
        UI::WindowsAndMessaging::GetForegroundWindow,
    },
};

use super::{
    key_check::{self, KeyEntry, PairedKey, Role, RELYING_PARTY},
    KeyDescriptor, KeyKind, KeySource,
};
use crate::{secret::random_bytes, Error, Result, Secret};

const RP_ID_W: PCWSTR = w!("zecky.app");
const RP_NAME_W: PCWSTR = w!("Zecky Lock it");
const ORIGIN: &str = "https://zecky.app";
const USER_NAME_W: PCWSTR = w!("Zecky Lock it");

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

    /// The version of Windows' security-key interface, if it has one.
    pub fn api_version() -> Option<u32> {
        // SAFETY: takes no arguments and only reads a version number.
        WebAuthnApi::get().map(|api| unsafe { (api.version)() })
    }

    /// Whether this Windows can ask a security key at all (Windows 10 1903
    /// and later).
    pub fn is_available() -> bool {
        Self::api_version().is_some_and(|version| version >= 1)
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

    /// Pairs the key: creates a credential and reads its public key.
    fn register(&self) -> Result<PairedKey> {
        let api = Self::api()?;
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
        let algorithms = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS { cCredentialParameters: 1, pCredentialParameters: &mut algorithm };
        let client = WEBAUTHN_CLIENT_DATA {
            dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
            cbClientDataJSON: client_data.len() as u32,
            pbClientDataJSON: client_data.as_mut_ptr(),
            pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
        };
        let options = WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
            // Version 1: understood by every Windows with webauthn.dll.
            dwVersion: WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS_VERSION_1,
            dwTimeoutMilliseconds: TIMEOUT_MS,
            dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            bRequireResidentKey: false.into(),
            dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
            dwAttestationConveyancePreference: WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
            ..Default::default()
        };

        let mut attestation: *mut WEBAUTHN_CREDENTIAL_ATTESTATION = std::ptr::null_mut();
        // SAFETY: every pointer refers to a local that outlives the call, and
        // the structures are filled in as `webauthn.h` documents.
        unsafe { (api.make_credential)(self.window(), &rp, &user, &algorithms, &client, &options, &mut attestation) }
            .ok()
            .map_err(|e| webauthn_error(api, e))?;
        if attestation.is_null() {
            return Err(Error::Platform("Windows returned no credential".into()));
        }
        let _free = FreeAttestation(api, attestation);
        // SAFETY: non-null and owned by us until `_free` drops; the API
        // guarantees the fields are valid for their stated lengths.
        let (credential_id, auth_data) = unsafe {
            let attestation = &*attestation;
            (
                copy_bytes(attestation.pbCredentialId, attestation.cbCredentialId),
                copy_bytes(attestation.pbAuthenticatorData, attestation.cbAuthenticatorData),
            )
        };
        key_check::read_registration(&auth_data, &credential_id)
    }

    /// Asks any of `keys` to sign a fresh challenge and checks the answer.
    /// Returns the key that answered.
    fn check(&self, keys: &[KeyEntry]) -> Result<KeyEntry> {
        let api = Self::api()?;
        if keys.is_empty() {
            return Err(Error::Format("no security key is paired".into()));
        }
        let mut client_data = client_data_json("webauthn.get")?;
        let client = WEBAUTHN_CLIENT_DATA {
            dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
            cbClientDataJSON: client_data.len() as u32,
            pbClientDataJSON: client_data.as_mut_ptr(),
            pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
        };
        let mut ids: Vec<Vec<u8>> = keys.iter().map(|k| k.paired().map(|p| p.credential_id)).collect::<Result<_>>()?;
        let mut credentials: Vec<WEBAUTHN_CREDENTIAL> = ids
            .iter_mut()
            .map(|id| WEBAUTHN_CREDENTIAL {
                dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
                cbId: id.len() as u32,
                pbId: id.as_mut_ptr(),
                pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            })
            .collect();
        let options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
            dwVersion: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_1,
            dwTimeoutMilliseconds: TIMEOUT_MS,
            CredentialList: WEBAUTHN_CREDENTIALS { cCredentials: credentials.len() as u32, pCredentials: credentials.as_mut_ptr() },
            dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
            dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
            ..Default::default()
        };

        let mut assertion: *mut WEBAUTHN_ASSERTION = std::ptr::null_mut();
        // SAFETY: every pointer refers to a local that outlives the call.
        unsafe { (api.get_assertion)(self.window(), RP_ID_W, &client, &options, &mut assertion) }
            .ok()
            .map_err(|e| webauthn_error(api, e))?;
        if assertion.is_null() {
            return Err(Error::Platform("Windows returned no answer from the key".into()));
        }
        let _free = FreeAssertion(api, assertion);
        // SAFETY: non-null and owned by us until `_free` drops.
        let (id, auth_data, signature) = unsafe {
            let assertion = &*assertion;
            (
                copy_bytes(assertion.Credential.pbId, assertion.Credential.cbId),
                copy_bytes(assertion.pbAuthenticatorData, assertion.cbAuthenticatorData),
                copy_bytes(assertion.pbSignature, assertion.cbSignature),
            )
        };
        key_check::check_any(keys, &id, &auth_data, &client_data, &signature)
    }

    fn keys(descriptor: &KeyDescriptor) -> Result<Vec<KeyEntry>> {
        if descriptor.param(RP_ID_PARAM)? != RELYING_PARTY {
            return Err(Error::Format("this security key was set up for a different app id".into()));
        }
        key_check::keys_of(descriptor)
    }

    /// Pairs one more key (touch it). Its entry is added to the lock by the
    /// caller, which seals the lock again.
    pub fn pair(&self, role: Role, now: i64) -> Result<KeyEntry> {
        Ok(KeyEntry::new(&self.register()?, role, now))
    }

    /// "Test my key": any paired key answers correctly. Returns its role.
    pub fn test(&self, descriptor: &KeyDescriptor) -> Result<Role> {
        Ok(self.check(&Self::keys(descriptor)?)?.role)
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

    /// Shows one system window: touch the key to pair it.
    fn enroll(&mut self, label: &str) -> Result<(KeyDescriptor, Secret)> {
        if !Self::is_available() {
            Self::api()?;
        }
        let mut descriptor = KeyDescriptor::new(KeyKind::SecurityKey, label)?;
        descriptor.params.insert(RP_ID_PARAM.into(), RELYING_PARTY.into());
        let entry = KeyEntry::new(&self.register()?, Role::Main, 0);
        key_check::set_keys(&mut descriptor, &[entry])?;
        let secret = key_check::vault_secret(&descriptor)?;
        Ok((descriptor, secret))
    }

    /// Shows one system window: touch any paired key; its answer is checked.
    fn unlock(&mut self, descriptor: &KeyDescriptor) -> Result<Secret> {
        self.check(&Self::keys(descriptor)?)?;
        key_check::vault_secret(descriptor)
    }
}

/// The client data the key signs over (by its hash). It holds a fresh random
/// challenge, so an old answer cannot be used again.
fn client_data_json(kind: &str) -> Result<Vec<u8>> {
    let challenge = B64URL.encode(random_bytes::<32>()?);
    Ok(format!(r#"{{"type":"{kind}","challenge":"{challenge}","origin":"{ORIGIN}","crossOrigin":false}}"#).into_bytes())
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
