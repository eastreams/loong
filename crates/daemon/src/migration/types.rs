use std::collections::BTreeMap;

use loong_app as mvp;
use serde::Serialize;

const RECOMMENDED_SOURCE_SELECTORS: &[&str] =
    &["recommended", "recommended_plan", "composed", "plan"];
const CURRENT_SOURCE_SELECTORS: &[&str] = &["current", "current_setup"];
const EXISTING_SOURCE_SELECTORS: &[&str] = &["existing", "existing_config", "loong"];
const CODEX_SOURCE_SELECTORS: &[&str] = &["codex"];
const ENVIRONMENT_SOURCE_SELECTORS: &[&str] = &["env", "environment"];
const EXPLICIT_PATH_SOURCE_SELECTORS: &[&str] = &["path"];

const PROVIDER_DOMAIN_SELECTORS: &[&str] = &["provider"];
const CHANNELS_DOMAIN_SELECTORS: &[&str] = &["channels", "channel"];
const CLI_DOMAIN_SELECTORS: &[&str] = &["cli"];
const MEMORY_DOMAIN_SELECTORS: &[&str] = &["memory"];
const TOOLS_DOMAIN_SELECTORS: &[&str] = &["tools", "tooling"];
const WORKSPACE_GUIDANCE_DOMAIN_SELECTORS: &[&str] = &["workspace_guidance", "guidance"];

const IMPORT_CLI_SOURCE_SELECTORS: [ImportSourceKind; 4] = [
    ImportSourceKind::RecommendedPlan,
    ImportSourceKind::ExistingLoongConfig,
    ImportSourceKind::CodexConfig,
    ImportSourceKind::Environment,
];

const SETUP_DOMAIN_SELECTORS: [SetupDomainKind; 6] = [
    SetupDomainKind::Provider,
    SetupDomainKind::Channels,
    SetupDomainKind::Cli,
    SetupDomainKind::Memory,
    SetupDomainKind::Tools,
    SetupDomainKind::WorkspaceGuidance,
];

#[derive(Debug, Clone, Copy)]
struct ImportSourceDescriptor {
    primary_selector: &'static str,
    selectors: &'static [&'static str],
    direct_starting_point_rank: usize,
    default_domain_decision: Option<PreviewDecision>,
    direct_starting_point_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct SetupDomainDescriptor {
    label: &'static str,
    primary_selector: &'static str,
    selectors: &'static [&'static str],
    changes_config: bool,
    keep_current_reason: Option<&'static str>,
    use_detected_reason: Option<&'static str>,
    supplement_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
struct PreviewStatusDescriptor {
    label: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct PreviewDecisionDescriptor {
    label: &'static str,
    outcome_label: &'static str,
    outcome_rank: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSourceKind {
    RecommendedPlan,
    CurrentSetup,
    ExistingLoongConfig,
    CodexConfig,
    Environment,
    #[allow(dead_code)]
    ExplicitPath,
}

impl ImportSourceKind {
    pub fn parse_import_cli_selector(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
        IMPORT_CLI_SOURCE_SELECTORS
            .iter()
            .copied()
            .find(|kind| kind.matches_selector(&normalized))
    }

    pub fn supported_import_cli_selector_list() -> String {
        IMPORT_CLI_SOURCE_SELECTORS
            .iter()
            .map(|kind| kind.primary_selector())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub const fn import_cli_selector(self) -> &'static str {
        self.primary_selector()
    }

    pub const fn direct_starting_point_rank(self) -> usize {
        self.descriptor().direct_starting_point_rank
    }

    pub fn onboarding_label(source_kind: Option<Self>, source: &str) -> String {
        crate::source_presentation::onboarding_source_label(source_kind, source)
    }

    pub const fn default_domain_decision(self) -> Option<PreviewDecision> {
        self.descriptor().default_domain_decision
    }

    pub const fn direct_starting_point_reason(self) -> Option<&'static str> {
        self.descriptor().direct_starting_point_reason
    }

    const fn primary_selector(self) -> &'static str {
        self.descriptor().primary_selector
    }

