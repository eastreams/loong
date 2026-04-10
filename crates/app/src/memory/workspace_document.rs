use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};

use super::{
    DerivedMemoryKind, MemoryAuthority, MemoryContextProvenance, MemoryRecordStatus,
    MemoryTrustLevel, WorkspaceMemoryDocumentKind, WorkspaceMemoryDocumentLocation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedWorkspaceMemoryDocument {
    pub body: String,
    pub body_line_offset: usize,
    pub provenance: MemoryContextProvenance,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WorkspaceMemoryFrontmatter {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    trust: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    superseded_by: Option<String>,
}

#[derive(Debug, Clone)]
struct WorkspaceFrontmatterSplit<'a> {
    raw_frontmatter: WorkspaceMemoryFrontmatter,
    body: &'a str,
    body_line_offset: usize,
}

pub(crate) fn parse_workspace_memory_document(
    raw_content: &str,
    location: &WorkspaceMemoryDocumentLocation,
    memory_system_id: &str,
    recall_mode: super::MemoryRecallMode,
) -> Result<Option<ParsedWorkspaceMemoryDocument>, String> {
    let split = split_workspace_memory_frontmatter(raw_content)?;
    let trimmed_body = split.body.trim();
    if trimmed_body.is_empty() {
        return Ok(None);
    }

    let normalized_frontmatter = normalize_workspace_memory_frontmatter(split.raw_frontmatter);
    let derived_kind = resolve_workspace_memory_kind(
        &normalized_frontmatter,
        location.kind,
        location.label.as_str(),
    )?;
    let trust_level = resolve_workspace_memory_trust(
        &normalized_frontmatter,
        location.kind,
        location.label.as_str(),
    )?;
    let record_status =
        resolve_workspace_memory_status(&normalized_frontmatter, location.label.as_str())?;
    let content_hash = workspace_memory_content_hash(trimmed_body);
    let maybe_freshness_ts = workspace_memory_freshness_ts(location)?;
    let maybe_superseded_by = normalized_frontmatter.superseded_by;
    let scope = workspace_memory_scope(location.kind);
    let mut provenance = MemoryContextProvenance::new(
        memory_system_id,
        super::MemoryProvenanceSourceKind::WorkspaceDocument,
        Some(location.label.clone()),
        Some(location.path.display().to_string()),
        Some(scope),
        recall_mode,
    )
    .with_trust_level(trust_level)
    .with_authority(MemoryAuthority::Advisory)
    .with_derived_kind(derived_kind)
    .with_content_hash(content_hash)
    .with_record_status(record_status);

    if let Some(freshness_ts) = maybe_freshness_ts {
        provenance = provenance.with_freshness_ts(freshness_ts);
    }

    if let Some(superseded_by) = maybe_superseded_by {
        provenance = provenance.with_superseded_by(superseded_by);
    }

    let parsed_document = ParsedWorkspaceMemoryDocument {
        body: trimmed_body.to_owned(),
        body_line_offset: split.body_line_offset,
        provenance,
    };

    Ok(Some(parsed_document))
}

fn split_workspace_memory_frontmatter(
    raw_content: &str,
) -> Result<WorkspaceFrontmatterSplit<'_>, String> {
    let mut segments = raw_content.split_inclusive('\n');
    let Some(first_segment) = segments.next() else {
        return Ok(WorkspaceFrontmatterSplit {
            raw_frontmatter: WorkspaceMemoryFrontmatter::default(),
            body: raw_content,
            body_line_offset: 0,
        });
    };

    let first_trimmed = first_segment.trim();
    if first_trimmed != "---" {
        return Ok(WorkspaceFrontmatterSplit {
            raw_frontmatter: WorkspaceMemoryFrontmatter::default(),
            body: raw_content,
            body_line_offset: 0,
        });
    }

    let mut raw_frontmatter_segments = Vec::new();
    let mut consumed_bytes = first_segment.len();
    let mut consumed_lines = 1usize;

    for segment in segments {
        consumed_bytes = consumed_bytes.saturating_add(segment.len());
        consumed_lines = consumed_lines.saturating_add(1);

        let trimmed_segment = segment.trim();
        if trimmed_segment == "---" {
            let raw_frontmatter =
                decode_workspace_memory_frontmatter(raw_frontmatter_segments.as_slice())?;
            let body = &raw_content[consumed_bytes..];
            let split = WorkspaceFrontmatterSplit {
                raw_frontmatter,
                body,
                body_line_offset: consumed_lines,
            };
            return Ok(split);
        }

        raw_frontmatter_segments.push(segment);
    }

    Err("workspace memory frontmatter is missing a closing `---` delimiter".to_owned())
}

