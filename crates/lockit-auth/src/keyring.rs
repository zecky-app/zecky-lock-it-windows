//! Extra wallets' words, behind the same lock (as Lock it Mac's
//! KeyRingVault): each is encrypted with a key derived from the main
//! wallet's seed. That key is never stored; it only exists while the main
//! lock is open. So every wallet on the key ring is exactly as well
//! protected as the lock, and adding one needs no new seal.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{secret::random_bytes, Error, Result};

const INFO: &[u8] = b"zecky-lockit-keyring-v1";

fn cipher(main_seed: &[u8]) -> Result<XChaCha20Poly1305> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(b"zecky"), main_seed)
        .expand(INFO, key.as_mut())
        .map_err(|_| Error::Platform("deriving the key ring key failed".into()))?;
    Ok(XChaCha20Poly1305::new(key.as_ref().into()))
}

/// Encrypts an extra wallet's words with the main wallet's seed. Base64.
pub fn seal_words(main_seed: &[u8], words: &str) -> Result<String> {
    let nonce: [u8; 24] = random_bytes()?;
    let mut out = nonce.to_vec();
    out.extend(
        cipher(main_seed)?
            .encrypt(XNonce::from_slice(&nonce), words.as_bytes())
            .map_err(|_| Error::Platform("sealing the words failed".into()))?,
    );
    Ok(B64.encode(out))
}

/// Opens what [`seal_words`] made, with the same main seed.
pub fn open_words(main_seed: &[u8], sealed: &str) -> Result<Zeroizing<String>> {
    let raw = B64.decode(sealed).map_err(|_| Error::Format("a key ring entry is damaged".into()))?;
    if raw.len() < 24 {
        return Err(Error::Format("a key ring entry is damaged".into()));
    }
    let plain = Zeroizing::new(
        cipher(main_seed)?
            .decrypt(XNonce::from_slice(&raw[..24]), &raw[24..])
            .map_err(|_| Error::WrongKeyOrDamaged)?,
    );
    String::from_utf8(plain.to_vec()).map(Zeroizing::new).map_err(|_| Error::Format("a key ring entry is damaged".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_open_only_with_the_main_seed() {
        let sealed = seal_words(&[1; 64], "abandon ability able").unwrap();
        assert_eq!(open_words(&[1; 64], &sealed).unwrap().as_str(), "abandon ability able");
        assert!(open_words(&[2; 64], &sealed).is_err());
        assert_ne!(seal_words(&[1; 64], "abandon ability able").unwrap(), sealed, "a fresh nonce every time");
    }
}
