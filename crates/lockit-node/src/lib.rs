//! Go Max: the owner's own pruned Zcash node, run quietly beside the wallet.
//!
//! Two upstream programs, pinned and unchanged, run as child processes of
//! Lock it:
//!
//! * **Zakura** (`zakurad`) is the full node: it talks to the Zcash network,
//!   checks every block and keeps a pruned copy of the chain.
//! * **Zaino** (`zainod`) reads from Zakura and serves wallets over the
//!   light-wallet protocol. Lock it, and later a paired phone, talk to Zaino.
//!
//! ```text
//! Lock it  ──gRPC──▶  Zaino  ──JSON-RPC (cookie)──▶  Zakura  ──P2P──▶  Zcash network
//!          127.0.0.1:8137        127.0.0.1:8232
//! ```
//!
//! This crate:
//!
//! * lays out the node's folders ([`NodeLayout`]),
//! * writes both config files from a few calm choices ([`GoMaxSettings`]),
//! * starts and stops both programs ([`GoMax`]), including the clean stop
//!   that Windows makes awkward (see [`process`]),
//! * reads the node's status for the Go Max page ([`NodeStatus`]).
//!
//! Nothing here holds keys or wallet data.

#![warn(missing_docs)]

mod config;
mod error;
mod layout;
pub mod process;
mod rpc;
mod supervisor;

pub use config::{GoMaxSettings, Listening};
pub use error::{Error, Result};
pub use layout::NodeLayout;
pub use rpc::{NodeRpc, NodeStatus};
pub use supervisor::{GoMax, NodePrograms, StopReport};