fn decode_workspace_memory_frontmatter(
    raw_frontmatter_segments: &[&str],
) -> Result<WorkspaceMemoryFrontmatter, String> {
    let raw_frontmatter = raw_frontmatter_segments.concat();
    let trimmed_frontmatter = raw_frontmatter.trim();
    if trimmed_frontmatter.is_empty() {
        return Ok(WorkspaceMemoryFrontmatter::default());
    }

    let parsed = serde_yaml::from_str::<YamlValue>(trimmed_frontmatter)
        .map_err(|error| format!("failed to parse workspace memory frontmatter: {error}"))?;
    match parsed {
        YamlValue::Null => Ok(WorkspaceMemoryFrontmatter::default()),
        YamlValue::Mapping(_) => serde_yaml::from_value(parsed).map_err(|error| {
            format!("failed to decode supported workspace memory frontmatter fields: {error}")
        }),
        YamlValue::Bool(_)
        | YamlValue::Number(_)
        | YamlValue::String(_)
        | YamlValue::Sequence(_)
        | YamlValue::Tagged(_) => {
            Err("workspace memory frontmatter must decode to a YAML mapping".to_owned())
        }
    }
}

fn normalize_workspace_memory_frontmatter(
    mut frontmatter: WorkspaceMemoryFrontmatter,
) -> WorkspaceMemoryFrontmatter {
    frontmatter.kind = normalize_optional_workspace_memory_enum_string(frontmatter.kind.take());
    frontmatter.trust = normalize_optional_workspace_memory_enum_string(frontmatter.trust.take());
    frontmatter.status = normalize_optional_workspace_memory_enum_string(frontmatter.status.take());
    frontmatter.superseded_by =
        normalize_optional_workspace_memory_string(frontmatter.superseded_by.take());

    frontmatter
}

fn normalize_optional_workspace_memory_enum_string(value: Option<String>) -> Option<String> {
    let normalized = normalize_optional_workspace_memory_string(value);
    normalized.map(|value| value.to_ascii_lowercase())
}

fn normalize_optional_workspace_memory_string(value: Option<String>) -> Option<String> {
    let normalized = value.map(|value| value.trim().to_owned());
    normalized.filter(|value| !value.is_empty())
}

fn resolve_workspace_memory_kind(
    frontmatter: &WorkspaceMemoryFrontmatter,
    document_kind: WorkspaceMemoryDocumentKind,
    source_label: &str,
) -> Result<DerivedMemoryKind, String> {
    if let Some(kind_text) = frontmatter.kind.as_deref() {
        let maybe_parsed_kind = DerivedMemoryKind::parse_id(kind_text);
        let Some(parsed_kind) = maybe_parsed_kind else {
            return Err(format!(
                "workspace memory file {source_label} declares unsupported frontmatter kind `{kind_text}`"
            ));
        };
        return Ok(parsed_kind);
    }

    let default_kind = match document_kind {
        WorkspaceMemoryDocumentKind::Curated => DerivedMemoryKind::Overview,
        WorkspaceMemoryDocumentKind::DailyLog => DerivedMemoryKind::Episode,
    };

    Ok(default_kind)
}

