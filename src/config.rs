//! Persistent configuration, shared by the daemon, the window, tuxhub and the
//! command line. Only the daemon writes it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "tuxhub-always-online";

pub const MIN_INTERVAL_S: u32 = 30;
pub const MAX_INTERVAL_S: u32 = 1800;
pub const MAX_HOURS: u32 = 24;
/// One entry per weekday, Monday first.
pub const DAYS: usize = 7;

/// A time of day in local time, with minute precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Clock(u16);

impl Clock {
    pub fn new(hour: u16, minute: u16) -> Option<Self> {
        (hour < 24 && minute < 60).then_some(Self(hour * 60 + minute))
    }

    /// Reads exactly `HH:MM` (two digits each, 00:00 to 23:59).
    pub fn parse(text: &str) -> Option<Self> {
        let b = text.as_bytes();
        if b.len() != 5 || b[2] != b':' {
            return None;
        }
        let digits = |pair: &[u8]| -> Option<u16> {
            pair.iter()
                .all(u8::is_ascii_digit)
                .then(|| u16::from(pair[0] - b'0') * 10 + u16::from(pair[1] - b'0'))
        };
        Self::new(digits(&b[0..2])?, digits(&b[3..5])?)
    }

    /// Minutes since midnight.
    pub fn minutes(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 60, self.0 % 60)
    }
}

impl Serialize for Clock {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Clock {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Clock::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid time {text:?}, use HH:MM")))
    }
}

/// The kind of input event that resets the idle time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Impulse {
    /// `KEY_F15`, pressed and released.
    Key,
    /// Pointer movement of one unit and back.
    Mouse,
}

/// The window of one weekday. `from > to` reaches past midnight into the next
/// day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub on: bool,
    pub from: Clock,
    pub to: Clock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    /// Time between two impulses.
    pub interval_s: u32,
    pub impulse: Impulse,
    /// Longest run in hours; 0 means no limit.
    pub max_hours: u32,
    pub schedule_enabled: bool,
    /// Monday first, exactly [`DAYS`] entries.
    pub schedule: Vec<Day>,
    pub start_at_login: bool,
}

impl Default for Config {
    fn default() -> Self {
        let day = Day {
            on: true,
            from: Clock::new(8, 0).unwrap(),
            to: Clock::new(23, 0).unwrap(),
        };
        Self {
            enabled: false,
            interval_s: 240,
            impulse: Impulse::Key,
            max_hours: 0,
            schedule_enabled: false,
            schedule: vec![day; DAYS],
            start_at_login: true,
        }
    }
}

impl Config {
    pub fn path() -> io::Result<PathBuf> {
        Ok(crate::paths::config_home()?
            .join(APP_DIR)
            .join("config.json"))
    }

