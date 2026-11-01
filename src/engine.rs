//! The state machine that decides when to send an impulse. It knows neither
//! the clock nor the device: the caller passes the local time of day and a
//! monotonic time, and does what the [`Decision`] says. That keeps every rule
//! testable with fixed moments.

use crate::config::Config;
use crate::schedule::{self, Moment};
use std::time::Duration;

/// The service looks at the clock at least this often, so that a change of the
/// local time (daylight saving time, NTP) or a schedule boundary is noticed
/// quickly.
pub const MAX_SLEEP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// The main switch is off.
    #[default]
    Off,
    /// The switch is on, but the schedule does not allow impulses now.
    Outside,
    /// The longest run is over; a new run starts with the next window or by
    /// switching off and on.
    Paused,
    Sending,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Outside => "outside",
            State::Paused => "paused",
            State::Sending => "sending",
        }
    }
}

/// What the caller has to do after a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub state: State,
    /// Send one impulse now.
    pub send: bool,
    /// Call [`Engine::step`] again after this long at the latest. Never zero
    /// and never longer than [`MAX_SLEEP`].
    pub wake_in: Duration,
}

#[derive(Debug, Default)]
pub struct Engine {
    state: State,
    /// When the current run began; `None` outside a run.
    run_started: Option<Duration>,
    last_impulse: Option<Duration>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// The state after the latest step; `Off` before the first.
    pub fn state(&self) -> State {
        self.state
    }

    /// How long the current run has lasted.
    pub fn run_elapsed(&self, now: Duration) -> Option<Duration> {
        self.run_started.map(|s| now.saturating_sub(s))
    }

    /// Time until the next impulse; only while sending.
    pub fn next_impulse_in(&self, cfg: &Config, now: Duration) -> Option<Duration> {
        if self.state() != State::Sending {
            return None;
        }
        let last = self.last_impulse?;
        let due = last + Duration::from_secs(u64::from(cfg.interval_s));
        Some(due.saturating_sub(now))
    }

    /// Decides what happens at `now` (monotonic, counting suspended time)
    /// and `local` (local day and time).
    pub fn step(&mut self, cfg: &Config, local: Moment, now: Duration) -> Decision {
        if !cfg.enabled {
            return self.idle(State::Off);
        }
        if !schedule::is_active(cfg, local) {
            return self.idle(State::Outside);
        }
        let started = *self.run_started.get_or_insert(now);
        let elapsed = now.saturating_sub(started);
        let limit =
            (cfg.max_hours > 0).then(|| Duration::from_secs(u64::from(cfg.max_hours) * 3600));
        if limit.is_some_and(|limit| elapsed >= limit) {
            self.state = State::Paused;
            return Decision {
                state: State::Paused,
                send: false,
                wake_in: MAX_SLEEP,
            };
        }

        let interval = Duration::from_secs(u64::from(cfg.interval_s));
        let due = self
            .last_impulse
            .is_none_or(|last| now.saturating_sub(last) >= interval);
        if due {
            self.last_impulse = Some(now);
        }
        let last = self.last_impulse.unwrap_or(now);
        let mut wake_in = MAX_SLEEP.min((last + interval).saturating_sub(now));
        if let Some(limit) = limit {
            wake_in = wake_in.min(limit - elapsed);
        }
        self.state = State::Sending;
        Decision {
            state: State::Sending,
            send: due,
            wake_in,
        }
    }

