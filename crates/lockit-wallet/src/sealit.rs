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
    /// The fingerprint of the phone's seed.
    pub seed_fingerprint: [u8; 32],
    /// Its accounts (the first is paired).
    pub accounts: Vec<SignerAccount>,
}

/// Reads `ur:zcash-accounts` (the Keystone registry format).
pub fn parse_accounts(message: &[u8]) -> Result<AccountsExport> {
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
    Ok(AccountsExport { seed_fingerprint, accounts })
}

/// Reads `ur:zcash-pczt` into the raw PCZT.
pub fn parse_pczt(message: &[u8]) -> Result<Vec<u8>> {
    let root = cbor_decode(message)?;
    root.get(1).and_then(Cbor::bytes).map(<[u8]>::to_vec).ok_or_else(|| bad("the signed transaction is missing"))
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

/// How far a scan is.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ScanProgress {
    /// Every part is in.
    pub done: bool,
    /// Parts seen so far.
    pub seen: usize,
    /// Parts in the message.
    pub of: usize,
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
        Ok(ScanProgress { done: self.message.is_some(), seen: self.seen.len().min(self.of.max(1)), of: self.of.max(1) })
    }

    fn take(&mut self, raw: &str) -> Result<()> {
        let lowered = raw.to_ascii_lowercase();
        let Some(start) = lowered.find("ur:") else {
            return Err(Error::Other("that QR code isn't from Seal it".into()));
        };
        let text = lowered[start..].trim_matches(|c: char| c.is_whitespace() || c.is_control());
        let parts: Vec<&str> = text[3..].split('/').collect();
        if parts[0] != self.expected {
            return Err(Error::Other(if self.expected == UR_ACCOUNTS {
                format!("that is not the account QR code (it is {}); in Seal it, show the account QR", parts[0])
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
        assert_eq!(export.seed_fingerprint, [7; 32]);
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