    /// Loads the configuration. A missing or unreadable file yields the defaults.
    pub fn load() -> Self {
        Self::path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Reads the contents of a configuration file. Every field counts on its
    /// own: an invalid or unknown one is dropped and keeps its default, so a
    /// file edited by hand never fails as a whole.
    fn parse(text: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut cfg = Config::default();
        for (key, v) in value.as_object()? {
            let mut one = serde_json::Map::new();
            one.insert(key.clone(), v.clone());
            if let Ok(next) = cfg.merged(one.into()) {
                cfg = next;
            }
        }
        Some(cfg)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::path()?)
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap() + "\n")?;
        std::fs::rename(tmp, path)
    }

    /// Strict validation for changes requested through the interface.
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_INTERVAL_S..=MAX_INTERVAL_S).contains(&self.interval_s) {
            return Err(format!(
                "the interval must be between {MIN_INTERVAL_S} and {MAX_INTERVAL_S} seconds"
            ));
        }
        if self.max_hours > MAX_HOURS {
            return Err(format!("the longest run is limited to {MAX_HOURS} hours"));
        }
        if self.schedule.len() != DAYS {
            return Err(format!("the schedule needs exactly {DAYS} days"));
        }
        if self.schedule.iter().any(|d| d.from == d.to) {
            return Err("a window must start and end at different times".into());
        }
        Ok(())
    }

    /// Applies the fields present in `patch` and validates the result.
    pub fn merged(&self, patch: serde_json::Value) -> Result<Self, String> {
        let mut value = serde_json::to_value(self).unwrap();
        let (Some(obj), Some(patch)) = (value.as_object_mut(), patch.as_object()) else {
            return Err("expected an object".into());
        };
        for (k, v) in patch {
            if !obj.contains_key(k) {
                return Err(format!("unknown setting {k:?}"));
            }
            obj.insert(k.clone(), v.clone());
        }
        let cfg: Config =
            serde_json::from_value(value).map_err(|e| format!("invalid settings: {e}"))?;
        cfg.validate()?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clock(text: &str) -> Clock {
        Clock::parse(text).unwrap()
    }

    fn day(on: bool, from: &str, to: &str) -> Day {
        Day {
            on,
            from: clock(from),
            to: clock(to),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tao-config-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn clock_accepts_exactly_hh_mm() {
        assert_eq!(clock("00:00").minutes(), 0);
        assert_eq!(clock("08:05").minutes(), 8 * 60 + 5);
        assert_eq!(clock("23:59").minutes(), 23 * 60 + 59);
        for bad in [
            "",
            "24:00",
            "8:00",
            "08:0",
            "08:60",
            "ab:cd",
            " 08:00",
            "08:00 ",
            "08-00",
            "0800",
            "08:000",
            "+8:00",
            "08:+5",
            "٠٨:٠٠",
            "08:00:00",
        ] {
            assert_eq!(Clock::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Clock::new(24, 0), None);
        assert_eq!(Clock::new(0, 60), None);
    }

    #[test]
    fn clock_is_written_as_hh_mm() {
        assert_eq!(clock("08:05").to_string(), "08:05");
        assert_eq!(serde_json::to_string(&clock("23:59")).unwrap(), "\"23:59\"");
        assert_eq!(
            serde_json::from_str::<Clock>("\"07:30\"").unwrap(),
            clock("07:30")
        );
        assert!(serde_json::from_str::<Clock>("\"7:30\"").is_err());
        assert!(serde_json::from_str::<Clock>("730").is_err());
    }

    #[test]
    fn defaults_follow_the_spec() {
        let cfg = Config::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.interval_s, 240);
        assert_eq!(cfg.impulse, Impulse::Key);
        assert_eq!(cfg.max_hours, 0);
        assert!(!cfg.schedule_enabled);
        assert_eq!(cfg.schedule, vec![day(true, "08:00", "23:00"); 7]);
        assert!(cfg.start_at_login);
        assert_eq!(cfg.validate(), Ok(()));
    }

    #[test]
    fn merges_partial_changes() {
        let cfg = Config::default()
            .merged(json!({ "enabled": true, "interval_s": 60, "impulse": "mouse" }))
            .unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.interval_s, 60);
        assert_eq!(cfg.impulse, Impulse::Mouse);
        // The rest is untouched.
        assert_eq!(cfg.max_hours, 0);
        assert!(cfg.start_at_login);
        assert_eq!(cfg.merged(json!({})).unwrap(), cfg);
    }

    #[test]
    fn checks_the_ranges() {
        let cfg = Config::default();
        let set = |k: &str, v: serde_json::Value| cfg.merged(json!({ k: v }));
        assert!(set("interval_s", json!(29)).is_err());
        assert_eq!(set("interval_s", json!(30)).unwrap().interval_s, 30);
        assert_eq!(set("interval_s", json!(1800)).unwrap().interval_s, 1800);
        assert!(set("interval_s", json!(1801)).is_err());
        assert!(set("interval_s", json!(-1)).is_err());
        assert!(set("interval_s", json!(60.5)).is_err());
        assert!(set("interval_s", json!("60")).is_err());
        assert_eq!(set("max_hours", json!(24)).unwrap().max_hours, 24);
        assert!(set("max_hours", json!(25)).is_err());
        assert!(set("max_hours", json!(-1)).is_err());
        assert!(set("impulse", json!("dbus")).is_err());
        assert!(set("impulse", json!("KEY")).is_err());
        assert!(set("enabled", json!("yes")).is_err());
        assert!(set("start_at_login", json!(1)).is_err());
        assert!(set("unknown", json!(1)).is_err());
        assert!(cfg.merged(json!([])).is_err());
        assert!(cfg.merged(json!("enabled")).is_err());
    }

    #[test]
    fn checks_the_schedule() {
        let cfg = Config::default();
        let set = |days: Vec<serde_json::Value>| cfg.merged(json!({ "schedule": days }));
        let ok = || json!({ "on": true, "from": "08:00", "to": "23:00" });
        let week = |d: serde_json::Value| vec![d; 7];

        assert!(set(week(ok())).is_ok());
        // Always one entry per weekday.
        assert!(set(vec![ok(); 6]).is_err());
        assert!(set(vec![ok(); 8]).is_err());
        assert!(set(vec![]).is_err());
        // A window over midnight, a full day and an off day are fine.
        assert!(set(week(json!({ "on": true, "from": "22:00", "to": "01:00" }))).is_ok());
        assert!(set(week(json!({ "on": true, "from": "00:00", "to": "23:59" }))).is_ok());
        assert!(set(week(json!({ "on": false, "from": "08:00", "to": "09:00" }))).is_ok());
        // Same start and end is neither a full day nor an empty one.
        assert!(set(week(json!({ "on": true, "from": "08:00", "to": "08:00" }))).is_err());
        assert!(set(week(json!({ "on": false, "from": "08:00", "to": "08:00" }))).is_err());
        // Wrong forms.
        for bad in [
            json!({ "on": true, "from": "24:00", "to": "23:00" }),
            json!({ "on": true, "from": "8:00", "to": "23:00" }),
            json!({ "on": true, "from": "ab:cd", "to": "23:00" }),
            json!({ "on": true, "from": "08:00" }),
            json!({ "from": "08:00", "to": "23:00" }),
            json!({ "on": "yes", "from": "08:00", "to": "23:00" }),
            json!({ "on": true, "from": "08:00", "to": "23:00", "extra": 1 }),
            json!("08:00-23:00"),
            json!(null),
        ] {
            assert!(set(week(bad.clone())).is_err(), "{bad}");
        }
        assert!(cfg.merged(json!({ "schedule": "mon" })).is_err());
        // The weekdays can differ.
        let mut days = week(ok());
        days[5] = json!({ "on": true, "from": "10:00", "to": "01:00" });
        days[6] = json!({ "on": false, "from": "10:00", "to": "01:00" });
        let cfg = set(days).unwrap();
        assert_eq!(cfg.schedule[0], day(true, "08:00", "23:00"));
        assert_eq!(cfg.schedule[5], day(true, "10:00", "01:00"));
        assert!(!cfg.schedule[6].on);
    }

    #[test]
    fn a_refused_change_leaves_the_configuration_as_it_was() {
        let cfg = Config::default()
            .merged(json!({ "enabled": true, "interval_s": 90 }))
            .unwrap();
        let before = cfg.clone();
        assert!(
            cfg.merged(json!({ "interval_s": 120, "max_hours": 99 }))
                .is_err()
        );
        assert_eq!(cfg, before);
    }

    #[test]
    fn repairs_a_file_edited_by_hand_field_by_field() {
        let cfg = Config::parse(
            &json!({
                "enabled": true,
                "interval_s": 99999,
                "impulse": "mouse",
                "max_hours": "many",
                "schedule_enabled": true,
                "schedule": [{ "on": true, "from": "99:00", "to": "23:00" }],
                "start_at_login": false,
                "unknown": 5,
            })
            .to_string(),
        )
        .unwrap();
        let defaults = Config::default();
        // Valid fields are kept, the others fall back to their defaults.
        assert!(cfg.enabled);
        assert_eq!(cfg.impulse, Impulse::Mouse);
        assert!(cfg.schedule_enabled);
        assert!(!cfg.start_at_login);
        assert_eq!(cfg.interval_s, defaults.interval_s);
        assert_eq!(cfg.max_hours, defaults.max_hours);
        assert_eq!(cfg.schedule, defaults.schedule);
        assert_eq!(cfg.validate(), Ok(()));
    }

    #[test]
    fn a_broken_schedule_does_not_stop_the_file_from_loading() {
        let cfg = Config::parse(
            r#"{ "enabled": true, "schedule_enabled": false, "schedule": "nonsense" }"#,
        )
        .unwrap();
        assert!(cfg.enabled);
        assert!(!cfg.schedule_enabled);
        assert_eq!(cfg.schedule, Config::default().schedule);
    }

    #[test]
    fn files_that_are_not_objects_give_the_defaults() {
        let dir = temp_dir("garbage");
        let path = dir.join("config.json");
        assert_eq!(Config::load_from(&path), Config::default());
        for text in ["", "{ not json", "[]", "42", "null", "\"enabled\""] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(Config::load_from(&path), Config::default(), "{text:?}");
        }
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_atomically_and_reads_back() {
        let dir = temp_dir("save");
        let path = dir.join("nested").join("config.json");
        let mut cfg = Config::default()
            .merged(
                json!({ "enabled": true, "interval_s": 45, "impulse": "mouse", "max_hours": 4 }),
            )
            .unwrap();
        cfg.schedule[2] = day(true, "22:00", "01:30");
        cfg.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), cfg);
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.json"]);
        // The times are written as HH:MM.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"from\": \"22:00\""), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
