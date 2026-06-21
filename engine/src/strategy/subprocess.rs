use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;

struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

use crate::infrastructure::db::signal_repository::{SignalRecord, SignalRepository};
use crate::wire_message::{SignalPayload, WsMessage};

pub struct SubprocessConfig {
    pub strategy_id: String,
    pub code: String,
}

/// Spawns a Python strategy subprocess bridged to the engine broadcaster.
///
/// Candle events from the broadcaster are fed line-by-line to the subprocess's stdin.
/// Signal JSON lines the subprocess writes to stdout are broadcast as `WsMessage::Signal`.
/// On Linux, a seccomp filter blocking socket/connect/bind is applied inside the
/// Python process via ctypes before any user code runs.
///
/// Aborting the returned handle kills the subprocess (via `kill_on_drop`).
pub fn spawn_subprocess_runner(
    config: SubprocessConfig,
    broadcaster: broadcast::Sender<String>,
    signal_repository: Option<Arc<dyn SignalRepository>>,
) -> AbortHandle {
    let handle = tokio::spawn(run_subprocess(config, broadcaster, signal_repository));
    handle.abort_handle()
}

/// Python seccomp preamble injected before user strategy code on Linux x86_64/aarch64.
/// Applies a BPF filter that denies socket(), connect(), and bind() with EACCES.
/// All other syscalls are allowed so Python's runtime and threading continue to work.
#[cfg(target_os = "linux")]
const SECCOMP_PREAMBLE: &str = r#"
import sys as _sys
import platform as _platform

def _sc_setup():
    _machine = _platform.machine()
    _nr_map = {
        'x86_64':  (_sys_socket := 41,  _sys_connect := 42,  _sys_bind := 49),
        'aarch64': (_sys_socket := 198, _sys_connect := 203, _sys_bind := 200),
    }
    if _machine not in _nr_map:
        print(f'seccomp: unknown arch {_machine}, skipping', file=_sys.stderr)
        return
    _SYS_SOCKET, _SYS_CONNECT, _SYS_BIND = _nr_map[_machine]
    try:
        import ctypes as _ct
        _libc = _ct.CDLL(None, use_errno=True)
        _BPF_LD=0x00;_BPF_W=0x00;_BPF_ABS=0x20;_BPF_JMP=0x05;_BPF_JEQ=0x10;_BPF_K=0x00;_BPF_RET=0x06
        _SECCOMP_RET_ERRNO=0x00050000;_SECCOMP_RET_ALLOW=0x7fff0000;_EACCES=13
        class _SF(_ct.Structure):
            _fields_=[('code',_ct.c_uint16),('jt',_ct.c_uint8),('jf',_ct.c_uint8),('k',_ct.c_uint32)]
        class _FP(_ct.Structure):
            _fields_=[('len',_ct.c_uint16),('filter',_ct.POINTER(_SF))]
        def _bs(c,k): return _SF(c,0,0,k)
        def _bj(c,k,jt,jf): return _SF(c,jt,jf,k)
        _f=(_SF*8)(
            _bs(_BPF_LD|_BPF_W|_BPF_ABS,0),
            _bj(_BPF_JMP|_BPF_JEQ|_BPF_K,_SYS_SOCKET,0,1),
            _bs(_BPF_RET|_BPF_K,_SECCOMP_RET_ERRNO|_EACCES),
            _bj(_BPF_JMP|_BPF_JEQ|_BPF_K,_SYS_CONNECT,0,1),
            _bs(_BPF_RET|_BPF_K,_SECCOMP_RET_ERRNO|_EACCES),
            _bj(_BPF_JMP|_BPF_JEQ|_BPF_K,_SYS_BIND,0,1),
            _bs(_BPF_RET|_BPF_K,_SECCOMP_RET_ERRNO|_EACCES),
            _bs(_BPF_RET|_BPF_K,_SECCOMP_RET_ALLOW),
        )
        _prog=_FP(8,_f)
        _libc.prctl(38,1,0,0,0)
        if _libc.prctl(22,2,_ct.byref(_prog),0,0)!=0:
            raise OSError(_ct.get_errno(),'prctl(PR_SET_SECCOMP) failed')
    except Exception as _e:
        print(f'seccomp: {_e}', file=_sys.stderr)

