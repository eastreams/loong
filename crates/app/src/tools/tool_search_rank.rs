use std::collections::BTreeSet;

use super::query_support::{SearchQuery, SearchSignalSet, ordered_overlap};
use super::{RankedSearchableToolEntry, SearchableToolEntry, ToolSearchRanking};

const COARSE_FALLBACK_DISCOVERY_CONCEPTS: &[&str] =
    &["fetch", "inspect", "list", "read", "search", "status"];
const MAX_SEARCH_WHY_REASONS: usize = 4;

#[derive(Debug, Clone)]
struct SearchScore {
    score: u32,
    why: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScoredSearchableToolEntry {
    entry: SearchableToolEntry,
    score: u32,
    why: Vec<String>,
}

pub(crate) fn rank_searchable_entries(
    entries: Vec<SearchableToolEntry>,
    query: &str,
    limit: usize,
) -> ToolSearchRanking {
    if entries.is_empty() {
        return ToolSearchRanking {
            results: Vec::new(),
            diagnostics_reason: Some("no_visible_tools"),
        };
    }

    let search_query = SearchQuery::new(query);
    let mut ranked = Vec::new();

    for entry in &entries {
        let score = score_entry(entry, &search_query);
        let Some(score) = score else {
            continue;
        };

        ranked.push(ScoredSearchableToolEntry {
            entry: entry.clone(),
            score: score.score,
            why: score.why,
        });
    }

    sort_scored_entries(&mut ranked);

    if !ranked.is_empty() {
        let results = ranked
            .into_iter()
            .take(limit)
            .map(|entry| RankedSearchableToolEntry {
                entry: entry.entry,
                why: entry.why,
            })
            .collect();

        return ToolSearchRanking {
            results,
            diagnostics_reason: None,
        };
    }

    coarse_fallback(entries, limit)
}

fn score_entry(entry: &SearchableToolEntry, query: &SearchQuery) -> Option<SearchScore> {
    let mut score = 0u32;
    let mut why = BTreeSet::new();

    let normalized_query = query.signal.normalized_text.as_str();
    let query_tokens = &query.signal.tokens;

    let _name_phrase_hit = add_phrase_score(
        "name",
        64,
        &entry.search_document.name,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _summary_phrase_hit = add_phrase_score(
        "summary",
        42,
        &entry.search_document.summary,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _argument_phrase_hit = add_phrase_score(
        "argument",
        30,
        &entry.search_document.arguments,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _schema_phrase_hit = add_phrase_score(
        "schema",
        28,
        &entry.search_document.schema,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _tag_phrase_hit = add_phrase_score(
        "tag",
        24,
        &entry.search_document.tags,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _name_token_hit = add_token_scores(
        "name",
        20,
        &entry.search_document.name,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _summary_token_hit = add_token_scores(
        "summary",
        12,
        &entry.search_document.summary,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _argument_token_hit = add_token_scores(
        "argument",
        10,
        &entry.search_document.arguments,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _schema_token_hit = add_token_scores(
        "schema",
        9,
        &entry.search_document.schema,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _tag_token_hit = add_token_scores(
        "tag",
        14,
        &entry.search_document.tags,
        query_tokens,
        &mut score,
        &mut why,
    );

    let concept_overlap = ordered_overlap(&query.concepts, &entry.search_document.concepts);
    for concept in concept_overlap {
        score += 26;
        why.insert(format!("concept:{concept}"));
    }

    let category_overlap = ordered_overlap(&query.categories, &entry.search_document.categories);
    for category in category_overlap {
        score += 12;
        why.insert(format!("category:{category}"));
    }

    if score == 0 {
        return None;
    }

    let mut why = why.into_iter().collect::<Vec<_>>();
    why.truncate(MAX_SEARCH_WHY_REASONS);

    Some(SearchScore { score, why })
}

fn add_phrase_score(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    normalized_query: &str,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let phrase_allowed = phrase_search_allowed(normalized_query);
    if !phrase_allowed {
        return false;
    }

    let contains_query = signal.normalized_text.contains(normalized_query);
    if !contains_query {
        return false;
    }

    *score += weight;
    why.insert(format!("{label}_phrase"));
    true
}

fn add_token_scores(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    query_tokens: &BTreeSet<String>,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let overlaps = ordered_overlap(query_tokens, &signal.tokens);
    if overlaps.is_empty() {
        return false;
    }

    for token in overlaps {
        *score += weight;
        why.insert(format!("{label}:{token}"));
    }

    true
}

fn phrase_search_allowed(normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return false;
    }

    let character_count = normalized_query.chars().count();
    if normalized_query.is_ascii() {
        return character_count >= 2;
    }

    character_count >= 1
}

fn coarse_fallback(entries: Vec<SearchableToolEntry>, limit: usize) -> ToolSearchRanking {
    let mut ranked = Vec::new();

    for entry in entries {
        let (score, why) = coarse_fallback_score(&entry);
        ranked.push(ScoredSearchableToolEntry { entry, score, why });
    }

    sort_scored_entries(&mut ranked);

    let results = ranked
        .into_iter()
        .take(limit)
        .map(|entry| RankedSearchableToolEntry {
            entry: entry.entry,
            why: entry.why,
        })
        .collect();

    ToolSearchRanking {
        results,
        diagnostics_reason: Some("coarse_fallback"),
    }
}

fn coarse_fallback_score(entry: &SearchableToolEntry) -> (u32, Vec<String>) {
    let mut score = 1u32;
    let mut why = BTreeSet::new();

    why.insert("coarse_fallback".to_owned());

    let mut discovery_bonus = 0u32;
    for concept in COARSE_FALLBACK_DISCOVERY_CONCEPTS {
        if entry.search_document.concepts.contains(*concept) {
            discovery_bonus += 1;
        }
    }

    if discovery_bonus > 0 {
        score += 40u32 + discovery_bonus * 6u32;
        why.insert("coarse_discovery_tool".to_owned());
    }

    score += entry.search_document.categories.len() as u32;
    score += entry.search_document.concepts.len() as u32;

    (score, why.into_iter().collect::<Vec<_>>())
}

fn sort_scored_entries(entries: &mut [ScoredSearchableToolEntry]) {
    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.entry.canonical_name.cmp(&right.entry.canonical_name))
    });
}
