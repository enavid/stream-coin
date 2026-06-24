/// Playback state machine for the chart page's scrub-through-a-backtest
/// feature (Loop 6i). Pure Rust, no Dioxus signals — the `Signal` wrapper
/// lives in `AppState`; this struct is the unit-testable logic underneath.
///
/// `cursor_time` is an RFC3339 string that acts as the playback head: the
/// chart only renders trades whose `exit_time <= cursor_time`. `advance`
/// steps it forward one candle at a time; the chart's `use_effect` uses a
/// JS-driven `setTimeout` to call it repeatedly while `playing` is `true`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackState {
    /// RFC3339 timestamp of the candle the cursor is currently at. An empty
    /// string means "before any candle" — the first `advance` call moves to
    /// `candle_times[0]`.
    pub cursor_time: String,
    pub speed: PlaybackSpeed,
    pub playing: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            cursor_time: String::new(),
            speed: PlaybackSpeed::OneX,
            playing: false,
        }
    }
}

impl PlaybackState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Steps `cursor_time` to the next entry in `candle_times` (the caller
    /// provides the full sorted slice of the currently loaded series).
    /// Returns `true` if the cursor moved, `false` if it was already at the
    /// last entry (end of range).
    pub fn advance(&mut self, candle_times: &[String]) -> bool {
        let next = candle_times
            .iter()
            .find(|t| t.as_str() > self.cursor_time.as_str());
        match next {
            Some(t) => {
                self.cursor_time = t.clone();
                true
            }
            None => false,
        }
    }

    /// Steps `cursor_time` back to the previous entry in `candle_times`.
    /// Returns `true` if the cursor moved, `false` if already at the first
    /// (or before the first) entry.
    pub fn retreat(&mut self, candle_times: &[String]) -> bool {
        let prev = candle_times
            .iter()
            .rev()
            .find(|t| t.as_str() < self.cursor_time.as_str());
        match prev {
            Some(t) => {
                self.cursor_time = t.clone();
                true
            }
            None => false,
        }
    }

    /// The current cursor position as a 0-based index into `candle_times`,
    /// or `None` when the cursor hasn't entered the range yet (empty
    /// `cursor_time`) or no candles are loaded.
    pub fn current_index(&self, candle_times: &[String]) -> Option<usize> {
        if self.cursor_time.is_empty() {
            return None;
        }
        candle_times
            .iter()
            .position(|t| t == &self.cursor_time)
    }

    /// Jumps the cursor directly to `candle_times[index]`. Does nothing if
    /// `index` is out of bounds.
    pub fn seek_to(&mut self, candle_times: &[String], index: usize) {
        if let Some(t) = candle_times.get(index) {
            self.cursor_time = t.clone();
        }
    }
}

/// Controls how many milliseconds elapse between consecutive `advance`
/// calls during playback. `OneX` matches TradingView's own replay default
/// (~1 candle per 150 ms); each higher level halves the interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackSpeed {
    OneX,
    TwoX,
    FiveX,
    TenX,
}

impl PlaybackSpeed {
    /// The delay between advance ticks in milliseconds.
    pub fn interval_ms(self) -> u64 {
        match self {
            Self::OneX  => 150,
            Self::TwoX  => 75,
            Self::FiveX => 30,
            Self::TenX  => 15,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OneX  => "1×",
            Self::TwoX  => "2×",
            Self::FiveX => "5×",
            Self::TenX  => "10×",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(ts: &[&str]) -> Vec<String> {
        ts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn playback_advance_steps_to_next_candle_time() {
        let candles = times(&[
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
            "2026-01-01T00:02:00Z",
        ]);
        let mut state = PlaybackState::new();

        let moved = state.advance(&candles);

        assert!(moved, "advance on an empty cursor must move to the first candle");
        assert_eq!(state.cursor_time, "2026-01-01T00:00:00Z");

        let moved2 = state.advance(&candles);
        assert!(moved2);
        assert_eq!(state.cursor_time, "2026-01-01T00:01:00Z");
    }

    #[test]
    fn playback_advance_returns_false_at_end_of_range() {
        let candles = times(&["2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z"]);
        let mut state = PlaybackState {
            cursor_time: "2026-01-01T00:01:00Z".to_string(),
            ..Default::default()
        };

        let moved = state.advance(&candles);

        assert!(
            !moved,
            "advance at the last candle must return false and leave cursor unchanged"
        );
        assert_eq!(state.cursor_time, "2026-01-01T00:01:00Z");
    }

    #[test]
    fn playback_speed_multiplies_step_interval() {
        let base = PlaybackSpeed::OneX.interval_ms();
        assert_eq!(base, 150, "1× baseline must be 150 ms");
        assert_eq!(PlaybackSpeed::TwoX.interval_ms(), base / 2);
        assert_eq!(PlaybackSpeed::FiveX.interval_ms(), base / 5);
        assert_eq!(PlaybackSpeed::TenX.interval_ms(), base / 10);
    }

    #[test]
    fn playback_retreat_steps_to_previous_candle_time() {
        let candles = times(&[
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
            "2026-01-01T00:02:00Z",
        ]);
        let mut state = PlaybackState {
            cursor_time: "2026-01-01T00:02:00Z".to_string(),
            ..Default::default()
        };

        let moved = state.retreat(&candles);
        assert!(moved);
        assert_eq!(state.cursor_time, "2026-01-01T00:01:00Z");
    }

    #[test]
    fn playback_retreat_returns_false_at_start_of_range() {
        let candles = times(&["2026-01-01T00:00:00Z"]);
        let mut state = PlaybackState {
            cursor_time: "2026-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };

        assert!(!state.retreat(&candles));
    }

    #[test]
    fn playback_current_index_returns_none_before_any_advance() {
        let candles = times(&["2026-01-01T00:00:00Z"]);
        let state = PlaybackState::new();
        assert!(state.current_index(&candles).is_none());
    }

    #[test]
    fn playback_current_index_returns_position_of_cursor() {
        let candles = times(&["2026-01-01T00:00:00Z", "2026-01-01T00:01:00Z"]);
        let mut state = PlaybackState::new();
        state.advance(&candles);
        assert_eq!(state.current_index(&candles), Some(0));
        state.advance(&candles);
        assert_eq!(state.current_index(&candles), Some(1));
    }

    #[test]
    fn playback_seek_to_jumps_cursor_to_index() {
        let candles = times(&[
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
            "2026-01-01T00:02:00Z",
        ]);
        let mut state = PlaybackState::new();
        state.seek_to(&candles, 2);
        assert_eq!(state.cursor_time, "2026-01-01T00:02:00Z");
    }

    #[test]
    fn playback_seek_to_out_of_bounds_does_nothing() {
        let candles = times(&["2026-01-01T00:00:00Z"]);
        let mut state = PlaybackState::new();
        state.seek_to(&candles, 99);
        assert!(state.cursor_time.is_empty(), "out-of-bounds seek must not change cursor");
    }
}
