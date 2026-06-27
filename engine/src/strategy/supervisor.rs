//! Restart supervision for long-lived strategy tasks (M10).
//!
//! A strategy runner or Python subprocess that died — the process crashed, the
//! task panicked, or its work future simply returned — previously just stopped:
//! the spawned task ended and no more signals were emitted, with nothing logging
//! the death or bringing it back. For a trading engine that silent stop means a
//! strategy goes dark indefinitely.
//!
//! [`spawn_supervised`] runs a work future under a supervisor that restarts it
//! with exponential backoff whenever it exits or panics, capping the retry rate
//! so a permanently-broken strategy can't hot-loop. The child runs in its own
//! task so a panic is isolated, and is aborted if the supervisor itself is
//! aborted (so the underlying subprocess, killed on drop, doesn't leak).

use std::future::Future;
use std::time::Duration;

use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;

/// Exponential-backoff schedule for restarts.
#[derive(Clone, Copy, Debug)]
pub struct BackoffPolicy {
    /// Delay before the first restart; doubles each consecutive failure.
    pub base: Duration,
    /// Upper bound on the restart delay.
    pub max: Duration,
    /// If a run stayed alive at least this long it is treated as healthy and the
    /// backoff resets — a strategy that runs for an hour then dies should retry
    /// fast, not inherit the long delay from an earlier crash burst.
    pub reset_after: Duration,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
            reset_after: Duration::from_secs(30),
        }
    }
}

impl BackoffPolicy {
    pub fn new(base: Duration, max: Duration, reset_after: Duration) -> Self {
        Self {
            base,
            max,
            reset_after,
        }
    }

    /// Delay before the restart following the `attempt`-th consecutive failure
    /// (0-based): `base * 2^attempt`, capped at `max`, overflow-safe.
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
        self.base
            .checked_mul(factor.min(u32::MAX as u64) as u32)
            .unwrap_or(self.max)
            .min(self.max)
    }
}

/// Guards a child task: aborts it if the supervisor task is dropped/cancelled,
/// so a supervised subprocess (killed on drop) never outlives its supervisor.
struct AbortOnDrop(AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawns `factory`'s future under a restart supervisor and returns the
/// supervisor's [`AbortHandle`]. Each time the work future completes or panics,
/// it is restarted after a [`BackoffPolicy`] delay. Aborting the returned handle
/// stops supervision and the in-flight child.
pub fn spawn_supervised<F, Fut>(name: String, policy: BackoffPolicy, mut factory: F) -> AbortHandle
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let handle: JoinHandle<()> = tokio::spawn(async move {
        let mut attempt: u32 = 0;
        loop {
            let started = Instant::now();
            let child = tokio::spawn(factory());
            let guard = AbortOnDrop(child.abort_handle());

            match child.await {
                Ok(()) => {
                    tracing::warn!(strategy = %name, "supervised task exited — will restart");
                }
                Err(e) if e.is_cancelled() => {
                    // Only the guard cancels the child, which happens when the
                    // supervisor itself is being dropped — stop, don't restart.
                    break;
                }
                Err(e) => {
                    tracing::error!(strategy = %name, error = %e, "supervised task panicked — will restart");
                }
            }
            drop(guard);

            // A run that stayed healthy long enough resets the backoff.
            if started.elapsed() >= policy.reset_after {
                attempt = 0;
            }
            let delay = policy.delay_for(attempt);
            attempt = attempt.saturating_add(1);
            tracing::error!(
                strategy = %name,
                restart_attempt = attempt,
                delay_ms = delay.as_millis() as u64,
                "restarting dead strategy task after backoff"
            );
            tokio::time::sleep(delay).await;
        }
    });
    handle.abort_handle()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let p = BackoffPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        assert_eq!(p.delay_for(0), Duration::from_secs(1));
        assert_eq!(p.delay_for(1), Duration::from_secs(2));
        assert_eq!(p.delay_for(2), Duration::from_secs(4));
        assert_eq!(p.delay_for(3), Duration::from_secs(8));
        assert_eq!(p.delay_for(4), Duration::from_secs(10), "capped at max");
        assert_eq!(
            p.delay_for(100),
            Duration::from_secs(10),
            "huge attempt stays capped"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn dead_strategy_is_restarted_with_backoff() {
        // A work future that dies immediately every time. The supervisor must
        // keep bringing it back — but rate-limited by the backoff, not hot-looping.
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let policy = BackoffPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(60),
            Duration::from_secs(30),
        );

        let handle = spawn_supervised("test-strategy".to_string(), policy, move || {
            let r = r.clone();
            async move {
                r.fetch_add(1, Ordering::SeqCst);
                // returns immediately = "died"
            }
        });

        // With the clock paused, sleeps auto-advance once the runtime is idle, so
        // several backoff cycles elapse across this wait window.
        tokio::time::sleep(Duration::from_secs(20)).await;
        handle.abort();

        let n = runs.load(Ordering::SeqCst);
        assert!(
            n >= 3,
            "a dead strategy must be restarted multiple times (got {n})"
        );
        // base 1s doubling (1,2,4,8,16,...) → at most ~6 starts in 20s. If backoff
        // were not applied this would be effectively unbounded.
        assert!(
            n <= 8,
            "restarts must be backoff-limited, not a hot loop (got {n})"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn supervisor_abort_stops_restarts() {
        let runs = Arc::new(AtomicUsize::new(0));
        let r = runs.clone();
        let handle = spawn_supervised(
            "test-abort".to_string(),
            BackoffPolicy::new(
                Duration::from_secs(1),
                Duration::from_secs(60),
                Duration::from_secs(30),
            ),
            move || {
                let r = r.clone();
                async move {
                    r.fetch_add(1, Ordering::SeqCst);
                }
            },
        );

        tokio::time::sleep(Duration::from_secs(5)).await;
        handle.abort();
        let after_abort = runs.load(Ordering::SeqCst);

        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            after_abort,
            "no further restarts must happen after the supervisor is aborted"
        );
    }
}
