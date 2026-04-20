use std::collections::BTreeSet;
use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone)]
pub(super) struct SearchSignalSet {
    pub(super) normalized_text: String,
    pub(super) tokens: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(super) struct SearchQuery {
    pub(super) signal: SearchSignalSet,
    pub(super) concepts: BTreeSet<String>,
    pub(super) categories: BTreeSet<String>,
}

struct SearchConceptDefinition {
    id: &'static str,
    categories: &'static [&'static str],
    forms: &'static [&'static str],
}

struct NormalizedSearchConcept {
    id: &'static str,
    categories: &'static [&'static str],
    forms: Vec<String>,
}

const SEARCH_CONCEPT_DEFINITIONS: &[SearchConceptDefinition] = &[
    SearchConceptDefinition {
        id: "search",
        categories: &["discovery"],
        forms: &["search", "find", "discover", "lookup", "query"],
    },
    SearchConceptDefinition {
        id: "fetch",
        categories: &["network", "retrieval"],
        forms: &["fetch", "download", "retrieve", "obtain"],
    },
    SearchConceptDefinition {
        id: "read",
        categories: &["retrieval"],
        forms: &["read", "view", "show", "display", "open"],
    },
    SearchConceptDefinition {
        id: "inspect",
        categories: &["discovery", "retrieval"],
        forms: &["inspect", "detail", "details", "metadata", "describe"],
    },
    SearchConceptDefinition {
        id: "list",
        categories: &["discovery"],
        forms: &["list", "enumerate", "browse"],
    },
    SearchConceptDefinition {
        id: "write",
        categories: &["mutation"],
        forms: &["write", "create", "save", "append", "record"],
    },
    SearchConceptDefinition {
        id: "edit",
        categories: &["mutation"],
        forms: &["edit", "modify", "update", "replace", "patch"],
    },
    SearchConceptDefinition {
        id: "file",
        categories: &["workspace"],
        forms: &[
            "file",
            "files",
            "filesystem",
            "path",
            "repo",
            "repository",
            "workspace",
        ],
    },
    SearchConceptDefinition {
        id: "directory",
        categories: &["workspace", "discovery"],
        forms: &[
            "directory",
            "directories",
            "folder",
            "folders",
            "current directory",
            "current folder",
            "dir",
            "directory tree",
            "folder tree",
        ],
    },
    SearchConceptDefinition {
        id: "memory",
        categories: &["workspace"],
        forms: &[
            "memory",
            "note",
            "notes",
            "memo",
            "recall",
            "durable",
            "knowledge",
        ],
    },
    SearchConceptDefinition {
        id: "web",
        categories: &["network"],
        forms: &[
            "web", "website", "site", "url", "http", "https", "internet", "page",
        ],
    },
    SearchConceptDefinition {
        id: "browser",
        categories: &["interactive", "network"],
        forms: &[
            "browser", "browse", "navigate", "click", "type", "selector", "tab",
        ],
    },
    SearchConceptDefinition {
        id: "session",
        categories: &["coordination"],
        forms: &[
            "session",
            "thread",
            "conversation",
            "chat",
            "history",
            "event",
            "status",
            "queue",
            "session_id",
        ],
    },
    SearchConceptDefinition {
        id: "message",
        categories: &["communication"],
        forms: &["message", "messages", "send", "post", "reply"],
    },
    SearchConceptDefinition {
        id: "delegate",
        categories: &["coordination"],
        forms: &[
            "delegate",
            "delegation",
            "child",
            "background",
            "async",
            "subtask",
        ],
    },
    SearchConceptDefinition {
        id: "skill",
        categories: &["extension"],
        forms: &[
            "skill",
            "skills",
            "plugin",
            "extension",
            "package",
            "skillset",
        ],
    },
    SearchConceptDefinition {
        id: "install",
        categories: &["extension", "mutation"],
        forms: &["install", "setup", "enable", "configure"],
    },
    SearchConceptDefinition {
        id: "remove",
        categories: &["extension", "mutation"],
        forms: &["remove", "delete", "uninstall", "disable", "erase"],
    },
    SearchConceptDefinition {
        id: "provider",
        categories: &["runtime"],
        forms: &[
            "provider", "model", "runtime", "profile", "engine", "backend",
        ],
    },
    SearchConceptDefinition {
        id: "switch",
        categories: &["mutation", "runtime"],
        forms: &["switch", "change", "select", "swap", "choose", "toggle"],
    },
    SearchConceptDefinition {
        id: "approval",
        categories: &["governance"],
        forms: &[
            "approval",
            "approve",
            "permission",
            "policy",
            "security",
            "allow",
            "deny",
        ],
    },
    SearchConceptDefinition {
        id: "wait",
        categories: &["coordination"],
        forms: &["wait", "poll", "watch", "monitor", "until"],
    },
    SearchConceptDefinition {
        id: "cancel",
        categories: &["coordination", "mutation"],
        forms: &["cancel", "stop", "abort", "kill", "terminate"],
    },
    SearchConceptDefinition {
        id: "archive",
        categories: &["coordination"],
        forms: &["archive", "store", "retain"],
    },
    SearchConceptDefinition {
        id: "recover",
        categories: &["coordination"],
        forms: &["recover", "restore", "resume", "repair", "fix"],
    },
];