    const fn descriptor(self) -> ImportSourceDescriptor {
        match self {
            ImportSourceKind::RecommendedPlan => ImportSourceDescriptor {
                primary_selector: "recommended",
                selectors: RECOMMENDED_SOURCE_SELECTORS,
                direct_starting_point_rank: 0,
                default_domain_decision: Some(PreviewDecision::UseDetected),
                direct_starting_point_reason: None,
            },
            ImportSourceKind::CurrentSetup => ImportSourceDescriptor {
                primary_selector: "current",
                selectors: CURRENT_SOURCE_SELECTORS,
                direct_starting_point_rank: 0,
                default_domain_decision: None,
                direct_starting_point_reason: Some("keep your current Loong setup"),
            },
            ImportSourceKind::ExistingLoongConfig => ImportSourceDescriptor {
                primary_selector: "existing",
                selectors: EXISTING_SOURCE_SELECTORS,
                direct_starting_point_rank: 0,
                default_domain_decision: Some(PreviewDecision::KeepCurrent),
                direct_starting_point_reason: Some("keep your current Loong setup"),
            },
            ImportSourceKind::CodexConfig => ImportSourceDescriptor {
                primary_selector: "codex",
                selectors: CODEX_SOURCE_SELECTORS,
                direct_starting_point_rank: 2,
                default_domain_decision: Some(PreviewDecision::UseDetected),
                direct_starting_point_reason: Some("reuse Codex config as your starting point"),
            },
            ImportSourceKind::Environment => ImportSourceDescriptor {
                primary_selector: "env",
                selectors: ENVIRONMENT_SOURCE_SELECTORS,
                direct_starting_point_rank: 3,
                default_domain_decision: Some(PreviewDecision::UseDetected),
                direct_starting_point_reason: Some("start from detected environment settings"),
            },
            ImportSourceKind::ExplicitPath => ImportSourceDescriptor {
                primary_selector: "path",
                selectors: EXPLICIT_PATH_SOURCE_SELECTORS,
                direct_starting_point_rank: 1,
                default_domain_decision: Some(PreviewDecision::UseDetected),
                direct_starting_point_reason: Some(
                    "reuse the selected config file as your starting point",
                ),
            },
        }
    }

