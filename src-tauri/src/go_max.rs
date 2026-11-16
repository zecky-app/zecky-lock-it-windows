//! Go Max in the app: start, stop and watch the owner's own node.
//!
//! The node programs ship inside the app, in its resource folder under
//! `node\` (`zakurad.exe`, `zainod.exe`). The chain state lives in the app's
//! local data folder under `go-max\`, never in a roaming profile.
//!
//! Phone access over Tor (an onion service for the wallet port, see
//! `lockit_tor`) is built but off until phone pairing exists: see
//! [`PHONE_ACCESS`].

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use lockit_node::rules::{pace, support, words};
use lockit_node::data_use::{self, DataUse};
use lockit_node::{GoMax, GoMaxSettings, NodeLayout, NodePrograms, NodeStatus};
use lockit_tor::{OnionAccess, OnionState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// Whether Go Max opens phone access over Tor. Off since beta 2: no phone app
/// pairs with it yet, so it would only be an open door with no use.
const PHONE_ACCESS: bool = false;

/// Phone access over Tor, as the page shows it.
#[derive(Clone, Serialize)]
pub struct OnionView {
    /// The onion address.
    pub address: String,
    /// How it is doing.
    pub state: OnionState,
}

/// What the Go Max page needs to know at a glance.
#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum GoMaxView {
    /// The node is not running.
    Off {
        /// Whether the node programs are inside this app at all.
        installed: bool,
        /// Why the last run ended, if it did not end by request.
        problem: Option<String>,
        /// The node quit before it first answered: the calm line and Try
        /// again instead of `problem`.
        stopped_while_starting: bool,
        /// Try again is waiting its half minute.
        retrying: bool,
        /// Whether the node's data is on this PC (Remove node and its data…).
        has_data: bool,
        /// This node version and the network's height.
        support: SupportView,
        /// Paused on battery ("Only sync while plugged in").
        paused: bool,
        /// The data overview, if the node has run on this PC.
        data: Option<DataView>,
    },
    /// Starting; `step` says what is happening.
    Starting { step: String },
    /// Running.
    On {
        /// The node's numbers, if it answered this time.
        status: Option<NodeStatus>,
        /// Whether wallets can connect yet.
        wallet_ready: bool,
        /// Why the wallet service is between restarts, if it is.
        wallet_note: Option<String>,
        /// Phone access over Tor, if it started.
        onion: Option<OnionView>,
        /// Why phone access is missing, if it failed to start.
        onion_problem: Option<String>,
        /// Lock it reads older blocks from its server (item: older blocks).
        reading_note: Option<String>,
        /// The wallet stopped hearing of the node's new blocks.
        wallet_quiet: Option<String>,
        /// Lock it's server did not answer when Lock it went back to it.
        unsettled: Option<String>,
        /// This node version and the network's height.
        support: SupportView,
        /// While the first check runs: when it should be done (Unix
        /// seconds), once there is a quarter of an hour of speed to go on.
        finish_at: Option<i64>,
        /// While the first check runs: where it stands against the spam years.
        spam_line: Option<String>,
        /// The data overview.
        data: Option<DataView>,
    },
    /// Stopping.
    Stopping,
}

/// This node version's last block, as the page shows it.
#[derive(Clone, Serialize)]
pub struct SupportView {
    /// Fine, soon (with about when) or ended.
    pub state: support::State,
    /// The line to show, if any.
    pub text: Option<String>,
    /// At the stop: whether Lock it was reading from the node (then it asks).
    pub ask: bool,
    /// The owner's choice at the stop: "server" or "wait".
    pub choice: Option<String>,
    /// About when the stop comes ("Dec 5"), when it is near.
    pub date: Option<String>,
}

/// Remembered across restarts (a public number, nothing about a wallet), so
/// the stop is recognised when the node can no longer say it itself.
#[derive(Default, Serialize, serde::Deserialize)]
#[serde(default)]
struct SupportFile {
    last_height: u32,
    seen_at: i64,
    choice: Option<String>,
    /// Lock it read from the node when the stop came.
    was_on_node: bool,
}

