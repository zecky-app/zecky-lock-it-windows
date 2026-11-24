//! Seal it: air-gapped signing on the phone (the Keystone QR protocol, as
//! Lock it Mac, Zashi and Keystone use it).
//!
//! ```text
//!   Pair:  scan ur:zcash-accounts  -> import the UFVK (key source "sealit")
//!   Send:  proposal -> PCZT -> redacted copy shown as ur:zcash-pczt
//!          proofs made here, on the full PCZT, while the phone signs
//!          scan the signed ur:zcash-pczt -> combine -> extract -> send
//! ```
//!
//! Spending keys never touch this PC. Sapling is refused (as on the Mac).
//!
//! The signing phone can also be Cupcake (Cake Wallet's offline signer):
//! it pairs with a plain `zcash-wallet:?label=…&private_key=<UFVK>` QR,
//! reads only sequenced lower-case frames, and signs PCZTs in its own
//! shielded-assets layout (see [`crate::cupcake`]); it makes the proofs
//! itself, so it gets the whole PCZT (Lock it Mac, 22 Nov 2026).

use crate::{Error, Result};

/// UR types exchanged with the signer.
pub const UR_ACCOUNTS: &str = "zcash-accounts";
/// The PCZT, both ways.
pub const UR_PCZT: &str = "zcash-pczt";
/// Keystone, Zashi and Seal it all use 200.
const FRAGMENT: usize = 200;

// ---------- A tiny CBOR reader (signers need not be deterministic) ----------

#[derive(Debug, Clone, PartialEq)]
enum Cbor {
    Uint(u64),
    Negative,
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(Cbor, Cbor)>),
    Tag(Box<Cbor>),
    Simple,
}