impl SearchSignalSet {
    pub(super) fn from_fragments(fragments: &[String]) -> Self {
        let mut normalized_fragments = Vec::new();
        let mut tokens = BTreeSet::new();

        for fragment in fragments {
            let normalized_fragment = normalize_search_text(fragment);
            if normalized_fragment.is_empty() {
                continue;
            }

            let fragment_tokens = tokenize_normalized_text(normalized_fragment.as_str());
            tokens.extend(fragment_tokens);
            normalized_fragments.push(normalized_fragment);
        }

        let normalized_text = normalized_fragments.join(" ");

        Self {
            normalized_text,
            tokens,
        }
    }

    pub(super) fn contains_term(&self, normalized_term: &str) -> bool {
        if normalized_term.is_empty() {
            return false;
        }

        let ascii_token_only = normalized_term
            .chars()
            .all(|character| character.is_ascii_alphanumeric());

        if ascii_token_only {
            return self.tokens.contains(normalized_term);
        }

        if self.tokens.contains(normalized_term) {
            return true;
        }

        self.normalized_text.contains(normalized_term)
    }
}

impl SearchQuery {
    pub(super) fn new(raw_query: &str) -> Self {
        let signal = search_query_signal_set(raw_query);
        let (mut concepts, mut categories) = extract_concepts_and_categories(&signal);
        apply_structural_query_hints(raw_query, &mut concepts, &mut categories);

        Self {
            signal,
            concepts,
            categories,
        }
    }
}

pub(super) fn search_query_signal_set(raw_query: &str) -> SearchSignalSet {
    let cleaned_tokens = raw_query
        .split_whitespace()
        .map(trim_structural_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut fragments = Vec::new();

    for token in cleaned_tokens {
        let has_path_separator = token.contains('/') || token.contains('\\');
        let host_like_path_token = has_path_separator && token_has_host_like_prefix(token);
        let url_like_token = token_looks_like_url(token);
        if url_like_token || host_like_path_token {
            continue;
        }

        fragments.push(token.to_owned());
    }

    SearchSignalSet::from_fragments(&fragments)
}

pub(super) fn identifier_phrase_variant(raw: &str) -> String {
    let normalized = normalize_search_text(raw);
    let mut characters = String::new();
    let mut last_was_space = false;

    for character in normalized.chars() {
        let replacement = if is_identifier_separator(character) {
            ' '
        } else {
            character
        };

        if replacement == ' ' {
            if last_was_space {
                continue;
            }

            last_was_space = true;
            characters.push(' ');
            continue;
        }

        last_was_space = false;
        characters.push(replacement);
    }

    characters.trim().to_owned()
}

pub(super) fn extract_concepts_and_categories(
    signal: &SearchSignalSet,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut concepts = BTreeSet::new();
    let mut categories = BTreeSet::new();

    for concept in normalized_search_concepts() {
        if !concept
            .forms
            .iter()
            .any(|form| signal.contains_term(form.as_str()))
        {
            continue;
        }

        concepts.insert(concept.id.to_owned());
        for category in concept.categories {
            categories.insert((*category).to_owned());
        }
    }

    (concepts, categories)
}

fn normalized_search_concepts() -> &'static Vec<NormalizedSearchConcept> {
    static SEARCH_CONCEPTS: OnceLock<Vec<NormalizedSearchConcept>> = OnceLock::new();

    SEARCH_CONCEPTS.get_or_init(|| {
        SEARCH_CONCEPT_DEFINITIONS
            .iter()
            .map(|concept| {
                let forms = concept
                    .forms
                    .iter()
                    .map(|form| normalize_search_text(form))
                    .filter(|form| !form.is_empty())
                    .collect::<Vec<_>>();

                NormalizedSearchConcept {
                    id: concept.id,
                    categories: concept.categories,
                    forms,
                }
            })
            .collect()
    })
}

