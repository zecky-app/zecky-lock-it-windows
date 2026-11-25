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
    /// `MAINNET_EOS_HEIGHT` of Zakura 1.8.5 to 1.8.9: about Dec 5, 2026.
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

    /// NU7's block and its expected moment (5 Nov 2026, 12:00 UTC); a block
    /// every 25 seconds since.
    const NU7_HEIGHT: u32 = 3_543_405;
    const NU7_TIME: i64 = 1_793_880_000;
    /// Room for blocks coming faster than every 25 seconds.
    pub const PLAUSIBLE_MARGIN: u32 = 5_000;

    /// A rough chain height for a moment, from the clock alone (as Lock it
    /// Mac's `Birthday.estimatedTip`). Only used as a ceiling.
    pub fn estimated_tip(now: i64) -> u32 {
        let since = (now - NU7_TIME).max(0) / BLOCK_SECONDS;
        NU7_HEIGHT.saturating_add(u32::try_from(since).unwrap_or(u32::MAX))
    }

    /// The highest network height the clock allows at a moment. Verified
    /// blocks are never capped by it; guesses and remembered heights are.
    pub fn plausible_max(now: i64) -> u32 {
        estimated_tip(now).saturating_add(PLAUSIBLE_MARGIN)
    }

    /// A height remembered across restarts, if the clock allows it when it
    /// was seen; 0 otherwise. One remembered from a guess can be far too
    /// high and would show "needs an update" weeks early whenever nothing is
    /// answering yet (seen on the Mac, 14 Nov 2026: "before Nov 27").
    pub fn trusted_remembered(height: u32, seen_at: Option<i64>, now: i64) -> u32 {
        if height == 0 {
            return 0;
        }
        if height <= plausible_max(seen_at.unwrap_or(now)) { height } else { 0 }
    }

    /// The network height the stop is judged by. The node's own estimate of
    /// the network is only trusted once the node is close to the newest
    /// block: during the first check of history it can be far too high (seen
    /// 13 Nov 2026: about 60,000 blocks, which showed "needs an update
    /// before Nov 17" three weeks early and could have stopped Go Max
    /// mid-sync). Before that, the node's verified blocks and the wallet's
    /// server count. An estimate is never trusted above what the clock
    /// allows; verified blocks always are.
    pub fn network_height(node_blocks: u32, node_estimate: u32, progress: f64, wallet_network: u32, now: i64) -> u32 {
        let estimate = node_estimate.min(plausible_max(now));
        let node = if progress >= 0.999 { node_blocks.max(estimate) } else { node_blocks };
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

    /// Blocks the node surely keeps in full: its smallest setting (10,000,
    /// while it syncs; 100,000 once caught up), less a margin.
    pub const SURELY_KEPT: u32 = crate::config::RETENTION_SYNCING - 1_000;

    /// The block the wallet needs that is older than the node surely keeps,
    /// whatever one block check says. Seen 25 Nov 2026 (Acer, a Keystone
    /// wallet paired as "already holds Zcash"): the node answered for the
    /// oldest block, but the blocks after it did not line up, and every pass
    /// failed with "put_blocks requires that the provided block range be
    /// sequential". The public server read them fine.
    pub fn too_old(oldest_needed: Option<u32>, node_blocks: u32) -> Option<u32> {
        let oldest = oldest_needed?;
        (oldest.saturating_add(SURELY_KEPT) <= node_blocks).then_some(oldest)
    }

    /// Whether a pass failed because the blocks the node served did not line
    /// up with the wallet's (the wallet library refuses to store them).
    pub fn blocks_do_not_line_up(error: &str) -> bool {
        error.contains("block range be sequential")
    }

    /// This many blocks (five minutes) the node is ahead of what the wallet
    /// last heard: the wallet's connection to the node has gone quiet.
    pub const QUIET_BLOCKS: u32 = 12;

    /// Whether the wallet has stopped hearing of the node's new blocks.
    pub fn wallet_quiet(node_blocks: u32, wallet_network: u32) -> bool {
        wallet_network > 0 && node_blocks.saturating_sub(wallet_network) >= QUIET_BLOCKS
    }
}