impl Cbor {
    fn untagged(&self) -> &Cbor {
        match self {
            Cbor::Tag(inner) => inner.untagged(),
            other => other,
        }
    }
    fn get(&self, key: u64) -> Option<&Cbor> {
        match self.untagged() {
            Cbor::Map(entries) => entries.iter().find(|(k, _)| *k == Cbor::Uint(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    fn bytes(&self) -> Option<&[u8]> {
        match self.untagged() {
            Cbor::Bytes(b) => Some(b),
            _ => None,
        }
    }
    fn text(&self) -> Option<&str> {
        match self.untagged() {
            Cbor::Text(t) => Some(t),
            _ => None,
        }
    }
    fn uint(&self) -> Option<u64> {
        match self.untagged() {
            Cbor::Uint(n) => Some(*n),
            _ => None,
        }
    }
    fn array(&self) -> Option<&[Cbor]> {
        match self.untagged() {
            Cbor::Array(a) => Some(a),
            _ => None,
        }
    }
}

fn bad(what: &str) -> Error {
    Error::Other(format!("the QR code could not be read: {what}"))
}

fn cbor_decode(data: &[u8]) -> Result<Cbor> {
    let mut at = 0;
    let item = cbor_item(data, &mut at, 0)?;
    if at != data.len() {
        return Err(bad("unexpected extra data"));
    }
    Ok(item)
}

fn cbor_arg(b: &[u8], at: &mut usize, info: u8) -> Result<u64> {
    let take = |at: &mut usize, n: usize| -> Result<u64> {
        if *at + n > b.len() {
            return Err(bad("it ended too early"));
        }
        let v = b[*at..*at + n].iter().fold(0u64, |v, x| (v << 8) | u64::from(*x));
        *at += n;
        Ok(v)
    };
    match info {
        0..=23 => Ok(u64::from(info)),
        24 => take(at, 1),
        25 => take(at, 2),
        26 => take(at, 4),
        27 => take(at, 8),
        _ => Err(bad("indefinite lengths")),
    }
}

fn cbor_item(b: &[u8], at: &mut usize, depth: u32) -> Result<Cbor> {
    if depth > 32 {
        return Err(bad("nested too deep"));
    }
    let initial = *b.get(*at).ok_or_else(|| bad("it ended too early"))?;
    *at += 1;
    let (major, info) = (initial >> 5, initial & 0x1f);
    let arg = cbor_arg(b, at, info)?;
    let mut data = |n: u64| -> Result<Vec<u8>> {
        let n = usize::try_from(n).map_err(|_| bad("too long"))?;
        if n > b.len() - *at {
            return Err(bad("it ended too early"));
        }
        let d = b[*at..*at + n].to_vec();
        *at += n;
        Ok(d)
    };
    Ok(match major {
        0 => Cbor::Uint(arg),
        1 => Cbor::Negative,
        2 => Cbor::Bytes(data(arg)?),
        3 => Cbor::Text(String::from_utf8(data(arg)?).map_err(|_| bad("invalid text"))?),
        4 => {
            if arg > b.len() as u64 {
                return Err(bad("it ended too early"));
            }
            let mut items = Vec::new();
            for _ in 0..arg {
                items.push(cbor_item(b, at, depth + 1)?);
            }
            Cbor::Array(items)
        }
        5 => {
            if arg > b.len() as u64 {
                return Err(bad("it ended too early"));
            }
            let mut entries = Vec::new();
            for _ in 0..arg {
                let k = cbor_item(b, at, depth + 1)?;
                let v = cbor_item(b, at, depth + 1)?;
                entries.push((k, v));
            }
            Cbor::Map(entries)
        }
        6 => Cbor::Tag(Box::new(cbor_item(b, at, depth + 1)?)),
        _ => {
            if info > 24 {
                return Err(bad("floats"));
            }
            Cbor::Simple
        }
    })
}

fn cbor_head(major: u8, value: u64) -> Vec<u8> {
    let m = major << 5;
    match value {
        0..=23 => vec![m | value as u8],
        24..=0xff => vec![m | 24, value as u8],
        0x100..=0xffff => {
            let mut v = vec![m | 25];
            v.extend((value as u16).to_be_bytes());
            v
        }
        0x1_0000..=0xffff_ffff => {
            let mut v = vec![m | 26];
            v.extend((value as u32).to_be_bytes());
            v
        }
        _ => {
            let mut v = vec![m | 27];
            v.extend(value.to_be_bytes());
            v
        }
    }
}

/// `{1: bytes(pczt)}`, the `ur:zcash-pczt` body (starts 0xa1 0x01).
pub fn pczt_envelope(pczt: &[u8]) -> Vec<u8> {
    let mut d = vec![0xa1, 0x01];
    d.extend(cbor_head(2, pczt.len() as u64));
    d.extend(pczt);
    d
}

/// One account from the signer's `ur:zcash-accounts`.
#[derive(Clone, Debug)]
pub struct SignerAccount {
    /// Its unified full viewing key.
    pub ufvk: String,
    /// Its ZIP 32 account number.
    pub index: u32,
    /// Its name on the phone.
    pub name: Option<String>,
}

/// What the phone shows to pair: its seed's fingerprint and accounts.
#[derive(Clone, Debug)]
pub struct AccountsExport {
    /// The fingerprint of the phone's seed; `None` for Cupcake, whose QR
    /// carries only the viewing key.
    pub seed_fingerprint: Option<[u8; 32]>,
    /// Its accounts (the first is paired).
    pub accounts: Vec<SignerAccount>,
}

impl AccountsExport {
    /// Paired from Cupcake's QR.
    pub fn from_cupcake(&self) -> bool {
        self.seed_fingerprint.is_none()
    }
}

/// Whether a scanned text is Cupcake's account QR.
pub fn is_cupcake_pairing(text: &str) -> bool {
    text.trim().to_ascii_lowercase().starts_with("zcash-wallet:")
}

/// Cupcake's account QR: `zcash-wallet:?label=<name>&private_key=<UFVK>`.
/// The "private_key" is a viewing key (it cannot spend); it must be a valid
/// mainnet unified full viewing key. No seed fingerprint or account number:
/// Cupcake signs with its own account.
pub fn parse_cupcake(text: &str) -> Result<AccountsExport> {
    let refused = || bad("it holds no valid Zcash viewing key (mainnet)");
    let text = text.trim();
    if !is_cupcake_pairing(text) {
        return Err(refused());
    }
    let query = text.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut label = None;
    let mut ufvk = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            "label" => label = Some(value),
            "private_key" => ufvk = Some(value),
            _ => {}
        }
    }
    let ufvk = ufvk.ok_or_else(refused)?;
    zcash_keys::keys::UnifiedFullViewingKey::decode(&zcash_protocol::consensus::MainNetwork, &ufvk).map_err(|_| refused())?;
    let label = label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).unwrap_or_else(|| "Cupcake".into());
    let name: String = label.chars().take(40).collect();
    Ok(AccountsExport { seed_fingerprint: None, accounts: vec![SignerAccount { ufvk, index: 0, name: Some(name) }] })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads `ur:zcash-accounts` (the Keystone registry format), or Cupcake's
/// plain `zcash-wallet:` QR.
pub fn parse_accounts(message: &[u8]) -> Result<AccountsExport> {
    if let Ok(text) = std::str::from_utf8(message) {
        if is_cupcake_pairing(text) {
            return parse_cupcake(text);
        }
    }
    let root = cbor_decode(message)?;
    let fingerprint = root.get(1).and_then(Cbor::bytes).ok_or_else(|| bad("the seed fingerprint is missing"))?;
    let seed_fingerprint: [u8; 32] = fingerprint.try_into().map_err(|_| bad("the seed fingerprint is not 32 bytes"))?;
    let list = root.get(2).and_then(Cbor::array).ok_or_else(|| bad("the accounts are missing"))?;
    let accounts: Vec<SignerAccount> = list
        .iter()
        .filter_map(|item| {
            Some(SignerAccount {
                ufvk: item.get(1)?.text()?.to_owned(),
                index: item.get(2).and_then(Cbor::uint).unwrap_or(0) as u32,
                name: item.get(3).and_then(Cbor::text).filter(|n| !n.is_empty()).map(str::to_owned),
            })
        })
        .collect();
    if accounts.is_empty() {
        return Err(bad("it holds no accounts"));
    }
    Ok(AccountsExport { seed_fingerprint: Some(seed_fingerprint), accounts })
}

/// Reads `ur:zcash-pczt` into the raw PCZT.
pub fn parse_pczt(message: &[u8]) -> Result<Vec<u8>> {
    let root = cbor_decode(message)?;
    root.get(1).and_then(Cbor::bytes).map(<[u8]>::to_vec).ok_or_else(|| bad("the signed transaction is missing"))
}

/// Keystone can fail to read a payment larger than this (Keystone issue
/// #2294: PCZTs over 14 KiB). Lock it still shows such a payment, with a
/// warning: the size depends on how many notes it spends.
pub const KEYSTONE_SAFE_PCZT: usize = 14 * 1024;

/// Whether a payment of `pczt_len` bytes for Keystone needs the warning.
pub fn keystone_may_not_read(pczt_len: usize) -> bool {
    pczt_len > KEYSTONE_SAFE_PCZT
}

/// The name of a newly paired wallet says where it signs: "Keystone ·
/// Account 0", "Cupcake · My wallet" (as Lock it Mac). A pairing with a
/// seed fingerprint comes from Keystone (Seal it uses the same QR but is not
/// live). At most 40 characters.
pub fn wallet_name(given: Option<&str>, cupcake: bool) -> String {
    let device = if cupcake { "Cupcake" } else { "Keystone" };
    let name = given.unwrap_or("").trim();
    let full = if name.is_empty() || name.eq_ignore_ascii_case(device) {
        device.to_owned()
    } else if name.to_lowercase().starts_with(&device.to_lowercase()) {
        name.to_owned()
    } else {
        format!("{device} · {name}")
    };
    full.chars().take(40).collect()
}

// ---------- UR (Uniform Resources) framing ----------

/// The frames of a (possibly animated) QR code, upper-case so the QR uses
/// the denser alphanumeric mode. A single frame when it fits; otherwise
/// twice the fragment count, so mixed fountain parts follow the plain ones.
pub fn ur_frames(ur_type: &str, message: &[u8]) -> Result<Vec<String>> {
    let mut encoder = ur::Encoder::new(message, FRAGMENT, ur_type).map_err(|e| Error::Other(e.to_string()))?;
    let count = encoder.fragment_count();
    if count == 1 {
        return Ok(vec![ur::encode(message, &ur::Type::Custom(ur_type)).to_uppercase()]);
    }
    (0..count * 2).map(|_| encoder.next_part().map(|p| p.to_uppercase()).map_err(|e| Error::Other(e.to_string()))).collect()
}

/// The frames for Cupcake: its reader only takes the sequenced form
/// "ur:type/n-m/…" (even for one frame), compares the type in lower case,
/// and finishes at exactly n distinct frames of n, so it never takes the
/// mixed frames after n. Lower case, sequenced, frames 1..n only, looped.
pub fn cupcake_frames(ur_type: &str, message: &[u8]) -> Result<Vec<String>> {
    let mut encoder = ur::Encoder::new(message, FRAGMENT, ur_type).map_err(|e| Error::Other(e.to_string()))?;
    let count = encoder.fragment_count();
    (0..count).map(|_| encoder.next_part().map(|p| p.to_ascii_lowercase()).map_err(|e| Error::Other(e.to_string()))).collect()
}

/// How far a scan is.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScanProgress {
    /// Every part is in.
    pub done: bool,
    /// Parts seen so far.
    pub seen: usize,
    /// Parts in the message.
    pub of: usize,
    /// The account QR came from Cupcake (it needs no "when were the words made").
    pub cupcake: bool,
}

/// Limits for what a scanned part may announce (Lock it Mac, audit 22 Nov
/// 2026): a huge part count or message length would allocate without limit.
/// A Seal it transaction is a few kilobytes in a few dozen parts.
pub const MAX_PARTS: u64 = 1000;
/// The largest message a sequence of parts may announce.
pub const MAX_MESSAGE: u64 = 10 * 1024 * 1024;

/// Refuses a part whose numbering ("n-m") or whose own header announces 0
/// parts, more than 1,000 parts, or a message of 0 bytes or over 10 MB.
fn check_part_lengths(numbering: &str, text: &str) -> Result<()> {
    let refused = || bad("the moving code announces an impossible length");
    let (_, of) = numbering.split_once('-').ok_or_else(|| bad("the moving code's numbering"))?;
    let of: u64 = of.parse().map_err(|_| bad("the moving code's numbering"))?;
    if !(1..=MAX_PARTS).contains(&of) {
        return Err(refused());
    }
    let (_, cbor) = ur::decode(text).map_err(|_| bad("the code is damaged"))?;
    let (parts, length) = part_header(&cbor).ok_or_else(|| bad("the code is damaged"))?;
    if !(1..=MAX_PARTS).contains(&parts) || !(1..=MAX_MESSAGE).contains(&length) {
        return Err(refused());
    }
    Ok(())
}

/// Part count and message length from a part's CBOR header:
/// [sequence, part count, message length, checksum, data].
fn part_header(cbor: &[u8]) -> Option<(u64, u64)> {
    fn uint(bytes: &[u8], at: &mut usize) -> Option<u64> {
        let first = *bytes.get(*at)?;
        if first >> 5 != 0 {
            return None;
        }
        *at += 1;
        let extra = match first & 0x1f {
            n @ 0..=23 => return Some(n as u64),
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => return None,
        };
        let raw = bytes.get(*at..*at + extra)?;
        *at += extra;
        Some(raw.iter().fold(0u64, |v, b| (v << 8) | *b as u64))
    }
    if cbor.first() != Some(&0x85) {
        return None;
    }
    let mut at = 1;
    let _sequence = uint(cbor, &mut at)?;
    let parts = uint(cbor, &mut at)?;
    let length = uint(cbor, &mut at)?;
    Some((parts, length))
}

/// Collects scanned QR strings until the message is complete.
pub struct UrScan {
    expected: String,
    decoder: ur::Decoder,
    message: Option<Vec<u8>>,
    seen: std::collections::BTreeSet<usize>,
    of: usize,
}

impl UrScan {
    /// A scan expecting this UR type.
    pub fn new(expected: &str) -> Self {
        Self { expected: expected.to_owned(), decoder: ur::Decoder::default(), message: None, seen: Default::default(), of: 0 }
    }

