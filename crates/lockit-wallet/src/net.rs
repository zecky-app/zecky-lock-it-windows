//! Talking to a light-wallet server: your own node (Go Max) or a public one.
//!
//! Reading: from Go Max on this computer when it runs, otherwise from a
//! public server over TLS.
//!
//! Sending: over Tor by default, as on Lock it Mac. The payment goes to two
//! public servers at once, each over its own Tor circuit. Your own node is not
//! among them, because it would announce the payment from your internet
//! address. Without Tor, the payment goes to the server Lock it reads from.

use std::{path::Path, time::Duration};

use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};
use zcash_client_backend::{
    proto::service::{
        compact_tx_streamer_client::CompactTxStreamerClient, BlockId, ChainSpec, RawTransaction, TreeState,
    },
    tor,
};

use crate::{Error, Result};

/// The built-in public servers, the same list as Lock it Mac.
pub const PUBLIC_SERVERS: [&str; 5] =
    ["zec.rocks", "eu.zec.rocks", "na.zec.rocks", "sa.zec.rocks", "ap.zec.rocks"];

/// How long sending over Tor may take, the first connection included.
const TOR_PATIENCE: Duration = Duration::from_secs(180);

/// Where Lock it reads from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    /// Your own node, Go Max, on this computer (Zaino's port, no TLS).
    OwnNode {
        /// Zaino's port.
        port: u16,
    },
    /// A public server, over TLS on port 443.
    Public {
        /// Its host name.
        host: String,
    },
    /// A server or node of the owner's choice. On a home or Tailscale
    /// address it is reached without TLS (as Lock it Mac does for Rock it);
    /// anywhere else with TLS.
    Custom {
        /// Host name or IP address.
        host: String,
        /// Port.
        port: u16,
    },
}

/// Whether `host` is on a home network, this PC, or a Tailscale address:
/// such a node is reached without TLS, as Lock it Mac does.
pub fn is_private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".local") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let [a, b, ..] = ip.octets();
            ip.is_private() || ip.is_loopback() || (a == 100 && (64..=127).contains(&b))
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

impl Server {
    /// The default public server.
    pub fn default_public() -> Self {
        Self::Public { host: PUBLIC_SERVERS[0].to_owned() }
    }

    fn uses_tls(&self) -> bool {
        match self {
            Self::OwnNode { .. } => false,
            Self::Public { .. } => true,
            Self::Custom { host, .. } => !is_private_host(host),
        }
    }

    fn uri(&self) -> String {
        match self {
            Self::OwnNode { port } => format!("http://127.0.0.1:{port}"),
            Self::Public { host } => format!("https://{host}:443"),
            Self::Custom { host, port } => {
                let host = if host.contains(':') { format!("[{host}]") } else { host.clone() };
                format!("{}://{host}:{port}", if self.uses_tls() { "https" } else { "http" })
            }
        }
    }

    /// For the status line: "your own node" or the host.
    pub fn label(&self) -> String {
        match self {
            Self::OwnNode { .. } => "your own node (Go Max)".into(),
            Self::Public { host } => host.clone(),
            Self::Custom { host, port } => format!("{host}:{port}"),
        }
    }

    /// Opens a direct connection (no Tor), for reading.
    pub async fn connect(&self) -> Result<CompactTxStreamerClient<Channel>> {
        let uri: Uri = self.uri().parse().map_err(|e| Error::Server(format!("{e}")))?;
        let mut endpoint = Endpoint::from(uri)
            .connect_timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(20));
        if self.uses_tls() {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_webpki_roots())
                .map_err(|e| Error::Server(e.to_string()))?;
        }
        let channel = endpoint.connect().await.map_err(|e| Error::Server(format!("{}: {e}", self.label())))?;
        Ok(CompactTxStreamerClient::new(channel).max_decoding_message_size(64 * 1024 * 1024))
    }
}

/// The newest block height the server knows.
pub(crate) async fn tip(client: &mut CompactTxStreamerClient<Channel>) -> Result<u32> {
    let block = client
        .get_latest_block(ChainSpec::default())
        .await
        .map_err(|e| Error::Server(e.message().to_owned()))?
        .into_inner();
    u32::try_from(block.height).map_err(|_| Error::Server("an impossible block height".into()))
}