/// "Only sync while plugged in": when Go Max waits for the charger. The same
/// rule as Lock it Mac (decided 16 Nov 2026). Only the heavy part waits: a
/// node that is up to date is light (a block every 75 seconds) and keeps
/// running on battery. A short unplug changes nothing.
pub mod battery {
    /// On battery this long, in a row, before a catching-up node pauses.
    pub const PAUSE_AFTER_SECONDS: i64 = 60;

    /// Whether to pause the node now: it is checking or catching up, and the
    /// PC has been on battery for about a minute. `on_battery_since` is when
    /// the PC went on battery (`None`: on the charger).
    pub fn pause(catching_up: bool, on_battery_since: Option<i64>, now: i64) -> bool {
        catching_up && on_battery_since.is_some_and(|since| now - since >= PAUSE_AFTER_SECONDS)
    }

    /// Whether a start on battery waits for the charger: only for a node that
    /// has never been up to date (its first check is all heavy work).
    pub fn hold_at_start(on_battery: bool, ever_up_to_date: bool) -> bool {
        on_battery && !ever_up_to_date
    }
}

/// How long the first check still takes, with the spam years counted in.
///
/// Speed is measured on this PC: blocks checked in the last hour. Blocks
/// are not equal, though. From mid-2022 to early 2023 someone filled most
/// blocks with spam (shielded transactions with hundreds of outputs, up to
/// 2 MB a block), and the chain grew from about 30 GB to well over 100 GB.
/// So each stretch of the chain gets a weight, roughly its work per block,
/// and the estimate measures speed in that work rather than in blocks. A
/// node in the spam years then knows the rest will go faster, and a node
/// before them knows they are coming. Only the ratios are guessed; the
/// speed is this PC's own, and the estimate corrects itself as it goes.
pub mod pace {
    use std::collections::VecDeque;

    /// The spam years, roughly: from mid-June 2022 (shortly after NU5 at
    /// 1,687,104) until the spam faded in 2023.
    pub const SPAM_START: u32 = 1_710_000;
    /// See [`SPAM_START`].
    pub const SPAM_END: u32 = 2_100_000;
    /// Canopy: before it, blocks are taken on Zakura's built-in checkpoints.
    const CANOPY: u32 = 1_046_400;

    /// From this height on, this much work per block (relative).
    const STRETCHES: [(u32, f64); 4] = [(0, 1.0), (CANOPY, 3.0), (SPAM_START, 20.0), (SPAM_END, 4.0)];

    /// The work to check every block below `height`.
    pub fn work(height: u32) -> f64 {
        let mut total = 0.0;
        for (i, &(from, weight)) in STRETCHES.iter().enumerate() {
            if height <= from {
                break;
            }
            let until = STRETCHES.get(i + 1).map_or(height, |next| next.0.min(height));
            total += f64::from(until - from) * weight;
        }
        total
    }

    /// Speed is taken over the last hour…
    pub const WINDOW_SECONDS: i64 = 3_600;
    /// …and only once there is a quarter of an hour of it.
    pub const MIN_SECONDS: i64 = 900;

    /// The node's height over the last hour.
    #[derive(Default)]
    pub struct Samples {
        list: VecDeque<(i64, u32)>,
    }

    impl Samples {
        /// Note the height now. A lower height than before (the node
        /// started over) begins a fresh measurement.
        pub fn add(&mut self, now: i64, height: u32) {
            if self.list.back().is_some_and(|&(_, last)| height < last) {
                self.list.clear();
            }
            self.list.push_back((now, height));
            while self.list.front().is_some_and(|&(at, _)| now - at > WINDOW_SECONDS) {
                self.list.pop_front();
            }
        }

        /// Start over (Go Max was switched off or paused).
        pub fn clear(&mut self) {
            self.list.clear();
        }

        /// When the node should reach `tip`, in Unix seconds: `None` while
        /// there is too little to go on or nothing moved.
        pub fn finish(&self, tip: u32) -> Option<i64> {
            let &(first_at, first) = self.list.front()?;
            let &(last_at, last) = self.list.back()?;
            let seconds = last_at - first_at;
            if seconds < MIN_SECONDS || last <= first || tip <= last {
                return None;
            }
            let speed = (work(last) - work(first)) / seconds as f64;
            let left = (work(tip) - work(last)) / speed;
            Some(last_at + left.round() as i64)
        }
    }

