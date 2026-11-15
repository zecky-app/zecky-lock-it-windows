//! This PC, as Go Max sees it: the "Before you start Go Max" check, keeping
//! the PC awake while the node catches up, and "Only sync while plugged in".
//!
//! Everything here is read from Windows itself. Nothing is sent anywhere.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
}

/// Go Max's own preferences on this PC.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    /// Only sync while plugged in. `None`: the default, on for laptops.
    plugged_in_only: Option<bool>,
    /// The check was shown once before the first switch-on.
    checked: bool,
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
    let data = app.path().app_local_data_dir().unwrap_or_default().join("go-max");
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

/// Whether a start should wait because the PC is on battery.
pub fn hold_on_battery(app: &AppHandle) -> bool {
    plugged_in_only(app) && platform::power().1
}

/// Watches power every 15 seconds on one thread: keeps the PC awake while
/// the node catches up (the request belongs to this thread), pauses Go Max
/// on battery when the owner wants that, and carries on at the charger.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut awake = false;
        loop {
            let slot = app.state::<GoMaxSlot>();
            let power = app.state::<PowerSlot>();
            let on_battery = platform::power().1;
            let running = slot.running();

            if running && on_battery && plugged_in_only(&app) {
                power.paused.store(true, Ordering::SeqCst);
                let _ = crate::go_max::stop(&app);
            } else if power.paused() && !on_battery && !slot.busy_now() {
                power.paused.store(false, Ordering::SeqCst);
                let _ = crate::go_max::start(&app);
            }

            // Awake while the node checks the chain; let go when it is up to
            // date, paused or off (as Lock it Mac's beginActivity).
            let want = running && slot.catching_up();
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
}
