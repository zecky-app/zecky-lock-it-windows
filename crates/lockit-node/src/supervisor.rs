//! Go Max itself: start both programs in order, watch them, stop them in order.

use std::{
    ffi::OsStr,
    net::TcpStream,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    process::{NodeProcess, ProcessGroup, STOP_GRACE},
    Error, GoMaxSettings, NodeLayout, NodeRpc, NodeStatus, Result,
};

/// How long Zakura gets to open its database and answer RPC.
///
/// Opening a large state can take minutes after an unclean stop.
const NODE_READY_TIMEOUT: Duration = Duration::from_secs(600);

/// How long Zaino must stay up after starting before Go Max counts it as
/// running. A bad config makes it exit within a second or two.
const ZAINO_SETTLE: Duration = Duration::from_secs(5);

/// The first pause before Zaino is started again after it stopped by itself.
///
/// During the first sync Zaino gives up when the node is busy or has not
/// stored the block Zaino wants to start from ("chain head could not
/// anchor"). That is expected: as on Lock it Mac, Zaino is started again as
/// long as the node answers, and the node keeps syncing either way.
const ZAINO_RETRY_FIRST: Duration = Duration::from_secs(15);
/// The longest pause between tries.
const ZAINO_RETRY_MAX: Duration = Duration::from_secs(120);
/// Zaino counts as settled once it has run this long; the pause starts over.
const ZAINO_SETTLED: Duration = Duration::from_secs(300);

/// Zaino's restart bookkeeping.
#[derive(Debug, Default)]
struct ZainoRetry {
    started_at: Option<Instant>,
    next_try: Option<Instant>,
    pause: Duration,
    restarts: u32,
    last_exit: Option<String>,
}

/// Where the two node programs are.
#[derive(Clone, Debug)]
pub struct NodePrograms {
    /// `zakurad` (`zakurad.exe` on Windows).
    pub zakurad: PathBuf,
    /// `zainod` (`zainod.exe` on Windows).
    pub zainod: PathBuf,
}

impl NodePrograms {
    /// The programs in `dir` (the app passes its resource folder's `node`).
    pub fn in_dir(dir: &std::path::Path) -> Self {
        let exe = std::env::consts::EXE_SUFFIX;
        Self { zakurad: dir.join(format!("zakurad{exe}")), zainod: dir.join(format!("zainod{exe}")) }
    }

    /// The programs as shipped inside the app: in `node` next to the app's
    /// own executable.
    pub fn bundled() -> Result<Self> {
        let me = std::env::current_exe().map_err(|e| Error::io("finding the app's own folder", e))?;
        let dir = me.parent().map(|dir| dir.join("node")).unwrap_or_else(|| PathBuf::from("node"));
        let exe = std::env::consts::EXE_SUFFIX;
        Ok(Self { zakurad: dir.join(format!("zakurad{exe}")), zainod: dir.join(format!("zainod{exe}")) })
    }
}

/// How a stop went.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct StopReport {
    /// Zaino was running and finished by itself when asked (or Go Max was
    /// not running at all).
    pub zaino_clean: bool,
    /// Zakura was running and finished by itself when asked (or Go Max was
    /// not running at all).
    pub zakura_clean: bool,
    /// How long the stop took, in whole seconds.
    pub seconds: u64,
}

impl StopReport {
    /// Both stopped by themselves.
    pub fn clean(&self) -> bool {
        self.zaino_clean && self.zakura_clean
    }
}

/// The running node, or the means to start it.
pub struct GoMax {
    layout: NodeLayout,
    settings: GoMaxSettings,
    programs: NodePrograms,
    group: Option<ProcessGroup>,
    zakura: Option<NodeProcess>,
    zaino: Option<NodeProcess>,
    zaino_retry: ZainoRetry,
}

impl GoMax {
    /// Prepares Go Max. Nothing starts until [`GoMax::start`].
    pub fn new(layout: NodeLayout, settings: GoMaxSettings, programs: NodePrograms) -> Self {
        Self { layout, settings, programs, group: None, zakura: None, zaino: None, zaino_retry: ZainoRetry::default() }
    }

    /// The node's folders.
    pub fn layout(&self) -> &NodeLayout {
        &self.layout
    }

    /// The settings in use.
    pub fn settings(&self) -> &GoMaxSettings {
        &self.settings
    }

    /// Whether the node is running (Zaino may be between restarts).
    pub fn is_running(&mut self) -> bool {
        self.check().is_ok() && self.zakura.is_some()
    }

