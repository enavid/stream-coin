use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::backtest::entity::ClosedTrade;
use crate::candle::entity::{Candle, CandlePayload, Interval};
use crate::exchange::entity::ExchangeId;
use crate::infrastructure::db::signal_repository::{SignalRecord, SignalRepository};
use crate::kafka::port::MessagePublisher;
use crate::price::entity::{Price, TradingPair};
use crate::strategy::entity::Signal;
use crate::strategy::live_trade_tracker::LiveTradeTracker;
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

/// Broadcasts a `LiveTradeTracker`-produced trade close over the WS feed,
/// the same way live order fills and signals already are. Not published to
/// Kafka — unlike a `Signal`, a live-preview `ClosedTrade` is a UI-overlay
/// derivative of signals already on the audit trail, not a new event to audit.
fn broadcast_closed_trade(broadcaster: &broadcast::Sender<String>, closed: ClosedTrade) {
    match serde_json::to_string(&WsMessage::ClosedTrade(closed)) {
        Ok(json) => {
            let _ = broadcaster.send(json);
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize live closed trade");
        }
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
    // Protobuf side of the `signals` topic (ROADMAP Loop 4c), published in
    // parallel with the JSON topic during the migration.
    let signals_proto_topic =
        std::env::var("KAFKA_TOPIC_SIGNALS_PROTO").unwrap_or_else(|_| "signals.proto".to_string());
    let tracker = LiveTradeTracker::new();

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

            // The reference price the strategy used to evaluate this tick —
            // the same value its `RiskRewardConfig::compute` (if any) was
            // given — doubles as the live-preview fill price for
            // `LiveTradeTracker::on_signal` below.
            let reference_price = match &msg {
                WsMessage::PriceUpdate(p) => Some(p.ask),
                WsMessage::Candle(c) => Some(c.close),
                WsMessage::Signal(_) | WsMessage::OrderUpdate(_) | WsMessage::ClosedTrade(_) => {
                    None
                }
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
                    // Intrabar SL/TP check happens before the strategy even
                    // sees this candle — mirrors the backtest venue's order
                    // (Loop 6f): a position can be stopped out by a candle's
                    // range before any new signal from that same candle.
                    if let Some(closed) = tracker.check_intrabar_stop_loss_take_profit(
                        strategy.strategy_id(),
                        c.low,
                        c.high,
                        c.time,
                    ) {
                        broadcast_closed_trade(&broadcaster, closed);
                    }
                    strategy.on_candle(&candle_from_payload(c))
                }
                WsMessage::Signal(_) | WsMessage::OrderUpdate(_) | WsMessage::ClosedTrade(_) => {
                    None
                }
            };

            if let Some(sig) = signal {
                if let Some(price) = reference_price {
                    if let Some(closed) = tracker.on_signal(&sig, price, sig.timestamp) {
                        broadcast_closed_trade(&broadcaster, closed);
                    }
                }
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
                    stop_loss: sig.stop_loss,
                    take_profit: sig.take_profit,
                };

                if let Some(ref pub_) = publisher {
                    let proto_bytes = crate::proto::encode_signal(&payload);
                    if let Err(e) = pub_
                        .publish_bytes(&signals_proto_topic, &sig.strategy_id, &proto_bytes)
                        .await
                    {
                        tracing::error!(
                            error = %e,
                            signal_id = %signal_id,
                            strategy_id = %sig.strategy_id,
                            "failed to publish signal protobuf to kafka"
                        );
                    }
                }

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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use prost::Message;

    use super::*;
    use crate::kafka::port::mock::MockPublisher;
    use crate::strategy::builtin::spread_threshold::SpreadThresholdStrategy;

    #[tokio::test(flavor = "current_thread")]
    async fn runner_publishes_signal_as_protobuf_on_signals_proto_topic() {
        let publisher = Arc::new(MockPublisher::new());
        let (tx, _rx) = broadcast::channel::<String>(16);
        let strategy: Arc<dyn Strategy> = Arc::new(SpreadThresholdStrategy::new(
            "test-spread",
            "tabdeal",
            "USDT/IRT",
            1000,
        ));

        let handle = spawn_strategy_runner(
            strategy,
            tx.clone(),
            Some(Arc::clone(&publisher) as Arc<dyn MessagePublisher>),
            None,
        );

        // spread = ask - bid = 2000 > threshold 1000 → a buy signal.
        let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            bid: 175_000,
            ask: 177_000,
            timestamp: Utc::now(),
        }))
        .unwrap();
        tx.send(price_json).unwrap();

        let signal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some((_, key, bytes)) = publisher
                    .published_bytes()
                    .into_iter()
                    .find(|(topic, _, _)| topic == "signals.proto")
                {
                    let signal = crate::proto::v1::Signal::decode(&bytes[..])
                        .expect("protobuf signal must decode");
                    break (key, signal);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("a signal must be published to the signals.proto topic within 2s");

        handle.abort();

        let (key, signal) = signal;
        assert_eq!(key, "test-spread", "proto signal keeps the strategy_id key");
        assert_eq!(signal.action, "buy");
        assert_eq!(signal.exchange, "tabdeal");
        assert_eq!(signal.pair, "USDT/IRT");
        assert!(!signal.signal_id.is_empty(), "signal_id must be set");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_without_publisher_still_broadcasts_signal() {
        let (tx, mut rx) = broadcast::channel::<String>(16);
        let strategy: Arc<dyn Strategy> = Arc::new(SpreadThresholdStrategy::new(
            "test-spread",
            "tabdeal",
            "USDT/IRT",
            1000,
        ));

        let handle = spawn_strategy_runner(strategy, tx.clone(), None, None);

        let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            bid: 175_000,
            ask: 177_000,
            timestamp: Utc::now(),
        }))
        .unwrap();
        tx.send(price_json).unwrap();

        let saw_signal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Ok(text) => {
                        if let Ok(WsMessage::Signal(_)) = serde_json::from_str::<WsMessage>(&text) {
                            break true;
                        }
                    }
                    Err(_) => break false,
                }
            }
        })
        .await
        .expect("signal broadcast must arrive within 2s");

        handle.abort();
        assert!(
            saw_signal,
            "a signal must still broadcast over WS when no Kafka publisher is configured"
        );
    }
}
