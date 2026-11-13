//! The lock's rules, as on Lock it Mac (TimeLock.swift, LockRules.swift):
//!
//! * the time lock: opening the lock has to be asked for 24 hours, 3 or 7
//!   days ahead, and can then be opened for 24 hours. A longer time lock
//!   takes effect at once; a shorter one (or off) only after the current
//!   wait. Break the glass opens at once, switches the time lock off and
//!   stays on record. The hours live inside the lock (authenticated with its
//!   header), so they cannot be edited out of the file;
//! * the Zecky code: six digits, a key of the lock on top of Hello or a
//!   passphrase;
//! * asking when Lock it opens ("Opening Zecky" on the Mac);
//! * a record of every change ("The lock" in Activity).
//!
//! When the time lock was asked for, and when the glass was broken, are kept
//! in `lock-state.bin`, protected with Windows DPAPI for this account.
//! Deleting it only makes the lock stricter.

use std::fs;

use lockit_auth::{keys::passphrase::PassphraseKey, protect, KeyKind, KeySource, Policy};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    problem,
    wallet::{data_dir, load_settings, now, open_fully, read_lock, reseal, save_settings, Typed, WalletSlot},
    CommandResult,
};

const HOUR: i64 = 3600;
/// How long the lock can be opened once the wait is over.
const WINDOW: i64 = 24 * HOUR;
/// The choices, in hours, as on the Mac.
const CHOICES: [u32; 4] = [0, 24, 72, 168];
/// The sentence to type before breaking the glass.
const GLASS_SENTENCE: &str = "I am breaking the glass myself";
const STATE_FILE: &str = "lock-state.bin";

/// What is kept outside the lock, protected with DPAPI.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct LockState {
    /// When the lock can be opened (asked for with "Start the wait").
    ready_at: Option<i64>,
    /// Every time the glass was broken, newest first (the last 10).
    broken: Vec<i64>,
    /// Home still has to say the glass was broken.
    broken_notice: bool,
    /// What happened to the lock itself, newest first (the last 50).
    log: Vec<LogEntry>,
}

/// One change to the lock, for Activity › The lock.
#[derive(Clone, Serialize, Deserialize)]
pub struct LogEntry {
    at: i64,
    text: String,
}

fn load_state(app: &AppHandle) -> LockState {
    let Ok(dir) = data_dir(app) else { return LockState::default() };
    fs::read(dir.join(STATE_FILE))
        .ok()
        .and_then(|bytes| protect::unprotect(&bytes).ok())
        .and_then(|plain| serde_json::from_slice(&plain).ok())
        .unwrap_or_default()
}

fn save_state(app: &AppHandle, state: &LockState) -> CommandResult<()> {
    let plain = serde_json::to_vec(state).map_err(|e| problem(e.to_string()))?;
    let sealed = protect::protect(&plain)?;
    fs::write(data_dir(app)?.join(STATE_FILE), sealed).map_err(|e| problem(format!("the lock's state could not be saved: {e}")))
}

/// Adds a line to Activity › The lock.
pub fn log(app: &AppHandle, text: String) {
    let mut state = load_state(app);
    state.log.insert(0, LogEntry { at: now(), text });
    state.log.truncate(50);
    let _ = save_state(app, &state);
}

/// For Erase everything.
pub fn erase(app: &AppHandle) {
    if let Ok(dir) = data_dir(app) {
        let _ = fs::remove_file(dir.join(STATE_FILE));
    }
}

fn label(hours: u32) -> String {
    match hours {
        0 => "Off".into(),
        24 => "24 hours".into(),
        h if h % 24 == 0 => format!("{} days", h / 24),
        h => format!("{h} hours"),
    }
}

/// The time lock in force now: a shorter one takes over once its wait passed.
fn effective(policy: &Policy, at: i64) -> u32 {
    match (policy.pending_hours, policy.pending_from) {
        (Some(hours), Some(from)) if from <= at => hours,
        _ => policy.time_lock_hours,
    }
}

