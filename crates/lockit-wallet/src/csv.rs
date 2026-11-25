//! A wallet's payments as a CSV file for bookkeeping (Lock it Mac's
//! ActivityCSV): one line per payment, amounts in ZEC with 8 decimals and a
//! dot, local time with its offset. Anyone can send a memo, so a field that a
//! spreadsheet would read as a formula gets a leading ' and every field is
//! quoted the standard way (RFC 4180).

use crate::Activity;

/// The first line of the file.
pub const HEADER: &str = "date,direction,description,amount_zec,fee_zec,status,block,memo,txid";

/// One line of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// When its block was made (Unix seconds); none while waiting.
    pub time: Option<i64>,
    /// Money in (or out).
    pub received: bool,
    /// The Activity wording.
    pub description: String,
    /// What was received, or what the receiver got: a payment's fee is only
    /// in `fee`, so amount + fee is what left the wallet.
    pub amount: u64,
    /// The network fee, for a payment out.
    pub fee: Option<u64>,
    /// confirmed, waiting or expired.
    pub status: &'static str,
    /// The block it is in.
    pub block: Option<u32>,
    /// The payment's text memo.
    pub memo: Option<String>,
    /// Transaction id.
    pub txid: String,
}

/// One Activity line as a row. Windows' change for a payment out includes
/// the fee; here it is taken out, so a TEX forwarding step (only its fee
/// leaves) is 0 with its fee, and nothing is counted twice.
pub fn row(item: &Activity, memo: Option<String>) -> Row {
    let received = item.change > 0;
    let status = if item.expired {
        "expired"
    } else if item.height.is_some() {
        "confirmed"
    } else {
        "waiting"
    };
    let fee = if received { None } else { item.fee };
    let amount = if received { item.change.unsigned_abs() } else { item.change.unsigned_abs().saturating_sub(fee.unwrap_or(0)) };
    // Only the fee left and it was no TEX step: a move inside the wallet
    // (locking in, the Ironwood move, a payment to an own address).
    let description = if !received && amount == 0 && item.tex_step.is_none() { "Moved inside the wallet · fee".to_owned() } else { item.title.clone() };
    Row { time: item.time, received, description, amount, fee, status, block: item.height, memo, txid: item.txid.clone() }
}

/// 8 decimals, no rounding: 123456789 zatoshi is 1.23456789.
pub fn zec(zatoshi: u64) -> String {
    format!("{}.{:08}", zatoshi / 100_000_000, zatoshi % 100_000_000)
}

