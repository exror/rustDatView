//! The application-to-chart adapter. Screen and figure export both use this data.
use std::{borrow::Cow, collections::BTreeMap, sync::{Arc, LazyLock}};

use dv_chart::{
    AxisFormat, ChartConfig, ChartResponse, ChartState, ChartTheme, Dash, Draw, Mark, Pane, Scale,
    Series, Tool,
};
use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

use super::theme;
use crate::{
    panes::{Annotation, ColorBy, LineDraw, PaneGrouping},
    settings::Settings,
    workspace::{Mode, Workspace},
};

/// Chart presentation, separate from transient interactions in `ChartState`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ChartOptions {
    pub log_x: bool,
    pub log_y: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub link_x: bool,
    pub include_zero: bool,
    pub pane_log_y: BTreeMap<u64, bool>,
    pub pane_include_zero: BTreeMap<u64, bool>,
    pub grid: bool,
    pub legend: bool,
    pub legend_values: bool,
    pub crosshair: bool,
    pub tooltip: bool,
    pub equal_aspect: bool,
    pub title: String,
    pub subtitle: String,
    pub x_label: String,
    pub y_label: String,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            log_x: false,
            log_y: false,
            flip_x: false,
            flip_y: false,
            link_x: true,
            include_zero: false,
            pane_log_y: BTreeMap::new(),
            pane_include_zero: BTreeMap::new(),
            grid: true,
            legend: true,
            legend_values: true,
            crosshair: true,
            tooltip: false,
            equal_aspect: false,
            title: String::new(),
            subtitle: String::new(),
            x_label: String::new(),
            y_label: String::new(),
        }
    }
}

fn grouping_label(grouping: PaneGrouping) -> &'static str {
    match grouping {
        PaneGrouping::Unit => "By unit",
        PaneGrouping::Single => "Single pane",
        PaneGrouping::Channel => "One per channel",
        PaneGrouping::Custom => "Custom",
    }
}

fn color_label(color: ColorBy) -> &'static str {
    match color {
        ColorBy::Auto => "Auto",
        ColorBy::Channel => "Channel",
        ColorBy::Source => "Source",
    }
}

