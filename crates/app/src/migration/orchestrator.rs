use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::CliResult;

use super::{inspect_import_path, plan_import_from_path, LegacyClawSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryOptions {
    pub include_child_directories: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            include_child_directories: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredImportSource {
    pub source: LegacyClawSource,
    pub path: PathBuf,
    pub confidence_score: u32,
    pub found_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveryReport {
    pub sources: Vec<DiscoveredImportSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedImportSource {
    pub source: LegacyClawSource,
    pub source_id: String,
    pub input_path: PathBuf,
    pub confidence_score: u32,
    pub prompt_addendum_present: bool,
    pub profile_note_present: bool,
    pub warning_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscoveryPlanSummary {
    pub plans: Vec<PlannedImportSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimarySourceRecommendation {
    pub source: LegacyClawSource,
    pub source_id: String,
    pub input_path: PathBuf,
    pub reasons: Vec<String>,
}

pub fn discover_import_sources(
    search_root: &Path,
    options: DiscoveryOptions,
) -> CliResult<DiscoveryReport> {
    if !search_root.exists() {
        return Err(format!(
            "discovery root does not exist: {}",
            search_root.display()
        ));
    }

    let mut sources = Vec::new();
    for candidate in collect_candidate_directories(search_root, &options)? {
        let Some(inspection) = inspect_import_path(&candidate, None)? else {
            continue;
        };
        sources.push(DiscoveredImportSource {
            source: inspection.source,
            confidence_score: score_discovered_source(&inspection),
            found_files: inspection.found_files,
            path: candidate,
        });
    }

    sources.sort_by(|left, right| {
        right
            .confidence_score
            .cmp(&left.confidence_score)
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(DiscoveryReport { sources })
}

pub fn plan_import_sources(report: &DiscoveryReport) -> CliResult<DiscoveryPlanSummary> {
    let mut plans = Vec::new();
    for source in &report.sources {
        let plan = plan_import_from_path(&source.path, Some(source.source))?;
        plans.push(PlannedImportSource {
            source: source.source,
            source_id: source.source.as_id().to_owned(),
            input_path: source.path.clone(),
            confidence_score: source.confidence_score,
            prompt_addendum_present: plan.system_prompt_addendum.is_some(),
            profile_note_present: plan.profile_note.is_some(),
            warning_count: plan.warnings.len(),
        });
    }
    Ok(DiscoveryPlanSummary { plans })
}

pub fn recommend_primary_source(
    summary: &DiscoveryPlanSummary,
) -> CliResult<PrimarySourceRecommendation> {
    let Some(best) = summary.plans.iter().max_by(|left, right| {
        primary_recommendation_score(left)
            .cmp(&primary_recommendation_score(right))
            .then_with(|| left.input_path.cmp(&right.input_path))
    }) else {
        return Err("cannot recommend primary source from an empty plan summary".to_owned());
    };

    let mut reasons = Vec::new();
    reasons.push(format!("confidence score {}", best.confidence_score));
    if best.prompt_addendum_present {
        reasons.push("contains imported prompt overlay".to_owned());
    }
    if best.profile_note_present {
        reasons.push("contains imported profile overlay".to_owned());
    }
    if best.warning_count == 0 {
        reasons.push("has no import warnings".to_owned());
    }

    Ok(PrimarySourceRecommendation {
        source: best.source,
        source_id: best.source_id.clone(),
        input_path: best.input_path.clone(),
        reasons,
    })
}

fn collect_candidate_directories(
    search_root: &Path,
    options: &DiscoveryOptions,
) -> CliResult<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    push_candidate(&mut candidates, &mut seen, search_root.to_path_buf());

    if options.include_child_directories && search_root.is_dir() {
        let entries = fs::read_dir(search_root).map_err(|error| {
            format!(
                "failed to read discovery root {}: {error}",
                search_root.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to enumerate discovery root {}: {error}",
                    search_root.display()
                )
            })?;
            let path = entry.path();
            if path.is_dir() {
                push_candidate(&mut candidates, &mut seen, path);
            }
        }
    }

    Ok(candidates)
}

fn push_candidate(candidates: &mut Vec<PathBuf>, seen: &mut BTreeSet<String>, path: PathBuf) {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone())
        .display()
        .to_string();
    if seen.insert(canonical) {
        candidates.push(path);
    }
}

fn score_discovered_source(inspection: &super::ImportPathInspection) -> u32 {
    let mut score = 0u32;
    if inspection.source != LegacyClawSource::Unknown {
        score = score.saturating_add(10);
    }
    score = score.saturating_add(inspection.custom_prompt_files as u32 * 12);
    score = score.saturating_add(inspection.custom_profile_files as u32 * 12);
    score = score.saturating_add(inspection.warning_count as u32 * 3);
    score = score.saturating_add(inspection.found_files.len() as u32);
    score
}

fn primary_recommendation_score(plan: &PlannedImportSource) -> u32 {
    let mut score = plan.confidence_score;
    if plan.prompt_addendum_present {
        score = score.saturating_add(10);
    }
    if plan.profile_note_present {
        score = score.saturating_add(10);
    }
    if plan.warning_count == 0 {
        score = score.saturating_add(3);
    }
    score.saturating_sub(plan.warning_count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn discover_import_sources_returns_ranked_candidates_from_fixture_root() {
        let root = unique_temp_dir("loongclaw-import-discovery-ranked");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "SOUL.md",
            "# Soul\n\nAlways prefer brief shell output.\n",
        );

        let report = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        assert_eq!(report.sources.len(), 2);
        assert_eq!(report.sources[0].source.as_id(), "openclaw");
        assert!(
            report.sources[0].confidence_score >= report.sources[1].confidence_score,
            "expected descending confidence scores"
        );
        assert!(report.sources[0]
            .found_files
            .iter()
            .any(|value| value == "SOUL.md"));
        assert!(report.sources[0]
            .found_files
            .iter()
            .any(|value| value == "IDENTITY.md"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn discover_import_sources_ignores_empty_or_stock_only_noise_directories() {
        let root = unique_temp_dir("loongclaw-import-discovery-noise");
        fs::create_dir_all(&root).expect("create fixture root");

        let empty_root = root.join("empty");
        fs::create_dir_all(&empty_root).expect("create empty root");

        let stock_nanobot = root.join("stock-nanobot");
        fs::create_dir_all(&stock_nanobot).expect("create stock nanobot root");
        write_file(
            &stock_nanobot,
            "SOUL.md",
            "# Soul\n\nI am nanobot 🐈, a personal AI assistant.\n",
        );
        write_file(
            &stock_nanobot,
            "memory/MEMORY.md",
            "# Long-term Memory\n\n*This file is automatically updated by nanobot when important information should be remembered.*\n",
        );

        let report = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        assert!(
            report.sources.is_empty(),
            "noise-only roots should be ignored"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn plan_import_sources_returns_summary_for_each_candidate() {
        let root = unique_temp_dir("loongclaw-import-plan-many");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("plan-many should succeed");

        assert_eq!(summary.plans.len(), 2);
        assert_eq!(summary.plans[0].source_id, "openclaw");
        assert!(summary.plans[0].prompt_addendum_present);
        assert!(summary.plans[0].profile_note_present);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recommend_primary_source_prefers_richer_custom_source() {
        let root = unique_temp_dir("loongclaw-import-recommend-primary");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "SOUL.md",
            "# Soul\n\nAlways prefer brief shell output.\n",
        );

        let discovery = discover_import_sources(&root, DiscoveryOptions::default())
            .expect("discovery should succeed");
        let summary = plan_import_sources(&discovery).expect("plan-many should succeed");
        let recommendation =
            recommend_primary_source(&summary).expect("primary recommendation should succeed");

        assert_eq!(recommendation.source_id, "openclaw");
        assert_eq!(recommendation.source, LegacyClawSource::OpenClaw);
        assert!(
            !recommendation.reasons.is_empty(),
            "recommendation reasons should be populated"
        );

        fs::remove_dir_all(&root).ok();
    }
}
