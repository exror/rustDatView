use super::*;

#[derive(Default)]
pub(super) struct SearchCache {
    identity: Option<u64>,
    table_name: String,
    active_name: String,
    names: Vec<ChannelName>,
    query: String,
    query_valid: bool,
    alternatives: Vec<Vec<String>>,
    ranked: Vec<(MatchRank, usize)>,
}

struct ChannelName {
    original: String,
    folded: String,
    numeric: bool,
    index: bool,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MatchRank {
    kind: u8,
    gaps: usize,
    start: usize,
}

// Unicode lowercase expands characters when needed. Also fold the common
// non-lowercase case equivalents that matter for searching telemetry names.
fn fold_case(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars().flat_map(char::to_lowercase) {
        match character {
            'ß' => folded.push_str("ss"),
            'ς' => folded.push('σ'),
            _ => folded.push(character),
        }
    }
    folded
}

fn token_rank(name: &str, token: &str) -> Option<MatchRank> {
    if name == token {
        return Some(MatchRank::default());
    }
    if let Some(start) = name.find(token) {
        return Some(MatchRank {
            kind: 1,
            gaps: 0,
            start: name[..start].chars().count(),
        });
    }
    let mut characters = name.chars().enumerate();
    let mut first = None;
    let mut last = 0;
    let mut count = 0;
    for wanted in token.chars() {
        let (position, _) = characters.find(|(_, character)| *character == wanted)?;
        first.get_or_insert(position);
        last = position;
        count += 1;
    }
    let start = first?;
    Some(MatchRank {
        kind: 2,
        gaps: last - start + 1 - count,
        start,
    })
}

fn alternative_rank(name: &str, tokens: &[String]) -> Option<MatchRank> {
    if name
        .split_whitespace()
        .eq(tokens.iter().map(String::as_str))
    {
        return Some(MatchRank::default());
    }
    let mut rank = MatchRank::default();
    for token in tokens {
        let matched = token_rank(name, token)?;
        rank.kind = rank.kind.max(matched.kind);
        rank.gaps += matched.gaps;
        rank.start += matched.start;
    }
    Some(rank)
}

impl SearchCache {
    fn refresh_names(&mut self, table: &Table) -> bool {
        let columns = table.data.columns();
        if self.identity == Some(table.identity())
            && self.table_name == table.raw_name()
            && self.active_name == table.active_name
            && self.names.len() == columns.len()
            && self.names.iter().zip(columns).all(|(cached, column)| {
                cached.original == column.name
                    && cached.numeric == (column.is_float() || column.is_datetime())
            })
        {
            return false;
        }
        self.identity = Some(table.identity());
        self.table_name.clear();
        self.table_name.push_str(table.raw_name());
        self.active_name.clone_from(&table.active_name);
        self.names = columns
            .iter()
            .map(|column| ChannelName {
                original: column.name.clone(),
                folded: fold_case(&column.name),
                numeric: column.is_float() || column.is_datetime(),
                index: selection::is_index(&column.name),
            })
            .collect();
        self.query_valid = false;
        true
    }

    fn search(&mut self, query: &str) {
        if self.query_valid && self.query == query {
            return;
        }
        self.query.clear();
        self.query.push_str(query);
        self.alternatives = query
            .split('|')
            .map(|alternative| {
                alternative
                    .split_whitespace()
                    .map(fold_case)
                    .collect::<Vec<_>>()
            })
            .filter(|tokens| !tokens.is_empty())
            .collect();
        self.ranked.clear();
        for (index, name) in self.names.iter().enumerate() {
            let rank = if self.alternatives.is_empty() {
                Some(MatchRank::default())
            } else {
                self.alternatives
                    .iter()
                    .filter_map(|tokens| alternative_rank(&name.folded, tokens))
                    .min()
            };
            if let Some(rank) = rank {
                self.ranked.push((rank, index));
            }
        }
        // The original column index explicitly breaks equal-score ties.
        self.ranked.sort_unstable();
        self.query_valid = true;
    }

    fn allowed(&self, index: usize, common: Option<&[usize]>) -> bool {
        self.names[index].numeric && common.is_none_or(|indices| indices.contains(&index))
    }

