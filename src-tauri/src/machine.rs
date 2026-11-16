//! This PC, as Go Max sees it: the "Before you start Go Max" check, keeping
//! the PC awake while the node catches up, and "Only sync while plugged in".
//!
//! Everything here is read from Windows itself. Nothing is sent anywhere.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use lockit_node::rules::battery;

use crate::go_max::GoMaxSlot;

const GB: u64 = 1_000_000_000;
/// Free space Go Max needs to start (the same as `lockit_node::checks`).
pub const NEEDED_GB: u64 = lockit_node::checks::FIRST_START_BYTES / GB;
/// Below this much memory the PC may feel slow during the first check.
const LOW_MEMORY_GB: f64 = 8.0;

/// The check, as the sheet shows it.
#[derive(Serialize)]
pub struct ComputerCheck {
    /// Free space on the drive that holds the node's data, in GB.
    free_gb: u64,
    /// What Go Max needs.
    needed_gb: u64,
    /// Whether there is enough; the only thing that stops a start.
    enough_space: bool,
    /// Memory, in GB (rounded), if Windows said.
    memory_gb: Option<f64>,
    /// Below 8 GB.
    low_memory: bool,
    /// "ssd", "hdd" or "unknown".
    disk: &'static str,
    /// Whether this PC has a battery (a laptop).
    has_battery: bool,
    /// Running on battery right now.
    on_battery: bool,
    /// Whether Windows marks the internet connection as metered (a data
    /// limit). `None` when Windows did not say.
    metered: Option<bool>,
    /// What the first sync downloads, roughly, in GB.
    download_gb: u64,
    /// The node's data is already here (on battery, an up-to-date node keeps
    /// running).
    has_node_data: bool,
}

/// What a first sync downloads, roughly: the whole chain passes through the
/// node once to be checked, even though only a little is kept.
const DOWNLOAD_GB: u64 = 350;

/// Go Max's own preferences on this PC.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    /// Only sync while plugged in. `None`: the default, on for laptops.
    plugged_in_only: Option<bool>,
    /// The check was shown once before the first switch-on.
    checked: bool,
    /// Where the node's data is kept, if not in the default place.
    data_dir: Option<std::path::PathBuf>,
    /// The node with this data has been up to date at least once: a start on
    /// battery need not wait for the charger.
    ever_up_to_date: bool,
}

/// Note that the node is up to date (written once).
fn mark_up_to_date(app: &AppHandle) {
    let mut prefs = load(app);
    if !prefs.ever_up_to_date {
        prefs.ever_up_to_date = true;
        save(app, &prefs);
    }
}

/// The node's data is gone or starts over: its first check is heavy again.
pub(crate) fn forget_up_to_date(app: &AppHandle) {
    let mut prefs = load(app);
    if prefs.ever_up_to_date {
        prefs.ever_up_to_date = false;
        save(app, &prefs);
    }
}

/// The default place for the node's data: next to the wallet, on the system
/// drive.
fn default_data_dir(app: &AppHandle) -> std::path::PathBuf {
    app.path().app_local_data_dir().unwrap_or_default().join("go-max")
}

/// Where the node's data is kept now.
pub(crate) fn data_dir(app: &AppHandle) -> std::path::PathBuf {
    load(app).data_dir.unwrap_or_else(|| default_data_dir(app))
}

/// Remembers where the node's data is kept (`None`: the default place).
pub(crate) fn set_data_dir(app: &AppHandle, dir: &std::path::Path) {
    let mut prefs = load(app);
    prefs.data_dir = (dir != default_data_dir(app)).then(|| dir.to_path_buf());
    save(app, &prefs);
}

/// Where the node's data would go on the drive `root`: the default place on
/// the system drive, else a folder of its own at the top of that drive.
pub(crate) fn data_dir_on(app: &AppHandle, root: &str) -> std::path::PathBuf {
    if same_drive(&default_data_dir(app), root) {
        default_data_dir(app)
    } else {
        std::path::Path::new(root).join("ZeckyLockIt").join("go-max")
    }
}

