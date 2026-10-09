//! File ▸ Scripts and ScriptUI.
//!
//! * **Installed scripts** live in the settings store under `Scripts/` (File ▸ Scripts ▸ Install
//!   Script File…) and `Scripts/ScriptUI Panels/` (Install ScriptUI Panel…), next to the
//!   settings on the desktop, so dropping `.jsx` files in those folders works too. File ▸ Scripts
//!   lists them with the bundled sample scripts and extensions ([`BUNDLED`]) and runs them by name; ScriptUI
//!   panels appear at the bottom of the Window menu and open as dockable panels
//!   (`window.scriptPanel`), running with `this` = the panel as in After Effects.
//! * **ScriptUI windows** (`scriptui.*`): list the open script windows, read their control
//!   trees, click buttons and set values like a user, close them (see [`crate::scriptui`]).
//! * **Script settings** (`script.settings.*`): After Effects' `app.settings`, string values by
//!   section and key that last between runs ([`SCRIPT_SETTINGS_FILE`]).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Event, Result, Session, cmd, scriptui};

/// The settings-store folders of installed scripts.
pub const SCRIPTS_DIR: &str = "Scripts";
pub const PANELS_DIR: &str = "Scripts/ScriptUI Panels";
/// The ScriptUI panels open when EffectCraft last quit (settings store), reopened at launch.
pub const OPEN_PANELS_FILE: &str = "scriptui_panels.json";
/// Scripts' `app.settings` in the settings store: `{section: {key: value}}`, as After Effects
/// keeps them in its preferences.
pub const SCRIPT_SETTINGS_FILE: &str = "script_settings.json";
/// User presets of the core Ease Presets panel of v0.6.0 (#254) and where the Ease Presets
/// ScriptUI panel that replaced it keeps them: (file, section, key). The file's text moves there
/// unchanged (the panel reads the same JSON) the first time settings load.
const LEGACY_EASE_PRESETS: (&str, &str, &str) = ("ease_presets.json", "Ease Presets", "userPresets");
/// Longest section, key and value a script may store (characters / bytes).
const MAX_SETTING_NAME: usize = 256;
const MAX_SETTING_VALUE: usize = 1 << 20;

/// Scripts' `app.settings`: values by section and key.
pub type ScriptSettings = BTreeMap<String, BTreeMap<String, String>>;

/// Load scripts' settings from the settings store (a corrupt file gives none), moving user ease
/// presets saved before Ease Presets became a ScriptUI panel into its section.
pub(crate) fn load_script_settings(s: &mut Session) {
    let Some(cfg) = s.config.clone() else { return };
    s.script_settings = cfg
        .read(SCRIPT_SETTINGS_FILE)
        .and_then(|t| serde_json::from_str(&t).map_err(|e| log::warn!("{SCRIPT_SETTINGS_FILE} is not valid ({e}); scripts start without their settings")).ok())
        .unwrap_or_default();
    let (file, section, key) = LEGACY_EASE_PRESETS;
    if let Some(text) = cfg.read(file)
        && !s.script_settings.get(section).is_some_and(|m| m.contains_key(key))
    {
        s.script_settings.entry(section.into()).or_default().insert(key.into(), text);
        if let Err(e) = store_script_settings(s) {
            log::warn!("cannot move {file} into the Ease Presets panel's settings: {e}");
        }
    }
}

fn store_script_settings(s: &Session) -> Result<()> {
    let Some(cfg) = &s.config else { return Ok(()) };
    let text = serde_json::to_string_pretty(&s.script_settings).map_err(|e| EngineError::Other(format!("script settings: {e}")))?;
    cfg.write(SCRIPT_SETTINGS_FILE, &text).map_err(|e| EngineError::Other(format!("cannot save script settings: {e}")))
}

fn setting_name<'a>(p: &'a Value, k: &str, c: &str) -> Result<&'a str> {
    let v = str_p(p, k).ok_or_else(|| bad(c, format!("missing `{k}`")))?;
    if v.chars().count() > MAX_SETTING_NAME {
        return Err(bad(c, format!("`{k}` is longer than {MAX_SETTING_NAME} characters")));
    }
    Ok(v)
}

/// `script.settings.get {section, key}` → `{have, value}` (`value` "" when there is none).
fn settings_get(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "script.settings.get";
    let (section, key) = (setting_name(p, "section", C)?, setting_name(p, "key", C)?);
    let v = s.script_settings.get(section).and_then(|m| m.get(key));
    Ok(json!({"have": v.is_some(), "value": v.cloned().unwrap_or_default()}))
}