/// One field: formula guard first, then quoted when needed.
pub fn field(raw: &str) -> String {
    let mut text = raw.to_owned();
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        text.insert(0, '\'');
    }
    let needs_quotes = text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ');
    if needs_quotes {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// Days since 1970-01-01 as (year, month, day) (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// Local time with its offset: "2026-11-25 07:49:12 +01:00". `offset`:
/// minutes east of UTC at that moment (summer time included).
pub fn date(time: Option<i64>, offset: i32) -> String {
    let Some(time) = time else { return String::new() };
    let local = time + i64::from(offset) * 60;
    let (year, month, day) = civil(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    let sign = if offset < 0 { '-' } else { '+' };
    let off = offset.unsigned_abs();
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} {sign}{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60,
        off / 60,
        off % 60
    )
}

/// One line of the file.
pub fn line(row: &Row, offset: i32) -> String {
    [
        date(row.time, offset),
        (if row.received { "received" } else { "sent" }).to_owned(),
        field(&row.description),
        zec(row.amount),
        row.fee.map(zec).unwrap_or_default(),
        row.status.to_owned(),
        row.block.map(|b| b.to_string()).unwrap_or_default(),
        field(row.memo.as_deref().unwrap_or("")),
        field(&row.txid),
    ]
    .join(",")
}

/// The whole file: UTF-8 with a byte order mark so Excel reads memos with
/// accents or emoji right, lines ending in CRLF. `offset(time)`: minutes
/// east of UTC at that moment.
pub fn file(rows: &[Row], offset: impl Fn(Option<i64>) -> i32) -> Vec<u8> {
    let mut text = String::from(HEADER);
    text.push_str("\r\n");
    for row in rows {
        text.push_str(&line(row, offset(row.time)));
        text.push_str("\r\n");
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// zecky-shop-2026-11-25.csv: the wallet name in plain letters; `today`
/// as year, month, day.
pub fn file_name(wallet: &str, today: (i64, u32, u32)) -> String {
    let plain: String = wallet.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = plain.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    let slug = if slug.is_empty() { "wallet".to_owned() } else { slug };
    format!("zecky-{slug}-{:04}-{:02}-{:02}.csv", today.0, today.1, today.2)
}

/// Today's date at this offset (minutes east of UTC).
pub fn today(now: i64, offset: i32) -> (i64, u32, u32) {
    civil((now + i64::from(offset) * 60).div_euclid(86_400))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(change: i64, fee: Option<u64>, height: Option<u32>, tex_step: Option<u8>, title: &str) -> Activity {
        Activity {
            txid: "ab12".into(),
            height,
            change,
            time: Some(1_795_589_352),
            expired: false,
            fee,
            tex_receiver: tex_step.map(|_| "tex1x".into()),
            tex_step,
            title: title.into(),
        }
    }

    /// Lock it Mac's testActivityCSV, line by line.
    #[test]
    fn the_mac_activity_csv_cases() {
        assert_eq!(zec(123_456_789), "1.23456789");
        assert_eq!(zec(10_000), "0.00010000");
        assert_eq!(zec(2_100_000_000_000_000), "21000000.00000000");
        assert_eq!(zec(0), "0.00000000");
        assert_eq!(field("Order 42"), "Order 42");
        assert_eq!(field("a, b"), "\"a, b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(field("two\nlines"), "\"two\nlines\"");
        assert_eq!(field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        for bad in ["+1", "-1", "@SUM(A1)", "\tx"] {
            assert!(field(bad).contains('\''), "{bad}");
        }
        // 2026-11-25 06:49:12 UTC, Amsterdam in winter: +01:00.
        let when = Some(1_795_589_352);
        assert_eq!(date(when, 60), "2026-11-25 07:49:12 +01:00");
        assert_eq!(date(when, -300), "2026-11-25 01:49:12 -05:00");
        assert_eq!(date(Some(0), 0), "1970-01-01 00:00:00 +00:00");
        assert_eq!(date(Some(951_782_400), 0), "2000-02-29 00:00:00 +00:00", "a leap day");
        let sent = Row {
            time: when, received: false, description: "Sent".into(), amount: 50_000_000, fee: Some(10_000), status: "confirmed",
            block: Some(3_609_479), memo: Some("Coffee, 2x".into()), txid: "ab12".into(),
        };
        assert_eq!(line(&sent, 60), "2026-11-25 07:49:12 +01:00,sent,Sent,0.50000000,0.00010000,confirmed,3609479,\"Coffee, 2x\",ab12");
        let waiting = Row { time: None, received: true, description: "Received".into(), amount: 1, fee: None, status: "waiting", block: None, memo: None, txid: "cd".into() };
        assert_eq!(line(&waiting, 60), ",received,Received,0.00000001,,waiting,,,cd");
        // From Activity: a payment's fee comes out of the amount, once.
        let out = row(&activity(-110_000, Some(10_000), Some(3_597_827), None, "Released"), None);
        assert_eq!((out.amount, out.fee, out.description.as_str(), out.status), (100_000, Some(10_000), "Released", "confirmed"));
        let forwarding = row(&activity(-15_000, Some(15_000), Some(3_597_828), Some(2), "TEX payment · forwarding fee"), None);
        assert_eq!((forwarding.amount, forwarding.fee), (0, Some(15_000)), "only the fee left");
        assert_eq!(forwarding.description, "TEX payment · forwarding fee");
        let got = row(&activity(180_000, None, None, None, "Locked"), Some("=1+1".into()));
        assert_eq!((got.amount, got.fee, got.status), (180_000, None, "waiting"));
        assert!(line(&got, 60).contains(",'=1+1,"), "a memo never runs as a formula");
        let mut gone = activity(-20_000, Some(10_000), None, None, "Released");
        gone.expired = true;
        assert_eq!(row(&gone, None).status, "expired");
        // A move inside the wallet: only the fee left.
        let moved = row(&activity(-15_000, Some(15_000), Some(3_597_830), None, "Released"), None);
        assert_eq!(moved.description, "Moved inside the wallet · fee");
        // The file: byte order mark, header, CRLF.
        let bytes = file(&[sent], |_| 60);
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        assert!(text.starts_with(&format!("{HEADER}\r\n")));
        assert!(text.ends_with("\r\n") && !text.contains("\r\n\r\n"));
        assert_eq!(file_name("My Shop · Café", today(1_795_589_352, 60)), "zecky-my-shop-caf-2026-11-25.csv");
        assert_eq!(file_name("", today(1_795_589_352, 60)), "zecky-wallet-2026-11-25.csv");
    }
}
