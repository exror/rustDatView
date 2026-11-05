//! Pixel-column M4 reduction and reusable range-extrema indexes.
//!
//! A `[NaN, NaN]` point separates disconnected runs in every returned vector.
//! Consumers must start a new path at each such point.

const BLOCK_SIZE: usize = 32;
const NO_INDEX: usize = usize::MAX;
const BREAK: [f64; 2] = [f64::NAN, f64::NAN];

/// Returns whether every x is finite and the sequence is nondecreasing.
///
/// Equal x values are allowed. An empty slice is sorted; a NaN or infinity is
/// not, even when it is the only value.
pub fn is_sorted(x: &[f64]) -> bool {
    x.iter().all(|value| value.is_finite()) && x.windows(2).all(|pair| pair[0] <= pair[1])
}

/// Reduces a series to the first, minimum-y, maximum-y, and last sample in
/// each occupied pixel column, in original sample order.
///
/// The x coordinates are normally sorted. Non-finite x or y values break the
/// line, and the samples immediately outside each visible run are retained
/// where available. Bounds may be reversed or infinite. A zero pixel width or
/// NaN bound produces an empty vector. Only paired x/y entries are considered.
/// For nonmonotonic x, the visible raw samples and their neighbors are
/// returned instead: reducing such a path by x column could change its shape.
pub fn m4(x: &[f64], y: &[f64], x0: f64, x1: f64, px: usize) -> Vec<[f64; 2]> {
    if px == 0 || x0.is_nan() || x1.is_nan() {
        return Vec::new();
    }
    M4Index::new(x, y, None).query(x, y, x0, x1, px)
}

#[derive(Clone, Copy, Debug)]
struct Run {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug)]
struct Extrema {
    min: usize,
    max: usize,
    min_positive: usize,
}

impl Extrema {
    const EMPTY: Self = Self {
        min: NO_INDEX,
        max: NO_INDEX,
        min_positive: NO_INDEX,
    };

    fn include(&mut self, index: usize, y: &[f64]) {
        if self.min == NO_INDEX
            || y[index] < y[self.min]
            || (y[index] == y[self.min] && index < self.min)
        {
            self.min = index;
        }
        if self.max == NO_INDEX
            || y[index] > y[self.max]
            || (y[index] == y[self.max] && index < self.max)
        {
            self.max = index;
        }
        if y[index] > 0.0
            && (self.min_positive == NO_INDEX
                || y[index] < y[self.min_positive]
                || (y[index] == y[self.min_positive] && index < self.min_positive))
        {
            self.min_positive = index;
        }
    }

