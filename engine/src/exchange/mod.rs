pub mod coinex;
pub mod entity;
pub mod historical_port;
pub mod hitobit;
pub mod port;
pub mod registry;
pub mod tabdeal;

use chrono::{DateTime, Utc};

/// Resolves a price tick's timestamp from the exchange's event-time (M2).
///
/// Every adapter previously stamped `Utc::now()` (receive time), which buckets
/// ticks into candles by *when the engine read the frame*, not when the
/// exchange produced it — distorting candle boundaries and cross-exchange
/// spread timing under any network/scheduling jitter. We use the exchange's own
/// event-time in epoch-milliseconds when present, falling back to receive time
/// (with a warning) only when the field is absent or implausible.
pub(crate) fn event_time_or_now(event_millis: Option<i64>, exchange: &str) -> DateTime<Utc> {
    match event_millis.and_then(DateTime::<Utc>::from_timestamp_millis) {
        Some(ts) => ts,
        None => {
            tracing::warn!(
                exchange,
                event_millis,
                "price tick missing a valid exchange event-time; falling back to receive time"
            );
            Utc::now()
        }
    }
}

/// Truncates a raw WS frame so a malformed-frame log line stays bounded (M13).
/// Truncates on a char boundary to keep the output valid UTF-8.
pub(crate) fn truncate_for_log(s: &str) -> String {
    const MAX_CHARS: usize = 256;
    if s.chars().count() <= MAX_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

/// Parses an exchange-supplied numeric string (price, amount, or volume) into
/// non-negative minor units (`u64`).
///
/// Exchanges send these as JSON strings; we never parse them as `f64` (the
/// financial-precision rule). Any fractional part is truncated — IRR/IRT prices
/// are integers in practice, and minor-unit volumes are whole. A leading `-`
/// is rejected outright rather than silently truncated to `0`.
///
/// This is the single shared parser for every adapter (tabdeal, hitobit, coinex
/// WS depth frames and the coinex historical kline adapter); previously each had
/// its own byte-identical copy with subtly different error strings (Q-duplication).
pub(crate) fn parse_minor_units(s: &str) -> Result<u64, String> {
    if s.starts_with('-') {
        return Err(format!("value must be non-negative: {s}"));
    }
    let integer_part = s.split_once('.').map_or(s, |(int, _)| int);
    integer_part
        .parse::<u64>()
        .map_err(|_| format!("invalid numeric value: {s}"))
}

/// Runs `f` with a temporary tracing subscriber and returns everything it
/// logged, so a test can assert that a code path actually emitted a log line.
#[cfg(test)]
pub(crate) fn capture_logs(f: impl FnOnce()) -> String {
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct BufWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl tracing_subscriber::fmt::MakeWriter<'_> for BufWriter {
        type Writer = BufWriter;
        fn make_writer(&self) -> BufWriter {
            self.clone()
        }
    }

    let buf = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(BufWriter(buf.clone()))
        .with_max_level(tracing::Level::TRACE)
        .finish();
    tracing::subscriber::with_default(subscriber, f);
    let bytes = buf.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_for_log_keeps_short_strings_intact() {
        assert_eq!(truncate_for_log("short"), "short");
    }

    #[test]
    fn truncate_for_log_bounds_long_strings_on_char_boundary() {
        let long = "x".repeat(1000);
        let out = truncate_for_log(&long);
        assert!(out.chars().count() <= 257, "256 chars + ellipsis");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn parse_minor_units_parses_a_plain_integer() {
        assert_eq!(parse_minor_units("42").unwrap(), 42);
    }

    #[test]
    fn parse_minor_units_truncates_the_fractional_part() {
        assert_eq!(parse_minor_units("100.999").unwrap(), 100);
        assert_eq!(parse_minor_units("0.5").unwrap(), 0);
    }

    #[test]
    fn parse_minor_units_rejects_a_negative_value() {
        let err = parse_minor_units("-5").unwrap_err();
        assert!(err.contains("non-negative"), "got: {err}");
    }

    #[test]
    fn parse_minor_units_rejects_non_numeric_input() {
        assert!(parse_minor_units("abc").is_err());
        assert!(parse_minor_units("").is_err());
        // A non-negative sentinel that overflows u64 must error, not wrap.
        assert!(parse_minor_units("99999999999999999999999999").is_err());
    }

    #[test]
    fn parse_minor_units_accepts_a_large_in_range_value() {
        assert_eq!(parse_minor_units("18446744073709551615").unwrap(), u64::MAX);
    }

    #[test]
    fn event_time_uses_exchange_millis_when_present() {
        let ts = event_time_or_now(Some(1_657_530_675_579), "tabdeal");
        assert_eq!(ts.timestamp_millis(), 1_657_530_675_579);
    }

    #[test]
    fn event_time_falls_back_to_now_when_absent() {
        let before = Utc::now();
        let ts = event_time_or_now(None, "tabdeal");
        assert!(ts >= before, "fallback must be a sane current timestamp");
    }

    #[test]
    fn event_time_falls_back_when_millis_are_implausible() {
        // i64::MAX milliseconds overflows the representable range → None → now().
        let before = Utc::now();
        let ts = event_time_or_now(Some(i64::MAX), "coinex");
        assert!(ts >= before);
    }
}
