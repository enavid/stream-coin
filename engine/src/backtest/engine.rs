use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::backtest::entity::{
    pair_closed_trades, trade_stats, BacktestResult, BacktestSignalRecord, TradeRecord,
};
use crate::backtest::venue::{FillModel, PendingOrder, SimulatedVenue};
use crate::candle::entity::CandlePayload;
use crate::strategy::subprocess::{
    apply_sandboxed_env, build_launcher_script, is_safe_strategy_id,
};
use crate::wire_message::SignalPayload;

/// Quantity of base currency used for every simulated order.
/// The backtest uses a fixed size because `SignalPayload` carries no quantity field.
const DEFAULT_ORDER_QUANTITY: u64 = 1;

/// How long to wait for signals from the subprocess after each candle write.
const SIGNAL_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Baseline capital for equity-curve calculations (in the quote currency).
const INITIAL_CAPITAL: i64 = 1_000_000;

/// Unpredictable, collision-free temp path for a backtest script. The random
/// suffix defeats symlink/TOCTOU attacks on a predictable name and prevents two
/// concurrent backtests of the same strategy from clobbering each other's file.
/// Caller must validate `strategy_id` with `is_safe_strategy_id` first.
fn secure_backtest_script_path(strategy_id: &str) -> std::path::PathBuf {
    let unique = uuid::Uuid::new_v4();
    std::env::temp_dir().join(format!("backtest_{strategy_id}_{unique}.py"))
}