    /// The line about the spam years, while the first check runs.
    pub fn spam_line(height: u32) -> String {
        if height < SPAM_START {
            "Ahead: the spam years, blocks 1,710,000 to 2,100,000 (2022–23). Most blocks there are full of spam, so this is the slowest stretch. The expected finish already counts it.".into()
        } else if height < SPAM_END {
            let done = (height - SPAM_START) * 100 / (SPAM_END - SPAM_START);
            format!("Now in the spam years, blocks 1,710,000 to 2,100,000 (2022–23): {done}% of this stretch done. It is the slowest part; after block 2,100,000 it goes much faster.")
        } else {
            "The spam years are done. The rest goes much faster.".into()
        }
    }
}

/// Up to date for sending: both heights known and the wallet at most 3
/// blocks behind the network (Lock it Mac's SyncMath.nearlyCurrent).
pub fn nearly_current(network: u32, wallet: u32) -> bool {
    network > 0 && wallet > 0 && network.saturating_sub(wallet) <= 3
}

/// The newest blocks are checked without a gap: nothing is left to check
/// from `window` blocks below the network block up to 3 below it (Lock it
/// Mac's SyncMath.newestChecked). Older blocks may still be open, for a
/// wallet just added; they do not change what the chain looks like now, so
/// they do not hold up sending. `to_scan` holds [start, end) ranges.
pub fn newest_checked(to_scan: &[(u32, u32)], network: u32, window: u32) -> bool {
    if network <= window {
        return false;
    }
    let (low, high) = (network - window, network - 3);
    !to_scan.iter().any(|&(first, end)| first <= high && end > low)
}

/// The window for `newest_checked`: about 40 minutes of blocks.
pub const NEWEST_WINDOW: u32 = 100;

/// A wallet's own blocks, from its start to the newest, are all checked:
/// no open range between them (the last 3 blocks may be new). As Lock it
/// Mac's SyncMath.ownChecked.
pub fn own_checked(to_scan: &[(u32, u32)], start: u32, network: u32) -> bool {
    if network == 0 {
        return false;
    }
    let high = network.saturating_sub(3);
    !to_scan.iter().any(|&(first, end)| end > start && first < high)
}

/// Why a wallet can't send yet.
#[derive(Debug, PartialEq, Eq)]
pub enum SendWait {
    /// The newest blocks are not checked yet.
    CatchingUp,
    /// The newest are, but this wallet's own older blocks (from `since`)
    /// are still open: its balance may not be complete.
    OlderBlocks {
        /// The wallet's start.
        since: u32,
    },
}

/// Whether the paying wallet may send: everything checked to within 3
/// blocks of the network; or the newest blocks checked and this wallet's
/// own blocks from its start `own_start` too. A wallet just added with an
/// older start waits while the others send (as Lock it Mac).
pub fn send_check(network: u32, wallet: u32, to_scan: &[(u32, u32)], own_start: u32) -> Result<(), SendWait> {
    if nearly_current(network, wallet) {
        return Ok(());
    }
    if !newest_checked(to_scan, network, NEWEST_WINDOW) {
        return Err(SendWait::CatchingUp);
    }
    if own_checked(to_scan, own_start, network) {
        Ok(())
    } else {
        Err(SendWait::OlderBlocks { since: own_start })
    }
}

/// Blocks below the tip still to check, in all ranges, overlaps once.
pub fn open_count(to_scan: &[(u32, u32)], tip: u32) -> u64 {
    let mut parts: Vec<(u32, u32)> = to_scan.iter().map(|&(a, e)| (a, e.min(tip))).filter(|&(a, e)| e > a).collect();
    parts.sort_unstable();
    let (mut open, mut reach) = (0u64, 0u32);
    for (a, e) in parts {
        let from = a.max(reach);
        if e > from {
            open += u64::from(e - from);
        }
        reach = reach.max(e);
    }
    open
}

