//! Amounts typed by the owner (Lock it Mac's AmountInput, 22 Nov 2026).

/// ZEC typed as "1.5", "1,5" or ".5": only the digits 0-9, at most 8
/// decimals, more than zero. Anything else is refused with a reason, never
/// read as 0: digits of other scripts ("١.5") are not 0-9 (audit, 22 Nov 2026).
pub fn parse_zec(text: &str) -> Result<u64, &'static str> {
    let text = text.trim().replace(',', ".");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let digits_only = whole.chars().chain(fraction.chars()).all(|c| c.is_ascii_digit());
    if whole.is_empty() && fraction.is_empty() || !digits_only {
        return Err("type the amount in ZEC with the digits 0-9, for example 0.25");
    }
    if fraction.len() > 8 {
        return Err("ZEC has at most 8 decimals");
    }
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| "that amount is too large")? };
    let fraction: u64 = format!("{fraction:0<8}").parse().map_err(|_| "type the amount in ZEC with the digits 0-9, for example 0.25")?;
    let zats = whole.checked_mul(100_000_000).and_then(|w| w.checked_add(fraction)).ok_or("that amount is too large")?;
    if zats == 0 {
        return Err("the amount must be more than zero");
    }
    Ok(zats)
}

#[cfg(test)]
mod tests {
    use super::parse_zec;

    /// Audit 22 Nov 2026 (Mac): other scripts' digits were read as 0.
    #[test]
    fn only_the_digits_zero_to_nine_are_read() {
        for text in ["١.5", "١", "1.٥", "١٢,٣٤٥", "1.5e3", "-1", "1.2.3", "", " ", ".", "0", "0.00000000"] {
            assert!(parse_zec(text).is_err(), "{text:?} must be refused");
        }
        assert_eq!(parse_zec("1.5"), Ok(150_000_000));
        assert_eq!(parse_zec("0,001"), Ok(100_000));
        assert_eq!(parse_zec(".5"), Ok(50_000_000));
        assert_eq!(parse_zec(" 0.00000001 "), Ok(1));
        assert_eq!(parse_zec("0.000000001"), Err("ZEC has at most 8 decimals"));
        assert_eq!(parse_zec("99999999999999"), Err("that amount is too large"));
    }
}
