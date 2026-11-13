//! Where the node keeps its things.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// The folders and files of one Go Max node, all under one root.
///
/// On Windows the app passes `%LOCALAPPDATA%\Zecky\Lock it\go-max` (local,
/// not roaming: the chain state must never be copied to other PCs).
///
/// ```text
/// <root>\
///   zakura\          chain state (pruned), node identity, RPC cookie
///   zaino\           Zaino's index
///   config\          zakurad.toml, zainod.toml (rewritten at every start)
///   tor\state\       the onion service's key (keep), Tor state
///   tor\cache\       Tor directory cache (safe to delete)
///   logs\            zakurad.log, zainod.log (previous run kept as *.old.log)
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeLayout {
    root: PathBuf,
}

impl NodeLayout {
    /// A layout under `root`. Nothing is created until [`NodeLayout::create`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates every folder.
    pub fn create(&self) -> Result<()> {
        for dir in [
            self.zakura_state(),
            self.zakura_identity(),
            self.zakura_rpc(),
            self.zaino_database(),
            self.config_dir(),
            self.log_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .map_err(|e| Error::io(format!("creating {}", dir.display()), e))?;
        }
        Ok(())
    }

    /// The root folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Zakura's chain state.
    pub fn zakura_state(&self) -> PathBuf {
        self.root.join("zakura").join("state")
    }

    /// Zakura's node identity (its P2P key).
    pub fn zakura_identity(&self) -> PathBuf {
        self.root.join("zakura").join("identity")
    }

    /// Where Zakura writes its RPC cookie.
    pub fn zakura_rpc(&self) -> PathBuf {
        self.root.join("zakura").join("rpc")
    }

    /// The RPC cookie file Zakura writes and Zaino reads.
    pub fn cookie_file(&self) -> PathBuf {
        self.zakura_rpc().join(".cookie")
    }

    /// Zaino's database.
    pub fn zaino_database(&self) -> PathBuf {
        self.root.join("zaino").join("db")
    }

    /// Tor's state: the onion service's key lives here and must be kept, so
    /// the onion address stays the same across restarts.
    pub fn tor_state(&self) -> PathBuf {
        self.root.join("tor").join("state")
    }

    /// Tor's cache: directory information, safe to delete.
    pub fn tor_cache(&self) -> PathBuf {
        self.root.join("tor").join("cache")
    }

    /// The config folder.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    /// Zakura's config file.
    pub fn zakura_config(&self) -> PathBuf {
        self.config_dir().join("zakurad.toml")
    }

    /// Zaino's config file.
    pub fn zaino_config(&self) -> PathBuf {
        self.config_dir().join("zainod.toml")
    }

    /// The log folder.
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// The current log of a program, for example `zakurad.log`.
    pub fn log_file(&self, program: &str) -> PathBuf {
        self.log_dir().join(format!("{program}.log"))
    }
}