    fn matches_selector(self, normalized: &str) -> bool {
        self.descriptor().selectors.contains(&normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupDomainKind {
    Provider,
    Channels,
    Cli,
    Memory,
    Tools,
    WorkspaceGuidance,
}

impl SetupDomainKind {
    pub fn parse_selector(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
        SETUP_DOMAIN_SELECTORS
            .iter()
            .copied()
            .find(|kind| kind.matches_selector(&normalized))
    }

    pub fn supported_selector_list() -> String {
        SETUP_DOMAIN_SELECTORS
            .iter()
            .map(|kind| kind.primary_selector())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub const fn label(self) -> &'static str {
        self.descriptor().label
    }

    pub const fn changes_config(self) -> bool {
        self.descriptor().changes_config
    }

    pub const fn starting_point_reason(self, decision: PreviewDecision) -> Option<&'static str> {
        let descriptor = self.descriptor();
        match decision {
            PreviewDecision::KeepCurrent => descriptor.keep_current_reason,
            PreviewDecision::UseDetected => descriptor.use_detected_reason,
            PreviewDecision::Supplement => descriptor.supplement_reason,
            PreviewDecision::ReviewConflict | PreviewDecision::AdjustedInSession => None,
        }
    }

    const fn primary_selector(self) -> &'static str {
        self.descriptor().primary_selector
    }

    const fn descriptor(self) -> SetupDomainDescriptor {
        match self {
            SetupDomainKind::Provider => SetupDomainDescriptor {
                label: "provider",
                primary_selector: "provider",
                selectors: PROVIDER_DOMAIN_SELECTORS,
                changes_config: true,
                keep_current_reason: Some("keep current provider"),
                use_detected_reason: Some("use detected provider"),
                supplement_reason: None,
            },
            SetupDomainKind::Channels => SetupDomainDescriptor {
                label: "channels",
                primary_selector: "channels",
                selectors: CHANNELS_DOMAIN_SELECTORS,
                changes_config: true,
                keep_current_reason: None,
                use_detected_reason: Some("use detected channels"),
                supplement_reason: Some("add detected channels"),
            },
            SetupDomainKind::Cli => SetupDomainDescriptor {
                label: "cli",
                primary_selector: "cli",
                selectors: CLI_DOMAIN_SELECTORS,
                changes_config: true,
                keep_current_reason: None,
                use_detected_reason: Some("reuse detected cli behavior"),
                supplement_reason: Some("reuse detected cli behavior"),
            },
            SetupDomainKind::Memory => SetupDomainDescriptor {
                label: "memory",
                primary_selector: "memory",
                selectors: MEMORY_DOMAIN_SELECTORS,
                changes_config: true,
                keep_current_reason: None,
                use_detected_reason: Some("reuse detected memory settings"),
                supplement_reason: Some("reuse detected memory settings"),
            },
            SetupDomainKind::Tools => SetupDomainDescriptor {
                label: "tools",
                primary_selector: "tools",
                selectors: TOOLS_DOMAIN_SELECTORS,
                changes_config: true,
                keep_current_reason: None,
                use_detected_reason: Some("reuse detected tool settings"),
                supplement_reason: Some("reuse detected tool settings"),
            },
            SetupDomainKind::WorkspaceGuidance => SetupDomainDescriptor {
                label: "workspace guidance",
                primary_selector: "workspace_guidance",
                selectors: WORKSPACE_GUIDANCE_DOMAIN_SELECTORS,
                changes_config: false,
                keep_current_reason: None,
                use_detected_reason: Some("reuse workspace guidance"),
                supplement_reason: Some("reuse workspace guidance"),
            },
        }
    }

    fn matches_selector(self, normalized: &str) -> bool {
        self.descriptor().selectors.contains(&normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    Ready,
    NeedsReview,
    Unavailable,
}

impl PreviewStatus {
    pub const fn label(self) -> &'static str {
        self.descriptor().label
    }

    const fn descriptor(self) -> PreviewStatusDescriptor {
        match self {
            PreviewStatus::Ready => PreviewStatusDescriptor { label: "Ready" },
            PreviewStatus::NeedsReview => PreviewStatusDescriptor {
                label: "Needs review",
            },
            PreviewStatus::Unavailable => PreviewStatusDescriptor {
                label: "Unavailable",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewDecision {
    KeepCurrent,
    UseDetected,
    Supplement,
    ReviewConflict,
    AdjustedInSession,
}

impl PreviewDecision {
    pub const fn label(self) -> &'static str {
        self.descriptor().label
    }

    pub const fn outcome_label(self) -> &'static str {
        self.descriptor().outcome_label
    }

    pub const fn outcome_rank(self) -> u8 {
        self.descriptor().outcome_rank
    }

    const fn descriptor(self) -> PreviewDecisionDescriptor {
        match self {
            PreviewDecision::KeepCurrent => PreviewDecisionDescriptor {
                label: "keep current value",
                outcome_label: "kept current",
                outcome_rank: 3,
            },
            PreviewDecision::UseDetected => PreviewDecisionDescriptor {
                label: "use detected value",
                outcome_label: "used detected",
                outcome_rank: 4,
            },
            PreviewDecision::Supplement => PreviewDecisionDescriptor {
                label: "supplement with detected values",
                outcome_label: "supplemented",
                outcome_rank: 2,
            },
            PreviewDecision::ReviewConflict => PreviewDecisionDescriptor {
                label: "review conflicting choices",
                outcome_label: "left for review",
                outcome_rank: 0,
            },
            PreviewDecision::AdjustedInSession => PreviewDecisionDescriptor {
                label: "adjusted in this setup",
                outcome_label: "adjusted now",
                outcome_rank: 1,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSetupState {
    Absent,
    Healthy,
    Repairable,
    LegacyOrIncomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DomainPreview {
    pub kind: SetupDomainKind,
    pub status: PreviewStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<PreviewDecision>,
    pub source: String,
    pub summary: String,
}

pub type WorkspaceGuidanceKind = mvp::workspace_guidance::WorkspaceGuidanceKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceGuidanceCandidate {
    pub kind: WorkspaceGuidanceKind,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelCandidate {
    pub id: &'static str,
    pub label: &'static str,
    pub status: PreviewStatus,
    pub source: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSurfaceLevel {
    Ready,
    Review,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportSurface {
    pub name: &'static str,
    pub domain: SetupDomainKind,
    pub level: ImportSurfaceLevel,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    pub source_kind: ImportSourceKind,
    pub source: String,
    pub config: mvp::config::LoongConfig,
    pub surfaces: Vec<ImportSurface>,
    pub domains: Vec<DomainPreview>,
    pub channel_candidates: Vec<ChannelCandidate>,
    pub workspace_guidance: Vec<WorkspaceGuidanceCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCredentialState {
    #[default]
    Missing,
    Partial,
    Ready,
}

impl ChannelCredentialState {
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ChannelImportReadiness {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    channel_states: BTreeMap<String, ChannelCredentialState>,
}

impl ChannelImportReadiness {
    pub fn with_state(mut self, channel_id: &str, state: ChannelCredentialState) -> Self {
        self.set_state(channel_id, state);
        self
    }

    pub fn set_state(&mut self, channel_id: &str, state: ChannelCredentialState) {
        if state == ChannelCredentialState::Missing {
            self.channel_states.remove(channel_id);
        } else {
            self.channel_states.insert(channel_id.to_owned(), state);
        }
    }

    pub fn state(&self, channel_id: &str) -> ChannelCredentialState {
        self.channel_states
            .get(channel_id)
            .copied()
            .unwrap_or(ChannelCredentialState::Missing)
    }

    pub fn is_ready(&self, channel_id: &str) -> bool {
        self.state(channel_id).is_ready()
    }
}
