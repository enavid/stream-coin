use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Interval {
    #[serde(rename = "1m")]
    OneMinute,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "1h")]
    OneHour,
}

impl Interval {
    pub fn as_secs(self) -> u64 {
        match self {
            Interval::OneMinute => 60,
            Interval::FiveMinutes => 5 * 60,
            Interval::FifteenMinutes => 15 * 60,
            Interval::OneHour => 60 * 60,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Interval::OneMinute => "1m",
            Interval::FiveMinutes => "5m",
            Interval::FifteenMinutes => "15m",
            Interval::OneHour => "1h",
        }
    }

    pub fn all() -> [Interval; 4] {
        [
            Interval::OneMinute,
            Interval::FiveMinutes,
            Interval::FifteenMinutes,
            Interval::OneHour,
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candle {
    pub exchange: String,
    pub pair: String,
    pub interval: Interval,
    pub time: DateTime<Utc>,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandlePayload {
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub time: DateTime<Utc>,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
    pub volume: u64,
}

impl From<&Candle> for CandlePayload {
    fn from(c: &Candle) -> Self {
        CandlePayload {
            exchange: c.exchange.clone(),
            pair: c.pair.clone(),
            interval: c.interval.as_str().to_string(),
            time: c.time,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
        }
    }
}
