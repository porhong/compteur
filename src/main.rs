// Release builds are GUI-only; debug builds keep a console for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod format;
mod metrics;
mod pdh;
mod sampler;

use eframe::egui;

fn main() -> eframe::Result {
    let config = config::Config::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Compteur")
        .with_inner_size([360.0, app::STRIP_HEIGHT])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_taskbar(false)
        .with_active(false);
    if let Some(position) = config.position {
        viewport = viewport.with_position(position);
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("compteur", options, Box::new(|cc| Ok(Box::new(app::App::new(cc, config)))))
}
