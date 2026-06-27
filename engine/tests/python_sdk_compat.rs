/// Cross-language protobuf compatibility: Rust prost encoder → Python betterproto decoder.
///
/// This test is the ROADMAP Loop 4c-2 acceptance test. It encodes a CandlePayload with
/// the same `encode_candle()` function that publishes to the Kafka `candles.proto` topic,
/// passes those bytes to the Python SDK's betterproto decoder, and verifies every field
/// survives the wire without loss or corruption.
///
/// The test is skipped (not failed) when the SDK venv or betterproto is unavailable so
/// that `just check` stays green in environments where Python dependencies aren't installed.
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use chrono::{DateTime, Utc};

use stream_coin::candle::entity::CandlePayload;
use stream_coin::proto::{encode_candle, encode_signal};
use stream_coin::wire_message::SignalPayload;

const SDK_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../python_sdk");

/// Returns the Python executable that has betterproto installed, preferring the
/// SDK-local venv over the system interpreter. Returns `None` if neither has it.
fn python_with_betterproto() -> Option<String> {
    let venv_python = format!("{SDK_DIR}/.venv/bin/python3");
    for candidate in [venv_python.as_str(), "python3"] {
        if !Path::new(candidate).exists() && candidate != "python3" {
            continue;
        }
        let ok = Command::new(candidate)
            .args(["-c", "import betterproto"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

fn at_ms(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).expect("valid milliseconds")
}

/// Spawn a Python script, write `input_bytes` to its stdin, and return stdout on success.
fn run_python(python: &str, script: &str, input_bytes: &[u8]) -> String {
    let mut child = Command::new(python)
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {python}: {e}"));

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input_bytes)
        .expect("write proto bytes to stdin");
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("wait for python");
    assert!(
        out.status.success(),
        "Python SDK script failed:\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("valid UTF-8 output")
}

// ---------------------------------------------------------------------------
// Test 1: main ROADMAP acceptance test

#[test]
fn python_sdk_consumes_protobuf_candle_correctly() {
    let Some(python) = python_with_betterproto() else {
        eprintln!(
            "SKIP python_sdk_consumes_protobuf_candle_correctly: \
             betterproto not available (run: cd python_sdk && .venv/bin/pip install betterproto)"
        );
        return;
    };

    let payload = CandlePayload {
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        interval: "1m".to_string(),
        time: at_ms(1_700_000_000_000),
        open: 570_000,
        high: 580_000,
        low: 565_000,
        close: 575_000,
        volume: 10_000,
    };
    let proto_bytes = encode_candle(&payload);

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{sdk_dir}')
from stream_coin.proto.candle import Candle
data = sys.stdin.buffer.read()
c = Candle().parse(data)
print(json.dumps({{
    'exchange': c.exchange,
    'pair': c.pair,
    'interval': c.interval,
    'time': c.time,
    'open': c.open,
    'high': c.high,
    'low': c.low,
    'close': c.close,
    'volume': c.volume,
}}))
"#,
        sdk_dir = SDK_DIR
    );

    let raw_json = run_python(&python, &script, &proto_bytes);
    let decoded: serde_json::Value = serde_json::from_str(&raw_json)
        .unwrap_or_else(|e| panic!("Python SDK must output valid JSON: {e}\nGot: {raw_json}"));

    assert_eq!(decoded["exchange"].as_str().unwrap(), "tabdeal");
    assert_eq!(decoded["pair"].as_str().unwrap(), "USDT/IRT");
    assert_eq!(decoded["interval"].as_str().unwrap(), "1m");
    assert_eq!(decoded["time"].as_i64().unwrap(), 1_700_000_000_000_i64);
    assert_eq!(decoded["open"].as_u64().unwrap(), 570_000);
    assert_eq!(decoded["high"].as_u64().unwrap(), 580_000);
    assert_eq!(decoded["low"].as_u64().unwrap(), 565_000);
    assert_eq!(decoded["close"].as_u64().unwrap(), 575_000);
    assert_eq!(decoded["volume"].as_u64().unwrap(), 10_000);
}

// ---------------------------------------------------------------------------
// Test 2: prost max uint64 must not truncate to int64 range

#[test]
fn python_sdk_decodes_max_uint64_price_without_truncation() {
    let Some(python) = python_with_betterproto() else {
        eprintln!("SKIP: betterproto not available");
        return;
    };

    let payload = CandlePayload {
        exchange: "x".to_string(),
        pair: "X/Y".to_string(),
        interval: "1m".to_string(),
        time: at_ms(0),
        open: u64::MAX,
        high: u64::MAX,
        low: 0,
        close: 0,
        volume: u64::MAX,
    };
    let proto_bytes = encode_candle(&payload);

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{sdk_dir}')
from stream_coin.proto.candle import Candle
c = Candle().parse(sys.stdin.buffer.read())
print(json.dumps({{'open': c.open, 'high': c.high, 'volume': c.volume}}))
"#,
        sdk_dir = SDK_DIR
    );

    let raw = run_python(&python, &script, &proto_bytes);
    let d: serde_json::Value = serde_json::from_str(&raw).unwrap();

    let max_u64 = u64::MAX;
    assert_eq!(
        d["open"].as_u64().unwrap(),
        max_u64,
        "uint64 MAX must not truncate"
    );
    assert_eq!(d["high"].as_u64().unwrap(), max_u64);
    assert_eq!(d["volume"].as_u64().unwrap(), max_u64);
}

// ---------------------------------------------------------------------------
// Test 3: negative int64 timestamp (pre-epoch candles are valid)

#[test]
fn python_sdk_decodes_negative_timestamp_correctly() {
    let Some(python) = python_with_betterproto() else {
        eprintln!("SKIP: betterproto not available");
        return;
    };

    let payload = CandlePayload {
        exchange: "x".to_string(),
        pair: "A/B".to_string(),
        interval: "1h".to_string(),
        time: at_ms(-86_400_000), // one day before Unix epoch
        open: 1,
        high: 1,
        low: 1,
        close: 1,
        volume: 1,
    };
    let proto_bytes = encode_candle(&payload);

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{sdk_dir}')
from stream_coin.proto.candle import Candle
c = Candle().parse(sys.stdin.buffer.read())
print(json.dumps({{'time': c.time}}))
"#,
        sdk_dir = SDK_DIR
    );

    let raw = run_python(&python, &script, &proto_bytes);
    let d: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        d["time"].as_i64().unwrap(),
        -86_400_000_i64,
        "negative int64 timestamp must remain signed after round-trip"
    );
}

