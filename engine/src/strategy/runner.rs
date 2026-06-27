use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::backtest::entity::ClosedTrade;
use crate::candle::entity::{Candle, CandlePayload, Interval};
use crate::exchange::entity::ExchangeId;
use crate::infrastructure::db::signal_repository::{SignalRecord, SignalRepository};
use crate::kafka::port::MessagePublisher;
use crate::presentation::shared::broadcast::BroadcastEnvelope;
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
fn broadcast_closed_trade(broadcaster: &broadcast::Sender<BroadcastEnvelope>, closed: ClosedTrade) {
    match serde_json::to_string(&WsMessage::ClosedTrade(closed)) {
        Ok(json) => {
            let _ = broadcaster.send(BroadcastEnvelope::public(json));
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
    broadcaster: broadcast::Sender<BroadcastEnvelope>,
    publisher: Option<Arc<dyn MessagePublisher>>,
    signal_repository: Option<Arc<dyn SignalRepository>>,
) -> AbortHandle {
    // Supervised so a panic in strategy evaluation restarts the runner with
    // backoff instead of the strategy silently going dark (M10). The factory
    // retains a broadcaster Sender, so the receiver never sees a spurious
    // "closed" between restarts.
    let name = format!("strategy:{}", strategy.strategy_id());
    // Subscribe synchronously here, before the first run, so a message published
    // immediately after this call is not missed in the window before the spawned
    // task starts. Restarts re-subscribe inside the factory (they only need to
    // catch the ongoing stream).
    let mut first_rx = Some(broadcaster.subscribe());
    crate::strategy::supervisor::spawn_supervised(
        name,
        crate::strategy::supervisor::BackoffPolicy::default(),
        move || {
            let rx = first_rx.take().unwrap_or_else(|| broadcaster.subscribe());
            run_strategy_loop(
                rx,
                strategy.clone(),
                broadcaster.clone(),
                publisher.clone(),
                signal_repository.clone(),
            )
        },
    )
}

async fn run_strategy_loop(
    mut rx: broadcast::Receiver<BroadcastEnvelope>,
    strategy: Arc<dyn Strategy>,
    broadcaster: broadcast::Sender<BroadcastEnvelope>,
    publisher: Option<Arc<dyn MessagePublisher>>,
    signal_repository: Option<Arc<dyn SignalRepository>>,
) {
    let signals_proto_topic =
        std::env::var("KAFKA_TOPIC_SIGNALS_PROTO").unwrap_or_else(|_| "signals.proto".to_string());
    let tracker = LiveTradeTracker::new();

    loop {
        let envelope = match rx.recv().await {
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

        let msg: WsMessage = match serde_json::from_str(&envelope.payload) {
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
            WsMessage::Signal(_) | WsMessage::OrderUpdate(_) | WsMessage::ClosedTrade(_) => None,
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
            WsMessage::Signal(_) | WsMessage::OrderUpdate(_) | WsMessage::ClosedTrade(_) => None,
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
                    let _ = broadcaster.send(BroadcastEnvelope::public(json));
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
        let (tx, _rx) = broadcast::channel::<BroadcastEnvelope>(16);
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
        tx.send(BroadcastEnvelope::public(price_json)).unwrap();

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

    /// ROADMAP Loop 4c-3 acceptance test for signals: the JSON `signals` topic
    /// must receive no messages once consumers have migrated to `signals.proto`.
    /// Uses proto arrival as the "signal was processed" sentinel.
    #[tokio::test(flavor = "current_thread")]
    async fn json_signal_topic_no_longer_receives_messages() {
        let publisher = Arc::new(MockPublisher::new());
        let (tx, _rx) = broadcast::channel::<BroadcastEnvelope>(16);
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

        let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            bid: 175_000,
            ask: 177_000,
            timestamp: Utc::now(),
        }))
        .unwrap();
        tx.send(BroadcastEnvelope::public(price_json)).unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if publisher
                    .published_bytes()
                    .iter()
                    .any(|(topic, _, _)| topic == "signals.proto")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("signals.proto must receive a message — used as processing sentinel");

        handle.abort();

        let json_messages = publisher.published();
        assert!(
            json_messages.iter().all(|(topic, _, _)| topic != "signals"),
            "the JSON signals topic must receive no messages after the protobuf migration"
        );
    }

    /// After proto migration, every JSON Kafka message from the strategy runner
    /// must come from a topic that is NOT `signals`. Signals only go as protobuf.
    #[tokio::test(flavor = "current_thread")]
    async fn strategy_runner_emits_no_json_kafka_traffic_after_signal_migration() {
        let publisher = Arc::new(MockPublisher::new());
        let (tx, _rx) = broadcast::channel::<BroadcastEnvelope>(16);
        let strategy: Arc<dyn Strategy> = Arc::new(SpreadThresholdStrategy::new(
            "sig-audit",
            "tabdeal",
            "USDT/IRT",
            500,
        ));

        let handle = spawn_strategy_runner(
            strategy,
            tx.clone(),
            Some(Arc::clone(&publisher) as Arc<dyn MessagePublisher>),
            None,
        );

        let price_json = serde_json::to_string(&WsMessage::PriceUpdate(PricePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            bid: 175_000,
            ask: 177_000,
            timestamp: Utc::now(),
        }))
        .unwrap();
        tx.send(BroadcastEnvelope::public(price_json)).unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if publisher
                    .published_bytes()
                    .iter()
                    .any(|(topic, _, _)| topic == "signals.proto")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("signals.proto must arrive to confirm processing");

        handle.abort();

        let json_messages = publisher.published();
        for (topic, _, _) in &json_messages {
            assert_ne!(
                topic, "signals",
                "the JSON signals Kafka topic must not be written after migration; found: {topic}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runner_without_publisher_still_broadcasts_signal() {
        let (tx, mut rx) = broadcast::channel::<BroadcastEnvelope>(16);
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
        tx.send(BroadcastEnvelope::public(price_json)).unwrap();

        let saw_signal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Ok(env) => {
                        if let Ok(WsMessage::Signal(_)) =
                            serde_json::from_str::<WsMessage>(&env.payload)
                        {
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