/// Minutes to go from how fast blocks get checked: each sample is (seconds,
/// blocks still open in all ranges), so the pace holds while older blocks
/// are checked for a new wallet, when the gap-free height hardly moves.
/// None until there are 3 samples over at least 20 seconds and the open
/// count is falling (Lock it Mac's SyncMath.minutesLeft).
pub fn minutes_left(open: u64, samples: &[(f64, u64)]) -> Option<u64> {
    if open == 0 || samples.len() < 3 {
        return None;
    }
    let (first, last) = (samples[0], samples[samples.len() - 1]);
    let seconds = last.0 - first.0;
    if seconds < 20.0 {
        return None;
    }
    let rate = (first.1 as f64 - last.1 as f64) / seconds;
    if rate <= 0.0 {
        return None;
    }
    Some(((open as f64 / rate / 60.0).round() as u64).max(1))
}

/// A payment is news, worth a message, when it is still waiting for a
/// block, or in one of the last 100 blocks (about 40 minutes), or its block
/// is from the last 30 minutes, or from after Lock it was last up to date
/// (`since`: a payment that came while the PC slept). Older payments found
/// later, for a wallet just added, are history (Lock it Mac's isNews).
/// Times in Unix seconds.
pub fn is_news(block: Option<u32>, block_time: Option<i64>, network: u32, since: Option<i64>, now: i64) -> bool {
    let Some(block) = block else { return true };
    if network > 0 && block >= network.saturating_sub(100) {
        return true;
    }
    let recent = now - 30 * 60;
    let from = since.map_or(recent, |since| since.min(recent));
    block_time.unwrap_or(0) >= from
}

/// The camera's state for Settings › Security (as Lock it Mac's
/// CameraAccess): Windows' own switch (`windows`: Some(false) when the
/// camera or desktop apps' access to it is off, None when unknown) and Lock
/// it's own permission in its window ("granted", "prompt" or "denied").
pub fn camera_text(windows: Option<bool>, window: &str) -> &'static str {
    if windows == Some(false) || window == "denied" {
        "Off"
    } else if window == "granted" {
        "Allowed · only while scanning"
    } else {
        "Not asked yet"
    }
}

/// Reads one value of Windows' camera consent from `reg query` output:
/// Some(true) for Allow, Some(false) for Deny, None when absent.
pub fn camera_consent(reg_output: &str) -> Option<bool> {
    reg_output.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next() == Some("Value") && words.next() == Some("REG_SZ")).then(|| words.next() == Some("Allow"))
    })
}

#[cfg(test)]
mod camera_tests {
    use super::{camera_consent, camera_text};

    /// Lock it Mac's CameraAccess texts, with Windows' own switch on top.
    #[test]
    fn the_camera_state_in_words() {
        assert_eq!(camera_text(Some(true), "granted"), "Allowed · only while scanning");
        assert_eq!(camera_text(None, "granted"), "Allowed · only while scanning");
        assert_eq!(camera_text(Some(true), "prompt"), "Not asked yet");
        assert_eq!(camera_text(Some(false), "granted"), "Off", "Windows' switch is off");
        assert_eq!(camera_text(Some(true), "denied"), "Off");
        let allow = "\r\nHKEY_CURRENT_USER\\Software\\...\\webcam\r\n    Value    REG_SZ    Allow\r\n";
        assert_eq!(camera_consent(allow), Some(true));
        assert_eq!(camera_consent("    Value    REG_SZ    Deny"), Some(false));
        assert_eq!(camera_consent("ERROR: The system was unable to find the specified registry key or value."), None);
    }
}

#[cfg(test)]
mod news_tests {
    use super::is_news;

    /// Lock it Mac's testOnlyNewPaymentsGetAMessage and the sleep case of
    /// testAuditBeforeSaturday.
    #[test]
    fn only_new_payments_get_a_message() {
        let now = 2_000_000_000;
        assert!(is_news(None, None, 3_609_479, None, now), "waiting for a block");
        assert!(is_news(Some(3_609_400), None, 3_609_479, None, now), "one of the last 100 blocks");
        assert!(is_news(Some(3_500_000), Some(now - 600), 0, None, now), "network unknown, block 10 minutes old");
        assert!(!is_news(Some(2_400_000), Some(now - 86_400 * 600), 3_609_479, None, now), "a payment from 2024");
        assert!(!is_news(Some(3_609_000), None, 3_609_479, None, now), "older than 100 blocks, no time");
        // Up to date at 22:00, asleep, paid at 23:00, awake at 07:00.
        let last_up = now - 9 * 3_600;
        let paid = now - 8 * 3_600;
        assert!(is_news(Some(3_608_000), Some(paid), 3_609_479, Some(last_up), now));
        assert!(!is_news(Some(3_608_000), Some(paid), 3_609_479, None, now), "fresh start: history");
        assert!(!is_news(Some(2_400_000), Some(last_up - 86_400 * 300), 3_609_479, Some(last_up), now), "old payment for a new wallet");
    }
}

