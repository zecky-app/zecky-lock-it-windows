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
    /// The block the shared scan starts from: the earliest start of all
    /// wallets on this PC (they are all checked in one scan).
    pub scan_from: u32,
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
    account: AccountUuid,
    /// What the receiver gets.
    pub amount: u64,
    /// The network fee.
    pub fee: u64,
    /// Where it goes.
    pub to: String,
}

impl PreparedPayment {
    /// The block height the payment was worked out for. Its "valid until"
    /// block follows from this when it is signed.
    pub fn target_height(&self) -> u32 {
        match &self.proposal {
            Planned::Notes(proposal) => proposal.min_target_height().into(),
            Planned::Shielding(proposal) => proposal.min_target_height().into(),
        }
    }
}

/// A worked-out payment: a normal one (shielded inputs), or locking in
/// transparent Zcash (transparent inputs only).
enum Planned {
    Notes(Proposal<StandardFeeRule, ReceivedNoteId>),
    Shielding(Proposal<StandardFeeRule, Infallible>),
}

/// The Orchard circuit of the PCZT's network upgrade (Ironwood shares it).
fn circuit_version(pczt: &pczt::Pczt) -> Result<orchard::circuit::OrchardCircuitVersion> {
    let branch = zcash_protocol::consensus::BranchId::try_from(*pczt.global().consensus_branch_id())
        .map_err(|_| other("the payment is for an unknown network upgrade"))?;
    zcash_primitives::transaction::components::orchard::bundle_version_for_branch(branch, ::orchard::ValuePool::Orchard)
        .map(|v| v.circuit_version())
        .ok_or_else(|| other("this network upgrade has no Orchard"))
}

/// Runs `work` for at most `budget`: Ok(true) when it finished, Ok(false)
/// when time ran out first.
pub async fn within<F: std::future::Future<Output = Result<()>>>(work: F, budget: std::time::Duration) -> Result<bool> {
    match tokio::time::timeout(budget, work).await {
        Ok(done) => done.map(|()| true),
        Err(_) => Ok(false),
    }
}

/// The wallet in one folder: `wallet.db` and, for sending over Tor, `tor\`.
#[derive(Clone)]
pub struct Wallet {
    dir: PathBuf,
    /// The wallet on the key ring this handle works on (an account id);
    /// `None` is the main wallet, the first one made.
    selected: Option<String>,
}

/// One wallet on the key ring.
#[derive(Clone, Debug, Serialize)]
pub struct AccountInfo {
    /// Its id (a UUID).
    pub id: String,
    /// The name it was added under.
    pub name: String,
    /// The block it starts from.
    pub birthday: u32,
    /// Watch-only: signed elsewhere (Seal it), never on this PC.
    pub watch_only: bool,
    /// The main wallet, the one whose words are sealed in the lock.
    pub main: bool,
}

/// Key source marking extra wallets with their own words on this PC.
pub const KEYRING_SOURCE: &str = "lockit-keyring";
/// Key source marking Seal it wallets (watch-only on this PC).
pub const SEALIT_SOURCE: &str = "sealit";
/// Where a Cupcake wallet starts looking: NU7 (Cupcake added Zcash after it).
const CUPCAKE_BIRTHDAY: u32 = 3_543_405;

