//! Where a restored wallet starts looking: the same estimate as Lock it Mac
//! (`Birthday.swift`), from the month and year the words were made.
//!
//! A block every 150 s before Blossom (block 653,600, 11 Dec 2019), 75 s
//! after, and 25 s since NU7 (block 3,543,405, 5 Nov 2026). Real block times
//! drift, and starting too late would miss funds, so it aims a few percent
//! (plus about two days) early.

/// Sapling activation: the earliest block a Zecky wallet can have funds in.
pub const SAPLING_ACTIVATION: u32 = 419_200;

const BLOSSOM_HEIGHT: i64 = 653_600;
const NU7_HEIGHT: i64 = 3_543_405;
/// 11 Dec 2019, 18:00 UTC.
const BLOSSOM_TIME: i64 = 1_576_087_200;
const MARGIN: f64 = 1.03;
const EXTRA_BLOCKS: i64 = 2_000;

/// How many blocks back from `tip` cover `seconds`, counting from Blossom on
/// (75 s, and 25 s from NU7). Rounded up: never too few.
fn blocks_back(seconds: f64, tip: i64) -> i64 {
    if seconds <= 0.0 {
        return 0;
    }
    if tip >= NU7_HEIGHT {
        let since_nu7 = tip - NU7_HEIGHT + 1;
        let after_nu7 = since_nu7 as f64 * 25.0;
        if seconds <= after_nu7 {
            return (seconds / 25.0).ceil() as i64;
        }
        return since_nu7 + ((seconds - after_nu7) / 75.0).ceil() as i64;
    }
    (seconds / 75.0).ceil() as i64
}

/// Days since 1 Jan 1970 for a date (proleptic Gregorian, UTC).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The block to start from for a wallet whose words were made in
/// `year`/`month` (1 to 12), given the chain's tip and the time now (Unix
/// seconds).
pub fn estimate_birthday(year: i32, month: u32, tip: u32, now: i64) -> u32 {
    let tip = tip as i64;
    let target = days_from_civil(year as i64, month.clamp(1, 12) as i64, 1) * 86_400;
    let mut height = if target >= BLOSSOM_TIME {
        let seconds_back = (now - target).max(0) as f64;
        tip - blocks_back(seconds_back * MARGIN, tip)
    } else {
        let seconds_before = (BLOSSOM_TIME - target) as f64;
        BLOSSOM_HEIGHT - (seconds_before / 150.0 * MARGIN).ceil() as i64
    };
    height -= EXTRA_BLOCKS;
    height.max(SAPLING_ACTIVATION as i64).min(tip) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_is_right() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2019, 12, 11) * 86_400 + 18 * 3600, BLOSSOM_TIME);
    }

    #[test]
    fn blocks_back_matches_the_mac() {
        assert_eq!(blocks_back(3600.0, 3_500_000), 48);
        assert_eq!(blocks_back(3600.0, 3_700_000), 144);
        assert_eq!(blocks_back(25_000.0 + 75.0 * 100.0, NU7_HEIGHT + 999), 1000 + 100);
    }

    #[test]
    fn a_restore_from_november_after_nu7_starts_early_enough() {
        // The Mac's test: on 30 Nov 2026, a wallet from November must start
        // before 1 Nov, about 4.5 days of 75-second blocks before NU7.
        let now = days_from_civil(2026, 11, 30) * 86_400;
        let tip = (NU7_HEIGHT + 84_672) as u32;
        assert!(estimate_birthday(2026, 11, tip, now) as i64 <= NU7_HEIGHT - 5_184);
    }

    #[test]
    fn very_old_or_future_dates_stay_in_range() {
        let now = days_from_civil(2026, 11, 13) * 86_400;
        let tip = 3_570_000;
        assert_eq!(estimate_birthday(2016, 10, tip, now), SAPLING_ACTIVATION);
        assert!(estimate_birthday(2030, 1, tip, now) <= tip);
    }
}
