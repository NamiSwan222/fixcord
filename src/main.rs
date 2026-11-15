#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod media;
mod model;
mod net;
mod snake;
mod theme;
mod update;

use std::sync::{Arc, Mutex};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let shot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Hearth")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([940.0, 560.0]);

    if shot.is_some() {
        // Open quietly: no focus, and on Windows behind other windows.
        viewport = viewport.with_active(false);
        #[cfg(target_os = "windows")]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, persist_window: shot.is_none(), ..Default::default() };

    #[cfg(target_os = "macos")]
    if shot.is_some() {
        options.event_loop_builder = Some(Box::new(|builder| {
            use eframe::egui_winit::winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        }));
    }

    let job = shot.map(|path| app::ScreenshotJob { path, frames: 0, requested: false, slot: Arc::new(Mutex::new(None)) });

    eframe::run_native("Hearth", options, Box::new(move |_cc| Ok(Box::new(app::HearthApp::new(job)))))
}
