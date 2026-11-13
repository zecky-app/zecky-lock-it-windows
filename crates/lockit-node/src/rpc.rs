//! Reading the node's status for the Go Max page.

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Error, Result};

/// A client for Zakura's JSON-RPC on this computer.
///
/// Authenticates with the cookie Zakura writes at every start, so no password
/// is ever stored or shown.
#[derive(Clone, Debug)]
pub struct NodeRpc {
    address: SocketAddr,
    cookie: PathBuf,
}

/// What the Go Max page shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NodeStatus {
    /// Height of the newest block this node has checked.
    pub height: u32,
    /// Height of the newest header this node knows of.
    pub headers: u32,
    /// The node's estimate of the network's current height.
    pub estimated_height: u32,
    /// How far the sync is, from 0 to 1, as reported by the node.
    pub progress: f64,
    /// Peers connected right now.
    pub peers: usize,
    /// Of those, how many connected to us.
    pub inbound_peers: usize,
    /// Whether the node keeps a pruned chain.
    pub pruned: bool,
    /// Chain state size on disk, in bytes, as reported by the node.
    pub size_on_disk: u64,
}

impl NodeStatus {
    /// Whether the node has caught up with the network, within two blocks.
    pub fn caught_up(&self) -> bool {
        self.estimated_height > 0 && self.height + 2 >= self.estimated_height
    }
}

#[derive(Deserialize)]
struct BlockchainInfo {
    blocks: u32,
    headers: u32,
    #[serde(rename = "estimatedheight")]
    estimated_height: u32,
    #[serde(rename = "verificationprogress")]
    progress: f64,
    pruned: bool,
    size_on_disk: u64,
}

#[derive(Deserialize)]
struct PeerInfo {
    #[serde(default)]
    inbound: bool,
}

impl NodeRpc {
    /// A client for the node at `address`, using the cookie at `cookie`.
    pub fn new(address: SocketAddr, cookie: impl Into<PathBuf>) -> Self {
        Self { address, cookie: cookie.into() }
    }

    /// Whether the node answers at all.
    ///
    /// Any JSON-RPC answer counts, an error included: on an empty chain some
    /// methods answer with an error until the first block arrives.
    pub fn is_ready(&self) -> bool {
        self.post("getblockchaininfo", json!([])).is_ok()
    }

    /// Reads the status.
    pub fn status(&self) -> Result<NodeStatus> {
        let info: BlockchainInfo = serde_json::from_value(self.call("getblockchaininfo", json!([]))?)
            .map_err(|e| Error::Rpc(format!("unexpected answer to getblockchaininfo: {e}")))?;
        let peers: Vec<PeerInfo> = serde_json::from_value(self.call("getpeerinfo", json!([]))?)
            .map_err(|e| Error::Rpc(format!("unexpected answer to getpeerinfo: {e}")))?;
        Ok(NodeStatus {
            height: info.blocks,
            headers: info.headers,
            estimated_height: info.estimated_height,
            progress: info.progress,
            peers: peers.len(),
            inbound_peers: peers.iter().filter(|peer| peer.inbound).count(),
            pruned: info.pruned,
            size_on_disk: info.size_on_disk,
        })
    }

    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut answer = self.post(method, params)?;
        if let Some(error) = answer.get("error").filter(|error| !error.is_null()) {
            return Err(Error::Rpc(error.to_string()));
        }
        Ok(answer.get_mut("result").map(Value::take).unwrap_or(Value::Null))
    }

    /// Sends one request and returns the whole JSON-RPC answer.
    fn post(&self, method: &str, params: Value) -> Result<Value> {
        let cookie = std::fs::read_to_string(&self.cookie)
            .map_err(|_| Error::Rpc("the node's cookie is not there yet".into()))?;
        let auth = format!("Basic {}", B64.encode(cookie.trim()));
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout(Duration::from_secs(10))
            .build();
        let response = match agent
            .post(&format!("http://{}/", self.address))
            .set("Authorization", &auth)
            .send_json(body)
        {
            Ok(response) => response,
            // JSON-RPC errors may come with an HTTP error status; the body
            // still holds the answer.
            Err(ureq::Error::Status(_, response)) => response,
            Err(error) => return Err(Error::Rpc(error.to_string())),
        };
        response.into_json().map_err(|e| Error::Rpc(format!("the node's answer was not JSON: {e}")))
    }
}
