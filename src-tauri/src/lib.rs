//! Zecky Lock it for Windows: the app shell.
//!
//! This build holds two things:
//!
//! * the lock: setting up a lock with one to three keys, and opening it
//!   again. It protects a random *practice secret*, so the keys can be tried
//!   on a real PC without risk;
//! * Go Max: starting, watching and stopping the owner's own pruned node
//!   (see `go_max`).
//!
//! There is no wallet and there are no funds in this build.
//!
//! Rules this file keeps:
//!
//! * The window only draws. Secrets never cross into it: commands return a
//!   short fingerprint, never the secret itself.
//! * System prompts (Windows Hello, "touch your security key") block, so
//!   they run on a worker thread and never freeze the window.

mod go_max;
mod wallet;

use std::{fs, path::PathBuf};

use lockit_auth::{
    keys::passphrase::PassphraseKey, KeyDescriptor, KeyKind, KeySource, Secret, Vault,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, WebviewWindow};
use zeroize::Zeroizing;

const VAULT_FILE: &str = "practice-lock.json";

/// Which kinds of key this computer can use.
#[derive(Serialize)]
struct Availability {
    device_hello: bool,
    security_key: bool,
    passphrase: bool,
}

/// A key as the window shows it.
#[derive(Serialize)]
pub(crate) struct KeySummary {
    pub(crate) kind: KeyKind,
    pub(crate) label: String,
}

/// The lock as the window shows it.
#[derive(Serialize)]
struct LockState {
    exists: bool,
    keys: Vec<KeySummary>,
}

/// One key the owner chose while setting up.
#[derive(Deserialize)]
pub(crate) struct ChosenKey {
    pub(crate) kind: KeyKind,
    pub(crate) label: String,
    /// Only for a passphrase key.
    pub(crate) passphrase: Option<String>,
}

/// What the window gets back after the lock opened.
#[derive(Serialize)]
struct Opened {
    /// First 8 hex characters of SHA-256 of the practice secret: enough to
    /// see that the same secret came back, useless for anything else.
    fingerprint: String,
}

/// An error as the window shows it.
#[derive(Serialize)]
pub(crate) struct Problem {
    cancelled: bool,
    message: String,
    /// A payment was signed but not handed to the network yet; it can be
    /// handed over again without paying twice.
    unsent: bool,
}

impl From<lockit_auth::Error> for Problem {
    fn from(error: lockit_auth::Error) -> Self {
        Self {
            cancelled: matches!(error, lockit_auth::Error::Cancelled),
            message: error.to_string(),
            unsent: false,
        }
    }
}

pub(crate) fn problem(message: impl Into<String>) -> Problem {
    Problem { cancelled: false, message: message.into(), unsent: false }
}

impl From<lockit_wallet::Error> for Problem {
    fn from(error: lockit_wallet::Error) -> Self {
        problem(error.to_string())
    }
}

pub(crate) type CommandResult<T> = Result<T, Problem>;

fn vault_path(app: &AppHandle) -> CommandResult<PathBuf> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| problem(format!("the app's data folder could not be found: {e}")))?;
    fs::create_dir_all(&dir).map_err(|e| problem(format!("the app's data folder could not be created: {e}")))?;
    Ok(dir.join(VAULT_FILE))
}

fn read_vault(app: &AppHandle) -> CommandResult<Option<Vault>> {
    let path = vault_path(app)?;
    match fs::read_to_string(&path) {
        Ok(json) => Ok(Some(Vault::from_json(&json)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(problem(format!("the lock file could not be read: {e}"))),
    }
}

fn fingerprint(secret: &[u8]) -> String {
    Sha256::digest(secret).iter().take(4).map(|byte| format!("{byte:02x}")).collect()
}

/// The raw handle of the app window, so system prompts appear above it.
#[cfg(windows)]
pub(crate) fn raw_window_handle(window: &WebviewWindow) -> isize {
    window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0)
}

/// Builds the source that answers for one kind of key.
pub(crate) fn source_for(
    kind: KeyKind,
    passphrase: Option<&str>,
    #[allow(unused_variables)] window_handle: isize,
) -> Result<Box<dyn KeySource>, lockit_auth::Error> {
    match kind {
        KeyKind::Passphrase => Ok(Box::new(PassphraseKey::new(passphrase.unwrap_or_default()))),
        #[cfg(windows)]
        KeyKind::DeviceHello => Ok(Box::new(lockit_auth::keys::windows_hello::WindowsHelloKey::new())),
        #[cfg(windows)]
        KeyKind::SecurityKey => Ok(Box::new(lockit_auth::keys::security_key::SecurityKey::for_window(
            window_handle,
        ))),
        #[cfg(not(windows))]
        other => Err(lockit_auth::Error::Unavailable {
            kind: other,
            reason: "this kind of key works on Windows only".into(),
        }),
    }
}

/// Which keys this computer offers.
///
/// Runs on a worker thread: Windows refuses to wait for its answers on the
/// window's own thread, and would quietly report "not available".
#[tauri::command]
async fn availability() -> Availability {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(windows)]
        {
            Availability {
                device_hello: lockit_auth::keys::windows_hello::WindowsHelloKey::is_available(),
                security_key: lockit_auth::keys::security_key::SecurityKey::is_available(),
                passphrase: true,
            }
        }
        #[cfg(not(windows))]
        {
            Availability { device_hello: false, security_key: false, passphrase: true }
        }
    })
    .await
    .unwrap_or(Availability { device_hello: false, security_key: false, passphrase: true })
}

