use std::sync::Arc;

use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::candle::entity::{Candle, CandlePayload, Interval};
use crate::exchange::entity::ExchangeId;
use crate::kafka::port::MessagePublisher;
use crate::presentation::ws_message::{PricePayload, SignalPayload, WsMessage};
use crate::price::entity::{Price, TradingPair};
use crate::strategy::entity::Signal;
use crate::strategy::port::Strategy;

fn price_from_payload(p: &PricePayload) -> Price {
    let pair: TradingPair = serde_json::from_value(Value::String(p.pair.clone()))
        .unwrap_or_else(|_| TradingPair::new("UNKNOWN", "UNKNOWN"));
    Price {
        exchange: ExchangeId::new(&p.exchange),
        pair,
        bid: p.bid,
        ask: p.ask,
        timestamp: p.timestamp,
    }
}

fn candle_from_payload(p: &CandlePayload) -> Candle {
    let interval = match p.interval.as_str() {
        "5m" => Interval::FiveMinutes,
        "15m" => Interval::FifteenMinutes,
        "1h" => Interval::OneHour,
        _ => Interval::OneMinute,
    };
    Candle {
        exchange: p.exchange.clone(),
        pair: p.pair.clone(),
        interval,
        time: p.time,
        open: p.open,
        high: p.high,
        low: p.low,
        close: p.close,
        volume: p.volume,
    }
}

pub fn spawn_strategy_runner(
    strategy: Arc<dyn Strategy>,
    broadcaster: broadcast::Sender<String>,
    publisher: Option<Arc<dyn MessagePublisher>>,
) -> AbortHandle {
    let mut rx = broadcaster.subscribe();
    let signals_topic =
        std::env::var("KAFKA_TOPIC_SIGNALS").unwrap_or_else(|_| "signals".to_string());

    let handle = tokio::spawn(async move {
        while let Ok(text) = rx.recv().await {
            let msg: WsMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let signal: Option<Signal> = match &msg {
                WsMessage::PriceUpdate(p) => strategy.on_price(&price_from_payload(p)),
                WsMessage::Candle(c) => strategy.on_candle(&candle_from_payload(c)),
                WsMessage::Signal(_) => None,
            };

            if let Some(sig) = signal {
                let payload = SignalPayload::from(&sig);
                match serde_json::to_string(&WsMessage::Signal(payload)) {
                    Ok(json) => {
                        let _ = broadcaster.send(json.clone());
                        if let Some(ref pub_) = publisher {
                            if let Err(e) =
                                pub_.publish(&signals_topic, &sig.strategy_id, &json).await
                            {
                                tracing::error!(error = %e, "failed to publish signal to kafka");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize signal");
                    }
                }
            }
        }
    });

    handle.abort_handle()
}
