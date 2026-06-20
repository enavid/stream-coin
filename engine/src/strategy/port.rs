use crate::candle::entity::Candle;
use crate::price::entity::Price;
use crate::strategy::entity::Signal;

pub trait Strategy: Send + Sync {
    fn strategy_id(&self) -> &str;
    fn on_candle(&self, _candle: &Candle) -> Option<Signal> {
        None
    }
    fn on_price(&self, _price: &Price) -> Option<Signal> {
        None
    }
}
