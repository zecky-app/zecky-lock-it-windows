//! The time lock's waits, as on Lock it Mac (TimeLock.swift, 22 Nov 2026).
//!
//! Opening the lock, and a shorter time lock (or off) taking over, both wait
//! a number of hours. A wait has passed only when it has passed by the clock
//! *and* by the Zcash blocks made since it started (150 s per block before
//! Blossom, 75 s before NU7, 25 s since), like the 48 hours of "Lost my
//! keys". Moving this PC's clock forward alone opens nothing; with no block
//! height known, nothing is ready.

use serde::Serialize;

use crate::Policy;

/// One hour, in seconds.
pub const HOUR: i64 = 3600;
/// How long the lock can be opened once the wait is over.
pub const WINDOW: i64 = 24 * HOUR;
/// Network upgrades that changed the block time (mainnet heights).
pub const BLOSSOM: u32 = 653_600;
/// NU7: 25-second blocks from this height on.
pub const NU7: u32 = 3_543_405;

fn spacing(height: u32) -> i64 {
    if height < BLOSSOM {
        150
    } else if height < NU7 {
        75
    } else {
        25
    }
}

/// Target seconds of the blocks after `from`, up to and including `to`.
pub fn block_seconds(from: u32, to: u32) -> i64 {
    if to <= from {
        return 0;
    }
    let mut total = 0;
    let mut h = from + 1;
    for edge in [BLOSSOM, NU7, u32::MAX] {
        if h > to {
            break;
        }
        let end = to.min(edge.saturating_sub(1));
        if end >= h {
            total += (end - h + 1) as i64 * spacing(h);
            h = end + 1;
        }
    }
    total
}

/// The wait has passed by the clock and by the blocks since `height`.
pub fn passed(wait: i64, since: i64, height: Option<u32>, now: i64, tip: u32) -> bool {
    if now - since < wait {
        return false;
    }
    if wait <= 0 {
        return true;
    }
    match height {
        Some(h) if h > 0 && tip >= h => block_seconds(h, tip) >= wait,
        _ => false,
    }
}

/// Seconds of the wait the blocks have not covered yet.
pub fn blocks_left(wait: i64, height: Option<u32>, tip: u32) -> i64 {
    match height {
        Some(h) if h > 0 && tip >= h => (wait - block_seconds(h, tip)).max(0),
        _ => wait,
    }
}

/// Opening asked for: when, at which block, and when it may open by the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// When opening was asked for (Unix seconds).
    pub asked_at: i64,
    /// The block height then, if known.
    pub asked_height: Option<u32>,
    /// When the wait is over by the clock (Unix seconds).
    pub ready_at: i64,
}

/// Where the time lock stands.
#[derive(Serialize, PartialEq, Eq, Debug)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// No time lock.
    Off,
    /// A time lock, and opening was not asked for.
    NotAsked,
    /// Asked for; the lock can be opened from `until` (clock and blocks).
    Waiting {
        /// When the lock can be opened, by the clock and the blocks.
        until: i64,
    },
    /// The wait is over; the lock can be opened until `until`.
    Ready {
        /// When the request lapses.
        until: i64,
    },
}

/// The phase of a time lock of `hours`, with opening asked for or not.
pub fn phase(hours: u32, request: Option<Request>, now: i64, tip: u32) -> Phase {
    if hours == 0 {
        return Phase::Off;
    }
    let Some(r) = request else { return Phase::NotAsked };
    // A request that was never used lapses after its window.
    if now > r.ready_at + WINDOW {
        return Phase::NotAsked;
    }
    let wait = r.ready_at - r.asked_at;
    if passed(wait, r.asked_at, r.asked_height, now, tip) {
        return Phase::Ready { until: r.ready_at + WINDOW };
    }
    // Not yet: by the clock, or the blocks still have to come.
    Phase::Waiting { until: r.ready_at.max(now + blocks_left(wait, r.asked_height, tip)) }
}

/// A shorter time lock (or off) waiting in `policy` has passed its wait.
/// `legacy_height` is the first height seen for a change chosen before the
/// lock recorded heights.
pub fn pending_due(policy: &Policy, now: i64, tip: u32, legacy_height: Option<u32>) -> bool {
    let (Some(_), Some(from)) = (policy.pending_hours, policy.pending_from) else { return false };
    let asked = policy.pending_asked_at.unwrap_or(from - policy.time_lock_hours as i64 * HOUR);
    let height = policy.pending_height.or(legacy_height);
    passed(from - asked, asked, height, now, tip)
}

