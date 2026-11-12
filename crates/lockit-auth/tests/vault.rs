//! Tests for the vault and the keys that run on any platform.

use lockit_auth::{
    keys::passphrase::PassphraseKey, Error, KeyDescriptor, KeyKind, KeySource, Secret, Vault,
};

const PHRASE: &[u8] = b"abandon ability able about above absent absorb abstract absurd abuse access accident";

/// A stand-in for a hardware key: answers with a fixed secret per salt.
struct FakeKey {
    kind: KeyKind,
    seed: u8,
    asked: usize,
}

impl FakeKey {
    fn new(kind: KeyKind, seed: u8) -> Self {
        Self { kind, seed, asked: 0 }
    }

    fn secret_for(&self, descriptor: &KeyDescriptor) -> Secret {
        let mut out = descriptor.salt_bytes().unwrap();
        for byte in &mut out {
            *byte ^= self.seed;
        }
        Secret::new(out)
    }
}

impl KeySource for FakeKey {
    fn kind(&self) -> KeyKind {
        self.kind
    }

    fn enroll(&mut self, label: &str) -> lockit_auth::Result<(KeyDescriptor, Secret)> {
        let descriptor = KeyDescriptor::new(self.kind, label)?;
        let secret = self.secret_for(&descriptor);
        Ok((descriptor, secret))
    }

    fn unlock(&mut self, descriptor: &KeyDescriptor) -> lockit_auth::Result<Secret> {
        self.asked += 1;
        Ok(self.secret_for(descriptor))
    }
}

fn quick_passphrase(text: &str) -> PassphraseKey {
    // Low cost so the tests are fast; real locks use the default cost.
    PassphraseKey::new(text).with_cost(8 * 1024, 1)
}

#[test]
fn one_key_seals_and_opens() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let vault = Vault::seal(PHRASE, vec![hello.enroll("This PC").unwrap()]).unwrap();

    let opened = vault.open_with(|key| hello.unlock(key)).unwrap();
    assert_eq!(opened.as_slice(), PHRASE);
}

#[test]
fn three_keys_are_all_needed() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let mut yubi = FakeKey::new(KeyKind::SecurityKey, 0x22);
    let mut pass = quick_passphrase("correct horse battery staple");

    // Enrolled out of order on purpose: the vault sorts them.
    let vault = Vault::seal(
        PHRASE,
        vec![
            pass.enroll("Passphrase").unwrap(),
            hello.enroll("This PC").unwrap(),
            yubi.enroll("Blue YubiKey").unwrap(),
        ],
    )
    .unwrap();
    let kinds: Vec<_> = vault.keys.iter().map(|k| k.kind).collect();
    assert_eq!(kinds, [KeyKind::DeviceHello, KeyKind::SecurityKey, KeyKind::Passphrase]);

    let all = [
        hello.unlock(&vault.keys[0]).unwrap(),
        yubi.unlock(&vault.keys[1]).unwrap(),
        pass.unlock(&vault.keys[2]).unwrap(),
    ];
    assert_eq!(vault.open(&all).unwrap().as_slice(), PHRASE);

    // Two of three is not enough.
    assert!(matches!(vault.open(&all[..2]), Err(Error::KeyCount { .. })));

    // A different security key does not open it.
    let mut other = FakeKey::new(KeyKind::SecurityKey, 0x33);
    let wrong = [all[0].clone(), other.unlock(&vault.keys[1]).unwrap(), all[2].clone()];
    assert!(matches!(vault.open(&wrong), Err(Error::WrongKeyOrDamaged)));

    // Right secrets in the wrong order do not open it either.
    let swapped = [all[1].clone(), all[0].clone(), all[2].clone()];
    assert!(matches!(vault.open(&swapped), Err(Error::WrongKeyOrDamaged)));
}

#[test]
fn wrong_passphrase_is_refused() {
    let mut pass = quick_passphrase("correct horse battery staple");
    let vault = Vault::seal(PHRASE, vec![pass.enroll("Passphrase").unwrap()]).unwrap();

    let mut wrong = quick_passphrase("correct horse battery stable");
    let result = vault.open_with(|key| wrong.unlock(key));
    assert!(matches!(result, Err(Error::WrongKeyOrDamaged)));
}