/// `script.settings.save {section, key, value}`: kept in the settings store.
fn settings_save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "script.settings.save";
    let (section, key) = (setting_name(p, "section", C)?, setting_name(p, "key", C)?);
    let value = match p.get("value") {
        Some(Value::String(v)) => v.clone(),
        Some(v) if !v.is_null() => v.to_string(),
        _ => return Err(bad(C, "missing `value`")),
    };
    if value.len() > MAX_SETTING_VALUE {
        return Err(bad(C, format!("`value` is larger than {MAX_SETTING_VALUE} bytes")));
    }
    s.script_settings.entry(section.into()).or_default().insert(key.into(), value);
    store_script_settings(s)?;
    Ok(Value::Null)
}

/// Scripts that ship with EffectCraft (original work): (file name, ScriptUI panel, source, code).
/// `sample`s show how to script EffectCraft; `extension`s (in `extensions/`) are optional tools
/// built on the public scripting API, outside the core, like third-party scripts in After Effects.
pub const BUNDLED: &[(&str, bool, &str, &str)] = &[
    ("Create Null at Selected Layers.jsx", false, "sample", include_str!("../../scripts/Create Null at Selected Layers.jsx")),
    ("Rename Layers.jsx", false, "sample", include_str!("../../scripts/Rename Layers.jsx")),
    ("Render Queue Batch.jsx", false, "sample", include_str!("../../scripts/Render Queue Batch.jsx")),
    ("Sort Layers by In Point.jsx", false, "sample", include_str!("../../scripts/Sort Layers by In Point.jsx")),
    ("Layer Tools.jsx", true, "sample", include_str!("../../scripts/ScriptUI Panels/Layer Tools.jsx")),
    ("Ease Presets.jsx", true, "extension", include_str!("../../../../extensions/scriptui-panels/Ease Presets.jsx")),
];

/// A script File ▸ Scripts (or the Window menu, for panels) offers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScriptEntry {
    /// File name (`Rename Layers.jsx`): what the menus show and commands take.
    pub name: String,
    /// In the ScriptUI Panels folder: opens as a dockable panel.
    pub panel: bool,
    /// `installed`, `sample` or `extension` (bundled, see [`BUNDLED`]).
    pub source: &'static str,
}

fn is_script(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".jsx") || l.ends_with(".js")
}