fn support_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("go-max-support.json"))
}

fn load_support(app: &AppHandle) -> SupportFile {
    support_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

fn save_support(app: &AppHandle, file: &SupportFile) {
    if let (Some(path), Ok(json)) = (support_path(app), serde_json::to_string(file)) {
        let _ = std::fs::write(path, json);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn date_text(unix: i64) -> String {
    words::date_text(unix)
}

/// Where this node version stands, judged by the newest network height
/// anything told Lock it. Called by the sync every round with the wallet's
/// server height; the node's own estimate counts only near the tip.
pub(crate) fn support_now(app: &AppHandle, wallet_network: u32) -> SupportView {
    let slot = app.state::<GoMaxSlot>();
    let status = slot.status_now();
    let at = now();
    let live = match &status {
        Some(s) => support::network_height(s.height, s.estimated_height, s.progress, wallet_network, at),
        None => wallet_network,
    };
    let mut file = load_support(app);
    let seen_at = (file.seen_at > 0).then_some(file.seen_at);
    // A height remembered from a guess is replaced, not kept (as on the Mac).
    let remembered = support::trusted_remembered(file.last_height, seen_at, at);
    if live <= support::plausible_max(at) && live > remembered {
        file.last_height = live;
        file.seen_at = at;
        save_support(app, &file);
    }
    let state = if live > 0 {
        support::state(live, None, at, support::STOP_HEIGHT)
    } else {
        support::state(remembered, seen_at, at, support::STOP_HEIGHT)
    };
    let text = match state {
        support::State::Fine => None,
        support::State::Soon { at } => Some(words::update_before(&date_text(at))),
        support::State::Ended => Some(words::ended()),
    };
    let date = match state {
        support::State::Soon { at } => Some(date_text(at)),
        _ => None,
    };
    SupportView { state, text, ask: file.was_on_node && file.choice.is_none(), choice: file.choice, date }
}

/// At the stop: the programs stop and Go Max waits for an update. Lock it's
/// server is left as it is until the owner chooses: nothing switched silently.
pub(crate) fn expire(app: &AppHandle, was_on_node: bool) {
    let slot = app.state::<GoMaxSlot>();
    let running = slot.node.lock().map(|g| g.is_some()).unwrap_or(false);
    let mut file = load_support(app);
    if running && !file.was_on_node {
        file.was_on_node = was_on_node;
        save_support(app, &file);
    }
    if running {
        let _ = stop(app);
    }
}

/// The owner's choice at the stop: "server" (read from Lock it's server
/// again) or "wait" (for the update; no new blocks meanwhile).
pub(crate) fn expired_choice(app: &AppHandle) -> Option<String> {
    let file = load_support(app);
    if file.was_on_node { file.choice.or_else(|| Some("ask".into())) } else { None }
}

/// The owner chose at the stop.
#[tauri::command]
pub fn go_max_choose_at_stop(app: AppHandle, choice: String) -> Result<(), String> {
    if choice != "server" && choice != "wait" {
        return Err("unknown choice".into());
    }
    let mut file = load_support(&app);
    file.choice = Some(choice);
    save_support(&app, &file);
    crate::wallet::nudge_app(&app);
    Ok(())
}

/// Go Max's place in the app's state.
#[derive(Default)]
pub struct GoMaxSlot {
    stopped_while_starting: AtomicBool,
    retrying: AtomicBool,
    node: Mutex<Option<GoMax>>,
    onion: Mutex<Option<OnionAccess>>,
    onion_problem: Mutex<Option<String>>,
    busy: AtomicBool,
    step: Mutex<Option<String>>,
    problem: Mutex<Option<String>>,
    /// When the node first reached the newest block, for the half-minute hold.
    at_tip_since: Mutex<Option<std::time::Instant>>,
    /// The node's height over the last hour, for the expected finish.
    pace: Mutex<pace::Samples>,
    /// The data overview's running totals.
    data: Mutex<DataTally>,
}

/// The running data totals, as loaded from and saved to the node's folder.
#[derive(Default)]
struct DataTally {
    /// Loaded at first use.
    totals: Option<DataUse>,
    /// When the totals were last written to disk.
    saved_at: Option<std::time::Instant>,
    /// The size of the node's folder, measured at most once a minute.
    kept: Option<(u64, std::time::Instant)>,
}

/// The data overview on the Go Max page, in bytes.
#[derive(Clone, Serialize)]
pub struct DataView {
    pub downloaded: u64,
    pub kept: u64,
    pub removed: u64,
    pub uploaded: u64,
}

/// Runs `f` on the totals, loading them first if needed.
fn with_totals<R>(app: &AppHandle, f: impl FnOnce(&mut DataUse, &mut DataTally, &std::path::Path) -> R) -> Option<R> {
    let file = layout(app).ok()?.data_use_file();
    let slot = app.state::<GoMaxSlot>();
    let mut tally = slot.data.lock().ok()?;
    let mut totals = match tally.totals.take() {
        Some(totals) => totals,
        None => DataUse::load(&file),
    };
    let result = f(&mut totals, &mut tally, &file);
    tally.totals = Some(totals);
    Some(result)
}

/// Reads Zakura's counters (on this computer) and adds what is new. Every
/// 15 seconds from the watcher while Go Max runs, and once at a stop.
/// Written to disk at most once a minute, and always when `save` is set.
pub(crate) fn record_data(app: &AppHandle, save: bool) {
    let counters = data_use::read(GoMaxSettings::default().metrics_address());
    with_totals(app, |totals, tally, file| {
        if let Some(counters) = counters {
            totals.add(counters);
        }
        let due = tally.saved_at.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(60));
        if (save || due) && file.parent().is_some_and(|dir| dir.exists()) {
            if totals.save(file).is_ok() {
                tally.saved_at = Some(std::time::Instant::now());
            }
        }
    });
}

/// The overview for the page, if there is anything to show.
fn data_view(app: &AppHandle) -> Option<DataView> {
    let root = layout(app).ok()?.root().to_path_buf();
    with_totals(app, |totals, tally, _| {
        let kept = match tally.kept {
            Some((bytes, at)) if at.elapsed() < std::time::Duration::from_secs(60) => bytes,
            _ => {
                let bytes = data_use::folder_size(&root);
                tally.kept = Some((bytes, std::time::Instant::now()));
                bytes
            }
        };
        (totals.downloaded > 0 || kept > 0).then(|| DataView {
            downloaded: totals.downloaded,
            kept,
            removed: totals.removed(kept),
            uploaded: totals.uploaded,
        })
    })
    .flatten()
}

fn programs(app: &AppHandle) -> Result<NodePrograms, String> {
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    Ok(NodePrograms::in_dir(&resources.join("node")))
}

fn layout(app: &AppHandle) -> Result<NodeLayout, String> {
    Ok(NodeLayout::new(crate::machine::data_dir(app)))
}

/// Keeps the node's data on another drive (Go Max must be off). With data
/// already there, `mode` says what happens to it: "move" copies it, checks
/// the copy and only then removes the old one; "fresh" removes it and the
/// node starts over on the new drive. Returns at once; a move runs on its
/// own and shows on the page.
#[tauri::command]
pub fn go_max_set_data_drive(app: AppHandle, slot: State<'_, GoMaxSlot>, root: String, mode: String) -> Result<(), String> {
    if slot.node.lock().map(|g| g.is_some()).unwrap_or(true) || slot.busy.swap(true, Ordering::SeqCst) {
        return Err("switch Go Max off first".into());
    }
    let from = crate::machine::data_dir(&app);
    let to = crate::machine::data_dir_on(&app, &root);
    let done = |slot: &GoMaxSlot| {
        if let Ok(mut tally) = slot.data.lock() {
            *tally = DataTally::default();
        }
        slot.busy.store(false, Ordering::SeqCst);
    };
    if from == to {
        done(&slot);
        return Ok(());
    }
    let has_data = from.exists() && data_use::folder_size(&from) > 0;
    if !has_data || mode == "fresh" {
        let result = if has_data { std::fs::remove_dir_all(&from) } else { Ok(()) };
        if result.is_ok() {
            crate::machine::set_data_dir(&app, &to);
            crate::machine::forget_up_to_date(&app);
        }
        done(&slot);
        return result.map_err(|e| format!("the old node data could not be removed: {e}"));
    }
    if to.exists() && data_use::folder_size(&to) > 0 {
        done(&slot);
        return Err(format!("{} is not empty", to.display()));
    }
    set(&slot.problem, None);
    let handle = app.clone();
    std::thread::spawn(move || {
        let slot = handle.state::<GoMaxSlot>();
        let total = data_use::folder_size(&from);
        let gb = |bytes: u64| format!("{:.1}", bytes as f64 / 1e9);
        let mut copied = 0u64;
        let mut last = std::time::Instant::now();
        let mut progress = |bytes: u64| {
            copied += bytes;
            if last.elapsed() >= std::time::Duration::from_millis(500) {
                last = std::time::Instant::now();
                set(&slot.step, Some(format!("Moving the node's data · {} of {} GB", gb(copied), gb(total))));
                let _ = handle.emit("go-max", "moving");
            }
        };
        set(&slot.step, Some(format!("Moving the node's data · 0.0 of {} GB", gb(total))));
        let result = copy_tree(&from, &to, &mut progress).and_then(|()| {
            // The copy must be whole before the original goes.
            let copy = data_use::folder_size(&to);
            if copy == total {
                Ok(())
            } else {
                Err(std::io::Error::other(format!("the copy has {copy} bytes, the original {total}")))
            }
        });
        match result {
            Ok(()) => {
                crate::machine::set_data_dir(&handle, &to);
                if std::fs::remove_dir_all(&from).is_err() {
                    set(&slot.problem, Some(format!("Moved. The old copy in {} could not be removed; you can delete it yourself.", from.display())));
                }
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&to);
                set(&slot.problem, Some(format!("The node's data could not be moved ({error}). It stays where it was.")));
            }
        }
        set(&slot.step, None);
        done(&slot);
        let _ = handle.emit("go-max", "changed");
    });
    Ok(())
}

/// Copies a folder with everything in it, telling `progress` the bytes.
fn copy_tree(from: &std::path::Path, to: &std::path::Path, progress: &mut impl FnMut(u64)) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, progress)?;
        } else {
            let mut source = std::fs::File::open(entry.path())?;
            let mut sink = std::fs::File::create(&target)?;
            let mut buffer = vec![0u8; 4 * 1024 * 1024];
            loop {
                let read = std::io::Read::read(&mut source, &mut buffer)?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut sink, &buffer[..read])?;
                progress(read as u64);
            }
            sink.sync_all()?;
        }
    }
    Ok(())
}