/// Where the time lock stands.
#[derive(Serialize, PartialEq, Debug)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// No time lock.
    Off,
    /// A time lock, and opening was not asked for.
    NotAsked,
    /// Asked for; the lock can be opened from `until`.
    Waiting { until: i64 },
    /// The wait is over; the lock can be opened until `until`.
    Ready { until: i64 },
}

fn phase_at(hours: u32, ready_at: Option<i64>, at: i64) -> Phase {
    if hours == 0 {
        return Phase::Off;
    }
    match ready_at {
        None => Phase::NotAsked,
        Some(ready) if at < ready => Phase::Waiting { until: ready },
        Some(ready) if at <= ready + WINDOW => Phase::Ready { until: ready + WINDOW },
        // A request that was never used lapses after its window.
        Some(_) => Phase::NotAsked,
    }
}

fn current_phase(app: &AppHandle) -> CommandResult<Phase> {
    let policy = read_lock(app)?.map(|v| v.policy).unwrap_or_default();
    Ok(phase_at(effective(&policy, now()), load_state(app).ready_at, now()))
}

/// Refuses to open the lock while the time lock says no.
pub fn may_open(app: &AppHandle) -> CommandResult<()> {
    match current_phase(app)? {
        Phase::Off | Phase::Ready { .. } => Ok(()),
        Phase::Waiting { .. } => Err(problem("the time lock is still running; the lock can be opened once the wait is over")),
        Phase::NotAsked => Err(problem("the time lock is on: ask to open the lock first")),
    }
}

/// Everything the screens need about the lock's rules.
#[derive(Serialize)]
pub struct RulesView {
    /// The time lock in force, in hours (0: off).
    hours: u32,
    /// A shorter time lock waiting to take over, and when.
    pending_hours: Option<u32>,
    pending_from: Option<i64>,
    phase: Phase,
    /// The glass was broken and home still has to say so; when it last was.
    broken_notice: bool,
    last_broken: Option<i64>,
    /// The lock has a Zecky code.
    code: bool,
    /// Ask when Lock it opens.
    ask_on_start: bool,
    /// Which keys "Ask when Lock it opens" uses: Windows Hello alone, or all.
    start_with_hello: bool,
}

/// The lock's rules, for the screens.
#[tauri::command]
pub fn lock_rules(app: AppHandle) -> CommandResult<RulesView> {
    let vault = read_lock(&app)?;
    let policy = vault.as_ref().map(|v| v.policy.clone()).unwrap_or_default();
    let at = now();
    let hours = effective(&policy, at);
    let pending = policy.pending_from.is_some_and(|from| from > at);
    let state = load_state(&app);
    let keys: Vec<KeyKind> = vault.as_ref().map(|v| v.keys.iter().map(|k| k.kind).collect()).unwrap_or_default();
    Ok(RulesView {
        hours,
        pending_hours: if pending { policy.pending_hours } else { None },
        pending_from: if pending { policy.pending_from } else { None },
        phase: phase_at(hours, state.ready_at, at),
        broken_notice: state.broken_notice,
        last_broken: state.broken.first().copied(),
        code: keys.contains(&KeyKind::Code),
        ask_on_start: load_settings(&app).ask_on_start,
        start_with_hello: keys.contains(&KeyKind::DeviceHello),
    })
}

/// Chooses a time lock. Longer: straight away. Shorter or off: after the
/// current wait. Sealed into the lock, so the keys are asked.
#[tauri::command]
pub async fn time_lock_choose(app: AppHandle, window: WebviewWindow, hours: u32, typed: Typed) -> CommandResult<()> {
    if !CHOICES.contains(&hours) {
        return Err(problem("choose off, 24 hours, 3 days or 7 days"));
    }
    let mut opened = open_fully(&app, &window, typed).await?;
    let current = opened.policy.time_lock_hours;
    if hours >= current {
        opened.policy = Policy { time_lock_hours: hours, pending_hours: None, pending_from: None };
        log(&app, if hours == 0 { "Time lock off".into() } else { format!("Time lock on · {}", label(hours).to_lowercase()) });
    } else {
        let from = now() + current as i64 * HOUR;
        opened.policy.pending_hours = Some(hours);
        opened.policy.pending_from = Some(from);
        log(&app, format!("{} from {}", if hours == 0 { "Time lock off".into() } else { format!("Time lock {}", label(hours).to_lowercase()) }, when(from)));
    }
    reseal(&app, &opened)
}

