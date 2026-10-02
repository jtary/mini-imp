#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod layout;
mod pdf_export;
mod pdfium_lib;
mod print_win;
mod sources;

use app::OpenImpApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([860.0, 620.0])
            .with_title("OpenImp"),
        ..Default::default()
    };
    eframe::run_native(
        "OpenImp",
        options,
        Box::new(|cc| Ok(Box::new(OpenImpApp::new(cc)))),
    )
}
