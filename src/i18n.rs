//! Translations loaded from `lang/*.yaml`.
//!
//! Language files are embedded in the binary; a file with the same name in
//! `<config dir>/lang/` overrides (or adds) keys at runtime. Lookups fall back
//! to English, then to the key itself, so a missing translation never leaves
//! a blank on screen.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../lang/en.yaml")),
    ("tr", include_str!("../lang/tr.yaml")),
    ("de", include_str!("../lang/de.yaml")),
    ("fr", include_str!("../lang/fr.yaml")),
    ("es", include_str!("../lang/es.yaml")),
    ("it", include_str!("../lang/it.yaml")),
    ("nl", include_str!("../lang/nl.yaml")),
    ("pt", include_str!("../lang/pt.yaml")),
    ("pl", include_str!("../lang/pl.yaml")),
    ("ja", include_str!("../lang/ja.yaml")),
    ("zh", include_str!("../lang/zh.yaml")),
];

/// Strings for the desktop setup tool (`lang/setup/`)
pub const SETUP_BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../lang/setup/en.yaml")),
    ("tr", include_str!("../lang/setup/tr.yaml")),
    ("de", include_str!("../lang/setup/de.yaml")),
    ("fr", include_str!("../lang/setup/fr.yaml")),
    ("es", include_str!("../lang/setup/es.yaml")),
    ("it", include_str!("../lang/setup/it.yaml")),
    ("nl", include_str!("../lang/setup/nl.yaml")),
    ("pt", include_str!("../lang/setup/pt.yaml")),
    ("pl", include_str!("../lang/setup/pl.yaml")),
    ("ja", include_str!("../lang/setup/ja.yaml")),
    ("zh", include_str!("../lang/setup/zh.yaml")),
];

type Table = HashMap<String, String>;

struct State {
    code: String,
    table: Table,
    fallback: Table,
}

static STATE: RwLock<Option<Arc<State>>> = RwLock::new(None);
static OVERRIDE_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Tests that switch the global language take this lock so they don't race.
#[cfg(test)]
pub static TEST_LANGUAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Flatten nested YAML into dotted keys; sequences become `key.0`, `key.1`...
fn flatten(prefix: &str, value: &serde_yaml::Value, out: &mut Table) {
    let join = |k: &str| if prefix.is_empty() { k.to_string() } else { format!("{}.{}", prefix, k) };
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (k, v) in map {
                let key = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
                };
                flatten(&join(&key), v, out);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for (i, v) in seq.iter().enumerate() {
                flatten(&join(&i.to_string()), v, out);
            }
        }
        serde_yaml::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        serde_yaml::Value::Number(n) => {
            out.insert(prefix.to_string(), n.to_string());
        }
        serde_yaml::Value::Bool(b) => {
            out.insert(prefix.to_string(), b.to_string());
        }
        _ => {}
    }
}

pub fn parse(src: &str) -> Result<Table, serde_yaml::Error> {
    let value: serde_yaml::Value = serde_yaml::from_str(src)?;
    let mut table = Table::new();
    flatten("", &value, &mut table);
    Ok(table)
}

fn load_table(code: &str) -> Table {
    let mut table = BUILTIN
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, src)| parse(src).ok())
        .unwrap_or_default();

    let dir = OVERRIDE_DIR.read().ok().and_then(|d| d.clone());
    if let Some(path) = dir.map(|d| d.join(format!("{}.yaml", code))) {
        if let Ok(src) = std::fs::read_to_string(&path) {
            match parse(&src) {
                Ok(extra) => {
                    tracing::info!("Loaded language overrides from {:?}", path);
                    table.extend(extra);
                }
                Err(e) => tracing::warn!("Ignoring invalid language file {:?}: {}", path, e),
            }
        }
    }
    table
}

/// Directory with user language files (usually `<config dir>/lang`).
pub fn set_override_dir(dir: Option<&Path>) {
    if let Ok(mut d) = OVERRIDE_DIR.write() {
        *d = dir.map(Path::to_path_buf);
    }
}

/// Switch the active language (no-op if it is already active).
pub fn set_language(code: &str) {
    let code = normalize(code);
    if current().map(|s| s.code == code).unwrap_or(false) {
        return;
    }
    let state = State { table: load_table(&code), fallback: load_table("en"), code };
    if let Ok(mut s) = STATE.write() {
        *s = Some(Arc::new(state));
    }
}

/// "pt-BR" -> "pt", "zh_CN" -> "zh", unknown -> as-is (may come from overrides)
pub fn normalize(code: &str) -> String {
    let lower = code.trim().to_lowercase().replace('_', "-");
    let base = lower.split('-').next().unwrap_or("en").to_string();
    if base.is_empty() {
        "en".to_string()
    } else {
        base
    }
}

fn current() -> Option<Arc<State>> {
    STATE.read().ok().and_then(|s| s.clone())
}

pub fn language() -> String {
    current().map(|s| s.code.clone()).unwrap_or_else(|| "en".to_string())
}

/// Look up a key in the active language.
pub fn t(key: &str) -> String {
    if current().is_none() {
        set_language("en");
    }
    let state = current();
    state
        .as_ref()
        .and_then(|s| s.table.get(key).or_else(|| s.fallback.get(key)))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Look up an optional key (no fallback to the key name).
pub fn try_t(key: &str) -> Option<String> {
    if current().is_none() {
        set_language("en");
    }
    current().and_then(|s| s.table.get(key).or_else(|| s.fallback.get(key)).cloned())
}

/// Look up a key and fill `{name}` placeholders.
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{}}}", name), value);
    }
    s
}