/// Keeps the current time lock: a shorter one waiting to take over is
/// dropped. Stricter, but sealed into the lock, so the keys are asked.
#[tauri::command]
pub async fn time_lock_keep(app: AppHandle, window: WebviewWindow, typed: Typed) -> CommandResult<()> {
    let mut opened = open_fully(&app, &window, typed).await?;
    opened.policy.pending_hours = None;
    opened.policy.pending_from = None;
    reseal(&app, &opened)?;
    log(&app, format!("Kept the {} time lock", label(opened.policy.time_lock_hours).to_lowercase()));
    Ok(())
}

/// "Start the wait": the lock can be opened once the time lock has passed.
#[tauri::command]
pub fn time_lock_request(app: AppHandle) -> CommandResult<()> {
    let policy = read_lock(&app)?.map(|v| v.policy).unwrap_or_default();
    let hours = effective(&policy, now());
    if hours == 0 {
        return Ok(());
    }
    let mut state = load_state(&app);
    state.ready_at = Some(now() + hours as i64 * HOUR);
    save_state(&app, &state)
}

/// "Cancel the request".
#[tauri::command]
pub fn time_lock_cancel_request(app: AppHandle) -> CommandResult<()> {
    let mut state = load_state(&app);
    state.ready_at = None;
    save_state(&app, &state)
}

/// Break the glass: after the typed sentence and the keys, the lock opens
/// straight away and the time lock turns off. It stays on record.
#[tauri::command]
pub async fn break_glass(app: AppHandle, window: WebviewWindow, slot: State<'_, WalletSlot>, sentence: String, typed: Typed) -> CommandResult<()> {
    let typed_sentence = sentence.trim().trim_matches(|c: char| c.is_ascii_punctuation()).to_lowercase();
    if typed_sentence != GLASS_SENTENCE.to_lowercase() {
        return Err(problem(format!("type: {GLASS_SENTENCE}")));
    }
    let mut opened = open_fully(&app, &window, typed).await?;
    let was = effective(&opened.policy, now());
    opened.policy = Policy::default();
    reseal(&app, &opened)?;
    let mut state = load_state(&app);
    state.broken.insert(0, now());
    state.broken.truncate(10);
    state.broken_notice = true;
    state.ready_at = None;
    state.log.insert(0, LogEntry { at: now(), text: format!("Opened past a {} time lock · time lock off", label(was).to_lowercase()) });
    save_state(&app, &state)?;
    crate::wallet::set_opened(&slot, opened.phrase);
    Ok(())
}

/// The home screen has said the glass was broken.
#[tauri::command]
pub fn seen_broken(app: AppHandle) -> CommandResult<()> {
    let mut state = load_state(&app);
    state.broken_notice = false;
    save_state(&app, &state)
}

/// Puts a Zecky code on the lock, or changes it. Asks the keys, and the
/// current code when there is one.
#[tauri::command]
pub async fn code_set(app: AppHandle, window: WebviewWindow, new_code: String, typed: Typed) -> CommandResult<()> {
    let new_code = zeroize::Zeroizing::new(new_code);
    if new_code.len() != 6 || !new_code.chars().all(|c| c.is_ascii_digit()) {
        return Err(problem("a Zecky code is six digits"));
    }
    let mut opened = open_fully(&app, &window, typed).await?;
    let had = opened.keys.iter().any(|(k, _)| k.kind == KeyKind::Code);
    opened.keys.retain(|(k, _)| k.kind != KeyKind::Code);
    let code = new_code.clone();
    let enrolled = tauri::async_runtime::spawn_blocking(move || PassphraseKey::code(code.as_str()).enroll("Zecky code"))
        .await
        .map_err(|e| problem(e.to_string()))??;
    opened.keys.push(enrolled);
    reseal(&app, &opened)?;
    log(&app, if had { "Zecky code changed".into() } else { "Zecky code set".into() });
    Ok(())
}

