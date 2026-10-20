//! The "Clean up PC" window: choose what the cleanup removes, see the sizes first, then clean.

use std::time::Duration;

use eframe::egui::{self, Align, CornerRadius, Layout, RichText, StrokeKind, Vec2, ViewportCommand, ViewportId};

use crate::{
    app::{Anchor, POPUP_BORDER, popup_builder},
    cleanup::{Category, Choices, Cleaner, Task},
    format, platform,
};

/// Used to place the window before its real size is known; it then resizes itself to fit.
const INITIAL_SIZE: Vec2 = Vec2::new(340.0, 320.0);
/// Sizes shown are re-measured when they are older than this.
pub const SCAN_MAX_AGE: Duration = Duration::from_secs(60);

pub struct CleanupWindow {
    anchor: Anchor,
    x: f32,
    size: Vec2,
}

impl CleanupWindow {
    /// Opens next to the strip, `x` points from its left edge, and measures what can be freed.
    pub fn new(anchor: Anchor, x: f32, cleaner: &Cleaner) -> Self {
        cleaner.scan_if_older(SCAN_MAX_AGE);
        Self { anchor, x, size: INITIAL_SIZE }
    }

    /// Draws the window. Returns false once the user closed it.
    pub fn show(&mut self, ctx: &egui::Context, cleaner: &Cleaner, choices: &mut Choices) -> bool {
        let mut open = true;
        let position = self.anchor.popup_position(self.x, self.size);
        let builder = popup_builder("Compteur cleanup", position, self.size);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("cleanup"), builder, |ui, _class| {
            if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
                open = false;
            }
            let panel = egui::CentralPanel::default().show(ui, |ui| {
                open &= contents(ui, cleaner, choices);
                ui.min_rect().size()
            });
            ui.painter().rect_stroke(ui.max_rect(), CornerRadius::ZERO, POPUP_BORDER, StrokeKind::Inside);
            // Fit the window to its contents plus the panel's 8 pt margin on each side.
            let wanted = (panel.inner + Vec2::splat(16.0)).ceil();
            if (self.size - wanted).length() > 1.0 {
                self.size = wanted;
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(wanted));
                ui.ctx().send_viewport_cmd(ViewportCommand::OuterPosition(self.anchor.popup_position(self.x, wanted)));
            }
        });
        open
    }
}

/// Returns false when the close button was clicked.
fn contents(ui: &mut egui::Ui, cleaner: &Cleaner, choices: &mut Choices) -> bool {
    let mut open = true;
    let busy = cleaner.busy();
    let scan = cleaner.scan();

    ui.horizontal(|ui| {
        ui.label(RichText::new("Clean up PC").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            open = !ui.small_button("×").on_hover_text("Close (Esc)").clicked();
        });
    });
    ui.label(RichText::new("Frees space without touching the apps you're using.").small().weak());
    ui.separator();

    egui::Grid::new("cleanup-categories").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
        for category in Category::ALL {
            let mut on = choices.get(category);
            if ui.checkbox(&mut on, category.label()).on_hover_text(category.hint()).changed() {
                choices.set(category, on);
            }
            let outcome = scan.as_ref().and_then(|scan| scan.get(category));
            let size = match outcome {
                None => "…".to_owned(),
                Some(outcome) if category == Category::Memory => format!("{} in use", format::bytes(outcome.bytes)),
                Some(outcome) => format::bytes(outcome.bytes),
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(size));
            ui.end_row();
            for note in outcome.map(|o| o.notes.as_slice()).unwrap_or_default() {
                ui.label(RichText::new(format!("      {note}")).small().weak());
                ui.end_row();
            }
        }
    });
    ui.separator();

    match busy {
        Some(task) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if task == Task::Scanning { "Measuring…" } else { "Cleaning…" });
            });
        }
        None => {
            let text = match &scan {
                Some(scan) => format!("Clean now · frees about {}", format::bytes(scan.disk_bytes(*choices))),
                None => "Clean now".to_owned(),
            };
            let button = egui::Button::new(RichText::new(text).strong()).min_size(Vec2::new(ui.available_width(), 28.0));
            if ui.add_enabled(choices.any(), button).clicked() {
                cleaner.clean(*choices);
            }
        }
    }
    if let Some((_, report)) = cleaner.report() {
        ui.label(RichText::new("Last clean").small().strong());
        for line in report.lines() {
            ui.label(RichText::new(line).small());
        }
    }

    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .link("Windows system files…")
            .on_hover_text("Opens Windows Disk Cleanup for Windows Update leftovers and other system files (asks for admin).")
            .clicked()
        {
            platform::open_disk_cleanup();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(busy.is_none(), egui::Button::new("Measure again").small()).clicked() {
                cleaner.scan_if_older(Duration::ZERO);
            }
        });
    });
    open
}
