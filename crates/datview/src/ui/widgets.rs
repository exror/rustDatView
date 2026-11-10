//! Small, reusable shell controls sharing the app's typography and colors.
use eframe::egui::{self, Color32, Response, RichText, Stroke, Ui};

use super::{
    icons,
    theme::{self, Tokens},
};

/// A compact pill group of mutually exclusive choices with tooltips. Returns
/// the newly chosen value. Works inside right-to-left layouts too: choices are
/// always shown in the given reading order.
pub(crate) fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: T,
    choices: &[(T, &str, &str)],
) -> Option<T> {
    let tokens = Tokens::for_dark(theme::is_dark(ui.ctx()));
    let mut chosen = None;
    egui::Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(7)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let reversed = ui.layout().prefer_right_to_left();
                let mut add = |&(choice, label, tooltip): &(T, &str, &str)| {
                    let selected = value == choice;
                    let text = RichText::new(label).size(12.5).color(if selected {
                        tokens.text
                    } else {
                        tokens.text_secondary
                    });
                    let button = egui::Button::new(text)
                        .fill(if selected {
                            tokens.accent_soft
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(if selected {
                            Stroke::new(1.0, tokens.accent.gamma_multiply(0.45))
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(5)
                        .min_size(egui::vec2(0.0, 24.0));
                    let response = ui.add(button);
                    let response = if tooltip.is_empty() {
                        response
                    } else {
                        response.on_hover_text(tooltip)
                    };
                    if response.clicked() && !selected {
                        chosen = Some(choice);
                    }
                };
                if reversed {
                    choices.iter().rev().for_each(&mut add);
                } else {
                    choices.iter().for_each(&mut add);
                }
            });
        });
    chosen
}

/// Icon glyphs use their own family, avoiding conflicts with user-selected PUA glyphs.
pub(crate) fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(icon).family(theme::icon_family()).size(16.0))
            .frame_when_inactive(false)
            .min_size(egui::vec2(28.0, 28.0)),
    )
    .on_hover_text(tooltip)
}

pub(crate) fn section_label(ui: &mut Ui, text: &str) -> Response {
    let color = Tokens::for_dark(theme::is_dark(ui.ctx())).text_muted;
    ui.label(
        RichText::new(text.to_uppercase())
            .family(theme::semibold())
            .size(11.0)
            .extra_letter_spacing(0.7)
            .color(color),
    )
}

/// Search icon, editable text, and a clear button inside one rounded field.
/// The field takes exactly the available width so it can never push a
/// resizable panel wider frame after frame. Returns the text edit response.
pub(crate) fn search_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let tokens = Tokens::for_dark(theme::is_dark(ui.ctx()));
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let background = ui.painter().add(egui::Shape::Noop);
    ui.painter().text(
        egui::pos2(rect.left() + 11.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icons::SEARCH,
        egui::FontId::new(14.0, theme::icon_family()),
        tokens.text_muted,
    );
    let clear_width = if text.is_empty() { 0.0 } else { 26.0 };
    let edit_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 32.0, rect.top() + 1.0),
        egui::pos2(rect.right() - 6.0 - clear_width, rect.bottom() - 1.0),
    );
    let mut response = ui.put(
        edit_rect,
        egui::TextEdit::singleline(text)
            .hint_text(hint.to_owned())
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_CENTER)
            .desired_width(edit_rect.width()),
    );
    if !text.is_empty() {
        let clear_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 17.0, rect.center().y),
            egui::vec2(22.0, 22.0),
        );
        let clear = ui.put(
            clear_rect,
            egui::Button::new(
                RichText::new(icons::CLOSE)
                    .family(theme::icon_family())
                    .size(13.0),
            )
            .frame_when_inactive(false),
        );
        if clear.on_hover_text("Clear search").clicked() {
            text.clear();
            response.mark_changed();
            response.request_focus();
        }
    }
    let stroke = if response.has_focus() {
        tokens.accent
    } else {
        tokens.border
    };
    ui.painter().set(
        background,
        egui::Shape::rect_filled(rect, 6.0, tokens.surface),
    );
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    response
}

pub(crate) fn chip(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(11.5))
            .small()
            .corner_radius(6),
    )
}

pub(crate) fn kbd_hint(ui: &mut Ui, text: &str) -> Response {
    let tokens = Tokens::for_dark(theme::is_dark(ui.ctx()));
    egui::Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1.0, tokens.border_strong))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(11.0)
                    .color(tokens.text_secondary),
            );
        })
        .response
}

/// Use two cells followed by `ui.end_row()` for each property.
pub(crate) fn property_grid<R>(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(egui::vec2(20.0, 6.0))
        .show(ui, contents)
}
