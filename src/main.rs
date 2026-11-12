#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod arrange;
mod history;
mod model;
mod pdf_engine;
mod renderer;
mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PDF Wizard")
            .with_inner_size([1100.0, 750.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PDF Wizard",
        options,
        Box::new(|cc| Ok(Box::new(app::PdfWizardApp::new(cc)))),
    )
}