fn same_drive(path: &std::path::Path, root: &str) -> bool {
    let path = path.to_string_lossy().to_ascii_lowercase();
    let root = root.to_ascii_lowercase();
    path.starts_with(&root) || (root == "/" && path.starts_with('/'))
}

/// One drive in the list, as the sheet shows it.
#[derive(Serialize)]
pub struct DriveView {
    /// "C:\", as Windows names it.
    root: String,
    /// Its name, if it has one ("Data").
    label: Option<String>,
    /// Free space in GB, counting the node's data if it is here already.
    free_gb: u64,
    /// "ssd", "hdd" or "unknown".
    disk: &'static str,
    /// The drive Windows runs from.
    system: bool,
    /// The node's data is kept here now.
    current: bool,
    /// The node fits.
    fits: bool,
    /// Lock it's recommendation.
    recommended: bool,
    /// The folder the node would use on it.
    path: String,
}

/// The drives to choose from, and where the node's data is now.
#[derive(Serialize)]
pub struct DriveChoice {
    drives: Vec<DriveView>,
    /// Node data already kept (GB), 0 if none.
    data_gb: f64,
    /// Go Max runs: the place cannot change now.
    running: bool,
}

/// The drives Go Max can use: this PC's internal drives, with the one Lock it
/// recommends. Read from Windows on this PC; nothing is sent.
#[tauri::command]
pub fn go_max_drives(app: AppHandle) -> DriveChoice {
    let current = data_dir(&app);
    let used = dir_size(&current);
    let mut drives: Vec<DriveView> = platform::fixed_drives()
        .into_iter()
        .map(|(root, label)| {
            let here = same_drive(&current, &root);
            let free = lockit_node::checks::free_bytes(std::path::Path::new(&root)).unwrap_or(0)
                + if here { used } else { 0 };
            let disk = platform::disk_kind(std::path::Path::new(&root));
            DriveView {
                fits: lockit_node::checks::fits(&lockit_node::checks::DriveFacts { free, ssd: None, system: false })
                    && (here || free >= used),
                free_gb: free / GB,
                disk,
                system: same_drive(&default_data_dir(&app), &root),
                current: here,
                recommended: false,
                path: data_dir_on(&app, &root).display().to_string(),
                label,
                root,
            }
        })
        .collect();
    let facts: Vec<_> = drives
        .iter()
        .map(|d| lockit_node::checks::DriveFacts {
            free: if d.fits { d.free_gb * GB } else { 0 },
            ssd: match d.disk {
                "ssd" => Some(true),
                "hdd" => Some(false),
                _ => None,
            },
            system: d.system,
        })
        .collect();
    if let Some(best) = lockit_node::checks::recommend(&facts) {
        drives[best].recommended = true;
    }
    DriveChoice {
        drives,
        data_gb: (used as f64 / 1e8).round() / 10.0,
        running: app.state::<GoMaxSlot>().running(),
    }
}

fn prefs_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_local_data_dir().ok().map(|d| d.join("go-max-prefs.json"))
}

fn load(app: &AppHandle) -> Prefs {
    prefs_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, prefs: &Prefs) {
    if let (Some(path), Ok(json)) = (prefs_path(app), serde_json::to_string(prefs)) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
}

fn plugged_in_only(app: &AppHandle) -> bool {
    load(app).plugged_in_only.unwrap_or_else(|| platform::power().0)
}

/// Runs the check. Local only.
#[tauri::command]
pub fn go_max_check_computer(app: AppHandle) -> ComputerCheck {
    let data = data_dir(&app);
    // Space already used by the node counts as available to it.
    let used = dir_size(&data);
    let free = lockit_node::checks::free_bytes(&data).unwrap_or(0).saturating_add(used);
    let free_gb = free / GB;
    let memory_gb = platform::memory_bytes().map(|b| (b as f64 / 1_073_741_824.0).round());
    let (has_battery, on_battery) = platform::power();
    ComputerCheck {
        free_gb,
        needed_gb: NEEDED_GB,
        enough_space: free >= lockit_node::checks::FIRST_START_BYTES,
        low_memory: memory_gb.is_some_and(|gb| gb < LOW_MEMORY_GB),
        memory_gb,
        disk: platform::disk_kind(&data),
        has_battery,
        on_battery,
        metered: platform::metered(),
        download_gb: DOWNLOAD_GB,
        has_node_data: used > 0,
    }
}

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Go Max's preferences, for the page.
#[derive(Serialize)]
pub struct GoMaxPrefs {
    plugged_in_only: bool,
    has_battery: bool,
    checked: bool,
    paused: bool,
}

