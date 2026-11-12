//! A 32-byte secret that is wiped when dropped and never printed.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// A 32-byte secret produced by a key.
///
/// The bytes are wiped from memory when the value is dropped, and `Debug`
/// never shows them.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Secret([u8; 32]);

impl Secret {
    /// Wraps 32 bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Makes a fresh secret from the operating system's random numbers.
    pub fn random() -> crate::Result<Self> {
        Ok(Self(random_bytes()?))
    }

    /// Borrows the bytes. Keep the borrow short.
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// Fills a buffer from the operating system's random number generator.
pub(crate) fn random_bytes<const N: usize>() -> crate::Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).map_err(|_| crate::Error::Random)?;
    Ok(out)
}
