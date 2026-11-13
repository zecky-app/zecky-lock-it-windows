//! The wallet in the app: create or restore, receive, send, and keep in sync.
//!
//! Rules this file keeps (as on Lock it Mac):
//!
//! * The 24 words live only inside the lock (`wallet-lock.json`, sealed by
//!   the owner's keys). They are shown once while creating a wallet, so they
//!   can be written down, and are otherwise opened only to sign a payment,
//!   then let go.
//! * Reading: from Go Max when it has caught up and still holds the blocks
//!   the wallet needs; otherwise from a public server (zec.rocks).
//! * Sending: over Tor by default, to two public servers. A signed payment
//!   that could not be handed over is kept, so "try again" never pays twice.

use std::{
    fs,
    path::PathBuf,
    sync::{mpsc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use lockit_auth::{KeyDescriptor, Secret, Vault};
use lockit_wallet::{Activity, Balance, PreparedPayment, Server, SignedPayment, Wallet};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use secrecy::ExposeSecret;
use zeroize::Zeroizing;

use crate::{go_max::GoMaxSlot, problem, source_for, ChosenKey, CommandResult, KeySummary, Problem};

const LOCK_FILE: &str = "wallet-lock.json";
/// How often the wallet looks for new blocks by itself.
const SYNC_EVERY: Duration = Duration::from_secs(25);
/// Go Max keeps this many recent blocks while syncing (its smallest window),
/// less a margin: a wallet further behind reads from a public server.
const NODE_WINDOW: u32 = 9_000;

/// How the sync is doing, for the home screen.
#[derive(Clone, Default, Serialize)]
pub struct SyncView {
    /// "starting", "syncing", "up_to_date" or "problem".
    state: String,
    /// Where Lock it reads from.
    server: String,
    /// The last problem, in words.
    problem: Option<String>,
}

/// The wallet's place in the app.
#[derive(Default)]
pub struct WalletSlot {
    /// New or typed words, waiting to be sealed into the lock.
    pending: Mutex<Option<Zeroizing<String>>>,
    /// A payment worked out and waiting for "Really open the lock?".
    prepared: Mutex<Option<PreparedPayment>>,
    /// A payment signed but not yet accepted by any server.
    unsent: Mutex<Option<SignedPayment>>,
    sync: Mutex<SyncView>,
    nudge: Mutex<Option<mpsc::Sender<()>>>,
    /// The words while the lock is open (after "Really open the lock?" and
    /// the keys). Let go when it closes, after a payment and when the
    /// window is hidden.
    opened: Mutex<Option<Zeroizing<String>>>,
    /// Recent sync problems, newest first, at most 5 (as Lock it Mac).
    problems: Mutex<Vec<SyncProblem>>,
}

/// One sync problem, for the sync details.
#[derive(Clone, Serialize)]
pub struct SyncProblem {
    /// When (Unix seconds).
    at: i64,
    /// In words.
    message: String,
}

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lockit-wallet")
            .enable_all()
            .build()
            .expect("the wallet's runtime could not start")
    })
}

pub(crate) fn data_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    let dir = app.path().app_local_data_dir().map_err(|e| problem(format!("the app's data folder could not be found: {e}")))?;
    fs::create_dir_all(&dir).map_err(|e| problem(format!("the app's data folder could not be created: {e}")))?;
    Ok(dir)
}

/// The wallet on screen (the main one, or another on the key ring).
pub(crate) fn wallet(app: &AppHandle) -> CommandResult<Wallet> {
    Ok(main_wallet(app)?.for_account(load_settings(app).current.as_deref()))
}

pub(crate) fn main_wallet(app: &AppHandle) -> CommandResult<Wallet> {
    Ok(Wallet::new(data_dir(app)?.join("wallet")))
}

/// One extra wallet's words, sealed with a key from the main words.
#[derive(Clone, Serialize, Deserialize)]
struct RingEntry {
    id: String,
    sealed: String,
}

fn keyring_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(data_dir(app)?.join("keyring.json"))
}

fn load_keyring(app: &AppHandle) -> Vec<RingEntry> {
    keyring_path(app)
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save_keyring(app: &AppHandle, ring: &[RingEntry]) -> CommandResult<()> {
    let json = serde_json::to_string_pretty(ring).map_err(|e| problem(e.to_string()))?;
    let path = keyring_path(app)?;
    let fresh = path.with_extension("new.json");
    fs::write(&fresh, json).map_err(|e| problem(format!("the key ring could not be saved: {e}")))?;
    fs::rename(&fresh, &path).map_err(|e| problem(format!("the key ring could not be saved: {e}")))
}

/// The words that sign for the wallet on screen: the main words, or an
/// extra wallet's words opened with them.
fn signing_phrase(app: &AppHandle, main_phrase: &Zeroizing<String>) -> CommandResult<Zeroizing<String>> {
    phrase_of(app, main_phrase, load_settings(app).current)
}

/// The words of the wallet with this id (`None`: the main one).
fn phrase_of(app: &AppHandle, main_phrase: &Zeroizing<String>, id: Option<String>) -> CommandResult<Zeroizing<String>> {
    let Some(current) = id else { return Ok(main_phrase.clone()) };
    let entry = load_keyring(app).into_iter().find(|entry| entry.id == current);
    let Some(entry) = entry else {
        return Err(problem("this wallet is watch-only here: it is signed on your phone with Seal it"));
    };
    let seed = lockit_wallet::phrase_to_seed(main_phrase)?;
    Ok(lockit_auth::keyring::open_words(seed.expose_secret(), &entry.sealed)?)
}

fn lock_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(data_dir(app)?.join(LOCK_FILE))
}

