//! Go Max's rules, as on Lock it Mac build 6, kept apart from the programs
//! so they can be tested on their own.
//!
//! * [`support`]: how long the node inside this build follows the chain.
//! * [`reading`]: when the wallet may read from the node, and when it has
//!   gone quiet there.

/// How long the node inside this build follows the chain.
///
/// Every Zakura release stops at a fixed block, about 45 days after it comes
/// out (its end-of-support height), and refuses to run past it. Lock it says
/// so two weeks ahead, and at the stop it asks: a public server, or wait.
/// Never by itself.
pub mod support {
    /// `MAINNET_EOS_HEIGHT` of Zakura 1.8.5 to 1.8.7: about Dec 5, 2026.
    pub const STOP_HEIGHT: u32 = 3_643_309;
    /// Seconds per block since NU7.
    pub const BLOCK_SECONDS: i64 = 25;
    /// The notice starts two weeks of blocks before the stop.
    pub const NOTICE_BLOCKS: u32 = 14 * 3_456;
    /// Without a fresh height (Lock it was closed for days), the stop is only
    /// taken as reached once the clock is a day of blocks past it: blocks do
    /// not come exactly every 25 seconds.
    pub const GRACE_BLOCKS: u32 = 3_456;

    /// Where this node version stands.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
    #[serde(rename_all = "snake_case", tag = "state")]
    pub enum State {
        /// Nothing to say.
        Fine,
        /// The stop is near; about when (Unix seconds).
        Soon {
            /// About when the stop comes.
            at: i64,
        },
        /// The network is past the node's last block.
        Ended,
    }

    /// `reported` is the newest network height anything told Lock it (0:
    /// none yet), `seen_at` when that was (`None`: just now).
    pub fn state(reported: u32, seen_at: Option<i64>, now: i64, stop: u32) -> State {
        if reported == 0 {
            return State::Fine;
        }
        if reported > stop {
            return State::Ended;
        }
        let elapsed = seen_at.map(|at| (now - at).max(0)).unwrap_or(0);
        let projected = reported.saturating_add(u32::try_from(elapsed / BLOCK_SECONDS).unwrap_or(u32::MAX));
        if projected > stop.saturating_add(GRACE_BLOCKS) {
            return State::Ended;
        }
        let remaining = stop.saturating_sub(projected);
        if remaining > NOTICE_BLOCKS {
            return State::Fine;
        }
        State::Soon { at: now + i64::from(remaining) * BLOCK_SECONDS }
    }

    /// The network height the stop is judged by. The node's own estimate of
    /// the network is only trusted once the node is close to the newest
    /// block: during the first check of history it can be far too high (seen
    /// 13 Nov 2026: about 60,000 blocks, which showed "needs an update
    /// before Nov 17" three weeks early and could have stopped Go Max
    /// mid-sync). Before that, the node's verified blocks and the wallet's
    /// server count.
    pub fn network_height(node_blocks: u32, node_estimate: u32, progress: f64, wallet_network: u32) -> u32 {
        let node = if progress >= 0.999 { node_blocks.max(node_estimate) } else { node_blocks };
        node.max(wallet_network)
    }
}

/// Where the wallet reads the chain while Go Max is on.
///
/// The node is pruned: it keeps only recent blocks. A wallet that still has
/// older blocks to check (after adding or restoring a wallet) asks the node
/// for blocks it no longer has and then stands still without an error. So
/// the wallet reads from the node only when the node has the oldest block it
/// still needs.
pub mod reading {
    /// The block the wallet needs and the node no longer has; `None` when the
    /// node can serve the wallet, or when it cannot be told (no answer). A
    /// block above the node's own newest block is not an older one: the node
    /// is a little behind and will have it.
    pub fn missing(oldest_needed: Option<u32>, node_blocks: u32, node_has_it: Option<bool>) -> Option<u32> {
        let oldest = oldest_needed?;
        (oldest <= node_blocks && node_has_it == Some(false)).then_some(oldest)
    }

    /// This many blocks (five minutes) the node is ahead of what the wallet
    /// last heard: the wallet's connection to the node has gone quiet.
    pub const QUIET_BLOCKS: u32 = 12;

    /// Whether the wallet has stopped hearing of the node's new blocks.
    pub fn wallet_quiet(node_blocks: u32, wallet_network: u32) -> bool {
        wallet_network > 0 && node_blocks.saturating_sub(wallet_network) >= QUIET_BLOCKS
    }
}

/// The words, the same as on Lock it Mac (Windows: "this PC").
pub mod words {
    fn grouped(n: u32) -> String {
        let digits = n.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    }

    /// Why Lock it reads from a server although the node is up to date.
    pub fn older_blocks(from: u32, server: &str) -> String {
        format!(
            "Lock it reads older blocks (from block {}) from {server}: this node keeps only recent blocks. It returns to this node by itself once they are checked.",
            grouped(from)
        )
    }