fn resolve_workspace_memory_trust(
    frontmatter: &WorkspaceMemoryFrontmatter,
    document_kind: WorkspaceMemoryDocumentKind,
    source_label: &str,
) -> Result<MemoryTrustLevel, String> {
    if let Some(trust_text) = frontmatter.trust.as_deref() {
        let maybe_parsed_trust = match trust_text {
            "session" => Some(MemoryTrustLevel::Session),
            "derived" => Some(MemoryTrustLevel::Derived),
            "workspace_curated" => Some(MemoryTrustLevel::WorkspaceCurated),
            "workspace_log" => Some(MemoryTrustLevel::WorkspaceLog),
            _ => None,
        };
        let Some(parsed_trust) = maybe_parsed_trust else {
            return Err(format!(
                "workspace memory file {source_label} declares unsupported frontmatter trust `{trust_text}`"
            ));
        };
        return Ok(parsed_trust);
    }

    let default_trust = match document_kind {
        WorkspaceMemoryDocumentKind::Curated => MemoryTrustLevel::WorkspaceCurated,
        WorkspaceMemoryDocumentKind::DailyLog => MemoryTrustLevel::WorkspaceLog,
    };

    Ok(default_trust)
}

fn resolve_workspace_memory_status(
    frontmatter: &WorkspaceMemoryFrontmatter,
    source_label: &str,
) -> Result<MemoryRecordStatus, String> {
    let Some(status_text) = frontmatter.status.as_deref() else {
        return Ok(MemoryRecordStatus::Active);
    };

    let maybe_parsed_status = match status_text {
        "active" => Some(MemoryRecordStatus::Active),
        "superseded" => Some(MemoryRecordStatus::Superseded),
        "tombstoned" => Some(MemoryRecordStatus::Tombstoned),
        "archived" => Some(MemoryRecordStatus::Archived),
        _ => None,
    };
    let Some(parsed_status) = maybe_parsed_status else {
        return Err(format!(
            "workspace memory file {source_label} declares unsupported frontmatter status `{status_text}`"
        ));
    };

    Ok(parsed_status)
}

fn workspace_memory_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());

    let digest = hasher.finalize();
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn workspace_memory_freshness_ts(
    location: &WorkspaceMemoryDocumentLocation,
) -> Result<Option<i64>, String> {
    if let Some(date) = location.date {
        let maybe_date_time = date.and_hms_opt(0, 0, 0);
        let Some(date_time) = maybe_date_time else {
            return Err(format!(
                "workspace memory file {} has an invalid date",
                location.label
            ));
        };
        let timestamp = date_time.and_utc().timestamp();
        return Ok(Some(timestamp));
    }

    let metadata = std::fs::metadata(location.path.as_path()).map_err(|error| {
        format!(
            "read workspace memory metadata {} failed: {error}",
            location.path.display()
        )
    })?;
    let modified_time = metadata.modified().map_err(|error| {
        format!(
            "read workspace memory modified time {} failed: {error}",
            location.path.display()
        )
    })?;
    let duration_since_epoch = modified_time.duration_since(UNIX_EPOCH).map_err(|error| {
        format!(
            "read workspace memory modified time {} failed: {error}",
            location.path.display()
        )
    })?;
    let seconds = duration_since_epoch.as_secs();
    let freshness_ts = i64::try_from(seconds).map_err(|error| {
        format!(
            "workspace memory modified time {} exceeds i64: {error}",
            location.path.display()
        )
    })?;

    Ok(Some(freshness_ts))
}

