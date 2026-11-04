use crate::{
    series::SeriesData,
    workspace::{Mode, Source},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PaneGrouping {
    #[default]
    Unit,
    Single,
    Channel,
    Custom,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ColorBy {
    #[default]
    Auto,
    Channel,
    Source,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LineDraw {
    #[default]
    Lines,
    Points,
    LinesPoints,
    Steps,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SeriesStyle {
    pub color: Option<[u8; 3]>,
    pub width: Option<f32>,
    /// 0 = solid, 1 = dashed, 2 = dotted, 3 = dash-dot.
    pub dash: Option<usize>,
    pub draw: Option<LineDraw>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PaneSettings {
    pub grouping: PaneGrouping,
    pub color_by: ColorBy,
    pub custom: BTreeMap<String, String>,
    pub colors: BTreeMap<String, [u8; 3]>,
    pub styles: BTreeMap<String, SeriesStyle>,
    pub slots: BTreeMap<String, usize>,
    pub source_slots: BTreeMap<String, usize>,
}
impl PaneSettings {
    /// A channel keeps its color once assigned. New channels take the first
    /// palette slot not used by the channels plotted with them.
    fn assign_channel_slots<'a>(&mut self, keys: impl Iterator<Item = &'a str> + Clone) {
        const PALETTE: usize = 10;
        let mut used: BTreeSet<usize> = keys
            .clone()
            .filter_map(|key| self.slots.get(key))
            .map(|slot| slot % PALETTE)
            .collect();
        for key in keys {
            if self.slots.contains_key(key) {
                continue;
            }
            let slot = (0..PALETTE)
                .find(|slot| !used.contains(slot))
                .unwrap_or(self.slots.len() % PALETTE);
            used.insert(slot);
            self.slots.insert(key.to_owned(), slot);
        }
    }
    fn channel_slot(&self, key: &str) -> usize {
        self.slots.get(key).copied().unwrap_or(0)
    }
    fn source_slot(&mut self, source: &Source) -> usize {
        let next = self
            .source_slots
            .values()
            .copied()
            .max()
            .map_or(0, |n| n + 1);
        *self.source_slots.entry(source.stable_key()).or_insert(next)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Annotation {
    Point {
        mode: Mode,
        curve: String,
        x: f64,
        y: f64,
        label: String,
        color: [u8; 3],
    },
    Region {
        mode: Mode,
        x_axis: String,
        min: f64,
        max: f64,
        label: String,
        color: [u8; 3],
    },
}
impl Annotation {
    pub(crate) fn mode(&self) -> Mode {
        match self {
            Self::Point { mode, .. } | Self::Region { mode, .. } => *mode,
        }
    }
    pub(crate) fn region(
        mode: Mode,
        x_axis: String,
        a: f64,
        b: f64,
        label: String,
        color: [u8; 3],
    ) -> Option<Self> {
        (a.is_finite() && b.is_finite() && a != b).then(|| Self::Region {
            mode,
            x_axis,
            min: a.min(b),
            max: a.max(b),
            label,
            color,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FilterMode {
    #[default]
    Keep,
    Exclude,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RangeRule {
    pub enabled: bool,
    pub mode: Mode,
    /// Empty matches every curve; otherwise the channel key or curve label.
    pub target: String,
    /// X min, X max, Y min, Y max, all inclusive.
    pub bounds: [Option<f64>; 4],
    pub action: FilterMode,
}
impl RangeRule {
    pub(crate) fn matches(&self, mode: Mode, key: &str, label: &str) -> bool {
        self.enabled
            && self.mode == mode
            && (self.target.is_empty() || self.target == key || self.target == label)
    }
    pub(crate) fn accepts(&self, x: f64, y: f64) -> bool {
        let [xmin, xmax, ymin, ymax] = self.bounds;
        let inside = x.is_finite()
            && y.is_finite()
            && xmin.is_none_or(|v| x >= v)
            && xmax.is_none_or(|v| x <= v)
            && ymin.is_none_or(|v| y >= v)
            && ymax.is_none_or(|v| y <= v);
        match self.action {
            FilterMode::Keep => inside,
            FilterMode::Exclude => !inside,
        }
    }
}

pub(crate) fn arrange(series: &mut [SeriesData], sources: &[Source], settings: &mut PaneSettings) {
    for s in series.iter_mut() {
        s.pane = match settings.grouping {
            PaneGrouping::Single => "Signals".into(),
            PaneGrouping::Channel => unit_pane(&s.channel_label, &s.unit),
            PaneGrouping::Custom => settings
                .custom
                .get(&s.key)
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| unit_pane(&s.family, &s.unit)),
            PaneGrouping::Unit => unit_pane(&s.family, &s.unit),
        };
    }
    let mut keys_by_pane: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for s in series.iter() {
        keys_by_pane.entry(&s.pane).or_default().insert(&s.key);
    }
    let source_slots: Vec<_> = sources
        .iter()
        .map(|source| settings.source_slot(source))
        .collect();
    settings.assign_channel_slots(series.iter().map(|s| s.key.as_str()));
    // Compute before mutating series so the pane map need not borrow its strings.
    let single: BTreeMap<String, bool> = keys_by_pane
        .into_iter()
        .map(|(pane, keys)| (pane.to_owned(), keys.len() == 1))
        .collect();
    for s in series.iter_mut() {
        let source_slot = source_slots.get(s.source).copied().unwrap_or(0);
        let by_source = match settings.color_by {
            ColorBy::Source => true,
            ColorBy::Channel => false,
            ColorBy::Auto => {
                sources.iter().filter(|source| source.active).count() > 1
                    && single.get(&s.pane).copied().unwrap_or(false)
            }
        };
        s.color_slot = if by_source {
            source_slot
        } else {
            settings.channel_slot(&s.key)
        };
        s.dash = settings
            .styles
            .get(&s.key)
            .and_then(|style| style.dash)
            .unwrap_or(if by_source { 0 } else { source_slot % 4 });
    }
}
fn unit_pane(family: &str, unit: &str) -> String {
    if unit.is_empty() {
        family.to_owned()
    } else {
        format!("{family} · {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rules_are_inclusive_non_destructive_and_targeted() {
        let mut rule = RangeRule {
            enabled: true,
            mode: Mode::Signals,
            target: "soc".into(),
            bounds: [Some(1.), Some(3.), None, Some(10.)],
            action: FilterMode::Keep,
        };
        assert!(rule.matches(Mode::Signals, "soc", "SOC"));
        assert!(!rule.matches(Mode::Spectrum, "soc", "SOC"));
        assert!(rule.accepts(1., 10.));
        assert!(!rule.accepts(2., 11.));
        rule.action = FilterMode::Exclude;
        assert!(!rule.accepts(3., 8.));
        assert!(rule.accepts(4., 9.));
    }
    #[test]
    fn reverse_region_is_normalized() {
        assert!(matches!(
            Annotation::region(
                Mode::Signals,
                "time".into(),
                8.,
                2.,
                String::new(),
                [1, 2, 3]
            ),
            Some(Annotation::Region {
                min: 2.,
                max: 8.,
                ..
            })
        ));
        assert!(Annotation::region(
            Mode::Signals,
            "time".into(),
            2.,
            2.,
            String::new(),
            [1, 2, 3]
        )
        .is_none());
    }
}
