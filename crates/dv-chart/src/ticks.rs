//! Bounded, scale-aware tick generation for chart axes.

use chrono::{DateTime, Datelike, NaiveDate};

use crate::format::{
    date_at, duration_style, format_duration_with_style, format_time, format_time_with_date,
    format_value, DurationStyle,
};
use crate::{AxisFormat, Scale};

const MAX_TICKS: usize = 512;
const DAY: f64 = 86_400.0;
const MONTH: f64 = 2_629_746.0; // Mean Gregorian month; actual boundaries are calendar-aligned.
const YEAR: f64 = MONTH * 12.0;

/// One tick in data coordinates. Minor ticks may have an empty label.
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label: String,
    pub major: bool,
}

/// Generate an approximately `target`-sized set of evenly spaced, nice ticks.
///
/// Bounds may be reversed. Non-finite bounds produce no ticks, and a finite
/// singleton range produces one tick. Work and output are bounded independently
/// of the requested target and of floating-point precision.
pub fn nice_ticks(min: f64, max: f64, target: usize) -> Vec<f64> {
    let Some((min, max)) = ordered_bounds(min, max) else {
        return Vec::new();
    };
    if target == 0 {
        return Vec::new();
    }
    if min == max {
        return vec![if min == 0.0 { 0.0 } else { min }];
    }
    let step = nice_step(desired_step(min, max, target));
    fixed_ticks(min, max, step)
}

/// Generate ticks and labels for a linear or base-10 logarithmic axis.
///
/// Logarithmic axes require strictly positive bounds. Powers of ten are major;
/// 2 through 9 are minor. A few minor ticks receive labels when zoomed into a
/// region with fewer than two visible major ticks.
pub fn axis_ticks(
    min: f64,
    max: f64,
    target: usize,
    format: &AxisFormat,
    scale: Scale,
) -> Vec<Tick> {
    let Some((min, max)) = ordered_bounds(min, max) else {
        return Vec::new();
    };
    if target == 0 || (scale == Scale::Log10 && min <= 0.0) {
        return Vec::new();
    }
    let target = target.clamp(1, MAX_TICKS);

    if let AxisFormat::Category(categories) = format {
        return category_ticks(min, max, target, categories);
    }
    if min == max {
        return label_ticks(
            vec![(min, true)],
            format,
            1.0,
            scale == Scale::Log10,
            target,
        );
    }
    if scale == Scale::Log10 {
        return label_ticks(log_ticks(min, max, target), format, 1.0, true, target);
    }

    let desired = desired_step(min, max, target);
    match format {
        AxisFormat::Number => {
            let step = nice_step(desired);
            let values = fixed_ticks(min, max, step)
                .into_iter()
                .map(|value| (value, true))
                .collect();
            label_ticks(values, format, step, false, target)
        }
        AxisFormat::Duration => {
            let step = fixed_time_step(desired);
            let values = fixed_ticks(min, max, step)
                .into_iter()
                .map(|value| (value, true))
                .collect();
            label_ticks(values, format, step, false, target)
        }
        AxisFormat::DateTime => {
            let (values, step) = match date_time_step(desired) {
                TimeStep::Fixed(step) => (datetime_fixed_ticks(min, max, step), step),
                TimeStep::Months(months) => match calendar_axis_ticks(min, max, months, target) {
                    Some(ticks) => ticks,
                    // chrono's calendar range is narrower than f64's range.
                    None => {
                        let step = nice_step(desired);
                        (fixed_ticks(min, max, step), step)
                    }
                },
            };
            label_ticks(
                values.into_iter().map(|value| (value, true)).collect(),
                format,
                step,
                false,
                target,
            )
        }
        AxisFormat::Category(_) => unreachable!("categories are handled above"),
    }
}

fn ordered_bounds(min: f64, max: f64) -> Option<(f64, f64)> {
    if !min.is_finite() || !max.is_finite() {
        None
    } else if min <= max {
        Some((min, max))
    } else {
        Some((max, min))
    }
}

fn desired_step(min: f64, max: f64, target: usize) -> f64 {
    let intervals = (target.clamp(2, MAX_TICKS) - 1) as f64;
    // Halving before subtraction prevents overflow across zero.
    let half_span = max * 0.5 - min * 0.5;
    let desired = (half_span / intervals) * 2.0;
    if desired.is_infinite() {
        f64::MAX
    } else if desired == 0.0 {
        f64::from_bits(1)
    } else {
        desired
    }
}