pub(crate) fn read_lock(app: &AppHandle) -> CommandResult<Option<Vault>> {
    match fs::read_to_string(lock_path(app)?) {
        Ok(json) => Ok(Some(Vault::from_json(&json)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(problem(format!("the lock file could not be read: {e}"))),
    }
}

fn sapling_params(app: &AppHandle) -> CommandResult<PathBuf> {
    let dir = app.path().resource_dir().map_err(|e| problem(format!("the app's own folder could not be found: {e}")))?;
    Ok(dir.join("params"))
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The owner's choices, kept in `settings.json` (nothing secret in it).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The server chosen in Settings › Network; `None` is zec.rocks.
    server: Option<(String, u16)>,
    /// Send payments over Tor (on by default, as on Lock it Mac).
    tor: bool,
    /// Seconds of "Really open the lock?"; 0 is Don't ask.
    wait: u32,
    /// Ask the keys when Lock it opens and after it locked itself
    /// (key 1, "Opening Zecky" on the Mac). On by default.
    pub(crate) ask_on_start: bool,
    /// The wallet of the key ring on screen (its id); `None` is the main one.
    current: Option<String>,
    /// Names given to wallets after they were added (id → name).
    names: std::collections::BTreeMap<String, String>,
    /// The order of the key ring's chips (ids; "main" for the main wallet).
    order: Vec<String>,
    /// Wallets kept out of sight on the key ring.
    hidden: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { server: None, tor: true, wait: 10, ask_on_start: true, current: None, names: Default::default(), order: Vec::new(), hidden: Vec::new() }
    }
}

const WAITS: [u32; 4] = [10, 5, 3, 0];

fn settings_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(data_dir(app)?.join("settings.json"))
}

pub(crate) fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub(crate) fn save_settings(app: &AppHandle, settings: &Settings) -> CommandResult<()> {
    let json = serde_json::to_string_pretty(settings).map_err(|e| problem(e.to_string()))?;
    fs::write(settings_path(app)?, json).map_err(|e| problem(format!("the settings could not be saved: {e}")))
}

fn server_for(choice: &Option<(String, u16)>) -> Server {
    match choice {
        None => Server::default_public(),
        Some((host, 443)) if lockit_wallet::PUBLIC_SERVERS.contains(&host.as_str()) => Server::Public { host: host.clone() },
        Some((host, port)) => Server::Custom { host: host.clone(), port: *port },
    }
}

/// The server chosen in Settings › Network.
pub(crate) fn chosen_server(app: &AppHandle) -> Server {
    server_for(&load_settings(app).server)
}

/// Where to read from now: Go Max if it has caught up and still holds the
/// blocks this wallet needs, else the chosen server.
fn reading_server(app: &AppHandle, wallet: &Wallet) -> Server {
    let scanned = wallet.balance().map(|b| b.scanned_height).unwrap_or(0);
    match app.state::<GoMaxSlot>().own_node_height() {
        Some(node) if scanned > 0 && scanned + NODE_WINDOW >= node => Server::OwnNode { port: 8137 },
        _ => chosen_server(app),
    }
}

fn set_sync(app: &AppHandle, state: &str, server: &Server, problem: Option<String>) {
    let slot = app.state::<WalletSlot>();
    if let (Some(message), Ok(mut list)) = (problem.as_ref(), slot.problems.lock()) {
        // The same message again within a minute is the same problem.
        let at = now();
        let repeat = list.first().is_some_and(|last| &last.message == message && at - last.at < 60);
        if !repeat {
            list.insert(0, SyncProblem { at, message: message.clone() });
            list.truncate(5);
        }
    }
    let view = SyncView { state: state.into(), server: server.label(), problem };
    if let Ok(mut sync) = slot.sync.lock() {
        *sync = view;
    };
}

/// Starts the background sync: now, then every [`SYNC_EVERY`], and at once
/// when nudged.
pub fn start_syncing(app: AppHandle) {
    let (sender, receiver) = mpsc::channel::<()>();
    if let Ok(mut nudge) = app.state::<WalletSlot>().nudge.lock() {
        *nudge = Some(sender);
    }
    std::thread::Builder::new()
        .name("lockit-sync".into())
        .spawn(move || loop {
            if let Ok(wallet) = wallet(&app) {
                if wallet.exists() {
                    let server = reading_server(&app, &wallet);
                    set_sync(&app, "syncing", &server, None);
                    match runtime().block_on(wallet.sync(&server)) {
                        Ok(()) => {
                            set_sync(&app, "up_to_date", &server, None);
                            if let Ok(balance) = wallet.balance() {
                                crate::rules::note_synced_height(&app, balance.chain_tip, &server);
                            }
                        }
                        Err(error) => set_sync(&app, "problem", &server, Some(error.to_string())),
                    }
                }
            }
            match receiver.recv_timeout(SYNC_EVERY) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            // Several nudges in a row need only one sync.
            while receiver.try_recv().is_ok() {}
        })
        .expect("the sync could not start");
}

pub(crate) fn nudge_slot(slot: &WalletSlot) {
    nudge(slot);
}

fn nudge(slot: &WalletSlot) {
    if let Ok(nudge) = slot.nudge.lock() {
        if let Some(sender) = nudge.as_ref() {
            let _ = sender.send(());
        }
    }
}

/// What the home screen shows.
#[derive(Serialize)]
pub struct Overview {
    exists: bool,
    keys: Vec<KeySummary>,
    balance: Option<Balance>,
    sync: SyncView,
    unsent: bool,
    lock_open: bool,
    problems: Vec<SyncProblem>,
    settings: Settings,
    /// Every wallet on the key ring, the main one first.
    wallets: Vec<lockit_wallet::AccountInfo>,
}

/// The wallet at a glance.
#[tauri::command]
pub async fn wallet_overview(app: AppHandle, slot: State<'_, WalletSlot>) -> CommandResult<Overview> {
    let lock = read_lock(&app)?;
    let keys = lock
        .as_ref()
        .map(|vault| vault.keys.iter().map(|key| KeySummary { kind: key.kind, label: key.label.clone() }).collect())
        .unwrap_or_default();
    let wallet = wallet(&app)?;
    let exists = lock.is_some() && wallet.exists();
    let (balance, wallets) = if exists {
        tauri::async_runtime::spawn_blocking(move || (wallet.balance().ok(), wallet.accounts().unwrap_or_default()))
            .await
            .unwrap_or((None, Vec::new()))
    } else {
        (None, Vec::new())
    };
    let sync = slot.sync.lock().map(|s| s.clone()).unwrap_or_default();
    let unsent = slot.unsent.lock().map(|u| u.is_some()).unwrap_or(false);
    let lock_open = slot.opened.lock().map(|o| o.is_some()).unwrap_or(false);
    let problems = slot.problems.lock().map(|p| p.clone()).unwrap_or_default();
    let settings = load_settings(&app);
    let mut wallets = wallets;
    for wallet in &mut wallets {
        if let Some(name) = settings.names.get(&wallet.id) {
            wallet.name = name.clone();
        }
    }
    Ok(Overview { exists, keys, balance, sync, unsent, lock_open, problems, settings, wallets })
}

