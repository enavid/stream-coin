use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub order_id: String,
    pub side: String,
    pub quantity: u64,
    pub fill_price: u64,
    pub strategy_id: String,
    pub candle_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSignalRecord {
    pub signal_id: String,
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestResult {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub interval: String,
    pub candle_count: usize,
    pub signal_count: usize,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub trade_log: Vec<TradeRecord>,
    pub signal_log: Vec<BacktestSignalRecord>,
}
