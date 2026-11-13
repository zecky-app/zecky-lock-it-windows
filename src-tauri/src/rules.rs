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

use lockit_auth::{
    keys::{key_check, passphrase::PassphraseKey},
    protect, KeyDescriptor, KeyKind, KeySource, Policy,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    problem,
    wallet::{data_dir, load_settings, now, open_fully, open_without_security_key, read_lock, reseal, save_settings, Typed, WalletSlot},
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
    /// "Lost my keys", while its 48 hours run.
    removal: Option<Removal>,
    /// The highest block seen while up to date on a built-in server, so a
    /// server that reports less cannot shorten a removal (as the Mac's R1).
    height_seen: u32,
}

/// "Lost my keys": started when, and from which block.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Removal {
    started_at: i64,
    start_height: u32,
}

/// 48 hours of clock, and 48 hours of Zcash blocks.
const REMOVAL_SECONDS: i64 = 48 * HOUR;
/// A server may lag a few blocks behind another; more is suspect.
const BEHIND_TOLERANCE: u32 = 10;
const BLOSSOM: u32 = 653_600;
const NU7: u32 = 3_543_405;

fn spacing(height: u32) -> i64 {
    if height < BLOSSOM { 150 } else if height < NU7 { 75 } else { 25 }
}

/// Target seconds of the blocks after `from`, up to and including `to`.
fn block_seconds(from: u32, to: u32) -> i64 {
    if to <= from {
        return 0;
    }
    let mut total = 0;
    let mut h = from + 1;
    for edge in [BLOSSOM, NU7, u32::MAX] {
        if h > to {
            break;
        }
        let end = to.min(edge.saturating_sub(1));
        if end >= h {
            total += (end - h + 1) as i64 * spacing(h);
            h = end + 1;
        }
    }
    total
}

/// Where a removal starts counting: never below the highest height seen.
/// `None` when the server is clearly behind.
fn removal_start_height(synced: u32, seen: u32) -> Option<u32> {
    if synced == 0 || (seen > 0 && synced + BEHIND_TOLERANCE < seen) {
        return None;
    }
    Some(synced.max(seen))
}

fn removal_is_due(removal: Option<Removal>, at: i64, network: u32) -> bool {
    match removal {
        Some(r) if r.start_height > 0 => at - r.started_at >= REMOVAL_SECONDS && block_seconds(r.start_height, network) >= REMOVAL_SECONDS,
        _ => false,
    }
}

/// Lock it reads from one of the built-in public servers, and is up to
/// date there. A server of one's own could report blocks that do not exist.
fn on_built_in_server(app: &AppHandle) -> bool {
    let (state, server) = crate::wallet::sync_state(&app.state::<WalletSlot>());
    state == "up_to_date" && lockit_wallet::PUBLIC_SERVERS.contains(&server.as_str())
}

fn network_height(app: &AppHandle) -> u32 {
    crate::wallet::wallet(app).ok().and_then(|w| w.balance().ok()).map(|b| b.chain_tip).unwrap_or(0)
}

/// The 48 hours of "Lost my keys" have passed, by the clock and by the
/// blocks of a built-in public server.
pub fn removal_due(app: &AppHandle) -> bool {
    let state = load_state(app);
    state.removal.is_some() && on_built_in_server(app) && removal_is_due(state.removal, now(), network_height(app))
}

/// The keys were taken off the lock: the removal is done.
pub fn finish_removal(app: &AppHandle) {
    let mut state = load_state(app);
    if state.removal.take().is_some() {
        state.height_seen = 0;
        state.log.insert(0, LogEntry { at: now(), text: "Lost my keys: all security keys removed after 48 hours".into() });
        let _ = save_state(app, &state);
    }
}

/// Remembers the highest block seen while up to date on a built-in server,
/// while the lock has security keys.
pub fn note_synced_height(app: &AppHandle, height: u32, server: &lockit_wallet::Server) {
    if !matches!(server, lockit_wallet::Server::Public { .. }) || height == 0 {
        return;
    }
    let has_keys = read_lock(app).ok().flatten().is_some_and(|v| v.keys.iter().any(|k| k.kind == KeyKind::SecurityKey));
    let mut state = load_state(app);
    if has_keys && height > state.height_seen {
        state.height_seen = height;
        let _ = save_state(app, &state);
    }
}