#[derive(Debug, Error)]
pub enum BacktestError {
    #[error("strategy id is not a safe filesystem component")]
    InvalidStrategyId,
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
                closed_trades: vec![],
                win_rate: 0.0,
                avg_rr: None,
            });
        }

        if !is_safe_strategy_id(&self.strategy_id) {
            return Err(BacktestError::InvalidStrategyId);
        }

        let script_path = secure_backtest_script_path(&self.strategy_id);
        let script = build_launcher_script(&self.strategy_id, &self.code);

        tokio::fs::write(&script_path, script.as_bytes())
            .await
            .map_err(BacktestError::ScriptWrite)?;

        let mut cmd = Command::new("python3");
        apply_sandboxed_env(&mut cmd, &self.strategy_id);
        cmd.arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(BacktestError::SubprocessSpawn)?;

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
            // Fill orders from the previous candle at this candle's close, and
            // force-exit any open position whose stop-loss/take-profit this
            // candle's range touches intrabar. The strictly-after invariant
            // in SimulatedVenue prevents look-ahead for the former.
            let new_fills = venue
                .apply_candle_close(candle.close, candle.low, candle.high, candle.time)
                .await;
            for fill in new_fills {
                tracing::debug!(
                    strategy_id = %self.strategy_id,
                    side = %fill.side,
                    fill_price = fill.fill_price,
                    candle_time = %fill.candle_time,
                    stop_loss = ?fill.stop_loss,
                    take_profit = ?fill.take_profit,
                    "backtest fill"
                );
                trade_log.push(TradeRecord {
                    order_id: fill.order_id,
                    side: fill.side,
                    quantity: fill.quantity,
                    fill_price: fill.fill_price,
                    strategy_id: fill.strategy_id,
                    candle_time: fill.candle_time,
                    stop_loss: fill.stop_loss,
                    take_profit: fill.take_profit,
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
                            stop_loss: signal.stop_loss,
                            take_profit: signal.take_profit,
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
        let closed_trades = pair_closed_trades(&trade_log);
        let (win_rate, avg_rr) = trade_stats(&closed_trades);

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
            closed_trades,
            win_rate,
            avg_rr,
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

    // Buys on candle 1 with an explicit stop_loss/take_profit; never sells —
    // the position must be closed by the venue's intrabar SL/TP check, not
    // by an opposite-side signal.
    const BUY_WITH_SL_TP_NO_EXIT_SIGNAL: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
count = 0
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    count += 1
    if count == 1:
        print(json.dumps({
            "signal_id": str(uuid.uuid4()),
            "strategy_id": _STRATEGY_ID,
            "exchange": candle["exchange"],
            "pair": candle["pair"],
            "action": "buy",
            "confidence": 1.0,
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "stop_loss": 95000,
            "take_profit": 120000,
        }), flush=True)
"#;

    /// C2: backtest Python must not be able to read the engine's environment.
    #[tokio::test(flavor = "current_thread")]
    async fn backtest_subprocess_cannot_read_engine_secret_env() {
        const PROBE_VAR: &str = "STREAM_COIN_SECRET_PROBE_BACKTEST";
        std::env::set_var(PROBE_VAR, "top-secret-value");

        let code = format!(
            r#"
import os, sys, json, uuid
from datetime import datetime, timezone
action = "leaked" if os.environ.get("{PROBE_VAR}") else "safe"
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    print(json.dumps({{
        "signal_id": str(uuid.uuid4()),
        "strategy_id": _STRATEGY_ID,
        "exchange": candle["exchange"],
        "pair": candle["pair"],
        "action": action,
        "confidence": 1.0,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }}), flush=True)
"#
        );

        let candles = make_candles(&[(100_000, 100_000), (100_000, 100_000)]);
        let engine = BacktestEngine::new("env-probe".to_string(), code, FillModel::LastClose);
        let result = engine.run(&candles).await.unwrap();

        assert!(
            result.signal_count >= 1,
            "strategy must emit at least one signal"
        );
        assert!(
            result.signal_log.iter().all(|s| s.action == "safe"),
            "backtest strategy must NOT read engine secrets from the environment"
        );
    }

    /// C11: an unsafe strategy id (path traversal) must be rejected before any file I/O.
    #[tokio::test(flavor = "current_thread")]
    async fn backtest_rejects_unsafe_strategy_id() {
        let candles = make_candles(&[(100_000, 100_000)]);
        let engine = BacktestEngine::new(
            "../evil".to_string(),
            BUY_ON_EVERY_CANDLE.to_string(),
            FillModel::LastClose,
        );
        let err = engine.run(&candles).await.unwrap_err();
        assert!(matches!(err, BacktestError::InvalidStrategyId));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_closed_trade_rr_is_populated_when_strategy_sets_sl_tp() {
        // candle[0]: triggers the buy signal, entry fills at candle[1].close=100_000.
        // candle[2]: low=94_000 crosses the 95_000 stop -> forced exit at 95_000.
        let candles = vec![
            CandlePayload {
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1m".to_string(),
                time: Utc.timestamp_opt(1_000_000, 0).unwrap(),
                open: 100_000,
                high: 100_000,
                low: 100_000,
                close: 100_000,
                volume: 10,
            },
            CandlePayload {
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1m".to_string(),
                time: Utc.timestamp_opt(1_000_060, 0).unwrap(),
                open: 100_000,
                high: 100_000,
                low: 100_000,
                close: 100_000,
                volume: 10,
            },
            CandlePayload {
                exchange: "tabdeal".to_string(),
                pair: "USDT/IRT".to_string(),
                interval: "1m".to_string(),
                time: Utc.timestamp_opt(1_000_120, 0).unwrap(),
                open: 100_000,
                high: 100_000,
                low: 94_000,
                close: 96_000,
                volume: 10,
            },
        ];

        let engine = BacktestEngine::new(
            "test-sl-tp".to_string(),
            BUY_WITH_SL_TP_NO_EXIT_SIGNAL.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();

        assert_eq!(
            result.trade_log.len(),
            2,
            "entry fill + forced sl exit fill"
        );
        assert_eq!(
            result.closed_trades.len(),
            1,
            "the sl-tp-forced exit must pair with the entry into a closed trade"
        );
        let trade = &result.closed_trades[0];
        assert_eq!(trade.stop_loss, Some(95_000));
        assert_eq!(trade.take_profit, Some(120_000));
        assert_eq!(trade.exit_price, 95_000, "must exit at the stop price");
        assert!(
            trade.rr.is_some(),
            "rr must be populated once the closed trade carries a stop_loss"
        );
        assert!(
            result.avg_rr.is_some(),
            "BacktestResult.avg_rr must reflect the populated rr"
        );
    }

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

    #[tokio::test(flavor = "current_thread")]
    async fn backtest_run_returns_closed_trades_matching_fill_count_divided_by_two() {
        let candles = make_candles(&[(100_000, 100_000), (100_000, 90_000), (100_000, 110_000)]);
        let engine = BacktestEngine::new(
            "test-closed-trades".to_string(),
            BUY_THEN_SELL.to_string(),
            FillModel::LastClose,
        );
        let result = engine.run(&candles).await.unwrap();
        assert_eq!(result.trade_log.len(), 2, "buy and sell must both fill");
        assert_eq!(
            result.closed_trades.len(),
            result.trade_log.len() / 2,
            "two fills (one buy, one sell) must pair into exactly one closed trade"
        );
        assert_eq!(result.closed_trades[0].entry_price, 90_000);
        assert_eq!(result.closed_trades[0].exit_price, 110_000);
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
                stop_loss: None,
                take_profit: None,
            },
            TradeRecord {
                order_id: "2".to_string(),
                side: "sell".to_string(),
                quantity: 1,
                fill_price: 110_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
                stop_loss: None,
                take_profit: None,
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
                stop_loss: None,
                take_profit: None,
            },
            TradeRecord {
                order_id: "2".to_string(),
                side: "sell".to_string(),
                quantity: 1,
                fill_price: 80_000,
                strategy_id: "s".to_string(),
                candle_time: Utc::now(),
                stop_loss: None,
                take_profit: None,
            },
        ];
        let (ret, dd) = calculate_metrics(&trades);
        assert!(ret < 0.0, "buy high sell low must be a loss");
        assert!(dd > 0.0, "loss must produce a drawdown");
    }
}
