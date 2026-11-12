//! Zecky Lock it for Windows: the app shell.
//!
//! This first slice holds access and authorization only: setting up a lock
//! with one to three keys, and opening it again. There is no wallet, no
//! network and no funds in this build. The lock protects a random
//! *practice secret*, so the keys can be tried on a real PC without risk.
//!
//! Rules this file keeps:
//!
//! * The window only draws. Secrets never cross into it: commands return a
//!   short fingerprint, never the secret itself.
//! * System prompts (Windows Hello, "touch your security key") block, so
//!   they run on a worker thread and never freeze the window.

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
struct KeySummary {
    kind: KeyKind,
    label: String,
}

/// The lock as the window shows it.
#[derive(Serialize)]
struct LockState {
    exists: bool,
    keys: Vec<KeySummary>,
}

/// One key the owner chose while setting up.
#[derive(Deserialize)]
struct ChosenKey {
    kind: KeyKind,
    label: String,
    /// Only for a passphrase key.
    passphrase: Option<String>,
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
struct Problem {
    cancelled: bool,
    message: String,
}

impl From<lockit_auth::Error> for Problem {
    fn from(error: lockit_auth::Error) -> Self {
        Self {
            cancelled: matches!(error, lockit_auth::Error::Cancelled),
            message: error.to_string(),
        }
    }
}

fn problem(message: impl Into<String>) -> Problem {
    Problem { cancelled: false, message: message.into() }
}

type CommandResult<T> = Result<T, Problem>;

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
fn raw_window_handle(window: &WebviewWindow) -> isize {
    window.hwnd().map(|hwnd| hwnd.0 as isize).unwrap_or(0)
}

/// Builds the source that answers for one kind of key.
fn source_for(
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

#[tauri::command]
fn availability() -> Availability {
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
fn remove_practice_lock(app: AppHandle) -> CommandResult<()> {
    let Some(vault) = read_vault(&app)? else {
        return Ok(());
    };
    let path = vault_path(&app)?;
    let retired = path.with_extension("removed.json");
    fs::rename(&path, &retired).map_err(|e| problem(format!("the lock file could not be set aside: {e}")))?;

    #[cfg(windows)]
    for key in vault.keys.iter().filter(|key| key.kind == KeyKind::DeviceHello) {
        // Best effort: a Hello key that is already gone is not a problem.
        let _ = lockit_auth::keys::windows_hello::WindowsHelloKey::forget(key);
    }
    #[cfg(not(windows))]
    let _ = vault;
    Ok(())
}

/// Starts the app.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            availability,
            lock_state,
            create_practice_lock,
            open_lock,
            remove_practice_lock
        ])
        .run(tauri::generate_context!())
        .expect("Zecky Lock it could not start");
}
