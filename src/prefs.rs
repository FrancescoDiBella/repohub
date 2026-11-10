//! Choices made in the app that outlive it: for now, the theme.
//!
//! The desktop app keeps them in `<data dir>/prefs.json`, the web build in the browser's
//! `localStorage`. A saved choice wins over `REPOHUB_THEME`, which is only the default.

use crate::state::Theme;

/// The saved theme; `None` when nothing was chosen yet (then `REPOHUB_THEME` decides).
pub fn load_theme() -> Option<Theme> {
    Theme::parse(&read()?)
}

pub fn save_theme(theme: Theme) {
    write(theme.name());
}

#[cfg(not(target_arch = "wasm32"))]
fn file() -> std::path::PathBuf {
    crate::notes::data_dir().join("prefs.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn read() -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file()).ok()?).ok()?;
    v.get("theme")?.as_str().map(str::to_owned)
}

#[cfg(not(target_arch = "wasm32"))]
fn write(theme: &str) {
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::json!({ "theme": theme }).to_string());
}

#[cfg(target_arch = "wasm32")]
fn storage_call(method: &str, args: &[&str]) -> Option<String> {
    use wasm_bindgen::JsCast;
    let storage = js_sys::Reflect::get(&js_sys::global(), &"localStorage".into()).ok()?;
    let f: js_sys::Function = js_sys::Reflect::get(&storage, &method.into()).ok()?.dyn_into().ok()?;
    let args: js_sys::Array = args.iter().map(|a| wasm_bindgen::JsValue::from_str(a)).collect();
    f.apply(&storage, &args).ok()?.as_string()
}

#[cfg(target_arch = "wasm32")]
fn read() -> Option<String> {
    storage_call("getItem", &["repohub.theme"])
}

#[cfg(target_arch = "wasm32")]
fn write(theme: &str) {
    storage_call("setItem", &["repohub.theme", theme]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_round_trip() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("repohub-prefs-{}", std::process::id()));
        // SAFETY: the lock keeps other environment-changing tests out.
        unsafe { std::env::set_var("REPOHUB_DATA_DIR", &dir) };
        assert_eq!(load_theme(), None);
        save_theme(Theme::Light);
        assert_eq!(load_theme(), Some(Theme::Light));
        save_theme(Theme::System);
        assert_eq!(load_theme(), Some(Theme::System));
        let _ = std::fs::remove_dir_all(dir);
    }
}