/// 24 new words, to be written down. Kept until sealed or forgotten.
#[tauri::command]
pub fn wallet_new_words(slot: State<'_, WalletSlot>) -> CommandResult<Vec<String>> {
    let phrase = lockit_wallet::new_phrase();
    let words = phrase.split(' ').map(str::to_owned).collect();
    *slot.pending.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(phrase);
    Ok(words)
}

/// Checks typed words for a restore and keeps them until sealed.
#[tauri::command]
pub fn wallet_use_words(slot: State<'_, WalletSlot>, words: String) -> CommandResult<()> {
    let words = Zeroizing::new(words);
    lockit_wallet::validate_phrase(&words)?;
    let normalized = Zeroizing::new(words.split_whitespace().map(str::to_lowercase).collect::<Vec<_>>().join(" "));
    *slot.pending.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(normalized);
    Ok(())
}

/// Lets go of words that were not sealed (the owner went back).
#[tauri::command]
pub fn wallet_forget_words(slot: State<'_, WalletSlot>) {
    if let Ok(mut pending) = slot.pending.lock() {
        *pending = None;
    }
}

/// For a restore: the month and year the words were made.
#[derive(Deserialize)]
pub struct MadeIn {
    pub(crate) year: i32,
    pub(crate) month: u32,
}

/// Creates the wallet from the pending words and seals them into a new lock.
///
/// The wallet is made first (it needs the network); if sealing then fails or
/// is cancelled, it is removed again and nothing is left behind.
#[tauri::command]
pub async fn wallet_create(
    app: AppHandle,
    window: WebviewWindow,
    slot: State<'_, WalletSlot>,
    keys: Vec<ChosenKey>,
    made_in: Option<MadeIn>,
) -> CommandResult<()> {
    if read_lock(&app)?.is_some() {
        return Err(problem("there is already a wallet on this PC"));
    }
    let phrase = slot.pending.lock().map_err(|_| problem("the app is in an unknown state"))?.clone()
        .ok_or_else(|| problem("the words are missing; start again"))?;
    #[cfg(windows)]
    let handle = crate::raw_window_handle(&window);
    #[cfg(not(windows))]
    let handle = {
        let _ = &window;
        0
    };
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> CommandResult<()> {
        let wallet = wallet(&app2)?;
        let dir = data_dir(&app2)?.join("wallet");
        if dir.exists() && !wallet.exists() {
            // Left over from an attempt that did not finish.
            let _ = fs::remove_dir_all(&dir);
        }
        let seed = lockit_wallet::phrase_to_seed(&phrase)?;
        let server = chosen_server(&app2);
        let birthday = match made_in {
            Some(made) => {
                let tip = runtime().block_on(Wallet::chain_tip(&server))?;
                Some(lockit_wallet::estimate_birthday(made.year, made.month, tip, now()))
            }
            None => None,
        };
        runtime().block_on(wallet.create(&seed, birthday, &server))?;

        let sealed = (|| -> CommandResult<Vault> {
            let mut enrolled: Vec<(KeyDescriptor, Secret)> = Vec::with_capacity(keys.len());
            for chosen in &keys {
                let mut source = source_for(chosen.kind, chosen.passphrase.as_deref(), handle)?;
                enrolled.push(source.enroll(chosen.label.trim())?);
            }
            Ok(Vault::seal(phrase.as_bytes(), enrolled)?)
        })();
        let vault = match sealed {
            Ok(vault) => vault,
            Err(error) => {
                let _ = fs::remove_dir_all(&dir);
                return Err(error);
            }
        };
        let json = vault.to_json()?;
        fs::write(lock_path(&app2)?, json).map_err(|e| {
            let _ = fs::remove_dir_all(&dir);
            problem(format!("the lock could not be saved: {e}"))
        })?;
        Ok(())
    })
    .await
    .map_err(|e| problem(format!("creating the wallet stopped unexpectedly: {e}")))??;

    wallet_forget_words(slot.clone());
    nudge(&slot);
    Ok(())
}

/// The address to receive on, and its QR code.
#[derive(Serialize)]
pub struct Receive {
    address: String,
    qr: String,
}

/// The shielded address to receive on.
#[tauri::command]
pub async fn wallet_receive(app: AppHandle) -> CommandResult<Receive> {
    let wallet = wallet(&app)?;
    tauri::async_runtime::spawn_blocking(move || -> CommandResult<Receive> {
        let address = wallet.receive_address()?;
        let qr = lockit_wallet::qr_svg(&address)?;
        Ok(Receive { address, qr })
    })
    .await
    .map_err(|e| problem(e.to_string()))?
}

/// The newest payments in and out.
#[tauri::command]
pub async fn wallet_activity(app: AppHandle) -> CommandResult<Vec<Activity>> {
    let wallet = wallet(&app)?;
    tauri::async_runtime::spawn_blocking(move || Ok(wallet.activity(1000)?))
        .await
        .map_err(|e| problem(e.to_string()))?
}

/// A payment worked out, before the lock is opened.
#[derive(Serialize)]
pub struct Prepared {
    to: String,
    amount: u64,
    fee: u64,
    total: u64,
}

