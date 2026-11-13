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

pub use net::{Server, PUBLIC_SERVERS};
pub use phrase::{new_phrase, phrase_to_seed, validate_phrase, Seed};
pub use birthday::{estimate_birthday, SAPLING_ACTIVATION};
pub use wallet::{Activity, Balance, PreparedPayment, SignedPayment, Wallet};

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
