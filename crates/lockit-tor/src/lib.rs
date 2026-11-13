//! Go Max's own onion address.
//!
//! When Go Max runs, Lock it also runs a Tor onion service (with Arti, Tor
//! in Rust, inside the app). Its address belongs to this install: the key is
//! made on the first start, kept in Go Max's `tor\state` folder, and reused
//! at every start after that, so a paired phone keeps working. Connections to
//! the onion address are passed to Zaino's wallet port on this computer and
//! to nothing else.
//!
//! This gives a phone a way in from anywhere, without Tailscale, port
//! forwarding or any account, and without revealing the home address.
//!
//! # What is and is not tested
//!
//! Creating the address offline, and getting the same address back after a
//! restart, has run on Linux and as a Windows program under Wine. Publishing
//! the service needs the Tor network and has **not** been tried yet.

#![warn(missing_docs)]

use std::{path::Path, sync::Arc, time::Duration};

use arti_client::{config::TorClientConfigBuilder, TorClient};
use serde::Serialize;
use tor_hsrproxy::{config::ProxyConfigBuilder, OnionServiceReverseProxy};
use tor_hsservice::{config::OnionServiceConfigBuilder, status::State, HsNickname, RunningOnionService};

/// The service's nickname inside Tor's state. Part of the format: changing it
/// would give every install a new onion address.
const NICKNAME: &str = "lockit-go-max";

/// Result type for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Tor could not be set up or started.
    #[error("Tor could not start: {0}")]
    Tor(String),
}

fn tor(error: impl std::fmt::Display) -> Error {
    Error::Tor(error.to_string())
}

/// How the onion service is doing, for the Go Max page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnionState {
    /// Connecting to Tor and announcing the address.
    Starting,
    /// Reachable.
    Reachable,
    /// Partly reachable; Tor is working on it.
    Degraded,
    /// Not reachable right now.
    Unreachable,
}

/// The running onion service.
pub struct OnionAccess {
    runtime: tokio::runtime::Runtime,
    // Kept alive for as long as the service runs.
    _client: Arc<TorClient<tor_rtcompat::PreferredRuntime>>,
    service: Arc<RunningOnionService>,
    proxy: Arc<OnionServiceReverseProxy>,
    address: String,
}

impl OnionAccess {
    /// Starts Tor and the onion service, passing `local_port` on the onion
    /// address to `127.0.0.1:local_port`.
    ///
    /// Returns as soon as the address is known; reaching the Tor network and
    /// announcing the address carry on in the background.
    pub fn start(state_dir: &Path, cache_dir: &Path, local_port: u16) -> Result<Self> {
        // Choose rustls's crypto provider once, before any TLS is set up.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lockit-tor")
            .enable_all()
            .build()
            .map_err(tor)?;

        let config = TorClientConfigBuilder::from_directories(state_dir, cache_dir).build().map_err(tor)?;
        let nickname = HsNickname::new(NICKNAME.to_owned()).map_err(tor)?;

        let (client, service, requests) = runtime.block_on(async {
            let client = TorClient::builder().config(config).create_unbootstrapped().map_err(tor)?;
            let service_config =
                OnionServiceConfigBuilder::default().nickname(nickname.clone()).build().map_err(tor)?;
            let (service, requests) = client
                .launch_onion_service(service_config)
                .map_err(tor)?
                .ok_or_else(|| Error::Tor("the onion service is switched off".into()))?;
            Ok::<_, Error>((client, service, requests))
        })?;

        use safelog::DisplayRedacted as _;
        let address = service
            .onion_address()
            .map(|id| id.display_unredacted().to_string())
            .ok_or_else(|| Error::Tor("the onion address could not be read".into()))?;

        // One rule: the wallet port goes to Zaino on this computer. Every
        // other port is refused.
        let rules = format!(
            r#"{{ "proxy_ports": [ [ "{local_port}", "127.0.0.1:{local_port}" ], [ "*", "destroy" ] ] }}"#
        );
        let proxy_config: ProxyConfigBuilder = serde_json::from_str(&rules).map_err(tor)?;
        let proxy = OnionServiceReverseProxy::new(proxy_config.build().map_err(tor)?);
        let tor_runtime = client.runtime().clone();
        runtime.spawn({
            let proxy = Arc::clone(&proxy);
            async move {
                let _ = proxy.handle_requests(tor_runtime, nickname, Box::pin(requests)).await;
            }
        });

        Ok(Self { runtime, _client: client, service, proxy, address })
    }

    /// The onion address, for example `abc…xyz.onion`.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// How the service is doing.
    pub fn state(&self) -> OnionState {
        match self.service.status().state() {
            State::Running => OnionState::Reachable,
            State::DegradedReachable => OnionState::Degraded,
            State::Bootstrapping | State::Recovering => OnionState::Starting,
            _ => OnionState::Unreachable,
        }
    }

    /// Stops the service and Tor.
    pub fn stop(self) {
        self.proxy.shutdown();
        self.runtime.shutdown_timeout(Duration::from_secs(5));
    }
}
