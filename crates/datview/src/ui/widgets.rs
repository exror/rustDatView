//! Small, reusable shell controls sharing the app's typography and colors.
use std::time::{Duration, Instant};

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
            .vertical_align(egui::Align::Center)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

fn toast_color(tokens: Tokens, kind: ToastKind) -> Color32 {
    match kind {
        ToastKind::Info => tokens.accent,
        ToastKind::Success => tokens.success,
        ToastKind::Warning => tokens.warning,
        ToastKind::Error => tokens.error,
    }
}

/// Draw one notification and return true when the dismiss button is pressed.
pub(crate) fn toast(ui: &mut Ui, message: &str, kind: ToastKind) -> bool {
    toast_card(ui, message, kind, None).0
}

fn toast_card(ui: &mut Ui, message: &str, kind: ToastKind, action: Option<&str>) -> (bool, bool) {
    let tokens = Tokens::for_dark(theme::is_dark(ui.ctx()));
    let icon = match kind {
        ToastKind::Info => icons::INFO,
        ToastKind::Success => icons::CHECK_CIRCLE,
        ToastKind::Warning => icons::WARNING,
        ToastKind::Error => icons::WARNING_CIRCLE,
    };
    egui::Frame::new()
        .fill(tokens.raised)
        .stroke(Stroke::new(1.0, tokens.border_strong))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon)
                        .family(theme::icon_family())
                        .size(16.0)
                        .color(toast_color(tokens, kind)),
                );
                ui.label(message);
                let action_clicked = action.is_some_and(|label| ui.button(label).clicked());
                let dismissed = icon_button(ui, icons::CLOSE, "Dismiss notification").clicked();
                (dismissed, action_clicked)
            })
            .inner
        })
        .inner
}

struct ToastEntry {
    message: String,
    kind: ToastKind,
    created: Instant,
    action: Option<(String, u64)>,
}

/// Owns notifications across frames; normal messages expire after four seconds,
/// whereas errors remain until dismissed or acted on.
#[derive(Default)]
pub(crate) struct Toasts {
    entries: Vec<ToastEntry>,
}

impl Toasts {
    pub(crate) fn push(&mut self, message: impl Into<String>, kind: ToastKind) {
        self.entries.push(ToastEntry {
            message: message.into(),
            kind,
            created: Instant::now(),
            action: None,
        });
    }

    /// The supplied action id is returned by `show` when the button is clicked.
    pub(crate) fn push_with_action(
        &mut self,
        message: impl Into<String>,
        kind: ToastKind,
        label: impl Into<String>,
        action_id: u64,
    ) {
        self.entries.push(ToastEntry {
            message: message.into(),
            kind,
            created: Instant::now(),
            action: Some((label.into(), action_id)),
        });
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context) -> Vec<u64> {
        const LIFETIME: Duration = Duration::from_secs(4);
        let now = Instant::now();
        self.entries.retain(|entry| {
            entry.kind == ToastKind::Error || now.duration_since(entry.created) < LIFETIME
        });
        if self.entries.is_empty() {
            return Vec::new();
        }
        if let Some(next) = self
            .entries
            .iter()
            .filter(|entry| entry.kind != ToastKind::Error)
            .map(|entry| LIFETIME.saturating_sub(now.duration_since(entry.created)))
            .min()
        {
            ctx.request_repaint_after(next);
        }
        let mut dismiss = Vec::new();
        let mut actions = Vec::new();
        egui::Area::new(egui::Id::unique("rustdatview-toasts"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -38.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    for (index, entry) in self.entries.iter().enumerate() {
                        let (closed, acted) = toast_card(
                            ui,
                            &entry.message,
                            entry.kind,
                            entry.action.as_ref().map(|(label, _)| label.as_str()),
                        );
                        if acted {
                            if let Some((_, id)) = entry.action.as_ref() {
                                actions.push(*id);
                            }
                        }
                        if closed || acted {
                            dismiss.push(index);
                        }
                    }
                });
            });
        for index in dismiss.into_iter().rev() {
            self.entries.remove(index);
        }
        actions
    }
}
