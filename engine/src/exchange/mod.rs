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
