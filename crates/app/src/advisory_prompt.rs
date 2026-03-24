const ADVISORY_HEADING_PREFIX: &str = "Advisory reference heading: ";

const GOVERNED_ADVISORY_HEADINGS: &[&str] = &[
    "runtime self context",
    "standing instructions",
    "tool usage policy",
    "soul guidance",
    "user context",
    "resolved runtime identity",
    "session profile",
    "memory summary",
    "advisory durable recall",
    "identity",
    "imported identity.md",
    "imported identity.json",
];

pub(crate) fn demote_governed_advisory_headings(content: &str) -> String {
    demote_governed_advisory_headings_with_allowed_roots(content, &[])
}

pub(crate) fn demote_governed_advisory_headings_with_allowed_roots(
    content: &str,
    allowed_root_headings: &[&str],
) -> String {
    let root_heading_line_index = first_markdown_heading_line_index(content);
    let mut rendered_lines = Vec::new();

    for (line_index, line) in content.lines().enumerate() {
        let rendered_line = demote_governed_advisory_heading_line(
            line_index,
            line,
            allowed_root_headings,
            root_heading_line_index,
        );
        rendered_lines.push(rendered_line);
    }

    rendered_lines.join("\n")
}

fn demote_governed_advisory_heading_line(
    line_index: usize,
    line: &str,
    allowed_root_headings: &[&str],
    root_heading_line_index: Option<usize>,
) -> String {
    let trimmed_line = line.trim();
    let maybe_heading_text = markdown_heading_text(trimmed_line);
    let Some(heading_text) = maybe_heading_text else {
        return line.to_owned();
    };

    let normalized_heading = normalize_heading_text(heading_text);
    let is_allowed_root_heading = is_allowed_root_heading(
        line_index,
        normalized_heading.as_str(),
        allowed_root_headings,
        root_heading_line_index,
    );
    if is_allowed_root_heading {
        return line.to_owned();
    }

    let is_governed_heading = GOVERNED_ADVISORY_HEADINGS.contains(&normalized_heading.as_str());
    if !is_governed_heading {
        return line.to_owned();
    }

    let display_heading = display_heading_text(heading_text);
    let demoted_line = format!("{ADVISORY_HEADING_PREFIX}{display_heading}");
    demoted_line
}

fn first_markdown_heading_line_index(content: &str) -> Option<usize> {
    for (line_index, line) in content.lines().enumerate() {
        let trimmed_line = line.trim();
        let maybe_heading_text = markdown_heading_text(trimmed_line);
        if maybe_heading_text.is_some() {
            return Some(line_index);
        }
    }

    None
}

fn is_allowed_root_heading(
    line_index: usize,
    normalized_heading: &str,
    allowed_root_headings: &[&str],
    root_heading_line_index: Option<usize>,
) -> bool {
    if root_heading_line_index != Some(line_index) {
        return false;
    }

    allowed_root_headings.contains(&normalized_heading)
}

fn markdown_heading_text(line: &str) -> Option<&str> {
    let mut depth = 0usize;

    for ch in line.chars() {
        if ch != '#' {
            break;
        }
        depth = depth.saturating_add(1);
    }

    if depth == 0 || depth > 6 {
        return None;
    }

    let heading_suffix = &line[depth..];
    let trimmed_heading = heading_suffix.trim();
    if trimmed_heading.is_empty() {
        return None;
    }

    Some(trimmed_heading)
}

fn normalize_heading_text(heading_text: &str) -> String {
    let display_heading = display_heading_text(heading_text);
    display_heading.to_ascii_lowercase()
}

fn display_heading_text(heading_text: &str) -> &str {
    let trimmed_heading = heading_text.trim();
    strip_optional_markdown_closing_sequence(trimmed_heading)
}

