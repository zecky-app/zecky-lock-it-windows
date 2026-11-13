//! The wallet on disk: one account, the same calls the Zecky engine makes.

use std::{
    convert::Infallible,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use secrecy::ExposeSecret;
use serde::Serialize;
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_proposed_transactions, input_selection::GreedyInputSelector, propose_transfer,
            ConfirmationsPolicy, SpendingKeys,
        },
        Account as _, AccountBirthday, WalletRead, WalletWrite,
    },
    fees::{standard::MultiOutputChangeStrategy, DustOutputPolicy, SplitPolicy, StandardFeeRule},
    proposal::Proposal,
    wallet::OvkPolicy,
    zip321::{Payment, TransactionRequest},
};
use zcash_client_sqlite::{
    util::SystemClock, wallet::init::init_wallet_db, AccountUuid, ReceivedNoteId, WalletDb,
};
use zcash_keys::keys::{ReceiverRequirement, UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::{
    consensus::{BlockHeight, MainNetwork},
    memo::MemoBytes,
    value::Zatoshis,
    ShieldedPool,
};

use crate::{cache::MemoryCache, net, other, rng_compat::OsRng, Error, Result, Seed, Server};

type Db = WalletDb<rusqlite::Connection, MainNetwork, SystemClock, OsRng>;

/// What the home screen shows, in zatoshis.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Balance {
    /// Can be sent now.
    pub spendable: u64,
    /// Arrived, waiting for confirmations.
    pub pending: u64,
    /// Everything the wallet holds.
    pub total: u64,
    /// The newest block the wallet has looked at.
    pub scanned_height: u32,
    /// The newest block the server reported.
    pub chain_tip: u32,
    /// How far the scan is, from 0 to 1.
    pub progress: f64,
    /// The block the wallet starts from.
    pub birthday: u32,
    /// Block ranges still to check, `[start, end)`, as the Mac's sync ring
    /// counts them.
    pub to_scan: Vec<(u32, u32)>,
    /// In the old Orchard pool: withdraw-only since NU6.3, "Move it to
    /// Ironwood" on home.
    pub orchard: u64,
    /// In the Ironwood pool, where new Zcash goes since NU6.3.
    pub ironwood: u64,
    /// In the Sapling pool.
    pub sapling: u64,
    /// On a transparent address, outside the lock: "Lock it in" on home.
    pub transparent: u64,
}

/// A payment worked out and ready to sign, with its fee.
pub struct PreparedPayment {
    proposal: Planned,
    /// What the receiver gets.
    pub amount: u64,
    /// The network fee.
    pub fee: u64,
    /// Where it goes.
    pub to: String,
}

/// A worked-out payment: a normal one (shielded inputs), or locking in
/// transparent Zcash (transparent inputs only).
enum Planned {
    Notes(Proposal<StandardFeeRule, ReceivedNoteId>),
    Shielding(Proposal<StandardFeeRule, Infallible>),
}

/// The wallet in one folder: `wallet.db` and, for sending over Tor, `tor\`.
pub struct Wallet {
    dir: PathBuf,
}