fn workspace_memory_scope(document_kind: WorkspaceMemoryDocumentKind) -> super::MemoryScope {
    match document_kind {
        WorkspaceMemoryDocumentKind::Curated => super::MemoryScope::Workspace,
        WorkspaceMemoryDocumentKind::DailyLog => super::MemoryScope::Session,
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use tempfile::tempdir;

    use super::*;

    fn curated_location(path: &std::path::Path) -> WorkspaceMemoryDocumentLocation {
        WorkspaceMemoryDocumentLocation {
            label: "MEMORY.md".to_owned(),
            path: path.to_path_buf(),
            kind: WorkspaceMemoryDocumentKind::Curated,
            date: None,
        }
    }

    #[test]
    fn parse_workspace_memory_document_strips_frontmatter_and_keeps_metadata() {
        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir.path().join("MEMORY.md");
        let raw = concat!(
            "---\n",
            "kind: procedure\n",
            "trust: workspace_curated\n",
            "status: active\n",
            "---\n",
            "Remember the deploy runbook.\n",
        );

        std::fs::write(&path, raw).expect("write memory");

        let maybe_parsed = parse_workspace_memory_document(
            raw,
            &curated_location(path.as_path()),
            "builtin",
            super::super::MemoryRecallMode::PromptAssembly,
        )
        .expect("parse workspace document");
        let parsed = maybe_parsed.expect("workspace document");

        assert_eq!(parsed.body, "Remember the deploy runbook.");
        assert_eq!(parsed.body_line_offset, 5);
        assert_eq!(
            parsed.provenance.derived_kind,
            Some(DerivedMemoryKind::Procedure)
        );
        assert_eq!(
            parsed.provenance.trust_level,
            Some(MemoryTrustLevel::WorkspaceCurated)
        );
        assert_eq!(
            parsed.provenance.record_status,
            Some(MemoryRecordStatus::Active)
        );
    }

    #[test]
    fn parse_workspace_memory_document_defaults_kind_and_trust_from_location_kind() {
        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir.path().join("memory").join("2026-04-08.md");

        std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        std::fs::write(&path, "Captured rollout follow-up.\n").expect("write memory");

        let location = WorkspaceMemoryDocumentLocation {
            label: "memory/2026-04-08.md".to_owned(),
            path,
            kind: WorkspaceMemoryDocumentKind::DailyLog,
            date: Some(NaiveDate::from_ymd_opt(2026, 4, 8).expect("date")),
        };
        let maybe_parsed = parse_workspace_memory_document(
            "Captured rollout follow-up.\n",
            &location,
            "builtin",
            super::super::MemoryRecallMode::PromptAssembly,
        )
        .expect("parse workspace document");
        let parsed = maybe_parsed.expect("workspace document");

        assert_eq!(
            parsed.provenance.derived_kind,
            Some(DerivedMemoryKind::Episode)
        );
        assert_eq!(
            parsed.provenance.trust_level,
            Some(MemoryTrustLevel::WorkspaceLog)
        );
        assert_eq!(
            parsed.provenance.record_status,
            Some(MemoryRecordStatus::Active)
        );
    }

    #[test]
    fn parse_workspace_memory_document_accepts_archived_status() {
        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir.path().join("MEMORY.md");
        let raw = concat!(
            "---\n",
            "status: archived\n",
            "---\n",
            "Remember the deploy runbook.\n",
        );

        std::fs::write(&path, raw).expect("write memory");

        let maybe_parsed = parse_workspace_memory_document(
            raw,
            &curated_location(path.as_path()),
            "builtin",
            super::super::MemoryRecallMode::PromptAssembly,
        )
        .expect("parse workspace document");
        let parsed = maybe_parsed.expect("workspace document");

        assert_eq!(
            parsed.provenance.record_status,
            Some(MemoryRecordStatus::Archived)
        );
    }

    #[test]
    fn parse_workspace_memory_document_returns_none_for_empty_body_after_frontmatter() {
        let temp_dir = tempdir().expect("tempdir");
        let path = temp_dir.path().join("MEMORY.md");
        let raw = concat!("---\n", "kind: overview\n", "---\n", "   \n");

        std::fs::write(&path, raw).expect("write memory");

        let parsed = parse_workspace_memory_document(
            raw,
            &curated_location(path.as_path()),
            "builtin",
            super::super::MemoryRecallMode::PromptAssembly,
        )
        .expect("parse workspace document");

        assert!(parsed.is_none());
    }
}