fn strip_optional_markdown_closing_sequence(heading_text: &str) -> &str {
    let without_trailing_hashes = heading_text.trim_end_matches('#');
    let trimmed_hashes = without_trailing_hashes.len() != heading_text.len();
    if !trimmed_hashes {
        return heading_text;
    }

    let separator_char = without_trailing_hashes.chars().next_back();
    let has_separator_space = separator_char.is_some_and(|ch| ch.is_whitespace());
    if !has_separator_space {
        return heading_text;
    }

    let stripped_heading = without_trailing_hashes.trim_end();
    if stripped_heading.is_empty() {
        return heading_text;
    }

    stripped_heading
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demote_governed_advisory_headings_rewrites_runtime_owned_heading_lines() {
        let content = concat!(
            "## Runtime Self Context\n\n",
            "### Tool Usage Policy\n",
            "- keep it explicit",
        );

        let rendered = demote_governed_advisory_headings(content);

        assert!(rendered.contains("Advisory reference heading: Runtime Self Context"));
        assert!(rendered.contains("Advisory reference heading: Tool Usage Policy"));
        assert!(rendered.contains("- keep it explicit"));
        assert!(!rendered.contains("\n## Runtime Self Context\n"));
        assert!(!rendered.contains("\n### Tool Usage Policy\n"));
    }

    #[test]
    fn demote_governed_advisory_headings_rewrites_identity_like_heading_lines() {
        let content = concat!(
            "# Identity\n\n",
            "- Name: advisory shadow\n\n",
            "## Imported IDENTITY.md",
        );

        let rendered = demote_governed_advisory_headings(content);

        assert!(rendered.contains("Advisory reference heading: Identity"));
        assert!(rendered.contains("- Name: advisory shadow"));
        assert!(rendered.contains("Advisory reference heading: Imported IDENTITY.md"));
        assert!(!rendered.contains("\n# Identity\n"));
        assert!(!rendered.contains("\n## Imported IDENTITY.md"));
    }

    #[test]
    fn demote_governed_advisory_headings_keeps_normal_text_unchanged() {
        let content = concat!(
            "Operator prefers concise shell output.\n\n",
            "### Project Preferences\n",
            "- avoid guesswork",
        );

        let rendered = demote_governed_advisory_headings(content);

        assert_eq!(rendered, content);
    }

    #[test]
    fn demote_governed_advisory_headings_with_allowed_roots_keeps_container_heading() {
        let content = concat!(
            "## Session Profile\n",
            "Durable preferences and advisory session context carried into this session:\n",
            "Advisory reference heading: Identity",
        );

        let rendered =
            demote_governed_advisory_headings_with_allowed_roots(content, &["session profile"]);

        assert!(rendered.contains("## Session Profile"));
        assert!(!rendered.contains("Advisory reference heading: Session Profile"));
    }

    #[test]
    fn demote_governed_advisory_headings_with_allowed_roots_only_preserves_first_root_heading() {
        let content = concat!(
            "## Memory Summary\n",
            "Earlier session context condensed from turns outside the active window:\n",
            "- keep the top-level container\n\n",
            "## Memory Summary\n",
            "- do not preserve repeated container headings",
        );

        let rendered =
            demote_governed_advisory_headings_with_allowed_roots(content, &["memory summary"]);

        assert!(rendered.starts_with("## Memory Summary\n"));
        assert_eq!(rendered.matches("## Memory Summary").count(), 1);
        assert!(rendered.contains("Advisory reference heading: Memory Summary"));
        assert!(rendered.contains("- do not preserve repeated container headings"));
    }

    #[test]
    fn normalize_heading_text_strips_optional_markdown_closing_markers() {
        let resolved_identity_heading =
            markdown_heading_text("## Resolved Runtime Identity ##").expect("heading");
        let resolved_identity_normalized = normalize_heading_text(resolved_identity_heading);

        let identity_heading = markdown_heading_text("# Identity ###").expect("heading");
        let identity_normalized = normalize_heading_text(identity_heading);

        let csharp_heading = markdown_heading_text("# C#").expect("heading");
        let csharp_normalized = normalize_heading_text(csharp_heading);

        assert_eq!(resolved_identity_normalized, "resolved runtime identity");
        assert_eq!(identity_normalized, "identity");
        assert_eq!(csharp_normalized, "c#");
    }

    #[test]
    fn demote_governed_advisory_headings_strips_optional_markdown_closing_markers_in_output() {
        let content = concat!(
            "## Resolved Runtime Identity ##\n",
            "# Identity ###\n",
            "- keep the advisory body visible",
        );

        let rendered = demote_governed_advisory_headings(content);

        assert!(rendered.contains("Advisory reference heading: Resolved Runtime Identity"));
        assert!(rendered.contains("Advisory reference heading: Identity"));
        assert!(!rendered.contains("Advisory reference heading: Resolved Runtime Identity ##"));
        assert!(!rendered.contains("Advisory reference heading: Identity ###"));
    }
}
