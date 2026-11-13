//! Go Max in the app: start, stop and watch the owner's own node.
//!
//! The node programs ship inside the app, in its resource folder under
//! `node\` (`zakurad.exe`, `zainod.exe`). The chain state lives in the app's
//! local data folder under `go-max\`, never in a roaming profile.
//!
//! While Go Max runs, an onion service gives the wallet port a private
//! address a paired phone can reach from anywhere (see `lockit_tor`).

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use lockit_node::rules::{support, words};
use lockit_node::{GoMax, GoMaxSettings, NodeLayout, NodePrograms, NodeStatus};
use lockit_tor::{OnionAccess, OnionState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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
    let live = match &status {
        Some(s) => support::network_height(s.height, s.estimated_height, s.progress, wallet_network),
        None => wallet_network,
    };
    let mut file = load_support(app);
    if live > file.last_height {
        file.last_height = live;
        file.seen_at = now();
        save_support(app, &file);
    }
    let state = if live > 0 {
        support::state(live, None, now(), support::STOP_HEIGHT)
    } else {
        support::state(file.last_height, (file.seen_at > 0).then_some(file.seen_at), now(), support::STOP_HEIGHT)
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
        let _ = go_max_stop(app.clone(), slot);
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
}

fn programs(app: &AppHandle) -> Result<NodePrograms, String> {
    let resources = app.path().resource_dir().map_err(|e| e.to_string())?;
    Ok(NodePrograms::in_dir(&resources.join("node")))
}

fn layout(app: &AppHandle) -> Result<NodeLayout, String> {
    let data = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    Ok(NodeLayout::new(data.join("go-max")))
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
            Ok(()) => GoMaxView::On {
                reading_note: crate::wallet::node_notes(&app).0,
                wallet_quiet: crate::wallet::node_notes(&app).1.then(|| words::WALLET_QUIET.to_owned()),
                unsettled: crate::wallet::node_notes(&app).2,
                support: support.clone(),
                status: node.status().ok(),
                wallet_ready: node.wallet_ready(),
                wallet_note: node.wallet_service_note(),
                onion: slot.onion.lock().ok().and_then(|guard| {
                    guard.as_ref().map(|onion| OnionView { address: onion.address().to_owned(), state: onion.state() })
                }),
                onion_problem: get(&slot.onion_problem),
            },
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

/// Starts Go Max. Returns at once; the page follows along with `go_max_view`.
#[tauri::command]
pub fn go_max_start(app: AppHandle, slot: State<'_, GoMaxSlot>) -> Result<(), String> {
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
        let mut node = GoMax::new(layout, GoMaxSettings::default(), programs);
        let started = node.start(|step| {
            set(&slot.step, Some(step.to_owned()));
            let _ = handle.emit("go-max", step);
        });
        match started {
            Ok(()) => {
                // Phone access over Tor. If it cannot start, Go Max still runs
                // for this computer, and the page says why.
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
        let _ = go_max_start(handle.clone(), handle.state::<GoMaxSlot>());
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
    slot.stopped_while_starting.store(false, Ordering::SeqCst);
    set(&slot.problem, None);
    Ok(())
}

/// Stops Go Max cleanly. Returns at once.
#[tauri::command]
pub fn go_max_stop(app: AppHandle, slot: State<'_, GoMaxSlot>) -> Result<(), String> {
    if slot.busy.swap(true, Ordering::SeqCst) {
        return Err("Go Max is busy".into());
    }
    let node = slot.node.lock().ok().and_then(|mut guard| guard.take());
    let Some(mut node) = node else {
        slot.busy.store(false, Ordering::SeqCst);
        return Ok(());
    };
    set(&slot.step, Some("Stopping".into()));
    let handle = app.clone();
    std::thread::spawn(move || {
        let slot = handle.state::<GoMaxSlot>();
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
        (status.estimated_height > 0 && status.height + 2 >= status.estimated_height).then_some(status.height)
    }
}
