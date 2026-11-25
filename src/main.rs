#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod scanner;
mod theme;
mod win32;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let icon = build_app_icon();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WhatsGoingOn — Terminal & Shell Monitor")
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

/// Generate a crisp 32x32 RGBA terminal-radar icon at runtime so no external asset is needed.
fn build_app_icon() -> egui::IconData {
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let dist = (dx * dx + dy * dy).sqrt();

            // Rounded dark slate tile with glowing emerald/blue terminal prompt accent
            if dist <= 15.0 {
                let is_ring = (11.5..=13.5).contains(&dist);
                let is_dot = dist <= 4.0;
                let (r, g, b, a) = if is_dot {
                    (46, 204, 138, 255)
                } else if is_ring {
                    (79, 142, 255, 255)
                } else {
                    (18, 22, 33, 255)
                };
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = a;
            }
        }
    }
    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}