fn nice_step(raw: f64) -> f64 {
    // Rescale the ends of the f64 range before calculating powers of ten, so
    // the power itself neither underflows to zero nor overflows to infinity.
    let (scaled, undo) = if raw < 1e-300 {
        (raw * 1e300, 1e-300)
    } else if raw > 1e300 {
        (raw * 1e-300, 1e300)
    } else {
        (raw, 1.0)
    };
    let exponent = scaled.log10().floor() as i32;
    let power = 10_f64.powi(exponent);
    let fraction = scaled / power;
    let factor = if fraction <= 1.0 + 1e-12 {
        1.0
    } else if fraction <= 2.0 + 2e-12 {
        2.0
    } else if fraction <= 5.0 + 5e-12 {
        5.0
    } else {
        10.0
    };
    let step = power * factor * undo;
    if step.is_infinite() {
        f64::MAX
    } else if step == 0.0 {
        f64::from_bits(1)
    } else {
        step
    }
}

fn fixed_ticks(min: f64, max: f64, step: f64) -> Vec<f64> {
    if !step.is_finite() || step <= 0.0 {
        return Vec::new();
    }
    let first_index = (min / step).ceil();
    if !first_index.is_finite() {
        return Vec::new();
    }
    let mut result: Vec<f64> = Vec::new();
    let tolerance = (step * 1e-10).max(min.abs().max(max.abs()) * f64::EPSILON * 2.0);
    for index in 0..MAX_TICKS {
        let mut value = (first_index + index as f64) * step;
        if let Some(&previous) = result.last() {
            if value <= previous {
                value = previous + step;
                if value <= previous {
                    value = previous.next_up();
                }
            }
        }
        if !value.is_finite() {
            break;
        }
        if value < min {
            if min - value <= tolerance {
                value = min;
            } else {
                continue;
            }
        }
        if value > max {
            if value - max <= tolerance {
                value = max;
            } else {
                break;
            }
        }
        if result.last().is_some_and(|previous| value <= *previous) {
            break;
        }
        result.push(if value == 0.0 { 0.0 } else { value });
    }
    result
}

fn datetime_fixed_ticks(min: f64, max: f64, step: f64) -> Vec<f64> {
    if step == 7.0 * DAY || step == 14.0 * DAY {
        // The Unix epoch was a Thursday; align week ticks to Monday.
        let offset = -3.0 * DAY;
        fixed_ticks(min - offset, max - offset, step)
            .into_iter()
            .map(|value| value + offset)
            .filter(|value| *value >= min && *value <= max)
            .collect()
    } else {
        fixed_ticks(min, max, step)
    }
}

fn log_value(exponent: i32, multiplier: u32) -> f64 {
    let multiplier = multiplier as f64;
    if exponent < -300 {
        (10_f64.powi(exponent + 300) * multiplier) * 1e-300
    } else if exponent > 300 {
        (10_f64.powi(exponent - 300) * multiplier) * 1e300
    } else {
        10_f64.powi(exponent) * multiplier
    }
}

fn log_ticks(min: f64, max: f64, target: usize) -> Vec<(f64, bool)> {
    // Include adjacent decades to tolerate rounding in log10 at exact powers.
    let first_exp = ((min.log10().floor() as i32) - 1).max(-324);
    let last_exp = (max.log10().ceil() as i32).min(308);
    let decades = (last_exp - first_exp + 1) as usize;
    // Label at most about `target` decades; crowded decade labels are unreadable.
    let stride = if decades > target.max(2) + 1 || decades > MAX_TICKS {
        decades.div_ceil(target.max(1)) as i32
    } else {
        1
    };
    let include_minors = stride == 1 && decades <= MAX_TICKS / 9;
    let mut ticks: Vec<(f64, bool)> = Vec::new();
    for exponent in first_exp..=last_exp {
        if stride > 1 && exponent.rem_euclid(stride) != 0 {
            continue;
        }
        let last_multiplier = if include_minors { 9 } else { 1 };
        for multiplier in 1..=last_multiplier {
            let mut value = log_value(exponent, multiplier);
            if !value.is_finite() || value == 0.0 {
                continue;
            }
            let tolerance = value * 1e-12;
            if value < min {
                if min - value <= tolerance {
                    value = min;
                } else {
                    continue;
                }
            }
            if value > max {
                if value - max <= tolerance {
                    value = max;
                } else {
                    continue;
                }
            }
            if let Some((previous, previous_major)) = ticks.last_mut() {
                if value == *previous && multiplier == 1 {
                    *previous_major = true;
                }
                if value <= *previous {
                    continue;
                }
            }
            ticks.push((value, multiplier == 1));
            if ticks.len() == MAX_TICKS {
                return ticks;
            }
        }
    }
    ticks
}

