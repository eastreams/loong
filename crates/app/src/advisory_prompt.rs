const ADVISORY_HEADING_PREFIX: &str = "Advisory reference heading: ";

const GOVERNED_ADVISORY_HEADINGS: &[&str] = &[
    "runtime self context",
    "standing instructions",
    "tool usage policy",
    "soul guidance",
    "user context",
    "resolved runtime identity",
    "session profile",
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
    let mut rendered_lines = Vec::new();

    for line in content.lines() {
        let rendered_line = demote_governed_advisory_heading_line(line, allowed_root_headings);
        rendered_lines.push(rendered_line);
    }

    rendered_lines.join("\n")
}

fn demote_governed_advisory_heading_line(line: &str, allowed_root_headings: &[&str]) -> String {
    let trimmed_line = line.trim();
    let maybe_heading_text = markdown_heading_text(trimmed_line);
    let Some(heading_text) = maybe_heading_text else {
        return line.to_owned();
    };

    let normalized_heading = normalize_heading_text(heading_text);
    let is_allowed_heading = allowed_root_headings.contains(&normalized_heading.as_str());
    if is_allowed_heading {
        return line.to_owned();
    }

    let is_governed_heading = GOVERNED_ADVISORY_HEADINGS.contains(&normalized_heading.as_str());
    if !is_governed_heading {
        return line.to_owned();
    }

    let demoted_line = format!("{ADVISORY_HEADING_PREFIX}{heading_text}");
    demoted_line
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
    let trimmed_heading = heading_text.trim();
    trimmed_heading.to_ascii_lowercase()
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
}