#[cfg(test)]
mod newest_blocks_tests {
    use super::*;

    /// Lock it Mac's testOlderBlocksForANewWalletDoNotHoldUpSending.
    #[test]
    fn older_blocks_for_a_new_wallet_do_not_hold_up_sending() {
        let tip = 3_609_479;
        let history = [(1_687_104, 3_550_000)];
        assert!(newest_checked(&history, tip, NEWEST_WINDOW));
        assert_eq!(send_check(tip, 1_687_104, &history, 3_550_000), Ok(()), "older blocks open, the newest checked");
        assert!(!newest_checked(&[(tip - 1_000, tip + 1)], tip, NEWEST_WINDOW));
        assert_eq!(send_check(tip, tip - 1_000, &[(tip - 1_000, tip + 1)], 3_550_000), Err(SendWait::CatchingUp));
        assert!(!newest_checked(&[(tip - 50, tip - 40)], tip, NEWEST_WINDOW), "a gap in the newest 100");
        assert!(newest_checked(&[(tip - 2, tip + 1)], tip, NEWEST_WINDOW), "the last 2 blocks may still be open");
        assert!(newest_checked(&[], tip, NEWEST_WINDOW));
        assert!(!newest_checked(&[], 0, NEWEST_WINDOW), "network block unknown");
        assert_eq!(send_check(tip, tip - 2, &[(tip - 2, tip + 1)], 3_550_000), Ok(()));
    }

    /// Lock it Mac's testAuditBeforeSaturday (1): newest checked, older
    /// blocks open below 3,524,536 for another wallet.
    #[test]
    fn each_wallet_sends_once_its_own_blocks_are_checked() {
        let open = [(2_306_540, 3_200_000)];
        assert!(own_checked(&open, 3_524_536, 3_609_479));
        assert!(!own_checked(&open, 2_306_540, 3_609_479));
        assert!(own_checked(&[(3_609_478, 3_609_480)], 3_524_536, 3_609_479), "the newest 3 may be new");
        assert_eq!(send_check(3_609_479, 2_306_540, &open, 3_524_536), Ok(()), "the main wallet sends");
        assert_eq!(send_check(3_609_479, 2_306_540, &open, 2_306_540), Err(SendWait::OlderBlocks { since: 2_306_540 }), "a wallet with an older start waits");
        assert_eq!(send_check(3_609_479, 3_608_000, &[(3_608_000, 3_609_480)], 3_524_536), Err(SendWait::CatchingUp));
        assert_eq!(send_check(3_609_479, 3_609_477, &[], 2_306_540), Ok(()), "everything checked");
        assert!(!own_checked(&[], 1, 0), "network block unknown");
        let said = words::older_blocks_before_send(2_306_540);
        assert!(said.contains("since block 2,306,540") && said.contains("your other wallets can send now") && said.ends_with("Nothing was sent."), "{said}");
    }

    #[test]
    fn open_blocks_count_overlaps_once_and_stop_at_the_tip() {
        assert_eq!(open_count(&[(100, 200), (150, 250), (900, 1_200)], 1_000), 250);
        assert_eq!(open_count(&[], 1_000), 0);
    }