/// Element `idx` of a list key such as `date.months`.
pub fn nth(key: &str, idx: usize) -> String {
    t(&format!("{}.{}", key, idx))
}

/// Format a number with the language's decimal separator.
pub fn num(value: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, value);
    let sep = t("meta.decimal");
    if sep == "." || sep.is_empty() {
        s
    } else {
        s.replace('.', &sep)
    }
}

/// Translate a Home Assistant state string; unknown states pass through.
pub fn state_label(raw: &str) -> String {
    let key = match raw {
        "on" => "on",
        "off" => "off",
        "unavailable" => "unavailable",
        "unknown" | "none" => "unknown",
        "run" | "running" => "running",
        "idle" | "ready" => "idle",
        "standby" => "standby",
        "heating" => "heating",
        "cooling" => "cooling",
        "drying" => "drying",
        "fan" => "fan",
        "printing" => "printing",
        "paused" | "pause" => "paused",
        "stop" | "stopped" => "stopped",
        "complete" | "completed" | "finished" | "finish" => "finished",
        "error" => "error",
        _ => return raw.to_string(),
    };
    t(&format!("state.{}", key))
}

/// Font family required by a language (e.g. "NotoSansSC"), if any.
pub fn font_for(code: &str) -> Option<String> {
    let code = normalize(code);
    BUILTIN
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, src)| parse(src).ok())
        .and_then(|t| t.get("meta.font").cloned())
        .filter(|f| !f.is_empty())
}

/// (code, native name) of every built-in language
pub fn available() -> Vec<(String, String)> {
    BUILTIN
        .iter()
        .map(|(code, src)| {
            let name = parse(src).ok().and_then(|t| t.get("meta.name").cloned()).unwrap_or_else(|| code.to_string());
            (code.to_string(), name)
        })
        .collect()
}

/// Setup tool strings for `code`, with English filling any gaps.
pub fn setup_strings(code: &str) -> Table {
    let code = normalize(code);
    let lookup = |c: &str| SETUP_BUILTIN.iter().find(|(k, _)| *k == c).and_then(|(_, src)| parse(src).ok()).unwrap_or_default();
    let mut table = lookup("en");
    if code != "en" {
        table.extend(lookup(&code));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_english_key() {
        let en = parse(BUILTIN[0].1).expect("en.yaml parses");
        for (code, src) in BUILTIN {
            let table = parse(src).unwrap_or_else(|e| panic!("{}.yaml: {}", code, e));
            let missing: Vec<&String> = en.keys().filter(|k| !table.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{}.yaml is missing {:?}", code, missing);
            for (key, value) in &en {
                for placeholder in ["{time}", "{temp}", "{day}", "{month}", "{value}"] {
                    if value.contains(placeholder) {
                        assert!(table[key].contains(placeholder), "{}.yaml: {} lost {}", code, key, placeholder);
                    }
                }
            }
        }
    }

    #[test]
    fn every_setup_language_has_every_english_key() {
        let en = parse(SETUP_BUILTIN[0].1).expect("setup/en.yaml parses");
        assert_eq!(SETUP_BUILTIN.len(), BUILTIN.len(), "setup tool and dashboard should offer the same languages");
        let placeholder = regex_lite_placeholders;
        for (code, src) in SETUP_BUILTIN {
            let table = parse(src).unwrap_or_else(|e| panic!("setup/{}.yaml: {}", code, e));
            let missing: Vec<&String> = en.keys().filter(|k| !table.contains_key(*k)).collect();
            assert!(missing.is_empty(), "setup/{}.yaml is missing {:?}", code, missing);
            let extra: Vec<&String> = table.keys().filter(|k| !en.contains_key(*k)).collect();
            assert!(extra.is_empty(), "setup/{}.yaml has unknown keys {:?}", code, extra);
            for (key, value) in &en {
                for p in placeholder(value) {
                    assert!(table[key].contains(&p), "setup/{}.yaml: {} lost {}", code, key, p);
                }
            }
        }
    }

    /// `{name}` placeholders in a string
    fn regex_lite_placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(start) = rest.find('{') {
            match rest[start..].find('}') {
                Some(end) => {
                    out.push(rest[start..start + end + 1].to_string());
                    rest = &rest[start + end + 1..];
                }
                None => break,
            }
        }
        out
    }

    #[test]
    fn lookup_and_formatting() {
        let _guard = TEST_LANGUAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language("tr");
        assert_eq!(t("state.on"), "Açık");
        assert_eq!(nth("date.months", 9), "Ekim");
        assert_eq!(num(21.56, 1), "21,6");
        assert_eq!(tf("climate.room", &[("temp", "21,5")]), "Oda 21,5°");
        assert_eq!(t("no.such.key"), "no.such.key");
        set_language("en");
        assert_eq!(num(21.56, 1), "21.6");
        assert_eq!(normalize("pt-BR"), "pt");
        assert_eq!(font_for("zh").as_deref(), Some("NotoSansSC"));
        assert_eq!(font_for("de"), None);
    }
}