fn installed(app: &AppHandle) -> bool {
    programs(app).map(|p| p.zakurad.is_file() && p.zainod.is_file()).unwrap_or(false)
}

fn set(slot: &Mutex<Option<String>>, value: Option<String>) {
    if let Ok(mut guard) = slot.lock() {
        *guard = value;
    }
}

fn get(slot: &Mutex<Option<String>>) -> Option<String> {
    slot.lock().ok().and_then(|guard| guard.clone())
}

/// The page's current view. Cheap enough to call every few seconds: it asks
/// the node on this computer, with short timeouts, and never waits on a stop.
#[tauri::command]
pub fn go_max_view(app: AppHandle, slot: State<'_, GoMaxSlot>) -> GoMaxView {
    let support = support_now(&app, crate::wallet::network_heard(&app));
    let off = |problem: Option<String>| GoMaxView::Off {
        installed: installed(&app),
        stopped_while_starting: slot.stopped_while_starting.load(Ordering::SeqCst),
        retrying: slot.retrying.load(Ordering::SeqCst),
        has_data: layout(&app).map(|l| l.root().exists()).unwrap_or(false),
        support: support.clone(),
        paused: app.state::<crate::machine::PowerSlot>().paused(),
        data: data_view(&app),
        problem,
    };
    if slot.retrying.load(Ordering::SeqCst) {
        return off(None);
    }
    if slot.busy.load(Ordering::SeqCst) {
        return match get(&slot.step) {
            Some(step) if step == "Stopping" => GoMaxView::Stopping,
            Some(step) => GoMaxView::Starting { step },
            None => GoMaxView::Starting { step: "Starting the node".into() },
        };
    }
    let Ok(mut guard) = slot.node.lock() else {
        return off(Some("Go Max is in an unknown state".into()));
    };
    match guard.as_mut() {
        Some(node) => match node.check() {
            Ok(()) => {
                let status = node.status().ok();
                // The network's height: the server's, else the clock's.
                let heard = crate::wallet::network_heard(&app);
                let tip = if heard > 0 { heard } else { support::estimated_tip(now()) };
                let (finish_at, spam_line) = status.as_ref().map_or((None, None), |s| slot.first_check(s, tip));
                GoMaxView::On {
                reading_note: crate::wallet::node_notes(&app).0,
                wallet_quiet: crate::wallet::node_notes(&app).1.then(|| words::WALLET_QUIET.to_owned()),
                unsettled: crate::wallet::node_notes(&app).2,
                support: support.clone(),
                status,
                finish_at,
                spam_line,
                data: data_view(&app),
                wallet_ready: node.wallet_ready(),
                wallet_note: node.wallet_service_note(),
                onion: slot.onion.lock().ok().and_then(|guard| {
                    guard.as_ref().map(|onion| OnionView { address: onion.address().to_owned(), state: onion.state() })
                }),
                onion_problem: get(&slot.onion_problem),
                }
            }
            Err(error) => {
                // A program ended by itself: stop the other one (off the
                // window's thread, it can take a while) and say why.
                let ended = guard.take();
                let onion = slot.onion.lock().ok().and_then(|mut onion| onion.take());
                std::thread::spawn(move || {
                    if let Some(onion) = onion {
                        onion.stop();
                    }
                    if let Some(mut node) = ended {
                        let _ = node.stop();
                    }
                });
                set(&slot.problem, Some(error.to_string()));
                off(Some(error.to_string()))
            }
        },
        None => off(get(&slot.problem)),
    }
}