impl Wallet {
    /// The wallet in `dir`. Nothing is created until [`Wallet::create`].
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), selected: None }
    }

    /// The same wallet folder, working on another wallet of the key ring.
    pub fn for_account(&self, id: Option<&str>) -> Self {
        Self { dir: self.dir.clone(), selected: id.map(str::to_owned) }
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

    fn main_account(db: &Db) -> Result<AccountUuid> {
        let ids = db.get_account_ids().map_err(db_err)?;
        // The main wallet is the one made without a key source; the first
        // one if, somehow, there are several.
        for id in &ids {
            if let Some(account) = db.get_account(*id).map_err(db_err)? {
                if account.source().key_source().is_none() {
                    return Ok(*id);
                }
            }
        }
        ids.into_iter().next().ok_or(Error::NoWallet)
    }

    fn account(&self, db: &Db) -> Result<AccountUuid> {
        match &self.selected {
            None => Self::main_account(db),
            Some(id) => {
                let uuid = uuid::Uuid::parse_str(id).map_err(|_| Error::Other("unknown wallet".into()))?;
                let account = AccountUuid::from_uuid(uuid);
                if db.get_account_ids().map_err(db_err)?.contains(&account) { Ok(account) } else { Err(Error::NoWallet) }
            }
        }
    }

    /// Every wallet on the key ring, the main one first.
    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let db = self.db()?;
        let main = Self::main_account(&db)?;
        let mut list = Vec::new();
        for id in db.get_account_ids().map_err(db_err)? {
            let Some(account) = db.get_account(id).map_err(db_err)? else { continue };
            let source = account.source().key_source().map(str::to_owned);
            list.push(AccountInfo {
                id: id.expose_uuid().to_string(),
                name: account.name().unwrap_or("Wallet").to_owned(),
                birthday: db.get_account_birthday(id).map(u32::from).unwrap_or(0),
                watch_only: source.as_deref() == Some(SEALIT_SOURCE),
                main: id == main,
            });
        }
        list.sort_by_key(|a| !a.main);
        Ok(list)
    }

    async fn birthday_from(server: &Server, birthday: Option<u32>) -> Result<(AccountBirthday, u32)> {
        let mut client = server.connect().await?;
        let tip = net::tip(&mut client).await?;
        let height = birthday.unwrap_or(tip).min(tip).max(SAPLING_ACTIVATION) - 1;
        let tree = net::tree_state(&mut client, height).await?;
        let birthday = AccountBirthday::from_treestate(tree, None).map_err(|_| other("the server sent an unusable tree state"))?;
        Ok((birthday, tip))
    }

    /// Adds another wallet with its own words to the key ring. Returns its id.
    pub async fn add_wallet(&self, name: &str, seed: &Seed, birthday: Option<u32>, server: &Server) -> Result<String> {
        let (birthday, tip) = Self::birthday_from(server, birthday).await?;
        self.add_wallet_from_birthday(name, seed, &birthday, tip)
    }

    fn add_wallet_from_birthday(&self, name: &str, seed: &Seed, birthday: &AccountBirthday, tip: u32) -> Result<String> {
        let mut db = self.db()?;
        let (account, _) = db
            .import_account_hd(name, seed, zip32::AccountId::ZERO, birthday, Some(KEYRING_SOURCE))
            .map_err(|e| {
                let text = e.to_string();
                if text.to_lowercase().contains("already") || text.to_lowercase().contains("unique") {
                    Error::Database("this wallet is already on the key ring".into())
                } else {
                    db_err(text)
                }
            })?;
        db.update_chain_tip(BlockHeight::from_u32(tip)).map_err(db_err)?;
        Ok(account.id().expose_uuid().to_string())
    }

    /// Pairs a Seal it phone: imports its first account's viewing key (the
    /// spending keys stay on the phone). Returns the wallet's id; the same
    /// phone account is never added twice.
    pub async fn add_sealit(&self, name: &str, export: &crate::sealit::AccountsExport, birthday: Option<u32>, server: &Server) -> Result<String> {
        use zcash_client_backend::data_api::{AccountPurpose, Zip32Derivation};
        let account = &export.accounts[0];
        let ufvk = zcash_keys::keys::UnifiedFullViewingKey::decode(&MainNetwork, account.ufvk.trim())
            .map_err(|_| Error::Address("the signing device sent an unusable viewing key".into()))?;
        {
            let db = self.db()?;
            for id in db.get_account_ids().map_err(db_err)? {
                if let Some(existing) = db.get_account(id).map_err(db_err)? {
                    if existing.ufvk().map(|k| k.encode(&MainNetwork)) == Some(ufvk.encode(&MainNetwork)) {
                        return Ok(id.expose_uuid().to_string());
                    }
                }
            }
        }
        let index = zip32::AccountId::try_from(account.index).map_err(|_| Error::Other("the signing device sent an unusable account number".into()))?;
        // Cupcake's QR has no seed fingerprint: no derivation is recorded,
        // which is also how a Cupcake wallet is told apart later. Its QR has
        // no birthday and its wallet may hold Zcash already: look from NU7 on
        // (Cupcake added Zcash after it), as Lock it Mac does.
        let derivation = export.seed_fingerprint.map(|f| Zip32Derivation::new(zip32::fingerprint::SeedFingerprint::from_bytes(f), index));
        let birthday = if export.from_cupcake() { Some(CUPCAKE_BIRTHDAY) } else { birthday };
        let (birthday, tip) = Self::birthday_from(server, birthday).await?;
        let mut db = self.db()?;
        let imported = db
            .import_account_ufvk(name, &ufvk, &birthday, AccountPurpose::Spending { derivation }, Some(SEALIT_SOURCE))
            .map_err(db_err)?;
        db.update_chain_tip(BlockHeight::from_u32(tip)).map_err(db_err)?;
        Ok(imported.id().expose_uuid().to_string())
    }

    /// Hands the main wallet's place to another wallet of the key ring: the
    /// main one is taken off this PC and `successor` becomes the main one.
    /// (The app reseals the lock with the successor's words first.)
    pub fn hand_over_to(&self, successor: &str) -> Result<()> {
        let mut db = self.db()?;
        let old = Self::main_account(&db)?;
        let uuid = uuid::Uuid::parse_str(successor).map_err(|_| Error::Other("unknown wallet".into()))?;
        let new = AccountUuid::from_uuid(uuid);
        if new == old || !db.get_account_ids().map_err(db_err)?.contains(&new) {
            return Err(Error::Other("unknown wallet".into()));
        }
        db.delete_account(old).map_err(db_err)?;
        drop(db);
        let conn = rusqlite::Connection::open(self.db_path()).map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(15)).map_err(db_err)?;
        conn.execute("UPDATE accounts SET key_source = NULL WHERE uuid = ?1", [uuid.as_bytes().to_vec()]).map_err(db_err)?;
        Ok(())
    }

    /// The oldest block any wallet here still has to scan; `None` when
    /// everything is scanned.
    pub fn oldest_to_scan(&self) -> Result<Option<u32>> {
        let db = self.db()?;
        Ok(db.suggest_scan_ranges().map_err(db_err)?.iter().map(|r| u32::from(r.block_range().start)).min())
    }

    /// Whether `server` still has the block at `height` (a pruned node may
    /// not); `None` when it cannot be told.
    pub async fn server_has_block(server: &Server, height: u32) -> Option<bool> {
        net::has_block(server, height).await
    }

    /// Whether the wallet on screen is signed on Cupcake: paired from its QR,
    /// so no seed fingerprint was recorded.
    pub fn is_cupcake(&self) -> Result<bool> {
        use zcash_client_backend::data_api::{Account as _, AccountPurpose, AccountSource};
        let db = self.db()?;
        let account = self.account(&db)?;
        Ok(db.get_account(account).map_err(db_err)?.is_some_and(|a| {
            matches!(
                a.source(),
                AccountSource::Imported { purpose: AccountPurpose::Spending { derivation: None }, key_source } if key_source.as_deref() == Some(SEALIT_SOURCE)
            )
        }))
    }

    /// Whether the wallet on screen is signed on a phone with Seal it.
    pub fn is_sealit(&self) -> Result<bool> {
        let db = self.db()?;
        let account = self.account(&db)?;
        Ok(db.get_account(account).map_err(db_err)?.and_then(|a| a.source().key_source().map(|s| s == SEALIT_SOURCE)).unwrap_or(false))
    }

    /// Seal it, step 1: the payment as a PCZT. Returns the full PCZT (kept
    /// here, for the proofs) and the QR frames of the redacted copy for the
    /// phone.
    ///
    /// For Cupcake the frames carry the whole PCZT, in its layout: it signs
    /// and then makes the proofs itself (as Zkool does).
    /// Returns the full PCZT (kept here), the QR frames, and the size of the
    /// PCZT the device gets.
    pub fn seal_start(&self, payment: &PreparedPayment) -> Result<(Vec<u8>, Vec<String>, usize)> {
        use pczt::roles::redactor::Redactor;
        let Planned::Notes(proposal) = &payment.proposal else {
            return Err(Error::Other("a signing device signs payments only".into()));
        };
        let mut db = self.db()?;
        let pczt = zcash_client_backend::data_api::wallet::create_pczt_from_proposal::<_, _, Infallible, _, Infallible, _>(
            &mut db,
            &MainNetwork,
            &SystemClock,
            &mut OsRng,
            payment.account,
            OvkPolicy::Sender,
            proposal,
            None,
            zcash_primitives::transaction::builder::BundlePadding::DEFAULT,
        )
        .map_err(other)?;
        if pczt.has_data_in_pool(zcash_protocol::PoolType::SAPLING) {
            return Err(Error::Address("A signing device can't sign to or from an old Sapling address. Use a u1… or t1… address.".into()));
        }
        let full = pczt.clone().serialize().map_err(|e| other(format!("{e:?}")))?;
        drop(db);
        if self.is_cupcake()? {
            let for_cupcake = crate::cupcake::to_cupcake(&full)?;
            let frames = crate::sealit::cupcake_frames(crate::sealit::UR_PCZT, &crate::sealit::pczt_envelope(&for_cupcake))?;
            let size = for_cupcake.len();
            return Ok((full, frames, size));
        }
        let redacted = Redactor::new(pczt)
            .redact_orchard_with(|mut r| {
                r.redact_actions(|mut a| a.clear_spend_witness());
                r.clear_zkproof();
            })
            .redact_ironwood_with(|mut r| {
                r.redact_actions(|mut a| a.clear_spend_witness());
                r.clear_zkproof();
            })
            .finish();
        let bytes = redacted.serialize().map_err(|e| other(format!("{e:?}")))?;
        let frames = crate::sealit::ur_frames(crate::sealit::UR_PCZT, &crate::sealit::pczt_envelope(&bytes))?;
        Ok((full, frames, bytes.len()))
    }

    /// Seal it, step 2 (while the phone signs): the Orchard and Ironwood
    /// proofs, made on this PC.
    pub fn seal_prove(full: &[u8]) -> Result<Vec<u8>> {
        use pczt::roles::prover::Prover;
        let pczt = pczt::Pczt::parse(full).map_err(|e| other(format!("{e:?}")))?;
        let pk = zcash_primitives::transaction::builder::cached_orchard_proving_key(circuit_version(&pczt)?);
        let mut prover = Prover::new(pczt);
        if prover.requires_sapling_proofs() {
            return Err(Error::Other("a signing device can't sign Sapling".into()));
        }
        if prover.requires_orchard_proof() {
            prover = prover.create_orchard_proof(OsRng, pk).map_err(|e| other(format!("the proof failed: {e:?}")))?;
        }
        if prover.requires_ironwood_proof() {
            prover = prover.create_ironwood_proof(OsRng, pk).map_err(|e| other(format!("the proof failed: {e:?}")))?;
        }
        prover.finish().serialize().map_err(|e| other(format!("{e:?}")))
    }

    /// Seal it, step 3: the phone's signatures and this PC's proofs
    /// together, ready to hand to the network (and stored in the wallet).
    ///
    /// From Cupcake, `proven` is the full PCZT without proofs: Cupcake's
    /// reply carries its own proofs and signatures. It is taken back into the
    /// standard layout (anything filled in for assets is refused) and
    /// combined with this PC's copy, so it can only finish this payment; the
    /// proofs and signatures are checked before anything is sent.
    pub fn seal_finish(&self, proven: &[u8], signed_message: &[u8]) -> Result<SignedPayment> {
        use pczt::roles::combiner::Combiner;
        let signed = crate::sealit::parse_pczt(signed_message)?;
        let signed = if self.is_cupcake()? { crate::cupcake::from_cupcake(&signed)? } else { signed };
        let signed = pczt::Pczt::parse(&signed).map_err(|_| Error::Other("the signed transaction from the signing device is damaged".into()))?;
        let proven = pczt::Pczt::parse(proven).map_err(|e| other(format!("{e:?}")))?;
        let combined = Combiner::new(vec![proven, signed])
            .combine()
            .map_err(|_| Error::Other("the signature is for another payment; start again".into()))?;
        let vk = orchard::circuit::VerifyingKey::build(circuit_version(&combined)?);
        let mut db = self.db()?;
        let txid = zcash_client_backend::data_api::wallet::extract_and_store_transaction_from_pczt::<_, ReceivedNoteId>(
            &mut db,
            &SystemClock,
            &mut OsRng,
            combined,
            None,
            Some(&vk),
        )
        .map_err(|e| other(format!("the signed payment did not check out: {e}")))?;
        let tx = db.get_transaction(txid).map_err(db_err)?.ok_or_else(|| other("the signed payment was not stored"))?;
        let mut bytes = Vec::new();
        tx.write(&mut bytes).map_err(other)?;
        Ok(SignedPayment { txids: vec![txid.to_string()], raw: vec![bytes], expiry_height: u32::from(tx.expiry_height()) })
    }

    /// Removes the selected wallet from the key ring (never the main one).
    pub fn remove_account(&self) -> Result<()> {
        let mut db = self.db()?;
        let account = self.account(&db)?;
        if account == Self::main_account(&db)? {
            return Err(Error::Other("the main wallet is removed with Erase everything".into()));
        }
        db.delete_account(account).map_err(db_err)
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

    /// One sync pass of at most `budget`: true when everything is checked,
    /// false when time ran out with blocks left. The scan library asks for
    /// the network block only at the start of a pass, and checks the newest
    /// blocks first; so short passes keep the newest blocks checked while a
    /// new wallet's older blocks take hours (25 Nov 2026, as Lock it Mac).
    /// Stopping between two batches is safe: each batch is written at once.
    pub async fn sync_for(&self, server: &Server, budget: std::time::Duration) -> Result<bool> {
        within(self.sync(server), budget).await
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
        balance.scan_from = Self::scan_start(&db);
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

    /// The wallet's transparent address, for senders that cannot pay a
    /// shielded one (some exchanges). One fixed address per wallet: its
    /// first external transparent address, the same in every Zcash wallet
    /// restored from these words. What arrives there is public until it is
    /// locked in.
    pub fn transparent_address(&self) -> Result<String> {
        use zcash_client_backend::data_api::Account as _;
        use zcash_transparent::keys::IncomingViewingKey as _;
        let db = self.db()?;
        let account = self.account(&db)?;
        let ufvk = db
            .get_account(account)
            .map_err(db_err)?
            .and_then(|a| a.ufvk().cloned())
            .ok_or_else(|| other("this wallet has no viewing key"))?;
        let transparent = ufvk.p2pkh().ok_or_else(|| other("this wallet has no transparent part"))?;
        let (address, _) = transparent.derive_external_ivk().map_err(other)?.default_address();
        Ok(zcash_keys::address::Address::Transparent(address).encode(&MainNetwork))
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
        Ok(PreparedPayment { proposal: Planned::Notes(proposal), account, amount, fee, to: to.trim().to_owned() })
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
        Ok(PreparedPayment { proposal: Planned::Notes(proposal), account, amount: held.saturating_sub(fee), fee, to: "Ironwood".into() })
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
        Ok(PreparedPayment { proposal: Planned::Shielding(proposal), account, amount: held.saturating_sub(fee), fee, to: "Lock it in".into() })
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
        let derived = db.get_account(payment.account).map_err(db_err)?.ok_or(Error::NoWallet)?;
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
        let mut signed = SignedPayment { txids: Vec::new(), raw: Vec::new(), expiry_height: 0 };
        for txid in txids.iter() {
            let tx = db.get_transaction(*txid).map_err(db_err)?.ok_or_else(|| other("the signed payment was not stored"))?;
            let expiry = u32::from(tx.expiry_height());
            // The earliest "valid until" of its transactions (0 = no limit).
            if expiry > 0 && (signed.expiry_height == 0 || expiry < signed.expiry_height) {
                signed.expiry_height = expiry;
            }
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
    /// Hands the payment's transactions to the network, in order. A payment
    /// can be two transactions (TEX: shielded to a temporary address of this
    /// wallet, then on to the receiver). On a refusal, says how many had
    /// already gone through, so "nothing was sent" is only said when true.
    pub async fn broadcast(&self, signed: &SignedPayment, reading_from: &Server, over_tor: bool) -> std::result::Result<Vec<String>, (usize, Error)> {
        let mut accepted_by = Vec::new();
        for (done, bytes) in signed.raw.iter().enumerate() {
            accepted_by = net::submit(bytes, reading_from, over_tor, &self.tor_dir()).await.map_err(|e| (done, e))?;
        }
        Ok(accepted_by)
    }

    /// The price of 1 ZEC, over Tor (see `net::price_over_tor`).
    pub async fn price_over_tor(&self) -> Result<std::collections::HashMap<String, f64>> {
        net::price_over_tor(&self.tor_dir()).await
    }

    /// The newest payments in and out, newest first.
    pub fn activity(&self, limit: u32) -> Result<Vec<Activity>> {
        let account = {
            let db = self.db()?;
            self.account(&db)?
        };
        let conn = rusqlite::Connection::open(self.db_path()).map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(15)).map_err(db_err)?;
        let mut statement = conn
            .prepare(
                "SELECT txid, mined_height, account_balance_delta, block_time, expired_unmined, fee_paid,
                        (SELECT o.to_address FROM v_tx_outputs o
                          WHERE o.txid = v_transactions.txid AND o.to_address LIKE 'tex1%' LIMIT 1)
                 FROM v_transactions
                 WHERE account_uuid = ?2
                 ORDER BY (mined_height IS NULL) DESC, mined_height DESC
                 LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map(rusqlite::params![limit, account.expose_uuid().as_bytes().to_vec()], |row| {
                let txid: Vec<u8> = row.get(0)?;
                Ok(Activity {
                    // Shown the way explorers show it: bytes reversed.
                    txid: txid.iter().rev().map(|b| format!("{b:02x}")).collect(),
                    height: row.get::<_, Option<u32>>(1)?,
                    change: row.get(2)?,
                    time: row.get::<_, Option<i64>>(3)?,
                    expired: row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                    fee: row.get::<_, Option<i64>>(5)?.map(|f| f.max(0) as u64),
                    tex_receiver: row.get::<_, Option<String>>(6)?,
                    tex_step: None,
                    title: String::new(),
                })
            })
            .map_err(db_err)?;
        let list = rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_err)?;
        Ok(Activity::mark(list))
    }

    /// The newest payment lines of every wallet on this PC, for the arrival
    /// message.
    pub fn incoming(&self, limit: u32) -> Result<Vec<Incoming>> {
        let conn = rusqlite::Connection::open(self.db_path()).map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(15)).map_err(db_err)?;
        let mut statement = conn
            .prepare(
                "SELECT txid, account_uuid, mined_height, block_time, account_balance_delta, expired_unmined,
                        is_shielding, sent_note_count, pool_crossing_value
                 FROM v_transactions
                 ORDER BY (mined_height IS NULL) DESC, mined_height DESC
                 LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = statement
            .query_map(rusqlite::params![limit], |row| {
                let txid: Vec<u8> = row.get(0)?;
                let account: Vec<u8> = row.get(1)?;
                Ok(Incoming {
                    txid: txid.iter().rev().map(|b| format!("{b:02x}")).collect(),
                    account: uuid::Uuid::from_slice(&account).map(|u| u.to_string()).unwrap_or_default(),
                    height: row.get::<_, Option<u32>>(2)?,
                    time: row.get::<_, Option<i64>>(3)?,
                    change: row.get(4)?,
                    expired: row.get::<_, Option<bool>>(5)?.unwrap_or(false),
                    shielding: row.get::<_, Option<bool>>(6)?.unwrap_or(false),
                    sent_notes: row.get::<_, Option<u32>>(7)?.unwrap_or(0),
                    pool_crossing: row.get::<_, Option<i64>>(8)?.is_some(),
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

    /// The block the shared scan starts from: the earliest start of all
    /// wallets here (0 when there is none).
    pub fn scan_from(&self) -> Result<u32> {
        Ok(Self::scan_start(&self.db()?))
    }

    fn scan_start<D: WalletRead>(db: &D) -> u32 {
        db.get_wallet_birthday().ok().flatten().map(u32::from).unwrap_or(0)
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
    /// The last block in which the network still takes it (0 = no limit).
    pub expiry_height: u32,
}

impl SignedPayment {
    /// A short name for messages: "Payment 3f9a2c…".
    pub fn short_name(&self) -> String {
        match self.txids.first() {
            Some(txid) => format!("Payment {}…", &txid[..txid.len().min(6)]),
            None => "The payment".into(),
        }
    }

    /// The part still to be handed over after the first `done` went through.
    pub fn without_first(mut self, done: usize) -> SignedPayment {
        let done = done.min(self.raw.len());
        self.raw.drain(..done);
        self.txids.drain(..done.min(self.txids.len()));
        self
    }

    /// Whether the network no longer takes it at `tip`.
    pub fn expired_at(&self, tip: u32) -> bool {
        self.expiry_height > 0 && tip > self.expiry_height
    }
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
    /// The network fee, for payments out.
    pub fee: Option<u64>,
    /// For a payment to a TEX address (ZIP 320): the tex1 receiver.
    pub tex_receiver: Option<String>,
    /// 1 or 2: which step of a TEX payment this is.
    pub tex_step: Option<u8>,
    /// "Locked", "Released", "TEX payment" or "TEX payment · forwarding fee".
    pub title: String,
}

/// A payment line of one wallet, for the arrival message: every wallet on
/// this PC at once (a payment between two of them has a line for each).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Incoming {
    /// Transaction id, as explorers show it.
    pub txid: String,
    /// The wallet's id (a UUID).
    pub account: String,
    /// The block it is in; `None` while waiting.
    pub height: Option<u32>,
    /// When its block was made (Unix seconds).
    pub time: Option<i64>,
    /// What it did to this wallet's balance, in zatoshis.
    pub change: i64,
    /// It never made it into a block and has expired.
    pub expired: bool,
    /// Locking in transparent Zcash (a shielding transaction).
    pub shielding: bool,
    /// Notes of this wallet it spent: this wallet sent it.
    pub sent_notes: u32,
    /// A move between pools of this wallet (e.g. to Ironwood).
    pub pool_crossing: bool,
}

impl Incoming {
    /// Money that came in from elsewhere: not sent by this wallet, no notes
    /// of it spent, not locking in, not a move between its pools, not
    /// expired, above zero (Lock it Mac's noteArrivals).
    pub fn is_received(&self) -> bool {
        self.change > 0 && self.sent_notes == 0 && !self.shielding && !self.pool_crossing && !self.expired
    }

    /// One payment for one wallet.
    pub fn key(&self) -> String {
        format!("{}:{}", self.txid, self.account)
    }
}

/// What an arrival message is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrivalKind {
    /// Zcash came in: "Locked in · <wallet>".
    LockedIn,
    /// Transparent Zcash was locked in: "Locked in ✓ · <wallet>".
    Shielded,
}

/// One arrival message, waiting to be shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Arrival {
    /// The payment.
    pub txid: String,
    /// The wallet it arrived in.
    pub account: String,
    /// What happened.
    pub kind: ArrivalKind,
    /// The amount, in zatoshis.
    pub amount: u64,
}

/// Which payments get a message, as Lock it Mac's noteArrivals: payments
/// already there are history; only news gets a message, and never twice.
#[derive(Default)]
pub struct ArrivalWatch {
    seen: std::collections::HashSet<String>,
    shieldings_waiting: std::collections::HashSet<String>,
    announced: std::collections::VecDeque<String>,
    seen_current_at: Option<i64>,
}

impl ArrivalWatch {
    /// How many announced payments are kept (on disk, across restarts).
    pub const KEEP: usize = 100;

    /// Starts with the payments announced before (newest last).
    pub fn new(announced: Vec<String>) -> Self {
        let mut watch = Self::default();
        for key in announced {
            watch.mark_announced(key);
        }
        watch
    }

    /// The announced payments to keep, oldest first.
    pub fn announced(&self) -> Vec<String> {
        self.announced.iter().cloned().collect()
    }

    fn mark_announced(&mut self, key: String) {
        if !self.announced.contains(&key) {
            self.announced.push_back(key);
        }
        while self.announced.len() > Self::KEEP {
            self.announced.pop_front();
        }
    }

    /// Looks at the newest payment lines of every wallet after a scan and
    /// returns the new messages. `current`: Lock it is up to date now.
    /// `is_news(block, block_time, since)` decides what is news.
    pub fn note(&mut self, lines: &[Incoming], current: bool, now: i64, is_news: impl Fn(Option<u32>, Option<i64>, Option<i64>) -> bool) -> Vec<Arrival> {
        let fresh: Vec<&Incoming> = lines.iter().filter(|line| self.seen.insert(line.key())).collect();
        // Locking in transparent Zcash: news once it is in a block.
        let shielded: Vec<&Incoming> = lines
            .iter()
            .filter(|line| line.shielding && line.height.is_some() && self.shieldings_waiting.remove(&line.key()))
            .collect();
        for line in lines.iter().filter(|line| line.shielding && line.height.is_none() && !line.expired) {
            self.shieldings_waiting.insert(line.key());
        }
        let since = self.seen_current_at;
        if current {
            self.seen_current_at = Some(now);
        }
        let received = fresh.into_iter().filter(|line| line.is_received() && is_news(line.height, line.time, since));
        let mut taken = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (line, kind) in shielded.into_iter().map(|l| (l, ArrivalKind::Shielded)).chain(received.map(|l| (l, ArrivalKind::LockedIn))) {
            let key = line.key();
            if self.announced.contains(&key) || !taken.insert(key.clone()) {
                continue;
            }
            self.mark_announced(key);
            out.push(Arrival { txid: line.txid.clone(), account: line.account.clone(), kind, amount: line.change.unsigned_abs() });
        }
        out
    }
}

impl Activity {
    /// A payment to a TEX address is two transactions: step 1 takes the
    /// Zcash from the private pool to a temporary transparent address of this
    /// wallet (the tex1 receiver is recorded on it), step 2 forwards it, so
    /// only its fee leaves the wallet. Step 2 is a payment out that is only a
    /// fee, in the same block as a step 1 (or, not yet in a block, within
    /// five minutes of it). Both get the tex1 receiver and a title.
    pub fn mark(mut items: Vec<Activity>) -> Vec<Activity> {
        let steps1: Vec<usize> = (0..items.len()).filter(|&i| items[i].change < 0 && items[i].tex_receiver.is_some()).collect();
        for &i in &steps1 {
            items[i].tex_step = Some(1);
        }
        for i in 0..items.len() {
            let item = &items[i];
            let fee_only = item.change < 0 && item.tex_receiver.is_none()
                && item.fee.is_some_and(|f| f > 0 && item.change.unsigned_abs() == f);
            if !fee_only {
                continue;
            }
            let matched = steps1.iter().copied().find(|&j| {
                let (a, b) = (&items[i], &items[j]);
                match (a.height, b.height) {
                    (Some(x), Some(y)) => x == y,
                    _ => match (a.time, b.time) {
                        (Some(x), Some(y)) => (x - y).abs() <= 300,
                        _ => a.height.is_none() && b.height.is_none() && a.expired == b.expired,
                    },
                }
            });
            if let Some(j) = matched {
                items[i].tex_step = Some(2);
                items[i].tex_receiver = items[j].tex_receiver.clone();
            }
        }
        for item in &mut items {
            item.title = match item.tex_step {
                Some(1) => "TEX payment".into(),
                Some(2) => "TEX payment · forwarding fee".into(),
                _ if item.change > 0 => "Locked".into(),
                _ => "Released".into(),
            };
        }
        items
    }
}

use crate::SAPLING_ACTIVATION;

fn db_err(error: impl std::fmt::Display) -> Error {
    Error::Database(error.to_string())
}

#[cfg(test)]
mod tex_tests {
    use super::Activity;

    fn item(change: i64, height: Option<u32>, time: Option<i64>, fee: u64, tex: Option<&str>) -> Activity {
        Activity { txid: String::new(), height, change, time, expired: false, fee: Some(fee), tex_receiver: tex.map(String::from), tex_step: None, title: String::new() }
    }

    /// Seen 21 Nov 2026 on Mac: step 2 of a TEX payment showed as "Sent −0.000 ZEC".
    #[test]
    fn both_steps_of_a_tex_payment_are_named() {
        let tex = "tex18qayysmdg7qqk4qta0vh0xss8ngx3qytyj4828";
        let marked = Activity::mark(vec![
            item(-110_000, Some(3_597_827), Some(1_795_211_580), 10_000, Some(tex)),
            item(-15_000, Some(3_597_827), Some(1_795_211_580), 15_000, None),
            item(-15_000, Some(3_590_000), Some(1_795_125_180), 15_000, None),
            item(50_000, Some(3_590_001), None, 0, None),
        ]);
        assert_eq!(marked[0].title, "TEX payment");
        assert_eq!(marked[1].title, "TEX payment · forwarding fee");
        assert_eq!(marked[1].tex_receiver.as_deref(), Some(tex));
        assert_eq!(marked[2].title, "Released", "a fee-only payment in another block is not a TEX step");
        assert_eq!(marked[3].title, "Locked");
    }

    #[test]
    fn a_waiting_pair_is_matched_too() {
        let tex = "tex1abc";
        let marked = Activity::mark(vec![item(-110_000, None, None, 10_000, Some(tex)), item(-15_000, None, None, 15_000, None)]);
        assert_eq!(marked.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["TEX payment", "TEX payment · forwarding fee"]);
    }
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

    /// A wallet paired from Cupcake's QR (no seed fingerprint) is told apart
    /// from a Seal it one, which goes through the usual proving path.
    #[test]
    fn a_cupcake_wallet_is_told_apart() {
        use zcash_client_backend::data_api::{AccountPurpose, Zip32Derivation};
        let (_dir, wallet, _seed) = fresh();
        assert!(!wallet.is_cupcake().unwrap(), "the main wallet is not");
        let other_seed = [9u8; 32];
        let ufvk = |i: u32| {
            zcash_keys::keys::UnifiedSpendingKey::from_seed(&MainNetwork, &other_seed, zip32::AccountId::try_from(i).unwrap())
                .unwrap()
                .to_unified_full_viewing_key()
        };
        let birthday = AccountBirthday::from_parts(zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION - 1), BlockHash([0; 32])), None);
        let (cupcake, sealit) = {
            let mut db = wallet.db().unwrap();
            let cupcake = db.import_account_ufvk("Cupcake", &ufvk(0), &birthday, AccountPurpose::Spending { derivation: None }, Some(SEALIT_SOURCE)).unwrap();
            let derivation = Zip32Derivation::new(zip32::fingerprint::SeedFingerprint::from_bytes([3; 32]), zip32::AccountId::try_from(1).unwrap());
            let sealit = db.import_account_ufvk("Seal it", &ufvk(1), &birthday, AccountPurpose::Spending { derivation: Some(derivation) }, Some(SEALIT_SOURCE)).unwrap();
            (cupcake.id().expose_uuid().to_string(), sealit.id().expose_uuid().to_string())
        };
        assert!(wallet.for_account(Some(&cupcake)).is_cupcake().unwrap());
        assert!(wallet.for_account(Some(&cupcake)).is_sealit().unwrap());
        assert!(!wallet.for_account(Some(&sealit)).is_cupcake().unwrap());
        assert!(wallet.for_account(Some(&sealit)).is_sealit().unwrap());
    }

    /// The arrival message's filter: only money from elsewhere.
    #[test]
    fn only_money_from_elsewhere_counts_as_received() {
        let paid = Incoming {
            txid: "ab".into(), account: "w".into(), height: Some(3_600_000), time: Some(1), change: 50_000,
            expired: false, shielding: false, sent_notes: 0, pool_crossing: false,
        };
        assert!(paid.is_received());
        assert!(!Incoming { change: -20_000, ..paid.clone() }.is_received(), "sent");
        assert!(!Incoming { sent_notes: 1, ..paid.clone() }.is_received(), "a move from this wallet (its change comes back)");
        assert!(!Incoming { shielding: true, ..paid.clone() }.is_received(), "locking in");
        assert!(!Incoming { pool_crossing: true, ..paid.clone() }.is_received(), "a move between pools");
        assert!(!Incoming { expired: true, ..paid.clone() }.is_received(), "expired");
        assert!(!Incoming { change: 0, ..paid.clone() }.is_received());
        assert_eq!(paid.key(), "ab:w");
    }

    fn line(txid: &str, account: &str, height: Option<u32>, change: i64) -> Incoming {
        Incoming { txid: txid.into(), account: account.into(), height, time: None, change, expired: false, shielding: false, sent_notes: 0, pool_crossing: false }
    }

    /// News once, never twice; history stays quiet; a lock-in gets its
    /// message once in a block; a payment between two wallets here is news
    /// only for the one that receives.
    #[test]
    fn arrivals_are_news_once() {
        let news = |height: Option<u32>, _t: Option<i64>, _s: Option<i64>| height.map_or(true, |h| h >= 3_609_380);
        let mut watch = ArrivalWatch::new(vec![]);
        let old = line("aa", "main", Some(2_400_000), 10_000);
        let new = line("bb", "main", Some(3_609_470), 25_000);
        let got = watch.note(&[old.clone(), new.clone()], true, 1_000, news);
        assert_eq!(got, vec![Arrival { txid: "bb".into(), account: "main".into(), kind: ArrivalKind::LockedIn, amount: 25_000 }]);
        assert!(watch.note(&[old.clone(), new.clone()], true, 1_025, news).is_empty(), "never twice in a session");
        // After a restart, the list on disk keeps it quiet.
        let mut again = ArrivalWatch::new(watch.announced());
        assert!(again.note(&[new.clone()], true, 2_000, news).is_empty(), "never twice across restarts");
        // Between two wallets here: the sender's line (its notes spent) is no news.
        let sent = Incoming { sent_notes: 2, change: -30_000, ..line("cc", "main", None, 0) };
        let came = line("cc", "shop", None, 20_000);
        let got = again.note(&[sent, came], true, 2_025, news);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].account, "shop");
        // Locking in: a message once it is in a block, not while waiting.
        let waiting = Incoming { shielding: true, ..line("dd", "main", None, -15_000) };
        assert!(again.note(&[waiting.clone()], true, 2_050, news).is_empty());
        let mined = Incoming { height: Some(3_609_475), ..waiting };
        let got = again.note(&[mined.clone()], true, 2_075, news);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, ArrivalKind::Shielded);
        assert!(again.note(&[mined], true, 2_100, news).is_empty());
    }

    #[test]
    fn the_announced_list_keeps_the_newest_100() {
        let watch = ArrivalWatch::new((0..150).map(|i| format!("t{i}:main")).collect());
        let kept = watch.announced();
        assert_eq!(kept.len(), ArrivalWatch::KEEP);
        assert_eq!(kept.first().unwrap(), "t50:main");
        assert_eq!(kept.last().unwrap(), "t149:main");
    }

    /// Lock it was up to date, then the PC slept: a payment from the
    /// sleep is news at the next scan (the rule gets the time it was
    /// last up to date).
    #[test]
    fn the_rule_gets_when_lock_it_was_last_up_to_date() {
        let mut watch = ArrivalWatch::new(vec![]);
        let seen = std::cell::RefCell::new(Vec::new());
        let rule = |_h: Option<u32>, _t: Option<i64>, since: Option<i64>| {
            seen.borrow_mut().push(since);
            false
        };
        watch.note(&[line("a1", "main", Some(1), 5)], true, 100, &rule);
        watch.note(&[line("a2", "main", Some(2), 5)], false, 200, &rule);
        watch.note(&[line("a3", "main", Some(3), 5)], true, 300, &rule);
        assert_eq!(*seen.borrow(), vec![None, Some(100), Some(100)]);
    }

    /// The query reads every wallet's lines (an empty new wallet has none).
    #[test]
    fn incoming_reads_the_wallet_database() {
        let (_dir, wallet, _seed) = fresh();
        assert!(wallet.incoming(200).unwrap().is_empty());
    }

    /// A sync pass stops when its time is up and says more is left; a
    /// quick one says it finished; an error stays an error.
    #[test]
    fn a_sync_pass_stops_on_time() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        rt.block_on(async {
            let slow = async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(())
            };
            let started = std::time::Instant::now();
            assert!(!within(slow, std::time::Duration::from_millis(50)).await.unwrap());
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            assert!(within(async { Ok(()) }, std::time::Duration::from_secs(1)).await.unwrap());
            assert!(within(async { Err(Error::Other("no".into())) }, std::time::Duration::from_secs(1)).await.is_err());
        });
    }

    /// The Sync screen's "Checked from": one scan for every wallet here,
    /// so it starts at the earliest wallet's start, whichever is selected.
    #[test]
    fn the_scan_starts_at_the_earliest_wallet() {
        use zcash_client_backend::data_api::{AccountPurpose, Zip32Derivation};
        let (_dir, wallet, _seed) = fresh();
        let main_start = wallet.scan_from().unwrap();
        let ufvk = zcash_keys::keys::UnifiedSpendingKey::from_seed(&MainNetwork, &[5u8; 32], zip32::AccountId::ZERO).unwrap().to_unified_full_viewing_key();
        let later = AccountBirthday::from_parts(zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(3_600_000), BlockHash([0; 32])), None);
        let id = {
            let mut db = wallet.db().unwrap();
            let derivation = Zip32Derivation::new(zip32::fingerprint::SeedFingerprint::from_bytes([4; 32]), zip32::AccountId::ZERO);
            let ks = db.import_account_ufvk("Keystone", &ufvk, &later, AccountPurpose::Spending { derivation: Some(derivation) }, Some(SEALIT_SOURCE)).unwrap();
            ks.id().expose_uuid().to_string()
        };
        let keystone = wallet.for_account(Some(&id));
        assert_eq!(keystone.scan_from().unwrap(), main_start, "the selected Keystone wallet still shows the shared start");
        assert!(main_start < 3_600_000 && main_start > 0, "{main_start}");
    }

    /// The Transparent tab: one fixed t1 address per wallet, which the
    /// wallet watches (so what arrives there shows as outside the lock).
    #[test]
    fn the_transparent_address_is_fixed_and_watched() {
        let (_dir, wallet, _seed) = fresh();
        let first = wallet.transparent_address().unwrap();
        assert!(first.starts_with("t1"), "{first}");
        assert_eq!(wallet.transparent_address().unwrap(), first, "the same each time");
        let db = wallet.db().unwrap();
        let account = wallet.account(&db).unwrap();
        let watched: Vec<String> = db
            .get_transparent_receivers(account, true, false)
            .unwrap()
            .into_keys()
            .map(|a| zcash_keys::address::Address::Transparent(a).encode(&MainNetwork))
            .collect();
        assert!(watched.contains(&first), "the wallet must watch it for arrivals");
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
    fn the_key_ring_holds_wallets_side_by_side() {
        let (_dir, wallet, _seed) = fresh();
        let birthday = AccountBirthday::from_parts(
            zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION - 1), BlockHash([0; 32])),
            None,
        );
        let second = phrase_to_seed(&new_phrase()).unwrap();
        let id = wallet.add_wallet_from_birthday("Savings", &second, &birthday, SAPLING_ACTIVATION + 10).unwrap();
        let list = wallet.accounts().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].main && !list[1].main, "the main wallet comes first");
        assert_eq!(list[1].name, "Savings");
        let extra = wallet.for_account(Some(&id));
        assert_ne!(extra.receive_address().unwrap(), wallet.receive_address().unwrap());
        assert!(extra.activity(10).unwrap().is_empty());
        assert!(wallet.add_wallet_from_birthday("Again", &second, &birthday, SAPLING_ACTIVATION + 10).is_err(), "never twice");
        assert!(wallet.remove_account().is_err(), "the main wallet is not removed this way");
        let third = phrase_to_seed(&new_phrase()).unwrap();
        let other = wallet.add_wallet_from_birthday("Spending", &third, &birthday, SAPLING_ACTIVATION + 10).unwrap();
        extra.remove_account().unwrap();
        assert_eq!(wallet.accounts().unwrap().len(), 2);
        // The main wallet is erased: Spending takes its place.
        let address = wallet.for_account(Some(&other)).receive_address().unwrap();
        wallet.hand_over_to(&other).unwrap();
        let list = wallet.accounts().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].main && list[0].id == other);
        assert_eq!(wallet.receive_address().unwrap(), address);
    }

    #[test]
    fn an_added_wallet_has_older_blocks_to_scan() {
        // Item 1 (Mac build 6): after adding a wallet, the oldest block still
        // to scan is its birthday's, the one Go Max must be asked about.
        let (_dir, wallet, _seed) = fresh();
        let birthday = AccountBirthday::from_parts(
            zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION + 99), BlockHash([0; 32])),
            None,
        );
        let seed = phrase_to_seed(&new_phrase()).unwrap();
        wallet.add_wallet_from_birthday("Old savings", &seed, &birthday, SAPLING_ACTIVATION + 5_000).unwrap();
        let oldest = wallet.oldest_to_scan().unwrap().expect("blocks to scan");
        assert!(oldest <= SAPLING_ACTIVATION + 100, "the added wallet's blocks come first: {oldest}");
    }

    #[test]
    fn a_wallet_is_never_created_twice() {
        let (_dir, wallet, seed) = fresh();
        let birthday = AccountBirthday::from_parts(zcash_client_backend::data_api::chain::ChainState::empty(BlockHeight::from_u32(SAPLING_ACTIVATION - 1), BlockHash([0; 32])), None);
        assert!(wallet.create_from_birthday(&seed, &birthday, SAPLING_ACTIVATION + 10).is_err());
    }
}

