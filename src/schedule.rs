//! The weekly schedule: when the tool may send impulses. Pure functions of a
//! time of day and a weekday, so they need no clock and are tested with fixed
//! moments.
//!
//! A window runs from `from` up to, but not including, `to`. The one
//! exception is `to = 23:59`, which means the end of the day, so that a window
//! of `00:00` to `23:59` has no gap at midnight. A window with `from > to`
//! belongs to the day it starts on and reaches into the next day.

use crate::config::{Clock, Config, DAYS, Day};

const MINUTES_PER_DAY: usize = 24 * 60;
const MINUTES_PER_WEEK: usize = DAYS * MINUTES_PER_DAY;

/// A minute of the week in local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// 0 is Monday, 6 is Sunday.
    pub weekday: u8,
    /// Minutes since midnight, 0 to 1439.
    pub minute: u16,
}

impl Moment {
    pub fn new(weekday: u8, hour: u16, minute: u16) -> Option<Self> {
        let clock = Clock::new(hour, minute)?;
        (usize::from(weekday) < DAYS).then_some(Self {
            weekday,
            minute: clock.minutes(),
        })
    }

    fn index(self) -> usize {
        usize::from(self.weekday) * MINUTES_PER_DAY + usize::from(self.minute)
    }
}

/// For each minute of the week whether a window covers it.
fn week_map(days: &[Day]) -> Vec<bool> {
    let mut map = vec![false; MINUTES_PER_WEEK];
    for (i, day) in days.iter().take(DAYS).enumerate().filter(|(_, d)| d.on) {
        let from = usize::from(day.from.minutes());
        let to = if day.to == Clock::new(23, 59).unwrap() {
            MINUTES_PER_DAY
        } else {
            usize::from(day.to.minutes())
        };
        let start = i * MINUTES_PER_DAY;
        if from < to {
            map[start + from..start + to].fill(true);
        } else {
            map[start + from..start + MINUTES_PER_DAY].fill(true);
            let next = ((i + 1) % DAYS) * MINUTES_PER_DAY;
            map[next..next + to].fill(true);
        }
    }
    map
}

/// Whether impulses may be sent at `now`. Without a schedule, always.
pub fn is_active(cfg: &Config, now: Moment) -> bool {
    !cfg.schedule_enabled || week_map(&cfg.schedule)[now.index()]
}