    /// Ends the run and waits for a change of the settings or the time.
    fn idle(&mut self, state: State) -> Decision {
        self.state = state;
        self.run_started = None;
        self.last_impulse = None;
        Decision {
            state,
            send: false,
            wake_in: MAX_SLEEP,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Clock, Day};
    use serde_json::json;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn mins(n: u64) -> Duration {
        secs(n * 60)
    }

    fn at(weekday: u8, hh: u16, mm: u16) -> Moment {
        Moment::new(weekday, hh, mm).unwrap()
    }

    /// Monday noon, inside the default window and outside any other.
    fn noon() -> Moment {
        at(0, 12, 0)
    }

    fn on() -> Config {
        Config::default()
            .merged(json!({ "enabled": true }))
            .unwrap()
    }

    fn with(cfg: &Config, patch: serde_json::Value) -> Config {
        cfg.merged(patch).unwrap()
    }

    #[test]
    fn stays_off_while_the_switch_is_off() {
        let mut e = Engine::new();
        let cfg = Config::default();
        for t in [0, 1, 500, 100_000] {
            let d = e.step(&cfg, noon(), secs(t));
            assert_eq!(d.state, State::Off);
            assert!(!d.send);
            assert_eq!(d.wake_in, MAX_SLEEP);
        }
        assert_eq!(e.state(), State::Off);
        assert_eq!(e.run_elapsed(secs(5)), None);
        assert_eq!(e.next_impulse_in(&cfg, secs(5)), None);
    }

    #[test]
    fn sends_at_once_and_then_every_interval() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "interval_s": 240 }));
        let d = e.step(&cfg, noon(), secs(1000));
        assert_eq!((d.state, d.send), (State::Sending, true));
        assert_eq!(d.wake_in, MAX_SLEEP);
        assert_eq!(e.next_impulse_in(&cfg, secs(1000)), Some(secs(240)));
        // Not yet.
        for t in [1001, 1100, 1239] {
            assert!(!e.step(&cfg, noon(), secs(t)).send, "t={t}");
        }
        assert_eq!(e.next_impulse_in(&cfg, secs(1239)), Some(secs(1)));
        // Exactly one interval after the first.
        assert!(e.step(&cfg, noon(), secs(1240)).send);
        assert!(!e.step(&cfg, noon(), secs(1241)).send);
        assert!(e.step(&cfg, noon(), secs(1480)).send);
        assert_eq!(e.run_elapsed(secs(1480)), Some(secs(480)));
    }

    #[test]
    fn wakes_up_for_the_next_impulse_and_never_later_than_30_seconds() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "interval_s": 240 }));
        assert_eq!(e.step(&cfg, noon(), secs(0)).wake_in, MAX_SLEEP);
        assert_eq!(e.step(&cfg, noon(), secs(100)).wake_in, MAX_SLEEP);
        assert_eq!(e.step(&cfg, noon(), secs(215)).wake_in, secs(25));
        assert_eq!(e.step(&cfg, noon(), secs(235)).wake_in, secs(5));
        // A short interval sets the pace, a long one does not.
        let fast = with(&cfg, json!({ "interval_s": 30 }));
        let mut e = Engine::new();
        assert_eq!(e.step(&fast, noon(), secs(0)).wake_in, secs(30));
        let slow = with(&cfg, json!({ "interval_s": 1800 }));
        let mut e = Engine::new();
        assert_eq!(e.step(&slow, noon(), secs(0)).wake_in, MAX_SLEEP);
        // Outside and paused states are looked at every 30 seconds too.
        let mut e = Engine::new();
        let scheduled = with(&cfg, json!({ "schedule_enabled": true }));
        assert_eq!(
            e.step(&scheduled, at(0, 3, 0), secs(0)),
            Decision {
                state: State::Outside,
                send: false,
                wake_in: MAX_SLEEP
            }
        );
    }

    #[test]
    fn an_overdue_impulse_is_sent_once_and_not_made_up() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "interval_s": 240 }));
        assert!(e.step(&cfg, noon(), secs(0)).send);
        // The computer slept for ten minutes.
        assert!(e.step(&cfg, noon(), mins(10)).send);
        assert!(!e.step(&cfg, noon(), mins(10) + secs(1)).send);
        assert!(!e.step(&cfg, noon(), mins(10) + secs(239)).send);
        assert!(e.step(&cfg, noon(), mins(10) + secs(240)).send);
    }

    #[test]
    fn a_changed_interval_counts_from_the_last_impulse() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "interval_s": 240 }));
        assert!(e.step(&cfg, noon(), secs(0)).send);
        // Shorter than what has passed: due at once.
        let shorter = with(&cfg, json!({ "interval_s": 60 }));
        assert!(e.step(&shorter, noon(), secs(100)).send);
        // Longer: waits for the new interval after the last impulse.
        let longer = with(&cfg, json!({ "interval_s": 600 }));
        assert!(!e.step(&longer, noon(), secs(400)).send);
        assert!(!e.step(&longer, noon(), secs(699)).send);
        assert!(e.step(&longer, noon(), secs(700)).send);
    }

    #[test]
    fn switching_off_ends_the_run_and_switching_on_starts_a_new_one() {
        let mut e = Engine::new();
        let cfg = on();
        assert!(e.step(&cfg, noon(), secs(0)).send);
        assert_eq!(
            e.step(&Config::default(), noon(), secs(10)).state,
            State::Off
        );
        assert_eq!(e.run_elapsed(secs(10)), None);
        // On again: an impulse at once, the run starts from zero.
        let d = e.step(&cfg, noon(), secs(20));
        assert_eq!((d.state, d.send), (State::Sending, true));
        assert_eq!(e.run_elapsed(secs(20)), Some(secs(0)));
    }

    #[test]
    fn follows_the_schedule() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "schedule_enabled": true }));
        // Before the window (08:00-23:00).
        let d = e.step(&cfg, at(0, 7, 59), secs(0));
        assert_eq!((d.state, d.send), (State::Outside, false));
        assert_eq!(e.next_impulse_in(&cfg, secs(0)), None);
        assert_eq!(e.run_elapsed(secs(0)), None);
        // The window opens: an impulse at once.
        let d = e.step(&cfg, at(0, 8, 0), secs(60));
        assert_eq!((d.state, d.send), (State::Sending, true));
        // It closes: no impulse, the run is over.
        let d = e.step(&cfg, at(0, 23, 0), secs(900 * 60));
        assert_eq!((d.state, d.send), (State::Outside, false));
        assert_eq!(e.run_elapsed(secs(900 * 60)), None);
        // Without a schedule the time of day does not matter.
        let always = on();
        let d = e.step(&always, at(0, 3, 0), secs(900 * 60 + 1));
        assert_eq!((d.state, d.send), (State::Sending, true));
    }

    #[test]
    fn follows_a_local_time_that_jumps_back() {
        // Daylight saving time ends: 03:00 becomes 02:00 again.
        let mut e = Engine::new();
        let mut days = [Day {
            on: true,
            from: Clock::parse("02:30").unwrap(),
            to: Clock::parse("04:00").unwrap(),
        }; 7]
            .to_vec();
        days[1].on = false;
        let cfg = Config {
            enabled: true,
            schedule_enabled: true,
            schedule: days,
            ..Config::default()
        };
        assert_eq!(e.step(&cfg, at(0, 2, 29), secs(0)).state, State::Outside);
        assert_eq!(e.step(&cfg, at(0, 3, 0), secs(1800)).state, State::Sending);
        assert_eq!(e.step(&cfg, at(0, 2, 0), secs(1801)).state, State::Outside);
        assert_eq!(e.step(&cfg, at(0, 2, 45), secs(2700)).state, State::Sending);
    }

    #[test]
    fn pauses_after_the_longest_run() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "max_hours": 1, "interval_s": 600 }));
        assert!(e.step(&cfg, noon(), secs(0)).send);
        // The service wakes up just in time for the end of the run.
        let d = e.step(&cfg, noon(), secs(3580));
        assert_eq!(d.state, State::Sending);
        assert_eq!(d.wake_in, secs(20));
        let d = e.step(&cfg, noon(), secs(3599));
        assert_eq!((d.state, d.send), (State::Sending, false));
        assert_eq!(d.wake_in, secs(1));
        // One hour: paused, nothing is sent, not even an overdue impulse.
        let d = e.step(&cfg, noon(), secs(3600));
        assert_eq!((d.state, d.send), (State::Paused, false));
        assert_eq!(d.wake_in, MAX_SLEEP);
        assert!(!e.step(&cfg, noon(), secs(10_000)).send);
        assert_eq!(e.next_impulse_in(&cfg, secs(10_000)), None);
        assert_eq!(e.run_elapsed(secs(10_000)), Some(secs(10_000)));
    }

    #[test]
    fn switching_off_and_on_ends_a_pause() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "max_hours": 1 }));
        e.step(&cfg, noon(), secs(0));
        assert_eq!(e.step(&cfg, noon(), secs(3600)).state, State::Paused);
        e.step(&Config::default(), noon(), secs(3700));
        let d = e.step(&cfg, noon(), secs(3800));
        assert_eq!((d.state, d.send), (State::Sending, true));
        // The new run has its own hour.
        assert_eq!(
            e.step(&cfg, noon(), secs(3800 + 3599)).state,
            State::Sending
        );
        assert_eq!(e.step(&cfg, noon(), secs(3800 + 3600)).state, State::Paused);
    }

    #[test]
    fn a_new_window_ends_a_pause() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "max_hours": 1, "schedule_enabled": true }));
        e.step(&cfg, at(0, 8, 0), secs(0));
        assert_eq!(e.step(&cfg, at(0, 9, 0), secs(3600)).state, State::Paused);
        assert_eq!(
            e.step(&cfg, at(0, 23, 0), secs(54_000)).state,
            State::Outside
        );
        // Next morning.
        let d = e.step(&cfg, at(1, 8, 0), secs(86_400));
        assert_eq!((d.state, d.send), (State::Sending, true));
    }

    #[test]
    fn changing_the_longest_run_applies_to_the_run_that_is_going() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "max_hours": 1, "interval_s": 600 }));
        e.step(&cfg, noon(), secs(0));
        assert_eq!(e.step(&cfg, noon(), secs(3600)).state, State::Paused);
        // No limit any more: sending resumes, the overdue impulse goes out.
        let unlimited = with(&cfg, json!({ "max_hours": 0 }));
        let d = e.step(&unlimited, noon(), secs(3700));
        assert_eq!((d.state, d.send), (State::Sending, true));
        // A limit the run has already exceeded: paused at once.
        let short = with(&cfg, json!({ "max_hours": 2 }));
        let d = e.step(&short, noon(), secs(3701));
        assert_eq!(d.state, State::Sending);
        let tiny = with(&cfg, json!({ "max_hours": 1 }));
        let d = e.step(&tiny, noon(), secs(3702));
        assert_eq!((d.state, d.send), (State::Paused, false));
    }

    #[test]
    fn no_limit_means_the_run_goes_on() {
        let mut e = Engine::new();
        let cfg = with(&on(), json!({ "max_hours": 0, "interval_s": 1800 }));
        e.step(&cfg, noon(), secs(0));
        let day = 24 * 3600;
        assert!(e.step(&cfg, noon(), secs(day)).send);
        assert_eq!(e.step(&cfg, noon(), secs(day + 1)).state, State::Sending);
    }
}
