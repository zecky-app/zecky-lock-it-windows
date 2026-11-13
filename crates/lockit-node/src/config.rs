//! The two config files, written from a few calm choices.
//!
//! Only the settings Go Max cares about are written; Zakura and Zaino fill in
//! their own defaults for everything else. Both files are rewritten at every
//! start, so they always match what the app shows.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use serde::Serialize;
use toml::{Table, Value};

use crate::{Error, NodeLayout, Result};

/// Whether other Zcash nodes may connect to this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Listening {
    /// Only outgoing connections. The node listens on this computer only, so
    /// Windows never shows a firewall prompt and the home address is never
    /// announced as a Zcash node. The quiet default.
    #[default]
    Private,
    /// Accept connections from other nodes, to help the network. Windows will
    /// ask once whether to allow it; the app must say so before it happens.
    HelpTheNetwork,
}

/// The choices the owner can make about Go Max. Everything else is fixed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoMaxSettings {
    /// Whether other nodes may connect in.
    pub listening: Listening,
    /// How many outgoing peers to aim for. `None` keeps Zakura's default.
    pub peer_target: Option<u32>,
    /// Zcash P2P port (legacy stack).
    pub p2p_port: u16,
    /// Zakura's P2P v2 port.
    pub p2p_v2_port: u16,
    /// Replaces Zakura's built-in seed list. `None` keeps the built-in one.
    ///
    /// For special setups only: for example a PC that should sync from the
    /// owner's own node at home, or a test without internet. With a list of
    /// IP addresses Zakura starts without looking up DNS seeds.
    pub initial_peers: Option<Vec<String>>,
    /// Zakura's JSON-RPC port, loopback only, cookie required.
    pub rpc_port: u16,
    /// Zaino's light-wallet (gRPC) port, loopback only.
    pub wallet_port: u16,
}

impl Default for GoMaxSettings {
    fn default() -> Self {
        Self {
            listening: Listening::Private,
            peer_target: None,
            initial_peers: None,
            p2p_port: 8233,
            p2p_v2_port: 8234,
            rpc_port: 8232,
            wallet_port: 8137,
        }
    }
}

impl GoMaxSettings {
    /// The address other nodes would connect to.
    ///
    /// Always IPv4. Zakura's own default is `[::]`, which makes the node stop
    /// at start-up on a PC where IPv6 is switched off.
    fn listen_ip(&self) -> Ipv4Addr {
        match self.listening {
            Listening::Private => Ipv4Addr::LOCALHOST,
            Listening::HelpTheNetwork => Ipv4Addr::UNSPECIFIED,
        }
    }

    fn loopback(port: u16) -> String {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).to_string()
    }

    /// The address Lock it connects to for wallet data.
    pub fn wallet_address(&self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.wallet_port))
    }

    /// The address of Zakura's RPC.
    pub fn rpc_address(&self) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.rpc_port))
    }

    /// Zakura's config, as TOML.
    pub fn zakura_toml(&self, layout: &NodeLayout) -> Result<String> {
        let mut network = Table::new();
        network.insert("network".into(), "Mainnet".into());
        network.insert(
            "listen_addr".into(),
            SocketAddr::V4(SocketAddrV4::new(self.listen_ip(), self.p2p_port)).to_string().into(),
        );
        network.insert("identity_dir".into(), path(&layout.zakura_identity())?);
        network.insert("cache_dir".into(), path(&layout.zakura_state().join("peers"))?);
        if let Some(target) = self.peer_target {
            network.insert("peerset_initial_target_size".into(), Value::Integer(target.into()));
        }
        if let Some(peers) = &self.initial_peers {
            let list = peers.iter().cloned().map(Value::String).collect();
            network.insert("initial_mainnet_peers".into(), Value::Array(list));
        }
        let mut v2 = Table::new();
        v2.insert(
            "listen_addr".into(),
            SocketAddr::V4(SocketAddrV4::new(self.listen_ip(), self.p2p_v2_port)).to_string().into(),
        );
        network.insert("zakura".into(), Value::Table(v2));

        let mut state = Table::new();
        state.insert("cache_dir".into(), path(&layout.zakura_state())?);
        state.insert("storage_mode".into(), "pruned".into());

        let mut rpc = Table::new();
        rpc.insert("listen_addr".into(), Self::loopback(self.rpc_port).into());
        rpc.insert("enable_cookie_auth".into(), true.into());
        rpc.insert("cookie_dir".into(), path(&layout.zakura_rpc())?);

        let mut tracing = Table::new();
        tracing.insert("use_color".into(), false.into());
        tracing.insert("force_use_color".into(), false.into());

        let mut root = Table::new();
        root.insert("network".into(), Value::Table(network));
        root.insert("state".into(), Value::Table(state));
        root.insert("rpc".into(), Value::Table(rpc));
        root.insert("tracing".into(), Value::Table(tracing));
        write_toml("zakurad.toml", root)
    }

    /// Zaino's config, as TOML.
    pub fn zaino_toml(&self, layout: &NodeLayout) -> Result<String> {
        let mut grpc = Table::new();
        grpc.insert("listen_address".into(), Self::loopback(self.wallet_port).into());

        let mut validator = Table::new();
        validator.insert("validator_jsonrpc_listen_address".into(), Self::loopback(self.rpc_port).into());
        validator.insert("validator_cookie_path".into(), path(&layout.cookie_file())?);

        let mut database = Table::new();
        database.insert("path".into(), path(&layout.zaino_database())?);
        let mut storage = Table::new();
        storage.insert("database".into(), Value::Table(database));

        let mut root = Table::new();
        root.insert("backend".into(), "rpc".into());
        root.insert("network".into(), "Mainnet".into());
        // Unused with the RPC backend, but Zaino's default is a Unix path that
        // is not valid on Windows, so set it to something real.
        root.insert("zebra_db_path".into(), path(&layout.zakura_state())?);
        root.insert("grpc_settings".into(), Value::Table(grpc));
        root.insert("validator_settings".into(), Value::Table(validator));
        root.insert("storage".into(), Value::Table(storage));
        write_toml("zainod.toml", root)
    }

    /// Writes both config files.
    pub fn write_configs(&self, layout: &NodeLayout) -> Result<()> {
        write_file(&layout.zakura_config(), &self.zakura_toml(layout)?)?;
        write_file(&layout.zaino_config(), &self.zaino_toml(layout)?)
    }
}

fn path(path: &std::path::Path) -> Result<Value> {
    path.to_str()
        .map(|text| Value::String(text.to_owned()))
        .ok_or_else(|| Error::Platform(format!("the folder {} has a name that cannot be used", path.display())))
}

fn write_toml(name: &str, table: Table) -> Result<String> {
    let body = toml::to_string_pretty(&table)
        .map_err(|e| Error::Platform(format!("could not write {name}: {e}")))?;
    Ok(format!("# Written by Zecky Lock it (Go Max). Changes here are replaced at the next start.\n\n{body}"))
}

fn write_file(path: &std::path::Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(|e| Error::io(format!("writing {}", path.display()), e))
}