    /// The same, when there is no server to go back to by itself.
    pub fn older_blocks_on_node(from: u32) -> String {
        format!(
            "This node keeps only recent blocks and no longer has the older ones your wallet still has to check (from block {}). Pick a server in Settings › Network to check them.",
            grouped(from)
        )
    }

    /// Lock it's server did not answer when Lock it went back to it.
    pub fn unsettled(server: &str) -> String {
        format!("Lock it could not connect to {server} yet · trying again.")
    }

    /// A new server did not answer: Lock it stays where it was.
    pub fn could_not_connect(new: &str, old: &str) -> String {
        format!("Could not connect to {new}. Lock it stays on {old}.")
    }

    /// The wallet could not be added yet: the connection is still being made.
    pub const STILL_CONNECTING: &str = "Lock it is still connecting. Try again in a moment.";
    /// The node quit before it first answered.
    pub const STOPPED_WHILE_STARTING: &str =
        "The node stopped while starting. Your wallet is fine: Lock it reads from its server meanwhile.";
    /// After Try again.
    pub const TRYING_AGAIN: &str = "Trying again in half a minute…";
    /// "Still not working?"
    pub const STILL_NOT_WORKING: &str = "Quit Lock it and open it again. If the node still stops, click \"Remove node and its data…\" below and switch Go Max on: it starts over. To help us, send the node's log with your feedback.";
    /// The node moves on and the wallet does not hear of it.
    pub const WALLET_QUIET: &str =
        "Lock it is not getting new blocks from this node. It has connected again; if this line stays, switch Go Max off and on.";

    /// Whether a problem means "not connected yet" (the server or the
    /// wallet's database busy), not a wrong input.
    pub fn still_connecting(text: &str) -> bool {
        let text = text.to_lowercase();
        [
            "database is locked",
            "database is busy",
            "transport error",
            "connection refused",
            "timed out",
            "dns error",
            "tcp connect",
            "unavailable",
            "connection reset",
        ]
        .iter()
        .any(|needle| text.contains(needle))
    }

    /// "Dec 5", in English (as on the Mac), from Unix seconds (UTC).
    pub fn date_text(unix: i64) -> String {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        // Days since 1970 to a civil date (Howard Hinnant's algorithm).
        let z = unix.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        format!("{} {}", MONTHS[(month - 1) as usize], day)
    }

    /// Two weeks ahead of the stop.
    pub fn update_before(date: &str) -> String {
        format!(
            "Go Max needs an update before {date}. This node version follows the chain up to block {}; a newer Lock it brings the next one.",
            grouped(super::support::STOP_HEIGHT)
        )
    }