    /// Feeds one scanned QR string in.
    pub fn receive(&mut self, raw: &str) -> Result<ScanProgress> {
        if self.message.is_none() {
            self.take(raw)?;
        }
        let cupcake = self.message.as_deref().and_then(|m| std::str::from_utf8(m).ok()).is_some_and(is_cupcake_pairing);
        Ok(ScanProgress { done: self.message.is_some(), seen: self.seen.len().min(self.of.max(1)), of: self.of.max(1), cupcake })
    }

    fn take(&mut self, raw: &str) -> Result<()> {
        // Cupcake shows its account as a plain QR, not a UR.
        let plain = raw.trim_matches(|c: char| c.is_whitespace() || c.is_control());
        if is_cupcake_pairing(plain) {
            if self.expected != UR_ACCOUNTS {
                return Err(Error::Other("that is Cupcake's code for connecting a wallet, not the signed transaction".into()));
            }
            parse_cupcake(plain)?;
            self.of = 1;
            self.seen.insert(1);
            self.message = Some(plain.as_bytes().to_vec());
            return Ok(());
        }
        let lowered = raw.to_ascii_lowercase();
        let Some(start) = lowered.find("ur:") else {
            return Err(Error::Other("that QR code isn't from Cupcake or Keystone".into()));
        };
        let text = lowered[start..].trim_matches(|c: char| c.is_whitespace() || c.is_control());
        let parts: Vec<&str> = text[3..].split('/').collect();
        if parts[0] != self.expected {
            return Err(Error::Other(if self.expected == UR_ACCOUNTS {
                format!("that is not the QR code for connecting a wallet (it is {}); on Cupcake or Keystone, show that QR code", parts[0])
            } else {
                format!("that is not the signed transaction (it is {})", parts[0])
            }));
        }
        match parts.len() {
            2 => {
                let (_, message) = ur::decode(text).map_err(|_| bad("the code is damaged"))?;
                self.of = 1;
                self.seen.insert(1);
                self.message = Some(message);
            }
            3 => {
                check_part_lengths(parts[1], text)?;
                if let Some((n, of)) = parts[1].split_once('-') {
                    if let (Ok(n), Ok(of)) = (n.parse::<usize>(), of.parse::<usize>()) {
                        self.of = of;
                        if n <= of {
                            self.seen.insert(n);
                        }
                    }
                }
                if self.decoder.receive(text).is_err() {
                    // A stray or mismatched part: start over on the next one.
                    self.decoder = ur::Decoder::default();
                    self.seen.clear();
                    return Ok(());
                }
                if self.decoder.complete() {
                    self.message = self.decoder.message().map_err(|_| bad("the moving code did not add up"))?;
                    self.seen = (1..=self.of).collect();
                }
            }
            _ => return Err(bad("the moving code's numbering")),
        }
        Ok(())
    }