fn apply_structural_query_hints(
    raw_query: &str,
    concepts: &mut BTreeSet<String>,
    categories: &mut BTreeSet<String>,
) {
    if query_looks_like_url(raw_query) {
        insert_concept_and_categories("web", concepts, categories);
    }

    if query_looks_like_file_reference(raw_query) {
        insert_concept_and_categories("file", concepts, categories);
    }
}

fn insert_concept_and_categories(
    concept_id: &str,
    concepts: &mut BTreeSet<String>,
    categories: &mut BTreeSet<String>,
) {
    concepts.insert(concept_id.to_owned());

    for concept in normalized_search_concepts() {
        if concept.id != concept_id {
            continue;
        }

        for category in concept.categories {
            categories.insert((*category).to_owned());
        }
    }
}

fn query_looks_like_url(raw_query: &str) -> bool {
    for token in raw_query.split_whitespace() {
        let cleaned = trim_structural_token(token);
        if cleaned.is_empty() {
            continue;
        }

        let has_path_separator = cleaned.contains('/') || cleaned.contains('\\');
        let host_like_path = has_path_separator && token_has_host_like_prefix(cleaned);
        if token_looks_like_url(cleaned) || host_like_path {
            return true;
        }
    }

    false
}

fn token_looks_like_url(token: &str) -> bool {
    token.contains("://") || token.starts_with("www.")
}

fn query_looks_like_file_reference(raw_query: &str) -> bool {
    let cleaned_tokens = raw_query
        .split_whitespace()
        .map(trim_structural_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let single_token_query = cleaned_tokens.len() == 1;

    for token in cleaned_tokens {
        if single_token_query && token_is_ambiguous_single_dot_token(token) {
            continue;
        }

        if token_looks_like_file_reference(token) {
            return true;
        }
    }

    false
}

fn token_looks_like_file_reference(token: &str) -> bool {
    if token_looks_like_url(token) {
        return false;
    }

    if token.contains('/') || token.contains('\\') {
        return !token_has_host_like_prefix(token);
    }

    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };

    if token.chars().filter(|character| *character == '.').count() == 1 {
        let stem_has_alpha = stem.chars().any(|character| character.is_alphabetic());
        let extension_length = extension.chars().count();
        let extension_length_valid = (2..=4).contains(&extension_length);
        let extension_characters_valid = extension
            .chars()
            .all(|character| character.is_ascii_alphabetic());

        return stem_has_alpha && extension_length_valid && extension_characters_valid;
    }

    let stem_valid = stem
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_' || character == '-');
    if !stem_valid {
        return false;
    }

    let extension_length = extension.chars().count();
    let extension_length_valid = (1..=8).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_alphanumeric());

    extension_length_valid && extension_characters_valid
}

fn token_has_host_like_prefix(token: &str) -> bool {
    let host_candidate = token.split(['/', '\\']).next().unwrap_or(token);
    let normalized_candidate = host_candidate.to_ascii_lowercase();
    let Some((stem, extension)) = normalized_candidate.rsplit_once('.') else {
        return false;
    };
    let extension_length = extension.chars().count();
    let extension_length_valid = (2..=4).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_ascii_lowercase());
    let stem_has_alpha = stem
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    let stem_characters_valid = stem.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '.'
    });

    stem_has_alpha && extension_length_valid && extension_characters_valid && stem_characters_valid
}

fn token_is_ambiguous_single_dot_token(token: &str) -> bool {
    if token.contains('/') || token.contains('\\') {
        return false;
    }

    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };
    if token.chars().filter(|character| *character == '.').count() != 1 {
        return false;
    }

    let stem_has_alpha = stem.chars().any(|character| character.is_alphabetic());
    let extension_length = extension.chars().count();
    let extension_length_valid = (2..=4).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_ascii_alphabetic());

    stem_has_alpha && extension_length_valid && extension_characters_valid
}

pub(super) fn trim_structural_token(token: &str) -> &str {
    token.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!' | '?'
            )
    })
}

pub(super) fn normalize_search_text(raw: &str) -> String {
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

fn tokenize_normalized_text(normalized: &str) -> BTreeSet<String> {
    let surface = identifier_phrase_variant(normalized);
    let mut tokens = BTreeSet::new();

    for word in UnicodeSegmentation::unicode_words(surface.as_str()) {
        let token = word.trim();
        if !should_keep_token(token) {
            continue;
        }

        tokens.insert(token.to_owned());
    }

    tokens
}

fn should_keep_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }

    if token
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        return token.len() >= 2;
    }

    true
}

fn is_identifier_separator(character: char) -> bool {
    matches!(
        character,
        '.' | '_' | '-' | '/' | '\\' | ':' | ',' | ';' | '|' | '(' | ')' | '[' | ']'
    )
}

pub(super) fn ordered_overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}
