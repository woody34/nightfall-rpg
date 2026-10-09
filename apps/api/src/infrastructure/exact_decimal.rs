//! Shared exact decimal parser for rule tables and NPC templates.

use crate::domain::zone::Scaled;

/// Parses a plain decimal (`-?digits(.digits)?`) to exact `1/Q` units. Rejects exponents,
/// signs other than a leading `-`, whitespace, more than six significant fractional digits
/// and values that overflow `i64`. No floating point is involved.
pub(super) fn parse_decimal(s: &str) -> Result<Scaled, String> {
    let (negative, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    if !digits(int_part) || frac_part.is_some_and(|f| !digits(f)) {
        return Err(format!("{s:?} is not a plain decimal"));
    }
    let frac = frac_part.unwrap_or("").trim_end_matches('0');
    if frac.len() > 6 {
        return Err(format!("{s:?} has more than 6 fractional digits (not exact at Q)"));
    }
    let overflow = || format!("{s:?} overflows the scaled range");
    let mut raw: i64 = 0;
    for b in int_part.bytes().chain(frac.bytes()) {
        raw = raw
            .checked_mul(10)
            .and_then(|r| r.checked_sub(i64::from(b.saturating_sub(b'0'))))
            .ok_or_else(overflow)?;
    }
    let pad = 6_u32.saturating_sub(u32::try_from(frac.len()).unwrap_or(6));
    raw = 10_i64
        .checked_pow(pad)
        .and_then(|scale| raw.checked_mul(scale))
        .ok_or_else(overflow)?;
    if !negative {
        raw = raw.checked_neg().ok_or_else(overflow)?;
    }
    Ok(Scaled::from_raw(raw))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn decimals_are_exact_and_bounded() {
        for bad in [
            "1.2e3",
            "1,2",
            "0.1234567",
            " 1.0",
            "+1",
            "",
            ".5",
            "5.",
            "--1",
            "1.0.0",
        ] {
            assert!(parse_decimal(bad).is_err(), "{bad:?} parsed");
        }
        assert_eq!(parse_decimal("0.1234560").unwrap().raw(), 123_456);
        assert_eq!(parse_decimal("-1.5").unwrap().raw(), -1_500_000);
        assert_eq!(parse_decimal("9223372036854").unwrap().raw(), 9_223_372_036_854_000_000);
        assert!(parse_decimal("9223372036855").is_err());
        assert_eq!(parse_decimal("0.000001").unwrap().raw(), 1);
        assert!(parse_decimal("1.0000001").is_err());
        assert!(parse_decimal("1e3").is_err());
        assert_eq!(parse_decimal("9223372036854.775807").unwrap().raw(), i64::MAX);
        assert_eq!(parse_decimal("-9223372036854.775808").unwrap().raw(), i64::MIN);
        assert!(parse_decimal("9223372036854.775808").is_err());
        assert!(parse_decimal("-9223372036854.775809").is_err());
    }
}
