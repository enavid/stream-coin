use chrono::{DateTime, TimeZone, Utc};

use crate::candle::entity::{Candle, Interval};
use crate::price::entity::Price;

pub struct CandleAggregator {
    exchange: String,
    pair: String,
    interval: Interval,
    current: Option<CandleState>,
}

struct CandleState {
    open_time: DateTime<Utc>,
    open: u64,
    high: u64,
    low: u64,
    close: u64,
}

fn candle_open_time(t: DateTime<Utc>, interval: Interval) -> DateTime<Utc> {
    let secs = t.timestamp();
    let window = interval.as_secs() as i64;
    let open_secs = secs - (secs % window);
    Utc.timestamp_opt(open_secs, 0).single().unwrap_or(t)
}

impl CandleAggregator {
    pub fn new(exchange: String, pair: String, interval: Interval) -> Self {
        Self {
            exchange,
            pair,
            interval,
            current: None,
        }
    }

    /// Feeds a price tick. Returns a closed `Candle` when the tick's timestamp
    /// falls outside the current window, otherwise returns `None`.
    pub fn push(&mut self, price: &Price) -> Option<Candle> {
        let open_time = candle_open_time(price.timestamp, self.interval);
        let mid = (price.bid + price.ask) / 2;

        match &mut self.current {
            None => {
                self.current = Some(CandleState {
                    open_time,
                    open: mid,
                    high: mid,
                    low: mid,
                    close: mid,
                });
                None
            }
            Some(state) if state.open_time == open_time => {
                if mid > state.high {
                    state.high = mid;
                }
                if mid < state.low {
                    state.low = mid;
                }
                state.close = mid;
                None
            }
            Some(_) => {
                let state = self.current.take().unwrap();
                let closed = Candle {
                    exchange: self.exchange.clone(),
                    pair: self.pair.clone(),
                    interval: self.interval,
                    time: state.open_time,
                    open: state.open,
                    high: state.high,
                    low: state.low,
                    close: state.close,
                    volume: 0,
                };
                self.current = Some(CandleState {
                    open_time,
                    open: mid,
                    high: mid,
                    low: mid,
                    close: mid,
                });
                Some(closed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::entity::Interval;
    use crate::exchange::entity::ExchangeId;
    use crate::price::entity::{Price, TradingPair};

    fn make_price(bid: u64, ask: u64, ts: &str) -> Price {
        Price {
            exchange: ExchangeId::new("tabdeal"),
            pair: TradingPair::new("USDT", "IRT"),
            bid,
            ask,
            timestamp: ts.parse::<DateTime<Utc>>().unwrap(),
        }
    }

    #[test]
    fn aggregator_opens_new_candle_on_first_tick() {
        let mut agg =
            CandleAggregator::new("tabdeal".into(), "USDT/IRT".into(), Interval::OneMinute);
        let result = agg.push(&make_price(58000, 58100, "2026-06-20T10:00:30Z"));
        assert!(
            result.is_none(),
            "first tick opens a candle but must not close one"
        );
    }

    #[test]
    fn aggregator_tracks_high_and_low() {
        let mut agg =
            CandleAggregator::new("tabdeal".into(), "USDT/IRT".into(), Interval::OneMinute);
        // All in the 10:00 window; mid = (bid + ask) / 2
        agg.push(&make_price(100, 100, "2026-06-20T10:00:10Z")); // mid 100
        agg.push(&make_price(150, 150, "2026-06-20T10:00:20Z")); // mid 150 → new high
        agg.push(&make_price(80, 80, "2026-06-20T10:00:50Z")); // mid 80 → new low

        // 10:01 window closes the 10:00 candle
        let candle = agg
            .push(&make_price(120, 120, "2026-06-20T10:01:05Z"))
            .unwrap();
        assert_eq!(
            candle.high, 150,
            "must track the highest mid price in the window"
        );
        assert_eq!(
            candle.low, 80,
            "must track the lowest mid price in the window"
        );
    }

    #[test]
    fn aggregator_closes_candle_on_interval_boundary() {
        let mut agg =
            CandleAggregator::new("tabdeal".into(), "USDT/IRT".into(), Interval::OneMinute);
        agg.push(&make_price(58000, 58100, "2026-06-20T10:00:30Z")); // window 10:00, mid 58050

        // Price in 10:01 window closes the 10:00 candle
        let result = agg.push(&make_price(59000, 59100, "2026-06-20T10:01:00Z"));
        assert!(
            result.is_some(),
            "price in new window must close the previous candle"
        );

        let candle = result.unwrap();
        assert_eq!(candle.exchange, "tabdeal");
        assert_eq!(candle.pair, "USDT/IRT");
        assert_eq!(candle.open, 58050);
        assert_eq!(candle.close, 58050);
    }

    #[test]
    fn aggregator_open_equals_first_tick_mid_and_close_equals_last_tick_mid() {
        let mut agg =
            CandleAggregator::new("tabdeal".into(), "USDT/IRT".into(), Interval::OneMinute);
        agg.push(&make_price(100, 100, "2026-06-20T10:00:05Z")); // open mid 100
        agg.push(&make_price(200, 200, "2026-06-20T10:00:30Z")); // close mid 200

        let candle = agg
            .push(&make_price(150, 150, "2026-06-20T10:01:00Z"))
            .unwrap();
        assert_eq!(candle.open, 100, "open must be the mid of the first tick");
        assert_eq!(candle.close, 200, "close must be the mid of the last tick");
    }

    #[test]
    fn aggregator_five_minute_interval_groups_ticks_correctly() {
        let mut agg =
            CandleAggregator::new("tabdeal".into(), "USDT/IRT".into(), Interval::FiveMinutes);
        // 10:00 and 10:04 are in the same 5m window [10:00, 10:05)
        agg.push(&make_price(100, 100, "2026-06-20T10:00:00Z"));
        let same_window = agg.push(&make_price(200, 200, "2026-06-20T10:04:59Z"));
        assert!(
            same_window.is_none(),
            "10:04:59 is still in the 10:00 5m window"
        );

        // 10:05 starts a new 5m window
        let new_window = agg.push(&make_price(300, 300, "2026-06-20T10:05:00Z"));
        assert!(
            new_window.is_some(),
            "10:05:00 must close the [10:00, 10:05) candle"
        );
    }
}
