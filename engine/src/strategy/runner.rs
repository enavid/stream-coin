use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::candle::entity::{Candle, CandlePayload, Interval};
use crate::exchange::entity::ExchangeId;
use crate::infrastructure::db::signal_repository::{SignalRecord, SignalRepository};
use crate::kafka::port::MessagePublisher;
use crate::price::entity::{Price, TradingPair};
use crate::strategy::entity::Signal;
use crate::strategy::port::Strategy;
use crate::wire_message::{PricePayload, SignalPayload, WsMessage};

fn price_from_payload(p: &PricePayload) -> Price {
    let pair: TradingPair = serde_json::from_value(serde_json::Value::String(p.pair.clone()))
        .unwrap_or_else(|_| {
            tracing::warn!(raw_pair = %p.pair, "unparseable pair in broadcaster message");
            TradingPair::new("UNKNOWN", "UNKNOWN")
        });
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
    signal_repository: Option<Arc<dyn SignalRepository>>,
) -> AbortHandle {
    let mut rx = broadcaster.subscribe();
    let signals_topic =
        std::env::var("KAFKA_TOPIC_SIGNALS").unwrap_or_else(|_| "signals".to_string());

    let handle = tokio::spawn(async move {
        loop {
            let text = match rx.recv().await {
                Ok(t) => t,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        dropped = n,
                        strategy_id = %strategy.strategy_id(),
                        "strategy runner lagged, messages dropped"
                    );
                    continue;
                }
                Err(_) => break,
            };

            let msg: WsMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let signal: Option<Signal> = match &msg {
                WsMessage::PriceUpdate(p) => {
                    tracing::trace!(
                        exchange = %p.exchange,
                        pair = %p.pair,
                        strategy_id = %strategy.strategy_id(),
                        "dispatching price to strategy"
                    );
                    strategy.on_price(&price_from_payload(p))
                }
                WsMessage::Candle(c) => {
                    tracing::trace!(
                        exchange = %c.exchange,
                        pair = %c.pair,
                        strategy_id = %strategy.strategy_id(),
                        "dispatching candle to strategy"
                    );
                    strategy.on_candle(&candle_from_payload(c))
                }
                WsMessage::Signal(_) => None,
            };

            if let Some(sig) = signal {
                let signal_id = uuid::Uuid::new_v4().to_string();

                tracing::info!(
                    signal_id = %signal_id,
                    strategy_id = %sig.strategy_id,
                    exchange = %sig.exchange,
                    pair = %sig.pair,
                    action = %sig.action.as_str(),
                    confidence = %sig.confidence,
                    timestamp = %sig.timestamp,
                    "signal emitted"
                );

                let payload = SignalPayload {
                    signal_id: signal_id.clone(),
                    strategy_id: sig.strategy_id.clone(),
                    exchange: sig.exchange.clone(),
                    pair: sig.pair.clone(),
                    action: sig.action.as_str().to_string(),
                    confidence: sig.confidence,
                    timestamp: sig.timestamp,
                };

                match serde_json::to_string(&WsMessage::Signal(payload)) {
                    Ok(json) => {
                        let _ = broadcaster.send(json.clone());
                        if let Some(ref pub_) = publisher {
                            if let Err(e) =
                                pub_.publish(&signals_topic, &sig.strategy_id, &json).await
                            {
                                tracing::error!(
                                    error = %e,
                                    signal_id = %signal_id,
                                    "failed to publish signal to kafka"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            strategy_id = %sig.strategy_id,
                            "failed to serialize signal"
                        );
                        continue;
                    }
                }

                if let Some(ref repo) = signal_repository {
                    let record = SignalRecord {
                        signal_id: signal_id.clone(),
                        strategy_id: sig.strategy_id.clone(),
                        exchange: sig.exchange.clone(),
                        pair: sig.pair.clone(),
                        action: sig.action.as_str().to_string(),
                        confidence: sig.confidence,
                        created_at: sig.timestamp,
                    };
                    if let Err(e) = repo.save(&record).await {
                        tracing::error!(
                            error = %e,
                            signal_id = %signal_id,
                            "failed to persist signal to db"
                        );
                    }
                }
            }
        }
    });

    handle.abort_handle()
}