    fn merge(&mut self, other: Self, y: &[f64]) {
        if other.min != NO_INDEX {
            self.include(other.min, y);
            self.include(other.max, y);
        }
        if other.min_positive != NO_INDEX {
            self.include(other.min_positive, y);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Gap {
    median: f64,
    factor: f64,
    threshold: f64,
}

impl Gap {
    fn from_x(x: &[f64], factor: Option<f64>) -> Option<Self> {
        let factor = factor.filter(|factor| factor.is_finite() && *factor >= 0.0)?;
        let mut deltas: Vec<f64> = x
            .windows(2)
            .filter_map(|pair| {
                if !pair[0].is_finite() || !pair[1].is_finite() {
                    return None;
                }
                let delta = pair[1] - pair[0];
                (delta.is_finite() && delta >= 0.0).then_some(delta)
            })
            .collect();
        if deltas.is_empty() {
            return None;
        }
        let middle = deltas.len() / 2;
        let upper = *deltas
            .select_nth_unstable_by(middle, |a, b| a.total_cmp(b))
            .1;
        let median = if deltas.len() % 2 == 0 {
            let lower = deltas[..middle]
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            lower + (upper - lower) * 0.5
        } else {
            upper
        };
        Some(Self {
            median,
            factor,
            threshold: median * factor,
        })
    }

    fn separates(self, previous: f64, current: f64) -> bool {
        let delta = current - previous;
        if delta.is_infinite() && self.threshold.is_infinite() {
            // Both full-size quantities can overflow although their halves do not.
            current * 0.5 - previous * 0.5 > (self.median * 0.5) * self.factor
        } else {
            delta > self.threshold
        }
    }
}

/// Properties depending only on a paired X slice, shared by series that use it.
#[derive(Clone, Debug)]
pub(crate) struct XInfo {
    len: usize,
    gap_factor: Option<f64>,
    gap: Option<Gap>,
    pub sorted: bool,
}
impl XInfo {
    pub fn new(x: &[f64], gap_factor: Option<f64>) -> Self {
        Self {
            len: x.len(),
            gap_factor,
            gap: Gap::from_x(x, gap_factor),
            sorted: is_sorted(x),
        }
    }
}

/// Reusable M4 range-extrema index for a single series generation.
///
/// Construction is linear in the number of paired samples. For monotonic x,
/// queries binary-search the relevant runs and pixel columns, and use a
/// block-based segment tree for extrema; they do not rescan all visible data.
/// The index stores sample positions, not the x/y arrays. Pass the same sample
/// contents to queries as were supplied to `new`, and rebuild it whenever the
/// contents or gap factor change. If the paired length changes, queries build a
/// temporary replacement rather than indexing beyond the supplied slices.
///
/// Non-finite coordinates separate runs. When `gap_factor` is finite and
/// nonnegative, adjacent valid points also separate when their positive Δx is
/// greater than the factor times the median finite nonnegative adjacent x
/// difference. The median includes zero intervals and uses x even where y is
/// non-finite; negative differences are excluded. `None`, an invalid factor, or
/// an unavailable median disables automatic gap detection.
#[derive(Clone, Debug)]
pub struct M4Index {
    len: usize,
    gap_factor: Option<f64>,
    ordered: bool,
    runs: Vec<Run>,
    positive_runs: Vec<Run>,
    tree_base: usize,
    tree: Vec<Extrema>,
    x_bounds: Option<[f64; 2]>,
    smallest_positive_x: Option<f64>,
}

impl M4Index {
    /// Builds an index over the paired prefix of `x` and `y`.
    pub fn new(x: &[f64], y: &[f64], gap_factor: Option<f64>) -> Self {
        let len = x.len().min(y.len());
        let info = XInfo::new(&x[..len], gap_factor);
        Self::with_x_info(x, y, &info)
    }

    pub(crate) fn with_x_info(x: &[f64], y: &[f64], info: &XInfo) -> Self {
        let len = x.len().min(y.len());
        if len != info.len {
            return Self::new(x, y, info.gap_factor);
        }
        let gap = info.gap;
        let mut runs = Vec::new();
        let mut positive_runs = Vec::new();
        let mut run_start = None;
        let mut positive_start = None;
        let mut previous_x = None;
        let mut ordered = true;
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_positive_x = f64::INFINITY;

        for i in 0..len {
            let xi = x[i];
            if !xi.is_finite() || !y[i].is_finite() {
                if let Some(start) = run_start.take() {
                    runs.push(Run { start, end: i });
                }
                if let Some(start) = positive_start.take() {
                    positive_runs.push(Run { start, end: i });
                }
                continue;
            }

            min_x = min_x.min(xi);
            max_x = max_x.max(xi);
            if xi > 0.0 {
                min_positive_x = min_positive_x.min(xi);
            }
            if previous_x.is_some_and(|previous| xi < previous) {
                ordered = false;
            }
            previous_x = Some(xi);

            let gap_before =
                run_start.is_some() && gap.is_some_and(|gap| gap.separates(x[i - 1], xi));
            match run_start {
                Some(start) if gap_before => {
                    runs.push(Run { start, end: i });
                    run_start = Some(i);
                }
                None => run_start = Some(i),
                _ => {}
            }
            if y[i] > 0.0 {
                match positive_start {
                    Some(start) if gap_before => {
                        positive_runs.push(Run { start, end: i });
                        positive_start = Some(i);
                    }
                    None => positive_start = Some(i),
                    _ => {}
                }
            } else if let Some(start) = positive_start.take() {
                positive_runs.push(Run { start, end: i });
            }
        }
        if let Some(start) = run_start {
            runs.push(Run { start, end: len });
        }
        if let Some(start) = positive_start {
            positive_runs.push(Run { start, end: len });
        }

        let (tree_base, tree) = if ordered && !runs.is_empty() {
            let blocks = len.div_ceil(BLOCK_SIZE);
            let base = blocks.next_power_of_two();
            let mut tree = vec![Extrema::EMPTY; base * 2];
            for run in &runs {
                for i in run.start..run.end {
                    tree[base + i / BLOCK_SIZE].include(i, y);
                }
            }
            for node in (1..base).rev() {
                let mut extrema = tree[node * 2];
                extrema.merge(tree[node * 2 + 1], y);
                tree[node] = extrema;
            }
            (base, tree)
        } else {
            (0, Vec::new())
        };

        Self {
            len,
            gap_factor: info.gap_factor,
            ordered,
            runs,
            positive_runs,
            tree_base,
            tree,
            x_bounds: (min_x != f64::INFINITY).then_some([min_x, max_x]),
            smallest_positive_x: min_positive_x.is_finite().then_some(min_positive_x),
        }
    }

    /// Full x bounds over samples whose x and y are both finite.
    pub fn x_bounds(&self) -> Option<[f64; 2]> {
        self.x_bounds
    }

    /// Smallest strictly positive x over samples whose x and y are both finite.
    pub fn smallest_positive_x(&self) -> Option<f64> {
        self.smallest_positive_x
    }

    /// Bounds of finite y samples inside the inclusive visible x range.
    /// Off-view edge neighbors do not affect these bounds. Monotonic series
    /// use the range index, including across NaN and time-gap breaks.
    pub fn y_bounds(&self, x: &[f64], y: &[f64], x0: f64, x1: f64) -> Option<[f64; 2]> {
        self.y_bounds_impl::<false>(x, y, x0, x1)
    }

    /// Bounds of strictly positive, finite y samples inside the inclusive
    /// visible x range. Off-view edge neighbors do not affect these bounds.
    ///
    /// For monotonic series this uses the range index, including across NaN
    /// and time-gap breaks, rather than scanning the visible samples.
    pub fn positive_y_bounds(&self, x: &[f64], y: &[f64], x0: f64, x1: f64) -> Option<[f64; 2]> {
        self.y_bounds_impl::<true>(x, y, x0, x1)
    }

    fn y_bounds_impl<const POSITIVE_ONLY: bool>(
        &self,
        x: &[f64],
        y: &[f64],
        x0: f64,
        x1: f64,
    ) -> Option<[f64; 2]> {
        if self.len != x.len().min(y.len()) {
            return Self::new(x, y, self.gap_factor).y_bounds_impl::<POSITIVE_ONLY>(x, y, x0, x1);
        }
        let [left, right] = self.view_bounds(x0, x1, false)?;
        if !self.ordered {
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for i in 0..self.len {
                if x[i].is_finite()
                    && x[i] >= left
                    && x[i] <= right
                    && y[i].is_finite()
                    && (!POSITIVE_ONLY || y[i] > 0.0)
                {
                    min = min.min(y[i]);
                    max = max.max(y[i]);
                }
            }
            return min.is_finite().then_some([min, max]);
        }

        let first = self.runs.partition_point(|run| x[run.end - 1] < left);
        let after = self.runs.partition_point(|run| x[run.start] <= right);
        if first >= after {
            return None;
        }
        let first_run = self.runs[first];
        let last_run = self.runs[after - 1];
        let start = first_run.start
            + x[first_run.start..first_run.end].partition_point(|value| *value < left);
        let end = last_run.start
            + x[last_run.start..last_run.end].partition_point(|value| *value <= right);
        if start >= end {
            return None;
        }
        let extrema = self.extrema::<true>(x, y, start, end);
        let min = if POSITIVE_ONLY {
            extrema.min_positive
        } else {
            extrema.min
        };
        (min != NO_INDEX).then(|| [y[min], y[extrema.max]])
    }

    /// Queries the visible range with M4 reduction, preserving breaks and the
    /// adjacent edge samples. Nonmonotonic series use a raw linear fallback.
    pub fn query(&self, x: &[f64], y: &[f64], x0: f64, x1: f64, px: usize) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, Some(px), false, false)
    }

    /// Queries the visible positive-x domain using equal-width log10 columns.
    /// Nonpositive x values break paths and are never returned.
    pub fn query_log(&self, x: &[f64], y: &[f64], x0: f64, x1: f64, px: usize) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, Some(px), true, false)
    }

    /// M4 query for a logarithmic y axis. Nonpositive y values break paths.
    /// Set `log_x` to use positive x and logarithmic pixel columns as well.
    pub fn query_log_y(
        &self,
        x: &[f64],
        y: &[f64],
        x0: f64,
        x1: f64,
        px: usize,
        log_x: bool,
    ) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, Some(px), log_x, true)
    }

    /// Queries without reducing samples, for drawing with decimation disabled.
    ///
    /// Duplicate x values, breaks, and the adjacent edge samples are retained.
    pub fn query_raw(&self, x: &[f64], y: &[f64], x0: f64, x1: f64) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, None, false, false)
    }

    /// The unreduced positive-x query for logarithmic axes.
    pub fn query_raw_log(&self, x: &[f64], y: &[f64], x0: f64, x1: f64) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, None, true, false)
    }

    /// Unreduced query for logarithmic y, optionally with logarithmic x.
    pub fn query_raw_log_y(
        &self,
        x: &[f64],
        y: &[f64],
        x0: f64,
        x1: f64,
        log_x: bool,
    ) -> Vec<[f64; 2]> {
        self.query_impl(x, y, x0, x1, None, log_x, true)
    }

    fn view_bounds(&self, x0: f64, x1: f64, log_x: bool) -> Option<[f64; 2]> {
        if x0.is_nan() || x1.is_nan() {
            return None;
        }
        let [data_min, data_max] = self.x_bounds?;
        let (mut left, mut right) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        if left == f64::NEG_INFINITY {
            left = data_min;
        }
        if right == f64::INFINITY {
            right = data_max;
        }
        if log_x && left <= 0.0 {
            left = self.smallest_positive_x?;
        }
        (left.is_finite()
            && right.is_finite()
            && left <= right
            && left <= data_max
            && right >= data_min
            && (!log_x || right > 0.0))
            .then_some([left, right])
    }

    fn query_impl(
        &self,
        x: &[f64],
        y: &[f64],
        x0: f64,
        x1: f64,
        px: Option<usize>,
        log_x: bool,
        log_y: bool,
    ) -> Vec<[f64; 2]> {
        if px == Some(0) {
            return Vec::new();
        }
        if self.len != x.len().min(y.len()) {
            return Self::new(x, y, self.gap_factor).query_impl(x, y, x0, x1, px, log_x, log_y);
        }
        let Some([left, right]) = self.view_bounds(x0, x1, log_x) else {
            return Vec::new();
        };

        let runs = if log_y {
            &self.positive_runs
        } else {
            &self.runs
        };
        if runs.is_empty() {
            return Vec::new();
        }
        let capacity = px.map_or(0, |px| self.len.min(px.saturating_mul(4)));
        let mut points = Vec::with_capacity(capacity);
        if !self.ordered {
            self.query_unordered(x, y, left, right, log_x, runs, &mut points);
            return points;
        }
        let scale = px.map(|px| PixelScale::new(left, right, px, log_x));

        // Each run is monotonic and their end points are monotonic across runs.
        let first_run = runs.partition_point(|run| x[run.end - 1] < left);
        for run in &runs[first_run..] {
            if x[run.start] > right {
                break;
            }
            let run_x = &x[run.start..run.end];
            let start = run.start + run_x.partition_point(|value| *value < left);
            let end = run.start + run_x.partition_point(|value| *value <= right);
            if !points.is_empty() {
                points.push(BREAK);
            }
            if start > run.start && (!log_x || x[start - 1] > 0.0) {
                points.push([x[start - 1], y[start - 1]]);
            }

            if let Some(scale) = scale {
                let mut at = start;
                while at < end {
                    let column = scale.column(x[at]);
                    let mut lo = at + 1;
                    let mut hi = end;
                    while lo < hi {
                        let mid = lo + (hi - lo) / 2;
                        if scale.column(x[mid]) <= column {
                            lo = mid + 1;
                        } else {
                            hi = mid;
                        }
                    }
                    let extrema = self.extrema::<false>(x, y, at, lo);
                    let mut candidates = [at, extrema.min, extrema.max, lo - 1];
                    candidates.sort_unstable();
                    for (position, &index) in candidates.iter().enumerate() {
                        if position == 0 || index != candidates[position - 1] {
                            points.push([x[index], y[index]]);
                        }
                    }
                    at = lo;
                }
            } else {
                points.extend((start..end).map(|i| [x[i], y[i]]));
            }

            if end < run.end {
                points.push([x[end], y[end]]);
            }
        }
        points
    }

    fn extrema<const CHECK_VALID: bool>(
        &self,
        x: &[f64],
        y: &[f64],
        start: usize,
        end: usize,
    ) -> Extrema {
        let mut result = Extrema::EMPTY;
        let mut at = start;
        while at < end && at % BLOCK_SIZE != 0 {
            if !CHECK_VALID || (x[at].is_finite() && y[at].is_finite()) {
                result.include(at, y);
            }
            at += 1;
        }

        let full_end = end - end % BLOCK_SIZE;
        let mut left = self.tree_base + at / BLOCK_SIZE;
        let mut right = self.tree_base + full_end / BLOCK_SIZE;
        while left < right {
            if left % 2 != 0 {
                result.merge(self.tree[left], y);
                left += 1;
            }
            if right % 2 != 0 {
                right -= 1;
                result.merge(self.tree[right], y);
            }
            left /= 2;
            right /= 2;
        }
        for i in at.max(full_end)..end {
            if !CHECK_VALID || (x[i].is_finite() && y[i].is_finite()) {
                result.include(i, y);
            }
        }
        result
    }

    fn query_unordered(
        &self,
        x: &[f64],
        y: &[f64],
        left: f64,
        right: f64,
        log_x: bool,
        runs: &[Run],
        points: &mut Vec<[f64; 2]>,
    ) {
        for run in runs {
            let mut connected = false;
            for i in run.start..run.end {
                if log_x && x[i] <= 0.0 {
                    connected = false;
                    continue;
                }
                let inside = x[i] >= left && x[i] <= right;
                let left_segment = i > run.start
                    && (!log_x || x[i - 1] > 0.0)
                    && intersects(x[i - 1], x[i], left, right);
                let right_segment = i + 1 < run.end
                    && (!log_x || x[i + 1] > 0.0)
                    && intersects(x[i], x[i + 1], left, right);
                if inside || left_segment || right_segment {
                    if !connected && !points.is_empty() {
                        points.push(BREAK);
                    }
                    points.push([x[i], y[i]]);
                    connected = true;
                } else {
                    connected = false;
                }
            }
        }
    }
}