    /// The whole message, once complete.
    pub fn message(&self) -> Option<&[u8]> {
        self.message.as_deref()
    }
}

#[cfg(test)]
mod cupcake_tests {
    use super::*;

    fn ufvk() -> String {
        let seed = [7u8; 32];
        let usk = zcash_keys::keys::UnifiedSpendingKey::from_seed(&zcash_protocol::consensus::MainNetwork, &seed, zip32::AccountId::ZERO).unwrap();
        usk.to_unified_full_viewing_key().encode(&zcash_protocol::consensus::MainNetwork).unwrap()
    }

    /// Cupcake's account QR is a plain "zcash-wallet:" URI with a viewing
    /// key, not a UR. Read for pairing, refused anywhere else.
    #[test]
    fn cupcakes_pairing_qr_is_read() {
        let key = ufvk();
        let qr = format!("zcash-wallet:?label=My%20Cupcake&private_key={key}");
        let mut scan = UrScan::new(UR_ACCOUNTS);
        let progress = scan.receive(&qr).unwrap();
        assert!(progress.done && progress.cupcake);
        let export = parse_accounts(scan.message().unwrap()).unwrap();
        assert!(export.from_cupcake());
        assert_eq!(export.seed_fingerprint, None);
        assert_eq!(export.accounts[0].ufvk, key);
        assert_eq!(export.accounts[0].name.as_deref(), Some("My Cupcake"));
        // Not where a signed payment is expected.
        assert!(UrScan::new(UR_PCZT).receive(&qr).is_err());
        // No label: named Cupcake.
        assert_eq!(parse_cupcake(&format!("zcash-wallet:?private_key={key}")).unwrap().accounts[0].name.as_deref(), Some("Cupcake"));
        // A broken key, no key, a testnet key, or another scheme: refused.
        for bad in [
            "zcash-wallet:?label=x&private_key=uview1notakey".to_string(),
            "zcash-wallet:?label=x".to_string(),
            "zcash-wallet:?private_key=uviewtest1qqqqqqqqqqqqqqqqqqqqqqqqqqq".to_string(),
            format!("zcash:?private_key={key}"),
        ] {
            assert!(parse_cupcake(&bad).is_err(), "{bad}");
        }
    }