#[tauri::command]
fn lock_state(app: AppHandle) -> CommandResult<LockState> {
    let keys = read_vault(&app)?
        .map(|vault| {
            vault
                .keys
                .iter()
                .map(|key| KeySummary { kind: key.kind, label: key.label.clone() })
                .collect::<Vec<_>>()
        });
    Ok(LockState { exists: keys.is_some(), keys: keys.unwrap_or_default() })
}

/// Sets up a practice lock with the chosen keys and seals a random secret.
#[tauri::command]
async fn create_practice_lock(
    app: AppHandle,
    window: WebviewWindow,
    keys: Vec<ChosenKey>,
) -> CommandResult<Opened> {
    if read_vault(&app)?.is_some() {
        return Err(problem("a practice lock already exists. Remove it first."));
    }
    let path = vault_path(&app)?;
    #[cfg(windows)]
    let handle = raw_window_handle(&window);
    #[cfg(not(windows))]
    let handle = {
        let _ = &window;
        0
    };

    tauri::async_runtime::spawn_blocking(move || -> CommandResult<Opened> {
        let mut enrolled: Vec<(KeyDescriptor, Secret)> = Vec::with_capacity(keys.len());
        for chosen in &keys {
            let mut source = source_for(chosen.kind, chosen.passphrase.as_deref(), handle)?;
            enrolled.push(source.enroll(chosen.label.trim())?);
        }

        let secret = Secret::random()?;
        let vault = Vault::seal(secret.expose(), enrolled)?;
        fs::write(&path, vault.to_json()?)
            .map_err(|e| problem(format!("the lock file could not be saved: {e}")))?;
        Ok(Opened { fingerprint: fingerprint(secret.expose()) })
    })
    .await
    .map_err(|e| problem(format!("setting up the lock stopped unexpectedly: {e}")))?
}

/// Opens the lock by asking each of its keys in turn.
#[tauri::command]
async fn open_lock(
    app: AppHandle,
    window: WebviewWindow,
    passphrase: Option<String>,
) -> CommandResult<Opened> {
    let vault = read_vault(&app)?.ok_or_else(|| problem("there is no lock yet"))?;
    #[cfg(windows)]
    let handle = raw_window_handle(&window);
    #[cfg(not(windows))]
    let handle = {
        let _ = &window;
        0
    };

    tauri::async_runtime::spawn_blocking(move || -> CommandResult<Opened> {
        let passphrase = passphrase.map(Zeroizing::new);
        let secret = vault.open_with(|descriptor| {
            let mut source = source_for(
                descriptor.kind,
                passphrase.as_ref().map(|text| text.as_str()),
                handle,
            )?;
            source.unlock(descriptor)
        })?;
        Ok(Opened { fingerprint: fingerprint(&secret) })
    })
    .await
    .map_err(|e| problem(format!("opening the lock stopped unexpectedly: {e}")))?
}

/// Removes the practice lock and, on Windows, its Hello key.
///
/// The lock file is renamed, not erased, so a slip can be undone by hand.
#[tauri::command]
async fn remove_practice_lock(app: AppHandle) -> CommandResult<()> {
    let Some(vault) = read_vault(&app)? else {
        return Ok(());
    };
    let path = vault_path(&app)?;
    let retired = path.with_extension("removed.json");
    fs::rename(&path, &retired).map_err(|e| problem(format!("the lock file could not be set aside: {e}")))?;

    #[cfg(windows)]
    {
        // On a worker thread, for the same reason as `availability`.
        let hello_keys: Vec<_> =
            vault.keys.iter().filter(|key| key.kind == KeyKind::DeviceHello).cloned().collect();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            for key in &hello_keys {
                // Best effort: a Hello key that is already gone is not a problem.
                let _ = lockit_auth::keys::windows_hello::WindowsHelloKey::forget(key);
            }
        })
        .await;
    }
    #[cfg(not(windows))]
    let _ = vault;
    Ok(())
}

/// Starts the app.
pub fn run() {
    let app = tauri::Builder::default()
        // One Lock it at a time: a second copy would fight the first over the
        // node, its ports and its database. Opening Lock it again just brings
        // the running window to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(go_max::GoMaxSlot::default())
        .manage(wallet::WalletSlot::default())
        .setup(|app| {
            wallet::start_syncing(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            availability,
            lock_state,
            create_practice_lock,
            open_lock,
            remove_practice_lock,
            wallet::wallet_overview,
            wallet::wallet_new_words,
            wallet::wallet_use_words,
            wallet::wallet_forget_words,
            wallet::wallet_create,
            wallet::wallet_receive,
            wallet::wallet_activity,
            wallet::wallet_prepare,
            wallet::wallet_send,
            wallet::wallet_send_again,
            wallet::wallet_sync_now,
            go_max::go_max_view,
            go_max::go_max_start,
            go_max::go_max_stop
        ])
        .build(tauri::generate_context!())
        .expect("Zecky Lock it could not start");

    app.run(|handle, event| {
        // Never leave the node running after the window is gone, and give it
        // the chance to stop cleanly first.
        if let tauri::RunEvent::Exit = event {
            go_max::stop_on_exit(handle);
        }
    });
}
