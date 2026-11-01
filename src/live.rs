//! tuxhub live interface (protocol 1): the schema of the basic settings and
//! the mapping between its keys and [`Config`]. The weekly schedule and the
//! access setup stay in the tool's own interface.

use crate::config::{Config, MAX_HOURS, MAX_INTERVAL_S, MIN_INTERVAL_S};
use serde_json::{Map, Value, json};

pub const PROTOCOL: u64 = 1;
/// Id of the tool in tuxhub, the name of the socket.
pub const TOOL_ID: &str = "always-online";

/// Keys of the schema, in the order of the page.
pub const KEYS: [&str; 5] = [
    "enabled",
    "interval_s",
    "impulse",
    "max_hours",
    "start_at_login",
];

/// A refused request; `field` names the setting it is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    pub field: Option<String>,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            field: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

fn on_field(field: &str, message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        field: Some(field.to_string()),
    }
}

/// Answer to `hello`.
pub fn hello(params: &Value) -> Result<Value, Failure> {
    match params.get("protocol").and_then(Value::as_u64) {
        Some(PROTOCOL) => Ok(json!({
            "protocol": PROTOCOL,
            "name": "Always Online",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        _ => Err(format!("only protocol {PROTOCOL} is supported").into()),
    }
}

/// Schema of the basic settings, in German for `de` and in English otherwise.
pub fn schema(locale: &str) -> Value {
    let de = locale == "de";
    let t = |en: &'static str, de_text: &'static str| if de { de_text } else { en };
    json!({
        "sections": [{
            "id": "general",
            "label": t("General", "Allgemein"),
            "fields": [
                {
                    "key": "enabled",
                    "type": "toggle",
                    "label": t("Keep me online", "Online halten"),
                    "help": t(
                        "Sends a harmless input at a fixed interval so apps do not set you to away",
                        "Sendet in festem Abstand eine harmlose Eingabe, damit Programme dich nicht auf Abwesend setzen",
                    ),
                    "default": false,
                },
                {
                    "key": "interval_s",
                    "type": "number",
                    "label": t("Interval", "Abstand"),
                    "help": t(
                        "Time between two impulses; keep it below the shortest away time of your apps, for example 5 minutes",
                        "Zeit zwischen zwei Impulsen; kürzer als die kürzeste Abwesend-Zeit deiner Programme, zum Beispiel 5 Minuten",
                    ),
                    "min": MIN_INTERVAL_S,
                    "max": MAX_INTERVAL_S,
                    "step": 10,
                    "unit": "s",
                    "default": 240,
                },
                {
                    "key": "impulse",
                    "type": "select",
                    "label": t("Impulse", "Impuls"),
                    "options": [
                        { "value": "key", "label": t("Key F15, which has no effect", "Taste F15, die nichts auslöst") },
                        { "value": "mouse", "label": t("Mouse one step and back", "Maus einen Schritt und zurück") },
                    ],
                    "default": "key",
                },
                {
                    "key": "max_hours",
                    "type": "number",
                    "label": t("Longest run", "Längste Laufzeit"),
                    "help": t(
                        "Pauses after this many hours; 0 means no limit",
                        "Pausiert nach so vielen Stunden; 0 heißt ohne Grenze",
                    ),
                    "min": 0,
                    "max": MAX_HOURS,
                    "step": 1,
                    "unit": "h",
                    "default": 0,
                },
                {
                    "key": "start_at_login",
                    "type": "toggle",
                    "label": t("Start at login", "Bei der Anmeldung starten"),
                    "default": true,
                },
            ],
        }],
        "actions": [
            { "id": "toggle", "label": t("Switch Keep me online on or off", "Online halten ein- oder ausschalten") },
            { "id": "test", "label": t("Send a test impulse", "Test-Impuls senden") },
        ],
    })
}

/// The values of the schema keys.
pub fn values(cfg: &Config) -> Map<String, Value> {
    let all = serde_json::to_value(cfg).unwrap_or_default();
    KEYS.iter()
        .map(|k| (k.to_string(), all.get(*k).cloned().unwrap_or(Value::Null)))
        .collect()
}

/// The configuration after `set {key, value}`, checked like every change.
pub fn set(cfg: &Config, params: &Value) -> Result<(Config, Value), Failure> {
    let key = params
        .get("key")
        .and_then(Value::as_str)
        .ok_or("`key` is missing")?;
    if !KEYS.contains(&key) {
        return Err(on_field(key, format!("unknown setting {key:?}")));
    }
    let value = params.get("value").cloned().unwrap_or(Value::Null);
    let fits = match key {
        "enabled" | "start_at_login" => value.is_boolean(),
        "impulse" => matches!(value.as_str(), Some("key" | "mouse")),
        "interval_s" | "max_hours" => value.as_u64().is_some(),
        _ => false,
    };
    if !fits {
        return Err(on_field(key, format!("invalid value for {key:?}")));
    }
    let next = cfg
        .merged(json!({ key: value }))
        .map_err(|e| on_field(key, e))?;
    let applied = values(&next).remove(key).unwrap_or(Value::Null);
    Ok((next, applied))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(schema: &Value) -> Vec<&Value> {
        schema["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["fields"].as_array().unwrap())
            .collect()
    }

    fn texts(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if matches!(k.as_str(), "label" | "help") {
                        out.push(v.as_str().unwrap().to_string());
                    } else {
                        texts(v, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|v| texts(v, out)),
            _ => {}
        }
    }

    #[test]
    fn schema_is_complete_in_german_and_english() {
        let (en, de) = (schema("en"), schema("de"));
        let keys: Vec<&str> = fields(&en)
            .iter()
            .map(|f| f["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, KEYS);
        let (mut en_texts, mut de_texts) = (Vec::new(), Vec::new());
        texts(&en, &mut en_texts);
        texts(&de, &mut de_texts);
        assert_eq!(en_texts.len(), de_texts.len());
        for (e, d) in en_texts.iter().zip(&de_texts) {
            assert!(!e.is_empty() && !d.is_empty());
            assert_ne!(e, d, "{e:?} is not translated");
        }
        // English for unknown languages.
        assert_eq!(schema("fr"), en);
        assert_eq!(schema(""), en);
        // The defaults are those of a new configuration.
        let defaults = values(&Config::default());
        for f in fields(&en) {
            assert_eq!(
                f["default"],
                defaults[f["key"].as_str().unwrap()],
                "{}",
                f["key"]
            );
        }
        // The limits are those of the configuration.
        let limits = |key: &str| {
            let f = fields(&en).into_iter().find(|f| f["key"] == key).unwrap();
            (f["min"].clone(), f["max"].clone())
        };
        assert_eq!(limits("interval_s"), (json!(30), json!(1800)));
        assert_eq!(limits("max_hours"), (json!(0), json!(24)));
        let actions: Vec<&str> = en["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect();
        assert_eq!(actions, ["toggle", "test"]);
    }

    #[test]
    fn values_have_exactly_the_schema_keys() {
        assert_eq!(
            Value::Object(values(&Config::default())),
            json!({
                "enabled": false,
                "interval_s": 240,
                "impulse": "key",
                "max_hours": 0,
                "start_at_login": true,
            })
        );
    }

    #[test]
    fn set_refuses_unknown_keys_wrong_types_and_values_out_of_range() {
        let cfg = Config::default();
        for (params, field) in [
            (json!({ "key": "schedule", "value": [] }), "schedule"),
            (json!({ "key": "nope", "value": 1 }), "nope"),
            (json!({ "key": "enabled", "value": "yes" }), "enabled"),
            (
                json!({ "key": "start_at_login", "value": 1 }),
                "start_at_login",
            ),
            (json!({ "key": "impulse", "value": "dbus" }), "impulse"),
            (json!({ "key": "impulse", "value": 1 }), "impulse"),
            (json!({ "key": "interval_s", "value": "100" }), "interval_s"),
            (json!({ "key": "interval_s", "value": -1 }), "interval_s"),
            (json!({ "key": "interval_s", "value": 1.5 }), "interval_s"),
            (json!({ "key": "interval_s", "value": 29 }), "interval_s"),
            (json!({ "key": "interval_s", "value": 1801 }), "interval_s"),
            (json!({ "key": "max_hours", "value": 25 }), "max_hours"),
            (json!({ "key": "max_hours", "value": null }), "max_hours"),
            (json!({ "key": "enabled" }), "enabled"),
        ] {
            let e = set(&cfg, &params).unwrap_err();
            assert_eq!(e.field.as_deref(), Some(field), "{params}");
        }
        let e = set(&cfg, &json!({ "value": 1 })).unwrap_err();
        assert!(e.field.is_none());
        assert!(set(&cfg, &json!("enabled")).is_err());
    }

    #[test]
    fn set_returns_the_new_configuration_and_value() {
        let (cfg, v) = set(
            &Config::default(),
            &json!({ "key": "interval_s", "value": 90 }),
        )
        .unwrap();
        assert_eq!((cfg.interval_s, v), (90, json!(90)));
        let (cfg, v) = set(&cfg, &json!({ "key": "impulse", "value": "mouse" })).unwrap();
        assert_eq!(v, json!("mouse"));
        let (cfg, _) = set(&cfg, &json!({ "key": "enabled", "value": true })).unwrap();
        assert_eq!(values(&cfg)["interval_s"], json!(90));
        assert_eq!(values(&cfg)["enabled"], json!(true));
        // The weekly schedule is untouched.
        assert_eq!(cfg.schedule, Config::default().schedule);
    }

    #[test]
    fn hello_needs_protocol_1() {
        let hi = hello(&json!({ "protocol": 1 })).unwrap();
        assert_eq!(hi["protocol"], json!(1));
        assert_eq!(hi["name"], json!("Always Online"));
        assert!(hello(&json!({ "protocol": 2 })).is_err());
        assert!(hello(&json!({})).is_err());
    }
}
