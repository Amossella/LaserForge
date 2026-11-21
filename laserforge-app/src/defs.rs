//! Menus, toolbars, icons and interface strings: the same tables as laserforge/defs.py (exported to defs.json),
//! so the Rust app has the same commands, labels and shortcuts as the Python app.

use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(en: bool) {
    ENGLISH.store(en, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

pub fn defs() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("defs.json")).expect("defs.json"))
}

/// Italian or English text depending on the interface language.
pub fn l(it: &str, en: &str) -> String {
    if english() { en.to_string() } else { it.to_string() }
}

/// I18N table lookup (falls back to the key).
pub fn tr(key: &str) -> String {
    let lang = if english() { "en" } else { "it" };
    defs()["I18N"][lang][key].as_str().or_else(|| defs()["I18N"]["it"][key].as_str()).unwrap_or(key).to_string()
}

#[derive(Debug, Clone)]
pub enum MenuItem {
    Action { id: String, it: String, en: String, key: String },
    Sub { it: String, en: String, items: Vec<MenuItem> },
    Recent { it: String, en: String },
    Sep,
}

#[derive(Debug, Clone)]
pub struct Menu {
    pub id: String,
    pub it: String,
    pub en: String,
    pub items: Vec<MenuItem>,
}

fn items(v: &Value) -> Vec<MenuItem> {
    v.as_array().map(|a| a.iter().filter_map(|it| {
        if it.as_str() == Some("-") {
            return Some(MenuItem::Sep);
        }
        if let Some(a) = it.as_array() {
            let s = |i: usize| a.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
            return Some(MenuItem::Action { id: s(0), it: s(1), en: s(2), key: s(3) });
        }
        let o = it.as_object()?;
        let s = |k: &str| o.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if o.get("dynamic").is_some() {
            return Some(MenuItem::Recent { it: s("it"), en: s("en") });
        }
        Some(MenuItem::Sub { it: s("it"), en: s("en"), items: items(o.get("items")?) })
    }).collect()).unwrap_or_default()
}

pub fn menus() -> &'static [Menu] {
    static M: OnceLock<Vec<Menu>> = OnceLock::new();
    M.get_or_init(|| defs()["MENUS"].as_array().unwrap().iter().map(|m| Menu {
        id: m["id"].as_str().unwrap_or("").into(),
        it: m["it"].as_str().unwrap_or("").into(),
        en: m["en"].as_str().unwrap_or("").into(),
        items: items(&m["items"]),
    }).collect())
}

fn find<'a>(items: &'a [MenuItem], id: &str) -> Option<&'a MenuItem> {
    for it in items {
        match it {
            MenuItem::Action { id: i, .. } if i == id => return Some(it),
            MenuItem::Sub { items, .. } => {
                if let Some(f) = find(items, id) {
                    return Some(f);
                }
            }
            _ => {}
        }
    }
    None
}

/// Menu label of an action (EXTRA_LABELS for toolbar-only actions).
pub fn label(id: &str) -> String {
    for m in menus() {
        if let Some(MenuItem::Action { it, en, .. }) = find(&m.items, id) {
            return if english() { en.clone() } else { it.clone() };
        }
    }
    let e = &defs()["EXTRA_LABELS"][id];
    e[if english() { 1 } else { 0 }].as_str().unwrap_or(id).to_string()
}

/// Default shortcut text of an action ("Ctrl+N", "Alt+Shift+U", ...), empty when none.
pub fn shortcut(id: &str) -> String {
    for m in menus() {
        if let Some(MenuItem::Action { key, .. }) = find(&m.items, id) {
            return key.clone();
        }
    }
    String::new()
}

/// Toolbar entries: (action id, icon name), None for a separator.
pub fn toolbar(name: &str) -> Vec<Option<(String, String)>> {
    defs()[name].as_array().map(|a| a.iter().map(|b| {
        let arr = b.as_array()?;
        Some((arr.first()?.as_str()?.to_string(), arr.get(1)?.as_str()?.to_string()))
    }).collect()).unwrap_or_default()
}

/// Icon as polylines in a 24×24 box (from the SVG fragments of ICON_PATHS).
pub fn icon(name: &str) -> &'static [(Vec<[f64; 2]>, bool)] {
    static C: OnceLock<std::collections::HashMap<String, Vec<(Vec<[f64; 2]>, bool)>>> = OnceLock::new();
    let map = C.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        if let Some(o) = defs()["ICON_PATHS"].as_object() {
            for (k, v) in o {
                let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\">{}</svg>", v.as_str().unwrap_or(""));
                let mut polys = Vec::new();
                if let Ok(shapes) = crate::importers::svg_shapes(&svg) {
                    for s in shapes {
                        let sh = crate::shapes::Shape(s);
                        for p in sh.paths() {
                            polys.push((p.iter().map(|q| [q[0] * 96.0 / 25.4, q[1] * 96.0 / 25.4]).collect(), false));
                        }
                    }
                }
                m.insert(k.clone(), polys);
            }
        }
        m
    });
    map.get(name).map(|v| v.as_slice()).unwrap_or(&[])
}