/// Starts Go Max, as the owner asks. On battery with "Only sync while
/// plugged in", it waits for the charger instead and the page says so.
#[tauri::command]
pub fn go_max_start(app: AppHandle) -> Result<(), String> {
    let power = app.state::<crate::machine::PowerSlot>();
    power.clear();
    if crate::machine::hold_on_battery(&app) {
        power.pause();
        return Ok(());
    }
    start(&app)
}

/// Stops Go Max, as the owner asks (no longer paused either).
#[tauri::command]
pub fn go_max_stop(app: AppHandle) -> Result<(), String> {
    app.state::<crate::machine::PowerSlot>().clear();
    stop(&app)
}

/// Starts Go Max. Returns at once; the page follows along with `go_max_view`.
pub(crate) fn start(app: &AppHandle) -> Result<(), String> {
    let slot = app.state::<GoMaxSlot>();
    let app = app.clone();
    if slot.node.lock().map(|guard| guard.is_some()).unwrap_or(true) {
        return Err("Go Max is already running".into());
    }
    // Past this node version's last block the node would not run: wait for
    // the update (as on the Mac).
    if support_now(&app, crate::wallet::network_heard(&app)).state == support::State::Ended {
        return Err(words::ended());
    }
    if slot.busy.swap(true, Ordering::SeqCst) {
        return Err("Go Max is busy".into());
    }
    let programs = match programs(&app) {
        Ok(programs) => programs,
        Err(error) => {
            slot.busy.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };
    let layout = match layout(&app) {
        Ok(layout) => layout,
        Err(error) => {
            slot.busy.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };
    set(&slot.problem, None);
    slot.stopped_while_starting.store(false, Ordering::SeqCst);

    let handle = app.clone();
    std::thread::spawn(move || {
        let slot = handle.state::<GoMaxSlot>();
        // A new Zakura: its counters begin at zero again.
        with_totals(&handle, |totals, _, _| totals.new_process());
        let mut node = GoMax::new(layout, GoMaxSettings::default(), programs);
        let started = node.start(|step| {
            set(&slot.step, Some(step.to_owned()));
            let _ = handle.emit("go-max", step);
        });
        match started {
            Ok(()) => {
                // Phone access over Tor. Off until phone pairing exists:
                // nothing on this PC listens for outside connections. If it
                // cannot start, Go Max still runs for this computer, and the
                // page says why.
                if PHONE_ACCESS {
                    set(&slot.step, Some("Making your private address".into()));
                    let layout = node.layout().clone();
                    let port = node.settings().wallet_port;
                    match OnionAccess::start(&layout.tor_state(), &layout.tor_cache(), port) {
                        Ok(onion) => {
                            if let Ok(mut guard) = slot.onion.lock() {
                                *guard = Some(onion);
                            }
                            set(&slot.onion_problem, None);
                        }
                        Err(error) => set(&slot.onion_problem, Some(error.to_string())),
                    }
                }
                if let Ok(mut guard) = slot.node.lock() {
                    *guard = Some(node);
                }
            }
            Err(error) => {
                // The node quit before it first answered: calm help, not
                // the raw error (as on the Mac).
                let quit = matches!(error, lockit_node::Error::Exited { .. } | lockit_node::Error::NotReady(_));
                slot.stopped_while_starting.store(quit, Ordering::SeqCst);
                set(&slot.problem, Some(error.to_string()));
            }
        }
        set(&slot.step, None);
        slot.busy.store(false, Ordering::SeqCst);
        let _ = handle.emit("go-max", "changed");
    });
    Ok(())
}

/// After "The node stopped while starting": stop what is left, give the node
/// from before half a minute to finish closing, then start again.
#[tauri::command]
pub fn go_max_try_again(app: AppHandle, slot: State<'_, GoMaxSlot>) -> Result<(), String> {
    if slot.retrying.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        let slot = handle.state::<GoMaxSlot>();
        stop_onion(&slot);
        if let Some(mut node) = slot.node.lock().ok().and_then(|mut g| g.take()) {
            let _ = node.stop();
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
        slot.retrying.store(false, Ordering::SeqCst);
        slot.stopped_while_starting.store(false, Ordering::SeqCst);
        let _ = start(&handle);
    });
    Ok(())
}

/// Removes the node and its data (Go Max must be off). It starts over at the
/// next switch-on. The wallet is not touched.
#[tauri::command]
pub fn go_max_remove_data(app: AppHandle, slot: State<'_, GoMaxSlot>) -> Result<(), String> {
    if slot.busy.load(Ordering::SeqCst) || slot.node.lock().map(|g| g.is_some()).unwrap_or(true) {
        return Err("switch Go Max off first".into());
    }
    let root = layout(&app)?.root().to_path_buf();
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| format!("the node's data could not be removed: {e}"))?;
    }
    // The totals went with the folder, and the next start is a first check.
    if let Ok(mut tally) = slot.data.lock() {
        *tally = DataTally::default();
    }
    crate::machine::forget_up_to_date(&app);
    slot.stopped_while_starting.store(false, Ordering::SeqCst);
    set(&slot.problem, None);
    Ok(())
}

