//! macOS: the application menus live in the system menu bar at the top of the screen, like every Mac app.
//! Other systems keep the menu bar inside the window (drawn by egui).
//!
//! Menu clicks are queued here and run by the app on its next frame; the queue wakes the window.

use crate::defs::{self, MenuItem};

/// Actions whose shortcut stays with the window: text fields need ⌘C / ⌘V / ⌘Z / ⌘A themselves.
const KEEP_IN_WINDOW: &[&str] = &["undo", "redo", "cut", "copy", "paste", "pasteInPlace", "selectAll", "delete"];

/// "Ctrl+Shift+S" (defs notation) → a menu accelerator; None for keys without a modifier, which would steal typing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn accel_text(key: &str) -> Option<String> {
    if key.is_empty() || !(key.contains("Ctrl") || key.contains("Alt")) {
        return None;
    }
    let parts: Vec<String> = key.split('+').filter(|p| !p.is_empty()).map(|p| match p {
        "Ctrl" => "CmdOrCtrl".to_string(),
        "Alt" => "Alt".to_string(),
        "Shift" => "Shift".to_string(),
        "Del" => "Delete".to_string(),
        "Up" => "ArrowUp".to_string(),
        "Down" => "ArrowDown".to_string(),
        k => k.to_string(),
    }).collect();
    // "Ctrl+=" / "Ctrl+-" split oddly on '+': rebuild the key when it was the last character
    let mut s = parts.join("+");
    if key.ends_with("++") {
        s.push_str("+Plus");
    }
    Some(s)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use muda::accelerator::Accelerator;
    use muda::{Menu, MenuEvent, MenuItem as Item, PredefinedMenuItem, Submenu};
    use std::sync::{Mutex, OnceLock};

    static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static HANDLER: OnceLock<()> = OnceLock::new();

    pub struct NativeMenu {
        _menu: Menu,
        pub sig: String,
    }

    fn label(it: &str, en: &str) -> String {
        if defs::english() { en.to_string() } else { it.to_string() }
    }

    fn fill(sub: &Submenu, items: &[MenuItem], recent: &[String], shortcut: &dyn Fn(&str) -> String) {
        for it in items {
            match it {
                MenuItem::Action { id, it, en, .. } => {
                    let acc = if KEEP_IN_WINDOW.contains(&id.as_str()) || id == "exit" || id == "settings" || id == "about" {
                        None
                    } else {
                        accel_text(&shortcut(id)).and_then(|a| a.parse::<Accelerator>().ok())
                    };
                    let _ = sub.append(&Item::with_id(id.as_str(), label(it, en), true, acc));
                }
                MenuItem::Sub { it, en, items } => {
                    let s = Submenu::new(label(it, en), true);
                    fill(&s, items, recent, shortcut);
                    let _ = sub.append(&s);
                }
                MenuItem::Recent { it, en } => {
                    let s = Submenu::new(label(it, en), true);
                    if recent.is_empty() {
                        let _ = s.append(&Item::with_id("recent:", label("(vuoto)", "(empty)"), false, None));
                    }
                    for p in recent {
                        let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(p.clone());
                        let _ = s.append(&Item::with_id(format!("recent:{p}"), name, true, None));
                    }
                    let _ = sub.append(&s);
                }
                MenuItem::Sep => {
                    let _ = sub.append(&PredefinedMenuItem::separator());
                }
            }
        }
    }

    /// Build the menu bar and install it as the application's main menu.
    pub fn build(recent: &[String], shortcut: &dyn Fn(&str) -> String, sig: String, ctx: &egui::Context) -> Option<NativeMenu> {
        if HANDLER.get().is_none() {
            let c = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                if let Ok(mut q) = EVENTS.lock() {
                    q.push(e.id.0.clone());
                }
                c.request_repaint();
            }));
            let _ = HANDLER.set(());
        }
        let menu = Menu::new();
        // the application menu (shown in bold with the app name)
        let app = Submenu::new("LaserForge", true);
        let _ = app.append_items(&[
            &Item::with_id("about", label("Informazioni su LaserForge", "About LaserForge"), true, None),
            &PredefinedMenuItem::separator(),
            &Item::with_id("settings", label("Impostazioni...", "Settings..."), true, "CmdOrCtrl+,".parse::<Accelerator>().ok()),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(&label("Nascondi LaserForge", "Hide LaserForge"))),
            &PredefinedMenuItem::hide_others(Some(&label("Nascondi altre", "Hide Others"))),
            &PredefinedMenuItem::show_all(Some(&label("Mostra tutte", "Show All"))),
            &PredefinedMenuItem::separator(),
            // our own Quit: it asks about unsaved changes first
            &Item::with_id("exit", label("Esci da LaserForge", "Quit LaserForge"), true, "CmdOrCtrl+Q".parse::<Accelerator>().ok()),
        ]);
        let _ = menu.append(&app);
        for m in defs::menus() {
            let sub = Submenu::new(label(&m.it, &m.en), true);
            fill(&sub, &m.items, recent, shortcut);
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();
        Some(NativeMenu { _menu: menu, sig })
    }

    pub fn take_events() -> Vec<String> {
        EVENTS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub struct NativeMenu {
        pub sig: String,
    }
    pub fn build(_: &[String], _: &dyn Fn(&str) -> String, _: String, _: &egui::Context) -> Option<NativeMenu> {
        None
    }
    pub fn take_events() -> Vec<String> {
        Vec::new()
    }
}

pub use imp::{build, take_events, NativeMenu};

/// True where the menus go in the system menu bar.
pub const SYSTEM_MENU_BAR: bool = cfg!(target_os = "macos");
