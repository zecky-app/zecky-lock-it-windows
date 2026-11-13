//! Go Max in the app: start, stop and watch the owner's own node.
//!
//! The node programs ship inside the app, in its resource folder under
//! `node\` (`zakurad.exe`, `zainod.exe`). The chain state lives in the app's
//! local data folder under `go-max\`, never in a roaming profile.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use lockit_node::{GoMax, GoMaxSettings, NodeLayout, NodePrograms, NodeStatus};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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
    },
    /// Starting; `step` says what is happening.
    Starting { step: String },
    /// Running.
    On {
        /// The node's numbers, if it answered this time.
        status: Option<NodeStatus>,
        /// Whether wallets can connect yet.
        wallet_ready: bool,
    },
    /// Stopping.
    Stopping,
}

/// Go Max's place in the app's state.
#[derive(Default)]
pub struct GoMaxSlot {
    node: Mutex<Option<GoMax>>,
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

/// The page's current view. Cheap enough to call every few seconds.
#[tauri::command]
pub fn go_max_view(app: AppHandle, slot: State<'_, GoMaxSlot>) -> GoMaxView {
    if slot.busy.load(Ordering::SeqCst) {
        return match get(&slot.step) {
            Some(step) if step == "Stopping" => GoMaxView::Stopping,
            Some(step) => GoMaxView::Starting { step },
            None => GoMaxView::Starting { step: "Starting the node".into() },
        };
    }
    let Ok(mut guard) = slot.node.lock() else {
        return GoMaxView::Off { installed: installed(&app), problem: Some("Go Max is in an unknown state".into()) };
    };
    match guard.as_mut() {
        Some(node) => match node.check() {
            Ok(()) => GoMaxView::On { status: node.status().ok(), wallet_ready: node.wallet_ready() },
            Err(error) => {
                // A program ended by itself: stop the other one and say why.
                let mut ended = guard.take();
                if let Some(node) = ended.as_mut() {
                    let _ = node.stop();
                }
                set(&slot.problem, Some(error.to_string()));
                GoMaxView::Off { installed: installed(&app), problem: Some(error.to_string()) }
            }
        },
        None => GoMaxView::Off { installed: installed(&app), problem: get(&slot.problem) },
    }
}

/// Starts Go Max. Returns at once; the page follows along with `go_max_view`.
#[tauri::command]
pub fn go_max_start(app: AppHandle, slot: State<'_, GoMaxSlot>) -> Result<(), String> {
    if slot.node.lock().map(|guard| guard.is_some()).unwrap_or(true) {
        return Err("Go Max is already running".into());
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
                if let Ok(mut guard) = slot.node.lock() {
                    *guard = Some(node);
                }
            }
            Err(error) => set(&slot.problem, Some(error.to_string())),
        }
        set(&slot.step, None);
        slot.busy.store(false, Ordering::SeqCst);
        let _ = handle.emit("go-max", "changed");
    });
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

/// Stops Go Max as the app closes. Blocks until both programs have ended.
pub fn stop_on_exit(app: &AppHandle) {
    if let Some(slot) = app.try_state::<GoMaxSlot>() {
        let node = slot.node.lock().ok().and_then(|mut guard| guard.take());
        if let Some(mut node) = node {
            let _ = node.stop();
        }
    }
}
