//! The desktop app: an egui/eframe window styled as frosted glass.
//!
//! The UI is deliberately thin. It draws `editor` state and sends
//! `Command`s, so everything it can do is also covered by the headless
//! scenario tests and reachable over MCP.

mod app;
mod canvas;
mod file_browser;
mod panels;
mod settings;
mod shortcuts;
mod textures;
mod theme;
mod tools;
mod workers;

use std::path::PathBuf;

pub fn run(initial: Option<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Spry")
            .with_app_id("spry")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Spry", options, Box::new(move |cc| Ok(Box::new(app::App::new(&cc.egui_ctx, initial)))))
}