/// "1.5", "0.001" or "1,5" in ZEC, to zatoshis. At most 8 decimals.
fn parse_zec(text: &str) -> CommandResult<u64> {
    let text = text.trim().replace(',', ".");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    if whole.is_empty() && fraction.is_empty() || !whole.chars().chain(fraction.chars()).all(|c| c.is_ascii_digit()) {
        return Err(problem("type the amount in ZEC, for example 0.25"));
    }
    if fraction.len() > 8 {
        return Err(problem("ZEC has at most 8 decimals"));
    }
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| problem("that amount is too large"))? };
    let fraction: u64 = format!("{fraction:0<8}").parse().unwrap_or(0);
    let zats = whole.checked_mul(100_000_000).and_then(|w| w.checked_add(fraction)).ok_or_else(|| problem("that amount is too large"))?;
    if zats == 0 {
        return Err(problem("the amount must be more than zero"));
    }
    Ok(zats)
}

/// Works out a payment and its fee. Nothing is signed or sent.
#[tauri::command]
pub async fn wallet_prepare(
    app: AppHandle,
    slot: State<'_, WalletSlot>,
    to: String,
    amount: String,
    memo: Option<String>,
) -> CommandResult<Prepared> {
    if slot.unsent.lock().map(|u| u.is_some()).unwrap_or(false) {
        return Err(problem("a payment is still waiting to be handed to the network; send that one first"));
    }
    let zats = parse_zec(&amount)?;
    if let Some(policy) = read_lock(&app)?.map(|v| v.policy) {
        if policy.only_saved && !policy.contacts.iter().any(|c| c.address == to.trim()) {
            return Err(problem("only saved addresses can be used; pick one from your address book, or change the rule in Settings"));
        }
    }
    let wallet = wallet(&app)?;
    let payment = tauri::async_runtime::spawn_blocking(move || wallet.prepare(&to, zats, memo.as_deref()))
        .await
        .map_err(|e| problem(e.to_string()))??;
    let view = Prepared { to: payment.to.clone(), amount: payment.amount, fee: payment.fee, total: payment.amount + payment.fee };
    *slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(payment);
    Ok(view)
}

/// A payment that the network accepted.
#[derive(Serialize)]
pub struct Sent {
    txids: Vec<String>,
    accepted_by: Vec<String>,
}

pub(crate) async fn broadcast(app: &AppHandle, slot: &WalletSlot, signed: SignedPayment, over_tor: bool) -> CommandResult<Sent> {
    let wallet = wallet(app)?;
    let server = reading_server(app, &wallet);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let outcome = runtime().block_on(wallet.broadcast(&signed, &server, over_tor));
        (signed, outcome)
    })
    .await
    .map_err(|e| problem(e.to_string()))?;
    match result {
        (signed, Ok(accepted_by)) => {
            nudge(slot);
            Ok(Sent { txids: signed.txids, accepted_by })
        }
        (signed, Err(error)) => {
            if let Ok(mut unsent) = slot.unsent.lock() {
                *unsent = Some(signed);
            }
            Err(Problem { unsent: true, ..problem(format!("signed, but not handed to the network yet: {error}")) })
        }
    }
}

/// What the owner typed for the keys that are typed: the passphrase and the
/// Zecky code. Wiped when dropped.
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Typed {
    passphrase: Option<Zeroizing<String>>,
    code: Option<Zeroizing<String>>,
}

impl Typed {
    fn for_kind(&self, kind: lockit_auth::KeyKind) -> Option<&str> {
        match kind {
            lockit_auth::KeyKind::Code => self.code.as_ref().map(|c| c.as_str()),
            _ => self.passphrase.as_ref().map(|p| p.as_str()),
        }
    }
}

/// The lock, opened: the words, every key with its secret (to seal it again
/// with other rules or keys), and its rules.
pub(crate) struct OpenedLock {
    pub phrase: Zeroizing<String>,
    pub keys: Vec<(lockit_auth::KeyDescriptor, lockit_auth::Secret)>,
    pub policy: lockit_auth::Policy,
}

/// Asks every key of the lock and opens it. A shorter time lock whose wait
/// has passed takes over here, sealed into the lock. When "Lost my keys" has
/// run its 48 hours, the security key is not asked and is taken off here.
pub(crate) async fn open_fully(app: &AppHandle, window: &WebviewWindow, typed: Typed) -> CommandResult<OpenedLock> {
    open_lock_inner(app, window, typed, false).await
}

/// Opens the lock without asking a security key: only for starting "Lost my
/// keys", which checks the owner with the other keys.
pub(crate) async fn open_without_security_key(app: &AppHandle, window: &WebviewWindow, typed: Typed) -> CommandResult<OpenedLock> {
    open_lock_inner(app, window, typed, true).await
}

async fn open_lock_inner(app: &AppHandle, window: &WebviewWindow, typed: Typed, skip_check: bool) -> CommandResult<OpenedLock> {
    let vault = read_lock(app)?.ok_or_else(|| problem("there is no wallet on this PC"))?;
    #[cfg(windows)]
    let handle = crate::raw_window_handle(window);
    #[cfg(not(windows))]
    let handle = {
        let _ = window;
        0
    };
    let removal_due = crate::rules::removal_due(app);
    let skip = skip_check || removal_due;
    let mut opened = tauri::async_runtime::spawn_blocking(move || -> CommandResult<OpenedLock> {
        let (payload, keys) = vault.open_with_secrets(|descriptor| {
            if skip && descriptor.kind == lockit_auth::KeyKind::SecurityKey {
                return lockit_auth::keys::key_check::vault_secret(descriptor);
            }
            let mut source = source_for(descriptor.kind, typed.for_kind(descriptor.kind), handle)?;
            source.unlock(descriptor)
        })?;
        let phrase = std::str::from_utf8(&payload).map_err(|_| problem("the lock holds damaged words"))?;
        Ok(OpenedLock { phrase: Zeroizing::new(phrase.to_owned()), keys, policy: vault.policy.clone() })
    })
    .await
    .map_err(|e| problem(format!("asking the keys stopped unexpectedly: {e}")))??;
    let mut changed = false;
    if let (Some(hours), Some(from)) = (opened.policy.pending_hours, opened.policy.pending_from) {
        if from <= now() {
            opened.policy.time_lock_hours = hours;
            opened.policy.pending_hours = None;
            opened.policy.pending_from = None;
            changed = true;
        }
    }
    if removal_due && opened.keys.iter().any(|(k, _)| k.kind == lockit_auth::KeyKind::SecurityKey) {
        opened.keys.retain(|(k, _)| k.kind != lockit_auth::KeyKind::SecurityKey);
        changed = true;
    }
    if changed {
        reseal(app, &opened)?;
    }
    if removal_due {
        crate::rules::finish_removal(app);
    }
    Ok(opened)
}

