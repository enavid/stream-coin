use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::backtest::entity::{BacktestResult, BacktestSignalRecord, TradeRecord};
use crate::backtest::venue::{FillModel, PendingOrder, SimulatedVenue};
use crate::candle::entity::CandlePayload;
use crate::strategy::subprocess::build_launcher_script;
use crate::wire_message::SignalPayload;

/// Quantity of base currency used for every simulated order.
/// The backtest uses a fixed size because `SignalPayload` carries no quantity field.
const DEFAULT_ORDER_QUANTITY: u64 = 1;

/// How long to wait for signals from the subprocess after each candle write.
const SIGNAL_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Baseline capital for equity-curve calculations (in the quote currency).
const INITIAL_CAPITAL: i64 = 1_000_000;

#[derive(Debug, Error)]
pub enum BacktestError {
    #[error("failed to write strategy script: {0}")]
    ScriptWrite(std::io::Error),
    #[error("failed to spawn python3 subprocess: {0}")]
    SubprocessSpawn(std::io::Error),
}

/// Replays historical candles through a Python strategy subprocess, collecting
/// signals and simulating order fills via `SimulatedVenue`.
///
/// The subprocess receives the same launcher script as the live path (including
/// the seccomp preamble on Linux), ensuring identical strategy behaviour.
pub struct BacktestEngine {
    strategy_id: String,
    code: String,
    fill_model: FillModel,
}

impl BacktestEngine {
    pub fn new(strategy_id: String, code: String, fill_model: FillModel) -> Self {
        Self {
            strategy_id,
            code,
            fill_model,
        }
    }