fn category_ticks(min: f64, max: f64, target: usize, categories: &[String]) -> Vec<Tick> {
    if categories.is_empty() || max < 0.0 || min > (categories.len() - 1) as f64 {
        return Vec::new();
    }
    let first = min.ceil().max(0.0) as usize;
    let last = (max.floor() as usize).min(categories.len() - 1);
    if first > last {
        return Vec::new();
    }
    let count = last - first + 1;
    let stride = count.div_ceil(target).max(1);
    let offset = first % stride;
    let start = if offset == 0 {
        first
    } else {
        match first.checked_add(stride - offset) {
            Some(start) => start,
            None => return Vec::new(),
        }
    };
    (start..=last)
        .step_by(stride)
        .take(MAX_TICKS)
        .map(|index| Tick {
            value: index as f64,
            label: categories[index].clone(),
            major: true,
        })
        .collect()
}

fn label_ticks(
    values: Vec<(f64, bool)>,
    format: &AxisFormat,
    step: f64,
    logarithmic: bool,
    target: usize,
) -> Vec<Tick> {
    let major_count = values.iter().filter(|(_, major)| *major).count();
    let minor_count = values.len() - major_count;
    let minor_stride = if logarithmic && major_count < 2 && minor_count > 0 {
        minor_count.div_ceil(target.saturating_sub(major_count).max(1))
    } else {
        usize::MAX
    };
    // Across two or three decades, label the 2× and 5× ticks too (or only
    // the 5× ticks when space is tighter) so the axis is not left with just
    // two numbers.
    let mantissas: &[f64] = if !logarithmic || major_count < 2 {
        &[]
    } else if major_count * 3 <= target + 1 {
        &[2.0, 5.0]
    } else if major_count * 2 <= target + 1 {
        &[5.0]
    } else {
        &[]
    };
    let precision = if logarithmic {
        4
    } else {
        number_precision(&values, step)
    };
    let duration_max = values
        .iter()
        .map(|(value, _)| value.abs())
        .fold(0.0_f64, f64::max);
    let duration_step = if logarithmic && duration_max > 0.0 {
        log_value(duration_max.log10().floor() as i32, 1)
    } else {
        step
    };
    let duration_style = duration_style(duration_max, duration_step);
    let mut minor_index = 0;
    let mut last_date = None;
    values
        .into_iter()
        .map(|(value, major)| {
            let show_label = if major {
                true
            } else if !mantissas.is_empty() {
                let mantissa = value / log_value(value.log10().floor() as i32, 1);
                mantissas.iter().any(|m| (mantissa - m).abs() < 1e-9 * m)
            } else {
                let selected = minor_index % minor_stride == 0;
                minor_index += 1;
                selected && minor_stride != usize::MAX
            };
            let label = if !show_label {
                String::new()
            } else {
                let label_step = if logarithmic {
                    let decade = log_value(value.log10().floor() as i32, 1);
                    if decade > 0.0 {
                        decade
                    } else {
                        value
                    }
                } else {
                    step
                };
                match format {
                    AxisFormat::Number if logarithmic => log_label(value, precision),
                    AxisFormat::Number => format_value(value, "", precision),
                    // Sub-second axes read better in ms/µs/ns (200 µs) than as
                    // fractions of a second (0.0002 s). One prefix serves the
                    // whole axis so labels stay comparable.
                    AxisFormat::Duration
                        if duration_style == DurationStyle::Seconds
                            && duration_max < 1.0
                            && !logarithmic =>
                    {
                        let (scale, unit) = if duration_max >= 1e-3 {
                            (1e-3, "ms")
                        } else if duration_max >= 1e-6 {
                            (1e-6, "µs")
                        } else {
                            (1e-9, "ns")
                        };
                        let decimals =
                            (-(label_step / scale).log10()).ceil().clamp(0.0, 9.0) as usize;
                        let scaled = value / scale;
                        let scaled = if scaled.abs() < label_step / scale * 1e-6 {
                            0.0
                        } else {
                            scaled
                        };
                        format!("{scaled:.decimals$} {unit}")
                    }
                    AxisFormat::Duration => {
                        format_duration_with_style(value, label_step, duration_style)
                    }
                    AxisFormat::DateTime => {
                        if label_step < DAY {
                            let date = date_at(value, label_step);
                            let show_date = date.is_some() && date != last_date;
                            last_date = date;
                            if show_date {
                                format_time_with_date(value, label_step)
                            } else {
                                format_time(value, label_step)
                            }
                        } else {
                            format_time(value, label_step)
                        }
                    }
                    AxisFormat::Category(_) => unreachable!("categories are handled above"),
                }
            };
            Tick {
                value,
                label,
                major,
            }
        })
        .collect()
}

