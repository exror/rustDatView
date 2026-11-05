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
            ui.label("Bins")
                .on_hover_text("Number of intervals used to estimate the distribution.");
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
            changed |= string_choice(
                ui,
                "fft_output",
                "Output",
                &mut options.y_type,
                FFT_OUTPUTS,
                "Choose amplitude, power spectral density, or frequency-weighted power.",
            );
            changed |= string_choice(
                ui,
                "fft_x",
                "X axis",
                &mut options.x_type,
                FFT_X_TYPES,
                "Show frequency, period, or angular frequency on the horizontal axis.",
            );
            changed |= string_choice(
                ui,
                "fft_averaging",
                "Averaging",
                &mut options.averaging,
                FFT_AVERAGING,
                "Welch averages overlapping segments; binning averages nearby frequency values.",
            );
            let welch = options.averaging == "Welch";
            ui.add_enabled_ui(welch, |ui| {
                changed |= string_choice(
                    ui,
                    "fft_window",
                    "Window",
                    &mut options.window,
                    FFT_WINDOWS,
                    "Shape each segment before averaging to reduce edge effects.",
                );
                ui.label("Segment power").on_hover_text(
                    "The number of samples in each segment is 2 raised to this power.",
                );
                changed |= ui
                    .add(DragValue::new(&mut options.n_exp).range(1..=usize::BITS - 1))
                    .on_hover_text("Welch segment length as a power of two")
                    .changed();
            });
            if options.averaging == "Binning" {
                ui.label("Points per decade")
                    .on_hover_text("Number of logarithmic frequency bins per factor of ten.");
                changed |= ui
                    .add(DragValue::new(&mut options.points_per_decade).range(1..=usize::MAX))
                    .on_hover_text("Number of logarithmic frequency bins per factor of ten.")
                    .changed();
            }
            changed |= ui
                .checkbox(&mut options.detrend, "Detrend")
                .on_hover_text("Subtract the mean before calculating the spectrum (for Welch, from each segment).")
                .changed();
        }
        Mode::Normalized => {
            let options = &mut workspace.options.normalized;
            changed |= ui
                .checkbox(&mut options.x_scale, "Scale X")
                .on_hover_text("Scale the horizontal values to range from 0 to 1.")
                .changed();
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
                "Shift each signal so its midpoint or mean is zero or the chosen reference.",
            );
            ui.add_enabled_ui(options.y_center.ends_with("=ref"), |ui| {
                ui.label("Reference");
                changed |= ui
                    .add(DragValue::new(&mut options.y_ref).range(f64::MIN..=f64::MAX))
                    .on_hover_text("The midpoint or mean is shifted to this value.")
                    .changed();
            });
        }
        Mode::Compare => {
            let options = &mut workspace.options;
            ui.label("Method")
                .on_hover_text("Compare each signal to the first signal in its group.");
            let selected_method = COMPARE_METHODS
                .iter()
                .find(|(method, _)| *method == options.compare_method)
                .map(|(method, label)| compare_label(*method, label))
                .unwrap_or("Unknown");
            ComboBox::from_id_salt("compare_method")
                .selected_text(selected_method)
                .show_ui(ui, |ui| {
                    for &(method, label) in COMPARE_METHODS {
                        changed |= ui
                            .selectable_value(
                                &mut options.compare_method,
                                method,
                                compare_label(method, label),
                            )
                            .changed();
                    }
                })
                .response
                .on_hover_text("Compare each signal to the first signal in its group.");
            ui.label("Pairing")
                .on_hover_text("Choose how channels from one or more sources are grouped.");
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
                })
                .response
                .on_hover_text("Choose how channels from one or more sources are grouped.");
        }
        Mode::Polar => {
            let options = &mut workspace.options.polar;
            changed |= ui
                .checkbox(&mut options.degrees, "Degrees")
                .on_hover_text("Interpret angles in degrees instead of radians.")
                .changed();
            ui.label("Bins")
                .on_hover_text("Average values within angular bins, or show all samples.");
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
                })
                .response
                .on_hover_text("Average values within angular bins, or show all samples.");
            changed |= string_choice(
                ui,
                "polar_about",
                "Axis",
                &mut options.about,
                POLAR_ABOUT,
                "Choose the axis around which the polar angle is measured.",
            );
            let mut use_reference = options.radius_ref.is_some();
            if ui
                .checkbox(&mut use_reference, "Radius reference")
                .on_hover_text("Shift the mean radius to a chosen reference value.")
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
    help: &'static str,
) -> bool {
    ui.label(label).on_hover_text(help);
    let selected = choices
        .iter()
        .find(|(value, _)| *value == current.as_str())
        .map(|(value, display)| choice_label(id, value, display))
        .unwrap_or("Unknown");
    let mut changed = false;
    ComboBox::from_id_salt(id)
        .width(if id == "normalize_centering" { 180.0 } else { 125.0 })
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for &(value, display) in choices {
                if ui
                    .selectable_label(current.as_str() == value, choice_label(id, value, display))
                    .clicked()
                    && current.as_str() != value
                {
                    *current = value.to_owned();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(help);
    changed
}

fn choice_label<'a>(id: &str, value: &str, fallback: &'a str) -> &'a str {
    match (id, value) {
        ("normalize_centering", "Mid=0") => "Midpoint at zero",
        ("normalize_centering", "Mid=ref") => "Midpoint at reference",
        ("normalize_centering", "Mean=0") => "Mean at zero",
        ("normalize_centering", "Mean=ref") => "Mean at reference",
        ("polar_about", "x") => "X axis",
        ("polar_about", "z") => "Z axis",
        _ => fallback,
    }
}

fn compare_label<'a>(method: dv_plot::plotdata::CompareMethod, fallback: &'a str) -> &'a str {
    match method {
        dv_plot::plotdata::CompareMethod::AbsRelative => "Absolute relative %",
        dv_plot::plotdata::CompareMethod::Absolute => "Difference",
        dv_plot::plotdata::CompareMethod::YY => "Signal vs reference",
        _ => fallback,
    }
}
