// Release builds are GUI-only; debug builds keep a console for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod controls;
mod format;
mod icon;
mod metrics;
mod pdh;
mod platform;
mod sampler;

use std::sync::Arc;

use eframe::egui;

fn main() -> eframe::Result {
    if !platform::claim_single_instance() {
        return Ok(()); // the running copy is reachable from the tray
    }
    let config = config::Config::load();

    let icon = egui::IconData {
        rgba: icon::window_icon_rgba(),
        width: icon::WINDOW_ICON_SIZE,
        height: icon::WINDOW_ICON_SIZE,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Compteur")
        .with_icon(Arc::new(icon))
        .with_inner_size([360.0, app::STRIP_HEIGHT])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_taskbar(false)
        .with_active(false)
        .with_mouse_passthrough(config.click_through);
    if let Some(position) = config.position {
        viewport = viewport.with_position(position);
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("compteur", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, config)))))
}
