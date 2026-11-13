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
use tauri::{AppHandle, Manager, State, WebviewWindow};
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
}

fn runtime() -> &'static tokio::runtime::Runtime {
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

fn data_dir(app: &AppHandle) -> CommandResult<PathBuf> {
    let dir = app.path().app_local_data_dir().map_err(|e| problem(format!("the app's data folder could not be found: {e}")))?;
    fs::create_dir_all(&dir).map_err(|e| problem(format!("the app's data folder could not be created: {e}")))?;
    Ok(dir)
}

fn wallet(app: &AppHandle) -> CommandResult<Wallet> {
    Ok(Wallet::new(data_dir(app)?.join("wallet")))
}

fn lock_path(app: &AppHandle) -> CommandResult<PathBuf> {
    Ok(data_dir(app)?.join(LOCK_FILE))
}

fn read_lock(app: &AppHandle) -> CommandResult<Option<Vault>> {
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

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Where to read from now: Go Max if it has caught up and still holds the
/// blocks this wallet needs, else the public server.
fn reading_server(app: &AppHandle, wallet: &Wallet) -> Server {
    let scanned = wallet.balance().map(|b| b.scanned_height).unwrap_or(0);
    match app.state::<GoMaxSlot>().own_node_height() {
        Some(node) if scanned > 0 && scanned + NODE_WINDOW >= node => Server::OwnNode { port: 8137 },
        _ => Server::default_public(),
    }
}

fn set_sync(app: &AppHandle, state: &str, server: &Server, problem: Option<String>) {
    if let Ok(mut sync) = app.state::<WalletSlot>().sync.lock() {
        *sync = SyncView { state: state.into(), server: server.label(), problem };
    }
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
                        Ok(()) => set_sync(&app, "up_to_date", &server, None),
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
    let balance = if exists {
        tauri::async_runtime::spawn_blocking(move || wallet.balance().ok()).await.ok().flatten()
    } else {
        None
    };
    let sync = slot.sync.lock().map(|s| s.clone()).unwrap_or_default();
    let unsent = slot.unsent.lock().map(|u| u.is_some()).unwrap_or(false);
    Ok(Overview { exists, keys, balance, sync, unsent })
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
    year: i32,
    month: u32,
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
        let server = Server::default_public();
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
    tauri::async_runtime::spawn_blocking(move || Ok(wallet.activity(30)?))
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

async fn broadcast(app: &AppHandle, slot: &WalletSlot, signed: SignedPayment, over_tor: bool) -> CommandResult<Sent> {
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

/// Opens the lock, signs the prepared payment and hands it to the network.
#[tauri::command]
pub async fn wallet_send(
    app: AppHandle,
    window: WebviewWindow,
    slot: State<'_, WalletSlot>,
    passphrase: Option<String>,
    over_tor: bool,
) -> CommandResult<Sent> {
    let payment = slot.prepared.lock().map_err(|_| problem("the app is in an unknown state"))?.take()
        .ok_or_else(|| problem("work out the payment again"))?;
    let vault = read_lock(&app)?.ok_or_else(|| problem("there is no wallet on this PC"))?;
    let params = sapling_params(&app)?;
    let wallet = wallet(&app)?;
    #[cfg(windows)]
    let handle = crate::raw_window_handle(&window);
    #[cfg(not(windows))]
    let handle = {
        let _ = &window;
        0
    };
    let signed = tauri::async_runtime::spawn_blocking(move || -> (PreparedPayment, CommandResult<SignedPayment>) {
        let passphrase = passphrase.map(Zeroizing::new);
        let opened = vault.open_with(|descriptor| {
            let mut source = source_for(descriptor.kind, passphrase.as_ref().map(|text| text.as_str()), handle)?;
            source.unlock(descriptor)
        });
        let result = (|| -> CommandResult<SignedPayment> {
            let phrase = opened?;
            let phrase = std::str::from_utf8(&phrase).map_err(|_| problem("the lock holds damaged words"))?;
            let seed = lockit_wallet::phrase_to_seed(phrase)?;
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
    broadcast(&app, &slot, signed, over_tor).await
}

/// Hands a signed payment that did not get through to the network again.
#[tauri::command]
pub async fn wallet_send_again(app: AppHandle, slot: State<'_, WalletSlot>, over_tor: bool) -> CommandResult<Sent> {
    let signed = slot.unsent.lock().map_err(|_| problem("the app is in an unknown state"))?.take()
        .ok_or_else(|| problem("there is no payment waiting"))?;
    broadcast(&app, &slot, signed, over_tor).await
}

/// Looks for new blocks now.
#[tauri::command]
pub fn wallet_sync_now(slot: State<'_, WalletSlot>) {
    nudge(&slot);
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