impl Wallet {
    /// The wallet in `dir`. Nothing is created until [`Wallet::create`].
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("wallet.db")
    }

    fn tor_dir(&self) -> PathBuf {
        self.dir.join("tor")
    }

    /// Opens the database the way the engine does: a busy timeout, because
    /// more than one connection may be open, and the array module.
    fn db(&self) -> Result<Db> {
        let conn = rusqlite::Connection::open(self.db_path()).map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(15)).map_err(db_err)?;
        rusqlite::vtab::array::load_module(&conn).map_err(db_err)?;
        Ok(WalletDb::from_connection(conn, MainNetwork, SystemClock, OsRng))
    }

    fn account(&self, db: &Db) -> Result<AccountUuid> {
        db.get_account_ids().map_err(db_err)?.into_iter().next().ok_or(Error::NoWallet)
    }

    /// Whether a wallet exists here.
    pub fn exists(&self) -> bool {
        self.db_path().is_file() && self.db().and_then(|db| self.account(&db)).is_ok()
    }

    /// Creates the wallet for `seed`, scanning from `birthday` (a block
    /// height). For a new wallet the app passes the chain tip; for a restore,
    /// the estimate from the month the words were made.
    pub async fn create(&self, seed: &Seed, birthday: Option<u32>, server: &Server) -> Result<()> {
        if self.exists() {
            return Err(Error::Database("a wallet already exists here".into()));
        }
        let mut client = server.connect().await?;
        let tip = net::tip(&mut client).await?;
        // The tree state is taken one block before the birthday, as the SDKs do.
        let height = birthday.unwrap_or(tip).min(tip).max(SAPLING_ACTIVATION) - 1;
        let tree = net::tree_state(&mut client, height).await?;
        drop(client);
        let birthday = AccountBirthday::from_treestate(tree, None).map_err(|_| other("the server sent an unusable tree state"))?;
        self.create_from_birthday(seed, &birthday, tip)
    }

    /// Creates the database and the account from a known birthday.
    fn create_from_birthday(&self, seed: &Seed, birthday: &AccountBirthday, tip: u32) -> Result<()> {
        if self.exists() {
            return Err(Error::Database("a wallet already exists here".into()));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| Error::Database(e.to_string()))?;
        let mut db = self.db()?;
        init_wallet_db(&mut db, Some(secrecy::SecretVec::new(seed.expose_secret().clone()))).map_err(other)?;
        db.create_account("Lock it", seed, birthday, None).map_err(db_err)?;
        // The wallet needs to know the chain's height to make an address.
        db.update_chain_tip(BlockHeight::from_u32(tip)).map_err(db_err)?;
        Ok(())
    }

    /// Downloads and scans everything new. Returns when the wallet is up to date.
    pub async fn sync(&self, server: &Server) -> Result<()> {
        let mut client = server.connect().await?;
        let cache = MemoryCache::default();
        let mut db = self.db()?;
        zcash_client_backend::sync::run(&mut client, &MainNetwork, &cache, &mut db, 1_000)
            .await
            .map_err(|e| Error::Server(e.to_string()))
    }

    /// The balance and how far the scan is.
    pub fn balance(&self) -> Result<Balance> {
        let db = self.db()?;
        let account = self.account(&db)?;
        let Some(summary) = db.get_wallet_summary(ConfirmationsPolicy::default()).map_err(db_err)? else {
            return Ok(Balance::default());
        };
        let progress = summary.progress();
        let scan = progress.scan();
        let fraction = if *scan.denominator() == 0 { 0.0 } else { *scan.numerator() as f64 / *scan.denominator() as f64 };
        let mut balance = Balance {
            scanned_height: summary.fully_scanned_height().into(),
            chain_tip: summary.chain_tip_height().into(),
            progress: fraction.clamp(0.0, 1.0),
            ..Default::default()
        };
        balance.birthday = db.get_account_birthday(account).map(u32::from).unwrap_or(0);
        balance.to_scan = db
            .suggest_scan_ranges()
            .map_err(db_err)?
            .iter()
            .map(|range| (u32::from(range.block_range().start), u32::from(range.block_range().end)))
            .collect();
        if let Some(account_balance) = summary.account_balances().get(&account) {
            balance.spendable = account_balance.spendable_value().into_u64();
            balance.total = account_balance.total().into_u64();
            balance.pending = balance.total.saturating_sub(balance.spendable);
            balance.orchard = account_balance.orchard_balance().total().into_u64();
            balance.ironwood = account_balance.ironwood_balance().total().into_u64();
            balance.sapling = account_balance.sapling_balance().total().into_u64();
            balance.transparent = account_balance.unshielded_balance().total().into_u64();
        }
        Ok(balance)
    }

    /// The shielded address to receive on (Orchard and Sapling, no transparent).
    ///
    /// Shows the newest address made so far, so the screen does not create a
    /// new one every time it opens.
    pub fn receive_address(&self) -> Result<String> {
        let mut db = self.db()?;
        let account = self.account(&db)?;
        let request = UnifiedAddressRequest::unsafe_custom(
            ReceiverRequirement::Require,
            ReceiverRequirement::Require,
            ReceiverRequirement::Omit,
        );
        if let Some(address) = db.get_last_generated_address_matching(account, request).map_err(db_err)? {
            return Ok(address.encode(&MainNetwork));
        }
        let (address, _) = db
            .get_next_available_address(account, request)
            .map_err(db_err)?
            .ok_or_else(|| other("no address could be made"))?;
        Ok(address.encode(&MainNetwork))
    }

    /// Works out a payment and its fee. Nothing is signed or sent.
    ///
    /// Same fee rule, change and input selection as the engine (ZIP 317,
    /// change to Orchard).
    pub fn prepare(&self, to: &str, amount: u64, memo: Option<&str>) -> Result<PreparedPayment> {
        let mut db = self.db()?;
        let account = self.account(&db)?;
        let address: zcash_address::ZcashAddress = to.trim().parse().map_err(|e| Error::Address(format!("{e}")))?;
        let value = Zatoshis::from_u64(amount).map_err(|_| Error::Address("the amount is not valid".into()))?;
        let memo = match memo.filter(|m| !m.trim().is_empty()) {
            Some(text) => Some(MemoBytes::from_bytes(text.as_bytes()).map_err(|_| Error::Address("the note is too long (512 bytes at most)".into()))?),
            None => None,
        };
        let payment = Payment::new(address, Some(value), memo, None, None, vec![])
            .map_err(|_| Error::Address("this address cannot take a note".into()))?;
        let request = TransactionRequest::new(vec![payment]).map_err(|e| Error::Address(e.to_string()))?;

        let change = MultiOutputChangeStrategy::new(
            StandardFeeRule::Zip317,
            None,
            ShieldedPool::Orchard,
            DustOutputPolicy::default(),
            SplitPolicy::with_min_output_value(NonZeroUsize::new(4).unwrap(), Zatoshis::const_from_u64(1000_0000)),
        );
        let selector = GreedyInputSelector::new();
        let proposal = propose_transfer::<_, _, _, _, Infallible>(
            &mut db,
            &MainNetwork,
            account,
            &selector,
            &change,
            request,
            ConfirmationsPolicy::default(),
            &Default::default(),
            None,
            None,
        )
        .map_err(|e| {
            let text = e.to_string();
            if text.to_lowercase().contains("insufficient") { Error::InsufficientFunds(text) } else { other(text) }
        })?;
        let fee: u64 = proposal.steps().iter().map(|step| step.balance().fee_required().into_u64()).sum();
        Ok(PreparedPayment { proposal: Planned::Notes(proposal), amount, fee, to: to.trim().to_owned() })
    }

    /// "Move it to Ironwood": everything in the old Orchard pool, to this
    /// wallet's own internal Orchard receiver, which since NU6.3 lands in
    /// Ironwood. The same call as the Zecky engine's turnstile
    /// (`propose_orchard_to_ironwood`): the maximum, fee included, so
    /// nothing is left behind. Sapling and transparent funds are untouched.
    pub fn prepare_ironwood_move(&self) -> Result<PreparedPayment> {
        use zcash_address::{unified, unified::Encoding as _, ToAddress as _, ZcashAddress};
        use zcash_client_backend::data_api::{wallet::input_selection::LockedInputPolicy, MaxSpendMode};
        let mut db = self.db()?;
        let account = self.account(&db)?;
        let orchard_fvk = db
            .get_account(account)
            .map_err(db_err)?
            .ok_or(Error::NoWallet)?
            .ufvk()
            .and_then(|ufvk| ufvk.orchard())
            .cloned()
            .ok_or_else(|| other("this wallet has no Orchard key"))?;
        // Internal scope: the funds come back as change, not as a payment.
        let receiver = orchard_fvk.address_at(0u32, orchard::keys::Scope::Internal);
        let recipient = ZcashAddress::from_unified(
            zcash_protocol::consensus::NetworkType::Main,
            unified::Address::try_from_items(
                unified::Revision::R0,
                vec![unified::Uitem::Data(unified::Receiver::Orchard(receiver.to_raw_address_bytes()))],
            )
            .map_err(other)?,
        );
        let proposal = zcash_client_backend::data_api::wallet::propose_send_max_transfer::<_, _, _, Infallible>(
            &mut db,
            &MainNetwork,
            account,
            &[ShieldedPool::Orchard],
            &StandardFeeRule::Zip317,
            recipient,
            None,
            MaxSpendMode::Everything,
            ConfirmationsPolicy::default(),
            &LockedInputPolicy::Exclude,
            None,
        )
        .map_err(|e| {
            let text = e.to_string();
            if text.to_lowercase().contains("insufficient") { Error::InsufficientFunds(text) } else { other(text) }
        })?;
        let fee: u64 = proposal.steps().iter().map(|step| step.balance().fee_required().into_u64()).sum();
        let held = self.balance()?.orchard;
        Ok(PreparedPayment { proposal: Planned::Notes(proposal), amount: held.saturating_sub(fee), fee, to: "Ironwood".into() })
    }

    /// "Lock it in": every transparent Zcash of this wallet into its own
    /// shielded pool, as the engine's `propose_shielding`.
    pub fn prepare_lock_in(&self) -> Result<PreparedPayment> {
        use zcash_client_backend::data_api::CoinbaseFilter;
        let mut db = self.db()?;
        let account = self.account(&db)?;
        let from: Vec<_> = db.get_transparent_receivers(account, true, false).map_err(db_err)?.into_keys().collect();
        if from.is_empty() {
            return Err(Error::InsufficientFunds("there is no transparent Zcash to lock in".into()));
        }
        let change = MultiOutputChangeStrategy::new(
            StandardFeeRule::Zip317,
            None,
            ShieldedPool::Orchard,
            DustOutputPolicy::default(),
            SplitPolicy::with_min_output_value(NonZeroUsize::new(4).unwrap(), Zatoshis::const_from_u64(1000_0000)),
        );
        let selector = GreedyInputSelector::new();
        let proposal = zcash_client_backend::data_api::wallet::propose_shielding::<_, _, _, _, Infallible>(
            &mut db,
            &MainNetwork,
            &selector,
            &change,
            Zatoshis::const_from_u64(10_000),
            &from,
            account,
            ConfirmationsPolicy::default(),
            CoinbaseFilter::AllTransparentOutputs,
            None,
        )
        .map_err(|e| {
            let text = e.to_string();
            if text.to_lowercase().contains("insufficient") { Error::InsufficientFunds(text) } else { other(text) }
        })?;
        let fee: u64 = proposal.steps().iter().map(|step| step.balance().fee_required().into_u64()).sum();
        let held = self.balance()?.transparent;
        Ok(PreparedPayment { proposal: Planned::Shielding(proposal), amount: held.saturating_sub(fee), fee, to: "Lock it in".into() })
    }

    /// Signs a prepared payment with the seed. The signed payment is stored
    /// in the wallet (its funds count as spent) but not yet sent: hand it to
    /// the network with [`Wallet::broadcast`]. If that fails, the same signed
    /// payment can be handed over again; nothing is paid twice.
    ///
    /// `sapling_params` holds `sapling-spend.params` and
    /// `sapling-output.params`, shipped with the app.
    pub fn sign(&self, payment: &PreparedPayment, seed: &Seed, sapling_params: &Path) -> Result<SignedPayment> {
        let mut db = self.db()?;
        let usk = UnifiedSpendingKey::from_seed(&MainNetwork, seed.expose_secret(), zip32::AccountId::ZERO).map_err(other)?;
        let account = self.account(&db)?;
        let derived = db.get_account(account).map_err(db_err)?.ok_or(Error::NoWallet)?;
        if derived.ufvk().map(|k| k.encode(&MainNetwork)) != Some(usk.to_unified_full_viewing_key().encode(&MainNetwork)) {
            return Err(Error::Phrase("the words in the lock do not belong to this wallet".into()));
        }
        let spend = sapling_params.join("sapling-spend.params");
        let output = sapling_params.join("sapling-output.params");
        if !spend.is_file() || !output.is_file() {
            return Err(Error::Other("the Sapling files that come with Lock it are missing".into()));
        }
        let prover = LocalTxProver::new(&spend, &output);
        let keys = SpendingKeys::from_unified_spending_key(usk);
        let txids: Vec<_> = match &payment.proposal {
            Planned::Notes(proposal) => create_proposed_transactions::<_, _, Infallible, _, Infallible, _>(
                &mut db, &MainNetwork, &SystemClock, &mut OsRng, &prover, &prover, &keys, OvkPolicy::Sender, proposal, None,
            )
            .map_err(other)?
            .into_iter()
            .collect(),
            Planned::Shielding(proposal) => create_proposed_transactions::<_, _, Infallible, _, Infallible, _>(
                &mut db, &MainNetwork, &SystemClock, &mut OsRng, &prover, &prover, &keys, OvkPolicy::Sender, proposal, None,
            )
            .map_err(other)?
            .into_iter()
            .collect(),
        };
        let mut signed = SignedPayment { txids: Vec::new(), raw: Vec::new() };
        for txid in txids.iter() {
            let tx = db.get_transaction(*txid).map_err(db_err)?.ok_or_else(|| other("the signed payment was not stored"))?;
            let mut bytes = Vec::new();
            tx.write(&mut bytes).map_err(other)?;
            signed.txids.push(txid.to_string());
            signed.raw.push(bytes);
        }
        Ok(signed)
    }

    /// Hands a signed payment to the network: over Tor to two public servers
    /// (the default, as on Lock it Mac), or directly to `reading_from`.
    /// Returns the servers that accepted it.
    pub async fn broadcast(&self, signed: &SignedPayment, reading_from: &Server, over_tor: bool) -> Result<Vec<String>> {
        let mut accepted_by = Vec::new();
        for bytes in &signed.raw {
            accepted_by = net::submit(bytes, reading_from, over_tor, &self.tor_dir()).await?;
        }
        Ok(accepted_by)
    }

    /// The newest payments in and out, newest first.
    pub fn activity(&self, limit: u32) -> Result<Vec<Activity>> {
        let conn = rusqlite::Connection::open(self.db_path()).map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(15)).map_err(db_err)?;
        let mut statement = conn
            .prepare(
                "SELECT txid, mined_height, account_balance_delta, block_time, expired_unmined
                 FROM v_transactions
                 ORDER BY (mined_height IS NULL) DESC, mined_height DESC
                 LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map([limit], |row| {
                let txid: Vec<u8> = row.get(0)?;
                Ok(Activity {
                    // Shown the way explorers show it: bytes reversed.
                    txid: txid.iter().rev().map(|b| format!("{b:02x}")).collect(),
                    height: row.get::<_, Option<u32>>(1)?,
                    change: row.get(2)?,
                    time: row.get::<_, Option<i64>>(3)?,
                    expired: row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                })
            })
            .map_err(db_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_err)
    }

    /// The wallet's unified full viewing key: sees everything, spends nothing.
    pub fn viewing_key(&self) -> Result<String> {
        let db = self.db()?;
        let account = db.get_account(self.account(&db)?).map_err(db_err)?.ok_or(Error::NoWallet)?;
        account.ufvk().ok_or_else(|| other("this wallet has no viewing key"))?.encode(&MainNetwork).map_err(other)
    }

    /// The newest block height the server knows.
    pub async fn chain_tip(server: &Server) -> Result<u32> {
        let mut client = server.connect().await?;
        net::tip(&mut client).await
    }
}