fn intersects(a: f64, b: f64, left: f64, right: f64) -> bool {
    (a >= left || b >= left) && (a <= right || b <= right)
}

#[derive(Clone, Copy)]
struct PixelScale {
    left: f64,
    right: f64,
    span: f64,
    px: usize,
    log_x: bool,
}

impl PixelScale {
    fn new(left: f64, right: f64, px: usize, log_x: bool) -> Self {
        let (left, right) = if log_x {
            (left.log10(), right.log10())
        } else {
            (left, right)
        };
        Self {
            left,
            right,
            span: right - left,
            px,
            log_x,
        }
    }

    fn column(self, value: f64) -> usize {
        let value = if self.log_x { value.log10() } else { value };
        if self.left == self.right || value <= self.left {
            return 0;
        }
        if value >= self.right {
            return self.px - 1;
        }
        let fraction = if self.span.is_finite() {
            (value - self.left) / self.span
        } else {
            (value * 0.5 - self.left * 0.5) / (self.right * 0.5 - self.left * 0.5)
        };
        ((fraction * self.px as f64) as usize).min(self.px - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_sorted, m4, M4Index, XInfo};

    fn assert_points(actual: &[[f64; 2]], expected: &[[f64; 2]]) {
        assert_eq!(actual.len(), expected.len(), "actual: {actual:?}");
        for (actual, expected) in actual.iter().zip(expected) {
            for axis in 0..2 {
                if expected[axis].is_nan() {
                    assert!(actual[axis].is_nan(), "actual: {actual:?}");
                } else {
                    assert_eq!(actual[axis], expected[axis]);
                }
            }
        }
    }

    #[test]
    fn sorted_validation_rejects_nonfinite_and_decreasing_x() {
        assert!(is_sorted(&[]));
        assert!(is_sorted(&[1.0]));
        assert!(is_sorted(&[-0.0, 0.0, 1.0, 1.0, 2.0]));
        assert!(!is_sorted(&[0.0, 2.0, 1.0]));
        assert!(!is_sorted(&[f64::NAN]));
        assert!(!is_sorted(&[0.0, f64::INFINITY]));
        assert!(!is_sorted(&[f64::NEG_INFINITY, 0.0]));
    }

    #[test]
    fn shared_x_metadata_preserves_each_series_breaks_and_extrema() {
        let x = [0., 1., 2., 9., 10.];
        let a = [1., 3., f64::NAN, 2., 4.];
        let b = [5., 2., 8., 1., 7.];
        let info = XInfo::new(&x, Some(2.0));
        for y in [&a[..], &b[..]] {
            let shared = M4Index::with_x_info(&x, y, &info);
            let independent = M4Index::new(&x, y, Some(2.0));
            assert_points(
                &shared.query(&x, y, 0.0, 10.0, 3),
                &independent.query(&x, y, 0.0, 10.0, 3),
            );
            assert_eq!(
                shared.y_bounds(&x, y, 0.0, 10.0),
                independent.y_bounds(&x, y, 0.0, 10.0)
            );
        }
    }

    #[test]
    fn m4_keeps_first_min_max_last_in_sample_order() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [4.0, 9.0, 2.0, -5.0, 3.0, -5.0, 2.0];
        assert_points(
            &m4(&x, &y, 0.0, 6.0, 1),
            &[[0.0, 4.0], [1.0, 9.0], [3.0, -5.0], [6.0, 2.0]],
        );

        let y = [1.0, -4.0, 0.0, 6.0, 2.0, 3.0, 1.0];
        assert_points(
            &m4(&x, &y, 0.0, 6.0, 1),
            &[[0.0, 1.0], [1.0, -4.0], [3.0, 6.0], [6.0, 1.0]],
        );
    }

