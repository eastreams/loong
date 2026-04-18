use std::collections::BTreeSet;
use std::sync::OnceLock;

use jieba_rs::Jieba;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn normalize_search_text(raw: &str) -> String {
    let compatibility = raw.nfkc().collect::<String>();
    let lowercased = compatibility.to_lowercase();

    let mut normalized = String::new();
    let mut last_was_space = false;

    for character in lowercased.nfd() {
        if is_combining_mark(character) {
            continue;
        }

        let normalized_character = if character.is_whitespace() {
            ' '
        } else {
            character
        };
        if normalized_character == ' ' {
            if last_was_space {
                continue;
            }

            last_was_space = true;
            normalized.push(' ');
            continue;
        }

        last_was_space = false;
        normalized.push(normalized_character);
    }

    normalized.trim().to_owned()
}

#[cfg(test)]
pub(crate) fn tokenize_search_text(raw: &str) -> Vec<String> {
    let normalized = normalize_search_text(raw);
    tokenize_normalized_search_text(normalized.as_str())
}

pub(crate) fn tokenize_normalized_search_text(normalized: &str) -> Vec<String> {
    let surface = identifier_phrase_variant(normalized);
    let mut tokens = Vec::new();
    let mut seen = BTreeSet::new();

    for word in UnicodeSegmentation::unicode_words(surface.as_str()) {
        push_search_token(word, &mut tokens, &mut seen);
    }

    for han_sequence in extract_han_sequences(surface.as_str()) {
        for token in jieba().cut_for_search(han_sequence.as_str(), true) {
            push_search_token(token, &mut tokens, &mut seen);
        }
    }

    tokens
}

pub(crate) fn build_search_fts_query(raw_query: &str, max_terms: usize) -> Option<String> {
    let normalized_query = normalize_search_text(raw_query);
    let trimmed_query = normalized_query.trim();
    if trimmed_query.is_empty() {
        return None;
    }

    let mut terms = Vec::new();
    let mut seen_terms = BTreeSet::new();

    push_quoted_fts_term(trimmed_query, &mut terms, &mut seen_terms);
    for token in tokenize_normalized_search_text(trimmed_query)
        .into_iter()
        .take(max_terms)
    {
        push_quoted_fts_term(token.as_str(), &mut terms, &mut seen_terms);
    }

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

pub(crate) fn build_search_index_text(fragments: &[&str]) -> String {
    let mut tokens = Vec::new();
    let mut seen = BTreeSet::new();

    for fragment in fragments {
        let normalized = normalize_search_text(fragment);
        if normalized.is_empty() {
            continue;
        }

        for token in tokenize_normalized_search_text(normalized.as_str()) {
            if seen.insert(token.clone()) {
                tokens.push(token);
            }
        }
    }

    tokens.join(" ")
}

fn push_search_token(token: &str, tokens: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return;
    }

    let normalized = normalize_search_text(trimmed);
    if !should_keep_token(normalized.as_str()) {
        return;
    }

    if seen.insert(normalized.clone()) {
        tokens.push(normalized);
    }
}

fn push_quoted_fts_term(term: &str, terms: &mut Vec<String>, seen_terms: &mut BTreeSet<String>) {
    if term.is_empty() {
        return;
    }

    let escaped_term = term.replace('"', "\"\"");
    let quoted_term = format!("\"{escaped_term}\"");
    if seen_terms.insert(quoted_term.clone()) {
        terms.push(quoted_term);
    }
}

fn should_keep_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }

    let ascii_token = token
        .chars()
        .all(|character| character.is_ascii_alphanumeric());
    if ascii_token {
        return token.len() >= 2;
    }

    token.chars().count() >= 2
}

fn identifier_phrase_variant(raw: &str) -> String {
    let mut surface = String::with_capacity(raw.len());
    let mut last_was_space = false;

    for character in raw.chars() {
        let normalized_character = if is_identifier_separator(character) {
            ' '
        } else {
            character
        };
        if normalized_character.is_whitespace() {
            if last_was_space {
                continue;
            }

            last_was_space = true;
            surface.push(' ');
            continue;
        }

        last_was_space = false;
        surface.push(normalized_character);
    }

    surface.trim().to_owned()
}

fn is_identifier_separator(character: char) -> bool {
    matches!(
        character,
        '.' | '_' | '-' | '/' | '\\' | ':' | ',' | ';' | '|' | '(' | ')' | '[' | ']'
    )
}

fn extract_han_sequences(raw: &str) -> Vec<String> {
    let mut sequences = Vec::new();
    let mut current = String::new();

    for character in raw.chars() {
        if is_han_character(character) {
            current.push(character);
            continue;
        }

        if !current.is_empty() {
            sequences.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        sequences.push(current);
    }

    sequences
}

fn is_han_character(character: char) -> bool {
    matches!(
        u32::from(character),
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

fn jieba() -> &'static Jieba {
    static JIEBA: OnceLock<Jieba> = OnceLock::new();
    JIEBA.get_or_init(Jieba::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_search_text_folds_fullwidth_and_combining_marks() {
        let normalized = normalize_search_text("Ｂúsqueda　中文分词");
        assert_eq!(normalized, "busqueda 中文分词");
    }

    #[test]
    fn tokenize_search_text_uses_jieba_for_han_queries() {
        let tokens = tokenize_search_text("中文分词");

        assert!(
            tokens.iter().any(|token| token == "中文"),
            "tokens={tokens:?}"
        );
        assert!(
            tokens.iter().any(|token| token == "分词"),
            "tokens={tokens:?}"
        );
    }

    #[test]
    fn build_search_fts_query_includes_segmented_han_terms() {
        let query = build_search_fts_query("中文分词", 6).expect("fts query");

        assert!(query.contains("\"中文分词\""), "query={query}");
        assert!(query.contains("\"中文\""), "query={query}");
        assert!(query.contains("\"分词\""), "query={query}");
    }

    #[test]
    fn build_search_index_text_joins_segmented_terms() {
        let index_text = build_search_index_text(&["中文分词用于数据库搜索", "memory_indexed"]);

        assert!(index_text.contains("中文"), "index_text={index_text}");
        assert!(index_text.contains("分词"), "index_text={index_text}");
        assert!(index_text.contains("数据库"), "index_text={index_text}");
        assert!(index_text.contains("memory"), "index_text={index_text}");
        assert!(index_text.contains("indexed"), "index_text={index_text}");
    }

    #[test]
    fn build_search_index_text_segments_chinese_inside_mixed_payloads() {
        let index_text = build_search_index_text(&[
            "{\"summary\":\"中文分词已经启用\"}",
            "event_kind=memory_indexed",
        ]);

        assert!(index_text.contains("中文"), "index_text={index_text}");
        assert!(index_text.contains("分词"), "index_text={index_text}");
        assert!(index_text.contains("启用"), "index_text={index_text}");
        assert!(index_text.contains("memory"), "index_text={index_text}");
    }
}
