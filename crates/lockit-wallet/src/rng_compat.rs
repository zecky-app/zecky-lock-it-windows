//! Zecky NU7 engine: rand 0.10 has no `OsRng`. The NU7 releases of the Zcash
//! crates take any `rand_core::Rng + CryptoRng`; this keeps the old name for
//! the operating system's secure generator (getrandom: CCRandomGenerateBytes
//! on iOS, getentropy on macOS), panicking instead of silently degrading if
//! it fails.
//!
//! `OsRng` is a stateless unit struct (both a type and a value, like the old
//! `rand::rngs::OsRng`), so `&mut OsRng` borrows a fresh temporary rather than
//! a `const` item. Every draw goes straight to the OS generator via
//! `rand::rngs::SysRng`; a failure panics, exactly as `rand_core::UnwrapErr`
//! does (and as rand 0.8's `OsRng` did).
use core::convert::Infallible;

use rand::rand_core::{Rng as _, TryCryptoRng, TryRng, UnwrapErr};
use rand::rngs::SysRng;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OsRng;

impl TryRng for OsRng {
    type Error = Infallible;

    #[inline]
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(UnwrapErr(SysRng).next_u32())
    }

    #[inline]
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(UnwrapErr(SysRng).next_u64())
    }

    #[inline]
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        UnwrapErr(SysRng).fill_bytes(dst);
        Ok(())
    }
}

impl TryCryptoRng for OsRng {}

#[cfg(test)]
mod tests {
    use super::OsRng;
    use rand::rand_core::{CryptoRng, Rng};

    fn assert_crypto_rng<R: CryptoRng>(_: &mut R) {}

    #[test]
    fn os_rng_is_a_crypto_rng_and_produces_output() {
        assert_crypto_rng(&mut OsRng);
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut a);
        OsRng.fill_bytes(&mut b);
        assert_ne!(a, [0u8; 32]);
        assert_ne!(a, b);
    }
}