/// Takes the Zecky code off the lock. Asks the keys and the current code.
#[tauri::command]
pub async fn code_remove(app: AppHandle, window: WebviewWindow, typed: Typed) -> CommandResult<()> {
    let mut opened = open_fully(&app, &window, typed).await?;
    opened.keys.retain(|(k, _)| k.kind != KeyKind::Code);
    reseal(&app, &opened)?;
    log(&app, "Zecky code removed".into());
    Ok(())
}

/// Unlocks the app itself ("Opening Zecky"): Windows Hello when the lock
/// has it, otherwise every key of the lock.
#[tauri::command]
pub async fn app_unlock(app: AppHandle, window: WebviewWindow, typed: Typed) -> CommandResult<()> {
    let vault = read_lock(&app)?.ok_or_else(|| problem("there is no wallet on this PC"))?;
    if let Some(hello) = vault.keys.iter().find(|k| k.kind == KeyKind::DeviceHello).cloned() {
        #[cfg(windows)]
        {
            tauri::async_runtime::spawn_blocking(move || {
                lockit_auth::keys::windows_hello::WindowsHelloKey::new().unlock(&hello).map(drop)
            })
            .await
            .map_err(|e| problem(e.to_string()))??;
            return Ok(());
        }
        #[cfg(not(windows))]
        let _ = hello;
    }
    drop(open_fully(&app, &window, typed).await?);
    Ok(())
}

/// Ask when Lock it opens, or not. Switching it off is a weakening: the
/// keys are asked first.
#[tauri::command]
pub async fn set_ask_on_start(app: AppHandle, window: WebviewWindow, on: bool, typed: Typed) -> CommandResult<()> {
    if !on {
        app_unlock(app.clone(), window, typed).await?;
    }
    let mut settings = load_settings(&app);
    settings.ask_on_start = on;
    save_settings(&app, &settings)?;
    log(&app, if on { "Ask when opening Lock it: on".into() } else { "Ask when opening Lock it: off".into() });
    Ok(())
}

/// Activity › The lock.
#[tauri::command]
pub fn lock_log(app: AppHandle) -> Vec<LogEntry> {
    load_state(&app).log
}

fn when(at: i64) -> String {
    // The window formats dates in the owner's own style; here only for the log.
    let days = at.div_euclid(86_400);
    let (y, m, d) = civil(days);
    let secs = at.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, secs % 3600 / 60)
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_lock_phases_follow_the_mac() {
        let t = 1_800_000_000;
        assert_eq!(phase_at(0, None, t), Phase::Off);
        assert_eq!(phase_at(72, None, t), Phase::NotAsked);
        assert_eq!(phase_at(72, Some(t + 10), t), Phase::Waiting { until: t + 10 });
        assert_eq!(phase_at(72, Some(t - 10), t), Phase::Ready { until: t - 10 + WINDOW });
        assert_eq!(phase_at(72, Some(t - WINDOW - 11), t), Phase::NotAsked, "an unused request lapses");
    }

    #[test]
    fn a_shorter_time_lock_waits_for_the_current_one() {
        let t = 1_800_000_000;
        let policy = Policy { time_lock_hours: 168, pending_hours: Some(0), pending_from: Some(t + 100) };
        assert_eq!(effective(&policy, t), 168);
        assert_eq!(effective(&policy, t + 100), 0);
        assert_eq!(label(72), "3 days");
        assert_eq!(label(24), "24 hours");
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
    }
}