_sc_setup()
del _sc_setup
"#;

#[cfg(not(target_os = "linux"))]
const SECCOMP_PREAMBLE: &str = "";

fn build_launcher_script(strategy_id: &str, user_code: &str) -> String {
    // strategy_id is a UUID — safe to embed in a Python single-quoted string literal
    format!(
        "import os as _os\n_STRATEGY_ID = _os.environ.get('STRATEGY_ID', '{strategy_id}')\n{preamble}\n{user_code}\n",
        strategy_id = strategy_id,
        preamble = SECCOMP_PREAMBLE,
        user_code = user_code,
    )
}

async fn run_subprocess(
    config: SubprocessConfig,
    broadcaster: broadcast::Sender<String>,
    signal_repository: Option<Arc<dyn SignalRepository>>,
) {
    let script_path = std::env::temp_dir().join(format!("strategy_{}.py", config.strategy_id));
    let full_script = build_launcher_script(&config.strategy_id, &config.code);

    if let Err(e) = tokio::fs::write(&script_path, full_script.as_bytes()).await {
        tracing::error!(
            error = %e,
            strategy_id = %config.strategy_id,
            "failed to write python strategy script"
        );
        return;
    }

    let mut cmd = Command::new("python3");
    cmd.arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("STRATEGY_ID", &config.strategy_id)
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                error = %e,
                strategy_id = %config.strategy_id,
                path = ?script_path,
                "failed to spawn python3 subprocess"
            );
            let _ = tokio::fs::remove_file(&script_path).await;
            return;
        }
    };

    tracing::info!(
        strategy_id = %config.strategy_id,
        "python strategy subprocess started"
    );

    let stdin = child.stdin.take().expect("stdin must be piped");
    let stdout = child.stdout.take().expect("stdout must be piped");
    let stderr = child.stderr.take().expect("stderr must be piped");

    let signals_topic =
        std::env::var("KAFKA_TOPIC_SIGNALS").unwrap_or_else(|_| "signals".to_string());

    // Feed candle events to subprocess stdin
    let mut rx = broadcaster.subscribe();
    let mut stdin_writer = stdin;
    let stdin_task = tokio::spawn(async move {
        loop {
            let text = match rx.recv().await {
                Ok(t) => t,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        dropped = n,
                        "subprocess stdin writer lagged — candle events dropped"
                    );
                    continue;
                }
                Err(_) => break,
            };

            let msg: WsMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if let WsMessage::Candle(candle) = msg {
                let line = match serde_json::to_string(&candle) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize candle for subprocess");
                        continue;
                    }
                };
                let mut bytes = line.into_bytes();
                bytes.push(b'\n');
                if let Err(e) = stdin_writer.write_all(&bytes).await {
                    tracing::debug!(
                        error = %e,
                        "subprocess stdin write error — process likely exited"
                    );
                    break;
                }
                if let Err(e) = stdin_writer.flush().await {
                    tracing::debug!(error = %e, "subprocess stdin flush error");
                    break;
                }
            }
        }
    });

    // Read signal JSON lines from subprocess stdout and broadcast them
    let strategy_id = config.strategy_id.clone();
    let broadcaster_clone = broadcaster.clone();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let payload: SignalPayload = match serde_json::from_str(&line) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        strategy_id = %strategy_id,
                        raw = %line,
                        "subprocess produced unparseable output — expected SignalPayload JSON"
                    );
                    continue;
                }
            };

            tracing::info!(
                signal_id = %payload.signal_id,
                strategy_id = %payload.strategy_id,
                exchange = %payload.exchange,
                pair = %payload.pair,
                action = %payload.action,
                confidence = payload.confidence,
                timestamp = %payload.timestamp,
                "signal emitted by python subprocess"
            );

            match serde_json::to_string(&WsMessage::Signal(payload.clone())) {
                Ok(json) => {
                    let _ = broadcaster_clone.send(json.clone());
                    // Kafka publish is handled by the order manager listener consuming the broadcaster
                    let _ = signals_topic.as_str(); // referenced to avoid unused warning
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        signal_id = %payload.signal_id,
                        "failed to serialize subprocess signal for broadcast"
                    );
                }
            }

            if let Some(ref repo) = signal_repository {
                let record = SignalRecord {
                    signal_id: payload.signal_id.clone(),
                    strategy_id: payload.strategy_id.clone(),
                    exchange: payload.exchange.clone(),
                    pair: payload.pair.clone(),
                    action: payload.action.clone(),
                    confidence: payload.confidence,
                    created_at: payload.timestamp,
                };
                if let Err(e) = repo.save(&record).await {
                    tracing::error!(
                        error = %e,
                        signal_id = %payload.signal_id,
                        "failed to persist subprocess signal to db"
                    );
                }
            }
        }
    });

    // Log subprocess stderr at DEBUG level
    let strategy_id_err = config.strategy_id.clone();
    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(
                strategy_id = %strategy_id_err,
                stderr = %line,
                "python subprocess"
            );
        }
    });

    // Abort all sub-tasks when this future is dropped (e.g., on outer task abort).
    let _stdin_guard = AbortOnDrop(stdin_task.abort_handle());
    let _stdout_guard = AbortOnDrop(stdout_task.abort_handle());
    let _stderr_guard = AbortOnDrop(stderr_task.abort_handle());

    match child.wait().await {
        Ok(status) => {
            tracing::info!(
                strategy_id = %config.strategy_id,
                exit_code = ?status.code(),
                "python subprocess exited"
            );
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                strategy_id = %config.strategy_id,
                "python subprocess wait error"
            );
        }
    }

    stdin_task.abort();
    stdout_task.abort();
    stderr_task.abort();
    let _ = tokio::fs::remove_file(&script_path).await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use tokio::sync::broadcast;

    use super::*;
    use crate::candle::entity::CandlePayload;
    use crate::wire_message::WsMessage;

    fn make_candle_msg() -> String {
        let candle = WsMessage::Candle(CandlePayload {
            exchange: "tabdeal".to_string(),
            pair: "USDT/IRT".to_string(),
            interval: "1m".to_string(),
            time: Utc::now(),
            open: 58_000,
            high: 58_500,
            low: 57_800,
            close: 58_200,
            volume: 100,
        });
        serde_json::to_string(&candle).unwrap()
    }

    /// Python code that reads one candle from stdin and writes one signal to stdout.
    const ECHO_SIGNAL_CODE: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    candle = json.loads(line)
    print(json.dumps({
        "signal_id": str(uuid.uuid4()),
        "strategy_id": "test-sub",
        "exchange": candle["exchange"],
        "pair": candle["pair"],
        "action": "buy",
        "confidence": 0.9,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }), flush=True)
    break
