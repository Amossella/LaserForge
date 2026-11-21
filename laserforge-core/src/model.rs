//! Layer and machine settings. Field names and JSON keys match the Python LaserForge, so saved
//! settings and projects load unchanged.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirAssist {
    Off,
    On,
}

impl AirAssist {
    pub fn as_str(self) -> &'static str {
        match self {
            AirAssist::Off => "off",
            AirAssist::On => "on",
        }
    }
}

impl Serialize for AirAssist {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AirAssist {
    /// Old files used "m7" / "m8" / true for "on"; false, null, "", 0, "0", "none" and "off" mean off.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let off = match &v {
            Value::Null => true,
            Value::Bool(b) => !b,
            Value::Number(n) => n.as_f64() == Some(0.0),
            Value::String(s) => matches!(s.as_str(), "" | "0" | "none" | "off" | "OFF" | "Off"),
            _ => false,
        };
        Ok(if off { AirAssist::Off } else { AirAssist::On })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaserMode {
    #[serde(rename = "M3")]
    Constant,
    #[serde(rename = "M4")]
    Dynamic,
}

impl LaserMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LaserMode::Constant => "M3",
            LaserMode::Dynamic => "M4",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutMode {
    Line,
    Fill,
    OffsetFill,
    LineFill,
}

impl CutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CutMode::Line => "line",
            CutMode::Fill => "fill",
            CutMode::OffsetFill => "offset_fill",
            CutMode::LineFill => "line_fill",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerSettings {
    pub name: String,
    pub color: String,
    pub speed: f64,
    pub power_min: f64,
    pub power_max: f64,
    pub passes: i64,
    pub cut_mode: CutMode,
    pub air_assist: AirAssist,
    pub air_power: f64,
    pub air_cut_only: bool,
    pub z_offset: f64,
    pub laser_mode: LaserMode,
    pub enabled: bool,
    pub line_interval: f64,
    pub overscan: f64,
    pub overscan_percent: bool,
    pub scan_angle: f64,
    pub cross_hatch: bool,
    pub bidir_scan: bool,
    pub dwell_time: f64,
    pub start_delay: f64,
    pub end_delay: f64,
    pub ramp_length: f64,
    pub tab_count: i64,
    pub tab_width: f64,
    pub kerf_offset: f64,
    pub power_scale: f64,
    pub dot_mode: bool,
    pub dot_time: f64,
    pub z_step: f64,
    pub tool: bool,
    pub lead_in: f64,
    pub lead_out: f64,
    pub overcut: f64,
    pub perforate: bool,
    pub perf_cut: f64,
    pub perf_skip: f64,
    pub flood_fill: bool,
    pub sublayers: Vec<Value>,
    pub pass_through: bool,
    pub dot_width: f64,
    pub bottom_up: bool,
}

impl Default for LayerSettings {
    fn default() -> Self {
        LayerSettings {
            name: "C00".into(),
            color: "#000000".into(),
            speed: 1000.0,
            power_min: 0.0,
            power_max: 100.0,
            passes: 1,
            cut_mode: CutMode::Line,
            air_assist: AirAssist::On,
            air_power: 100.0,
            air_cut_only: false,
            z_offset: 0.0,
            laser_mode: LaserMode::Dynamic,
            enabled: true,
            line_interval: 0.1,
            overscan: 2.5,
            overscan_percent: false,
            scan_angle: 0.0,
            cross_hatch: false,
            bidir_scan: true,
            dwell_time: 0.0,
            start_delay: 0.0,
            end_delay: 0.0,
            ramp_length: 0.0,
            tab_count: 0,
            tab_width: 2.0,
            kerf_offset: 0.0,
            power_scale: 100.0,
            dot_mode: false,
            dot_time: 1.0,
            z_step: 0.0,
            tool: false,
            lead_in: 0.0,
            lead_out: 0.0,
            overcut: 0.0,
            perforate: false,
            perf_cut: 1.0,
            perf_skip: 1.0,
            flood_fill: false,
            sublayers: Vec::new(),
            pass_through: false,
            dot_width: 0.0,
            bottom_up: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineSettings {
    pub max_speed: f64,
    pub max_power: i64,
    pub workspace_x: f64,
    pub workspace_y: f64,
    pub home_on_start: bool,
    pub return_to_origin: bool,
    pub baud_rate: i64,
    pub air_assist_default: AirAssist,
    pub use_g0_overscan: bool,
    pub frame_power: f64,
    pub rotary_enabled: bool,
    pub rotary_type: String,
    pub rotary_mm_per_rev: f64,
    pub rotary_diameter: f64,
    pub start_gcode: String,
    pub end_gcode: String,
    pub flavor: String,
    pub origin: String,
    pub air_command: String,
    pub marlin_air_fan: bool,
    pub transfer_mode: String,
    pub acceleration: f64,
    pub backlash_x: f64,
    pub backlash_y: f64,
    pub finish_position: String,
    pub finish_x: f64,
    pub finish_y: f64,
    pub fire_power: f64,
    pub fire_enabled: bool,
    pub z_enabled: bool,
    pub z_speed: f64,
    pub z_relative_only: bool,
    pub rotary_axis: String,
    pub raster_relative: bool,
    pub fast_whitespace: bool,
    pub whitespace_speed: f64,
    pub scan_offset_enabled: bool,
    pub scan_offsets: String,
    pub feeder_enabled: bool,
    pub feeder_axis: String,
    pub feeder_distance: f64,
    pub feeder_repeats: i64,
    pub feeder_speed: f64,
    pub feeder_gcode: String,
}

impl Default for MachineSettings {
    fn default() -> Self {
        MachineSettings {
            max_speed: 6000.0,
            max_power: 1000,
            workspace_x: 400.0,
            workspace_y: 400.0,
            home_on_start: true,
            return_to_origin: true,
            baud_rate: 115200,
            air_assist_default: AirAssist::On,
            use_g0_overscan: false,
            frame_power: 1.0,
            rotary_enabled: false,
            rotary_type: "roller".into(),
            rotary_mm_per_rev: 100.0,
            rotary_diameter: 50.0,
            start_gcode: String::new(),
            end_gcode: String::new(),
            flavor: "grbl".into(),
            origin: "fl".into(),
            air_command: "M8".into(),
            marlin_air_fan: false,
            transfer_mode: "buffered".into(),
            acceleration: 1000.0,
            backlash_x: 0.0,
            backlash_y: 0.0,
            finish_position: "origin".into(),
            finish_x: 0.0,
            finish_y: 0.0,
            fire_power: 1.0,
            fire_enabled: false,
            z_enabled: false,
            z_speed: 5.0,
            z_relative_only: false,
            rotary_axis: "Y".into(),
            raster_relative: true,
            fast_whitespace: false,
            whitespace_speed: 6000.0,
            scan_offset_enabled: false,
            scan_offsets: String::new(),
            feeder_enabled: false,
            feeder_axis: "A".into(),
            feeder_distance: 100.0,
            feeder_repeats: 1,
            feeder_speed: 1000.0,
            feeder_gcode: String::new(),
        }
    }
}

/// A point of a vector path (machine coordinates, mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

/// A path plus optional manual tab positions (fractions of its length).
#[derive(Debug, Clone, Default)]
pub struct GPath {
    pub pts: Vec<Pt>,
    pub tabs: Option<Vec<f64>>,
}

impl GPath {
    pub fn new(pts: Vec<Pt>) -> Self {
        GPath { pts, tabs: None }
    }
}

/// A raster image already in machine orientation: row 0 at the lowest machine Y, 0 = no burn, 255 = full.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Raster {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub cols: usize,
    pub rows: usize,
    pub data: Vec<u8>,
    #[serde(default)]
    pub angle: Option<f64>,
}