/// Seals the lock again with the same words, `opened.keys` and
/// `opened.policy`. Written to a new file first, then moved over the old one.
pub(crate) fn reseal(app: &AppHandle, opened: &OpenedLock) -> CommandResult<()> {
    let vault = Vault::seal_with_policy(opened.phrase.as_bytes(), opened.keys.clone(), opened.policy.clone())?;
    let json = vault.to_json()?;
    let path = lock_path(app)?;
    let fresh = path.with_extension("new.json");
    fs::write(&fresh, json).map_err(|e| problem(format!("the lock could not be saved: {e}")))?;
    fs::rename(&fresh, &path).map_err(|e| problem(format!("the lock could not be saved: {e}")))?;
    Ok(())
}

/// Asks every key of the lock and returns the words. Used as the check
/// before showing the words, the viewing key, erasing, and weakening a key.
async fn ask_the_keys(app: &AppHandle, window: &WebviewWindow, typed: Typed) -> CommandResult<Zeroizing<String>> {
    Ok(open_fully(app, window, typed).await?.phrase)
}

/// Opens the lock: every key of the lock is asked. The words stay in memory
/// until the lock closes.
#[tauri::command]
pub async fn wallet_open_lock(
    app: AppHandle,
    window: WebviewWindow,
    slot: State<'_, WalletSlot>,
    typed: Typed,
) -> CommandResult<()> {
    crate::rules::may_open(&app)?;
    let phrase = ask_the_keys(&app, &window, typed).await?;
    *slot.opened.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(phrase);
    Ok(())
}

/// The sync's state and where it reads from, as plain words.
pub(crate) fn sync_state(slot: &WalletSlot) -> (String, String) {
    slot.sync.lock().map(|s| (s.state.clone(), s.server.clone())).unwrap_or_default()
}

/// Keeps the opened words: the lock is open.
pub(crate) fn set_opened(slot: &WalletSlot, phrase: Zeroizing<String>) {
    if let Ok(mut opened) = slot.opened.lock() {
        *opened = Some(phrase);
    }
}

/// Closes the lock: the words are let go.
#[tauri::command]
pub fn wallet_close_lock(slot: State<'_, WalletSlot>) {
    if let Ok(mut opened) = slot.opened.lock() {
        *opened = None;
    }
    if let Ok(mut prepared) = slot.prepared.lock() {
        *prepared = None;
    }
}

pub(crate) fn step(app: &AppHandle, text: &str) {
    let _ = app.emit("send-step", text);
}

/// Signs the prepared payment with the open lock's words and hands it to
/// the network. The lock closes again once the payment is out.
#[tauri::command]
pub async fn wallet_send(app: AppHandle, slot: State<'_, WalletSlot>, over_tor: bool) -> CommandResult<Sent> {
    let phrase = slot.opened.lock().map_err(|_| problem("the app is in an unknown state"))?.clone()
        .ok_or_else(|| problem("the lock closed; open it again to send"))?;
    let payment = slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))?.take()
        .ok_or_else(|| problem("work out the payment again"))?;
    let phrase = match signing_phrase(&app, &phrase) {
        Ok(phrase) => phrase,
        Err(error) => {
            if let Ok(mut prepared) = slot.prepared.lock() {
                *prepared = Some(payment);
            }
            return Err(error);
        }
    };
    let params = sapling_params(&app)?;
    let wallet = wallet(&app)?;
    step(&app, "Signing on this PC");
    let signed = tauri::async_runtime::spawn_blocking(move || -> (PreparedPayment, CommandResult<SignedPayment>) {
        let result = (|| -> CommandResult<SignedPayment> {
            let seed = lockit_wallet::phrase_to_seed(&phrase)?;
            // (`phrase` is already the signing words of the wallet on screen.)
            Ok(wallet.sign(&payment, &seed, &params)?)
        })();
        (payment, result)
    })
    .await
    .map_err(|e| problem(format!("signing stopped unexpectedly: {e}")))?;
    let signed = match signed {
        (_, Ok(signed)) => signed,
        (payment, Err(error)) => {
            // Nothing was signed: keep the payment so "Send" can be tried again.
            if let Ok(mut prepared) = slot.prepared.lock() {
                *prepared = Some(payment);
            }
            return Err(error);
        }
    };
    step(&app, if over_tor { "Connecting over Tor" } else { "Handing it to the network" });
    let sent = broadcast(&app, &slot, signed, over_tor).await;
    if sent.is_ok() {
        wallet_close_lock(slot.clone());
    }
    sent
}

/// Hands a signed payment that did not get through to the network again.
#[tauri::command]
pub async fn wallet_send_again(app: AppHandle, slot: State<'_, WalletSlot>, over_tor: bool) -> CommandResult<Sent> {
    let signed = slot.unsent.lock().map_err(|_| problem("the app is in an unknown state"))?.take()
        .ok_or_else(|| problem("there is no payment waiting"))?;
    broadcast(&app, &slot, signed, over_tor).await
}

/// Works out an internal move: "ironwood" (Orchard into Ironwood) or
/// "lock_in" (transparent Zcash into the lock). Nothing leaves the wallet.
#[tauri::command]
pub async fn wallet_plan_move(app: AppHandle, slot: State<'_, WalletSlot>, kind: String) -> CommandResult<Prepared> {
    if slot.unsent.lock().map(|u| u.is_some()).unwrap_or(false) {
        return Err(problem("a payment is still waiting to be handed to the network; send that one first"));
    }
    let wallet = wallet(&app)?;
    let payment = tauri::async_runtime::spawn_blocking(move || match kind.as_str() {
        "ironwood" => wallet.prepare_ironwood_move(),
        "lock_in" => wallet.prepare_lock_in(),
        _ => Err(lockit_wallet::Error::Other("unknown move".into())),
    })
    .await
    .map_err(|e| problem(e.to_string()))??;
    let view = Prepared { to: payment.to.clone(), amount: payment.amount, fee: payment.fee, total: payment.amount + payment.fee };
    *slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(payment);
    Ok(view)
}

