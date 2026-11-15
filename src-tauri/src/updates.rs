//! "Check for updates": asks GitHub for the newest Lock it for Windows, only
//! when the owner presses the button. Never by itself, and it never
//! downloads anything: a newer version is shown with a link to
//! zecky.app/windows.html. GitHub sees this PC's internet address.

use serde::Serialize;
use tauri::AppHandle;

use crate::{problem, CommandResult};

const RELEASES: &str = "https://api.github.com/repos/zecky-app/zecky-lock-it-windows/releases?per_page=10";

/// The answer, as Settings › About shows it.
#[derive(Serialize)]
pub struct UpdateCheck {
    /// This version.
    current: String,
    /// The newest published version.
    newest: String,
    /// Whether the newest is newer than this one.
    newer: bool,
}

/// A version such as 0.1.0-beta.2: numbers, then an optional pre-release
/// (a release ranks above any of its pre-releases).
fn rank(version: &str) -> Option<(u64, u64, u64, bool, String, u64)> {
    let version = version.trim().trim_start_matches('v');
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let mut numbers = core.split('.').map(|n| n.parse::<u64>().ok());
    let (major, minor, patch) = (numbers.next()??, numbers.next()??, numbers.next().flatten().unwrap_or(0));
    match pre {
        None => Some((major, minor, patch, true, String::new(), 0)),
        Some(pre) => {
            let (word, number) = pre.split_once('.').unwrap_or((pre, "0"));
            Some((major, minor, patch, false, word.to_owned(), number.parse().unwrap_or(0)))
        }
    }
}

/// Whether `newest` is newer than `current`.
pub fn is_newer(newest: &str, current: &str) -> bool {
    match (rank(newest), rank(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[tauri::command]
pub fn check_for_updates(app: AppHandle) -> CommandResult<UpdateCheck> {
    let current = app.package_info().version.to_string();
    let answer: serde_json::Value = ureq::get(RELEASES)
        .set("User-Agent", "Zecky-Lock-it-Windows")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|_| problem("GitHub could not be reached. Check this PC's internet connection and try again."))?
        .into_json()
        .map_err(|_| problem("GitHub's answer could not be read. Try again later."))?;
    let newest = answer
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter_map(|r| r["tag_name"].as_str())
        .filter(|tag| rank(tag).is_some())
        .max_by(|a, b| rank(a).cmp(&rank(b)))
        .map(|tag| tag.trim_start_matches('v').to_owned())
        .ok_or_else(|| problem("GitHub listed no versions. Try again later."))?;
    Ok(UpdateCheck { newer: is_newer(&newest, &current), current, newest })
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions_are_ranked() {
        assert!(is_newer("v0.1.0-beta.2", "0.1.0-beta.1"));
        assert!(is_newer("0.1.0-beta.10", "0.1.0-beta.9"));
        assert!(is_newer("0.1.0", "0.1.0-beta.9"));
        assert!(is_newer("0.2.0-beta.1", "0.1.0"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0-beta.1"));
        assert!(!is_newer("0.1.0-beta.1", "0.1.0-beta.2"));
        assert!(!is_newer("nonsense", "0.1.0-beta.1"));
    }
}
