//! LaserForge desktop app in Rust: egui shell, canvas, layers, laser panel and GRBL connection.
#![allow(deprecated)]
pub mod app;
pub mod bitmap;
pub mod bundle;
pub mod camera;
pub mod camera_ui;
pub mod canvas;
pub mod connection;
pub mod corners;
pub mod defs;
pub mod dialogs;
pub mod dialogs2;
pub mod edit;
pub mod importers;
pub mod library;
pub mod mesh3d;
pub mod native_menu;
pub mod nest;
pub mod panels;
pub mod pcb;
pub mod prefs;
pub mod project;
pub mod remote;
pub mod shapes;
pub mod shx;
pub mod text;
pub mod tools;

pub use defs::{l, tr};