    #[test]
    fn time_left_follows_blocks_checked() {
        // 300 blocks a second: 1,200,000 open is about 67 minutes.
        let samples: Vec<(f64, u64)> = (0..4).map(|i| (f64::from(i) * 10.0, 1_209_000 - u64::from(i as u32) * 3_000)).collect();
        assert_eq!(minutes_left(1_200_000, &samples), Some(67));
        assert_eq!(minutes_left(1_200_000, &samples[..2]), None, "too few samples");
        let quick: Vec<(f64, u64)> = samples[..3].iter().map(|&(_, open)| (0.0, open)).collect();
        assert_eq!(minutes_left(1_200_000, &quick), None, "under 20 seconds");
        let rising: Vec<(f64, u64)> = samples.iter().rev().enumerate().map(|(i, &(_, open))| (i as f64 * 10.0, open)).collect();
        assert_eq!(minutes_left(1_200_000, &rising), None, "not falling");
        assert_eq!(minutes_left(0, &samples), None);
    }
}

#[cfg(test)]
mod send_rule_tests {
    use super::{nearly_current, words};

    /// Mac 21 Nov 2026: a payment priced about 1,000 blocks behind expired.
    #[test]
    fn nothing_is_built_before_the_wallet_is_up_to_date() {
        assert!(nearly_current(3_597_825, 3_597_825));
        assert!(nearly_current(3_597_825, 3_597_822), "a few blocks behind counts as up to date");
        assert!(!nearly_current(3_597_825, 3_597_821));
        assert!(!nearly_current(3_597_825, 3_596_744));
        assert!(!nearly_current(0, 0));
        assert!(!nearly_current(3_597_825, 0));
        assert_eq!(
            words::catching_up_before_send(3_596_744, 3_597_825),
            "Lock it is still catching up (block 3,596,744 of 3,597,825). You can send once it says Up to date. Nothing was sent."
        );
    }

    #[test]
    fn a_partly_sent_payment_never_says_nothing_was_sent() {
        let text = words::partly_refused("-25: expired");
        assert!(text.starts_with("Only part of this payment went through"));
        assert!(!text.contains("Nothing was sent"));
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

    /// Sending from a wallet whose own older blocks are still checked
    /// (Lock it Mac's olderBlocks).
    pub fn older_blocks_before_send(since: u32) -> String {
        format!(
            "Lock it is still checking this wallet's older blocks (since block {}), so its balance may not be complete yet. You can send from it once that is done; your other wallets can send now. Nothing was sent.",
            grouped(since)
        )
    }

    /// Send is asked while the wallet is still catching up (as Lock it Mac).
    pub fn catching_up_before_send(wallet: u32, network: u32) -> String {
        if wallet == 0 || network == 0 {
            return "Lock it is still catching up with the Zcash network. You can send once it says Up to date.".into();
        }
        format!(
            "Lock it is still catching up (block {} of {}). You can send once it says Up to date. Nothing was sent.",
            grouped(wallet),
            grouped(network)
        )
    }

    /// A payment of two transactions (TEX) of which the first went through.
    pub fn partly_refused(why: &str) -> String {
        format!("Only part of this payment went through: the first step was sent, the next was refused ({why}). Your Zcash is not lost: it is on a temporary address of this wallet. Check Activity before sending again.")
    }

    /// A payment that never got through has expired: it was never paid.
    pub fn waiting_payment_expired(name: &str) -> String {
        format!("{name} expired and was never paid. Work out a new one.")
    }

    /// A worked-out payment waited too long before signing.
    pub const WORK_OUT_AGAIN: &str = "This payment was worked out a while ago. Work it out again, so the network still takes it.";

    /// "Your own node (Go Max)" is chosen, but the node can't serve the
    /// wallet yet (off, catching up, or its wallet service is starting).
    pub fn own_node_not_ready(server: &str) -> String {
        format!("Go Max is not up to date yet, so Lock it reads from {server} meanwhile. It moves to your own node by itself once it is.")
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

    /// Where a newer Lock it is.
    pub const DOWNLOAD_PAGE: &str = "zecky.app/windows.html";

    /// Three or four blocks behind the network's height still counts as up
    /// to date: the estimate runs ahead of the real tip (as on the Mac).
    pub const UP_TO_DATE_BLOCKS: u32 = 10;

    /// Whether the node is shown as up to date.
    pub fn shown_up_to_date(node_blocks: u32, network: u32) -> bool {
        network > 0 && node_blocks.saturating_add(UP_TO_DATE_BLOCKS) >= network
    }

    /// The node is up but not quite at the newest block.
    pub fn a_little_behind(blocks: u32) -> String {
        format!("A little behind · {} block{} to catch up", grouped(blocks), if blocks == 1 { "" } else { "s" })
    }

    /// Caught up, but Lock it has not moved to the node yet.
    pub fn still_on_server(server: &str) -> String {
        format!(
            "Lock it still reads the chain from {server}. It moves to this node by itself once the node has stayed at the newest block for half a minute."
        )
    }

    /// No other Zcash nodes for a while.
    pub const NOT_CONNECTED: &str = "Not connected to other Zcash nodes · check this PC's internet connection. Some networks (office, hotel, guest Wi-Fi) block Zcash nodes. A second Zcash node on the same internet connection, such as Rock it, may also be taking the peers.";

    /// Paused on battery (Only sync while plugged in).
    pub const PAUSED_ON_BATTERY: &str = "Paused while on battery. Go Max carries on when the charger is connected; Lock it reads from its server meanwhile.";

    /// The node from last time is still closing.
    pub fn port_still_closing(port: u16) -> String {
        format!("Go Max's node from the last time is still closing (port {port}). Wait a minute, then switch Go Max on again.")
    }

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
            "Go Max needs an update before {date}. This node version follows the chain up to block {}; a newer Lock it brings the next one. Get the new version at zecky.app/windows.html.",
            grouped(super::support::STOP_HEIGHT)
        )
    }