/// Stops Go Max cleanly. Returns at once.
pub(crate) fn stop(app: &AppHandle) -> Result<(), String> {
    let slot = app.state::<GoMaxSlot>();
    let app = app.clone();
    if slot.busy.swap(true, Ordering::SeqCst) {
        return Err("Go Max is busy".into());
    }
    slot.forget_pace();
    let node = slot.node.lock().ok().and_then(|mut guard| guard.take());
    let Some(mut node) = node else {
        slot.busy.store(false, Ordering::SeqCst);
        return Ok(());
    };
    set(&slot.step, Some("Stopping".into()));
    let handle = app.clone();
    std::thread::spawn(move || {
        let slot = handle.state::<GoMaxSlot>();
        record_data(&handle, true);
        stop_onion(&slot);
        match node.stop() {
            Ok(report) if !report.clean() => set(
                &slot.problem,
                Some("The node had to be stopped hard. It will check its data at the next start.".into()),
            ),
            Ok(_) => {}
            Err(error) => set(&slot.problem, Some(error.to_string())),
        }
        set(&slot.step, None);
        slot.busy.store(false, Ordering::SeqCst);
        let _ = handle.emit("go-max", "changed");
    });
    Ok(())
}

fn stop_onion(slot: &GoMaxSlot) {
    let onion = slot.onion.lock().ok().and_then(|mut guard| guard.take());
    if let Some(onion) = onion {
        onion.stop();
    }
}