/// The time lock in force now: a shorter one takes over once its wait passed.
pub fn effective(policy: &Policy, now: i64, tip: u32, legacy_height: Option<u32>) -> u32 {
    match policy.pending_hours {
        Some(hours) if pending_due(policy, now, tip, legacy_height) => hours,
        _ => policy.time_lock_hours,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit 22 Nov 2026: the time lock used only the clock.
    #[test]
    fn moving_the_clock_forward_alone_does_not_open_the_time_lock() {
        let wait = 24 * HOUR;
        let asked = 1_795_300_000;
        let height = 3_600_000;
        let blocks24h = (wait / 25) as u32; // 3,456 blocks
        assert!(!passed(wait, asked, Some(height), asked + wait + 60, height + 10), "clock a day ahead, 10 blocks");
        assert!(!passed(wait, asked, Some(height), asked + 3600, height + blocks24h), "blocks there, clock not");
        assert!(passed(wait, asked, Some(height), asked + wait, height + blocks24h), "both");
        assert!(!passed(wait, asked, None, asked + wait * 2, height), "height not known");
        assert!(!passed(wait, asked, Some(height), asked + wait * 2, 0), "tip not known");
        assert!((blocks_left(wait, Some(height), height + blocks24h / 2) - wait / 2).abs() <= 25);
        assert_eq!(blocks_left(wait, Some(height), height + blocks24h + 5), 0);
    }

    #[test]
    fn the_phases_count_blocks_too() {
        let t = 1_800_000_000;
        let h = 3_600_000;
        let r = |asked_at: i64, ready_at: i64| Some(Request { asked_at, asked_height: Some(h), ready_at });
        assert_eq!(phase(0, None, t, h), Phase::Off);
        assert_eq!(phase(72, None, t, h), Phase::NotAsked);
        // Clock and blocks both done.
        let wait = 72 * HOUR;
        let blocks = (wait / 25) as u32;
        assert_eq!(phase(72, r(t - wait, t), t, h + blocks), Phase::Ready { until: t + WINDOW });
        // Clock done, blocks not: still waiting, until the blocks can be there.
        assert_eq!(phase(72, r(t - wait, t), t, h + blocks - 100), Phase::Waiting { until: t + 100 * 25 });
        // An unused request lapses.
        assert_eq!(phase(72, r(t - wait - WINDOW - 11, t - WINDOW - 11), t, h + blocks * 2), Phase::NotAsked);
        // No height when asked: never ready.
        let no_height = Some(Request { asked_at: t - wait, asked_height: None, ready_at: t });
        assert!(matches!(phase(72, no_height, t, h), Phase::Waiting { .. }));
    }

    #[test]
    fn a_shorter_time_lock_waits_by_clock_and_blocks() {
        let t = 1_800_000_000;
        let h = 3_600_000;
        let week = 168 * HOUR;
        let policy = Policy {
            time_lock_hours: 168,
            pending_hours: Some(0),
            pending_from: Some(t + week),
            pending_asked_at: Some(t),
            pending_height: Some(h),
            ..Default::default()
        };
        let blocks = (week / 25) as u32;
        assert_eq!(effective(&policy, t, h, None), 168);
        assert_eq!(effective(&policy, t + week, h + 100, None), 168, "clock moved, blocks did not");
        assert_eq!(effective(&policy, t + week, h + blocks, None), 0);
        // Chosen before heights were recorded: counted from the first height seen.
        let old = Policy { time_lock_hours: 168, pending_hours: Some(0), pending_from: Some(t + week), ..Default::default() };
        assert_eq!(effective(&old, t + week, h + blocks, None), 168, "no height: not yet");
        assert_eq!(effective(&old, t + week, h + blocks, Some(h)), 0);
    }

    #[test]
    fn block_seconds_follow_the_upgrades() {
        assert_eq!(block_seconds(3_400_000, 3_402_304), 48 * HOUR);
        assert_eq!(block_seconds(3_600_000, 3_606_912), 48 * HOUR);
        assert_eq!(block_seconds(3_543_400, 3_543_410), 4 * 75 + 6 * 25);
    }
}
