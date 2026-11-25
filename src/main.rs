#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod scanner;
mod theme;
mod win32;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let icon = load_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WhatsGoingOn (wgo) — Terminal & Shell Monitor")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([920.0, 600.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "WhatsGoingOn",
        options,
        Box::new(|cc| Ok(Box::new(app::WhatsGoingOnApp::new(cc)))),
    )
}

fn load_app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes)
        .expect("embedded assets/icon.png should be valid PNG")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}
