use crate::runtime_self::RuntimeSelfModel;

const LEGACY_IMPORTED_IDENTITY_HEADINGS: &[&str] =
    &["## imported identity.md", "## imported identity.json"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeIdentitySource {
    WorkspaceSelf,
    LegacyProfileNoteImport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRuntimeIdentity {
    pub source: RuntimeIdentitySource,
    pub content: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProfileNotePartition {
    identity_blocks: Vec<String>,
    advisory_blocks: Vec<String>,
}

pub(crate) fn resolve_runtime_identity(
    runtime_self_model: Option<&RuntimeSelfModel>,
    profile_note: Option<&str>,
) -> Option<ResolvedRuntimeIdentity> {
    let workspace_identity = runtime_self_model.and_then(resolve_workspace_identity);
    if let Some(content) = workspace_identity {
        let identity = ResolvedRuntimeIdentity {
            source: RuntimeIdentitySource::WorkspaceSelf,
            content,
        };
        return Some(identity);
    }

    let legacy_identity = resolve_legacy_imported_identity(profile_note);
    legacy_identity.map(|content| ResolvedRuntimeIdentity {
        source: RuntimeIdentitySource::LegacyProfileNoteImport,
        content,
    })
}

pub(crate) fn render_runtime_identity_section(identity: &ResolvedRuntimeIdentity) -> String {
    let intro = runtime_identity_intro(identity.source);
    let content = identity.content.trim().to_owned();

    let sections = [
        "## Resolved Runtime Identity".to_owned(),
        intro.to_owned(),
        content,
    ];
    sections.join("\n\n")
}

pub(crate) fn render_session_profile_section(profile_note: Option<&str>) -> Option<String> {
    let advisory_profile_note = resolve_advisory_profile_note(profile_note)?;

    let sections = [
        "## Session Profile".to_owned(),
        "Durable preferences and advisory session context carried into this session:".to_owned(),
        advisory_profile_note,
    ];
    Some(sections.join("\n"))
}

fn runtime_identity_intro(source: RuntimeIdentitySource) -> &'static str {
    match source {
        RuntimeIdentitySource::WorkspaceSelf => {
            "Active workspace identity context loaded from runtime self sources."
        }
        RuntimeIdentitySource::LegacyProfileNoteImport => {
            "Fallback identity context recovered from legacy imported profile state."
        }
    }
}

fn resolve_workspace_identity(model: &RuntimeSelfModel) -> Option<String> {
    let entries = &model.identity_context;
    join_trimmed_entries(entries)
}

fn resolve_legacy_imported_identity(profile_note: Option<&str>) -> Option<String> {
    let trimmed_profile_note = trim_profile_note(profile_note)?;
    let partition = partition_profile_note(trimmed_profile_note);
    let identity_blocks = partition.identity_blocks;
    join_blocks(identity_blocks)
}

fn resolve_advisory_profile_note(profile_note: Option<&str>) -> Option<String> {
    let trimmed_profile_note = trim_profile_note(profile_note)?;
    let partition = partition_profile_note(trimmed_profile_note);
    let advisory_blocks = partition.advisory_blocks;
    join_blocks(advisory_blocks)
}

fn trim_profile_note(profile_note: Option<&str>) -> Option<&str> {
    let raw_profile_note = profile_note?;
    let trimmed_profile_note = raw_profile_note.trim();
    if trimmed_profile_note.is_empty() {
        return None;
    }
    Some(trimmed_profile_note)
}

fn partition_profile_note(profile_note: &str) -> ProfileNotePartition {
    let mut partition = ProfileNotePartition::default();
    let blocks = split_profile_note_blocks(profile_note);

    for block in blocks {
        let is_identity_block = is_legacy_imported_identity_block(&block);
        if is_identity_block {
            partition.identity_blocks.push(block);
            continue;
        }
        partition.advisory_blocks.push(block);
    }

    partition
}

fn split_profile_note_blocks(profile_note: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_lines = Vec::new();

    for line in profile_note.lines() {
        let trimmed_line = line.trim();
        let starts_heading = trimmed_line.starts_with("## ");
        let has_current_block = !current_lines.is_empty();

        if starts_heading && has_current_block {
            let block = finalize_profile_note_block(&current_lines);
            if let Some(block) = block {
                blocks.push(block);
            }
            current_lines.clear();
        }

        current_lines.push(line.to_owned());
    }

    let final_block = finalize_profile_note_block(&current_lines);
    if let Some(final_block) = final_block {
        blocks.push(final_block);
    }

    blocks
}

fn finalize_profile_note_block(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    let joined_lines = lines.join("\n");
    let trimmed_block = joined_lines.trim();
    if trimmed_block.is_empty() {
        return None;
    }

    Some(trimmed_block.to_owned())
}

fn is_legacy_imported_identity_block(block: &str) -> bool {
    let heading = first_non_empty_line(block);
    let Some(heading) = heading else {
        return false;
    };

    let normalized_heading = heading.trim().to_ascii_lowercase();
    LEGACY_IMPORTED_IDENTITY_HEADINGS.contains(&normalized_heading.as_str())
}

fn first_non_empty_line(block: &str) -> Option<&str> {
    for line in block.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }
        return Some(trimmed_line);
    }

    None
}