    /// Cupcake's reader (Mac, 22 Nov 2026) failed with "Unable to handle ''
    /// UR tag": it needs "ur:type/1-1/…" even for one frame, in lower case.
    #[test]
    fn cupcake_gets_sequenced_lower_case_frames() {
        let small = vec![7u8; 40];
        let usual = ur_frames(UR_PCZT, &pczt_envelope(&small)).unwrap();
        assert!(usual[0].starts_with("UR:ZCASH-PCZT/") && usual.len() == 1, "Seal it and Keystone: as before");
        let frames = cupcake_frames(UR_PCZT, &pczt_envelope(&small)).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].starts_with("ur:zcash-pczt/1-1/"), "{}", frames[0]);
        let mut scan = UrScan::new(UR_PCZT);
        assert!(scan.receive(&frames[0]).unwrap().done);
        assert_eq!(parse_pczt(scan.message().unwrap()).unwrap(), small);
    }

    /// Cupcake finishes at exactly n distinct frames of n: only frames 1..n,
    /// and those n alone rebuild the message.
    #[test]
    fn cupcake_gets_only_the_plain_frames() {
        let big: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let frames = cupcake_frames(UR_PCZT, &pczt_envelope(&big)).unwrap();
        let n = frames.len();
        assert!(n > 1);
        for (i, frame) in frames.iter().enumerate() {
            assert!(frame.starts_with(&format!("ur:zcash-pczt/{}-{n}/", i + 1)), "{frame}");
        }
        let mut scan = UrScan::new(UR_PCZT);
        for frame in &frames {
            scan.receive(frame).unwrap();
        }
        assert_eq!(parse_pczt(scan.message().unwrap()).unwrap(), big);
    }
}

#[cfg(test)]
mod length_tests {
    use super::*;

    /// A part with this header: [seq, parts, length, checksum, b"ab"].
    fn part(seq: u64, parts: u64, length: u64) -> String {
        fn uint(out: &mut Vec<u8>, v: u64) {
            if v < 24 {
                out.push(v as u8);
            } else if v <= 0xff {
                out.extend([24, v as u8]);
            } else if v <= 0xffff {
                out.push(25);
                out.extend((v as u16).to_be_bytes());
            } else if v <= 0xffff_ffff {
                out.push(26);
                out.extend((v as u32).to_be_bytes());
            } else {
                out.push(27);
                out.extend(v.to_be_bytes());
            }
        }
        let mut cbor = vec![0x85];
        for v in [seq, parts, length, 7] {
            uint(&mut cbor, v);
        }
        cbor.extend([0x42, b'a', b'b']);
        format!("ur:zcash-pczt/{seq}-{parts}/{}", ur::bytewords::encode(&cbor, ur::bytewords::Style::Minimal))
    }

    /// Audit 22 Nov 2026 (Mac): 0 parts trapped the reader; a huge count or
    /// length allocated without limit.
    #[test]
    fn impossible_part_and_message_lengths_are_refused() {
        let mut scan = UrScan::new(UR_PCZT);
        for (seq, parts, length) in [(1, 0, 100), (1, 1_001, 100), (1, 5, 0), (1, 5, 10 * 1024 * 1024 + 1), (1, 5, u32::MAX as u64)] {
            let text = part(seq, parts, length);
            let text = if parts == 0 { text.replace("/1-0/", "/1-1/") } else { text };
            assert!(scan.receive(&text).is_err(), "{seq}-{parts}, {length} bytes must be refused");
        }
        assert!(scan.receive("ur:zcash-pczt/1-0/lpad").is_err(), "0 parts in the numbering");
        assert!(scan.receive("ur:zcash-pczt/1-99999999/lpad").is_err(), "too many parts in the numbering");
        assert_eq!(part_header(&[0x85, 1, 5, 25, 0x01, 0x00, 7]), Some((5, 256)));
    }

