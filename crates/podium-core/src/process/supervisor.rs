//! Auto-restart policy machinery: exponential backoff and a circuit breaker.
//!
//! The pure bookkeeping lives here ([`RestartState`], per process); the
//! orchestrator owns the actual scheduling (a tokio sleep task per pending
//! restart). Timings are injectable via [`SupervisorConfig`] so tests can run
//! with millisecond backoffs.

use std::time::{Duration, Instant};

use tokio::task::AbortHandle;

/// Timing knobs for the supervisor. Defaults: 500ms → 30s exponential
/// backoff, breaker at 5 restarts per rolling 60s window, backoff reset once
/// a process stays running for 60s.
#[derive(Debug, Clone, Copy)]
pub struct SupervisorConfig {
    /// First restart delay; doubles on each attempt.
    pub backoff_base: Duration,
    /// Upper bound on the restart delay.
    pub backoff_cap: Duration,
    /// Rolling window the circuit breaker counts restarts in.
    pub breaker_window: Duration,
    /// Max restarts within the window before the supervisor gives up.
    pub breaker_max_restarts: usize,
    /// A process running at least this long resets the backoff to the base.
    pub backoff_reset_after: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            backoff_base: Duration::from_millis(500),
            backoff_cap: Duration::from_secs(30),
            breaker_window: Duration::from_secs(60),
            breaker_max_restarts: 5,
            backoff_reset_after: Duration::from_secs(60),
        }
    }
}

/// Per-process restart bookkeeping: backoff attempt counter, restart
/// timestamps for the breaker window, and the pending restart task (if any).
///
/// `generation` guards against races: it is bumped by every schedule and
/// every cancel, and a scheduled restart only fires if the generation it
/// captured is still current.
#[derive(Debug, Default)]
pub(crate) struct RestartState {
    attempt: u32,
    recent: Vec<Instant>,
    generation: u64,
    pending: Option<AbortHandle>,
}

impl RestartState {
    /// Ask to schedule a restart at `now`. Returns the backoff delay and the
    /// new generation, or `None` when the circuit breaker trips (too many
    /// restarts within the rolling window).
    pub(crate) fn try_schedule(
        &mut self,
        now: Instant,
        config: &SupervisorConfig,
    ) -> Option<(Duration, u64)> {
        self.recent
            .retain(|t| now.duration_since(*t) < config.breaker_window);
        if self.recent.len() >= config.breaker_max_restarts {
            return None;
        }
        let exp = 1u32.checked_shl(self.attempt).unwrap_or(u32::MAX);
        let delay = config
            .backoff_base
            .saturating_mul(exp)
            .min(config.backoff_cap);
        self.attempt = self.attempt.saturating_add(1);
        self.recent.push(now);
        self.generation += 1;
        Some((delay, self.generation))
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Record the spawned sleep task so it can be aborted on cancel.
    pub(crate) fn set_pending(&mut self, handle: AbortHandle) {
        self.pending = Some(handle);
    }

    /// Detach the pending handle without aborting (the task is about to run).
    pub(crate) fn clear_pending(&mut self) {
        self.pending = None;
    }

    /// Cancel any pending restart and invalidate its generation. Returns
    /// whether a restart was actually pending.
    pub(crate) fn cancel(&mut self) -> bool {
        self.generation += 1;
        match self.pending.take() {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    /// Reset the backoff to the base delay (process ran long enough).
    pub(crate) fn reset_backoff(&mut self) {
        self.attempt = 0;
    }

    /// Full reset (manual start): backoff back to base, breaker window clear.
    pub(crate) fn reset(&mut self) {
        self.attempt = 0;
        self.recent.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SupervisorConfig {
        SupervisorConfig {
            backoff_base: Duration::from_millis(500),
            backoff_cap: Duration::from_secs(30),
            breaker_window: Duration::from_secs(60),
            breaker_max_restarts: 5,
            backoff_reset_after: Duration::from_secs(60),
        }
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let cfg = config();
        let mut s = RestartState::default();
        let now = Instant::now();
        let mut delays = Vec::new();
        for _ in 0..5 {
            let (delay, _) = s.try_schedule(now, &cfg).unwrap();
            delays.push(delay);
            s.recent.clear(); // keep the breaker out of this test
        }
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
            ]
        );
        s.attempt = 10; // 500ms * 2^10 = 512s → capped
        let (delay, _) = s.try_schedule(now, &cfg).unwrap();
        assert_eq!(delay, Duration::from_secs(30));
    }

    #[test]
    fn breaker_trips_after_max_restarts_in_window() {
        let cfg = config();
        let mut s = RestartState::default();
        let now = Instant::now();
        for _ in 0..5 {
            assert!(s.try_schedule(now, &cfg).is_some());
        }
        assert!(s.try_schedule(now, &cfg).is_none(), "6th restart denied");
    }

    #[test]
    fn breaker_window_is_rolling() {
        let cfg = config();
        let mut s = RestartState::default();
        let old = Instant::now();
        let later = old + cfg.breaker_window + Duration::from_secs(1);
        for _ in 0..5 {
            assert!(s.try_schedule(old, &cfg).is_some());
        }
        assert!(s.try_schedule(old, &cfg).is_none());
        // Outside the window the old restarts no longer count.
        assert!(s.try_schedule(later, &cfg).is_some());
    }

    #[test]
    fn cancel_bumps_generation_and_reports_pending() {
        let mut s = RestartState::default();
        let gen_before = s.generation();
        assert!(!s.cancel(), "nothing pending yet");
        assert_eq!(s.generation(), gen_before + 1);
    }

    #[test]
    fn reset_clears_backoff_and_window() {
        let cfg = config();
        let mut s = RestartState::default();
        let now = Instant::now();
        for _ in 0..5 {
            s.try_schedule(now, &cfg);
        }
        assert!(s.try_schedule(now, &cfg).is_none());
        s.reset();
        let (delay, _) = s.try_schedule(now, &cfg).unwrap();
        assert_eq!(delay, cfg.backoff_base, "backoff back to base");
    }

    #[test]
    fn reset_backoff_keeps_breaker_window() {
        let cfg = config();
        let mut s = RestartState::default();
        let now = Instant::now();
        for _ in 0..5 {
            s.try_schedule(now, &cfg);
        }
        s.reset_backoff();
        assert!(
            s.try_schedule(now, &cfg).is_none(),
            "breaker window survives a backoff reset"
        );
    }
}