// ---------------------------------------------------------------------------
// Test 4: signal optional fields — absent vs explicit zero

#[test]
fn python_sdk_decodes_signal_with_no_stop_loss_as_none() {
    let Some(python) = python_with_betterproto() else {
        eprintln!("SKIP: betterproto not available");
        return;
    };

    let payload = SignalPayload {
        signal_id: "sig-001".to_string(),
        strategy_id: "rsi_cross".to_string(),
        exchange: "tabdeal".to_string(),
        pair: "USDT/IRT".to_string(),
        action: "buy".to_string(),
        confidence: 0.92,
        timestamp: at_ms(1_700_000_000_000),
        stop_loss: None,
        take_profit: Some(600_000),
    };
    let proto_bytes = encode_signal(&payload);

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{sdk_dir}')
from stream_coin.proto.signal import Signal
s = Signal().parse(sys.stdin.buffer.read())
print(json.dumps({{
    'action': s.action,
    'confidence': s.confidence,
    'stop_loss': s.stop_loss,
    'take_profit': s.take_profit,
}}))
"#,
        sdk_dir = SDK_DIR
    );

    let raw = run_python(&python, &script, &proto_bytes);
    let d: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(d["action"].as_str().unwrap(), "buy");
    assert!(
        (d["confidence"].as_f64().unwrap() - 0.92_f64).abs() < 1e-9,
        "double confidence must survive"
    );
    assert!(
        d["stop_loss"].is_null(),
        "absent stop_loss must decode as null/None"
    );
    assert_eq!(d["take_profit"].as_u64().unwrap(), 600_000);
}

// ---------------------------------------------------------------------------
// Test 5: signal with all optional fields present, including explicit zero stop_loss

#[test]
fn python_sdk_decodes_signal_stop_loss_zero_as_zero_not_none() {
    let Some(python) = python_with_betterproto() else {
        eprintln!("SKIP: betterproto not available");
        return;
    };

    let payload = SignalPayload {
        signal_id: "sig-002".to_string(),
        strategy_id: "arb".to_string(),
        exchange: "coinex".to_string(),
        pair: "BTC/USDT".to_string(),
        action: "sell".to_string(),
        confidence: 0.75,
        timestamp: at_ms(1_700_000_000_000),
        stop_loss: Some(0), // explicit zero — must not collapse to None
        take_profit: Some(999_999),
    };
    let proto_bytes = encode_signal(&payload);

    let script = format!(
        r#"
import sys, json
sys.path.insert(0, r'{sdk_dir}')
from stream_coin.proto.signal import Signal
s = Signal().parse(sys.stdin.buffer.read())
print(json.dumps({{'stop_loss': s.stop_loss, 'take_profit': s.take_profit}}))
"#,
        sdk_dir = SDK_DIR
    );

    let raw = run_python(&python, &script, &proto_bytes);
    let d: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(
        d["stop_loss"].as_u64().unwrap(),
        0,
        "explicit Some(0) stop_loss must decode as 0, not null"
    );
    assert_eq!(d["take_profit"].as_u64().unwrap(), 999_999);
}
