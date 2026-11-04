use dv_core::{
    common::clean_col,
    units::{parse_unit, split_name_unit},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Channel {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub family: String,
    pub original: String,
}

pub(crate) fn catalog(tables: &dv_core::TableList) -> Vec<Channel> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for table in &tables.tabs {
        for column in table.data.columns() {
            if !(column.is_float() || column.is_datetime()) {
                continue;
            }
            let key = clean_col(&column.name);
            if key.is_empty() || !seen.insert(key.clone()) {
                continue;
            }
            let (label, unit) = split_name_unit(&column.name);
            let family = if column.is_datetime() {
                dv_core::units::UnitFamily::Time.to_string()
            } else {
                parse_unit(&unit).family.to_string()
            };
            result.push(Channel {
                key,
                label,
                unit,
                family,
                original: column.name.clone(),
            });
        }
    }
    result
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MatchRank {
    kind: u8,
    gaps: usize,
    start: usize,
}

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
    let mut chars = name.chars().enumerate();
    let mut first = None;
    let mut last = 0;
    let mut count = 0;
    for wanted in token.chars() {
        let (position, _) = chars.find(|(_, c)| *c == wanted)?;
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

#[derive(Default)]
pub(crate) struct SearchCache {
    names: Vec<String>,
    query: Option<String>,
    ranked: Vec<usize>,
}
impl SearchCache {
    pub(crate) fn reset(&mut self, channels: &[Channel]) {
        self.names = channels.iter().map(|c| fold_case(&c.original)).collect();
        self.query = None;
        self.ranked.clear();
    }
    pub(crate) fn search(&mut self, query: &str) -> Vec<usize> {
        if self.query.as_deref() == Some(query) {
            return self.ranked.clone();
        }
        let alternatives: Vec<Vec<String>> = query
            .split('|')
            .map(|part| part.split_whitespace().map(fold_case).collect())
            .filter(|part: &Vec<String>| !part.is_empty())
            .collect();
        let mut ranked: Vec<_> = self
            .names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                let rank = if alternatives.is_empty() {
                    Some(MatchRank::default())
                } else {
                    alternatives
                        .iter()
                        .filter_map(|tokens| alternative_rank(name, tokens))
                        .min()
                };
                rank.map(|rank| (rank, index))
            })
            .collect();
        ranked.sort_unstable();
        self.ranked = ranked.into_iter().map(|(_, i)| i).collect();
        self.query = Some(query.to_owned());
        self.ranked.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cache(names: &[&str]) -> SearchCache {
        let channels: Vec<_> = names
            .iter()
            .map(|s| Channel {
                key: clean_col(s),
                label: s.to_string(),
                unit: String::new(),
                family: String::new(),
                original: s.to_string(),
            })
            .collect();
        let mut cache = SearchCache::default();
        cache.reset(&channels);
        cache
    }
    #[test]
    fn exact_substring_subsequence_and_original_order() {
        let mut c = cache(&[
            "Core Power Usage",
            "CPU Package",
            "CPU",
            "CPU Core",
            "Power CPU",
        ]);
        assert_eq!(c.search("cpu"), vec![2, 1, 3, 4, 0]);
        assert!(token_rank("cpu", "upc").is_none());
    }
    #[test]
    fn complete_multitoken_and_unicode_or_queries() {
        let mut c = cache(&[
            "CPU Power Extra",
            "CPU Power",
            "CPU Package Power",
            "TÉMPÉRATURE",
            "Straße",
            "ΟΣ",
        ]);
        assert_eq!(c.search("cpu power"), vec![1, 0, 2]);
        assert_eq!(c.search(" CPU   POWER | témp "), vec![1, 3, 0, 2]);
        assert_eq!(c.search("STRASSE | οσ"), vec![4, 5]);
        assert_eq!(c.search(" | ").len(), 6);
    }
    #[test]
    fn resetting_catalog_invalidates_the_cached_query() {
        let mut c = cache(&["CPU Power", "Clock"]);
        assert_eq!(c.search("power"), vec![0]);
        c.reset(&[Channel {
            key: "clock".into(),
            label: "Clock".into(),
            unit: "MHz".into(),
            family: "Frequency".into(),
            original: "Clock [MHz]".into(),
        }]);
        assert!(c.search("power").is_empty());
        assert_eq!(c.search("mhz"), vec![0]);
    }
}
