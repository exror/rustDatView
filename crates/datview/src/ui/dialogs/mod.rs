//! Modal dialogs. Each dialog owns its draft state and applies changes to the
//! workspace only when the user confirms.
pub(crate) mod analysis;
pub(crate) mod formula;
pub(crate) mod pipeline;

use eframe::egui::{self, RichText, Ui};

use super::{icons, theme, widgets};

/// A modal with a title row and close button. `add` receives a flag it may set
/// to close the dialog. Returns the body's value and whether the dialog closed
/// (close button, Escape, or a click outside).
pub(crate) fn modal<R>(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    width: f32,
    add: impl FnOnce(&mut Ui, &mut bool) -> R,
) -> (R, bool) {
    let mut close = false;
    let response = egui::Modal::new(egui::Id::unique(id)).show(ctx, |ui| {
        ui.set_width(width);
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).family(theme::semibold()).size(17.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::CLOSE, "Close · Esc").clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(8.0);
        add(ui, &mut close)
    });
    let closed = close || response.should_close();
    (response.inner, closed)
}

/// The single emphasized action of a dialog.
pub(crate) fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
    let button = egui::Button::new(RichText::new(text).color(egui::Color32::WHITE))
        .fill(if enabled {
            tokens.accent
        } else {
            tokens.accent.gamma_multiply(0.4)
        })
        .min_size(egui::vec2(96.0, 30.0));
    ui.add_enabled(enabled, button)
}

/// Muted explanatory text under a field.
pub(crate) fn hint(ui: &mut Ui, text: &str) {
    let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
    ui.label(RichText::new(text).size(11.5).color(tokens.text_muted));
}