    #[test]
    fn a_normal_moving_code_still_reads() {
        let message: Vec<u8> = (0..900u32).map(|i| (i % 251) as u8).collect();
        let frames = ur_frames(UR_PCZT, &message).unwrap();
        assert!(frames.len() > 1);
        let mut scan = UrScan::new(UR_PCZT);
        for frame in frames.iter().cycle().take(frames.len() * 3) {
            if scan.receive(frame).unwrap().done {
                break;
            }
        }
        assert_eq!(scan.message(), Some(&message[..]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts_message() -> Vec<u8> {
        // {1: bytes(32), 2: [#49203 {1: "uview1...", 2: 0, 3: "Phone"}]}
        let mut m = vec![0xa2, 0x01, 0x58, 32];
        m.extend([7u8; 32]);
        m.extend([0x02, 0x81, 0xd9, 0xc0, 0x33, 0xa3, 0x01]);
        let ufvk = b"uview1test";
        m.extend(cbor_head(3, ufvk.len() as u64));
        m.extend(ufvk);
        m.extend([0x02, 0x00, 0x03, 0x65]);
        m.extend(b"Phone");
        m
    }

    #[test]
    fn accounts_are_read() {
        let export = parse_accounts(&accounts_message()).unwrap();
        assert_eq!(export.seed_fingerprint, Some([7; 32]));
        assert_eq!(export.accounts[0].ufvk, "uview1test");
        assert_eq!(export.accounts[0].index, 0);
        assert_eq!(export.accounts[0].name.as_deref(), Some("Phone"));
    }

    #[test]
    fn a_pczt_goes_round_through_animated_qr_codes() {
        let pczt: Vec<u8> = (0..3000u32).map(|i| (i * 7 % 251) as u8).collect();
        let frames = ur_frames(UR_PCZT, &pczt_envelope(&pczt)).unwrap();
        assert!(frames.len() > 2 && frames[0].starts_with("UR:ZCASH-PCZT/1-"));
        let mut scan = UrScan::new(UR_PCZT);
        // Start in the middle, skip some, as a camera would.
        for frame in frames.iter().skip(3).step_by(1).chain(frames.iter().take(3)) {
            if scan.receive(frame).unwrap().done {
                break;
            }
        }
        assert_eq!(parse_pczt(scan.message().unwrap()).unwrap(), pczt);
    }

    #[test]
    fn a_small_message_is_one_still_code() {
        let frames = ur_frames(UR_ACCOUNTS, &accounts_message()).unwrap();
        assert_eq!(frames.len(), 1);
        let mut scan = UrScan::new(UR_ACCOUNTS);
        assert!(scan.receive(&frames[0]).unwrap().done);
        assert_eq!(parse_accounts(scan.message().unwrap()).unwrap().accounts.len(), 1);
    }

    #[test]
    fn the_wrong_code_is_named() {
        let frames = ur_frames(UR_ACCOUNTS, &accounts_message()).unwrap();
        let mut scan = UrScan::new(UR_PCZT);
        assert!(scan.receive(&frames[0]).unwrap_err().to_string().contains("not the signed transaction"));
        assert!(scan.receive("https://example.com").is_err());
    }
}

#[cfg(test)]
mod keystone_scan_tests {
    use super::*;

    /// Keystone's moving codes: upper case, and after the first round only
    /// mixed (fountain) parts. Made with bc-ur (the library Keystone's apps
    /// use): a 700-byte message in 4 parts, of which only parts 5 to 14 are
    /// given here, none of them a plain part.
    const MIXED_PARTS: [&str; 10] = [
        "UR:ZCASH-PCZT/5-4/LPAHAACFAOSECYVWPMDMLUHDPAHNFPTSOLCTREIHENLSMEZMKOUELNLKSTJLDITKLUISUOPYHGLYPTLOVTCYMULOHEWNHFISJNATVOAETYIARFPSISMNSPMUMDCEFMECOXSWLNJPJZAHOLCSUOYAMUDLCXPFYKESPFIOZSCNNBVYRTUEVWFRRLQDRNRSFZOLWERPHNMNBBLUJKMELYZTCSVOJZNEADJZTOIOWFSADATYVYRKRNBZDMMEINHYTABEFLMDCHKKENHKJSVDQZSRPYASDIZETYFDFXDSDPGHLUBAVSKOKTKGROJTZTEHGDHGMOTYRLGDFPUTCYBWIDJNBNATGSRHBTZCWKJTHEFSSTBKSTTSCXJKLGJOLASWHTLYJEFX",
        "UR:ZCASH-PCZT/6-4/LPAMAACFAOSECYVWPMDMLUHDPACTNDTTRPYTHNMDIOYNLFBKPFTAAORNRNRPFMRDSKFRZSFERLVOIATIIHCHEHKIVYBYVDESISOYQDWNSFGLSOCYFYTYVDTBNSGALRSNMTBNOEKKHLYLNYRTMKVOZSWMCFSBJLCPASSPBKJSHGVSENCAFWBKMDRNLBZMTOAOGLFLHPAOKTURMSGEGTMEAYRSJYHGZMMKKERYFDJKJSLGETDMRLJKHNSOCEFTSPOXHKAOHKZEJKBDFSWPWTGURYMENLECHEBNEEWNHTNNURHKVSLTZSHYRKBBIALYHNPRGWTBOLFTLRCMYKAAIHAOZOOTNYFSMOJETPBZISLNRFDNOYHEDRPDSPJOLASWLDBEKKNL",
        "UR:ZCASH-PCZT/7-4/LPATAACFAOSECYVWPMDMLUHDPAOYADHKAORFFYHYRDVTSKCWPMBGWMNSNYKTWSEHLBTNAMBEVATLWTGYKPBWYKSWATLREYKKTACFKPGLGUDSWYVTIASGHDLPBTOEYKRNVTPAGYAHOLGYWNCSDPKPRPDWHTDLFEASGORDYAEYSGDKBDEHWZFNTPFYCYINSSESKGNYCWRHPEWTGMYLASYNSROSEHCPBTSEFZMNOXOTWNFRLKIAGHVEUYSFJNBEHLCSSPZEWKPRTNRKPAGHHKTAMDASIYGLHDKPIEBYQZCKMSGLLTFEGMGSBDFYMHCMMKRNSBLUFYKTTSKTSGGHHGAEWNLGDAAXKNNEPFDYVWUTAOBYIMSKSBWSCHATJLYLQZMHTAJT",
        "UR:ZCASH-PCZT/8-4/LPAYAACFAOSECYVWPMDMLUHDPASEFZMNOXOTWNFRLKIAGHVEUYSFJNBEHLCSSPZEWKPRTNRKPAGHHKTAMDASIYGLHDKPIEBYQZCKMSGLLTFEGMGSBDFYMHCMMKRNSBLUFYKTTSKTSGGHHGAEWNLGDAAXKNNEPFDYVWUTAOBYIMSKSBWSCHATJLYLOXTBLRNEMTDWKGEMRKKGCLIYLOBKUYFEHLRYBNPMMNWLHGHSTYWSJNTPWDWNYKHLAAGLLRAYMYJEVLSBWPVORTQDWECYFMAEFPPFLKFSDIEMNLGECEFZJLEODAEMQDDRJZDISPWLHKHEWFDIMTPKTIFLECJNZCLEINRDKTIDFYHYRDVTSKCWPMBGWMNSNYKTWSEHCHRSCKDI",
        "UR:ZCASH-PCZT/9-4/LPASAACFAOSECYVWPMDMLUHDPAUEUYHEBGHTMEPLWMMDTBWYJEBZJLPLVLPLYNFYEHLDCXZEAMRPFTASWTCKHGEORHIELSDEUORSDKRSGRBDNDHFGWMHKTRTAAYLGWFGTDKGKPBAMSOTSNRTINJLURVSIAGHURBGWPBZAYHNFSDPZCWZGOBTZSGAUYDTGENTTPJECXECSFOXRPDWSKNDTEZSDTWDWFECWZGHCTBGONIDGOYNHLLFMDMWCSJYGSPSTBINRDECNEWLZCHECAGALSMETPLPTEEHBWSWSRTYSRCFLTQZURINAYFMBSOLPDHPCMLDGOCABGRFDAFXGDJLAMDTWFLTVWASNSGRTDIYKKDYBNGTSEEEGMATJLYLTLFLCXWE",
        "UR:ZCASH-PCZT/10-4/LPBKAACFAOSECYVWPMDMLUHDPALBTNAMBEVATLWTGYKPBWYKSWATLREYKKTACFKPGLGUDSWYVTIASGHDLPBTOEYKRNVTPAGYAHOLGYWNCSDPKPRPDWHTDLFEASGORDYAEYSGDKBDEHWZFNTPFYCYINSSESKGNYCWRHPEWTGMYLASYNSROSEHCPBTSEFZMNOXOTWNFRLKIAGHVEUYSFJNBEHLCSSPZEWKPRTNRKPAGHHKTAMDASIYGLHDKPIEBYQZCKMSGLLTFEGMGSBDFYMHCMMKRNSBLUFYKTTSKTSGGHHGAEWNLGDAAXKNNEPFDYVWUTAOBYIMSKSBWSCHATJLYLOXTBLRNEMTDWKGEMRKKGCLIYLOBKUYFEAEAEAEAESSGMZM",
        "UR:ZCASH-PCZT/11-4/LPBDAACFAOSECYVWPMDMLUHDPAOYADHKAORFFYHYRDVTSKCWPMBGWMNSNYKTWSEHLBTNAMBEVATLWTGYKPBWYKSWATLREYKKTACFKPGLGUDSWYVTIASGHDLPBTOEYKRNVTPAGYAHOLGYWNCSDPKPRPDWHTDLFEASGORDYAEYSGDKBDEHWZFNTPFYCYINSSESKGNYCWRHPEWTGMYLASYNSROSEHCPBTSEFZMNOXOTWNFRLKIAGHVEUYSFJNBEHLCSSPZEWKPRTNRKPAGHHKTAMDASIYGLHDKPIEBYQZCKMSGLLTFEGMGSBDFYMHCMMKRNSBLUFYKTTSKTSGGHHGAEWNLGDAAXKNNEPFDYVWUTAOBYIMSKSBWSCHATJLYLCTLBINTD",
        "UR:ZCASH-PCZT/12-4/LPBNAACFAOSECYVWPMDMLUHDPAHNFPTSOLCTREIHENLSMEZMKOUELNLKSTJLDITKLUISUOPYHGLYPTLOVTCYMULOHEWNHFISJNATVOAETYIARFPSISMNSPMUMDCEFMECOXSWLNJPJZAHOLCSUOYAMUDLCXPFYKESPFIOZSCNNBVYRTUEVWFRRLQDRNRSFZOLWERPHNMNBBLUJKMELYZTCSVOJZNEADJZTOIOWFSADATYVYRKRNBZDMMEINHYTABEFLMDCHKKENHKJSVDQZSRPYASDIZETYFDFXDSDPGHLUBAVSKOKTKGROJTZTEHGDHGMOTYRLGDFPUTCYBWIDJNBNATGSRHBTZCWKJTHEFSSTBKSTTSCXJKLGJOLASWMNIDWSMN",
        "UR:ZCASH-PCZT/13-4/LPBTAACFAOSECYVWPMDMLUHDPARKGTGODTJLGSWYGDGTYTDNTBGYAYIHZOWMLSRPISREBWBGTBENLKRYRYZCRTLORFBZPTRYHNDMTPBGATOEDNTNYLESZCVSNSAYEEFPPYDNMDVTCHWMTNPEPYSTSNHDEOOSFDWDVTMEGOLFJOKBNSSNAHFHYAFXYKMTJYKPDWAXAHROMSCYLKVDHEKNMWDAAXROTOVDOLRKHDMDOXKIINHPOXLNOLTOMKAYPAKIFZKTCHPMGOVWUTMYFTBDETNSFRRTVYWPLPNBHEETMNPDWTPKMYVSMSGLGSSSINVDYKDMMWWTNBCASSYNHKTNRSRHWFYTPYBEFWBATTDTGSKKHFHFUOJEJLFPOESBCSTTOYYA",
        "UR:ZCASH-PCZT/14-4/LPBAAACFAOSECYVWPMDMLUHDPASEFZMNOXOTWNFRLKIAGHVEUYSFJNBEHLCSSPZEWKPRTNRKPAGHHKTAMDASIYGLHDKPIEBYQZCKMSGLLTFEGMGSBDFYMHCMMKRNSBLUFYKTTSKTSGGHHGAEWNLGDAAXKNNEPFDYVWUTAOBYIMSKSBWSCHATJLYLOXTBLRNEMTDWKGEMRKKGCLIYLOBKUYFEHLRYBNPMMNWLHGHSTYWSJNTPWDWNYKHLAAGLLRAYMYJEVLSBWPVORTQDWECYFMAEFPPFLKFSDIEMNLGECEFZJLEODAEMQDDRJZDISPWLHKHEWFDIMTPKTIFLECJNZCLEINRDKTIDFYHYRDVTSKCWPMBGWMNSNYKTWSEHFWFDSWKK",
    ];

    #[test]
    fn keystone_mixed_parts_alone_complete_the_message() {
        let mut scan = UrScan::new(UR_PCZT);
        let mut done_after = None;
        for (i, part) in MIXED_PARTS.iter().enumerate() {
            if scan.receive(part).unwrap().done {
                done_after = Some(i + 1);
                break;
            }
        }
        assert!(done_after.is_some(), "the mixed parts did not complete the message");
        let message = scan.message().unwrap();
        assert_eq!(message.len(), 705);
        assert_eq!(parse_pczt(message).unwrap().len(), 700);
    }

    #[test]
    fn a_part_read_twice_or_in_lower_case_is_fine() {
        let mut scan = UrScan::new(UR_PCZT);
        scan.receive(MIXED_PARTS[0]).unwrap();
        scan.receive(MIXED_PARTS[0]).unwrap();
        let mut done = false;
        for part in &MIXED_PARTS[1..] {
            done = scan.receive(&part.to_lowercase()).unwrap().done;
            if done {
                break;
            }
        }
        assert!(done);
    }

    #[test]
    fn a_pairing_scan_refuses_a_payment_code() {
        let mut scan = UrScan::new(UR_ACCOUNTS);
        assert!(scan.receive(MIXED_PARTS[0]).is_err());
    }
}

#[cfg(test)]
mod wallet_name_tests {
    use super::{keystone_may_not_read, wallet_name};

    #[test]
    fn keystone_gets_a_warning_over_14_kib_only() {
        assert!(!keystone_may_not_read(6_000));
        assert!(!keystone_may_not_read(14 * 1024));
        assert!(keystone_may_not_read(14 * 1024 + 1));
    }

    /// The same cases as Lock it Mac (testASignerWalletIsNamedAfterItsDevice).
    #[test]
    fn a_signer_wallet_is_named_after_its_device() {
        assert_eq!(wallet_name(Some("Account 0"), false), "Keystone · Account 0");
        assert_eq!(wallet_name(None, false), "Keystone");
        assert_eq!(wallet_name(Some("Keystone Main"), false), "Keystone Main");
        assert_eq!(wallet_name(Some("Cupcake"), true), "Cupcake");
        assert_eq!(wallet_name(Some("My wallet"), true), "Cupcake · My wallet");
        assert_eq!(wallet_name(Some(" "), true), "Cupcake");
        assert!(wallet_name(Some(&"x".repeat(80)), false).chars().count() <= 40);
    }
}