/// Signs the worked-out internal move after the keys and hands it over.
/// The time lock does not apply: nothing leaves the wallet (as on Lock it
/// Mac), but the keys are still asked, because it is signed with the words.
#[tauri::command]
pub async fn wallet_do_move(app: AppHandle, window: WebviewWindow, slot: State<'_, WalletSlot>, typed: Typed) -> CommandResult<Sent> {
    let over_tor = load_settings(&app).tor;
    let payment = slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))?.take()
        .ok_or_else(|| problem("work out the move again"))?;
    let opened = match open_fully(&app, &window, typed).await {
        Ok(opened) => opened,
        Err(error) => {
            if let Ok(mut prepared) = slot.prepared.lock() {
                *prepared = Some(payment);
            }
            return Err(error);
        }
    };
    let params = sapling_params(&app)?;
    let wallet = wallet(&app)?;
    step(&app, "Signing on this PC");
    let phrase = signing_phrase(&app, &opened.phrase)?;
    let signed = tauri::async_runtime::spawn_blocking(move || -> CommandResult<SignedPayment> {
        let seed = lockit_wallet::phrase_to_seed(&phrase)?;
        Ok(wallet.sign(&payment, &seed, &params)?)
    })
    .await
    .map_err(|e| problem(format!("signing stopped unexpectedly: {e}")))??;
    step(&app, if over_tor { "Connecting over Tor" } else { "Handing it to the network" });
    broadcast(&app, &slot, signed, over_tor).await
}

/// Reads a payment request ("zcash:..."): address, amount and note.
#[tauri::command]
pub fn wallet_parse_request(text: String) -> CommandResult<lockit_wallet::PaymentRequest> {
    Ok(lockit_wallet::parse_payment_request(&text)?)
}

/// The price of 1 ZEC in USD, EUR and BTC, from CoinGecko. Only asked when
/// the owner chose a currency (Settings › Display); CoinGecko sees this PC's
/// internet address and a price request, never anything about the wallet.
#[tauri::command]
pub async fn price_get() -> CommandResult<std::collections::HashMap<String, f64>> {
    tauri::async_runtime::spawn_blocking(|| {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
        let json: serde_json::Value = agent
            .get("https://api.coingecko.com/api/v3/simple/price?ids=zcash&vs_currencies=usd,eur,btc")
            .call()
            .map_err(|e| problem(format!("the price could not be fetched: {e}")))?
            .into_json()
            .map_err(|e| problem(format!("the price could not be read: {e}")))?;
        let zcash = json.get("zcash").and_then(|z| z.as_object()).ok_or_else(|| problem("the price could not be read"))?;
        Ok(zcash.iter().filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f))).collect())
    })
    .await
    .map_err(|e| problem(e.to_string()))?
}

/// Looks for new blocks now.
#[tauri::command]
pub fn wallet_sync_now(slot: State<'_, WalletSlot>) {
    nudge(&slot);
}


// ---------- Settings ----------

/// Settings › Network: what is chosen now.
#[derive(Serialize)]
pub struct SettingsView {
    settings: Settings,
    /// Where Lock it reads from at this moment (may be Go Max).
    reading_from: String,
    version: String,
}

/// The owner's settings.
#[tauri::command]
pub fn settings_get(app: AppHandle) -> SettingsView {
    let settings = load_settings(&app);
    let reading_from = app.state::<WalletSlot>().sync.lock().map(|s| s.server.clone()).unwrap_or_default();
    SettingsView { settings, reading_from, version: app.package_info().version.to_string() }
}

/// Chooses the server to read from, after checking that it answers.
/// `host` `None` is the default, zec.rocks.
#[tauri::command]
pub async fn settings_set_server(app: AppHandle, slot: State<'_, WalletSlot>, host: Option<String>, port: Option<u16>) -> CommandResult<String> {
    let choice = match host.map(|h| h.trim().to_owned()).filter(|h| !h.is_empty()) {
        None => None,
        Some(host) => {
            if !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)) {
                return Err(problem("that is not a server address"));
            }
            Some((host.trim_matches(|c| c == '[' || c == ']').to_owned(), port.unwrap_or(443)))
        }
    };
    let server = server_for(&choice);
    let tip = tauri::async_runtime::spawn_blocking(move || {
        runtime().block_on(async { tokio::time::timeout(Duration::from_secs(20), Wallet::chain_tip(&server)).await })
    })
    .await
    .map_err(|e| problem(e.to_string()))?
    .map_err(|_| problem("the server did not answer within 20 seconds; nothing was changed"))??;
    let mut settings = load_settings(&app);
    settings.server = choice;
    save_settings(&app, &settings)?;
    crate::rules::restart_removal_if_running(&app);
    nudge(&slot);
    Ok(format!("Connected. The network is at block {tip}."))
}

/// Sends payments over Tor, or not.
#[tauri::command]
pub fn settings_set_tor(app: AppHandle, on: bool) -> CommandResult<()> {
    let mut settings = load_settings(&app);
    settings.tor = on;
    save_settings(&app, &settings)
}

/// How long "Really open the lock?" waits. Making it longer goes straight
/// through; making it shorter asks the keys first, as on Lock it Mac.
#[tauri::command]
pub async fn settings_set_wait(app: AppHandle, window: WebviewWindow, seconds: u32, typed: Typed) -> CommandResult<()> {
    if !WAITS.contains(&seconds) {
        return Err(problem("choose 10, 5 or 3 seconds, or Don't ask"));
    }
    let mut settings = load_settings(&app);
    let weaker = seconds < settings.wait || (seconds == 0 && settings.wait != 0);
    if weaker {
        drop(ask_the_keys(&app, &window, typed).await?);
        crate::rules::log(&app, if seconds == 0 { "Really open the lock?: don't ask".into() } else { format!("Really open the lock?: {seconds} seconds") });
    } else {
        crate::rules::log(&app, format!("Really open the lock?: {seconds} seconds"));
    }
    settings.wait = seconds;
    save_settings(&app, &settings)
}

