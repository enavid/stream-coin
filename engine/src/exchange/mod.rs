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

#[cfg(test)]
mod tests {
    use super::*;

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
