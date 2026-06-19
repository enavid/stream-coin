//! Pure number formatting for prices and spreads. Kept free of any
//! Dioxus/WASM dependency so it can be unit tested on the host target.

/// Formats a price for display: thousands separators below 1e6, then
/// `M`/`B` suffixes for larger magnitudes. Always rounds to the nearest
/// integer first — upstream prices may arrive as floats.
pub fn format_price(value: f64) -> String {
    let rounded = value.round();
    let abs = rounded.abs();

    if abs >= 1e9 {
        format!("{:.3}B", rounded / 1e9)
    } else if abs >= 1e6 {
        format!("{:.2}M", rounded / 1e6)
    } else {
        format_with_thousands(rounded as i64)
    }
}

/// Formats a spread (always non-negative in practice, but defensively
/// handled): `K`/`M` suffixes kick in earlier than [`format_price`] since
/// spreads are typically much smaller than the prices they're derived from.
pub fn format_spread(value: f64) -> String {
    let rounded = value.round();
    let abs = rounded.abs();

    if abs >= 1e6 {
        format!("{:.1}M", rounded / 1e6)
    } else if abs >= 1e3 {
        format!("{:.1}K", rounded / 1e3)
    } else {
        format!("{rounded}")
    }
}

/// Pulls the `HH:MM:SS` clock time out of an RFC3339 timestamp
/// (`2026-06-18T10:26:33.123Z`) for display in the live feed. Falls back
/// to the raw input if it doesn't contain a `T` separator, so malformed
/// timestamps degrade gracefully instead of panicking.
pub fn extract_time(timestamp: &str) -> &str {
    match timestamp.split_once('T') {
        Some((_, rest)) => rest.split(['.', 'Z']).next().unwrap_or(rest),
        None => timestamp,
    }
}

fn format_with_thousands(n: i64) -> String {
    let negative = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }

    if negative {
        format!("-{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_price_adds_thousands_separators_below_a_million() {
        assert_eq!(format_price(92815.0), "92,815");
    }

    #[test]
    fn format_price_rounds_fractional_values() {
        assert_eq!(format_price(92797.742), "92,798");
    }

    #[test]
    fn format_price_uses_million_suffix_at_or_above_1e6() {
        assert_eq!(format_price(1_500_000.0), "1.50M");
    }

    #[test]
    fn format_price_uses_billion_suffix_at_or_above_1e9() {
        assert_eq!(format_price(4_218_500_000.0), "4.218B");
    }

    #[test]
    fn format_price_billion_suffix_rounds_up_when_unambiguous() {
        assert_eq!(format_price(4_221_900_000.0), "4.222B");
    }

    #[test]
    fn format_price_handles_small_values_without_separators() {
        assert_eq!(format_price(815.0), "815");
    }

    #[test]
    fn format_price_handles_zero() {
        assert_eq!(format_price(0.0), "0");
    }

    #[test]
    fn format_spread_returns_plain_integer_below_a_thousand() {
        assert_eq!(format_spread(121.0), "121");
    }

    #[test]
    fn format_spread_uses_k_suffix_at_or_above_1e3() {
        assert_eq!(format_spread(2500.0), "2.5K");
    }

    #[test]
    fn format_spread_uses_m_suffix_at_or_above_1e6() {
        assert_eq!(format_spread(2_500_000.0), "2.5M");
    }

    #[test]
    fn format_spread_rounds_fractional_values() {
        // Regression: spreads derived from float subtraction must never
        // leak long decimal tails into the UI.
        assert_eq!(format_spread(113.815289576), "114");
    }

    #[test]
    fn extract_time_returns_the_clock_portion_of_an_rfc3339_timestamp() {
        assert_eq!(extract_time("2026-06-18T10:26:33.123Z"), "10:26:33");
    }

    #[test]
    fn extract_time_handles_timestamps_without_fractional_seconds() {
        assert_eq!(extract_time("2026-06-18T10:26:33Z"), "10:26:33");
    }

    #[test]
    fn extract_time_falls_back_to_raw_input_when_there_is_no_t_separator() {
        assert_eq!(extract_time("not-a-timestamp"), "not-a-timestamp");
    }

    // --- boundary / edge value tests ---

    #[test]
    fn format_price_negative_value_shows_minus_prefix() {
        assert_eq!(format_price(-92_815.0), "-92,815");
    }

    #[test]
    fn format_price_negative_million_uses_m_suffix() {
        assert_eq!(format_price(-1_500_000.0), "-1.50M");
    }

    #[test]
    fn format_price_sub_unit_value_rounds_to_nearest_integer() {
        assert_eq!(format_price(0.3), "0");
        assert_eq!(format_price(0.5), "1");
        assert_eq!(format_price(0.9), "1");
    }

    #[test]
    fn format_price_exactly_one_million_uses_m_suffix() {
        assert_eq!(format_price(1_000_000.0), "1.00M");
    }

    #[test]
    fn format_price_exactly_one_billion_uses_b_suffix() {
        assert_eq!(format_price(1_000_000_000.0), "1.000B");
    }

    #[test]
    fn format_spread_zero_returns_zero_string() {
        assert_eq!(format_spread(0.0), "0");
    }

    #[test]
    fn format_spread_negative_value_below_threshold_shows_plain_number() {
        assert_eq!(format_spread(-500.0), "-500");
    }
}