/// A payment that has been signed but perhaps not yet handed to the network.
pub struct SignedPayment {
    /// Transaction id(s), as shown by block explorers.
    pub txids: Vec<String>,
    raw: Vec<Vec<u8>>,
}

/// One line of the wallet's history.
#[derive(Clone, Debug, Serialize)]
pub struct Activity {
    /// Transaction id, as shown by block explorers.
    pub txid: String,
    /// The block it is in; `None` while waiting.
    pub height: Option<u32>,
    /// What it did to the balance, in zatoshis (negative: paid out, fee included).
    pub change: i64,
    /// When its block was made (Unix seconds).
    pub time: Option<i64>,
    /// It never made it into a block and has expired.
    pub expired: bool,
}

use crate::SAPLING_ACTIVATION;

fn db_err(error: impl std::fmt::Display) -> Error {
    Error::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use zcash_primitives::block::BlockHash;

    use super::*;
    use crate::{new_phrase, phrase_to_seed};

    fn fresh() -> (tempfile::TempDir, Wallet, Seed) {
        let dir = tempfile::tempdir().unwrap();
        let wallet = Wallet::new(dir.path().join("wallet"));
        let seed = phrase_to_seed(&new_phrase()).unwrap();
        let birthday = AccountBirthday::from_parts(zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION - 1), BlockHash([0; 32])), None);
        wallet.create_from_birthday(&seed, &birthday, SAPLING_ACTIVATION + 10).unwrap();
        (dir, wallet, seed)
    }

    #[test]
    fn a_new_wallet_has_one_account_a_stable_address_and_no_funds() {
        let (_dir, wallet, _seed) = fresh();
        assert!(wallet.exists());
        let first = wallet.receive_address().unwrap();
        assert!(first.starts_with("u1"), "{first}");
        assert_eq!(wallet.receive_address().unwrap(), first, "opening Receive again shows the same address");
        assert_eq!(wallet.balance().unwrap().spendable, 0);
    }

    #[test]
    fn paying_without_funds_is_refused_before_anything_is_signed() {
        let (_dir, wallet, _seed) = fresh();
        let to = wallet.receive_address().unwrap();
        assert!(wallet.prepare(&to, 10_000, Some("hello")).is_err());
        assert!(matches!(wallet.prepare("not an address", 1, None), Err(Error::Address(_))));
    }

    #[test]
    fn a_new_wallet_has_no_activity_and_its_address_makes_a_qr_code() {
        let (_dir, wallet, _seed) = fresh();
        assert!(wallet.activity(20).unwrap().is_empty());
        assert!(wallet.viewing_key().unwrap().starts_with("uview1"));
        let svg = crate::qr_svg(&wallet.receive_address().unwrap()).unwrap();
        assert!(svg.starts_with("<?xml") && svg.contains("#14161C"));
    }

    #[test]
    fn the_internal_moves_refuse_calmly_on_an_empty_wallet() {
        let (_dir, wallet, _seed) = fresh();
        let b = wallet.balance().unwrap();
        assert_eq!((b.orchard, b.ironwood, b.sapling, b.transparent), (0, 0, 0, 0));
        assert!(wallet.prepare_ironwood_move().is_err(), "nothing in Orchard to move");
        assert!(wallet.prepare_lock_in().is_err(), "nothing transparent to lock in");
        assert!(wallet.balance().is_ok(), "the wallet is unchanged");
    }

    #[test]
    fn a_wallet_is_never_created_twice() {
        let (_dir, wallet, seed) = fresh();
        let birthday = AccountBirthday::from_parts(zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION - 1), BlockHash([0; 32])), None);
        assert!(wallet.create_from_birthday(&seed, &birthday, SAPLING_ACTIVATION + 10).is_err());
    }
}