#[tauri::command]
pub fn go_max_prefs(app: AppHandle) -> GoMaxPrefs {
    let prefs = load(&app);
    GoMaxPrefs {
        plugged_in_only: plugged_in_only(&app),
        has_battery: platform::power().0,
        checked: prefs.checked,
        paused: app.state::<PowerSlot>().paused.load(Ordering::SeqCst),
    }
}

#[tauri::command]
pub fn go_max_set_plugged_in_only(app: AppHandle, on: bool) {
    let mut prefs = load(&app);
    prefs.plugged_in_only = Some(on);
    save(&app, &prefs);
}

/// The check was seen (Start Go Max or Not now).
#[tauri::command]
pub fn go_max_checked(app: AppHandle) {
    let mut prefs = load(&app);
    prefs.checked = true;
    save(&app, &prefs);
}

/// Paused on battery (in memory: a restart on battery simply does not start).
#[derive(Default)]
pub struct PowerSlot {
    paused: AtomicBool,
}

impl PowerSlot {
    /// Paused now (a start on battery with Only sync while plugged in).
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Whether Go Max is paused on battery.
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// The owner switched Go Max off or on: no longer paused.
    pub fn clear(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

/// Whether a start should wait because the PC is on battery: only for a node
/// that has never been up to date.
pub fn hold_on_battery(app: &AppHandle) -> bool {
    plugged_in_only(app) && battery::hold_at_start(platform::power().1, load(app).ever_up_to_date)
}

/// Watches power every 15 seconds on one thread: keeps the PC awake while
/// the node catches up (the request belongs to this thread), pauses a
/// catching-up node after about a minute on battery when the owner wants
/// that, and carries on at the charger. Up to date, the node keeps running.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut awake = false;
        let mut on_battery_since: Option<i64> = None;
        loop {
            let slot = app.state::<GoMaxSlot>();
            let power = app.state::<PowerSlot>();
            let on_battery = platform::power().1;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            on_battery_since = if on_battery { on_battery_since.or(Some(now)) } else { None };
            let running = slot.running();
            let settled = running && !slot.busy_now();
            // Asked once a look: it also feeds the expected finish.
            let catching_up = running && slot.catching_up();
            if settled && !catching_up {
                mark_up_to_date(&app);
            }

            // Never while Go Max is between steps (starting, stopping, or
            // moving its data): only a running node is paused.
            if settled && plugged_in_only(&app) && battery::pause(catching_up, on_battery_since, now) {
                power.paused.store(true, Ordering::SeqCst);
                let _ = crate::go_max::stop(&app);
            } else if power.paused() && !on_battery && !slot.busy_now() {
                power.paused.store(false, Ordering::SeqCst);
                let _ = crate::go_max::start(&app);
            }

            // The data overview's counters, on this computer only.
            if running && !slot.busy_now() {
                crate::go_max::record_data(&app, false);
            }

            // Awake while the node checks the chain; let go when it is up to
            // date, paused or off (as Lock it Mac's beginActivity).
            let want = catching_up;
            if want != awake {
                platform::keep_awake(want);
                awake = want;
            }
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    });
}