    fn select_matches(&self, selection: &mut Selection, common: Option<&[usize]>) -> bool {
        let before = selection.ys.len();
        for &(_, index) in &self.ranked {
            if self.allowed(index, common)
                && index != selection.x
                && !self.names[index].index
                && !selection.ys.contains(&index)
            {
                selection.ys.push(index);
            }
        }
        if before != selection.ys.len() {
            selection.ys.sort_unstable();
            true
        } else {
            false
        }
    }

    fn deselect_matches(&self, selection: &mut Selection, common: Option<&[usize]>) -> bool {
        let before = selection.ys.len();
        selection.ys.retain(|index| {
            !self
                .ranked
                .iter()
                .any(|(_, matched)| matched == index && self.allowed(*matched, common))
        });
        before != selection.ys.len()
    }
}

impl DatView {
    pub(super) fn channel_picker(
        &mut self,
        ui: &mut egui::Ui,
        table_index: usize,
        common: Option<&[usize]>,
    ) {
        let table = &self.tables.tabs[table_index];
        let Some(selection) = self.selections.get_mut(&table_index) else {
            return;
        };
        let cache = self.channel_search.entry(table_index).or_default();
        cache.refresh_names(table);
        let dirty = &mut self.dirty;
        ui.push_id(table_index, |ui| {
            egui::CollapsingHeader::new(table.active_name.as_str())
                .default_open(true)
                .show(ui, |ui| {
                    egui::ComboBox::from_id_salt("x")
                        .selected_text(format!(
                            "X: {}",
                            cache.names.get(selection.x).map(|name| name.original.as_str()).unwrap_or("—")
                        ))
                        .show_ui(ui, |ui| {
                            for (index, name) in cache.names.iter().enumerate() {
                                if cache.allowed(index, common)
                                    && ui.selectable_value(&mut selection.x, index, &name.original).changed()
                                {
                                    *dirty = true;
                                }
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut selection.filter)
                            .hint_text("Filter channels: cpu power | vcore")
                            .desired_width(f32::INFINITY),
                    ).on_hover_text("Case-insensitive fuzzy search. Spaces require every token; | separates alternatives. Hidden selections are preserved.");
                    ui.horizontal_wrapped(|ui| {
                        for (label, query) in [
                            ("Power rails", "power | vcore | voltage | vrm | rail | +12 | +5 | +3.3"),
                            ("Clocks", "clock | frequency | mhz | ghz"),
                            ("Temperatures", "temperature | temp | thermal | °c"),
                        ] {
                            if ui.small_button(label).clicked() {
                                selection.filter.clear();
                                selection.filter.push_str(query);
                            }
                        }
                    });
                    cache.search(&selection.filter);
                    let matches = cache.ranked.iter().filter(|(_, index)| cache.allowed(*index, common)).count();
                    let matching_selected = cache.ranked.iter().filter(|(_, index)| {
                        cache.allowed(*index, common) && selection.ys.contains(index)
                    }).count();
                    ui.weak(format!("{matches} matches · {} selected ({matching_selected} matching)", selection.ys.len()));
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Select matches").on_hover_text("Add matching Y channels without changing hidden selections. X and Index are excluded.").clicked() {
                            *dirty |= cache.select_matches(selection, common);
                        }
                        if ui.button("Deselect matches").clicked() {
                            *dirty |= cache.deselect_matches(selection, common);
                        }
                        if ui.button("Clear all").on_hover_text("Clear every Y selection, including channels hidden by this search.").clicked() {
                            *dirty |= !selection.ys.is_empty();
                            selection.ys.clear();
                        }
                    });
                    if matches == 0 {
                        ui.weak("No matching channels. Try fewer tokens or a different preset.");
                    }
                    for &(_, index) in &cache.ranked {
                        if !cache.allowed(index, common) {
                            continue;
                        }
                        let mut selected = selection.ys.contains(&index);
                        if ui.checkbox(&mut selected, &cache.names[index].original).changed() {
                            if selected {
                                selection.ys.push(index);
                                selection.ys.sort_unstable();
                            } else {
                                selection.ys.retain(|column| *column != index);
                            }
                            *dirty = true;
                        }
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::DataFrame;

    fn table(names: &[&str]) -> Table {
        Table::from_df(
            DataFrame::from_floats(names.iter().copied(), vec![vec![1., 2.]; names.len()]).unwrap(),
            "telemetry",
        )
    }

    fn matches(cache: &SearchCache) -> Vec<&str> {
        cache
            .ranked
            .iter()
            .map(|(_, index)| cache.names[*index].original.as_str())
            .collect()
    }

    #[test]
    fn ranks_exact_then_substring_then_ordered_subsequence() {
        let mut cache = SearchCache::default();
        cache.refresh_names(&table(&[
            "Core Power Usage",
            "CPU Package",
            "CPU",
            "CPU Core",
            "Power CPU",
        ]));
        cache.search("cpu");
        assert_eq!(
            matches(&cache),
            [
                "CPU",
                "CPU Package",
                "CPU Core",
                "Power CPU",
                "Core Power Usage"
            ]
        );
        assert!(token_rank("cpu", "upc").is_none());
    }

    #[test]
    fn complete_multi_token_names_rank_before_longer_substrings() {
        let mut cache = SearchCache::default();
        cache.refresh_names(&table(&[
            "CPU Power Extra",
            "CPU Power",
            "Core Package Watts",
        ]));
        cache.search("cpu power");
        assert_eq!(matches(&cache), ["CPU Power", "CPU Power Extra"]);
    }

    #[test]
    fn tokens_are_and_with_or_alternatives_and_unicode_case() {
        let mut cache = SearchCache::default();
        cache.refresh_names(&table(&[
            "CPU Package Power",
            "CPU Clock",
            "GPU Power",
            "TÉMPÉRATURE",
            "Straße",
            "ΟΣ",
        ]));
        cache.search(" CPU   POWER | témp ");
        assert_eq!(matches(&cache), ["TÉMPÉRATURE", "CPU Package Power"]);
        cache.search("STRASSE | οσ");
        assert_eq!(matches(&cache), ["Straße", "ΟΣ"]);
        cache.search(" |  ");
        assert_eq!(cache.ranked.len(), cache.names.len());
    }

    #[test]
    fn cache_refreshes_for_renames_identity_and_columns() {
        let mut source = table(&["CPU Power", "Clock"]);
        let mut cache = SearchCache::default();
        assert!(cache.refresh_names(&source));
        cache.search("power");
        assert!(!cache.refresh_names(&source));
        let allocation = cache.names[1].folded.as_ptr();
        let rankings = cache.ranked.as_ptr();
        cache.search("power");
        assert_eq!(allocation, cache.names[1].folded.as_ptr());
        assert_eq!(rankings, cache.ranked.as_ptr());
        source.data.column_mut(1).name = "Temperature".into();
        assert!(cache.refresh_names(&source));
        cache.search("power");
        assert!(matches(&cache).is_empty());
        source.set_raw_name("renamed");
        assert!(cache.refresh_names(&source));
        let replacement = table(&["Temperature", "Clock"]);
        assert!(cache.refresh_names(&replacement));
        source.data.remove(2);
        assert!(cache.refresh_names(&source));
        cache.search("clock");
        assert!(matches(&cache).is_empty());
    }

    #[test]
    fn bulk_selection_is_additive_filtered_and_excludes_x_and_index() {
        let source = table(&["Time [s]", "CPU Power", "GPU Power", "Temperature"]);
        let mut cache = SearchCache::default();
        cache.refresh_names(&source);
        let mut selection = Selection {
            x: 1,
            ys: vec![4],
            ..Default::default()
        };
        cache.search("power");
        assert!(cache.select_matches(&mut selection, Some(&[0, 1, 2, 4])));
        assert_eq!(selection.ys, [2, 4]);
        assert!(!cache.select_matches(&mut selection, Some(&[0, 1, 2, 4])));
        assert!(cache.deselect_matches(&mut selection, None));
        assert_eq!(selection.ys, [4]);
        cache.search("");
        cache.select_matches(&mut selection, None);
        assert_eq!(selection.ys, [2, 3, 4]);
        cache.search("power");
        cache.deselect_matches(&mut selection, Some(&[2]));
        assert_eq!(selection.ys, [3, 4]);
    }
}
