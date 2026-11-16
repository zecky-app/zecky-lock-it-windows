//! How much data Go Max has moved, for the overview on the Go Max page.
//!
//! Zakura counts the bytes it receives from and sends to other Zcash nodes,
//! and the block data it has checked and stored. It shows these counters on
//! its metrics endpoint, which Go Max opens on this computer only. The
//! counters start at zero whenever Zakura starts, so Lock it keeps the
//! running totals itself, in a small file next to the node's data. Nothing
//! is sent anywhere.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

use serde::{Deserialize, Serialize};

/// Zakura's counters as read once, since its last start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    /// `zcash.net.in.bytes.total`: received from other nodes.
    pub received: u64,
    /// `zcash.net.out.bytes.total`: sent to other nodes.
    pub sent: u64,
    /// `sync.block.payload.committed.bytes`: block data checked and stored.
    pub committed: u64,
}

/// The running totals, kept across restarts of Zakura and of Lock it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataUse {
    /// Downloaded from other Zcash nodes, in bytes.
    pub downloaded: u64,
    /// Uploaded to other Zcash nodes, in bytes.
    pub uploaded: u64,
    /// Block data checked and stored, in bytes. Most of it is removed again
    /// once checked (the node is pruned).
    pub committed: u64,
    /// Zakura's counters at the last reading, to count only what is new.
    pub last: Counters,
}

impl DataUse {
    /// Zakura was just started: its counters begin at zero again.
    pub fn new_process(&mut self) {
        self.last = Counters::default();
    }

    /// Add what is new since the last reading. A counter lower than before
    /// means Zakura restarted without Lock it noticing: all of it is new.
    pub fn add(&mut self, now: Counters) {
        let new = |now: u64, last: u64| if now >= last { now - last } else { now };
        self.downloaded += new(now.received, self.last.received);
        self.uploaded += new(now.sent, self.last.sent);
        self.committed += new(now.committed, self.last.committed);
        self.last = now;
    }

    /// Block data removed after it was checked: everything checked and
    /// stored minus what is still kept on this PC. Never below zero.
    ///
    /// Zakura counts stored block data only on its newer network stack and
    /// with an extra build option; Go Max runs the long-standing stack, so
    /// that counter usually stays at zero. Then what was downloaded stands
    /// in for it: during a first sync nearly all of it is block data.
    pub fn removed(&self, kept: u64) -> u64 {
        let checked = if self.committed > 0 { self.committed } else { self.downloaded };
        checked.saturating_sub(kept)
    }

    /// Reads the totals from `file`; empty if there is none yet.
    pub fn load(file: &Path) -> Self {
        std::fs::read(file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    /// Writes the totals to `file`, replacing it in one step.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        let temporary = file.with_extension("json.new");
        std::fs::write(&temporary, serde_json::to_vec(self)?)?;
        std::fs::rename(&temporary, file)
    }
}

/// Asks Zakura's metrics endpoint (on this computer) for its counters.
pub fn read(address: SocketAddr) -> Option<Counters> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(stream, "GET /metrics HTTP/1.0\r\nHost: {address}\r\nConnection: close\r\n\r\n").ok()?;
    let mut answer = Vec::new();
    stream.take(32 * 1024 * 1024).read_to_end(&mut answer).ok()?;
    let answer = String::from_utf8_lossy(&answer);
    let body = answer.split_once("\r\n\r\n")?.1;
    parse(body)
}

/// Picks the three counters out of Prometheus text. Names come with dots
/// turned into underscores, perhaps with `_total` added, perhaps with labels.
pub fn parse(text: &str) -> Option<Counters> {
    let value = |wanted: &str| -> Option<u64> {
        let wanted = wanted.replace('.', "_");
        let wanted = wanted.strip_suffix("_total").unwrap_or(&wanted).to_owned();
        let mut found = None;
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let Some((name, number)) = line.rsplit_once(' ') else { continue };
            let name = name.split('{').next().unwrap_or(name).trim();
            let name = name.strip_suffix("_total").unwrap_or(name);
            if name == wanted {
                let number: f64 = number.trim().parse().ok()?;
                *found.get_or_insert(0) += number as u64;
            }
        }
        found
    };
    let received = value("zcash.net.in.bytes.total");
    let sent = value("zcash.net.out.bytes.total");
    let committed = value("sync.block.payload.committed.bytes");
    if received.is_none() && sent.is_none() && committed.is_none() {
        return None;
    }
    Some(Counters { received: received.unwrap_or(0), sent: sent.unwrap_or(0), committed: committed.unwrap_or(0) })
}

/// The size of everything under `path`, in bytes.
pub fn folder_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => folder_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# TYPE zcash_net_in_bytes_total counter\n\
        zcash_net_in_bytes_total{addr=\"a\"} 1000\n\
        zcash_net_in_bytes_total{addr=\"b\"} 500\n\
        zcash_net_out_bytes_total 40\n\
        sync_block_payload_committed_bytes 900\n\
        zcash_net_in_bytes_totally_other 7\n";

    #[test]
    fn the_counters_are_found_in_prometheus_text() {
        assert_eq!(parse(TEXT), Some(Counters { received: 1500, sent: 40, committed: 900 }));
        assert_eq!(parse("sync_block_payload_committed_bytes_total 3e3\n").unwrap().committed, 3000);
        assert_eq!(parse("something_else 1\n"), None);
    }

    #[test]
    fn totals_carry_on_over_restarts() {
        let mut data = DataUse::default();
        data.add(Counters { received: 100, sent: 10, committed: 80 });
        data.add(Counters { received: 150, sent: 12, committed: 120 });
        assert_eq!((data.downloaded, data.uploaded, data.committed), (150, 12, 120));
        // Zakura restarted and Lock it knew.
        data.new_process();
        data.add(Counters { received: 30, sent: 1, committed: 20 });
        assert_eq!((data.downloaded, data.uploaded, data.committed), (180, 13, 140));
        // Zakura restarted and Lock it did not know: the lower counter is all new.
        data.add(Counters { received: 5, sent: 0, committed: 4 });
        assert_eq!((data.downloaded, data.uploaded, data.committed), (185, 13, 144));
        assert_eq!(data.removed(100), 44);
        assert_eq!(data.removed(500), 0, "never below zero");
        // Without Zakura's stored-block counter, what was downloaded stands in.
        let mut legacy = DataUse::default();
        legacy.add(Counters { received: 1_000, sent: 20, committed: 0 });
        assert_eq!(legacy.removed(300), 700);
    }

    #[test]
    fn totals_are_kept_in_a_file() {
        let dir = std::env::temp_dir().join(format!("lockit-data-use-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("data-use.json");
        assert_eq!(DataUse::load(&file), DataUse::default());
        let mut data = DataUse::default();
        data.add(Counters { received: 7, sent: 3, committed: 5 });
        data.save(&file).unwrap();
        assert_eq!(DataUse::load(&file), data);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