"#;

    #[tokio::test(flavor = "current_thread")]
    async fn subprocess_starts_and_receives_candle() {
        let (tx, mut rx) = broadcast::channel(16);

        let handle = spawn_subprocess_runner(
            SubprocessConfig {
                strategy_id: "test-recv-candle".to_string(),
                code: ECHO_SIGNAL_CODE.to_string(),
            },
            tx.clone(),
            None,
        );

        // Allow subprocess to start before sending the candle
        tokio::time::sleep(Duration::from_millis(400)).await;
        tx.send(make_candle_msg()).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match rx.recv().await {
                    Ok(text) => {
                        if let Ok(WsMessage::Signal(sig)) = serde_json::from_str::<WsMessage>(&text)
                        {
                            return sig;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => panic!("broadcaster closed unexpectedly"),
                }
            }
        })
        .await
        .expect("subprocess must emit a signal within the deadline");

        assert_eq!(result.action, "buy");
        assert_eq!(result.exchange, "tabdeal");
        assert_eq!(result.pair, "USDT/IRT");
        assert!(!result.signal_id.is_empty());

        handle.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn subprocess_killed_on_stop() {
        let (tx, _) = broadcast::channel::<String>(16);

        let handle = spawn_subprocess_runner(
            SubprocessConfig {
                strategy_id: "test-kill".to_string(),
                code: "import time\nwhile True:\n    time.sleep(0.1)\n".to_string(),
            },
            tx.clone(),
            None,
        );

        // Wait for the subprocess task to subscribe to the broadcaster
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            tx.receiver_count(),
            1,
            "subprocess must have subscribed to broadcaster"
        );

        handle.abort();

        // After abort, the stdin_task (and its receiver) must be dropped
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            tx.receiver_count(),
            0,
            "broadcaster subscription must be released after abort"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signal_from_subprocess_reaches_order_manager() {
        use crate::infrastructure::db::order_repository::FakeOrderRepository;
        use crate::order::entity::SafetyConfig;
        use crate::order::manager::{spawn_order_manager_listener, OrderManager};
        use crate::order::port::OrderAdapter;
        use rust_decimal::Decimal;
        use std::collections::HashMap;

        let (tx, _) = broadcast::channel::<String>(32);

        // Order manager in dry_run so no real exchange calls are made
        let cfg = SafetyConfig {
            dry_run: true,
            min_confidence: 0.5,
            max_position_size: Decimal::new(10_000, 0),
            default_order_quantity: Decimal::new(100, 0),
            circuit_breaker_max_orders: 100,
            circuit_breaker_window_secs: 60,
        };
        let repo = Arc::new(FakeOrderRepository::new());
        let adapters: HashMap<String, Arc<dyn OrderAdapter>> = HashMap::new();
        let manager = Arc::new(OrderManager::new(
            Arc::new(adapters),
            repo.clone(),
            tx.clone(),
            cfg,
        ));
        let _om_handle = spawn_order_manager_listener(Arc::clone(&manager), tx.clone());

        // Spawn subprocess that emits a buy signal when it receives a candle
        let _sub_handle = spawn_subprocess_runner(
            SubprocessConfig {
                strategy_id: "test-om-bridge".to_string(),
                code: ECHO_SIGNAL_CODE.to_string(),
            },
            tx.clone(),
            None,
        );

        // Wait for subprocess to start
        tokio::time::sleep(Duration::from_millis(400)).await;
        tx.send(make_candle_msg()).unwrap();

        // Wait for the order to be persisted
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if tokio::time::Instant::now() > deadline {
                panic!("order manager did not process signal within deadline");
            }
            let records = repo.all_records().await;
            if !records.is_empty() {
                assert_eq!(records[0].status, "dry_run");
                assert_eq!(records[0].side, "buy");
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn seccomp_blocks_network_call_from_strategy() {
        // "buy" means socket() was blocked (seccomp working)
        // "sell" means socket() succeeded (seccomp did NOT block — test fails)
        const CODE: &str = r#"
import sys, json, uuid
from datetime import datetime, timezone
try:
    import socket
    socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    action = "sell"
except PermissionError:
    action = "buy"
print(json.dumps({
    "signal_id": str(uuid.uuid4()),
    "strategy_id": "test-seccomp",
    "exchange": "test",
    "pair": "TEST/IRT",
    "action": action,
    "confidence": 0.9,
    "timestamp": datetime.now(timezone.utc).isoformat(),
}), flush=True)
"#;

        let (tx, mut rx) = broadcast::channel(16);
        let _handle = spawn_subprocess_runner(
            SubprocessConfig {
                strategy_id: "test-seccomp".to_string(),
                code: CODE.to_string(),
            },
            tx,
            None,
        );

        let action = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(text) => {
                        if let Ok(WsMessage::Signal(sig)) = serde_json::from_str::<WsMessage>(&text)
                        {
                            return sig.action;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return "error".to_string(),
                }
            }
        })
        .await
        .unwrap_or_else(|_| "timeout".to_string());

        assert_eq!(
            action, "buy",
            "seccomp must block socket() — strategy emits 'sell' if socket succeeds"
        );
    }
}