/// Changing the server while a removal runs starts the 48 hours again.
pub fn restart_removal_if_running(app: &AppHandle) {
    let mut state = load_state(app);
    if let Some(r) = state.removal {
        state.removal = Some(Removal { started_at: now(), start_height: r.start_height.max(state.height_seen) });
        state.log.insert(0, LogEntry { at: now(), text: "Lost my keys: the server changed, the 48 hours start again".into() });
        let _ = save_state(app, &state);
    }
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
    /// The paired security keys.
    security_keys: Vec<SecurityKeyView>,
    /// Whether this Windows can use security keys.
    security_keys_available: bool,
    /// "Lost my keys", while it runs.
    removal: Option<RemovalView>,
    /// The address book.
    contacts: Vec<lockit_auth::Contact>,
    /// Only send to saved addresses.
    only_saved: bool,
}

/// One paired security key, for the screens.
#[derive(Serialize)]
pub struct SecurityKeyView {
    id: String,
    role: lockit_auth::keys::key_check::Role,
    added: i64,
}

/// "Lost my keys", for the screens.
#[derive(Serialize)]
pub struct RemovalView {
    started_at: i64,
    /// When 48 hours of clock have passed.
    clock_done_at: i64,
    /// Blocks still to come before 48 hours of blocks have passed.
    blocks_to_go: u32,
    /// Both have passed: the keys come off at the next opening of the lock.
    due: bool,
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
        security_keys: vault
            .as_ref()
            .and_then(|v| v.keys.iter().find(|k| k.kind == KeyKind::SecurityKey).cloned())
            .and_then(|d| key_check::keys_of(&d).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|e| SecurityKeyView { id: e.id, role: e.role, added: e.added })
            .collect(),
        security_keys_available: security_keys_available(),
        contacts: policy.contacts.clone(),
        only_saved: policy.only_saved,
        removal: state.removal.map(|r| {
            let network = network_height(&app);
            let mut target = r.start_height;
            while block_seconds(r.start_height, target) < REMOVAL_SECONDS {
                target += 1;
            }
            RemovalView {
                started_at: r.started_at,
                clock_done_at: r.started_at + REMOVAL_SECONDS,
                blocks_to_go: target.saturating_sub(network),
                due: removal_is_due(Some(r), at, network) && on_built_in_server(&app),
            }
        }),
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
        opened.policy.time_lock_hours = hours;
        opened.policy.pending_hours = None;
        opened.policy.pending_from = None;
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
    opened.policy.time_lock_hours = 0;
    opened.policy.pending_hours = None;
    opened.policy.pending_from = None;
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

fn security_keys_available() -> bool {
    #[cfg(windows)]
    return lockit_auth::keys::security_key::SecurityKey::is_available();
    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
fn key_source(handle: isize) -> lockit_auth::keys::security_key::SecurityKey {
    lockit_auth::keys::security_key::SecurityKey::for_window(handle)
}

/// Adds a security key (or a backup key). With keys already paired, the lock
/// asks one of them first (it opens with every key): otherwise someone could
/// add their own key. Then touch the new key.
#[tauri::command]
pub async fn seckey_add(app: AppHandle, window: WebviewWindow, backup: bool, typed: Typed) -> CommandResult<()> {
    if load_state(&app).removal.is_some() {
        return Err(problem("cancel the key removal first, or wait until it is done"));
    }
    if !security_keys_available() {
        return Err(problem("this Windows cannot use security keys"));
    }
    let mut opened = open_fully(&app, &window, typed).await?;
    #[cfg(windows)]
    {
        let handle = crate::raw_window_handle(&window);
        let role = if backup { key_check::Role::Backup } else { key_check::Role::Main };
        let at = now();
        let entry = tauri::async_runtime::spawn_blocking(move || key_source(handle).pair(role, at))
            .await
            .map_err(|e| problem(e.to_string()))??;
        let position = opened.keys.iter().position(|(k, _)| k.kind == KeyKind::SecurityKey);
        let mut descriptor = match position {
            Some(i) => opened.keys.remove(i).0,
            None => {
                let mut d = KeyDescriptor::new(KeyKind::SecurityKey, "Security key")?;
                d.params.insert("rp_id".into(), key_check::RELYING_PARTY.into());
                d
            }
        };
        let mut list = if position.is_some() { key_check::keys_of(&descriptor)? } else { vec![] };
        if list.iter().any(|k| k.id == entry.id) {
            return Err(problem("this key is already added"));
        }
        list.push(entry);
        key_check::set_keys(&mut descriptor, &list)?;
        let secret = key_check::vault_secret(&descriptor)?;
        opened.keys.push((descriptor, secret));
        reseal(&app, &opened)?;
        log(&app, if backup { "Backup security key added".into() } else { "Security key added".into() });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (&mut opened, backup);
        Err(problem("security keys work on Windows only"))
    }
}

/// Removes one security key. The lock opens with every key, so a paired key
/// is touched first.
#[tauri::command]
pub async fn seckey_remove(app: AppHandle, window: WebviewWindow, id: String, typed: Typed) -> CommandResult<()> {
    let mut opened = open_fully(&app, &window, typed).await?;
    let Some(i) = opened.keys.iter().position(|(k, _)| k.kind == KeyKind::SecurityKey) else {
        return Err(problem("no security key is added"));
    };
    let mut descriptor = opened.keys.remove(i).0;
    let mut list = key_check::keys_of(&descriptor)?;
    list.retain(|k| k.id != id);
    if !list.is_empty() {
        key_check::set_keys(&mut descriptor, &list)?;
        let secret = key_check::vault_secret(&descriptor)?;
        opened.keys.push((descriptor, secret));
    }
    reseal(&app, &opened)?;
    if list.is_empty() {
        let mut state = load_state(&app);
        state.height_seen = 0;
        let _ = save_state(&app, &state);
    }
    log(&app, "Security key removed".into());
    Ok(())
}

/// "Test my key": touch any paired key; its answer is checked.
#[tauri::command]
pub async fn seckey_test(app: AppHandle, window: WebviewWindow) -> CommandResult<String> {
    let descriptor = read_lock(&app)?
        .and_then(|v| v.keys.into_iter().find(|k| k.kind == KeyKind::SecurityKey))
        .ok_or_else(|| problem("no security key is added"))?;
    #[cfg(windows)]
    {
        let handle = crate::raw_window_handle(&window);
        let role = tauri::async_runtime::spawn_blocking(move || key_source(handle).test(&descriptor))
            .await
            .map_err(|e| problem(e.to_string()))??;
        Ok(match role {
            key_check::Role::Main => "Your key answered correctly.".into(),
            key_check::Role::Backup => "Your backup key answered correctly.".into(),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (window, descriptor);
        Err(problem("security keys work on Windows only"))
    }
}

/// "Lost my keys": the other keys of the lock confirm it's you, then all
/// security keys come off after 48 hours of clock and 48 hours of Zcash
/// blocks, counted on a built-in public server. Any paired key can cancel.
#[tauri::command]
pub async fn seckey_removal_start(app: AppHandle, window: WebviewWindow, typed: Typed) -> CommandResult<()> {
    if load_state(&app).removal.is_some() {
        return Ok(());
    }
    if !on_built_in_server(&app) {
        return Err(problem("wait until Lock it is up to date on one of the built-in servers (Settings › Network), then try again"));
    }
    drop(open_without_security_key(&app, &window, typed).await?);
    let mut state = load_state(&app);
    let synced = network_height(&app);
    let Some(start) = removal_start_height(synced, state.height_seen) else {
        return Err(problem("this server is behind what Lock it has seen before; switch back to your usual server, then try again"));
    };
    state.removal = Some(Removal { started_at: now(), start_height: start });
    state.log.insert(0, LogEntry { at: now(), text: "Lost my keys: removal of all security keys started (48 hours)".into() });
    save_state(&app, &state)
}

/// Cancels "Lost my keys": touch any paired key.
#[tauri::command]
pub async fn seckey_removal_cancel(app: AppHandle, window: WebviewWindow) -> CommandResult<()> {
    seckey_test(app.clone(), window).await?;
    let mut state = load_state(&app);
    state.removal = None;
    state.log.insert(0, LogEntry { at: now(), text: "Lost my keys: removal cancelled".into() });
    save_state(&app, &state)
}

// ---------- Address book (Contacts.swift) ----------

const MAX_NAME: usize = 32;

fn clean_name(name: &str) -> CommandResult<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    if name.is_empty() {
        return Err(problem("give the address a name"));
    }
    Ok(name)
}

/// Saves an address. Sealed into the lock, so the keys are asked: nobody
/// can quietly add an address of their own.
#[tauri::command]
pub async fn contacts_add(app: AppHandle, window: WebviewWindow, name: String, address: String, typed: Typed) -> CommandResult<()> {
    let name = clean_name(&name)?;
    let address = address.trim().to_owned();
    if !lockit_wallet::is_valid_address(&address) {
        return Err(problem("that is not a valid Zcash address"));
    }
    if let Some(policy) = read_lock(&app)?.map(|v| v.policy) {
        if let Some(known) = policy.contacts.iter().find(|c| c.address == address) {
            return Err(problem(format!("this address is already saved as {}", known.name)));
        }
    }
    let mut opened = open_fully(&app, &window, typed).await?;
    opened.policy.contacts.push(lockit_auth::Contact { name: name.clone(), address });
    reseal(&app, &opened)?;
    log(&app, format!("Added {name} to the address book"));
    Ok(())
}

/// Renames a saved address.
#[tauri::command]
pub async fn contacts_rename(app: AppHandle, window: WebviewWindow, address: String, name: String, typed: Typed) -> CommandResult<()> {
    let name = clean_name(&name)?;
    let mut opened = open_fully(&app, &window, typed).await?;
    let Some(contact) = opened.policy.contacts.iter_mut().find(|c| c.address == address) else {
        return Err(problem("this address is not in the address book"));
    };
    let old = std::mem::replace(&mut contact.name, name.clone());
    reseal(&app, &opened)?;
    if old != name {
        log(&app, format!("Renamed {old} to {name}"));
    }
    Ok(())
}

/// Removes a saved address.
#[tauri::command]
pub async fn contacts_remove(app: AppHandle, window: WebviewWindow, address: String, typed: Typed) -> CommandResult<()> {
    let mut opened = open_fully(&app, &window, typed).await?;
    let name = opened.policy.contacts.iter().find(|c| c.address == address).map(|c| c.name.clone()).unwrap_or_default();
    opened.policy.contacts.retain(|c| c.address != address);
    reseal(&app, &opened)?;
    log(&app, format!("Removed {name} from the address book"));
    Ok(())
}

/// "Only send to saved addresses", on or off.
#[tauri::command]
pub async fn contacts_only_saved(app: AppHandle, window: WebviewWindow, on: bool, typed: Typed) -> CommandResult<()> {
    let mut opened = open_fully(&app, &window, typed).await?;
    opened.policy.only_saved = on;
    reseal(&app, &opened)?;
    log(&app, if on { "Only send to saved addresses: on".into() } else { "Only send to saved addresses: off".into() });
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
    fn the_removal_counts_48_hours_of_clock_and_of_blocks() {
        // The Mac's numbers: 2,304 blocks of 75 s before NU7, 6,912 after.
        assert_eq!(block_seconds(3_400_000, 3_402_304), REMOVAL_SECONDS);
        assert_eq!(block_seconds(3_600_000, 3_606_912), REMOVAL_SECONDS);
        assert_eq!(block_seconds(3_543_400, 3_543_410), 4 * 75 + 6 * 25);
        let start = Removal { started_at: 0, start_height: 3_600_000 };
        assert!(!removal_is_due(Some(start), REMOVAL_SECONDS - 1, 3_700_000), "the clock is not done");
        assert!(!removal_is_due(Some(start), REMOVAL_SECONDS, 3_606_911), "the blocks are not done");
        assert!(removal_is_due(Some(start), REMOVAL_SECONDS, 3_606_912));
        assert_eq!(removal_start_height(0, 100), None);
        assert_eq!(removal_start_height(80, 100), None, "a server clearly behind");
        assert_eq!(removal_start_height(95, 100), Some(100));
    }

    #[test]
    fn a_shorter_time_lock_waits_for_the_current_one() {
        let t = 1_800_000_000;
        let policy = Policy { time_lock_hours: 168, pending_hours: Some(0), pending_from: Some(t + 100), ..Default::default() };
        assert_eq!(effective(&policy, t), 168);
        assert_eq!(effective(&policy, t + 100), 0);
        assert_eq!(label(72), "3 days");
        assert_eq!(label(24), "24 hours");
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
    }
}
