//! Choices made in the app that outlive it: the theme and the pinned repositories.
//!
//! The desktop app keeps them in `<data dir>/prefs.json` (one JSON object, one key per choice),
//! the web build in the browser's `localStorage` (one `repohub.<key>` entry per choice). A saved
//! choice wins over its `.env` default (`REPOHUB_THEME`, `REPOHUB_PINNED`).
use serde_json::Value;

use crate::state::Theme;

/// The saved theme; `None` when nothing was chosen yet (then `REPOHUB_THEME` decides).
pub fn load_theme() -> Option<Theme> {
    Theme::parse(get("theme")?.as_str()?)
}

pub fn save_theme(theme: Theme) {
    set("theme", Value::from(theme.name()));
}

/// The repositories pinned in the app (full names); `None` when never changed (then
/// `REPOHUB_PINNED`, or the most recently pushed, decide).
pub fn load_pins() -> Option<Vec<String>> {
    Some(get("pins")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
}

pub fn save_pins(pins: &[String]) {
    set("pins", Value::from(pins.to_vec()));
}

/// One saved value.
pub fn get(key: &str) -> Option<Value> {
    read_key(key)
}

/// Saves one value, keeping the others.
pub fn set(key: &str, value: Value) {
    write_key(key, value);
}

// ------------------------------------------------------------------ desktop: prefs.json

#[cfg(not(target_arch = "wasm32"))]
fn file() -> std::path::PathBuf {
    crate::notes::data_dir().join("prefs.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn all() -> serde_json::Map<String, Value> {
    std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn read_key(key: &str) -> Option<Value> {
    all().remove(key)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_key(key: &str, value: Value) {
    let mut map = all();
    map.insert(key.to_owned(), value);
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written aside and moved in place, so a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, Value::Object(map).to_string()).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

// ------------------------------------------------------------------ web: localStorage

#[cfg(target_arch = "wasm32")]
fn storage_call(method: &str, args: &[&str]) -> Option<String> {
    use wasm_bindgen::JsCast;
    let storage = js_sys::Reflect::get(&js_sys::global(), &"localStorage".into()).ok()?;
    let f: js_sys::Function = js_sys::Reflect::get(&storage, &method.into()).ok()?.dyn_into().ok()?;
    let args: js_sys::Array = args.iter().map(|a| wasm_bindgen::JsValue::from_str(a)).collect();
    f.apply(&storage, &args).ok()?.as_string()
}

#[cfg(target_arch = "wasm32")]
fn read_key(key: &str) -> Option<Value> {
    let raw = storage_call("getItem", &[&format!("repohub.{key}")])?;
    // The theme was stored as a bare word before; anything that isn't JSON is a string.
    Some(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
}

#[cfg(target_arch = "wasm32")]
fn write_key(key: &str, value: Value) {
    storage_call("setItem", &[&format!("repohub.{key}"), &value.to_string()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip_and_keep_each_other() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        assert_eq!(load_theme(), None);
        assert_eq!(load_pins(), None);
        save_theme(Theme::Light);
        save_pins(&["octo/app".into(), "acme/site".into()]);
        assert_eq!(load_theme(), Some(Theme::Light), "saving pins keeps the theme");
        assert_eq!(load_pins(), Some(vec!["octo/app".to_owned(), "acme/site".to_owned()]));
        save_theme(Theme::System);
        assert_eq!(load_pins().map(|p| p.len()), Some(2), "saving the theme keeps the pins");
        save_pins(&[]);
        assert_eq!(load_pins(), Some(Vec::new()), "an empty list is a choice, not nothing");
        let _ = std::fs::remove_dir_all(dir);
    }
}