    /// At the stop.
    pub fn ended() -> String {
        format!(
            "This node version follows the Zcash chain up to block {}, and the network is now past it. Go Max carries on after an update of Lock it; the node's data is kept. Your Zcash is not affected.",
            grouped(super::support::STOP_HEIGHT)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{reading, support, words};
    use support::{State, STOP_HEIGHT};

    const NOW: i64 = 1_794_600_000;

    // ---- 6. No early "needs an update" during the first sync ----

    #[test]
    fn the_estimate_counts_only_near_the_tip() {
        // First check of history: the estimate is ~60,000 too high.
        assert_eq!(support::network_height(556_480, 3_628_874, 0.15, 3_568_874), 3_568_874);
        // Verified blocks above the wallet's server: those count.
        assert_eq!(support::network_height(3_570_000, 3_630_000, 0.998, 3_568_874), 3_570_000);
        // Near the tip the estimate counts.
        assert_eq!(support::network_height(3_569_990, 3_570_002, 0.9995, 3_569_000), 3_570_002);
        // Nothing from the wallet: the node alone.
        assert_eq!(support::network_height(100, 200, 0.5, 0), 100);
    }

    #[test]
    fn mid_sync_the_wrong_estimate_shows_no_notice_and_no_stop() {
        let height = support::network_height(556_480, STOP_HEIGHT + 100, 0.15, 3_568_874);
        assert_eq!(support::state(height, None, NOW, STOP_HEIGHT), State::Fine);
    }

    #[test]
    fn the_notice_shows_only_in_the_last_two_weeks() {
        let two_weeks = support::NOTICE_BLOCKS;
        assert_eq!(support::state(STOP_HEIGHT - two_weeks - 1, None, NOW, STOP_HEIGHT), State::Fine);
        assert_eq!(
            support::state(STOP_HEIGHT - two_weeks, None, NOW, STOP_HEIGHT),
            State::Soon { at: NOW + i64::from(two_weeks) * 25 }
        );
        assert_eq!(support::state(STOP_HEIGHT - 10, None, NOW, STOP_HEIGHT), State::Soon { at: NOW + 250 });
        assert_eq!(support::state(STOP_HEIGHT + 1, None, NOW, STOP_HEIGHT), State::Ended);
        assert_eq!(support::state(0, None, NOW, STOP_HEIGHT), State::Fine);
    }

    #[test]
    fn an_old_height_is_projected_with_a_day_of_grace() {
        // Seen 3 days before the stop, Lock it closed for 4 days: past the
        // stop only once a day of grace is used up.
        let reported = STOP_HEIGHT - 3 * 3_456;
        assert!(matches!(support::state(reported, Some(NOW - 2 * 86_400), NOW, STOP_HEIGHT), State::Soon { .. }));
        assert!(matches!(support::state(reported, Some(NOW - 4 * 86_400 + 600), NOW, STOP_HEIGHT), State::Soon { .. }));
        assert_eq!(support::state(reported, Some(NOW - 5 * 86_400), NOW, STOP_HEIGHT), State::Ended);
    }

    #[test]
    fn the_stop_block_is_zakuras() {
        assert_eq!(STOP_HEIGHT, 3_643_309);
        assert!(words::update_before("Dec 5").contains("block 3,643,309"));
        assert!(words::ended().contains("block 3,643,309"));
    }

    // ---- 1. Older blocks come from the server ----

    #[test]
    fn an_older_block_the_node_lacks_goes_to_the_server() {
        assert_eq!(reading::missing(Some(2_900_000), 3_568_000, Some(false)), Some(2_900_000));
        // The node has it, or cannot say: stay on the node.
        assert_eq!(reading::missing(Some(2_900_000), 3_568_000, Some(true)), None);
        assert_eq!(reading::missing(Some(2_900_000), 3_568_000, None), None);
        // Nothing left to scan.
        assert_eq!(reading::missing(None, 3_568_000, Some(false)), None);
        // Above the node's newest block: the node is a little behind.
        assert_eq!(reading::missing(Some(3_568_005), 3_568_000, Some(false)), None);
    }

    #[test]
    fn the_older_blocks_line_is_the_macs() {
        assert_eq!(
            words::older_blocks(2_900_000, "zec.rocks"),
            "Lock it reads older blocks (from block 2,900,000) from zec.rocks: this node keeps only recent blocks. It returns to this node by itself once they are checked."
        );
    }

    // ---- 4. Wallet quiet on the node ----

    #[test]
    fn twelve_blocks_ahead_is_quiet() {
        assert!(!reading::wallet_quiet(3_568_011, 3_568_000));
        assert!(reading::wallet_quiet(3_568_012, 3_568_000));
        assert!(!reading::wallet_quiet(3_568_012, 0));
        assert!(!reading::wallet_quiet(3_567_990, 3_568_000));
    }

    // ---- 2. Adding a wallet: "still connecting" ----

    #[test]
    fn a_connection_not_ready_is_still_connecting() {
        assert!(words::still_connecting("database is locked"));
        assert!(words::still_connecting("the server did not answer: transport error"));
        assert!(words::still_connecting("status: Unavailable, message: \"tcp connect error\""));
        assert!(!words::still_connecting("these are the main wallet's words"));
        assert!(!words::still_connecting("this wallet is already on the key ring"));
        assert_eq!(words::STILL_CONNECTING, "Lock it is still connecting. Try again in a moment.");
    }

    #[test]
    fn the_stop_date_reads_as_on_the_mac() {
        // 5 Dec 2026, 12:00 UTC.
        assert_eq!(words::date_text(1_796_472_000), "Dec 5");
        assert_eq!(words::date_text(0), "Jan 1");
        assert_eq!(words::date_text(1_709_208_000), "Feb 29");
    }

    // ---- 5. Node stops while starting ----

    #[test]
    fn the_calm_help_is_the_macs() {
        assert_eq!(words::STOPPED_WHILE_STARTING, "The node stopped while starting. Your wallet is fine: Lock it reads from its server meanwhile.");
        assert_eq!(words::TRYING_AGAIN, "Trying again in half a minute…");
        assert!(words::STILL_NOT_WORKING.starts_with("Quit Lock it and open it again."));
        assert!(words::STILL_NOT_WORKING.contains("\"Remove node and its data…\""));
    }

    #[test]
    fn the_quiet_line_is_the_macs() {
        assert_eq!(words::WALLET_QUIET, "Lock it is not getting new blocks from this node. It has connected again; if this line stays, switch Go Max off and on.");
    }

    // ---- 3. A failed server switch ----

    #[test]
    fn a_failed_switch_names_both_servers() {
        assert_eq!(words::could_not_connect("example.org", "zec.rocks"), "Could not connect to example.org. Lock it stays on zec.rocks.");
        assert_eq!(words::unsettled("zec.rocks"), "Lock it could not connect to zec.rocks yet · trying again.");
    }
}