#[cfg(windows)]
mod platform {
    use std::{os::windows::ffi::OsStrExt, path::Path};

    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::CloseHandle,
            Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
            System::{
                Ioctl::{
                    PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, DEVICE_SEEK_PENALTY_DESCRIPTOR,
                    IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
                },
                Power::{GetSystemPowerStatus, SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SYSTEM_POWER_STATUS},
                SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
                IO::DeviceIoControl,
            },
        },
    };

    /// (has a battery, running on battery).
    pub fn power() -> (bool, bool) {
        let mut status = SYSTEM_POWER_STATUS::default();
        // SAFETY: a valid out-pointer.
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return (false, false);
        }
        let has_battery = status.BatteryFlag != 128 && status.BatteryFlag != 255;
        (has_battery, has_battery && status.ACLineStatus == 0)
    }

    pub fn memory_bytes() -> Option<u64> {
        let mut memory = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        // SAFETY: a valid out-pointer with its length set.
        unsafe { GlobalMemoryStatusEx(&mut memory) }.ok()?;
        Some(memory.ullTotalPhys)
    }

    /// Asks the drive whether it has a seek penalty: a hard drive does, an
    /// SSD does not. Needs no administrator rights (no access asked).
    pub fn disk_kind(path: &Path) -> &'static str {
        let mut letter = path.as_os_str().encode_wide().take(2).collect::<Vec<u16>>();
        if letter.len() != 2 || letter[1] != u16::from(b':') {
            return "unknown";
        }
        let mut name: Vec<u16> = "\\\\.\\".encode_utf16().collect();
        name.append(&mut letter);
        name.push(0);
        // SAFETY: `name` is null-terminated; the handle is closed below.
        let Ok(handle) = (unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }) else {
            return "unknown";
        };
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        let mut answer = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
        let mut returned = 0u32;
        // SAFETY: both buffers live through the call and their sizes are given.
        let asked = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                Some((&mut answer as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR).cast()),
                std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                Some(&mut returned),
                None,
            )
        };
        // SAFETY: the handle came from CreateFileW above.
        unsafe {
            let _ = CloseHandle(handle);
        }
        if asked.is_err() || (returned as usize) < std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() {
            return "unknown";
        }
        if answer.IncursSeekPenalty { "hdd" } else { "ssd" }
    }

    /// Whether the internet connection is metered: Windows' own setting
    /// (Settings → Network → "Metered connection"), set by the user or by a
    /// mobile network. A fixed or variable cost, roaming or over the limit
    /// all count. Only asked on this PC; nothing goes over the network.
    pub fn metered() -> Option<bool> {
        use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
        let profile = NetworkInformation::GetInternetConnectionProfile().ok()?;
        let cost = profile.GetConnectionCost().ok()?;
        let kind = cost.NetworkCostType().ok()?;
        let limited = kind == NetworkCostType::Fixed || kind == NetworkCostType::Variable;
        Some(limited || cost.Roaming().unwrap_or(false) || cost.OverDataLimit().unwrap_or(false))
    }

    /// This PC's internal drives ("C:\\", its label). USB sticks, network
    /// and CD drives are left out: a node there breaks when it goes away.
    pub fn fixed_drives() -> Vec<(String, Option<String>)> {
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
        const DRIVE_FIXED: u32 = 3;
        // SAFETY: no arguments.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .filter_map(|i| {
                let root = format!("{}:\\", char::from(b'A' + i));
                let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
                // SAFETY: `wide` is null-terminated and outlives the call.
                if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } != DRIVE_FIXED {
                    return None;
                }
                let mut name = [0u16; 261];
                // SAFETY: `wide` is null-terminated; `name` is a valid buffer.
                let label = unsafe {
                    GetVolumeInformationW(PCWSTR(wide.as_ptr()), Some(&mut name), None, None, None, None)
                }
                .ok()
                .map(|_| String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)]))
                .filter(|l| !l.trim().is_empty());
                Some((root, label))
            })
            .collect()
    }

    /// Keeps the PC from sleeping (the screen may still turn off), or lets go.
    pub fn keep_awake(on: bool) {
        let flags = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
        // SAFETY: plain flags; applies to the calling thread until changed.
        unsafe {
            SetThreadExecutionState(flags);
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    pub fn power() -> (bool, bool) {
        (false, false)
    }
    pub fn memory_bytes() -> Option<u64> {
        None
    }
    pub fn disk_kind(_: &Path) -> &'static str {
        "unknown"
    }
    pub fn keep_awake(_: bool) {}
    pub fn metered() -> Option<bool> {
        None
    }
    pub fn fixed_drives() -> Vec<(String, Option<String>)> {
        vec![("/".into(), None)]
    }
}