/// Stops Go Max as the app closes. Blocks until both programs have ended.
pub fn stop_on_exit(app: &AppHandle) {
    if let Some(slot) = app.try_state::<GoMaxSlot>() {
        stop_onion(&slot);
        let node = slot.node.lock().ok().and_then(|mut guard| guard.take());
        if let Some(mut node) = node {
            let _ = node.stop();
        }
    }
}

impl GoMaxSlot {
    /// Whether the node runs or is starting.
    pub fn running(&self) -> bool {
        self.busy.load(Ordering::SeqCst) && get(&self.step).as_deref() != Some("Stopping")
            || self.node.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Whether Go Max is between steps (starting or stopping).
    pub fn busy_now(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Whether the node is still checking the chain: keep the PC awake.
    pub fn catching_up(&self) -> bool {
        if self.busy.load(Ordering::SeqCst) {
            return true;
        }
        match self.status_now() {
            Some(status) => {
                let network = status.estimated_height.min(support::plausible_max(now()));
                let catching_up =
                    status.progress < 0.999 || status.height.saturating_add(words::UP_TO_DATE_BLOCKS) < network;
                // Every 15 seconds from the watcher: the speed behind the
                // expected finish, also while the page is closed.
                if let Ok(mut pace) = self.pace.lock() {
                    if catching_up {
                        pace.add(now(), status.height);
                    } else {
                        pace.clear();
                    }
                }
                catching_up
            }
            None => self.node.lock().map(|g| g.is_some()).unwrap_or(false),
        }
    }

    /// Forget the measured speed: the node was switched off or paused.
    pub fn forget_pace(&self) {
        if let Ok(mut pace) = self.pace.lock() {
            pace.clear();
        }
    }

    /// The expected finish and the spam-years line, while the first check
    /// runs. `tip` is the network's height.
    fn first_check(&self, status: &NodeStatus, tip: u32) -> (Option<i64>, Option<String>) {
        if tip == 0 || status.height.saturating_add(words::UP_TO_DATE_BLOCKS) >= tip {
            return (None, None);
        }
        let finish = self.pace.lock().ok().and_then(|pace| pace.finish(tip));
        (finish, Some(pace::spam_line(status.height)))
    }

    /// The node's numbers now, if it runs and answers. Never waits.
    pub fn status_now(&self) -> Option<NodeStatus> {
        if self.busy.load(Ordering::SeqCst) {
            return None;
        }
        let guard = self.node.try_lock().ok()?;
        guard.as_ref()?.status().ok()
    }

    /// The node's height, if wallets can read from it now: the wallet
    /// service answers and the node has caught up with the network. Never
    /// waits for a busy Go Max.
    pub fn own_node_height(&self) -> Option<u32> {
        if self.busy.load(Ordering::SeqCst) {
            return None;
        }
        let guard = self.node.try_lock().ok()?;
        let node = guard.as_ref()?;
        if !node.wallet_ready() {
            return None;
        }
        let status = node.status().ok()?;
        let at_tip = status.estimated_height > 0 && status.height + 2 >= status.estimated_height;
        // Lock it moves to the node only once it has stayed at the newest
        // block for half a minute (as on the Mac): no back and forth while
        // it is just arriving.
        let mut since = self.at_tip_since.lock().ok()?;
        if !at_tip {
            *since = None;
            return None;
        }
        let first = *since.get_or_insert_with(std::time::Instant::now);
        (first.elapsed() >= std::time::Duration::from_secs(30)).then_some(status.height)
    }
}