/// The 24 words, after every key of the lock was asked. Only for showing.
#[tauri::command]
pub async fn wallet_show_words(app: AppHandle, window: WebviewWindow, typed: Typed, id: Option<String>) -> CommandResult<Vec<String>> {
    let main = ask_the_keys(&app, &window, typed).await?;
    let id = id.or_else(|| load_settings(&app).current);
    let phrase = phrase_of(&app, &main, id.clone())?;
    let birthday = main_wallet(&app)?.for_account(id.as_deref()).balance().map(|b| b.birthday).unwrap_or(0);
    let mut words: Vec<String> = phrase.split(' ').map(str::to_owned).collect();
    words.push(birthday.to_string());
    Ok(words)
}

/// The viewing key, after every key of the lock was asked.
#[tauri::command]
pub async fn wallet_viewing_key(app: AppHandle, window: WebviewWindow, typed: Typed) -> CommandResult<String> {
    drop(ask_the_keys(&app, &window, typed).await?);
    let wallet = wallet(&app)?;
    tauri::async_runtime::spawn_blocking(move || Ok(wallet.viewing_key()?)).await.map_err(|e| problem(e.to_string()))?
}

/// Erases the wallet from this PC: the lock, the wallet's files, its
/// Windows Hello key and the settings. Go Max's node data stays. Asks
/// every key first.
#[tauri::command]
pub async fn wallet_erase(app: AppHandle, window: WebviewWindow, slot: State<'_, WalletSlot>, typed: Typed) -> CommandResult<()> {
    drop(ask_the_keys(&app, &window, typed).await?);
    let vault = read_lock(&app)?;
    wallet_close_lock(slot.clone());
    if let Ok(mut unsent) = slot.unsent.lock() {
        *unsent = None;
    }
    let dir = data_dir(&app)?;
    // The lock goes first: without it the wallet's files are only history.
    fs::remove_file(dir.join(LOCK_FILE)).map_err(|e| problem(format!("the lock could not be removed: {e}")))?;
    let _ = fs::remove_dir_all(dir.join("wallet"));
    let _ = fs::remove_file(dir.join("settings.json"));
    let _ = fs::remove_file(dir.join("keyring.json"));
    crate::firstrun::forget_tour(&app);
    crate::rules::erase(&app);
    #[cfg(windows)]
    if let Some(vault) = vault {
        let hello: Vec<_> = vault.keys.iter().filter(|key| key.kind == lockit_auth::KeyKind::DeviceHello).cloned().collect();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            for key in &hello {
                let _ = lockit_auth::keys::windows_hello::WindowsHelloKey::forget(key);
            }
        })
        .await;
    }
    #[cfg(not(windows))]
    let _ = vault;
    if let Ok(mut problems) = slot.problems.lock() {
        problems.clear();
    }
    Ok(())
}

/// Shows the wallet with this id (`None`: the main one).
pub(crate) fn select(app: &AppHandle, id: Option<String>) -> CommandResult<()> {
    let mut settings = load_settings(app);
    settings.current = id;
    save_settings(app, &settings)
}

/// The worked-out payment, taken for signing.
pub(crate) fn take_prepared(slot: &WalletSlot) -> CommandResult<PreparedPayment> {
    slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))?.take().ok_or_else(|| problem("work out the payment again"))
}

/// Shows another wallet of the key ring; `None` is the main one.
#[tauri::command]
pub async fn keyring_select(app: AppHandle, slot: State<'_, WalletSlot>, id: Option<String>) -> CommandResult<()> {
    let main = main_wallet(&app)?;
    let list = tauri::async_runtime::spawn_blocking(move || main.accounts()).await.map_err(|e| problem(e.to_string()))??;
    let id = match id {
        Some(id) if list.iter().any(|a| a.id == id && !a.main) => Some(id),
        Some(_) if !list.is_empty() => None,
        _ => None,
    };
    if let Ok(mut prepared) = slot.prepared.lock() {
        *prepared = None;
    }
    let mut settings = load_settings(&app);
    settings.current = id;
    save_settings(&app, &settings)
}

/// Adds the pending words (new or typed) as another wallet on the key ring.
/// Every key of the lock is asked: the words are sealed with the main ones.
#[tauri::command]
pub async fn keyring_add(
    app: AppHandle,
    window: WebviewWindow,
    slot: State<'_, WalletSlot>,
    name: String,
    made_in: Option<MadeIn>,
    typed: Typed,
) -> CommandResult<String> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(problem("give the wallet a name of up to 40 letters"));
    }
    let phrase = slot.pending.lock().map_err(|_| problem("the app is in an unknown state"))?.clone()
        .ok_or_else(|| problem("the words are missing; start again"))?;
    let main = ask_the_keys(&app, &window, typed).await?;
    if main.as_str() == phrase.as_str() {
        return Err(problem("these are the main wallet's words"));
    }
    let main_wallet = main_wallet(&app)?;
    let server = chosen_server(&app);
    let app2 = app.clone();
    let id = tauri::async_runtime::spawn_blocking(move || -> CommandResult<String> {
        let main_seed = lockit_wallet::phrase_to_seed(&main)?;
        let sealed = lockit_auth::keyring::seal_words(main_seed.expose_secret(), &phrase)?;
        let seed = lockit_wallet::phrase_to_seed(&phrase)?;
        let birthday = match made_in {
            Some(made) => {
                let tip = runtime().block_on(Wallet::chain_tip(&server))?;
                Some(lockit_wallet::estimate_birthday(made.year, made.month, tip, now()))
            }
            None => None,
        };
        let id = runtime().block_on(main_wallet.add_wallet(&name, &seed, birthday, &server))?;
        let mut ring = load_keyring(&app2);
        ring.retain(|entry| entry.id != id);
        ring.push(RingEntry { id: id.clone(), sealed });
        if let Err(error) = save_keyring(&app2, &ring) {
            let _ = main_wallet.for_account(Some(&id)).remove_account();
            return Err(error);
        }
        Ok(id)
    })
    .await
    .map_err(|e| problem(format!("adding the wallet stopped unexpectedly: {e}")))??;
    wallet_forget_words(slot.clone());
    let mut settings = load_settings(&app);
    settings.current = Some(id.clone());
    save_settings(&app, &settings)?;
    nudge(&slot);
    Ok(id)
}