    pub async fn run(&self, candles: &[CandlePayload]) -> Result<BacktestResult, BacktestError> {
        let exchange = candles
            .first()
            .map(|c| c.exchange.clone())
            .unwrap_or_default();
        let pair = candles.first().map(|c| c.pair.clone()).unwrap_or_default();
        let interval = candles
            .first()
            .map(|c| c.interval.clone())
            .unwrap_or_default();

        if candles.is_empty() {
            return Ok(BacktestResult {
                strategy_id: self.strategy_id.clone(),
                exchange,
                pair,
                interval,
                candle_count: 0,
                signal_count: 0,
                total_return_pct: 0.0,
                max_drawdown_pct: 0.0,
                trade_log: vec![],
                signal_log: vec![],
            });
        }

        let script_path = std::env::temp_dir().join(format!("backtest_{}.py", self.strategy_id));
        let script = build_launcher_script(&self.strategy_id, &self.code);

        tokio::fs::write(&script_path, script.as_bytes())
            .await
            .map_err(BacktestError::ScriptWrite)?;

        let mut child = Command::new("python3")
            .arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("STRATEGY_ID", &self.strategy_id)
            .kill_on_drop(true)
            .spawn()
            .map_err(BacktestError::SubprocessSpawn)?;

        let mut stdin = child.stdin.take().expect("stdin must be piped");
        let stdout = child.stdout.take().expect("stdout must be piped");
        let stderr = child.stderr.take().expect("stderr must be piped");

        let strategy_id_log = self.strategy_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(
                    strategy_id = %strategy_id_log,
                    stderr = %line,
                    "backtest subprocess stderr"
                );
            }
        });

        let mut stdout_lines = BufReader::new(stdout).lines();
        let venue = SimulatedVenue::new(self.fill_model.clone());
        let mut signal_log: Vec<BacktestSignalRecord> = Vec::new();
        let mut trade_log: Vec<TradeRecord> = Vec::new();

        for candle in candles {
            // Fill orders from the previous candle at this candle's close.
            // The strictly-after invariant in SimulatedVenue prevents look-ahead.
            let new_fills = venue.apply_candle_close(candle.close, candle.time).await;
            for fill in new_fills {
                tracing::debug!(
                    strategy_id = %self.strategy_id,
                    side = %fill.side,
                    fill_price = fill.fill_price,
                    candle_time = %fill.candle_time,
                    "backtest fill"
                );
                trade_log.push(TradeRecord {
                    order_id: fill.order_id,
                    side: fill.side,
                    quantity: fill.quantity,
                    fill_price: fill.fill_price,
                    strategy_id: fill.strategy_id,
                    candle_time: fill.candle_time,
                });
            }

            // Send this candle to the subprocess.
            let candle_json = match serde_json::to_string(candle) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        strategy_id = %self.strategy_id,
                        "failed to serialize candle for backtest subprocess"
                    );
                    continue;
                }
            };
            if stdin
                .write_all(format!("{candle_json}\n").as_bytes())
                .await
                .is_err()
                || stdin.flush().await.is_err()
            {
                tracing::warn!(
                    strategy_id = %self.strategy_id,
                    "backtest subprocess stdin closed early"
                );
                break;
            }

            // Collect any signals the subprocess emits in response to this candle.
            let signals = drain_signals(&mut stdout_lines, SIGNAL_READ_TIMEOUT).await;
            for signal in signals {
                tracing::debug!(
                    strategy_id = %self.strategy_id,
                    signal_id = %signal.signal_id,
                    action = %signal.action,
                    confidence = signal.confidence,
                    exchange = %signal.exchange,
                    pair = %signal.pair,
                    "backtest signal received"
                );
                let action = signal.action.clone();
                signal_log.push(BacktestSignalRecord {
                    signal_id: signal.signal_id.clone(),
                    strategy_id: signal.strategy_id.clone(),
                    exchange: signal.exchange.clone(),
                    pair: signal.pair.clone(),
                    action: signal.action.clone(),
                    confidence: signal.confidence,
                    timestamp: signal.timestamp,
                });
                if action == "buy" || action == "sell" {
                    venue
                        .place_order(PendingOrder {
                            order_id: signal.signal_id,
                            side: action,
                            quantity: DEFAULT_ORDER_QUANTITY,
                            strategy_id: signal.strategy_id,
                            placed_at: candle.time,
                        })
                        .await;
                }
            }
        }

        // Close stdin — signals EOF to the subprocess, letting it flush and exit.
        drop(stdin);

        // Drain any final signals the strategy emits after processing all candles.
        let remaining = drain_signals(&mut stdout_lines, Duration::from_millis(500)).await;
        for signal in remaining {
            signal_log.push(BacktestSignalRecord {
                signal_id: signal.signal_id,
                strategy_id: signal.strategy_id,
                exchange: signal.exchange,
                pair: signal.pair,
                action: signal.action,
                confidence: signal.confidence,
                timestamp: signal.timestamp,
            });
        }

        let _ = child.wait().await;
        let _ = tokio::fs::remove_file(&script_path).await;

        let candle_count = candles.len();
        let signal_count = signal_log.len();
        let (total_return_pct, max_drawdown_pct) = calculate_metrics(&trade_log);

        tracing::info!(
            strategy_id = %self.strategy_id,
            exchange = %exchange,
            pair = %pair,
            interval = %interval,
            candle_count = candle_count,
            signal_count = signal_count,
            trade_count = trade_log.len(),
            total_return_pct = total_return_pct,
            max_drawdown_pct = max_drawdown_pct,
            "backtest complete"
        );

        Ok(BacktestResult {
            strategy_id: self.strategy_id.clone(),
            exchange,
            pair,
            interval,
            candle_count,
            signal_count,
            total_return_pct,
            max_drawdown_pct,
            trade_log,
            signal_log,
        })
    }
}

/// Read signal JSON lines from `lines` until the timeout elapses between reads.
async fn drain_signals<B>(lines: &mut tokio::io::Lines<B>, timeout: Duration) -> Vec<SignalPayload>
where
    B: tokio::io::AsyncBufRead + Unpin,
{
    let mut signals = Vec::new();
    loop {
        match tokio::time::timeout(timeout, lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<SignalPayload>(trimmed) {
                    Ok(s) => signals.push(s),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            raw = %trimmed,
                            "backtest subprocess produced unparseable output"
                        );
                    }
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => break, // timeout
        }
    }
    signals
}