/// Installed scripts and panels, then the bundled ones not shadowed by an installed one, by name.
pub fn scripts(s: &Session) -> Vec<ScriptEntry> {
    let mut v: Vec<ScriptEntry> = vec![];
    if let Some(cfg) = &s.config {
        for (dir, panel) in [(SCRIPTS_DIR, false), (PANELS_DIR, true)] {
            for n in cfg.list(dir).into_iter().filter(|n| is_script(n)) {
                v.push(ScriptEntry { name: n, panel, source: "installed" });
            }
        }
    }
    for (name, panel, source, _) in BUNDLED {
        if !v.iter().any(|e| e.name == *name && e.panel == *panel) {
            v.push(ScriptEntry { name: name.to_string(), panel: *panel, source });
        }
    }
    v.sort_by(|a, b| a.panel.cmp(&b.panel).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// The code of an installed or bundled script / panel.
pub fn script_code(s: &Session, name: &str, panel: Option<bool>) -> Option<(bool, String)> {
    if let Some(cfg) = &s.config {
        for (dir, is_panel) in [(SCRIPTS_DIR, false), (PANELS_DIR, true)] {
            if panel.is_some_and(|p| p != is_panel) {
                continue;
            }
            if let Some(code) = cfg.read(&format!("{dir}/{name}")) {
                return Some((is_panel, code));
            }
        }
    }
    BUNDLED.iter().find(|(n, p, ..)| *n == name && panel.is_none_or(|w| w == *p)).map(|(_, p, _, c)| (*p, c.to_string()))
}

fn install(s: &mut Session, p: &Value, panel: bool) -> Result<Value> {
    let c = if panel { "file.installScriptUIPanel" } else { "file.installScript" };
    let path = str_p(p, "path").ok_or_else(|| bad(c, "missing `path` (a .jsx / .js script)"))?;
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if !is_script(&name) {
        return Err(bad(c, "scripts are .jsx or .js files (.jsxbin is not supported)"));
    }
    let bytes = s.services.read_file(path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    let code = String::from_utf8(bytes).map_err(|_| bad(c, "the script is not UTF-8 text"))?;
    let cfg = s.config.clone().ok_or_else(|| EngineError::Other("this session has no settings folder to install scripts into".into()))?;
    let dir = if panel { PANELS_DIR } else { SCRIPTS_DIR };
    cfg.write(&format!("{dir}/{name}"), &code).map_err(|e| EngineError::Other(format!("cannot install {name}: {e}")))?;
    // After Effects asks for a restart; our menus pick it up at once.
    s.toast(if panel {
        format!("Installed ScriptUI panel {name}: find it at the bottom of the Window menu")
    } else {
        format!("Installed script {name}: find it in File ▸ Scripts")
    });
    Ok(json!({"name": name, "panel": panel}))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!(scripts(s)))
}

fn uninstall(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "file.uninstallScript";
    let name = str_p(p, "name").ok_or_else(|| bad(c, "missing `name`"))?;
    let cfg = s.config.clone().ok_or_else(|| bad(c, "no settings folder"))?;
    let mut removed = false;
    for dir in [SCRIPTS_DIR, PANELS_DIR] {
        let key = format!("{dir}/{name}");
        if cfg.read(&key).is_some() {
            cfg.remove(&key).map_err(|e| EngineError::Other(e.to_string()))?;
            removed = true;
        }
    }
    if removed { Ok(json!({"removed": name})) } else { Err(bad(c, format!("`{name}` is not installed"))) }
}

/// Run an installed or bundled script by name (File ▸ Scripts ▸ <name>). A ScriptUI panel run
/// this way gets `this` = a floating palette-like panel window.
pub(crate) fn run_named(s: &mut Session, name: &str) -> Result<Value> {
    let (panel, code) = script_code(s, name, None).ok_or_else(|| bad("file.runScript", format!("no installed or bundled script `{name}`")))?;
    if panel {
        return open_panel(s, &json!({"name": name}));
    }
    super::file_more::run_js_named(s, &code, name)
}

/// The code a ScriptUI panel runs with: `this` is a dockable Panel titled after the script.
pub fn panel_wrapper(title: &str, code: &str) -> String {
    // On one line, so line numbers in errors stay those of the file.
    format!("(function () {{ {code}\n}}).call(__uiDockPanel({}));", Value::String(title.to_string()))
}

/// The ScriptUI panels open now (script names).
pub(crate) fn open_panel_names(s: &Session) -> Vec<String> {
    s.script_ui.windows.iter().filter(|w| w.kind == scriptui::WindowKind::Panel).map(|w| w.script.clone()).collect()
}

/// The panels open when EffectCraft last quit (`{"open": [names]}`); a missing or unreadable
/// file is none.
fn remembered_panels(s: &Session) -> Vec<String> {
    let text = s.config.as_ref().and_then(|c| c.read(OPEN_PANELS_FILE)).unwrap_or_default();
    let doc: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    doc.get("open").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter(|n| is_script(n)).map(str::to_string).collect()
}

/// Change the remembered panels with `f` (written only when it changed). The list is kept so
/// the next launch reopens them, as After Effects reopens the panels of its workspace; quitting
/// leaves it as it is.
fn edit_remembered_panels(s: &Session, f: impl FnOnce(&mut Vec<String>)) {
    let Some(cfg) = &s.config else { return };
    let before = remembered_panels(s);
    let mut list = before.clone();
    f(&mut list);
    if list != before
        && let Err(e) = cfg.write(OPEN_PANELS_FILE, &json!({"open": list}).to_string())
    {
        log::warn!("cannot remember the open ScriptUI panels: {e}");
    }
}

/// Forget the panels of `before` (open before a script window event) that are closed now.
pub(crate) fn forget_closed_panels(s: &Session, before: &[String]) {
    let now = open_panel_names(s);
    let closed: Vec<&String> = before.iter().filter(|n| !now.contains(n)).collect();
    if !closed.is_empty() {
        edit_remembered_panels(s, |l| l.retain(|n| !closed.contains(&n)));
    }
}

/// `window.restoreScriptPanels`: open the ScriptUI panels that were open when EffectCraft last
/// quit (frontends call it once at launch). Panels whose script is gone or fails are reported
/// and forgotten; the others still open.
fn restore_panels(s: &mut Session, _: &Value) -> Result<Value> {
    let (mut opened, mut failed) = (vec![], vec![]);
    for name in remembered_panels(s) {
        let r = match script_code(s, &name, Some(true)) {
            Some(_) => open_panel(s, &json!({"name": name})).map_err(|e| e.to_string()),
            None => Err("no such ScriptUI panel".to_string()),
        };
        match r {
            Ok(_) => opened.push(name),
            Err(error) => {
                edit_remembered_panels(s, |l| l.retain(|n| *n != name));
                failed.push(json!({"name": name, "error": error}));
            }
        }
    }
    Ok(json!({"opened": opened, "failed": failed}))
}

/// Window ▸ <ScriptUI panel>: run the panel script (or bring its panel forward).
fn open_panel(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "window.scriptPanel";
    let name = str_p(p, "name").ok_or_else(|| bad(c, "missing `name` (a script in the ScriptUI Panels folder)"))?.to_string();
    let title = name.trim_end_matches(".jsx").trim_end_matches(".js").to_string();
    if let Some(w) = s.script_ui.windows.iter().find(|w| w.kind == scriptui::WindowKind::Panel && w.script == name) {
        let id = w.id;
        s.events.push(Event::Frontend { command: c.into(), params: json!({"window": id}) });
        return Ok(json!({"window": id, "open": true}));
    }
    let (_, code) = script_code(s, &name, Some(true)).or_else(|| script_code(s, &name, None)).ok_or_else(|| bad(c, format!("no ScriptUI panel `{name}`")))?;
    let out = super::file_more::run_js_named(s, &panel_wrapper(&title, &code), &name)?;
    let id = s.script_ui.windows.iter().rev().find(|w| w.kind == scriptui::WindowKind::Panel && w.script == name).map(|w| w.id);
    if let Some(id) = id {
        s.events.push(Event::Frontend { command: c.into(), params: json!({"window": id}) });
    }
    if id.is_some() {
        edit_remembered_panels(s, |l| {
            if !l.contains(&name) {
                l.push(name.clone());
            }
        });
    }
    Ok(json!({"window": id, "result": out}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.installScript",
            "Install Script File...",
            ["File", "Scripts"],
            None,
            "{path (.jsx / .js)} → copies it to the Scripts folder; it appears in File ▸ Scripts",
            always,
            |s, p| install(s, p, false)
        ),
        cmd!(
            "file.installScriptUIPanel",
            "Install ScriptUI Panel...",
            ["File", "Scripts"],
            None,
            "{path (.jsx / .js)} → copies it to the ScriptUI Panels folder; it appears in the Window menu",
            always,
            |s, p| install(s, p, true)
        ),
        cmd!("file.uninstallScript", "Uninstall Script", [], None, "{name}", always, uninstall),
        crate::query!("file.scripts.list", "List Scripts", "{} → [{name, panel, source: installed|sample|extension}]", list),
        cmd!(
            "window.scriptPanel",
            "ScriptUI Panel",
            [],
            None,
            "{name (a script in the ScriptUI Panels folder, e.g. `Layer Tools.jsx`)} → opens it as a dockable panel",
            always,
            open_panel
        ),
        cmd!(
            "window.restoreScriptPanels",
            "Restore ScriptUI Panels",
            [],
            None,
            "{} → reopens the ScriptUI panels open when EffectCraft last quit (frontends call it at launch) → {opened: [name], failed: [{name, error}]}",
            always,
            restore_panels
        ),
        crate::query!("script.settings.get", "Get Script Setting", "{section, key} → {have, value} (app.settings.haveSetting / getSetting)", settings_get),
        cmd!(
            "script.settings.save",
            "Save Script Setting",
            [],
            None,
            "{section, key, value (text)} → kept in the settings store (app.settings.saveSetting)",
            always,
            settings_save
        ),
        crate::query!("scriptui.list", "List Script Windows", "{} → [{window, title, kind: dialog|palette|window|panel, script, modal, size}]", scriptui::list),
        crate::query!(
            "scriptui.get",
            "Script Window Controls",
            "{window?: id | title} → {id, title, kind, root: {id, type, name, text, value, checked, items, selection, bounds, enabled, draw (onDraw paint list), children…}}",
            scriptui::get
        ),
        cmd!(
            "scriptui.click",
            "Click Script Window Control",
            [],
            None,
            "{window?: id | title, widget: id | \"#id\" | properties.name | text} (buttons, checkboxes, radio buttons, tabs)",
            always,
            scriptui::click
        ),
        cmd!(
            "scriptui.set",
            "Set Script Window Control",
            [],
            None,
            "{window?, widget, value: text | number | bool | item index | item text, changing?: bool (a live update: each keystroke / slider step; fires onChanging only)} (edit text, sliders, checkboxes, lists) → fires onChanging / onChange",
            always,
            scriptui::set
        ),
        cmd!(
            "scriptui.close",
            "Close Script Window",
            [],
            None,
            "{window?, result? (a dialog's show() returns it; default 2 = Cancel)}",
            always,
            scriptui::close
        ),
    ]
}