    /// Starts Zakura, waits until it answers, then starts Zaino and waits
    /// until its wallet port is open.
    ///
    /// `progress` is told each step, for the page to show.
    pub fn start(&mut self, mut progress: impl FnMut(&str)) -> Result<()> {
        if self.zakura.is_some() || self.zaino.is_some() {
            return Err(Error::AlreadyRunning);
        }
        self.layout.create()?;
        crate::checks::before_start(&self.layout, &self.settings)?;
        self.settings.write_configs(&self.layout)?;
        // A cookie left from an earlier run is stale; Zakura writes a new one.
        let _ = std::fs::remove_file(self.layout.cookie_file());

        let group = ProcessGroup::new()?;

        progress("Starting the node");
        let zakura_config = self.layout.zakura_config();
        let mut zakura = NodeProcess::start(
            &group,
            "Zakura",
            &self.programs.zakurad,
            &[OsStr::new("-c"), zakura_config.as_os_str(), OsStr::new("start")],
            &self.layout.log_file("zakurad"),
        )?;

        progress("Opening the chain state");
        let rpc = self.rpc();
        let ready = wait_until("The node", NODE_READY_TIMEOUT, || {
            zakura.check_running()?;
            Ok(rpc.is_ready())
        });
        if let Err(error) = ready {
            let _ = zakura.stop(STOP_GRACE);
            return Err(error);
        }

        progress("Starting the wallet service");
        let zaino_config = self.layout.zaino_config();
        let zaino = NodeProcess::start(
            &group,
            "Zaino",
            &self.programs.zainod,
            &[OsStr::new("start"), OsStr::new("-c"), zaino_config.as_os_str()],
            &self.layout.log_file("zainod"),
        );
        let mut zaino = match zaino {
            Ok(zaino) => zaino,
            Err(error) => {
                let _ = zakura.stop(STOP_GRACE);
                return Err(error);
            }
        };

        // Zaino opens its wallet port only once the node has blocks, which on
        // a fresh install can take a while. So "running" means both programs
        // are up; whether wallets can connect yet is `wallet_ready`.
        // Only the node must stay up here; a Zaino that stops is started
        // again by `check`.
        let settle = wait_until("The wallet service", ZAINO_SETTLE, || {
            zakura.check_running()?;
            Ok(zaino.check_running().is_err())
        });
        if let Err(error) = settle.or_else(|error| match error {
            Error::NotReady(_) => Ok(()),
            other => Err(other),
        }) {
            let _ = zaino.stop(STOP_GRACE);
            let _ = zakura.stop(STOP_GRACE);
            return Err(error);
        }

        progress("Running");
        self.group = Some(group);
        self.zakura = Some(zakura);
        self.zaino = Some(zaino);
        self.zaino_retry = ZainoRetry { started_at: Some(Instant::now()), pause: ZAINO_RETRY_FIRST, ..Default::default() };
        Ok(())
    }

    /// Checks on both programs. An error means the node itself ended, and Go
    /// Max is over. If only Zaino ended, it is started again after a pause,
    /// as long as the node answers; the node keeps going meanwhile.
    pub fn check(&mut self) -> Result<()> {
        if let Some(zakura) = self.zakura.as_mut() {
            zakura.check_running()?;
        } else {
            return Ok(());
        }
        let now = Instant::now();
        if let Some(zaino) = self.zaino.as_mut() {
            if let Err(exit) = zaino.check_running() {
                let retry = &mut self.zaino_retry;
                let lived = retry.started_at.map(|t| now.duration_since(t)).unwrap_or_default();
                retry.pause = if lived >= ZAINO_SETTLED {
                    ZAINO_RETRY_FIRST
                } else {
                    (retry.pause.max(ZAINO_RETRY_FIRST / 2) * 2).min(ZAINO_RETRY_MAX)
                };
                retry.next_try = Some(now + retry.pause);
                retry.last_exit = Some(exit.to_string());
                self.zaino = None;
            }
        }
        if self.zaino.is_none() && self.zaino_retry.next_try.is_some_and(|t| now >= t) && self.rpc().is_ready() {
            self.zaino_retry.next_try = None;
            let started = match self.group.as_ref() {
                Some(group) => NodeProcess::start(
                    group,
                    "Zaino",
                    &self.programs.zainod,
                    &[OsStr::new("start"), OsStr::new("-c"), self.layout.zaino_config().as_os_str()],
                    &self.layout.log_file("zainod"),
                ),
                None => return Ok(()),
            };
            match started {
                Ok(zaino) => {
                    self.zaino = Some(zaino);
                    self.zaino_retry.started_at = Some(now);
                    self.zaino_retry.restarts += 1;
                }
                Err(error) => {
                    self.zaino_retry.last_exit = Some(error.to_string());
                    self.zaino_retry.next_try = Some(now + ZAINO_RETRY_MAX);
                }
            }
        } else if self.zaino.is_none() && self.zaino_retry.next_try.is_some_and(|t| now >= t) {
            // The node is busy and did not answer: try again shortly.
            self.zaino_retry.next_try = Some(now + ZAINO_RETRY_FIRST);
        }
        Ok(())
    }

    /// Why the wallet service is not running right now, if it is between
    /// restarts. `None` while it runs.
    pub fn wallet_service_note(&self) -> Option<String> {
        if self.zaino.is_some() || self.zakura.is_none() {
            return None;
        }
        let wait = self.zaino_retry.next_try.map(|t| t.saturating_duration_since(Instant::now()).as_secs()).unwrap_or(0);
        Some(format!(
            "The wallet service is waiting for the node and starts again in about {wait} s. This is normal while the node catches up."
        ))
    }

    /// How often Zaino has been started again in this run.
    pub fn wallet_service_restarts(&self) -> u32 {
        self.zaino_retry.restarts
    }

    /// Reads the node's status.
    pub fn status(&self) -> Result<NodeStatus> {
        self.rpc().status()
    }

    /// Whether wallets can connect yet (Zaino's port is open).
    pub fn wallet_ready(&self) -> bool {
        TcpStream::connect_timeout(&self.settings.wallet_address(), Duration::from_millis(500)).is_ok()
    }

    /// Stops Zaino first, then Zakura, each cleanly if it can.
    pub fn stop(&mut self) -> Result<StopReport> {
        let started = Instant::now();
        let zaino_clean = match self.zaino.take() {
            Some(zaino) => zaino.stop(STOP_GRACE)?,
            None => true,
        };
        let zakura_clean = match self.zakura.take() {
            Some(zakura) => zakura.stop(STOP_GRACE)?,
            None => true,
        };
        self.group = None;
        self.zaino_retry = ZainoRetry::default();
        Ok(StopReport { zaino_clean, zakura_clean, seconds: started.elapsed().as_secs() })
    }

    fn rpc(&self) -> NodeRpc {
        NodeRpc::new(self.settings.rpc_address(), self.layout.cookie_file())
    }
}

impl Drop for GoMax {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn wait_until(what: &str, timeout: Duration, mut ready: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if ready()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::NotReady(what.into()));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