/// The note commitment trees at `height`, to start a wallet from.
pub(crate) async fn tree_state(client: &mut CompactTxStreamerClient<Channel>, height: u32) -> Result<TreeState> {
    Ok(client
        .get_tree_state(BlockId { height: height.into(), hash: vec![] })
        .await
        .map_err(|e| Error::Server(e.message().to_owned()))?
        .into_inner())
}

/// Hands a payment to one server. `Ok` only if the server accepted it.
async fn submit_to(client: &mut CompactTxStreamerClient<Channel>, raw: &[u8], label: &str) -> Result<()> {
    let response = client
        .send_transaction(RawTransaction { data: raw.to_vec(), height: 0 })
        .await
        .map_err(|e| Error::Server(format!("{label}: {}", e.message())))?
        .into_inner();
    if response.error_code != 0 {
        return Err(Error::Server(format!("{label} refused the payment: {}", response.error_message)));
    }
    Ok(())
}

/// Hands a payment to the network.
///
/// Over Tor: to two public servers at once, each on its own circuit; succeeds
/// if at least one accepted it. Never falls back to a direct connection by
/// itself: if Tor fails, the error says so and the app asks first.
pub(crate) async fn submit(raw: &[u8], reading_from: &Server, over_tor: bool, tor_dir: &Path) -> Result<Vec<String>> {
    if !over_tor {
        let mut client = reading_from.connect().await?;
        submit_to(&mut client, raw, &reading_from.label()).await?;
        return Ok(vec![reading_from.label()]);
    }

    std::fs::create_dir_all(tor_dir).map_err(|e| Error::Tor(e.to_string()))?;
    let tor = tokio::time::timeout(TOR_PATIENCE, tor::Client::create(tor_dir, |_| {}))
        .await
        .map_err(|_| Error::Tor("Tor did not connect within 3 minutes".into()))?
        .map_err(|e| Error::Tor(e.to_string()))?;

    // The public server being read from (if not your own node), then the built-in ones.
    let mut hosts: Vec<String> = Vec::new();
    // Never the owner's own node: it would announce the payment from this
    // PC's internet address. A public server being read from goes first.
    if let Server::Public { host } = reading_from {
        hosts.push(host.clone());
    }
    for host in PUBLIC_SERVERS {
        if !hosts.iter().any(|h| h == host) {
            hosts.push(host.to_owned());
        }
    }
    hosts.truncate(2);

    let attempts = hosts.iter().map(|host| {
        let circuit = tor.isolated_client();
        let host = host.clone();
        async move {
            let uri: Uri = format!("https://{host}:443").parse().map_err(|e| Error::Tor(format!("{e}")))?;
            let mut client = circuit
                .connect_to_lightwalletd(uri, false)
                .await
                .map_err(|e| Error::Tor(format!("{host}: {e}")))?;
            submit_to(&mut client, raw, &host).await.map(|()| host)
        }
    });
    // Tor can take a while the first time; never longer than this.
    let results = match tokio::time::timeout(TOR_PATIENCE, futures_util::future::join_all(attempts)).await {
        Ok(results) => results,
        Err(_) => return Err(Error::Tor("no server was reached within 3 minutes".into())),
    };

    let accepted: Vec<String> = results.iter().filter_map(|r| r.as_ref().ok().cloned()).collect();
    if accepted.is_empty() {
        let reasons: Vec<String> = results.into_iter().filter_map(|r| r.err().map(|e| e.to_string())).collect();
        return Err(Error::Tor(reasons.join("; ")));
    }
    Ok(accepted)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_tailscale_addresses_skip_tls_and_public_ones_use_it() {
        for host in ["192.168.1.20", "10.0.0.5", "172.16.3.4", "100.101.102.103", "127.0.0.1", "localhost", "umbrel.local"] {
            assert!(is_private_host(host), "{host}");
        }
        for host in ["zec.rocks", "8.8.8.8", "100.200.1.1", "example.com"] {
            assert!(!is_private_host(host), "{host}");
        }
        assert_eq!(Server::Custom { host: "192.168.1.20".into(), port: 9067 }.uri(), "http://192.168.1.20:9067");
        assert_eq!(Server::Custom { host: "lwd.example.com".into(), port: 443 }.uri(), "https://lwd.example.com:443");
    }
}