/// Calculate total return % and max drawdown % from the trade log.
///
/// Uses mark-to-market equity at each fill: equity = cash + position × last_fill_price.
/// This means a buy does not create a drawdown if the price hasn't moved since purchase.
fn calculate_metrics(trade_log: &[TradeRecord]) -> (f64, f64) {
    if trade_log.is_empty() {
        return (0.0, 0.0);
    }

    let mut cash: i64 = INITIAL_CAPITAL;
    let mut position: i64 = 0;
    let mut peak_total: f64 = INITIAL_CAPITAL as f64;
    let mut max_drawdown_pct: f64 = 0.0;

    for trade in trade_log {
        let value = (trade.fill_price as i64) * (trade.quantity as i64);
        match trade.side.as_str() {
            "buy" => {
                cash -= value;
                position += trade.quantity as i64;
            }
            "sell" => {
                cash += value;
                position -= trade.quantity as i64;
            }
            _ => {}
        }
        let total = cash + position * (trade.fill_price as i64);
        let total_f = total as f64;
        if total_f > peak_total {
            peak_total = total_f;
        }
        if peak_total > 0.0 {
            let drawdown = (peak_total - total_f) / peak_total * 100.0;
            if drawdown > max_drawdown_pct {
                max_drawdown_pct = drawdown;
            }
        }
    }

    let total_return_pct = (cash as f64 - INITIAL_CAPITAL as f64) / INITIAL_CAPITAL as f64 * 100.0;
    (total_return_pct, max_drawdown_pct)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn make_candles(closes: &[(u64, u64)]) -> Vec<CandlePayload> {
        closes
            .iter()
            .enumerate()
            .map(|(i, (open, close))| CandlePayload {
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1m".to_string(),
                time: Utc.timestamp_opt(1_000_000 + i as i64 * 60, 0).unwrap(),
                open: *open,
                high: *open,
                low: *close,
                close: *close,
                volume: 100,
            })
            .collect()
    }

    // Python strategy that emits a buy signal for every candle it receives.
    const BUY_ON_EVERY_CANDLE: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    print(json.dumps({
        "signal_id": str(uuid.uuid4()),
        "strategy_id": _STRATEGY_ID,
        "exchange": candle["exchange"],
        "pair": candle["pair"],
        "action": "buy",
        "confidence": 1.0,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }), flush=True)
"#;

    // Python strategy that buys on candle 1, sells on candle 2.
    const BUY_THEN_SELL: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    count += 1
    action = "buy" if count == 1 else "sell" if count == 2 else None
    if action:
        print(json.dumps({
            "signal_id": str(uuid.uuid4()),
            "strategy_id": _STRATEGY_ID,
            "exchange": candle["exchange"],
            "pair": candle["pair"],
            "action": action,
            "confidence": 1.0,
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }), flush=True)
"#;

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_engine_empty_candles_returns_empty_result() {
        let engine = BacktestEngine::new(
            "test-empty".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&[]).await.unwrap();
        assert_eq!(result.candle_count, 0);
        assert_eq!(result.signal_count, 0);
        assert!(result.trade_log.is_empty());
        assert!((result.total_return_pct).abs() < f64::EPSILON);
        assert!((result.max_drawdown_pct).abs() < f64::EPSILON);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_engine_subprocess_receives_all_candles() {
        let candles = make_candles(&[(100_000, 100_000); 3]);
        let engine = BacktestEngine::new(
            "test-receive".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        // One signal per candle proves every candle reached the subprocess.
        assert_eq!(
            result.signal_count, 3,
            "subprocess must receive all candles"
        );
        assert_eq!(result.candle_count, 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_engine_collects_signals_from_subprocess() {
        let candles = make_candles(&[(100_000, 100_000); 3]);
        let engine = BacktestEngine::new(
            "test-signals".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        assert_eq!(result.signal_log.len(), 3);
        assert!(result.signal_log.iter().all(|s| s.action == "buy"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_engine_fills_order_at_next_candle_close() {
        // candle[0] close=100K, candle[1] close=90K
        // Buy signal arrives after candle[0] → must fill at candle[1].close = 90K, NOT 100K.
        let candles = make_candles(&[(100_000, 100_000), (100_000, 90_000)]);
        let engine = BacktestEngine::new(
            "test-fill".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();

        // Signal from candle[0] produces a fill at candle[1].close = 90K.
        // Signal from candle[1] has no next candle to fill at, so stays pending.
        assert_eq!(result.trade_log.len(), 1, "exactly one fill expected");
        assert_eq!(result.trade_log[0].side, "buy");
        assert_eq!(
            result.trade_log[0].fill_price, 90_000,
            "must fill at next candle close, not current candle close"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_produces_same_signals_as_live_for_same_candles() {
        let candles = make_candles(&[(100_000, 100_000); 5]);
        let engine = BacktestEngine::new(
            "test-deterministic-a".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let run1 = engine.run(&candles).await.unwrap();

        let engine2 = BacktestEngine::new(
            "test-deterministic-b".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let run2 = engine2.run(&candles).await.unwrap();

        assert_eq!(
            run1.signal_count, run2.signal_count,
            "same code + same candles must produce the same number of signals"
        );
        let actions1: Vec<&str> = run1.signal_log.iter().map(|s| s.action.as_str()).collect();
        let actions2: Vec<&str> = run2.signal_log.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(actions1, actions2, "signal actions must be identical");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_result_total_return_positive_on_profit() {
        // buy fills at candle[1].close=90K, sell fills at candle[2].close=110K → profit
        let candles = make_candles(&[(100_000, 100_000), (100_000, 90_000), (100_000, 110_000)]);
        let engine = BacktestEngine::new(
            "test-return-profit".to_string(),
            BUY_THEN_SELL.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        assert_eq!(
            result.trade_log.len(),
            2,
            "buy and sell must both be filled"
        );
        assert!(
            result.total_return_pct > 0.0,
            "profitable trades must yield positive return, got {}",
            result.total_return_pct
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_result_total_return_negative_on_loss() {
        // buy fills at candle[1].close=110K, sell fills at candle[2].close=80K → loss
        let candles = make_candles(&[(100_000, 100_000), (100_000, 110_000), (100_000, 80_000)]);
        let engine = BacktestEngine::new(
            "test-return-loss".to_string(),
            BUY_THEN_SELL.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        assert_eq!(result.trade_log.len(), 2);
        assert!(
            result.total_return_pct < 0.0,
            "losing trades must yield negative return, got {}",
            result.total_return_pct
        );
        assert!(
            result.max_drawdown_pct > 0.0,
            "sell below buy must produce a drawdown, got {}",
            result.max_drawdown_pct
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_result_max_drawdown_zero_on_consistent_profit() {
        // buy at 90K, sell at 120K → mark-to-market equity never drops below peak
        let candles = make_candles(&[(100_000, 100_000), (100_000, 90_000), (100_000, 120_000)]);
        let engine = BacktestEngine::new(
            "test-drawdown-zero".to_string(),
            BUY_THEN_SELL.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        assert_eq!(result.trade_log.len(), 2);
        assert!(
            result.max_drawdown_pct < 1e-9,
            "no drawdown expected for a profit trade, got {}",
            result.max_drawdown_pct
        );
    }

    #[test]
    fn calculate_metrics_buy_profit_returns_positive() {
        let trades = vec![
            TradeRecord {
                order_id: "1".to_string(),
                side: "buy".to_string(),
                quantity: 1,
                fill_price: 90_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
            },
            TradeRecord {
                order_id: "2".to_string(),
                side: "sell".to_string(),
                quantity: 1,
                fill_price: 110_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
            },
        ];
        let (ret, dd) = calculate_metrics(&trades);
        assert!(ret > 0.0, "buy low sell high must be a profit");
        assert!(dd < 1e-9, "no drawdown expected for immediate profit");
    }

    #[test]
    fn calculate_metrics_buy_loss_returns_negative_with_drawdown() {
        let trades = vec![
            TradeRecord {
                order_id: "1".to_string(),
                side: "buy".to_string(),
                quantity: 1,
                fill_price: 110_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
            },
            TradeRecord {
                order_id: "2".to_string(),
                side: "sell".to_string(),
                quantity: 1,
                fill_price: 80_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
            },
        ];
        let (ret, dd) = calculate_metrics(&trades);
        assert!(ret < 0.0, "buy high sell low must be a loss");
        assert!(dd > 0.0, "loss must produce a drawdown");
    }
}
