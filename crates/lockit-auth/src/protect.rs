//! Small state files the owner should not be able to change by hand (when
//! the time lock was asked for, when the glass was broken), protected with
//! Windows DPAPI for this Windows account. Deleting such a file only makes
//! the lock stricter: the request is gone and has to be made again.
//!
//! On other systems (tests) the bytes pass through unchanged.

use crate::Result;

/// Encrypts `data` for the current Windows account.
pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    platform::protect(data)
}

/// Decrypts what [`protect`] made. Fails if the bytes were changed.
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    platform::unprotect(data)
}

#[cfg(windows)]
mod platform {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB},
        },
    };

    use crate::{Error, Result};

    /// Mixed in, so other apps' DPAPI blobs are not accepted as ours.
    const ENTROPY: &[u8] = b"zecky-lockit/state/v1";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI returned a buffer of `cbData` bytes that we own and
        // free exactly once with LocalFree.
        unsafe {
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
            bytes
        }
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: the input blobs point at live slices for the whole call.
        unsafe { CryptProtectData(&input, PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| Error::Platform(format!("protecting a state file failed: {e}")))?;
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as above.
        unsafe { CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|_| Error::Format("a protected state file was changed or belongs to another account".into()))?;
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::Result;

    pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
        Ok(data.to_vec())
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
        Ok(data.to_vec())
    }
}
