//! User preferences (Settings dialog), same keys and defaults as laserforge/prefs.py so exported preference
//! files work in both apps.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub units: String,
    pub speed_units: String,
    pub theme: String,
    pub ui_font: f64,
    pub icon_size: f64,
    pub backup_enabled: bool,
    pub backup_minutes: f64,
    pub backup_keep: usize,
    pub grid_show: bool,
    pub grid_spacing: f64,
    pub grid_snap: bool,
    pub snap_step: f64,
    pub obj_snap: bool,
    pub obj_snap_px: f64,
    pub click_tol: f64,
    pub trackpad_scroll: String,
    pub node_size: f64,
    pub curve_quality: f64,
    pub nudge: f64,
    pub nudge_shift: f64,
    pub nudge_fine: f64,
    pub cut_selected: bool,
    pub frame_laser_on: bool,
    pub frame_power: f64,
    pub checklist_enabled: bool,
    pub shx_folder: String,
    pub default_start_from: String,
    pub default_origin: i64,
    pub origin_from_selection: bool,
    pub ui_scale: f64,
    pub remote_enabled: bool,
    pub remote_port: u16,
    pub remote_pin: String,
    pub camera_quality: f64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            units: "mm".into(),
            speed_units: "mm/min".into(),
            theme: "dark".into(),
            ui_font: 12.0,
            icon_size: 20.0,
            backup_enabled: true,
            backup_minutes: 5.0,
            backup_keep: 20,
            grid_show: true,
            grid_spacing: 0.0,
            grid_snap: true,
            snap_step: 1.0,
            obj_snap: true,
            obj_snap_px: 8.0,
            click_tol: 5.0,
            trackpad_scroll: "zoom".into(),
            node_size: 7.0,
            curve_quality: 0.25,
            nudge: 1.0,
            nudge_shift: 10.0,
            nudge_fine: 0.1,
            cut_selected: false,
            frame_laser_on: false,
            frame_power: 1.0,
            checklist_enabled: false,
            shx_folder: String::new(),
            default_start_from: "last".into(),
            default_origin: -1,
            origin_from_selection: false,
            ui_scale: 100.0,
            remote_enabled: false,
            remote_port: 8642,
            remote_pin: String::new(),
            camera_quality: 1600.0,
        }
    }
}

impl Prefs {
    /// Display units per mm (1 for mm, 1/25.4 for inches).
    pub fn unit_factor(&self) -> f64 {
        if self.units == "in" { 1.0 / 25.4 } else { 1.0 }
    }
    pub fn unit_suffix(&self) -> &'static str {
        if self.units == "in" { " in" } else { " mm" }
    }
    /// Display speed units per mm/min.
    pub fn speed_factor(&self) -> f64 {
        match self.speed_units.as_str() {
            "mm/s" => 1.0 / 60.0,
            "in/min" => 1.0 / 25.4,
            _ => 1.0,
        }
    }
    pub fn fmt_len(&self, mm: f64) -> String {
        if self.units == "in" { format!("{:.3} in", mm / 25.4) } else { format!("{mm:.2} mm") }
    }
}