/// Decades far from 1 read better as powers of ten ("10⁶", "10⁻⁴") than as
/// long digit strings; other values use the regular number format.
fn log_label(value: f64, precision: usize) -> String {
    let exponent = value.log10().round();
    let decade = 10_f64.powf(exponent);
    if (value / decade - 1.0).abs() < 1e-9 && !(-2.0..=4.0).contains(&exponent) {
        const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
        let mut label = String::from("10");
        if exponent < 0.0 {
            label.push('⁻');
        }
        for digit in (exponent.abs() as u32).to_string().bytes() {
            label.push(DIGITS[usize::from(digit - b'0')]);
        }
        label
    } else {
        format_value(value, "", precision)
    }
}

fn number_precision(values: &[(f64, bool)], step: f64) -> usize {
    let max_abs = values
        .iter()
        .map(|(value, _)| value.abs())
        .fold(0.0, f64::max);
    if max_abs == 0.0 || !step.is_finite() || step <= 0.0 {
        return 4;
    }
    let magnitude = max_abs.log10().floor() as i32;
    let step_magnitude = step.log10().floor() as i32;
    (magnitude - step_magnitude + 1).clamp(4, 17) as usize
}

fn fixed_time_step(desired: f64) -> f64 {
    const STEPS: [f64; 23] = [
        1.0,
        2.0,
        5.0,
        10.0,
        15.0,
        30.0,
        60.0,
        120.0,
        300.0,
        600.0,
        900.0,
        1800.0,
        3600.0,
        7200.0,
        10_800.0,
        21_600.0,
        43_200.0,
        DAY,
        2.0 * DAY,
        7.0 * DAY,
        14.0 * DAY,
        30.0 * DAY,
        90.0 * DAY,
    ];
    if desired < 1.0 {
        nice_step(desired)
    } else if let Some(step) = STEPS.into_iter().find(|step| *step >= desired) {
        step
    } else {
        let step = nice_step(desired / DAY) * DAY;
        if step.is_finite() {
            step
        } else {
            f64::MAX
        }
    }
}

enum TimeStep {
    Fixed(f64),
    Months(i64),
}

fn date_time_step(desired: f64) -> TimeStep {
    if desired <= 14.0 * DAY {
        return TimeStep::Fixed(fixed_time_step(desired));
    }
    for months in [1_i64, 2, 3, 6, 12] {
        if desired <= months as f64 * MONTH {
            return TimeStep::Months(months);
        }
    }
    let years = nice_step(desired / YEAR);
    let months = (years as i64).saturating_mul(12).max(12);
    TimeStep::Months(months)
}

fn calendar_axis_ticks(
    min: f64,
    max: f64,
    mut months: i64,
    target: usize,
) -> Option<(Vec<f64>, f64)> {
    // A coarse calendar step can place every tick outside a short view.
    // Reduce it until the visible interval has useful calendar labels.
    for _ in 0..64 {
        let ticks = calendar_ticks(min, max, months)?;
        if ticks.len() >= 2 || (target == 1 && !ticks.is_empty()) {
            return Some((ticks, months as f64 * MONTH));
        }
        months = match months {
            1 => {
                let step = 7.0 * DAY;
                return Some((datetime_fixed_ticks(min, max, step), step));
            }
            2 => 1,
            3 => 2,
            4..=6 => 3,
            7..=12 => 6,
            _ => ((months / 12) / 2).max(1) * 12,
        };
    }
    None
}

