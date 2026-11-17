//! Keeping the node's chain data when Zakura's database format moves up a
//! major version (Zakura 1.8.9: format 30 to 31).
//!
//! Zakura keeps its database in `state/v<major>/mainnet`. When the major
//! version goes up and the upgrade allows reuse, Zakura itself renames the old
//! folder to the new one and upgrades it in place. On Windows that step gives
//! up silently: Zakura works with the folder's verbatim path (`\\?\C:\…`),
//! in which Windows does not read `/` as a separator, and the database library
//! joins file names with `/`, so the old database looks absent. Zakura then
//! starts an empty database and the first check begins again from zero
//! (seen 17 Nov 2026 with 1.8.9, under Wine; the same path rules hold on
//! Windows).
//!
//! So Go Max makes the same move first, as Zakura's own code does it: the old
//! folder is renamed, its version file still names the old format, and
//! Zakura runs its own upgrade on it at start-up. Nothing is copied or
//! deleted, so the data stays whole if anything goes wrong.

use std::{io, path::Path};

/// The database format major version of the bundled node (Zakura 1.8.9).
pub const NODE_DB_MAJOR: u64 = 31;

/// Older major versions that Zakura upgrades in place to `NODE_DB_MAJOR`
/// (its upgrades to 31.0.0 and 30.0.0 are both marked reusable), newest
/// first. A folder older than these is left alone: Zakura would not reuse it.
const REUSABLE_FROM: [u64; 1] = [30];

/// Moves an older database to where the bundled node looks for it, if there
/// is one and the new place is still empty. `cache_dir` is Zakura's
/// `state.cache_dir`. Returns the version moved from, if any.
pub fn move_for_upgrade(cache_dir: &Path) -> io::Result<Option<u64>> {
    let place = |major: u64| cache_dir.join("state").join(format!("v{major}")).join("mainnet");
    let new = place(NODE_DB_MAJOR);
    if new.exists() {
        return Ok(None);
    }
    for old_major in REUSABLE_FROM {
        let old = place(old_major);
        // A real database has its CURRENT file; anything else is left alone.
        if !old.join("CURRENT").is_file() {
            continue;
        }
        if let Some(parent) = new.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&old, &new)?;
        // `state/v30` is empty now: remove it, as Zakura does.
        if let Some(parent) = old.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        return Ok(Some(old_major));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lockit-db-upgrade-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn database(dir: &Path, major: u64) -> std::path::PathBuf {
        let db = dir.join("state").join(format!("v{major}")).join("mainnet");
        std::fs::create_dir_all(&db).unwrap();
        std::fs::write(db.join("CURRENT"), "MANIFEST-000005\n").unwrap();
        std::fs::write(db.join("version"), format!("{major}.0.0")).unwrap();
        db
    }

    #[test]
    fn an_older_database_is_moved_whole_with_its_version_file() {
        let dir = temp("move");
        database(&dir, 30);
        assert_eq!(move_for_upgrade(&dir).unwrap(), Some(30));
        let new = dir.join("state/v31/mainnet");
        assert!(new.join("CURRENT").is_file());
        // Zakura reads this and runs its own upgrade from 30 to 31.
        assert_eq!(std::fs::read_to_string(new.join("version")).unwrap(), "30.0.0");
        assert!(!dir.join("state/v30").exists());
        // A second start finds the new place in use and does nothing.
        assert_eq!(move_for_upgrade(&dir).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_moves_when_there_is_nothing_to_move_or_the_new_place_is_used() {
        // A fresh node.
        let dir = temp("fresh");
        assert_eq!(move_for_upgrade(&dir).unwrap(), None);
        // Both there: the new one wins, the old one is not touched.
        database(&dir, 30);
        database(&dir, 31);
        assert_eq!(move_for_upgrade(&dir).unwrap(), None);
        assert!(dir.join("state/v30/mainnet/CURRENT").is_file());
        // A folder without a database is left alone.
        let other = temp("empty");
        std::fs::create_dir_all(other.join("state/v30/mainnet")).unwrap();
        assert_eq!(move_for_upgrade(&other).unwrap(), None);
        // Too old for Zakura to reuse: left alone.
        let old = temp("old");
        database(&old, 29);
        assert_eq!(move_for_upgrade(&old).unwrap(), None);
        for d in [dir, other, old] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
}
