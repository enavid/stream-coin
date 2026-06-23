//! Pure WS-reconnect backoff math, shared by every platform's transport
//! (today just `ui/web/src/ws.rs`) — kept here instead of in the platform
//! crate so it's unit-testable on the host target without a browser
//! runtime, same reasoning as `theme.rs`.

/// Exponential backoff starting at 500ms, capped at 30s, with up to 30%
/// jitter on top of the base delay — matches `ROADMAP.md`'s API standard:
/// "WebSocket clients reconnect with exponential backoff starting at
/// 500ms, capped at 30 seconds, with jitter."
///
/// `attempt` is 0-indexed (0 = the first reconnect attempt after a
/// disconnect). `random_unit` must be in `[0.0, 1.0)`; the caller supplies
/// real randomness (e.g. `js_sys::Math::random()`) so this function stays
/// deterministic and testable.
pub fn reconnect_delay_ms(attempt: u32, random_unit: f64) -> u32 {
    let base = 500u64.saturating_mul(1u64 << attempt.min(6)).min(30_000) as u32;
    let jitter = (f64::from(base) * 0.3 * random_unit.clamp(0.0, 1.0)) as u32;
    (base + jitter).min(30_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_delay_ms_starts_at_500ms_on_first_attempt_with_no_jitter() {
        assert_eq!(reconnect_delay_ms(0, 0.0), 500);
    }

    #[test]
    fn reconnect_delay_ms_doubles_with_each_attempt() {
        assert_eq!(reconnect_delay_ms(1, 0.0), 1000);
        assert_eq!(reconnect_delay_ms(2, 0.0), 2000);
        assert_eq!(reconnect_delay_ms(3, 0.0), 4000);
    }

    #[test]
    fn reconnect_delay_ms_caps_at_30_seconds_for_large_attempts() {
        assert_eq!(reconnect_delay_ms(20, 1.0), 30_000);
    }

    #[test]
    fn reconnect_delay_ms_adds_up_to_30_percent_jitter() {
        let with_jitter = reconnect_delay_ms(0, 1.0);
        assert_eq!(with_jitter, 650, "500ms base + 30% jitter at random_unit=1.0");
    }

    #[test]
    fn reconnect_delay_ms_clamps_out_of_range_random_unit() {
        assert_eq!(reconnect_delay_ms(0, -5.0), 500);
        assert_eq!(reconnect_delay_ms(0, 5.0), 650);
    }
}
