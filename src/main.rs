#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod layout;
mod pdf_export;
mod pdfium_lib;
mod print_win;
mod sources;
mod theme;

use app::OpenImpApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([980.0, 640.0])
            .with_title("OpenImp"),
        ..Default::default()
    };
    eframe::run_native(
        "OpenImp",
        options,
        Box::new(|cc| Ok(Box::new(OpenImpApp::new(cc)))),
    )
}