/// Gives a wallet of the key ring another name.
#[tauri::command]
pub fn keyring_rename(app: AppHandle, id: Option<String>, name: String) -> CommandResult<()> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(problem("give the wallet a name of up to 40 letters"));
    }
    let mut settings = load_settings(&app);
    let id = match id {
        Some(id) => id,
        None => main_wallet(&app)?.accounts()?.into_iter().find(|a| a.main).map(|a| a.id).ok_or_else(|| problem("there is no wallet"))?,
    };
    settings.names.insert(id, name);
    save_settings(&app, &settings)
}

/// The key ring's order and the wallets kept out of sight.
#[tauri::command]
pub fn keyring_layout(app: AppHandle, order: Vec<String>, hidden: Vec<String>) -> CommandResult<()> {
    let mut settings = load_settings(&app);
    settings.order = order.into_iter().take(64).collect();
    settings.hidden = hidden.into_iter().filter(|h| h != "main").take(64).collect();
    save_settings(&app, &settings)
}

/// Erases the main wallet only: the lock passes to the next wallet of the
/// key ring with words on this PC (first in the owner's order), as on Lock
/// it Mac. Every key is asked; the keys and the lock's rules stay.
#[tauri::command]
pub async fn keyring_erase_main(app: AppHandle, window: WebviewWindow, slot: State<'_, WalletSlot>, typed: Typed) -> CommandResult<String> {
    let mut ring = load_keyring(&app);
    let settings = load_settings(&app);
    let successor = settings
        .order
        .iter()
        .filter_map(|id| ring.iter().find(|e| &e.id == id))
        .chain(ring.iter())
        .next()
        .cloned()
        .ok_or_else(|| problem("this is the only wallet with its words on this PC; erase it with Erase everything in Settings › Security"))?;
    let opened = open_fully(&app, &window, typed).await?;
    let main_seed = lockit_wallet::phrase_to_seed(&opened.phrase)?;
    let words = lockit_auth::keyring::open_words(main_seed.expose_secret(), &successor.sealed)?;
    let new_seed = lockit_wallet::phrase_to_seed(&words)?;
    ring.retain(|e| e.id != successor.id);
    let mut moved = Vec::with_capacity(ring.len());
    for entry in &ring {
        let other = lockit_auth::keyring::open_words(main_seed.expose_secret(), &entry.sealed)?;
        moved.push(RingEntry { id: entry.id.clone(), sealed: lockit_auth::keyring::seal_words(new_seed.expose_secret(), &other)? });
    }
    let old_main = main_wallet(&app)?.accounts()?.into_iter().find(|a| a.main).map(|a| a.id);
    // The lock first: from here on it holds the successor's words.
    reseal(&app, &OpenedLock { phrase: words, keys: opened.keys, policy: opened.policy })?;
    save_keyring(&app, &moved)?;
    let main = main_wallet(&app)?;
    let id = successor.id.clone();
    tauri::async_runtime::spawn_blocking(move || main.hand_over_to(&id)).await.map_err(|e| problem(e.to_string()))??;
    let mut settings = load_settings(&app);
    if let Some(old) = old_main {
        settings.names.remove(&old);
    }
    settings.order.retain(|o| o != &successor.id);
    settings.hidden.retain(|o| o != &successor.id);
    settings.current = None;
    save_settings(&app, &settings)?;
    wallet_close_lock(slot.clone());
    Ok(successor.id)
}

/// Removes another wallet from the key ring after every key of the lock.
/// Its words are gone from this PC; the main wallet goes with Erase.
#[tauri::command]
pub async fn keyring_remove(app: AppHandle, window: WebviewWindow, slot: State<'_, WalletSlot>, id: String, typed: Typed) -> CommandResult<()> {
    // A Seal it wallet's keys are on the phone: removing it takes nothing
    // that can spend off this PC, so the keys are not asked (as on the Mac).
    let watch_only = main_wallet(&app)?.accounts()?.iter().any(|a| a.id == id && a.watch_only);
    if !watch_only {
        drop(ask_the_keys(&app, &window, typed).await?);
    }
    let target = main_wallet(&app)?.for_account(Some(&id));
    tauri::async_runtime::spawn_blocking(move || target.remove_account()).await.map_err(|e| problem(e.to_string()))??;
    let mut ring = load_keyring(&app);
    ring.retain(|entry| entry.id != id);
    save_keyring(&app, &ring)?;
    let mut settings = load_settings(&app);
    settings.names.remove(&id);
    settings.order.retain(|o| o != &id);
    settings.hidden.retain(|o| o != &id);
    if settings.current.as_deref() == Some(id.as_str()) {
        settings.current = None;
        wallet_close_lock(slot.clone());
    }
    save_settings(&app, &settings)
}

#[cfg(test)]
mod tests {
    use super::parse_zec;

    #[test]
    fn amounts_are_read_exactly() {
        assert_eq!(parse_zec("1").ok(), Some(100_000_000));
        assert_eq!(parse_zec("0.001").ok(), Some(100_000));
        assert_eq!(parse_zec("1,5").ok(), Some(150_000_000));
        assert_eq!(parse_zec(".00000001").ok(), Some(1));
        assert!(parse_zec("0").is_err());
        assert!(parse_zec("1.000000001").is_err());
        assert!(parse_zec("abc").is_err());
        assert!(parse_zec("").is_err());
    }
}