    /// At the stop.
    pub fn ended() -> String {
        format!(
            "This node version follows the Zcash chain up to block {}, and the network is now past it. Go Max carries on after an update of Lock it; the node's data is kept. Your Zcash is not affected. Get the new version at zecky.app/windows.html.",
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
    fn only_catching_up_waits_for_the_charger_and_not_at_once() {
        use super::battery::*;
        let t = 1_000;
        // Up to date: keeps running on battery, however long.
        assert!(!pause(false, Some(t), t + 3_600));
        // Catching up: a short unplug changes nothing; after a minute it pauses.
        assert!(!pause(true, Some(t), t + 15));
        assert!(!pause(true, Some(t), t + 59));
        assert!(pause(true, Some(t), t + 60));
        // On the charger: never.
        assert!(!pause(true, None, t));
        // At start: wait only if the node was never up to date.
        assert!(hold_at_start(true, false));
        assert!(!hold_at_start(true, true));
        assert!(!hold_at_start(false, false));
    }

    #[test]
    fn spam_years_weigh_more_and_the_finish_counts_them() {
        use super::pace::*;
        assert_eq!(work(0), 0.0);
        // A spam block is much more work than one before or after.
        assert!(work(SPAM_START + 1) - work(SPAM_START) > 4.0 * (work(SPAM_END + 1) - work(SPAM_END)));
        // 7,200 blocks an hour in the spam years, as on a tester's laptop.
        let mut s = Samples::default();
        let t = 1_763_200_000;
        s.add(t, 1_855_000);
        assert_eq!(s.finish(3_560_000), None, "one sample is not a speed");
        s.add(t + 3_600, 1_862_200);
        let finish = s.finish(3_560_000).unwrap();
        // At spam speed the rest would take (3,560,000 - 1,862,200) / 7,200
        // = 236 hours; counted with the faster rest it is far less.
        let hours = (finish - (t + 3_600)) / 3_600;
        assert!(hours > 30 && hours < 236, "{hours} hours");
        // The spam that is left alone: 237,800 blocks at 7,200 an hour.
        let spam_left = f64::from(SPAM_END - 1_862_200) / 7_200.0;
        assert!(hours as f64 > spam_left, "the spam left must fit in");
        // Nothing moved, or the node started over: no estimate.
        let mut still = Samples::default();
        still.add(t, 100);
        still.add(t + 1_000, 100);
        assert_eq!(still.finish(200), None);
        still.add(t + 2_000, 50);
        assert_eq!(still.finish(200), None);
    }

    #[test]
    fn the_spam_line_says_where_the_node_is() {
        use super::pace::spam_line;
        assert!(spam_line(1_000_000).starts_with("Ahead: the spam years"));
        assert!(spam_line(1_905_000).contains(": 50% of this stretch done"));
        assert!(spam_line(2_200_000).starts_with("The spam years are done"));
    }

    #[test]
    fn the_estimate_counts_only_near_the_tip() {
        // First check of history: the estimate is ~60,000 too high.
        assert_eq!(support::network_height(556_480, 3_628_874, 0.15, 3_568_874, NOW), 3_568_874);
        // Verified blocks above the wallet's server: those count.
        assert_eq!(support::network_height(3_570_000, 3_630_000, 0.998, 3_568_874, NOW), 3_570_000);
        // Near the tip the estimate counts.
        assert_eq!(support::network_height(3_569_990, 3_570_002, 0.9995, 3_569_000, NOW), 3_570_002);
        // Nothing from the wallet: the node alone.
        assert_eq!(support::network_height(100, 200, 0.5, 0, NOW), 100);
    }

    #[test]
    fn mid_sync_the_wrong_estimate_shows_no_notice_and_no_stop() {
        let height = support::network_height(556_480, STOP_HEIGHT + 100, 0.15, 3_568_874, NOW);
        assert_eq!(support::state(height, None, NOW, STOP_HEIGHT), State::Fine);
    }

    #[test]
    fn heights_the_clock_does_not_allow_are_not_trusted() {
        // The Mac's test: 14 Nov 2026, 13:10 UTC; the real tip was 3,572,285.
        let now = 1_794_661_800;
        let limit = support::plausible_max(now);
        assert!(limit > 3_572_285 && limit < 3_590_000, "{limit}");
        // A remembered guess about 26,000 blocks too high is dropped; a real one is kept.
        assert_eq!(support::trusted_remembered(3_598_400, Some(now), now), 0);
        assert_eq!(support::trusted_remembered(3_572_285, Some(now), now), 3_572_285);
        assert_eq!(support::trusted_remembered(0, None, now), 0);
        // Dropped, it shows no early notice.
        assert_eq!(support::state(support::trusted_remembered(3_598_400, Some(now), now), Some(now), now, STOP_HEIGHT), State::Fine);
        // A caught-up node's estimate above the clock is capped; its verified blocks are not.
        assert_eq!(support::network_height(3_572_285, 3_640_000, 1.0, 3_572_285, now), limit);
        assert_eq!(support::network_height(3_643_400, 0, 1.0, 0, now), 3_643_400);
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
        assert!(words::update_before("Dec 5").ends_with("Get the new version at zecky.app/windows.html."));
        assert!(words::ended().ends_with("Get the new version at zecky.app/windows.html."));
    }

    #[test]
    fn up_to_date_allows_a_few_blocks_and_says_why_still_on_the_server() {
        assert!(words::shown_up_to_date(3_572_281, 3_572_285));
        assert!(words::shown_up_to_date(3_572_275, 3_572_285));
        assert!(!words::shown_up_to_date(3_572_240, 3_572_285));
        assert!(!words::shown_up_to_date(3_572_285, 0));
        assert_eq!(words::a_little_behind(45), "A little behind · 45 blocks to catch up");
        assert_eq!(words::a_little_behind(1), "A little behind · 1 block to catch up");
        assert!(words::still_on_server("zec.rocks").starts_with("Lock it still reads the chain from zec.rocks."));
        assert!(words::NOT_CONNECTED.contains("such as Rock it"));
    }

    // ---- 1. Older blocks come from the server ----

    #[test]
    fn an_older_block_the_node_lacks_goes_to_the_server() {
        // Older than the node surely keeps: from the server, whatever the
        // one block check says (25 Nov, Acer).
        assert_eq!(reading::too_old(Some(3_000_000), 3_568_000), Some(3_000_000));
        assert_eq!(reading::too_old(Some(3_559_000), 3_568_000), Some(3_559_000));
        assert_eq!(reading::too_old(Some(3_559_001), 3_568_000), None);
        assert_eq!(reading::too_old(None, 3_568_000), None);
        assert_eq!(reading::too_old(Some(5), 100), None, "a node at the start is never 'too far ahead'");
        assert!(reading::blocks_do_not_line_up("the server did not answer: Error while interacting with wallet database: `put_blocks` requires that the provided block range be sequential"));
        assert!(!reading::blocks_do_not_line_up("the server did not answer: timed out"));
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