/// Draw the chart toolbar and interactive surface. The returned response carries
/// hover, selection, view and context-menu information for the rest of the app.
pub(crate) fn show(
    ui: &mut egui::Ui,
    workspace: &mut Workspace,
    state: &mut ChartState,
    settings: &Settings,
    options: &mut ChartOptions,
    inspector_visible: &mut bool,
) -> ChartResponse {
    let mut regroup = false;
    let mut toolbar_view_changed = false;
    let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
    ui.horizontal(|ui| {
        const TOOLS: [(Tool, &str, &str); 3] = [
            (
                Tool::Select,
                "Select",
                "Drag across the chart to select a range and read its statistics",
            ),
            (
                Tool::Pan,
                "Pan",
                "Drag to pan. Right or middle drag pans with any tool",
            ),
            (
                Tool::Zoom,
                "Zoom",
                "Drag a box to zoom. Ctrl+drag zooms with any tool",
            ),
        ];
        if let Some(tool) = super::widgets::segmented(ui, state.tool, &TOOLS) {
            state.tool = tool;
        }
        ui.add_space(6.0);
        if super::widgets::icon_button(ui, super::icons::FIT, "Fit all data · Home or double-click")
            .clicked()
        {
            state.fit_all();
            toolbar_view_changed = true;
        }
        if ui
            .add(egui::Button::new("Log Y").selected(options.log_y))
            .on_hover_text("Logarithmic Y axis on every pane")
            .clicked()
        {
            options.log_y = !options.log_y;
            options.pane_log_y.clear();
            state.y_views.clear();
            toolbar_view_changed = true;
        }
        ui.menu_button("Panes", |ui| {
            ui.set_min_width(230.0);
            ui.label(egui::RichText::new("Group channels into panes").color(tokens.text_secondary));
            for grouping in [
                PaneGrouping::Unit,
                PaneGrouping::Single,
                PaneGrouping::Channel,
                PaneGrouping::Custom,
            ] {
                regroup |= ui
                    .radio_value(
                        &mut workspace.panes.grouping,
                        grouping,
                        grouping_label(grouping),
                    )
                    .changed();
            }
            if workspace.panes.grouping == PaneGrouping::Custom {
                ui.separator();
                ui.label(
                    egui::RichText::new("Channels with the same pane name share an axis.")
                        .color(tokens.text_secondary),
                );
                for key in &workspace.selected {
                    if let Some(channel) = workspace
                        .channels
                        .iter()
                        .find(|channel| &channel.key == key)
                    {
                        let mut name = workspace
                            .panes
                            .custom
                            .get(key)
                            .cloned()
                            .or_else(|| {
                                workspace
                                    .series
                                    .iter()
                                    .find(|series| &series.key == key)
                                    .map(|series| series.pane.clone())
                            })
                            .unwrap_or_else(|| {
                                if channel.unit.is_empty() {
                                    channel.family.clone()
                                } else {
                                    format!("{} · {}", channel.family, channel.unit)
                                }
                            });
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [130.0, 20.0],
                                egui::Label::new(&channel.label).truncate(),
                            )
                            .on_hover_text(&channel.label);
                            if ui
                                .add(egui::TextEdit::singleline(&mut name).desired_width(120.0))
                                .changed()
                            {
                                workspace.panes.custom.insert(key.clone(), name);
                                for series in workspace.series.iter().filter(|series| &series.key == key) {
                                    workspace.panes.custom_series.remove(&series.id);
                                }
                                regroup = true;
                            }
                        });
                    }
                }
            }
            ui.separator();
            ui.label(egui::RichText::new("Color series by").color(tokens.text_secondary));
            for color in [ColorBy::Auto, ColorBy::Channel, ColorBy::Source] {
                regroup |= ui
                    .radio_value(&mut workspace.panes.color_by, color, color_label(color))
                    .changed();
            }
        })
        .response
        .on_hover_text("Pane grouping and series coloring");
        ui.menu_button("Display", |ui| {
            ui.set_min_width(220.0);
            let mut changed = false;
            changed |= ui.checkbox(&mut options.grid, "Grid").changed();
            changed |= ui.checkbox(&mut options.legend, "Legend").changed();
            changed |= ui
                .checkbox(
                    &mut options.legend_values,
                    "Values in legend while hovering",
                )
                .changed();
            changed |= ui.checkbox(&mut options.crosshair, "Crosshair").changed();
            changed |= ui
                .checkbox(&mut options.tooltip, "Tooltip with every value")
                .changed();
            ui.separator();
            changed |= ui
                .checkbox(&mut options.link_x, "Link X across panes")
                .changed();
            changed |= ui
                .checkbox(&mut options.include_zero, "Include zero on Y")
                .changed();
            changed |= ui.checkbox(&mut options.log_x, "Logarithmic X").changed();
            changed |= ui.checkbox(&mut options.flip_x, "Reverse X").changed();
            changed |= ui.checkbox(&mut options.flip_y, "Reverse Y").changed();
            changed |= ui
                .checkbox(&mut options.equal_aspect, "Equal X/Y scale")
                .changed();
            if changed {
                state.y_views.clear();
                toolbar_view_changed = true;
            }
        })
        .response
        .on_hover_text("Grid, legend, crosshair and axis options");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::widgets::icon_button(ui, super::icons::SLIDERS, "Show or hide inspector · Ctrl+I").clicked() {
                *inspector_visible = !*inspector_visible;
            }
            if let Some(range) = state.selection {
                if super::widgets::icon_button(ui, super::icons::CLOSE, "Clear selection · Esc")
                    .clicked()
                {
                    state.selection = None;
                    toolbar_view_changed = true;
                }
                if ui
                    .button("Zoom to selection")
                    .on_hover_text("Or double-click inside the selection")
                    .clicked()
                {
                    if options.link_x {
                        state.x_view = Some([range[0].min(range[1]), range[0].max(range[1])]);
                    } else {
                        for series in &workspace.series {
                            state.pane_x_views.insert(pane_id(&series.pane), range);
                        }
                    }
                    toolbar_view_changed = true;
                }
            } else if !workspace.series.is_empty() && ui.available_width() > 330.0 {
                ui.label(
                    egui::RichText::new("Drag to select  ·  Scroll to zoom  ·  Right-drag to pan")
                        .size(11.5)
                        .color(tokens.text_muted),
                );
            }
        });
    });
    ui.add_space(2.0);
    if regroup {
        workspace.rebuild();
    }
    let dark = theme::is_dark(ui.ctx());
    let mut response = with_chart(
        workspace,
        settings,
        options,
        dark,
        true,
        |panes, config, marks, chart_theme| {
            dv_chart::show(ui, state, panes, config, marks, chart_theme)
        },
    );
    if workspace.series.is_empty() {
        let tokens = theme::Tokens::for_dark(dark);
        let text = workspace.chart_notice.as_deref().unwrap_or(if workspace.selected.is_empty() {
            "Choose channels in the sidebar to plot them."
        } else {
            "The selected channels have no values to plot here."
        });
        ui.painter().text(
            response.response.rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(14.0),
            tokens.text_secondary,
        );
    }
    let mut pane_changed = false;
    match response.pane_action.as_ref() {
        Some(dv_chart::PaneAction::ToggleLog(id)) => {
            let id = *id;
            let value = !options
                .pane_log_y
                .get(&id)
                .copied()
                .unwrap_or(options.log_y);
            if value == options.log_y {
                options.pane_log_y.remove(&id);
            } else {
                options.pane_log_y.insert(id, value);
            }
            state.y_views.remove(&id);
            pane_changed = true;
        }
        Some(dv_chart::PaneAction::ToggleIncludeZero(id)) => {
            let id = *id;
            let value = !options
                .pane_include_zero
                .get(&id)
                .copied()
                .unwrap_or(options.include_zero);
            if value == options.include_zero {
                options.pane_include_zero.remove(&id);
            } else {
                options.pane_include_zero.insert(id, value);
            }
            pane_changed = true;
        }
        Some(dv_chart::PaneAction::Remove(id)) => {
            let keys: std::collections::BTreeSet<_> = workspace
                .series
                .iter()
                .filter(|series| pane_id(&series.pane) == *id)
                .map(|series| series.key.clone())
                .collect();
            workspace.selected.retain(|key| !keys.contains(key));
            options.pane_log_y.remove(id);
            options.pane_include_zero.remove(id);
            state.y_views.remove(id);
            state.pane_x_views.remove(id);
            state.pane_weights.remove(id);
            if state.maximized == Some(*id) {
                state.maximized = None;
            }
            workspace.rebuild();
            pane_changed = true;
        }
        _ => {}
    }
    if pane_changed {
        response.view_changed = true;
        ui.ctx().request_repaint();
    }
    response.view_changed |= toolbar_view_changed || regroup;
    response
}

