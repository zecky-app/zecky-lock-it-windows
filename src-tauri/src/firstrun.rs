//! First run, as on Lock it Mac: "Before you start" (the terms, accepted
//! once per terms version) and the short tour on home (shown once).
//! Kept in `first-run.json`, apart from the wallet: nothing secret in it.

use std::fs;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{problem, wallet::data_dir, CommandResult};

/// Raise when the terms change: "Before you start" comes once more.
const TERMS_VERSION: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct FirstRun {
    terms_version: u32,
    terms_at: i64,
    tour_seen: bool,
}

fn load(app: &AppHandle) -> FirstRun {
    data_dir(app)
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join("first-run.json")).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, state: &FirstRun) -> CommandResult<()> {
    let json = serde_json::to_string_pretty(state).map_err(|e| problem(e.to_string()))?;
    fs::write(data_dir(app)?.join("first-run.json"), json).map_err(|e| problem(format!("could not be saved: {e}")))
}

/// What the first run still has to show.
#[derive(Serialize)]
pub struct FirstRunView {
    terms_accepted: bool,
    tour_seen: bool,
}

#[tauri::command]
pub fn first_run(app: AppHandle) -> FirstRunView {
    let state = load(&app);
    FirstRunView { terms_accepted: state.terms_version >= TERMS_VERSION, tour_seen: state.tour_seen }
}

/// "I understand": the terms version and the moment, kept on this PC only.
#[tauri::command]
pub fn terms_accept(app: AppHandle) -> CommandResult<()> {
    let mut state = load(&app);
    state.terms_version = TERMS_VERSION;
    state.terms_at = crate::wallet::now();
    save(&app, &state)
}

/// The tour was finished or skipped (`false`: show it again).
#[tauri::command]
pub fn tour_seen(app: AppHandle, seen: bool) -> CommandResult<()> {
    let mut state = load(&app);
    state.tour_seen = seen;
    save(&app, &state)
}

/// After Erase everything: the tour comes again with the next wallet (as on
/// the Mac); the accepted terms stay.
pub(crate) fn forget_tour(app: &AppHandle) {
    let _ = tour_seen(app.clone(), false);
}

/// Opens the terms or the privacy page in the default browser. Only these
/// two addresses, never one from the page.
#[tauri::command]
pub fn open_page(page: String) -> CommandResult<()> {
    let url = match page.as_str() {
        "terms" => "https://zecky.app/terms.html",
        "privacy" => "https://zecky.app/privacy.html",
        _ => return Err(problem("unknown page")),
    };
    #[cfg(windows)]
    std::process::Command::new("explorer").arg(url).spawn().map_err(|e| problem(format!("the browser could not be opened: {e}")))?;
    #[cfg(not(windows))]
    let _ = url;
    Ok(())
}
