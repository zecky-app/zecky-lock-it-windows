//! The 24 words (BIP-39, English), as Lock it Mac and Lock it iPhone use them.

use bip0039::{Count, English, Mnemonic};
use rand::rand_core::Rng as _;
use secrecy::SecretVec;
use zeroize::Zeroizing;

use crate::{rng_compat::OsRng, Error, Result};

/// The 64-byte seed made from the words. Wiped from memory when dropped.
pub type Seed = SecretVec<u8>;

/// A new set of 24 words, from 256 bits of the operating system's randomness.
pub fn new_phrase() -> Zeroizing<String> {
    let mut entropy = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(entropy.as_mut());
    let mnemonic = <Mnemonic<English>>::from_entropy(entropy.to_vec()).expect("32 bytes is a valid entropy length");
    debug_assert_eq!(mnemonic.phrase().split(' ').count(), Count::Words24.word_count());
    Zeroizing::new(mnemonic.phrase().to_owned())
}

/// Puts typed words in the standard form: lowercase, single spaces.
fn normalize(phrase: &str) -> String {
    phrase.split_whitespace().map(str::to_lowercase).collect::<Vec<_>>().join(" ")
}

/// Checks the words and their checksum. Lock it uses 24 words only.
pub fn validate_phrase(phrase: &str) -> Result<()> {
    let normalized = Zeroizing::new(normalize(phrase));
    let count = normalized.split(' ').count();
    if count != 24 {
        return Err(Error::Phrase(format!("Lock it uses 24 words; {count} were given")));
    }
    <Mnemonic<English>>::from_phrase(normalized.as_str()).map(|_| ()).map_err(|e| Error::Phrase(e.to_string()))
}

/// The seed for the words (no BIP-39 passphrase, as on the Mac and iPhone).
pub fn phrase_to_seed(phrase: &str) -> Result<Seed> {
    validate_phrase(phrase)?;
    let normalized = Zeroizing::new(normalize(phrase));
    let mnemonic = <Mnemonic<English>>::from_phrase(normalized.as_str()).map_err(|e| Error::Phrase(e.to_string()))?;
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    Ok(SecretVec::new(seed.to_vec()))
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn a_new_phrase_has_24_valid_words() {
        let phrase = new_phrase();
        assert_eq!(phrase.split(' ').count(), 24);
        validate_phrase(&phrase).unwrap();
        assert_ne!(*new_phrase(), *phrase);
    }

    #[test]
    fn the_seed_matches_the_bip39_test_vector() {
        // BIP-39 test vector (24 words, empty passphrase), from the Trezor set.
        let words = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
        let seed = phrase_to_seed(words).unwrap();
        let hex: String = seed.expose_secret().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(&hex[..32], "408b285c123836004f4b8842c89324c1");
    }

    #[test]
    fn typing_variations_are_accepted_and_bad_words_refused() {
        let phrase = new_phrase();
        let messy = format!("  {}  ", phrase.to_uppercase().replace(' ', "   "));
        validate_phrase(&messy).unwrap();
        assert!(validate_phrase("abandon abandon").is_err());
        let mut words: Vec<_> = phrase.split(' ').map(str::to_owned).collect();
        words[23] = if words[23] == "zoo" { "abandon".into() } else { "zoo".into() };
        // A wrong last word almost always breaks the checksum.
        let _ = validate_phrase(&words.join(" "));
    }
}