fn calendar_ticks(min: f64, max: f64, months: i64) -> Option<Vec<f64>> {
    if min < i64::MIN as f64 || min >= i64::MAX as f64 {
        return None;
    }
    let start = DateTime::from_timestamp(min.floor() as i64, 0)?.date_naive();
    let month_index = i64::from(start.year()) * 12 + i64::from(start.month0());
    let mut current = month_index.div_euclid(months).checked_mul(months)?;
    let mut result = Vec::new();
    for _ in 0..=MAX_TICKS {
        let year = i32::try_from(current.div_euclid(12)).ok()?;
        let month = current.rem_euclid(12) as u32 + 1;
        let date = NaiveDate::from_ymd_opt(year, month, 1)?;
        let value = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp() as f64;
        if value > max {
            break;
        }
        if value >= min {
            result.push(value);
            if result.len() == MAX_TICKS {
                break;
            }
        }
        current = current.checked_add(months)?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::NaiveDate;

    use super::{axis_ticks, nice_ticks, MAX_TICKS};
    use crate::{AxisFormat, Scale};

    fn epoch(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> f64 {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_utc()
            .timestamp() as f64
    }

    #[test]
    fn nice_ticks_handle_negative_tiny_and_huge_ranges() {
        assert_eq!(nice_ticks(-2.0, 2.0, 5), vec![-2.0, -1.0, 0.0, 1.0, 2.0]);
        assert_eq!(nice_ticks(2.0, -2.0, 5), nice_ticks(-2.0, 2.0, 5));
        let tiny = nice_ticks(1e-300, 5e-300, 5);
        assert!(!tiny.is_empty());
        assert!(tiny.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(tiny
            .iter()
            .all(|x| x.is_finite() && *x >= 1e-300 && *x <= 5e-300));
        let subnormal = nice_ticks(0.0, f64::from_bits(4), 5);
        assert!(!subnormal.is_empty() && subnormal.len() <= MAX_TICKS);
        assert!(subnormal.windows(2).all(|pair| pair[0] < pair[1]));
        let huge = nice_ticks(-1e308, 1e308, 5);
        assert!(huge.len() >= 3 && huge.len() <= MAX_TICKS);
        assert!(huge.iter().all(|x| x.is_finite()));
        assert!(nice_ticks(f64::NAN, 1.0, 5).is_empty());
        assert!(nice_ticks(0.0, 1.0, usize::MAX).len() <= MAX_TICKS);
    }

    #[test]
    fn logarithmic_ticks_have_decade_majors_and_minor_labels_when_zoomed() {
        let ticks = axis_ticks(1.0, 100.0, 5, &AxisFormat::Number, Scale::Log10);
        for value in [1.0, 10.0, 100.0] {
            assert!(ticks
                .iter()
                .any(|tick| tick.value == value && tick.major && !tick.label.is_empty()));
        }
        for value in 2..=9 {
            assert!(ticks
                .iter()
                .any(|tick| tick.value == value as f64 && !tick.major));
        }
        let zoomed = axis_ticks(2.0, 9.0, 4, &AxisFormat::Number, Scale::Log10);
        assert!(zoomed.iter().all(|tick| !tick.major));
        assert!(zoomed.iter().any(|tick| !tick.label.is_empty()));
        assert!(axis_ticks(-1.0, 10.0, 5, &AxisFormat::Number, Scale::Log10).is_empty());
        let extreme = axis_ticks(
            f64::from_bits(1),
            1e308,
            8,
            &AxisFormat::Number,
            Scale::Log10,
        );
        assert!(!extreme.is_empty() && extreme.len() <= MAX_TICKS);
        let dense_target = axis_ticks(
            f64::from_bits(1),
            1e308,
            MAX_TICKS,
            &AxisFormat::Number,
            Scale::Log10,
        );
        assert!(dense_target.last().is_some_and(|tick| tick.value >= 1e300));
        let boundary = axis_ticks(1e-300, 1e-299, 5, &AxisFormat::Number, Scale::Log10);
        assert!(boundary
            .iter()
            .any(|tick| tick.major && (tick.value / 1e-300 - 1.0).abs() < 1e-12));
        assert!(boundary
            .iter()
            .any(|tick| tick.major && (tick.value / 1e-299 - 1.0).abs() < 1e-12));
        let durations = axis_ticks(0.001, 0.01, 5, &AxisFormat::Duration, Scale::Log10);
        assert!(durations.iter().any(|tick| tick.label == "0.001 s"));
        assert!(durations.iter().any(|tick| tick.label == "0.01 s"));
    }

    #[test]
    fn datetime_ticks_show_new_dates_at_midnight_and_new_year() {
        let ticks = axis_ticks(
            epoch(2023, 12, 31, 23, 59),
            epoch(2024, 1, 1, 0, 2),
            4,
            &AxisFormat::DateTime,
            Scale::Linear,
        );
        assert_eq!(ticks[0].label, "2023-12-31 23:59");
        assert_eq!(ticks[1].label, "2024-01-01 00:00");
        assert_eq!(ticks[2].label, "00:01");
    }

    #[test]
    fn datetime_week_ticks_start_on_monday() {
        let ticks = axis_ticks(
            epoch(2024, 1, 1, 0, 0),
            epoch(2024, 1, 22, 0, 0),
            4,
            &AxisFormat::DateTime,
            Scale::Linear,
        );
        assert_eq!(
            ticks
                .iter()
                .map(|tick| tick.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2024-01-01", "2024-01-08", "2024-01-15", "2024-01-22"]
        );
    }

    #[test]
    fn datetime_months_and_years_use_calendar_boundaries() {
        let months = axis_ticks(
            epoch(2024, 1, 15, 0, 0),
            epoch(2024, 5, 20, 0, 0),
            6,
            &AxisFormat::DateTime,
            Scale::Linear,
        );
        assert_eq!(
            months
                .iter()
                .map(|tick| tick.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2024-02", "2024-03", "2024-04", "2024-05"]
        );
        assert_eq!(months[1].value - months[0].value, 29.0 * 86_400.0);
        let years = axis_ticks(
            epoch(2022, 7, 1, 0, 0),
            epoch(2026, 7, 1, 0, 0),
            6,
            &AxisFormat::DateTime,
            Scale::Linear,
        );
        assert_eq!(
            years
                .iter()
                .map(|tick| tick.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2023", "2024", "2025", "2026"]
        );
        let sparse_month = axis_ticks(
            epoch(2024, 1, 2, 0, 0),
            epoch(2024, 1, 20, 0, 0),
            2,
            &AxisFormat::DateTime,
            Scale::Linear,
        );
        assert_eq!(
            sparse_month
                .iter()
                .map(|tick| tick.label.as_str())
                .collect::<Vec<_>>(),
            vec!["2024-01-08", "2024-01-15"]
        );
    }

    #[test]
    fn duration_and_category_labels_use_data_coordinates() {
        let duration = axis_ticks(-120.0, 120.0, 5, &AxisFormat::Duration, Scale::Linear);
        assert_eq!(
            duration
                .iter()
                .map(|tick| tick.label.as_str())
                .collect::<Vec<_>>(),
            vec!["-2:00", "-1:00", "0:00", "1:00", "2:00"]
        );
        let categories =
            AxisFormat::Category(Arc::from(["Idle".into(), "Load".into(), "Cool".into()]));
        let ticks = axis_ticks(-2.0, 3.0, 5, &categories, Scale::Linear);
        assert_eq!(
            ticks
                .iter()
                .map(|tick| (tick.value, tick.label.as_str()))
                .collect::<Vec<_>>(),
            vec![(0.0, "Idle"), (1.0, "Load"), (2.0, "Cool")]
        );
    }

    #[test]
    fn duration_ticks_use_one_style_for_the_entire_axis() {
        let labels = |max, target| {
            axis_ticks(0.0, max, target, &AxisFormat::Duration, Scale::Linear)
                .into_iter()
                .map(|tick| tick.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(120.0, 9),
            ["0:00", "0:15", "0:30", "0:45", "1:00", "1:15", "1:30", "1:45", "2:00"]
        );
        assert_eq!(labels(7200.0, 3), ["0:00:00", "1:00:00", "2:00:00"]);
        // Oscilloscope captures: one SI prefix for the whole axis.
        let scope: Vec<_> = axis_ticks(-0.0005, 0.0005, 5, &AxisFormat::Duration, Scale::Linear)
            .into_iter()
            .map(|tick| tick.label)
            .collect();
        assert_eq!(scope, ["-500 µs", "0 µs", "500 µs"]);
        assert_eq!(labels(0.002, 4), ["0 ms", "1 ms", "2 ms"]);
    }
}