#[test]
fn short_passphrase_is_refused_at_setup() {
    let mut pass = quick_passphrase("too short");
    assert!(matches!(pass.enroll("Passphrase"), Err(Error::PassphraseTooShort { .. })));
}

#[test]
fn key_count_and_kinds_are_checked() {
    assert!(matches!(Vault::seal(PHRASE, vec![]), Err(Error::KeyCount { given: 0, .. })));

    let mut a = FakeKey::new(KeyKind::SecurityKey, 1);
    let mut b = FakeKey::new(KeyKind::SecurityKey, 2);
    let result = Vault::seal(PHRASE, vec![a.enroll("A").unwrap(), b.enroll("B").unwrap()]);
    assert!(matches!(result, Err(Error::DuplicateKind(KeyKind::SecurityKey))));
}

#[test]
fn survives_a_round_trip_through_json() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let vault = Vault::seal(PHRASE, vec![hello.enroll("This PC").unwrap()]).unwrap();

    let json = vault.to_json().unwrap();
    assert!(!json.contains("abandon"), "the phrase must never appear in the file");
    let read_back = Vault::from_json(&json).unwrap();
    assert_eq!(read_back, vault);

    let opened = read_back.open_with(|key| hello.unlock(key)).unwrap();
    assert_eq!(opened.as_slice(), PHRASE);
}

#[test]
fn any_change_to_the_file_is_noticed() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let mut yubi = FakeKey::new(KeyKind::SecurityKey, 0x22);
    let vault = Vault::seal(
        PHRASE,
        vec![hello.enroll("This PC").unwrap(), yubi.enroll("Blue YubiKey").unwrap()],
    )
    .unwrap();
    let secrets = [hello.unlock(&vault.keys[0]).unwrap(), yubi.unlock(&vault.keys[1]).unwrap()];
    assert!(vault.open(&secrets).is_ok());

    // Renaming a key.
    let mut renamed = vault.clone();
    renamed.keys[1].label = "Someone else's key".into();
    assert!(matches!(renamed.open(&secrets), Err(Error::WrongKeyOrDamaged)));

    // Adding a public field to a key.
    let mut extra = vault.clone();
    extra.keys[0].params.insert("note".into(), "x".into());
    assert!(matches!(extra.open(&secrets), Err(Error::WrongKeyOrDamaged)));

    // Dropping the second key from the file, to try to open with one.
    let mut fewer = vault.clone();
    fewer.keys.pop();
    assert!(matches!(fewer.open(&secrets[..1]), Err(Error::WrongKeyOrDamaged)));

    // Flipping one bit of the ciphertext.
    let mut flipped = vault.clone();
    let mut raw = base64_decode(&flipped.ciphertext);
    raw[0] ^= 1;
    flipped.ciphertext = base64_encode(&raw);
    assert!(matches!(flipped.open(&secrets), Err(Error::WrongKeyOrDamaged)));

    // A newer file version is refused before any key is asked.
    let mut newer = vault.clone();
    newer.version = 2;
    assert!(matches!(newer.open(&secrets), Err(Error::UnsupportedVersion { found: 2, .. })));
}

#[test]
fn a_cancelled_prompt_opens_nothing() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let mut yubi = FakeKey::new(KeyKind::SecurityKey, 0x22);
    let vault = Vault::seal(
        PHRASE,
        vec![hello.enroll("This PC").unwrap(), yubi.enroll("Blue YubiKey").unwrap()],
    )
    .unwrap();

    let result = vault.open_with(|descriptor| match descriptor.kind {
        KeyKind::DeviceHello => hello.unlock(descriptor),
        _ => Err(Error::Cancelled),
    });
    assert!(matches!(result, Err(Error::Cancelled)));
}

#[test]
fn every_sealing_is_different() {
    let mut hello = FakeKey::new(KeyKind::DeviceHello, 0x11);
    let enrolled = hello.enroll("This PC").unwrap();
    let first = Vault::seal(PHRASE, vec![enrolled.clone()]).unwrap();
    let second = Vault::seal(PHRASE, vec![enrolled]).unwrap();
    assert_ne!(first.vault_id, second.vault_id);
    assert_ne!(first.nonce, second.nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
}

#[test]
fn secrets_are_not_printed() {
    let secret = Secret::new([7u8; 32]);
    assert_eq!(format!("{secret:?}"), "Secret(..)");
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(text).unwrap()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