/// Minutes from `now` until `is_active` changes, at least 1. `None` if it
/// never changes: no schedule, no day on, or every minute covered.
pub fn next_change(cfg: &Config, now: Moment) -> Option<u32> {
    if !cfg.schedule_enabled {
        return None;
    }
    let map = week_map(&cfg.schedule);
    let here = now.index();
    (1..=MINUTES_PER_WEEK)
        .find(|step| map[(here + step) % MINUTES_PER_WEEK] != map[here])
        .map(|step| step as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(weekday: u8, hh: u16, mm: u16) -> Moment {
        Moment::new(weekday, hh, mm).unwrap()
    }

    fn day(on: bool, from: &str, to: &str) -> Day {
        Day {
            on,
            from: Clock::parse(from).unwrap(),
            to: Clock::parse(to).unwrap(),
        }
    }

    fn cfg(days: [Day; 7]) -> Config {
        Config {
            schedule_enabled: true,
            schedule: days.to_vec(),
            ..Config::default()
        }
    }

    const MON: u8 = 0;
    const TUE: u8 = 1;
    const SAT: u8 = 5;
    const SUN: u8 = 6;

    #[test]
    fn a_window_includes_its_start_and_excludes_its_end() {
        let c = cfg([day(true, "08:00", "23:00"); 7]);
        assert!(!is_active(&c, at(MON, 7, 59)));
        assert!(is_active(&c, at(MON, 8, 0)));
        assert!(is_active(&c, at(MON, 22, 59)));
        assert!(!is_active(&c, at(MON, 23, 0)));
        assert!(!is_active(&c, at(MON, 0, 0)));
    }

    #[test]
    fn a_window_over_midnight_belongs_to_the_day_it_starts_on() {
        let mut days = [day(false, "08:00", "09:00"); 7];
        days[usize::from(MON)] = day(true, "22:00", "01:00");
        let c = cfg(days);
        assert!(!is_active(&c, at(MON, 21, 59)));
        assert!(is_active(&c, at(MON, 22, 0)));
        assert!(is_active(&c, at(MON, 23, 30)));
        assert!(is_active(&c, at(TUE, 0, 30)));
        assert!(!is_active(&c, at(TUE, 1, 0)));
        // Sunday's window reaches into Monday.
        let mut days = [day(false, "08:00", "09:00"); 7];
        days[usize::from(SUN)] = day(true, "22:00", "01:00");
        let c = cfg(days);
        assert!(is_active(&c, at(SUN, 23, 0)));
        assert!(is_active(&c, at(MON, 0, 59)));
        assert!(!is_active(&c, at(MON, 1, 0)));
    }

    #[test]
    fn the_next_day_does_not_cancel_the_window_over_midnight() {
        let mut days = [day(true, "08:00", "23:00"); 7];
        days[usize::from(MON)] = day(true, "22:00", "01:00");
        days[usize::from(TUE)] = day(false, "08:00", "23:00");
        let c = cfg(days);
        // Tuesday is off, but Monday's window still runs until 01:00.
        assert!(is_active(&c, at(TUE, 0, 30)));
        assert!(!is_active(&c, at(TUE, 1, 0)));
        assert!(!is_active(&c, at(TUE, 10, 0)));
        // Tuesday's own window does not start; Wednesday's does.
        assert!(is_active(&c, at(2, 10, 0)));
    }

    #[test]
    fn an_off_day_is_never_active() {
        let mut days = [day(true, "08:00", "23:00"); 7];
        days[usize::from(SAT)] = day(false, "08:00", "23:00");
        let c = cfg(days);
        assert!(is_active(&c, at(4, 12, 0)));
        assert!(!is_active(&c, at(SAT, 12, 0)));
    }

    #[test]
    fn without_a_schedule_it_is_always_active() {
        let c = Config {
            schedule_enabled: false,
            schedule: vec![day(false, "08:00", "09:00")],
            ..Config::default()
        };
        assert!(is_active(&c, at(MON, 3, 0)));
        assert_eq!(next_change(&c, at(MON, 3, 0)), None);
    }

    #[test]
    fn a_window_ending_at_2359_runs_to_the_end_of_the_day() {
        let c = cfg([day(true, "08:00", "23:59"); 7]);
        assert!(is_active(&c, at(MON, 23, 59)));
        assert!(!is_active(&c, at(TUE, 0, 0)));

        // Around the clock: no gap at midnight, so it never changes.
        let c = cfg([day(true, "00:00", "23:59"); 7]);
        assert!(is_active(&c, at(MON, 23, 59)));
        assert!(is_active(&c, at(TUE, 0, 0)));
        assert!(is_active(&c, at(SUN, 23, 59)));
        assert!(is_active(&c, at(MON, 0, 0)));
        assert_eq!(next_change(&c, at(MON, 12, 0)), None);
    }

    #[test]
    fn finds_the_next_change() {
        let c = cfg([day(true, "08:00", "23:00"); 7]);
        assert_eq!(next_change(&c, at(MON, 7, 30)), Some(30));
        assert_eq!(next_change(&c, at(MON, 22, 59)), Some(1));
        assert_eq!(next_change(&c, at(MON, 8, 0)), Some(15 * 60));
        // Inactive at 23:00 until 08:00 the next day.
        assert_eq!(next_change(&c, at(MON, 23, 0)), Some(9 * 60));
        // Across the end of the week.
        assert_eq!(next_change(&c, at(SUN, 23, 30)), Some(8 * 60 + 30));
        // Through a window over midnight.
        let mut days = [day(false, "08:00", "09:00"); 7];
        days[usize::from(MON)] = day(true, "22:00", "01:00");
        let c = cfg(days);
        assert_eq!(next_change(&c, at(MON, 23, 0)), Some(2 * 60));
    }

    #[test]
    fn a_week_without_days_never_changes() {
        let off = cfg([day(false, "08:00", "23:00"); 7]);
        assert!(!is_active(&off, at(MON, 12, 0)));
        assert_eq!(next_change(&off, at(MON, 12, 0)), None);
    }

    #[test]
    fn a_schedule_with_fewer_days_counts_the_rest_as_off() {
        let c = Config {
            schedule_enabled: true,
            schedule: vec![day(true, "08:00", "23:00")],
            ..Config::default()
        };
        assert!(is_active(&c, at(MON, 12, 0)));
        assert!(!is_active(&c, at(TUE, 12, 0)));
    }

    #[test]
    fn moments_are_checked() {
        assert_eq!(Moment::new(7, 0, 0), None);
        assert_eq!(Moment::new(0, 24, 0), None);
        assert_eq!(Moment::new(0, 0, 60), None);
        assert_eq!(Moment::new(6, 23, 59).map(|m| m.minute), Some(1439));
    }
}