    #[test]
    fn columns_are_independent_and_right_boundary_is_included() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = [0.0, -2.0, 3.0, 1.0, 0.0, 4.0, -5.0, 2.0, 1.0];
        assert_points(
            &m4(&x, &y, 0.0, 8.0, 2),
            &[
                [0.0, 0.0],
                [1.0, -2.0],
                [2.0, 3.0],
                [3.0, 1.0],
                [4.0, 0.0],
                [5.0, 4.0],
                [6.0, -5.0],
                [8.0, 1.0],
            ],
        );
    }

    #[test]
    fn clipping_preserves_edge_neighbors_and_crossing_segments() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let index = M4Index::new(&x, &y, None);
        let expected = [[2.0, 2.0], [3.0, 3.0], [4.0, 4.0], [5.0, 5.0]];
        assert_points(&index.query(&x, &y, 2.5, 4.5, 10), &expected);
        assert_points(&index.query(&x, &y, 4.5, 2.5, 10), &expected);
        assert_points(
            &index.query(&x, &y, 2.2, 2.8, 10),
            &[[2.0, 2.0], [3.0, 3.0]],
        );
        assert_points(
            &index.query(&x, &y, 2.0, 2.0, 1),
            &[[1.0, 1.0], [2.0, 2.0], [3.0, 3.0]],
        );
        assert!(index.query(&x, &y, 7.0, 8.0, 10).is_empty());
        assert!(index.query(&x, &y, 0.0, 6.0, 0).is_empty());
        assert!(index.query(&x, &y, f64::NAN, 6.0, 10).is_empty());
    }

    #[test]
    fn raw_query_preserves_duplicate_x_and_inclusive_edges() {
        let x = [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 3.0];
        let y = [0.0, 1.0, 5.0, 3.0, -2.0, 2.0, 6.0, 7.0];
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query_raw(&x, &y, 1.0, 2.0),
            &[
                [0.0, 0.0],
                [1.0, 1.0],
                [1.0, 5.0],
                [1.0, 3.0],
                [1.0, -2.0],
                [1.0, 2.0],
                [2.0, 6.0],
                [3.0, 7.0],
            ],
        );
        assert_points(
            &index.query(&x, &y, 1.0, 2.0, 10),
            &[
                [0.0, 0.0],
                [1.0, 1.0],
                [1.0, 5.0],
                [1.0, -2.0],
                [1.0, 2.0],
                [2.0, 6.0],
                [3.0, 7.0],
            ],
        );
    }

    #[test]
    fn nonfinite_coordinates_break_lines_without_spurious_connections() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, f64::NAN, 6.0, 7.0];
        let y = [0.0, 1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0, 7.0];
        let index = M4Index::new(&x, &y, None);
        let separator = [f64::NAN, f64::NAN];
        let expected = [
            [0.0, 0.0],
            [1.0, 1.0],
            separator,
            [3.0, 3.0],
            [4.0, 4.0],
            separator,
            [6.0, 6.0],
            [7.0, 7.0],
        ];
        assert_points(&index.query(&x, &y, -1.0, 8.0, 1), &expected);
        assert_points(&index.query_raw(&x, &y, -1.0, 8.0), &expected);
        assert!(index.query(&x, &y, 1.5, 2.5, 1).is_empty());
    }

    #[test]
    fn median_interval_gaps_break_lines_only_when_enabled() {
        let x = [0.0, 1.0, 2.0, 3.0, 100.0, 101.0, 102.0];
        let y = [0.0, 1.0, 5.0, 3.0, 0.0, -5.0, 0.0];
        let index = M4Index::new(&x, &y, Some(10.0));
        assert_points(
            &index.query(&x, &y, 0.0, 102.0, 1),
            &[
                [0.0, 0.0],
                [2.0, 5.0],
                [3.0, 3.0],
                [f64::NAN, f64::NAN],
                [100.0, 0.0],
                [101.0, -5.0],
                [102.0, 0.0],
            ],
        );
        assert!(index.query(&x, &y, 10.0, 90.0, 1).is_empty());
        let no_gaps = M4Index::new(&x, &y, None).query_raw(&x, &y, 0.0, 102.0);
        assert_eq!(no_gaps.len(), x.len());
        assert!(no_gaps.iter().all(|point| point[0].is_finite()));
    }

    #[test]
    fn median_includes_duplicate_time_intervals() {
        let x = [0.0, 0.0, 1.0, 1.0, 2.0, 2.0];
        let y = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let index = M4Index::new(&x, &y, Some(10.0));
        assert_points(
            &index.query_raw(&x, &y, 0.0, 2.0),
            &[
                [0.0, 0.0],
                [0.0, 1.0],
                [f64::NAN, f64::NAN],
                [1.0, 2.0],
                [1.0, 3.0],
                [f64::NAN, f64::NAN],
                [2.0, 4.0],
                [2.0, 5.0],
            ],
        );
    }

    #[test]
    fn indexed_extrema_span_many_blocks_and_panned_ranges() {
        let x: Vec<f64> = (0..4096).map(|i| i as f64).collect();
        let mut y = vec![0.0; x.len()];
        y[17] = -5.0;
        y[247] = 20.0;
        y[3130] = -7.0;
        y[3800] = 40.0;
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query(&x, &y, 0.0, 4095.0, 1),
            &[[0.0, 0.0], [3130.0, -7.0], [3800.0, 40.0], [4095.0, 0.0]],
        );
        assert_points(
            &index.query(&x, &y, 200.0, 500.0, 1),
            &[
                [199.0, 0.0],
                [200.0, 0.0],
                [247.0, 20.0],
                [500.0, 0.0],
                [501.0, 0.0],
            ],
        );
    }

    #[test]
    fn unordered_fallback_retains_crossings_and_path_breaks() {
        let x = [0.0, 3.0, 4.0, 5.0, 0.0];
        let y = [0.0, 3.0, 4.0, 5.0, 1.0];
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query(&x, &y, 1.0, 2.0, 1),
            &[
                [0.0, 0.0],
                [3.0, 3.0],
                [f64::NAN, f64::NAN],
                [5.0, 5.0],
                [0.0, 1.0],
            ],
        );
    }

    #[test]
    fn bounds_ignore_unplottable_pairs_and_support_infinite_views() {
        let x = [-2.0, 0.0, 0.5, 2.0, 8.0, f64::INFINITY];
        let y = [1.0, f64::NAN, 1.0, 1.0, f64::INFINITY, 1.0];
        let index = M4Index::new(&x, &y, None);
        assert_eq!(index.x_bounds(), Some([-2.0, 2.0]));
        assert_eq!(index.smallest_positive_x(), Some(0.5));
        assert_points(
            &index.query_raw(&x, &y, f64::NEG_INFINITY, f64::INFINITY),
            &[[-2.0, 1.0], [f64::NAN, f64::NAN], [0.5, 1.0], [2.0, 1.0]],
        );
        assert!(index
            .query(&x, &y, f64::INFINITY, f64::INFINITY, 10)
            .is_empty());
        let empty = M4Index::new(&[f64::NAN], &[1.0], None);
        assert_eq!(empty.x_bounds(), None);
        assert_eq!(empty.smallest_positive_x(), None);
    }

    #[test]
    fn different_paired_lengths_rebuild_safely() {
        let x = [0.0, 1.0, 2.0, 3.0];
        let y = [0.0, 1.0, 2.0, 3.0];
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query(&x, &y[..2], 0.0, 3.0, 1),
            &[[0.0, 0.0], [1.0, 1.0]],
        );
    }

    #[test]
    fn logarithmic_x_uses_decade_columns_and_excludes_nonpositive_points() {
        let x = [
            0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0,
            70.0, 80.0, 90.0, 100.0,
        ];
        let index = M4Index::new(&x, &x, None);
        assert_points(
            &index.query_log(&x, &x, 1.0, 100.0, 2),
            &[[1.0, 1.0], [9.0, 9.0], [10.0, 10.0], [100.0, 100.0]],
        );
        let raw = index.query_raw_log(&x, &x, 0.0, 100.0);
        assert_eq!(raw.len(), x.len() - 1);
        assert!(raw.iter().all(|point| point[0] > 0.0));
    }

    #[test]
    fn logarithmic_x_preserves_breaks_and_positive_neighbors() {
        let x = [0.1, 1.0, 10.0, f64::NAN, 100.0, 1000.0];
        let y = [1.0, 2.0, 3.0, 0.0, 4.0, 5.0];
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query_log(&x, &y, 2.0, 200.0, 2),
            &[
                [1.0, 2.0],
                [10.0, 3.0],
                [f64::NAN, f64::NAN],
                [100.0, 4.0],
                [1000.0, 5.0],
            ],
        );

        let x = [1.0, 2.0, -1.0, 3.0, 4.0];
        let y = [1.0, 2.0, 3.0, 4.0, 5.0];
        let index = M4Index::new(&x, &y, None);
        assert_points(
            &index.query_log(&x, &y, 1.0, 4.0, 1),
            &[
                [1.0, 1.0],
                [2.0, 2.0],
                [f64::NAN, f64::NAN],
                [3.0, 4.0],
                [4.0, 5.0],
            ],
        );
    }

    #[test]
    fn logarithmic_y_splits_before_m4_for_both_x_scales() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let y = [1.0, 2.0, -1.0, 3.0, 0.0, 4.0, 1.0, 9.0, 6.0];
        let index = M4Index::new(&x, &y, None);
        let expected = [
            [1.0, 1.0],
            [2.0, 2.0],
            [f64::NAN, f64::NAN],
            [4.0, 3.0],
            [f64::NAN, f64::NAN],
            [6.0, 4.0],
            [7.0, 1.0],
            [8.0, 9.0],
            [9.0, 6.0],
        ];
        assert_points(&index.query_log_y(&x, &y, 1.0, 9.0, 1, false), &expected);
        assert_points(&index.query_log_y(&x, &y, 1.0, 9.0, 1, true), &expected);
        assert_points(&index.query_raw_log_y(&x, &y, 1.0, 9.0, true), &expected);
        assert!(index.query_log_y(&x, &y, 2.4, 3.6, 1, false).is_empty());
    }

    #[test]
    fn positive_y_range_uses_index_and_ignores_invalid_samples() {
        let mut x: Vec<f64> = (0..4096).map(|i| i as f64).collect();
        let mut y = vec![-3.0; x.len()];
        y[16] = 0.25;
        y[247] = 10.0;
        y[255] = f64::INFINITY;
        x[256] = f64::NAN;
        y[256] = 0.000_001;
        y[3130] = 0.001;
        y[3800] = 500.0;
        let index = M4Index::new(&x, &y, None);
        assert_eq!(
            index.positive_y_bounds(&x, &y, 0.0, 4095.0),
            Some([0.001, 500.0])
        );
        assert_eq!(
            index.positive_y_bounds(&x, &y, 500.0, 100.0),
            Some([10.0, 10.0])
        );
        assert_eq!(index.positive_y_bounds(&x, &y, 248.0, 3129.0), None);
        assert_eq!(
            index.positive_y_bounds(&x, &y, 247.0, 247.0),
            Some([10.0, 10.0])
        );

        let x = [4.0, 1.0, 3.0, 2.0];
        let y = [3.0, 4.0, -2.0, 0.1];
        let index = M4Index::new(&x, &y, None);
        assert_eq!(index.positive_y_bounds(&x, &y, 1.5, 4.0), Some([0.1, 3.0]));
    }

    #[test]
    fn finite_y_bounds_cover_indexed_ranges_and_unordered_data() {
        let x: Vec<f64> = (0..128).map(|i| i as f64).collect();
        let mut y = vec![0.0; x.len()];
        y[12] = -5.0;
        y[77] = 9.0;
        y[88] = -100.0;
        y[90] = f64::NAN;
        y[100] = 7.0;
        let index = M4Index::new(&x, &y, None);
        assert_eq!(index.y_bounds(&x, &y, 0.0, 127.0), Some([-100.0, 9.0]));
        assert_eq!(index.y_bounds(&x, &y, 80.0, 10.0), Some([-5.0, 9.0]));
        assert_eq!(index.y_bounds(&x, &y, 89.0, 89.0), Some([0.0, 0.0]));
        assert_eq!(index.y_bounds(&x, &y, 90.0, 90.0), None);

        let x = [3.0, 1.0, 2.0, 4.0];
        let y = [100.0, -10.0, 5.0, 20.0];
        let index = M4Index::new(&x, &y, None);
        assert_eq!(index.y_bounds(&x, &y, 1.0, 2.0), Some([-10.0, 5.0]));
        assert_eq!(index.y_bounds(&x, &y, 2.5, 2.5), None);
    }
}
