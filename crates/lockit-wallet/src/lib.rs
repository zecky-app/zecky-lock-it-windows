//! Zecky Lock it: the wallet.
//!
//! Everything here is built on the official Zcash crates (librustzcash), in
//! the exact versions the Zecky engine pins: `zcash_client_backend` 0.25,
//! `zcash_client_sqlite` 0.23, `zcash_keys` 0.17, `zcash_proofs` 0.31. The
//! wallet calls are the same ones the engine's FFI makes for Lock it Mac
//! (opening the database, creating the account, proposing and building a
//! payment), with the same settings. What is new here is only the part the
//! Mac does in Swift: driving the sync, and talking to the server.
//!
//! The 24 words never live in this crate's files. The app keeps them sealed in
//! the lock (`lockit-auth`) and hands the seed in only to create the account
//! and to sign a payment.

#![warn(missing_docs)]

mod birthday;
mod cache;
mod net;
mod phrase;
mod rng_compat;
mod wallet;

pub use net::{is_private_host, Server, PUBLIC_SERVERS};
pub use phrase::{new_phrase, phrase_to_seed, validate_phrase, Seed};
pub use birthday::{estimate_birthday, SAPLING_ACTIVATION};
pub use wallet::{Activity, Balance, PreparedPayment, SignedPayment, Wallet};

/// Whether `address` is a Zcash mainnet address Lock it can pay.
pub fn is_valid_address(address: &str) -> bool {
    zcash_keys::address::Address::decode(&zcash_protocol::consensus::MainNetwork, address.trim()).is_some()
}

/// A payment request (ZIP 321, "zcash:<address>?amount=0.5&memo=..."), as
/// Lock it Mac reads it: one recipient only.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PaymentRequest {
    /// The address to pay.
    pub address: String,
    /// The amount asked for, in zatoshis.
    pub amount: Option<u64>,
    /// The note, if it is text.
    pub memo: Option<String>,
}

/// Reads a payment request. A request for several recipients is refused
/// instead of quietly paying only the first.
pub fn parse_payment_request(text: &str) -> Result<PaymentRequest> {
    let request = zcash_client_backend::zip321::TransactionRequest::from_uri(text.trim())
        .map_err(|_| Error::Address("that is not a Zcash payment request Lock it can read".into()))?;
    let payments = request.payments();
    if payments.len() != 1 {
        return Err(Error::Address("this request pays several recipients; Lock it sends to one at a time".into()));
    }
    let payment = payments.values().next().expect("one payment");
    let memo = match payment.memo() {
        Some(bytes) => match zcash_protocol::memo::Memo::try_from(bytes.clone()) {
            Ok(zcash_protocol::memo::Memo::Text(text)) => Some(text.to_string()),
            Ok(zcash_protocol::memo::Memo::Empty) => None,
            _ => return Err(Error::Address("the note in this payment request could not be read".into())),
        },
        None => None,
    };
    Ok(PaymentRequest {
        address: payment.recipient_address().encode(),
        amount: payment.amount().map(|a| a.into_u64()),
        memo,
    })
}

/// A QR code for `text`, as an SVG image (gold on ink, Lock it's colours).
pub fn qr_svg(text: &str) -> Result<String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).map_err(other)?;
    Ok(code
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#14161C"))
        .light_color(qrcode::render::svg::Color("#F4EFE3"))
        .quiet_zone(true)
        .build())
}

/// Result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong, in words the owner can read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The 24 words are not valid.
    #[error("these words are not a valid recovery phrase: {0}")]
    Phrase(String),
    /// The address is not a Zcash mainnet address.
    #[error("this is not a Zcash address Lock it can pay: {0}")]
    Address(String),
    /// Not enough funds that can be spent yet.
    #[error("not enough funds that can be spent yet: {0}")]
    InsufficientFunds(String),
    /// The server could not be reached or answered with an error.
    #[error("the server did not answer: {0}")]
    Server(String),
    /// Tor could not connect.
    #[error("Tor could not connect: {0}")]
    Tor(String),
    /// The wallet's files.
    #[error("the wallet file: {0}")]
    Database(String),
    /// There is no wallet yet.
    #[error("there is no wallet on this computer yet")]
    NoWallet,
    /// Anything else from the Zcash crates.
    #[error("{0}")]
    Other(String),
}

pub(crate) fn other(error: impl std::fmt::Display) -> Error {
    Error::Other(error.to_string())
}

#[cfg(test)]
mod request_tests {
    use super::*;

    fn an_address() -> String {
        use secrecy::ExposeSecret as _;
        let seed = phrase_to_seed(&new_phrase()).unwrap();
        let usk = zcash_keys::keys::UnifiedSpendingKey::from_seed(
            &zcash_protocol::consensus::MainNetwork,
            seed.expose_secret(),
            zip32::AccountId::ZERO,
        )
        .unwrap();
        let (ua, _) = usk
            .to_unified_full_viewing_key()
            .default_address(zcash_keys::keys::UnifiedAddressRequest::SHIELDED)
            .unwrap();
        ua.encode(&zcash_protocol::consensus::MainNetwork)
    }

    #[test]
    fn a_payment_request_fills_in_address_amount_and_note() {
        let address = an_address();
        #[allow(non_snake_case)]
        let ADDR = address.as_str();
        assert!(is_valid_address(ADDR));
        assert!(!is_valid_address("not an address"));
        let r = parse_payment_request(&format!("zcash:{ADDR}?amount=0.25&memo=SGVsbG8")).unwrap();
        assert_eq!(r.address, ADDR);
        assert_eq!(r.amount, Some(25_000_000));
        assert_eq!(r.memo.as_deref(), Some("Hello"));
        let two = format!("zcash:?address={ADDR}&amount=1&address.1={ADDR}&amount.1=2");
        assert!(parse_payment_request(&two).is_err(), "several recipients are refused");
        assert!(parse_payment_request("hello").is_err());
    }
}