fn join_trimmed_entries(entries: &[String]) -> Option<String> {
    let mut normalized_entries = Vec::new();

    for entry in entries {
        let trimmed_entry = entry.trim();
        if trimmed_entry.is_empty() {
            continue;
        }
        normalized_entries.push(trimmed_entry.to_owned());
    }

    join_blocks(normalized_entries)
}

fn join_blocks(blocks: Vec<String>) -> Option<String> {
    let mut normalized_blocks = Vec::new();

    for block in blocks {
        let trimmed_block = block.trim();
        if trimmed_block.is_empty() {
            continue;
        }
        normalized_blocks.push(trimmed_block.to_owned());
    }

    if normalized_blocks.is_empty() {
        return None;
    }

    Some(normalized_blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_runtime_identity_prefers_workspace_identity_over_legacy_profile_note_identity() {
        let runtime_self_model = RuntimeSelfModel {
            identity_context: vec!["# Identity\n\n- Name: Workspace build copilot".to_owned()],
            ..RuntimeSelfModel::default()
        };
        let legacy_profile_note =
            "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";

        let resolved =
            resolve_runtime_identity(Some(&runtime_self_model), Some(legacy_profile_note))
                .expect("resolved runtime identity");

        assert_eq!(resolved.source, RuntimeIdentitySource::WorkspaceSelf);
        assert!(resolved.content.contains("Workspace build copilot"));
        assert!(!resolved.content.contains("Legacy build copilot"));
    }

    #[test]
    fn resolve_runtime_identity_falls_back_to_legacy_profile_note_identity() {
        let legacy_profile_note =
            "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";

        let resolved = resolve_runtime_identity(None, Some(legacy_profile_note))
            .expect("resolved runtime identity");

        assert_eq!(
            resolved.source,
            RuntimeIdentitySource::LegacyProfileNoteImport
        );
        assert!(resolved.content.contains("Legacy build copilot"));
    }

    #[test]
    fn resolve_runtime_identity_ignores_non_identity_profile_notes() {
        let profile_note = "Operator prefers concise shell output.";
        let resolved = resolve_runtime_identity(None, Some(profile_note));

        assert_eq!(resolved, None);
    }

    #[test]
    fn render_session_profile_section_strips_legacy_identity_blocks() {
        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Imported External Skills Artifacts\n- kind=skills_catalog";

        let rendered =
            render_session_profile_section(Some(profile_note)).expect("session profile section");

        assert!(rendered.contains("Imported External Skills Artifacts"));
        assert!(!rendered.contains("Legacy build copilot"));
    }

    #[test]
    fn render_session_profile_section_keeps_plain_profile_note_text() {
        let profile_note = "Operator prefers concise shell output.";
        let rendered =
            render_session_profile_section(Some(profile_note)).expect("session profile section");

        assert!(rendered.contains("Operator prefers concise shell output."));
    }
}