/// Stable across processes and platforms, so saved manual Y ranges and marks
/// continue to refer to the same logical pane after a session is reopened.
pub(crate) fn pane_id(name: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// The color a series is drawn with: explicit style, stored channel color, or palette slot.
pub(crate) fn series_color(
    workspace: &Workspace,
    palette: &[Color32],
    series: &crate::series::SeriesData,
) -> Color32 {
    let specific = workspace.panes.specific_style(series.id);
    specific
        .or_else(|| workspace.panes.styles.get(&series.key))
        .and_then(|style| style.color)
        .or_else(|| {
            specific.is_none().then(|| workspace.panes.colors.get(&series.key).copied()).flatten()
        })
        .map(color)
        .unwrap_or(palette[series.color_slot % palette.len()])
}

fn dash(index: usize) -> Dash {
    match index % 4 {
        0 => Dash::Solid,
        1 => Dash::Dashed,
        2 => Dash::Dotted,
        _ => Dash::DashDot,
    }
}

/// Borrow the cached sample arrays; only small per-series descriptors are
/// constructed here, never copies of X/Y data.
pub(crate) fn with_chart<R>(
    workspace: &Workspace,
    settings: &Settings,
    options: &ChartOptions,
    dark: bool,
    include_marks: bool,
    render: impl FnOnce(&[Pane<'_>], &ChartConfig<'_>, &[Mark], &ChartTheme) -> R,
) -> R {
    let mut chart_theme = if dark {
        ChartTheme::dark()
    } else {
        ChartTheme::light()
    };
    chart_theme.series_palette = theme::palette_for(dark, settings.palette);

    // Retain selection order instead of sorting panes alphabetically.
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    let mut by_name = BTreeMap::new();
    for (index, series) in workspace.series.iter().enumerate() {
        let group = *by_name.entry(series.pane.as_str()).or_insert_with(|| {
            groups.push((series.pane.as_str(), Vec::new()));
            groups.len() - 1
        });
        groups[group].1.push(index);
    }
    let polar_y_labels: Vec<String> = groups
        .iter()
        .map(|(_, indices)| {
            if workspace.mode != Mode::Polar {
                return String::new();
            }
            let axis = if workspace.options.polar.about.starts_with('x') {
                "Z"
            } else {
                "Y"
            };
            let unit = &workspace.series[indices[0]].unit;
            if !unit.is_empty()
                && indices
                    .iter()
                    .all(|&index| workspace.series[index].unit == *unit)
            {
                format!("{axis} [{unit}]")
            } else {
                axis.to_owned()
            }
        })
        .collect();
    let mut panes = Vec::with_capacity(groups.len());
    for (group_index, (name, indices)) in groups.iter().enumerate() {
        let first = &workspace.series[indices[0]];
        let flags = workspace.mode == Mode::Signals
            && indices.iter().all(|&index| workspace.series[index].family == "Flags");
        let unit = if indices
            .iter()
            .all(|&index| workspace.series[index].unit == first.unit)
        {
            first.unit.as_str()
        } else {
            ""
        };
        let title_has_unit = |symbol: &str| {
            !symbol.is_empty()
                && (*name == symbol
                    || name
                        .strip_suffix(symbol)
                        .is_some_and(|prefix| prefix.ends_with(" · ")))
        };
        let unit_in_title = title_has_unit(unit)
            || (!unit.is_empty() && title_has_unit(&dv_core::units::parse_unit(unit).symbol));
        let y_label = if !options.y_label.is_empty() {
            options.y_label.as_str()
        } else if workspace.mode == Mode::Polar {
            polar_y_labels[group_index].as_str()
        } else if unit_in_title || flags {
            ""
        } else {
            unit
        };
        let mut lines = Vec::with_capacity(indices.len());
        for &index in indices {
            let series = &workspace.series[index];
            let style = workspace.panes.style_for(series.id, &series.key);
            let series_color = series_color(workspace, &chart_theme.series_palette, series);
            let draw = match style.and_then(|style| style.draw) {
                Some(LineDraw::Points) => Draw::Points,
                Some(LineDraw::LinesPoints) => Draw::LinesPoints,
                Some(LineDraw::Steps) => Draw::Steps,
                Some(LineDraw::Lines) => Draw::Lines,
                None if workspace.mode == Mode::Signals && series.family == "Flags" => Draw::Steps,
                None => Draw::Lines,
            };
            let width = style
                .and_then(|style| style.width)
                .unwrap_or(settings.line_width);
            lines.push(Series {
                id: series.id,
                label: &series.label,
                unit: &series.unit,
                x: &series.x,
                y: &series.y,
                generation: series.generation,
                color: series_color,
                width,
                dash: dash(series.dash),
                draw,
                marker_radius: 3.0,
                visible: true,
                overlay: false,
            });
        }
        for overlay in &workspace.overlays {
            let Some(target) = indices
                .iter()
                .map(|&index| &workspace.series[index])
                .find(|series| series.id == overlay.target)
            else {
                continue;
            };
            let mut id = overlay.target ^ 0x9e37_79b9_7f4a_7c15;
            for byte in overlay.label.bytes() {
                id = (id ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
            lines.push(Series {
                id,
                label: &overlay.label,
                unit: &target.unit,
                x: &overlay.x,
                y: &overlay.y,
                generation: overlay.generation,
                color: chart_theme.text,
                width: settings.line_width + 0.4,
                dash: Dash::Dashed,
                draw: Draw::Lines,
                marker_radius: 3.0,
                visible: true,
                overlay: true,
            });
        }
        let id = pane_id(name);
        panes.push(Pane {
            id,
            title: name,
            y_label,
            y_scale: if options
                .pane_log_y
                .get(&id)
                .copied()
                .unwrap_or(options.log_y && !flags)
            {
                Scale::Log10
            } else {
                Scale::Linear
            },
            y_format: if flags {
                static LABELS: LazyLock<Arc<[String]>> =
                    LazyLock::new(|| Arc::from(["No".into(), "Yes".into()]));
                AxisFormat::Category(Arc::clone(&LABELS))
            } else {
                AxisFormat::Number
            },
            y_flip: options.flip_y,
            include_zero: options
                .pane_include_zero
                .get(&id)
                .copied()
                .unwrap_or(options.include_zero),
            series: lines,
            weight: if flags { 0.45 } else { 1.0 },
        });
    }

    let x_channel = workspace.x_key.as_ref().and_then(|key| {
        workspace
            .channels
            .iter()
            .find(|channel| &channel.key == key)
    });
    let x_name = x_channel.map(|channel| {
        if channel.unit.is_empty() {
            Cow::Borrowed(channel.label.as_str())
        } else {
            Cow::Owned(format!("{} [{}]", channel.label, channel.unit))
        }
    });
    let shared_x_label = workspace.series.first().and_then(|first| {
        (!first.x_label.is_empty()
            && workspace
                .series
                .iter()
                .all(|series| series.x_label == first.x_label))
        .then_some(first.x_label.as_str())
    });
    let default_x_label = match workspace.mode {
        Mode::Signals => x_name.as_deref().unwrap_or(""),
        Mode::Distribution => shared_x_label.unwrap_or("Value"),
        Mode::Spectrum => shared_x_label.unwrap_or(match workspace.options.fft.x_type.as_str() {
            "x" => "X",
            "2pi/x" => "Angular inverse X",
            _ => "Inverse X",
        }),
        Mode::Normalized if workspace.options.normalized.x_scale => "Normalized X",
        Mode::Polar => {
            shared_x_label.unwrap_or(if workspace.options.polar.about.starts_with('x') {
                "Y"
            } else {
                "X"
            })
        }
        _ => shared_x_label.or(x_name.as_deref()).unwrap_or(""),
    };
    let (x_format, aligned_datetime) = x_axis_format(workspace);
    let datetime = x_format == AxisFormat::DateTime;
    let duration = x_format == AxisFormat::Duration;
    let x_label = if !options.x_label.is_empty() {
        options.x_label.as_str()
    } else if aligned_datetime {
        "Elapsed time"
    } else {
        default_x_label
    };
    let overview = settings.overview
        && (datetime || duration)
        && workspace
            .series
            .first()
            .is_some_and(|series| series.x.len() >= 2_000);
    let config = ChartConfig {
        x_label,
        x_scale: if options.log_x {
            Scale::Log10
        } else {
            Scale::Linear
        },
        x_format,
        x_flip: options.flip_x,
        // Distributions of different quantities (W, MHz, °C) have unrelated
        // X axes; sharing one would squeeze all but one into a sliver.
        link_x: options.link_x
            && !(workspace.mode == Mode::Distribution
                && workspace.series.iter().any(|s| s.family != workspace.series[0].family)),
        grid: options.grid,
        legend: options.legend,
        legend_values: options.legend_values,
        crosshair: options.crosshair,
        overview,
        tooltip: options.tooltip,
        precision: settings.precision,
        equal_aspect: options.equal_aspect || matches!(workspace.mode, Mode::Polar),
        gap_factor: (workspace.mode != Mode::Polar
            && settings.break_gaps
            && settings.gap_factor.is_finite()
            && settings.gap_factor > 0.0)
            .then_some(settings.gap_factor),
        decimate: true,
        title: &options.title,
        subtitle: &options.subtitle,
    };
    let mut marks = Vec::new();
    if include_marks {
        for annotation in &workspace.annotations {
            match annotation {
                Annotation::Point {
                    mode,
                    curve,
                    x,
                    y,
                    label,
                    color: mark_color,
                } if *mode == workspace.mode => {
                    let curve_id = crate::panes::RangeRule::target_id(curve);
                    for (name, indices) in &groups {
                        if indices.iter().any(|&index| {
                            let series = &workspace.series[index];
                            curve_id.map_or_else(
                                || series.key == *curve || series.label == *curve || series.channel_label == *curve,
                                |id| series.id == id,
                            )
                        }) {
                            marks.push(Mark::Point {
                                pane: Some(pane_id(name)),
                                x: *x,
                                y: *y,
                                label: label.clone(),
                                color: color(*mark_color),
                            });
                        }
                    }
                }
                Annotation::Region {
                    mode,
                    x_axis,
                    min,
                    max,
                    label,
                    color: mark_color,
                } if *mode == workspace.mode
                    && (x_axis.is_empty()
                        || workspace.x_key.as_deref() == Some(x_axis.as_str())) =>
                {
                    marks.push(Mark::Region {
                        x0: *min,
                        x1: *max,
                        label: label.clone(),
                        color: color(*mark_color),
                    });
                }
                Annotation::VLine {
                    mode,
                    x_axis,
                    x,
                    label,
                    color: mark_color,
                } if *mode == workspace.mode
                    && (x_axis.is_empty()
                        || workspace.x_key.as_deref() == Some(x_axis.as_str())) =>
                {
                    marks.push(Mark::VLine {
                        x: *x,
                        label: label.clone(),
                        color: color(*mark_color),
                    });
                }
                Annotation::HLine {
                    mode,
                    pane,
                    y,
                    label,
                    color: mark_color,
                } if *mode == workspace.mode => {
                    marks.push(Mark::HLine {
                        pane: pane_id(pane),
                        y: *y,
                        label: label.clone(),
                        color: color(*mark_color),
                    });
                }
                _ => {}
            }
        }
    }
    render(&panes, &config, &marks, &chart_theme)
}

/// How the X axis is labelled: clock time, elapsed duration or plain numbers.
/// The second value is true when absolute timestamps were aligned to elapsed time.
pub(crate) fn x_axis_format(workspace: &Workspace) -> (AxisFormat, bool) {
    let first = workspace.series.first();
    if first.is_some_and(|series| series.datetime) {
        return (AxisFormat::DateTime, false);
    }
    if workspace.mode != Mode::Signals {
        return (AxisFormat::Number, false);
    }
    let aligned_datetime = first.is_some_and(|series| {
        workspace.sources.get(series.source).is_some_and(|source| source.align_start)
            && workspace.tables.tabs.get(series.source).is_some_and(|table| {
                workspace.x_key.as_ref().is_some_and(|key| {
                    table.data.columns().iter().any(|column| {
                        column.is_datetime() && dv_core::common::clean_col(&column.name) == *key
                    })
                })
            })
    });
    // The plotted source's own X column decides: the channel catalog merges
    // every loaded source, including hidden ones with other units.
    let seconds = workspace.x_key.as_ref().is_some_and(|key| {
        let own = first.and_then(|series| workspace.tables.tabs.get(series.source)).and_then(|table| {
            table.data.columns().iter().find(|column| dv_core::common::clean_col(&column.name) == *key)
        });
        match own {
            Some(column) => dv_core::units::split_name_unit(&column.name).1 == "s",
            None => workspace.channels.iter().any(|channel| &channel.key == key && channel.unit == "s"),
        }
    });
    if aligned_datetime || seconds {
        (AxisFormat::Duration, aligned_datetime)
    } else {
        (AxisFormat::Number, false)
    }
}

/// The X position under the cursor, written the way the X axis writes it.
pub(crate) fn x_readout(workspace: &Workspace, x: f64, precision: usize) -> String {
    match x_axis_format(workspace).0 {
        AxisFormat::DateTime => dv_chart::format::format_time(x, 0.001),
        AxisFormat::Duration => dv_chart::format::format_elapsed(x, precision.max(5)),
        _ => {
            let unit = workspace.x_key.as_ref()
                .and_then(|key| workspace.channels.iter().find(|channel| &channel.key == key))
                .map_or("", |channel| channel.unit.as_str());
            dv_chart::format::format_value(x, unit, precision)
        }
    }
}
