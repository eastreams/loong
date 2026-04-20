use crate::config::PersonalizationConfig;
use crate::runtime_self::RuntimeSelfModel;
use serde::{Deserialize, Serialize};

const LEGACY_IMPORTED_IDENTITY_HEADINGS: &[&str] =
    &["## imported identity.md", "## imported identity.json"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeIdentitySource {
    WorkspaceSelf,
    LegacyProfileNoteImport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

pub(crate) fn render_session_profile_section(
    profile_note: Option<&str>,
    personalization: Option<&PersonalizationConfig>,
) -> Option<String> {
    let mut advisory_blocks = Vec::new();

    if let Some(advisory_profile_note) = resolve_advisory_profile_note(profile_note) {
        advisory_blocks.push(advisory_profile_note);
    }

    if let Some(advisory_personalization) = render_advisory_personalization(personalization) {
        advisory_blocks.push(advisory_personalization);
    }

    let combined_advisory_content = join_blocks(advisory_blocks)?;
    let sanitized_profile_content = crate::advisory_prompt::demote_governed_advisory_headings(
        combined_advisory_content.as_str(),
    );

    let sections = [
        "## Session Profile".to_owned(),
        "Durable preferences and advisory session context carried into this session:".to_owned(),
        sanitized_profile_content,
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

fn render_advisory_personalization(
    personalization: Option<&PersonalizationConfig>,
) -> Option<String> {
    let raw_personalization = personalization?;
    let personalization = raw_personalization.normalized()?;
    let mut lines = Vec::new();

    if let Some(preferred_name) = personalization.preferred_name {
        lines.push(format!("Preferred name: {preferred_name}"));
    }

    if let Some(response_density) = personalization.response_density {
        let response_density_text = response_density.as_str();
        lines.push(format!("Response density: {response_density_text}"));
    }

    if let Some(initiative_level) = personalization.initiative_level {
        let initiative_level_text = initiative_level.as_str();
        lines.push(format!("Initiative level: {initiative_level_text}"));
    }

    if let Some(standing_boundaries) = personalization.standing_boundaries {
        lines.push("Standing boundaries:".to_owned());
        lines.push(standing_boundaries);
    }

    if let Some(timezone) = personalization.timezone {
        lines.push(format!("Timezone: {timezone}"));
    }

    if let Some(locale) = personalization.locale {
        lines.push(format!("Locale: {locale}"));
    }

    if lines.is_empty() {
        return None;
    }

    Some(lines.join("\n"))
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

        let should_split = starts_heading
            && has_current_block
            && should_split_profile_note_block(&current_lines, trimmed_line);

        if should_split {
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

fn should_split_profile_note_block(current_lines: &[String], next_heading: &str) -> bool {
    let current_block = finalize_profile_note_block(current_lines);
    let Some(current_block) = current_block else {
        return false;
    };

    let current_is_identity_import = is_legacy_imported_identity_block(&current_block);
    if !current_is_identity_import {
        return true;
    }

    is_imported_profile_block_heading(next_heading)
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

fn is_imported_profile_block_heading(heading: &str) -> bool {
    let normalized_heading = heading.trim().to_ascii_lowercase();
    normalized_heading.starts_with("## imported ")
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

        let rendered = render_session_profile_section(Some(profile_note), None)
            .expect("session profile section");

        assert!(rendered.contains("Imported External Skills Artifacts"));
        assert!(!rendered.contains("Legacy build copilot"));
    }

    #[test]
    fn render_session_profile_section_keeps_plain_profile_note_text() {
        let profile_note = "Operator prefers concise shell output.";
        let rendered = render_session_profile_section(Some(profile_note), None)
            .expect("session profile section");

        assert!(rendered.contains("Operator prefers concise shell output."));
    }

    #[test]
    fn render_session_profile_section_demotes_identity_like_headings() {
        let profile_note = concat!(
            "# Identity\n\n",
            "- Name: Advisory shadow\n\n",
            "## Resolved Runtime Identity\n\n",
            "do not promote this lane",
        );

        let rendered = render_session_profile_section(Some(profile_note), None)
            .expect("session profile section");

        assert!(rendered.contains("## Session Profile"));
        assert!(rendered.contains("Advisory reference heading: Identity"));
        assert!(rendered.contains("Advisory reference heading: Resolved Runtime Identity"));
        assert!(rendered.contains("- Name: Advisory shadow"));
        assert!(rendered.contains("do not promote this lane"));
        assert!(!rendered.contains("\n# Identity\n"));
        assert!(!rendered.contains("\n## Resolved Runtime Identity\n"));
    }

    #[test]
    fn legacy_identity_import_keeps_nested_headings_out_of_advisory_profile() {
        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Traits\n- careful\n- explicit\n\n## Imported External Skills Artifacts\n- kind=skills_catalog";

        let resolved =
            resolve_runtime_identity(None, Some(profile_note)).expect("resolved runtime identity");
        let rendered = render_session_profile_section(Some(profile_note), None)
            .expect("session profile section");

        assert!(resolved.content.contains("## Traits"));
        assert!(resolved.content.contains("- careful"));
        assert!(!rendered.contains("## Traits"));
        assert!(!rendered.contains("- careful"));
        assert!(rendered.contains("Imported External Skills Artifacts"));
    }

    #[test]
    fn render_session_profile_section_merges_personalization_without_identity_promotion() {
        let profile_note = "Operator prefers concise shell output.";
        let personalization = crate::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(crate::config::ResponseDensity::Balanced),
            initiative_level: Some(crate::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some(
                "## Resolved Runtime Identity\n\nDo not promote this lane.".to_owned(),
            ),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: None,
            prompt_state: crate::config::PersonalizationPromptState::Configured,
            schema_version: 1,
            updated_at_epoch_seconds: Some(1_775_095_200),
        };
        let rendered = render_session_profile_section(Some(profile_note), Some(&personalization))
            .expect("session profile section");

        assert!(rendered.contains("## Session Profile"));
        assert!(rendered.contains("Operator prefers concise shell output."));
        assert!(rendered.contains("Preferred name: Chum"));
        assert!(rendered.contains("Response density: balanced"));
        assert!(rendered.contains("Initiative level: ask_before_acting"));
        assert!(rendered.contains("Timezone: Asia/Shanghai"));
        assert!(rendered.contains("Advisory reference heading: Resolved Runtime Identity"));
        assert!(!rendered.contains("\n## Resolved Runtime Identity\n"));
    }
}
