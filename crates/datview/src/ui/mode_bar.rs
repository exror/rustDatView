use dv_plot::plotdata::{
    COMPARE_METHODS, COMPARE_MODES, FFT_AVERAGING, FFT_OUTPUTS, FFT_WINDOWS, FFT_X_TYPES,
    MINMAX_CENTERING, POLAR_ABOUT, POLAR_BINS,
};
use eframe::egui::{self, ComboBox, DragValue};

use crate::workspace::{Mode, Workspace};

/// Draw the options for the current mode. The caller rebuilds the series if this returns true.
pub(crate) fn show(ui: &mut egui::Ui, workspace: &mut Workspace) -> bool {
    if workspace.mode == Mode::Signals {
        return false;
    }

    let mut changed = false;
    ui.horizontal_wrapped(|ui| match workspace.mode {
        Mode::Signals => {}
        Mode::Distribution => {
            let options = &mut workspace.options.pdf;
            ui.label("Bins");
            changed |= ui
                .add(DragValue::new(&mut options.bins).range(1..=usize::MAX))
                .on_hover_text("Number of histogram bins, or points in the smoothed density")
                .changed();
            changed |= ui
                .checkbox(&mut options.smooth, "Smooth (KDE)")
                .on_hover_text("Estimate a smooth probability density")
                .changed();
        }
        Mode::Spectrum => {
            let options = &mut workspace.options.fft;
            changed |= string_choice(ui, "fft_output", "Output", &mut options.y_type, FFT_OUTPUTS);
            changed |= string_choice(ui, "fft_x", "X axis", &mut options.x_type, FFT_X_TYPES);
            changed |= string_choice(
                ui,
                "fft_averaging",
                "Averaging",
                &mut options.averaging,
                FFT_AVERAGING,
            );
            let welch = options.averaging == "Welch";
            ui.add_enabled_ui(welch, |ui| {
                changed |=
                    string_choice(ui, "fft_window", "Window", &mut options.window, FFT_WINDOWS);
                ui.label("Segment 2ⁿ");
                changed |= ui
                    .add(DragValue::new(&mut options.n_exp).range(1..=usize::BITS - 1))
                    .on_hover_text("Welch segment length as a power of two")
                    .changed();
            });
            if options.averaging == "Binning" {
                ui.label("Points per decade");
                changed |= ui
                    .add(DragValue::new(&mut options.points_per_decade).range(1..=usize::MAX))
                    .changed();
            }
            changed |= ui.checkbox(&mut options.detrend, "Detrend").changed();
        }
        Mode::Normalized => {
            let options = &mut workspace.options.normalized;
            changed |= ui.checkbox(&mut options.x_scale, "Scale X").changed();
            ui.add_enabled_ui(options.y_center == "None", |ui| {
                changed |= ui
                    .checkbox(&mut options.y_scale, "Scale Y")
                    .on_hover_text("Scale Y to 0–1 when centering is None")
                    .changed();
            });
            changed |= string_choice(
                ui,
                "normalize_centering",
                "Centering",
                &mut options.y_center,
                MINMAX_CENTERING,
            );
            ui.add_enabled_ui(options.y_center.ends_with("=ref"), |ui| {
                ui.label("Reference");
                changed |= ui
                    .add(DragValue::new(&mut options.y_ref).range(f64::MIN..=f64::MAX))
                    .changed();
            });
        }
        Mode::Compare => {
            let options = &mut workspace.options;
            ui.label("Method");
            let selected_method = COMPARE_METHODS
                .iter()
                .find(|(method, _)| *method == options.compare_method)
                .map(|(_, label)| *label)
                .unwrap_or("Unknown");
            ComboBox::from_id_salt("compare_method")
                .selected_text(selected_method)
                .show_ui(ui, |ui| {
                    for &(method, label) in COMPARE_METHODS {
                        changed |= ui
                            .selectable_value(&mut options.compare_method, method, label)
                            .changed();
                    }
                });
            ui.label("Pairing");
            let selected_mode = match options.compare_mode {
                None => "Auto",
                Some(mode) => COMPARE_MODES
                    .iter()
                    .find(|(candidate, _)| *candidate == mode)
                    .map(|(_, label)| *label)
                    .unwrap_or("Unknown"),
            };
            ComboBox::from_id_salt("compare_pairing")
                .selected_text(selected_mode)
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut options.compare_mode, None, "Auto")
                        .changed();
                    for &(mode, label) in COMPARE_MODES {
                        changed |= ui
                            .selectable_value(&mut options.compare_mode, Some(mode), label)
                            .changed();
                    }
                });
        }
        Mode::Polar => {
            let options = &mut workspace.options.polar;
            changed |= ui.checkbox(&mut options.degrees, "Degrees").changed();
            ui.label("Bins");
            let selected_bins = POLAR_BINS
                .iter()
                .find(|(bins, _)| *bins == options.bins)
                .map(|(_, label)| *label)
                .unwrap_or("Custom");
            ComboBox::from_id_salt("polar_bins")
                .selected_text(selected_bins)
                .show_ui(ui, |ui| {
                    for &(bins, label) in POLAR_BINS {
                        changed |= ui
                            .selectable_value(&mut options.bins, bins, label)
                            .changed();
                    }
                });
            changed |= string_choice(ui, "polar_about", "About", &mut options.about, POLAR_ABOUT);
            let mut use_reference = options.radius_ref.is_some();
            if ui
                .checkbox(&mut use_reference, "Radius reference")
                .changed()
            {
                options.radius_ref = use_reference.then_some(0.0);
                changed = true;
            }
            if let Some(reference) = &mut options.radius_ref {
                changed |= ui
                    .add(DragValue::new(reference).range(f64::MIN..=f64::MAX))
                    .on_hover_text("Shift the mean radius to this value")
                    .changed();
            }
        }
    });
    changed
}

/// Choice tables store the backend value first and the user-facing label second.
fn string_choice(
    ui: &mut egui::Ui,
    id: &'static str,
    label: &'static str,
    current: &mut String,
    choices: &'static [(&'static str, &'static str)],
) -> bool {
    ui.label(label);
    let selected = choices
        .iter()
        .find(|(value, _)| *value == current.as_str())
        .map(|(_, display)| *display)
        .unwrap_or("Unknown");
    let mut changed = false;
    ComboBox::from_id_salt(id)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for &(value, display) in choices {
                if ui
                    .selectable_label(current.as_str() == value, display)
                    .clicked()
                    && current.as_str() != value
                {
                    *current = value.to_owned();
                    changed = true;
                }
            }
        });
    changed
}
