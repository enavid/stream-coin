use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Buy,
    Sell,
    Hold,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Buy => "buy",
            Action::Sell => "sell",
            Action::Hold => "hold",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Signal {
    pub strategy_id: String,
    pub exchange: String,
    pub pair: String,
    pub action: Action,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    /// No built-in strategy computes a stop-loss yet — always `None` today.
    pub stop_loss: Option<u64>,
    /// No built-in strategy computes a take-profit yet — always `None` today.
    pub take_profit: Option<u64>,
}
