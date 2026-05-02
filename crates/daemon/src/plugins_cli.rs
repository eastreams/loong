use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use clap::{Args, Subcommand, ValueEnum};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::{
    CURRENT_PLUGIN_HOST_API, CURRENT_PLUGIN_MANIFEST_API_VERSION, Capability, ExecutionRoute,
    HarnessKind, PACKAGE_MANIFEST_FILE_NAME, PluginBridgeKind, PluginCompatibility, PluginManifest,
    VerticalPackManifest, plugin_runtime_scaffold_defaults,
};
use crate::{
    BridgeSupportSpec, CliResult, HumanApprovalMode, HumanApprovalSpec, JsonSchemaDescriptor,
    MaterializedBridgeSupportDeltaArtifact, OperationSpec, PluginInventoryResult,
    PluginPreflightBridgeProfileRecommendation, PluginPreflightProfile, PluginPreflightResult,
    PluginScanSpec, ResolvedBridgeSupportSelection, RunnerSpec, SecurityProfileSignatureSpec,
    SpecRunReport, default_plugin_inventory_limit, default_plugin_preflight_limit, execute_spec,
    json_schema_descriptor, materialize_bridge_support_delta_artifact,
    materialize_bridge_support_template, resolve_bridge_support_policy,
    resolve_bridge_support_selection,
};

pub const PLUGINS_COMMAND_SCHEMA_VERSION: u32 = 1;
pub const PLUGINS_COMMAND_SCHEMA_SURFACE: &str = "plugin_governance";
pub const PLUGINS_INVENTORY_SCHEMA_PURPOSE: &str = "package_inventory";
pub const PLUGINS_DOCTOR_SCHEMA_PURPOSE: &str = "package_doctor";
pub const PLUGINS_BRIDGE_PROFILES_SCHEMA_PURPOSE: &str = "bridge_profiles_catalog";
pub const PLUGINS_BRIDGE_TEMPLATE_SCHEMA_PURPOSE: &str = "bridge_support_materialization";
pub const PLUGINS_PREFLIGHT_SCHEMA_PURPOSE: &str = "ecosystem_preflight_evaluation";
pub const PLUGINS_ACTIONS_SCHEMA_PURPOSE: &str = "operator_action_plan";
pub const PLUGINS_INIT_SCHEMA_PURPOSE: &str = "package_scaffold";
pub const PLUGINS_INVOKE_EXTENSION_SCHEMA_PURPOSE: &str = "native_extension_smoke_probe";
pub const PLUGINS_INVOKE_HOST_HOOK_SCHEMA_PURPOSE: &str = "trusted_host_hook_probe";
pub const PLUGINS_INVOKE_TUI_SURFACE_SCHEMA_PURPOSE: &str = "trusted_host_tui_surface_probe";

fn plugins_command_schema(purpose: &str) -> JsonSchemaDescriptor {
    let version = PLUGINS_COMMAND_SCHEMA_VERSION;
    let surface = PLUGINS_COMMAND_SCHEMA_SURFACE;

    json_schema_descriptor(version, surface, purpose)
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum PluginsCommands {
    /// Scaffold a new manifest-first plugin package root for external authors
    Init(PluginInitCommand),
    /// Smoke-test a native process_stdio extension entrypoint through the governed bridge
    InvokeExtension(PluginInvokeExtensionCommand),
    /// Probe a declared trusted-host hook through the bounded process bridge
    InvokeHostHook(PluginInvokeHostHookCommand),
    /// Probe a declared trusted-host TUI surface through the bounded process bridge
    InvokeTuiSurface(PluginInvokeTuiSurfaceCommand),
    /// Inspect manifest-first package truth across one or more plugin roots
    Inventory(PluginInventoryCommand),
    /// Diagnose manifest-first plugin packages with author-facing remediation
    Doctor(PluginDoctorCommand),
    /// List bundled bridge support profiles for controlled ecosystem compatibility
    BridgeProfiles(PluginBridgeProfilesCommand),
    /// Emit the effective recommended bridge support profile template for the scanned ecosystem
    BridgeTemplate(PluginBridgeTemplateCommand),
    /// Run profile-aware plugin preflight across one or more scan roots
    Preflight(PluginPreflightCommand),
    /// Print the deduplicated operator action plan derived from plugin preflight
    Actions(PluginActionsCommand),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginScanSourceArgs {
    /// Scan root to inspect for plugins. Repeat the flag for multiple roots.
    #[arg(long = "root", required = true, value_name = "ROOT")]
    pub roots: Vec<String>,
    /// Filter plugins by query before evaluating preflight
    #[arg(long, default_value = "")]
    pub query: String,
    /// Maximum number of plugins to return
    #[arg(long)]
    pub limit: Option<usize>,
    /// Optional JSON file containing a bridge support policy
    #[arg(long, conflicts_with = "bridge_profile")]
    pub bridge_support: Option<String>,
    /// Optional bundled bridge support profile for controlled ecosystem compatibility
    #[arg(long, value_enum, conflicts_with = "bridge_support")]
    pub bridge_profile: Option<PluginBridgeProfileArg>,
    /// Optional delta artifact JSON file derived from a bundled bridge support profile
    #[arg(long, conflicts_with_all = ["bridge_support", "bridge_profile"])]
    pub bridge_support_delta: Option<String>,
    /// Optional sha256 pin for the resolved bridge support policy
    #[arg(long)]
    pub bridge_support_sha256: Option<String>,
    /// Optional sha256 pin for the bridge support delta artifact
    #[arg(long)]
    pub bridge_support_delta_sha256: Option<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginGovernanceSourceArgs {
    #[command(flatten)]
    pub scan: PluginScanSourceArgs,
    /// Active governance profile to evaluate
    #[arg(long, value_enum, default_value_t = PluginPreflightProfileArg::RuntimeActivation)]
    pub profile: PluginPreflightProfileArg,
    /// Optional plugin preflight policy JSON file
    #[arg(long)]
    pub policy_path: Option<String>,
    /// Optional sha256 pin for the plugin preflight policy file
    #[arg(long)]
    pub policy_sha256: Option<String>,
    /// Optional base64-encoded public key for plugin preflight policy signature verification
    #[arg(long)]
    pub policy_signature_public_key_base64: Option<String>,
    /// Optional base64-encoded signature for plugin preflight policy verification
    #[arg(long)]
    pub policy_signature_base64: Option<String>,
    /// Signature algorithm for the provided policy signature
    #[arg(long, default_value = "ed25519")]
    pub policy_signature_algorithm: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginInventoryCommand {
    #[command(flatten)]
    pub source: PluginScanSourceArgs,
    /// Include ready or setup-incomplete plugins in the inventory results
    #[arg(long, default_value_t = true)]
    pub include_ready: bool,
    /// Include blocked plugins in the inventory results
    #[arg(long, default_value_t = true)]
    pub include_blocked: bool,
    /// Include deferred plugins in the inventory results
    #[arg(long, default_value_t = true)]
    pub include_deferred: bool,
    /// Include input/output examples in inventory result rows
    #[arg(long, default_value_t = false)]
    pub include_examples: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginDoctorSourceArgs {
    #[command(flatten)]
    pub scan: PluginScanSourceArgs,
    /// Author-facing governance profile to evaluate
    #[arg(long, value_enum, default_value_t = PluginPreflightProfileArg::SdkRelease)]
    pub profile: PluginPreflightProfileArg,
    /// Optional plugin preflight policy JSON file
    #[arg(long)]
    pub policy_path: Option<String>,
    /// Optional sha256 pin for the plugin preflight policy file
    #[arg(long)]
    pub policy_sha256: Option<String>,
    /// Optional base64-encoded public key for plugin preflight policy signature verification
    #[arg(long)]
    pub policy_signature_public_key_base64: Option<String>,
    /// Optional base64-encoded signature for plugin preflight policy verification
    #[arg(long)]
    pub policy_signature_base64: Option<String>,
    /// Signature algorithm for the provided policy signature
    #[arg(long, default_value = "ed25519")]
    pub policy_signature_algorithm: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
#[command(
    about = "Diagnose manifest-first plugin packages and show author-facing remediation",
    long_about = "Diagnose manifest-first plugin packages and show author-facing remediation.\n\nThis command reuses the shared spec `plugin_preflight` surface, but defaults to the `sdk_release` profile and renders package-author truth first: setup contract, activation status, diagnostics, remediation classes, and required operator actions. Use `--profile runtime-activation` or `--profile marketplace-submission` when you need host-specific or stricter ecosystem review."
)]
pub struct PluginDoctorCommand {
    #[command(flatten)]
    pub source: PluginDoctorSourceArgs,
    /// Include plugins that pass the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_passed: bool,
    /// Include plugins that warn under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_warned: bool,
    /// Include plugins that block under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_blocked: bool,
    /// Include deferred plugins in the doctor scan
    #[arg(long, default_value_t = true)]
    pub include_deferred: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginPreflightCommand {
    #[command(flatten)]
    pub source: PluginGovernanceSourceArgs,
    /// Include plugins that pass the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_passed: bool,
    /// Include plugins that warn under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_warned: bool,
    /// Include plugins that block under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_blocked: bool,
    /// Include deferred plugins in the preflight scan
    #[arg(long, default_value_t = true)]
    pub include_deferred: bool,
    /// Include input/output examples in preflight result rows
    #[arg(long, default_value_t = false)]
    pub include_examples: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginBridgeProfilesCommand {
    /// Restrict output to one or more bundled bridge support profiles
    #[arg(long = "profile", value_enum, value_name = "PROFILE")]
    pub profiles: Vec<PluginBridgeProfileArg>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginBridgeTemplateCommand {
    #[command(flatten)]
    pub source: PluginGovernanceSourceArgs,
    /// Include plugins that pass the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_passed: bool,
    /// Include plugins that warn under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_warned: bool,
    /// Include plugins that block under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_blocked: bool,
    /// Include deferred plugins in the preflight scan
    #[arg(long, default_value_t = true)]
    pub include_deferred: bool,
    /// Optionally write the emitted bridge support template JSON to a file
    #[arg(long)]
    pub output: Option<String>,
    /// Optionally write the emitted minimal bridge support delta artifact JSON to a file
    #[arg(long)]
    pub delta_output: Option<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct PluginActionsCommand {
    #[command(flatten)]
    pub source: PluginGovernanceSourceArgs,
    /// Include plugins that pass the selected governance profile
    #[arg(long, default_value_t = false)]
    pub include_passed: bool,
    /// Include plugins that warn under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_warned: bool,
    /// Include plugins that block under the selected governance profile
    #[arg(long, default_value_t = true)]
    pub include_blocked: bool,
    /// Include deferred plugins in the preflight scan
    #[arg(long, default_value_t = true)]
    pub include_deferred: bool,
    /// Restrict returned actions to one or more owning surfaces
    #[arg(long, value_enum)]
    pub surface: Vec<PluginActionSurfaceArg>,
    /// Restrict returned actions to one or more action kinds
    #[arg(long, value_enum)]
    pub kind: Vec<PluginActionKindArg>,
    /// Restrict returned actions by reload requirement
    #[arg(long)]
    pub requires_reload: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum PluginInitBridgeKindArg {
    HttpJson,
    ProcessStdio,
    NativeFfi,
    WasmComponent,
    McpServer,
    AcpBridge,
    AcpRuntime,
}

impl PluginInitBridgeKindArg {
    fn as_bridge_kind(self) -> PluginBridgeKind {
        match self {
            Self::HttpJson => PluginBridgeKind::HttpJson,
            Self::ProcessStdio => PluginBridgeKind::ProcessStdio,
            Self::NativeFfi => PluginBridgeKind::NativeFfi,
            Self::WasmComponent => PluginBridgeKind::WasmComponent,
            Self::McpServer => PluginBridgeKind::McpServer,
            Self::AcpBridge => PluginBridgeKind::AcpBridge,
            Self::AcpRuntime => PluginBridgeKind::AcpRuntime,
        }
    }
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
#[command(
    about = "Scaffold a manifest-first plugin package root for external authors",
    long_about = "Scaffold a manifest-first plugin package root for external authors.\n\nThe generated package contains a canonical `loong.plugin.json` plus a README that points authors to `loong plugins doctor` and `loong plugins actions` for shared governance validation. Use `--host-hook` and `--tui-surface` to scaffold the bounded trusted host lane on top of the same process_stdio runtime path. This command scaffolds package metadata and local runtime stubs only; it does not widen trust policy by itself."
)]
pub struct PluginInitCommand {
    /// Target package root to create or reuse when the directory is empty
    #[arg(value_name = "PACKAGE_ROOT")]
    pub package_root: String,
    /// Stable plugin identity used by governance, inventory, and audit surfaces
    #[arg(long)]
    pub plugin_id: String,
    /// Optional provider id override; defaults to plugin_id
    #[arg(long)]
    pub provider_id: Option<String>,
    /// Optional connector name override; defaults to plugin_id
    #[arg(long)]
    pub connector_name: Option<String>,
    /// Runtime bridge surface declared by the plugin package
    #[arg(long, value_enum)]
    pub bridge_kind: PluginInitBridgeKindArg,
    /// Source language for language-specific bridges such as process_stdio or native_ffi
    #[arg(long)]
    pub source_language: Option<String>,
    /// Additional declared capability names beyond the default connector baseline
    #[arg(long = "capability")]
    pub capabilities: Vec<String>,
    /// Declared read-only trusted host hook names to scaffold on the trusted host lane
    #[arg(long = "host-hook")]
    pub host_hooks: Vec<String>,
    /// Declared trusted-host TUI surfaces to scaffold on the trusted host lane
    #[arg(long = "tui-surface")]
    pub tui_surfaces: Vec<String>,
    /// Initial package version written to the manifest
    #[arg(long, default_value = "0.1.0")]
    pub version: String,
    /// Optional one-line summary written to the manifest
    #[arg(long)]
    pub summary: Option<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
#[command(
    about = "Smoke-test a native process_stdio extension entrypoint through the governed bridge",
    long_about = "Smoke-test a native process_stdio extension entrypoint through the governed bridge.\n\nThis command scans a package root, selects the named plugin package, and invokes one host-facing extension method through the same bounded process bridge used by runtime execution. It is intended for external authoring and local validation, not for widening trust policy."
)]
pub struct PluginInvokeExtensionCommand {
    #[arg(long = "root", value_name = "ROOT")]
    pub root: String,
    #[arg(long)]
    pub plugin_id: String,
    #[arg(long)]
    pub method: String,
    #[arg(long, default_value = "{}")]
    pub payload: String,
    #[arg(long = "allow-command")]
    pub allow_commands: Vec<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
#[command(
    about = "Probe a declared trusted-host hook through the bounded process bridge",
    long_about = "Probe a declared trusted-host hook through the bounded process bridge.\n\nThis command scans a package root, selects the named plugin package, verifies that it declares the trusted host extension family and trust lane, and invokes the hook through the existing process_stdio bridge with a read-only host-hook envelope. It is a bounded authoring probe, not an automatic host runtime."
)]
pub struct PluginInvokeHostHookCommand {
    #[arg(long = "root", value_name = "ROOT")]
    pub root: String,
    #[arg(long)]
    pub plugin_id: String,
    #[arg(long)]
    pub hook: String,
    #[arg(long, default_value = "{}")]
    pub payload: String,
    #[arg(long = "allow-command")]
    pub allow_commands: Vec<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
#[command(
    about = "Probe a declared trusted-host TUI surface through the bounded process bridge",
    long_about = "Probe a declared trusted-host TUI surface through the bounded process bridge.\n\nThis command scans a package root, selects the named plugin package, verifies that it declares the trusted host extension family and trust lane plus the named shell-first TUI surface, and invokes the surface through the existing process_stdio bridge with a read-only TUI envelope. It is a bounded authoring probe, not live shell dispatch."
)]
pub struct PluginInvokeTuiSurfaceCommand {
    #[arg(long = "root", value_name = "ROOT")]
    pub root: String,
    #[arg(long)]
    pub plugin_id: String,
    #[arg(long = "tui-surface")]
    pub tui_surface: String,
    #[arg(long, default_value = "{}")]
    pub payload: String,
    #[arg(long = "allow-command")]
    pub allow_commands: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PluginsCommandOptions {
    pub json: bool,
    pub command: PluginsCommands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginPreflightProfileArg {
    RuntimeActivation,
    SdkRelease,
    MarketplaceSubmission,
}

impl PluginPreflightProfileArg {
    fn as_profile(self) -> PluginPreflightProfile {
        match self {
            Self::RuntimeActivation => PluginPreflightProfile::RuntimeActivation,
            Self::SdkRelease => PluginPreflightProfile::SdkRelease,
            Self::MarketplaceSubmission => PluginPreflightProfile::MarketplaceSubmission,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginBridgeProfileArg {
    NativeBalanced,
    OpenclawEcosystemBalanced,
}

impl PluginBridgeProfileArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::NativeBalanced => "native-balanced",
            Self::OpenclawEcosystemBalanced => "openclaw-ecosystem-balanced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginActionSurfaceArg {
    HostRuntime,
    BridgePolicy,
    PluginPackage,
    OperatorReview,
}

impl PluginActionSurfaceArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::HostRuntime => "host_runtime",
            Self::BridgePolicy => "bridge_policy",
            Self::PluginPackage => "plugin_package",
            Self::OperatorReview => "operator_review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginActionKindArg {
    QuarantineLoadedProvider,
    ReabsorbPlugin,
    UpdateBridgeSupportPolicy,
    UpdatePluginPackage,
    ResolveSlotOwnership,
    ReviewDiagnostics,
}

impl PluginActionKindArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::QuarantineLoadedProvider => "quarantine_loaded_provider",
            Self::ReabsorbPlugin => "reabsorb_plugin",
            Self::UpdateBridgeSupportPolicy => "update_bridge_support_policy",
            Self::UpdatePluginPackage => "update_plugin_package",
            Self::ResolveSlotOwnership => "resolve_slot_ownership",
            Self::ReviewDiagnostics => "review_diagnostics",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsActionView {
    pub action_id: String,
    pub surface: String,
    pub kind: String,
    pub target_plugin_id: String,
    pub target_provider_id: Option<String>,
    pub target_source_path: String,
    pub target_manifest_path: Option<String>,
    pub follow_up_profile: Option<String>,
    pub requires_reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsActionSupportView {
    pub remediation_class: String,
    pub diagnostic_code: Option<String>,
    pub field_path: Option<String>,
    pub blocking: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsActionPlanItemView {
    pub action: PluginsActionView,
    pub supporting_results: usize,
    pub blocked_results: usize,
    pub warned_results: usize,
    pub passed_results: usize,
    pub supporting_remediations: Vec<PluginsActionSupportView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsBridgeProfileFitView {
    pub profile_id: String,
    pub source: String,
    pub policy_version: Option<String>,
    pub checksum: String,
    pub sha256: String,
    pub fits_all_plugins: bool,
    pub supported_plugins: usize,
    pub blocked_plugins: usize,
    pub blocking_reasons: BTreeMap<String, usize>,
    pub sample_blocked_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsBridgeShimProfileDeltaView {
    pub shim_id: String,
    pub shim_family: String,
    pub supported_dialects: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_source_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsBridgeProfileDeltaView {
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_compatibility_modes: Vec<String>,
    pub supported_compatibility_shims: Vec<String>,
    pub shim_profile_additions: Vec<PluginsBridgeShimProfileDeltaView>,
    pub unresolved_blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsBridgeProfileRecommendationView {
    pub kind: String,
    pub target_profile_id: String,
    pub target_profile_source: String,
    pub target_policy_version: Option<String>,
    pub summary: String,
    pub delta: Option<PluginsBridgeProfileDeltaView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsBridgeSupportProvenanceView {
    pub source: Option<String>,
    pub sha256: Option<String>,
    pub delta_source: Option<String>,
    pub delta_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsInventorySummaryView {
    pub returned_plugins: usize,
    pub ready_plugins: usize,
    pub setup_incomplete_plugins: usize,
    pub blocked_plugins: usize,
    pub deferred_plugins: usize,
    pub loaded_plugins: usize,
    pub source_kind_distribution: BTreeMap<String, usize>,
    pub bridge_kind_distribution: BTreeMap<String, usize>,
    pub capability_distribution: BTreeMap<String, usize>,
    pub source_language_distribution: BTreeMap<String, usize>,
    pub setup_surface_distribution: BTreeMap<String, usize>,
    pub activation_status_distribution: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsInventoryExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub scan_roots: Vec<String>,
    pub query: String,
    pub limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub bridge_support_source: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub summary: PluginsInventorySummaryView,
    pub returned_results: usize,
    pub results: Vec<PluginInventoryResult>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsDoctorSummaryView {
    pub matched_plugins: usize,
    pub returned_plugins: usize,
    pub passed_plugins: usize,
    pub warned_plugins: usize,
    pub blocked_plugins: usize,
    pub activation_ready_plugins: usize,
    pub setup_incomplete_plugins: usize,
    pub deferred_plugins: usize,
    pub loaded_plugins: usize,
    pub packages_requiring_author_attention: usize,
    pub packages_with_operator_actions: usize,
    pub total_recommended_actions: usize,
    pub total_operator_actions: usize,
    pub remediation_counts: BTreeMap<String, usize>,
    pub bridge_kind_distribution: BTreeMap<String, usize>,
    pub capability_distribution: BTreeMap<String, usize>,
    pub source_language_distribution: BTreeMap<String, usize>,
    pub setup_surface_distribution: BTreeMap<String, usize>,
    pub activation_status_distribution: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsDoctorExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub scan_roots: Vec<String>,
    pub query: String,
    pub limit: usize,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub bridge_support_source: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub summary: PluginsDoctorSummaryView,
    pub preflight_summary: PluginsPreflightSummaryView,
    pub returned_results: usize,
    pub results: Vec<PluginPreflightResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsPreflightSummaryView {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub profile: String,
    pub policy_source: String,
    pub policy_version: Option<String>,
    pub policy_checksum: String,
    pub policy_sha256: String,
    pub matched_plugins: usize,
    pub returned_plugins: usize,
    pub truncated: bool,
    pub passed_plugins: usize,
    pub warned_plugins: usize,
    pub blocked_plugins: usize,
    pub total_diagnostics: usize,
    pub blocking_diagnostics: usize,
    pub error_diagnostics: usize,
    pub warning_diagnostics: usize,
    pub info_diagnostics: usize,
    pub remediation_counts: BTreeMap<String, usize>,
    pub source_kind_distribution: BTreeMap<String, usize>,
    pub dialect_distribution: BTreeMap<String, usize>,
    pub compatibility_mode_distribution: BTreeMap<String, usize>,
    pub bridge_kind_distribution: BTreeMap<String, usize>,
    pub source_language_distribution: BTreeMap<String, usize>,
    pub operator_action_plan: Vec<PluginsActionPlanItemView>,
    pub operator_action_counts_by_surface: BTreeMap<String, usize>,
    pub operator_action_counts_by_kind: BTreeMap<String, usize>,
    pub operator_actions_requiring_reload: usize,
    pub operator_actions_without_reload: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub active_bridge_profile: Option<String>,
    pub recommended_bridge_profile: Option<String>,
    pub recommended_bridge_profile_source: Option<String>,
    pub active_bridge_profile_matches_recommended: Option<bool>,
    pub active_bridge_support_fits_all_plugins: Option<bool>,
    pub bridge_profile_fits: Vec<PluginsBridgeProfileFitView>,
    pub bridge_profile_recommendation: Option<PluginsBridgeProfileRecommendationView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginActionFiltersView {
    pub surface: Vec<String>,
    pub kind: Vec<String>,
    pub requires_reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsPreflightExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub scan_roots: Vec<String>,
    pub query: String,
    pub limit: usize,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub bridge_support_source: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub summary: PluginsPreflightSummaryView,
    pub returned_results: usize,
    pub results: Vec<PluginPreflightResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsActionsExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub scan_roots: Vec<String>,
    pub query: String,
    pub limit: usize,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub bridge_support_source: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub filters: PluginActionFiltersView,
    pub summary: PluginsPreflightSummaryView,
    pub total_actions: usize,
    pub matched_actions: usize,
    pub filtered_action_counts_by_surface: BTreeMap<String, usize>,
    pub filtered_action_counts_by_kind: BTreeMap<String, usize>,
    pub filtered_actions_requiring_reload: usize,
    pub filtered_actions_without_reload: usize,
    pub actions: Vec<PluginsActionPlanItemView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsBridgeShimSupportProfileView {
    pub shim_id: String,
    pub shim_family: String,
    pub version: Option<String>,
    pub supported_dialects: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_source_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsBridgeProfileExecutionView {
    pub profile_id: String,
    pub source: String,
    pub policy_version: Option<String>,
    pub checksum: String,
    pub sha256: String,
    pub supported_bridges: Vec<String>,
    pub supported_compatibility_modes: Vec<String>,
    pub supported_compatibility_shims: Vec<String>,
    pub shim_support_profiles: Vec<PluginsBridgeShimSupportProfileView>,
    pub execute_process_stdio: bool,
    pub execute_http_json: bool,
    pub enforce_supported: bool,
    pub enforce_execution_success: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsBridgeProfilesExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub returned_profiles: usize,
    pub profiles: Vec<PluginsBridgeProfileExecutionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsBridgeTemplateExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub scan_roots: Vec<String>,
    pub query: String,
    pub limit: usize,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
    pub bridge_support_source: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub summary: PluginsPreflightSummaryView,
    pub template_kind: String,
    pub template_profile_id: String,
    pub template_source: String,
    pub template_checksum: String,
    pub template_sha256: String,
    pub template_policy_version: Option<String>,
    pub output_path: Option<String>,
    pub delta_output_path: Option<String>,
    pub delta_artifact: MaterializedBridgeSupportDeltaArtifact,
    pub template: BridgeSupportSpec,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginsInitExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub package_root: String,
    pub manifest_path: String,
    pub readme_path: String,
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub version: String,
    pub bridge_kind: String,
    pub source_language: Option<String>,
    pub adapter_family: String,
    pub entrypoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoke_test_command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_files_written: Vec<String>,
    pub files_written: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsInvokeExtensionExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub package_root: String,
    pub plugin_id: String,
    pub bridge_kind: String,
    pub source_language: Option<String>,
    pub method: String,
    pub payload: Value,
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsInvokeHostHookExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub package_root: String,
    pub plugin_id: String,
    pub extension_family: Option<String>,
    pub extension_trust_lane: Option<String>,
    pub bridge_kind: String,
    pub source_language: Option<String>,
    pub hook: String,
    pub payload: Value,
    pub dispatched_method: String,
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginsInvokeTuiSurfaceExecution {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub package_root: String,
    pub plugin_id: String,
    pub extension_family: Option<String>,
    pub extension_trust_lane: Option<String>,
    pub bridge_kind: String,
    pub source_language: Option<String>,
    pub tui_surface: String,
    pub payload: Value,
    pub dispatched_method: String,
    pub response_payload: Value,
    pub runtime_evidence: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum PluginsCommandExecution {
    Init(Box<PluginsInitExecution>),
    InvokeExtension(Box<PluginsInvokeExtensionExecution>),
    InvokeHostHook(Box<PluginsInvokeHostHookExecution>),
    InvokeTuiSurface(Box<PluginsInvokeTuiSurfaceExecution>),
    Inventory(Box<PluginsInventoryExecution>),
    Doctor(Box<PluginsDoctorExecution>),
    BridgeProfiles(Box<PluginsBridgeProfilesExecution>),
    BridgeTemplate(Box<PluginsBridgeTemplateExecution>),
    Preflight(Box<PluginsPreflightExecution>),
    Actions(Box<PluginsActionsExecution>),
}

pub async fn run_plugins_cli(options: PluginsCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_plugins_command(options).await?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&execution)
            .map_err(|error| format!("serialize plugins CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_plugins_cli_text(&execution));
    Ok(())
}

pub async fn execute_plugins_command(
    options: PluginsCommandOptions,
) -> CliResult<PluginsCommandExecution> {
    match options.command {
        PluginsCommands::Init(command) => {
            let execution = execute_plugins_init(command)?;
            Ok(PluginsCommandExecution::Init(Box::new(execution)))
        }
        PluginsCommands::InvokeExtension(command) => {
            let execution = execute_plugins_invoke_extension(command).await?;
            Ok(PluginsCommandExecution::InvokeExtension(Box::new(
                execution,
            )))
        }
        PluginsCommands::InvokeHostHook(command) => {
            let execution = execute_plugins_invoke_host_hook(command).await?;
            Ok(PluginsCommandExecution::InvokeHostHook(Box::new(execution)))
        }
        PluginsCommands::InvokeTuiSurface(command) => {
            let execution = execute_plugins_invoke_tui_surface(command).await?;
            Ok(PluginsCommandExecution::InvokeTuiSurface(Box::new(
                execution,
            )))
        }
        PluginsCommands::Inventory(command) => {
            let context = build_plugin_inventory_context(
                &command.source,
                command.include_ready,
                command.include_blocked,
                command.include_deferred,
                command.include_examples,
            )?;
            let report = execute_spec(&context.spec, false).await;
            if let Some(reason) = report.blocked_reason.as_deref() {
                return Err(format!("plugin inventory blocked: {reason}"));
            }
            let bridge_support_provenance = context.bridge_support_provenance();
            let mut results = decode_plugin_inventory_results(&report)?;
            for result in &mut results {
                populate_native_extension_authoring_guidance(result);
            }
            let summary = summarize_plugin_inventory_results(&results);

            Ok(PluginsCommandExecution::Inventory(Box::new(
                PluginsInventoryExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_INVENTORY_SCHEMA_PURPOSE),
                    scan_roots: context.scan_roots,
                    query: context.query,
                    limit: context.limit,
                    bridge_support_provenance,
                    bridge_support_source: context.bridge_support_source,
                    bridge_support_sha256: context.bridge_support_sha256,
                    bridge_support_delta_source: context.bridge_support_delta_source,
                    bridge_support_delta_sha256: context.bridge_support_delta_sha256,
                    returned_results: results.len(),
                    summary,
                    results,
                },
            )))
        }
        PluginsCommands::Doctor(command) => {
            let context = build_plugin_doctor_context(
                &command.source,
                command.include_passed,
                command.include_warned,
                command.include_blocked,
                command.include_deferred,
            )?;
            let report = execute_spec(&context.spec, false).await;
            if let Some(reason) = report.blocked_reason.as_deref() {
                return Err(format!("plugin doctor blocked: {reason}"));
            }
            let bridge_support_provenance = context.bridge_support_provenance();
            let preflight_summary =
                decode_preflight_summary(&report, bridge_support_provenance.clone())?;
            let mut results = decode_preflight_results(&report)?;
            for result in &mut results {
                populate_native_extension_authoring_guidance(&mut result.plugin);
            }
            let summary = summarize_plugin_doctor_results(&results, &preflight_summary);

            Ok(PluginsCommandExecution::Doctor(Box::new(
                PluginsDoctorExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_DOCTOR_SCHEMA_PURPOSE),
                    scan_roots: context.scan_roots,
                    query: context.query,
                    limit: context.limit,
                    profile: context.profile,
                    bridge_support_provenance,
                    bridge_support_source: context.bridge_support_source,
                    bridge_support_sha256: context.bridge_support_sha256,
                    bridge_support_delta_source: context.bridge_support_delta_source,
                    bridge_support_delta_sha256: context.bridge_support_delta_sha256,
                    summary,
                    preflight_summary,
                    returned_results: results.len(),
                    results,
                },
            )))
        }
        PluginsCommands::BridgeProfiles(command) => {
            let profiles = load_bridge_profile_views(&command.profiles)?;
            Ok(PluginsCommandExecution::BridgeProfiles(Box::new(
                PluginsBridgeProfilesExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_BRIDGE_PROFILES_SCHEMA_PURPOSE),
                    returned_profiles: profiles.len(),
                    profiles,
                },
            )))
        }
        PluginsCommands::BridgeTemplate(command) => {
            let context = build_plugin_preflight_context(
                &command.source,
                command.include_passed,
                command.include_warned,
                command.include_blocked,
                command.include_deferred,
                false,
            )?;
            let report = execute_spec(&context.spec, false).await;
            if let Some(reason) = report.blocked_reason.as_deref() {
                return Err(format!("plugin bridge template blocked: {reason}"));
            }
            let bridge_support_provenance = context.bridge_support_provenance();
            let summary = decode_preflight_summary(&report, bridge_support_provenance.clone())?;
            if summary.matched_plugins == 0 {
                return Err(
                    "plugins bridge-template requires at least one matched plugin".to_owned(),
                );
            }
            let recommendation = decode_preflight_bridge_profile_recommendation(&report)?;
            let (template_kind, template_profile_id, template_delta) =
                match recommendation.as_ref() {
                    Some(recommendation) => (
                        match recommendation.kind {
                            crate::PluginPreflightBridgeProfileRecommendationKind::AdoptBundledProfile => {
                                "recommended_bundled_profile"
                            }
                            crate::PluginPreflightBridgeProfileRecommendationKind::AuthorBridgeProfileDelta => {
                                "derived_custom_profile"
                            }
                        }
                        .to_owned(),
                        recommendation.target_profile_id.clone(),
                        recommendation.delta.as_ref(),
                    ),
                    None => {
                        let active_profile_id = summary
                            .active_bridge_profile
                            .clone()
                            .or_else(|| summary.recommended_bridge_profile.clone())
                            .ok_or_else(|| {
                                "plugins bridge-template could not resolve an active or recommended bridge profile"
                                    .to_owned()
                            })?;
                        ("active_aligned_profile".to_owned(), active_profile_id, None)
                    }
                };
            let template =
                materialize_bridge_support_template(template_profile_id.as_str(), template_delta)?;
            let delta_artifact = materialize_bridge_support_delta_artifact(
                template_profile_id.as_str(),
                template_delta,
            )?;
            if let Some(path) = command.output.as_deref() {
                write_bridge_support_template(path, &template.profile)?;
            }
            if let Some(path) = command.delta_output.as_deref() {
                write_bridge_support_delta_artifact(path, &delta_artifact)?;
            }

            Ok(PluginsCommandExecution::BridgeTemplate(Box::new(
                PluginsBridgeTemplateExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_BRIDGE_TEMPLATE_SCHEMA_PURPOSE),
                    scan_roots: context.scan_roots,
                    query: context.query,
                    limit: context.limit,
                    profile: context.profile,
                    bridge_support_provenance,
                    bridge_support_source: context.bridge_support_source,
                    bridge_support_sha256: context.bridge_support_sha256,
                    bridge_support_delta_source: context.bridge_support_delta_source,
                    bridge_support_delta_sha256: context.bridge_support_delta_sha256,
                    summary,
                    template_kind,
                    template_profile_id: template.base_profile_id,
                    template_source: template.source,
                    template_checksum: template.checksum,
                    template_sha256: template.sha256,
                    template_policy_version: template.profile.policy_version.clone(),
                    output_path: command.output,
                    delta_output_path: command.delta_output,
                    delta_artifact,
                    template: template.profile,
                },
            )))
        }
        PluginsCommands::Preflight(command) => {
            let context = build_plugin_preflight_context(
                &command.source,
                command.include_passed,
                command.include_warned,
                command.include_blocked,
                command.include_deferred,
                command.include_examples,
            )?;
            let report = execute_spec(&context.spec, false).await;
            if let Some(reason) = report.blocked_reason.as_deref() {
                return Err(format!("plugin governance preflight blocked: {reason}"));
            }
            let bridge_support_provenance = context.bridge_support_provenance();
            let summary = decode_preflight_summary(&report, bridge_support_provenance.clone())?;
            let results = decode_preflight_results(&report)?;
            Ok(PluginsCommandExecution::Preflight(Box::new(
                PluginsPreflightExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_PREFLIGHT_SCHEMA_PURPOSE),
                    scan_roots: context.scan_roots,
                    query: context.query,
                    limit: context.limit,
                    profile: context.profile,
                    bridge_support_provenance,
                    bridge_support_source: context.bridge_support_source,
                    bridge_support_sha256: context.bridge_support_sha256,
                    bridge_support_delta_source: context.bridge_support_delta_source,
                    bridge_support_delta_sha256: context.bridge_support_delta_sha256,
                    returned_results: results.len(),
                    summary,
                    results,
                },
            )))
        }
        PluginsCommands::Actions(command) => {
            let context = build_plugin_preflight_context(
                &command.source,
                command.include_passed,
                command.include_warned,
                command.include_blocked,
                command.include_deferred,
                false,
            )?;
            let report = execute_spec(&context.spec, false).await;
            if let Some(reason) = report.blocked_reason.as_deref() {
                return Err(format!("plugin governance actions blocked: {reason}"));
            }
            let bridge_support_provenance = context.bridge_support_provenance();
            let summary = decode_preflight_summary(&report, bridge_support_provenance.clone())?;
            let filters = PluginActionFiltersView {
                surface: command
                    .surface
                    .iter()
                    .map(|surface| surface.as_str().to_owned())
                    .collect(),
                kind: command
                    .kind
                    .iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
                requires_reload: command.requires_reload,
            };
            let filtered = summary
                .operator_action_plan
                .iter()
                .filter(|item| action_matches_filters(item, &filters))
                .cloned()
                .collect::<Vec<_>>();
            let (
                filtered_action_counts_by_surface,
                filtered_action_counts_by_kind,
                filtered_actions_requiring_reload,
                filtered_actions_without_reload,
            ) = summarize_filtered_actions(&filtered);

            Ok(PluginsCommandExecution::Actions(Box::new(
                PluginsActionsExecution {
                    schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
                    schema: plugins_command_schema(PLUGINS_ACTIONS_SCHEMA_PURPOSE),
                    scan_roots: context.scan_roots,
                    query: context.query,
                    limit: context.limit,
                    profile: context.profile,
                    bridge_support_provenance,
                    bridge_support_source: context.bridge_support_source,
                    bridge_support_sha256: context.bridge_support_sha256,
                    bridge_support_delta_source: context.bridge_support_delta_source,
                    bridge_support_delta_sha256: context.bridge_support_delta_sha256,
                    filters,
                    total_actions: summary.operator_action_plan.len(),
                    matched_actions: filtered.len(),
                    filtered_action_counts_by_surface,
                    filtered_action_counts_by_kind,
                    filtered_actions_requiring_reload,
                    filtered_actions_without_reload,
                    actions: filtered,
                    summary,
                },
            )))
        }
    }
}

const PLUGINS_INIT_README_FILE_NAME: &str = "README.md";

#[derive(Debug, Serialize)]
struct PluginPackageScaffoldManifestDocument {
    api_version: String,
    version: String,
    plugin_id: String,
    provider_id: String,
    connector_name: String,
    capabilities: BTreeSet<Capability>,
    metadata: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    compatibility: PluginCompatibility,
}

fn execute_plugins_init(command: PluginInitCommand) -> CliResult<PluginsInitExecution> {
    let package_root = normalize_required_cli_value("package root", &command.package_root)?;
    let plugin_id = normalize_required_cli_value("--plugin-id", &command.plugin_id)?;
    let provider_id = normalize_optional_cli_value(command.provider_id.as_deref())
        .unwrap_or_else(|| plugin_id.clone());
    let connector_name = normalize_optional_cli_value(command.connector_name.as_deref())
        .unwrap_or_else(|| plugin_id.clone());
    let version = normalize_required_cli_value("--version", &command.version)?;
    let summary = normalize_optional_cli_value(command.summary.as_deref());
    let bridge_kind = command.bridge_kind.as_bridge_kind();
    let declared_capabilities =
        resolve_scaffold_declared_capabilities(command.capabilities.as_slice())?;
    let declared_host_hooks = resolve_scaffold_host_hooks(command.host_hooks.as_slice())?;
    let declared_tui_surfaces = resolve_scaffold_tui_surfaces(command.tui_surfaces.as_slice())?;

    validate_plugin_scaffold_version(&version)?;

    let scaffold_defaults =
        plugin_runtime_scaffold_defaults(bridge_kind, command.source_language.as_deref())
            .map_err(|error| format!("plugins init failed: {error}; use --source-language when required by the selected bridge"))?;
    let process_stdio_profile =
        crate::native_extension_authoring::process_stdio_native_extension_language_profile(
            &scaffold_defaults,
        )?;
    if (!declared_host_hooks.is_empty() || !declared_tui_surfaces.is_empty())
        && process_stdio_profile.is_none()
    {
        return Err(
            "plugins init only scaffolds trusted host hooks and TUI surfaces on runnable process_stdio extension entrypoints"
                .to_owned(),
        );
    }

    let manifest = build_plugin_scaffold_manifest(
        &plugin_id,
        &provider_id,
        &connector_name,
        &version,
        summary,
        &scaffold_defaults,
        declared_capabilities,
        declared_host_hooks.clone(),
        declared_tui_surfaces.clone(),
        process_stdio_profile,
    );

    let package_root_path = Path::new(package_root.as_str());
    ensure_empty_plugin_scaffold_root(package_root_path)?;

    let manifest_path = package_root_path.join(PACKAGE_MANIFEST_FILE_NAME);
    let readme_path = package_root_path.join(PLUGINS_INIT_README_FILE_NAME);

    let manifest_document = plugin_scaffold_manifest_document(&manifest)?;
    let rendered_manifest = serde_json::to_string_pretty(&manifest_document)
        .map_err(|error| format!("serialize scaffold manifest failed: {error}"))?;
    let smoke_test_command = render_plugin_scaffold_smoke_test_command(
        package_root.as_str(),
        plugin_id.as_str(),
        process_stdio_profile,
        declared_host_hooks.as_slice(),
        declared_tui_surfaces.as_slice(),
    );
    let rendered_readme = render_plugin_scaffold_readme(
        package_root.as_str(),
        plugin_id.as_str(),
        bridge_kind.as_str(),
        smoke_test_command.as_deref(),
    );

    let runtime_files_written = write_plugin_scaffold_files(
        package_root_path,
        &manifest_path,
        rendered_manifest.as_str(),
        &readme_path,
        rendered_readme.as_str(),
        process_stdio_profile,
    )?;

    let manifest_path_string = manifest_path.display().to_string();
    let readme_path_string = readme_path.display().to_string();
    let mut files_written = vec![manifest_path_string.clone(), readme_path_string.clone()];
    files_written.extend(runtime_files_written.iter().cloned());

    Ok(PluginsInitExecution {
        schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
        schema: plugins_command_schema(PLUGINS_INIT_SCHEMA_PURPOSE),
        package_root,
        manifest_path: manifest_path_string,
        readme_path: readme_path_string,
        plugin_id,
        provider_id,
        connector_name,
        version,
        bridge_kind: bridge_kind.as_str().to_owned(),
        source_language: scaffold_defaults.source_language,
        adapter_family: scaffold_defaults.adapter_family,
        entrypoint: scaffold_defaults.entrypoint_hint,
        smoke_test_command,
        runtime_files_written,
        files_written,
    })
}

async fn execute_plugins_invoke_extension(
    command: PluginInvokeExtensionCommand,
) -> CliResult<PluginsInvokeExtensionExecution> {
    let package_root = normalize_required_cli_value("--root", &command.root)?;
    let plugin_id = normalize_required_cli_value("--plugin-id", &command.plugin_id)?;
    let method = normalize_required_cli_value("--method", &command.method)?;
    let payload = serde_json::from_str::<Value>(command.payload.as_str()).map_err(|error| {
        format!("plugins invoke-extension requires --payload to be valid JSON: {error}")
    })?;
    let plugin = scan_single_plugin_from_root(
        package_root.as_str(),
        plugin_id.as_str(),
        "plugins invoke-extension",
    )?;
    ensure_process_stdio_invocable_plugin(
        &plugin,
        plugin_id.as_str(),
        "plugins invoke-extension",
        "native extensions",
    )?;
    let bridge_policy =
        crate::trusted_host_runtime::build_process_stdio_bridge_policy_from_allow_commands(
            command.allow_commands,
            "plugins invoke-extension requires at least one --allow-command for process_stdio smoke probes",
        )?;
    let outcome = crate::trusted_host_runtime::invoke_process_stdio_extension_operation(
        &plugin,
        method.as_str(),
        payload.clone(),
        &bridge_policy,
    )
    .await
    .map_err(|error| format!("plugins invoke-extension failed: {error}"))?;

    Ok(PluginsInvokeExtensionExecution {
        schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
        schema: plugins_command_schema(PLUGINS_INVOKE_EXTENSION_SCHEMA_PURPOSE),
        package_root,
        plugin_id,
        bridge_kind: plugin.runtime.bridge_kind.as_str().to_owned(),
        source_language: Some(plugin.runtime.source_language.clone()),
        method,
        payload,
        response_payload: outcome.response_payload,
        runtime_evidence: outcome.runtime_evidence,
    })
}

async fn execute_plugins_invoke_host_hook(
    command: PluginInvokeHostHookCommand,
) -> CliResult<PluginsInvokeHostHookExecution> {
    let package_root = normalize_required_cli_value("--root", &command.root)?;
    let plugin_id = normalize_required_cli_value("--plugin-id", &command.plugin_id)?;
    let hook = normalize_required_cli_value("--hook", &command.hook)?;
    let payload = serde_json::from_str::<Value>(command.payload.as_str()).map_err(|error| {
        format!("plugins invoke-host-hook requires --payload to be valid JSON: {error}")
    })?;
    let plugin = scan_single_plugin_from_root(
        package_root.as_str(),
        plugin_id.as_str(),
        "plugins invoke-host-hook",
    )?;
    ensure_process_stdio_invocable_plugin(
        &plugin,
        plugin_id.as_str(),
        "plugins invoke-host-hook",
        "trusted host extensions",
    )?;
    let declarations =
        crate::kernel::plugin_native_extension_declarations_from_metadata(&plugin.metadata);
    if declarations.family.as_deref() != Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        || declarations.trust_lane.as_deref()
            != Some(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
    {
        return Err(format!(
            "plugins invoke-host-hook requires plugin `{plugin_id}` to declare loong_extension_family=`{}` and loong_extension_trust_lane=`{}`",
            crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY,
            crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE
        ));
    }
    if !declarations
        .methods
        .iter()
        .any(|method| method == "extension/event")
    {
        return Err(format!(
            "plugins invoke-host-hook requires plugin `{plugin_id}` to declare extension/event in loong_extension_methods_json"
        ));
    }
    if !declarations
        .host_hooks
        .iter()
        .any(|value| value == hook.as_str())
    {
        return Err(format!(
            "plugins invoke-host-hook requires plugin `{plugin_id}` to declare host hook `{hook}` in loong_extension_host_hooks_json"
        ));
    }
    if !crate::kernel::TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.contains(&hook.as_str()) {
        return Err(format!(
            "plugins invoke-host-hook requires supported read-only hook `{hook}`; supported hooks are {}",
            crate::kernel::TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.join(", ")
        ));
    }
    let bridge_policy =
        crate::trusted_host_runtime::build_process_stdio_bridge_policy_from_allow_commands(
            command.allow_commands,
            "plugins invoke-host-hook requires at least one --allow-command for process_stdio host-hook probes",
        )?;
    let hook_payload = crate::trusted_host_runtime::build_read_only_trusted_host_hook_payload(
        hook.as_str(),
        payload.clone(),
    );
    let dispatched_method = "extension/event".to_owned();
    let outcome = crate::trusted_host_runtime::invoke_process_stdio_extension_operation(
        &plugin,
        dispatched_method.as_str(),
        hook_payload,
        &bridge_policy,
    )
    .await
    .map_err(|error| format!("plugins invoke-host-hook failed: {error}"))?;

    Ok(PluginsInvokeHostHookExecution {
        schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
        schema: plugins_command_schema(PLUGINS_INVOKE_HOST_HOOK_SCHEMA_PURPOSE),
        package_root,
        plugin_id,
        extension_family: declarations.family,
        extension_trust_lane: declarations.trust_lane,
        bridge_kind: plugin.runtime.bridge_kind.as_str().to_owned(),
        source_language: Some(plugin.runtime.source_language.clone()),
        hook,
        payload,
        dispatched_method,
        response_payload: outcome.response_payload,
        runtime_evidence: outcome.runtime_evidence,
    })
}

async fn execute_plugins_invoke_tui_surface(
    command: PluginInvokeTuiSurfaceCommand,
) -> CliResult<PluginsInvokeTuiSurfaceExecution> {
    let package_root = normalize_required_cli_value("--root", &command.root)?;
    let plugin_id = normalize_required_cli_value("--plugin-id", &command.plugin_id)?;
    let tui_surface = normalize_required_cli_value("--tui-surface", &command.tui_surface)?;
    let payload = serde_json::from_str::<Value>(command.payload.as_str()).map_err(|error| {
        format!("plugins invoke-tui-surface requires --payload to be valid JSON: {error}")
    })?;
    let plugin = scan_single_plugin_from_root(
        package_root.as_str(),
        plugin_id.as_str(),
        "plugins invoke-tui-surface",
    )?;
    ensure_process_stdio_invocable_plugin(
        &plugin,
        plugin_id.as_str(),
        "plugins invoke-tui-surface",
        "trusted-host TUI surfaces",
    )?;
    let declarations =
        crate::kernel::plugin_native_extension_declarations_from_metadata(&plugin.metadata);
    if declarations.family.as_deref() != Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        || declarations.trust_lane.as_deref()
            != Some(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
    {
        return Err(format!(
            "plugins invoke-tui-surface requires plugin `{plugin_id}` to declare loong_extension_family=`{}` and loong_extension_trust_lane=`{}`",
            crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY,
            crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE
        ));
    }
    if !declarations
        .methods
        .iter()
        .any(|method| method == "extension/event")
    {
        return Err(format!(
            "plugins invoke-tui-surface requires plugin `{plugin_id}` to declare extension/event in loong_extension_methods_json"
        ));
    }
    if !declarations
        .tui_surfaces
        .iter()
        .any(|surface| surface == &tui_surface)
    {
        return Err(format!(
            "plugins invoke-tui-surface requires plugin `{plugin_id}` to declare TUI surface `{tui_surface}` in loong_extension_tui_surfaces_json"
        ));
    }
    if !crate::kernel::TRUSTED_HOST_TUI_EXTENSION_SURFACES.contains(&tui_surface.as_str()) {
        return Err(format!(
            "plugins invoke-tui-surface requires supported surface `{tui_surface}`; supported surfaces are {}",
            crate::kernel::TRUSTED_HOST_TUI_EXTENSION_SURFACES.join(", ")
        ));
    }
    let bridge_policy =
        crate::trusted_host_runtime::build_process_stdio_bridge_policy_from_allow_commands(
            command.allow_commands,
            "plugins invoke-tui-surface requires at least one --allow-command for process_stdio TUI-surface probes",
        )?;
    let surface_payload =
        crate::trusted_host_runtime::build_read_only_trusted_host_tui_surface_payload(
            tui_surface.as_str(),
            payload.clone(),
        );
    let outcome = crate::trusted_host_runtime::invoke_process_stdio_extension_operation(
        &plugin,
        "extension/event",
        surface_payload,
        &bridge_policy,
    )
    .await
    .map_err(|error| format!("plugins invoke-tui-surface failed: {error}"))?;

    Ok(PluginsInvokeTuiSurfaceExecution {
        schema_version: PLUGINS_COMMAND_SCHEMA_VERSION,
        schema: plugins_command_schema(PLUGINS_INVOKE_TUI_SURFACE_SCHEMA_PURPOSE),
        package_root,
        plugin_id,
        extension_family: declarations.family,
        extension_trust_lane: declarations.trust_lane,
        bridge_kind: plugin.runtime.bridge_kind.as_str().to_owned(),
        source_language: Some(plugin.runtime.source_language.clone()),
        tui_surface,
        payload,
        dispatched_method: "extension/event".to_owned(),
        response_payload: outcome.response_payload,
        runtime_evidence: outcome.runtime_evidence,
    })
}

fn normalize_required_cli_value(field_name: &str, raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err(format!("plugins init requires a non-empty {field_name}"));
    }

    Ok(trimmed.to_owned())
}

fn scan_single_plugin_from_root(
    package_root: &str,
    plugin_id: &str,
    command_name: &str,
) -> CliResult<crate::kernel::PluginIR> {
    let scanner = crate::kernel::PluginScanner::new();
    let scan_report = scanner
        .scan_path(package_root)
        .map_err(|error| format!("scan extension package failed: {error}"))?;
    let translator = crate::kernel::PluginTranslator::new();
    let translation_report = translator.translate_scan_report(&scan_report);
    let matching_entries = translation_report
        .entries
        .iter()
        .filter(|entry| entry.plugin_id == plugin_id)
        .cloned()
        .collect::<Vec<_>>();
    if matching_entries.is_empty() {
        return Err(format!(
            "{command_name} could not find plugin_id `{plugin_id}` under root `{package_root}`"
        ));
    }
    if matching_entries.len() > 1 {
        return Err(format!(
            "{command_name} found multiple plugin entries named `{plugin_id}` under root `{package_root}`"
        ));
    }
    matching_entries.into_iter().next().ok_or_else(|| {
        format!("{command_name} could not find plugin_id `{plugin_id}` under root `{package_root}`")
    })
}

fn ensure_process_stdio_invocable_plugin(
    plugin: &crate::kernel::PluginIR,
    plugin_id: &str,
    command_name: &str,
    plugin_surface: &str,
) -> CliResult<()> {
    if plugin.runtime.bridge_kind != PluginBridgeKind::ProcessStdio {
        return Err(format!(
            "{command_name} currently supports only process_stdio {plugin_surface}; plugin `{plugin_id}` declares bridge_kind `{}`",
            plugin.runtime.bridge_kind.as_str()
        ));
    }
    Ok(())
}

fn write_plugin_scaffold_files(
    package_root: &Path,
    manifest_path: &Path,
    rendered_manifest: &str,
    readme_path: &Path,
    rendered_readme: &str,
    process_stdio_profile: Option<
        crate::native_extension_authoring::ProcessStdioNativeExtensionLanguageProfile,
    >,
) -> CliResult<Vec<String>> {
    let manifest_write_result = fs::write(manifest_path, rendered_manifest);
    if let Err(error) = manifest_write_result {
        return Err(format!(
            "write scaffold manifest `{}` failed: {error}",
            manifest_path.display()
        ));
    }

    let readme_write_result = fs::write(readme_path, rendered_readme);
    if let Err(error) = readme_write_result {
        let manifest_cleanup_result = fs::remove_file(manifest_path);
        if let Err(cleanup_error) = manifest_cleanup_result {
            return Err(format!(
                "write scaffold readme `{}` failed: {error}; cleanup scaffold manifest `{}` failed: {cleanup_error}",
                readme_path.display(),
                manifest_path.display()
            ));
        }

        return Err(format!(
            "write scaffold readme `{}` failed: {error}",
            readme_path.display()
        ));
    }

    let mut runtime_files_written = Vec::new();
    if let Some(profile) = process_stdio_profile {
        for scaffold_file in profile.scaffold_files {
            let file_path = package_root.join(scaffold_file.relative_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "create scaffold runtime directory `{}` failed: {error}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&file_path, scaffold_file.contents).map_err(|error| {
                format!(
                    "write scaffold runtime file `{}` failed: {error}",
                    file_path.display()
                )
            })?;
            runtime_files_written.push(file_path.display().to_string());
        }
    }

    Ok(runtime_files_written)
}

fn normalize_optional_cli_value(raw: Option<&str>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_owned())
    })
}

fn resolve_scaffold_declared_capabilities(raw: &[String]) -> CliResult<BTreeSet<Capability>> {
    let mut declared_capabilities = BTreeSet::from([Capability::InvokeConnector]);

    for capability_name in raw {
        let trimmed = capability_name.trim();
        if trimmed.is_empty() {
            return Err("plugins init requires each --capability value to be non-empty".to_owned());
        }
        let Some(capability) = Capability::parse(trimmed) else {
            return Err(format!(
                "plugins init received unsupported --capability `{trimmed}`"
            ));
        };
        declared_capabilities.insert(capability);
    }

    Ok(declared_capabilities)
}

fn resolve_scaffold_host_hooks(raw: &[String]) -> CliResult<Vec<String>> {
    let mut declared_host_hooks = Vec::new();

    for hook_name in raw {
        let trimmed = hook_name.trim();
        if trimmed.is_empty() {
            return Err("plugins init requires each --host-hook value to be non-empty".to_owned());
        }
        if !crate::kernel::TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.contains(&trimmed) {
            return Err(format!(
                "plugins init received unsupported --host-hook `{trimmed}`; supported hooks are {}",
                crate::kernel::TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.join(", ")
            ));
        }
        if declared_host_hooks
            .iter()
            .any(|existing| existing == trimmed)
        {
            continue;
        }
        declared_host_hooks.push(trimmed.to_owned());
    }

    Ok(declared_host_hooks)
}

fn resolve_scaffold_tui_surfaces(raw: &[String]) -> CliResult<Vec<String>> {
    let mut declared_tui_surfaces = Vec::new();

    for surface_name in raw {
        let trimmed = surface_name.trim();
        if trimmed.is_empty() {
            return Err(
                "plugins init requires each --tui-surface value to be non-empty".to_owned(),
            );
        }
        if !crate::kernel::TRUSTED_HOST_TUI_EXTENSION_SURFACES.contains(&trimmed) {
            return Err(format!(
                "plugins init received unsupported --tui-surface `{trimmed}`; supported surfaces are {}",
                crate::kernel::TRUSTED_HOST_TUI_EXTENSION_SURFACES.join(", ")
            ));
        }
        if declared_tui_surfaces
            .iter()
            .any(|existing| existing == trimmed)
        {
            continue;
        }
        declared_tui_surfaces.push(trimmed.to_owned());
    }

    Ok(declared_tui_surfaces)
}

fn render_plugin_scaffold_smoke_test_command(
    package_root: &str,
    plugin_id: &str,
    process_stdio_profile: Option<
        crate::native_extension_authoring::ProcessStdioNativeExtensionLanguageProfile,
    >,
    declared_host_hooks: &[String],
    declared_tui_surfaces: &[String],
) -> Option<String> {
    let profile = process_stdio_profile?;
    if let Some(hook) = declared_host_hooks.first() {
        return Some(
            crate::native_extension_authoring::render_authoring_host_hook_probe_command(
                package_root,
                plugin_id,
                hook.as_str(),
                profile.smoke_allow_command,
            ),
        );
    }
    if let Some(surface) = declared_tui_surfaces.first() {
        return Some(
            crate::native_extension_authoring::render_authoring_tui_surface_probe_command(
                package_root,
                plugin_id,
                surface.as_str(),
                profile.smoke_allow_command,
            ),
        );
    }
    Some(
        crate::native_extension_authoring::render_authoring_smoke_test_command(
            package_root,
            plugin_id,
            profile.smoke_allow_command,
        ),
    )
}

fn validate_plugin_scaffold_version(version: &str) -> CliResult<()> {
    Version::parse(version)
        .map(|_| ())
        .map_err(|error| format!("plugins init requires --version to be valid semver: {error}"))
}

fn ensure_empty_plugin_scaffold_root(package_root: &Path) -> CliResult<()> {
    if package_root.exists() {
        let root_is_directory = package_root.is_dir();
        if !root_is_directory {
            return Err(format!(
                "plugins init requires package root `{}` to be a directory",
                package_root.display()
            ));
        }

        let mut entries = fs::read_dir(package_root).map_err(|error| {
            format!(
                "inspect scaffold package root `{}` failed: {error}",
                package_root.display()
            )
        })?;
        let first_entry = entries.next().transpose().map_err(|error| {
            format!(
                "inspect scaffold package root `{}` failed: {error}",
                package_root.display()
            )
        })?;
        if first_entry.is_some() {
            return Err(format!(
                "plugins init requires an empty package root; `{}` already contains files",
                package_root.display()
            ));
        }

        return Ok(());
    }

    fs::create_dir_all(package_root).map_err(|error| {
        format!(
            "create scaffold package root `{}` failed: {error}",
            package_root.display()
        )
    })
}

fn build_plugin_scaffold_manifest(
    plugin_id: &str,
    provider_id: &str,
    connector_name: &str,
    version: &str,
    summary: Option<String>,
    scaffold_defaults: &crate::kernel::PluginRuntimeScaffoldDefaults,
    capabilities: BTreeSet<Capability>,
    host_hooks: Vec<String>,
    tui_surfaces: Vec<String>,
    process_stdio_profile: Option<
        crate::native_extension_authoring::ProcessStdioNativeExtensionLanguageProfile,
    >,
) -> PluginManifest {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "bridge_kind".to_owned(),
        scaffold_defaults.bridge_kind.as_str().to_owned(),
    );
    metadata.insert(
        "adapter_family".to_owned(),
        scaffold_defaults.adapter_family.clone(),
    );
    metadata.insert(
        "entrypoint".to_owned(),
        scaffold_defaults.entrypoint_hint.clone(),
    );
    if let Some(source_language) = scaffold_defaults.source_language.as_ref() {
        metadata.insert("source_language".to_owned(), source_language.clone());
    }
    if let Some(profile) = process_stdio_profile {
        metadata.insert("command".to_owned(), profile.command.to_owned());
        metadata.insert(
            "args_json".to_owned(),
            serde_json::to_string(
                &crate::native_extension_authoring::process_stdio_scaffold_args(profile),
            )
            .unwrap_or_else(|_| "[]".to_owned()),
        );
        metadata.insert(
            "process_timeout_ms".to_owned(),
            profile.process_timeout_ms.to_string(),
        );
        metadata.insert(
            "loong_extension_contract".to_owned(),
            crate::native_extension_authoring::PROCESS_STDIO_NATIVE_EXTENSION_CONTRACT.to_owned(),
        );
        if !host_hooks.is_empty() || !tui_surfaces.is_empty() {
            metadata.insert(
                "loong_extension_family".to_owned(),
                crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY.to_owned(),
            );
            metadata.insert(
                "loong_extension_trust_lane".to_owned(),
                crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE.to_owned(),
            );
            metadata.insert(
                "loong_extension_methods_json".to_owned(),
                serde_json::to_string(
                    &crate::native_extension_authoring::TRUSTED_HOST_PROCESS_STDIO_EXTENSION_METHODS
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".to_owned()),
            );
        } else {
            metadata.insert(
                "loong_extension_methods_json".to_owned(),
                serde_json::to_string(
                    &crate::native_extension_authoring::PROCESS_STDIO_NATIVE_EXTENSION_METHODS
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".to_owned()),
            );
            metadata.insert(
                "loong_extension_events_json".to_owned(),
                serde_json::to_string(
                    &crate::native_extension_authoring::PROCESS_STDIO_NATIVE_EXTENSION_EVENTS
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".to_owned()),
            );
        }
        metadata.insert(
            "loong_extension_host_hooks_json".to_owned(),
            serde_json::to_string(&host_hooks).unwrap_or_else(|_| "[]".to_owned()),
        );
        metadata.insert(
            "loong_extension_tui_surfaces_json".to_owned(),
            serde_json::to_string(&tui_surfaces).unwrap_or_else(|_| "[]".to_owned()),
        );
    }

    let host_version_req = format!(">={}", env!("CARGO_PKG_VERSION"));
    let compatibility = PluginCompatibility {
        host_api: Some(CURRENT_PLUGIN_HOST_API.to_owned()),
        host_version_req: Some(host_version_req),
    };

    PluginManifest {
        api_version: Some(CURRENT_PLUGIN_MANIFEST_API_VERSION.to_owned()),
        version: Some(version.to_owned()),
        plugin_id: plugin_id.to_owned(),
        provider_id: provider_id.to_owned(),
        connector_name: connector_name.to_owned(),
        channel_id: None,
        endpoint: None,
        capabilities,
        trust_tier: Default::default(),
        metadata,
        summary,
        tags: Vec::new(),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: false,
        setup: None,
        slot_claims: Vec::new(),
        compatibility: Some(compatibility),
    }
}

fn plugin_scaffold_manifest_document(
    manifest: &PluginManifest,
) -> CliResult<PluginPackageScaffoldManifestDocument> {
    let api_version = manifest
        .api_version
        .clone()
        .ok_or_else(|| "scaffold manifest is missing api_version".to_owned())?;
    let version = manifest
        .version
        .clone()
        .ok_or_else(|| "scaffold manifest is missing version".to_owned())?;
    let compatibility = manifest
        .compatibility
        .clone()
        .ok_or_else(|| "scaffold manifest is missing compatibility".to_owned())?;

    Ok(PluginPackageScaffoldManifestDocument {
        api_version,
        version,
        plugin_id: manifest.plugin_id.clone(),
        provider_id: manifest.provider_id.clone(),
        connector_name: manifest.connector_name.clone(),
        capabilities: manifest.capabilities.clone(),
        metadata: manifest.metadata.clone(),
        summary: manifest.summary.clone(),
        compatibility,
    })
}

fn render_plugin_scaffold_readme(
    package_root: &str,
    plugin_id: &str,
    bridge_kind: &str,
    smoke_test_command: Option<&str>,
) -> String {
    let doctor_command =
        format!("loong plugins doctor --root \"{package_root}\" --profile sdk-release");
    let actions_command =
        format!("loong plugins actions --root \"{package_root}\" --profile sdk-release");

    let mut lines = vec![
        format!("# {plugin_id}"),
        String::new(),
        "This package was scaffolded by `loong plugins init`.".to_owned(),
        String::new(),
        format!("- Bridge kind: `{bridge_kind}`"),
        format!("- Manifest: `{PACKAGE_MANIFEST_FILE_NAME}`"),
        "- Trust default: `unverified`".to_owned(),
        String::new(),
        "## Next Steps".to_owned(),
        String::new(),
        format!(
            "1. Replace the scaffolded bridge entrypoint in `{PACKAGE_MANIFEST_FILE_NAME}` with the real runtime entry for your package."
        ),
        format!(
            "2. Fill in `summary`, `setup`, `slot_claims`, `tags`, and examples in `{PACKAGE_MANIFEST_FILE_NAME}` as your package contract becomes concrete."
        ),
        "3. Diagnose the package contract through the shared author-facing governance surface:"
            .to_owned(),
        String::new(),
        "```bash".to_owned(),
        doctor_command,
        "```".to_owned(),
        String::new(),
        "4. Review the deduplicated operator action plan before release or marketplace handoff:"
            .to_owned(),
        String::new(),
        "```bash".to_owned(),
        actions_command,
        "```".to_owned(),
    ];
    if let Some(smoke_test_command) = smoke_test_command {
        lines.extend([
            String::new(),
            "5. Run the bounded runtime smoke probe before iterating on the package implementation:"
                .to_owned(),
            String::new(),
            "```bash".to_owned(),
            smoke_test_command.to_owned(),
            "```".to_owned(),
        ]);
    }
    lines.join("\n")
}

fn render_plugins_cli_text(execution: &PluginsCommandExecution) -> String {
    let (title, body) = match execution {
        PluginsCommandExecution::Init(execution) => {
            ("plugins init", render_plugins_init_text(execution))
        }
        PluginsCommandExecution::InvokeExtension(execution) => (
            "plugins invoke-extension",
            render_plugins_invoke_extension_text(execution),
        ),
        PluginsCommandExecution::InvokeHostHook(execution) => (
            "plugins invoke-host-hook",
            render_plugins_invoke_host_hook_text(execution),
        ),
        PluginsCommandExecution::InvokeTuiSurface(execution) => (
            "plugins invoke-tui-surface",
            render_plugins_invoke_tui_surface_text(execution),
        ),
        PluginsCommandExecution::Inventory(execution) => (
            "plugins inventory",
            render_plugins_inventory_text(execution),
        ),
        PluginsCommandExecution::Doctor(execution) => {
            ("plugins doctor", render_plugins_doctor_text(execution))
        }
        PluginsCommandExecution::BridgeProfiles(execution) => (
            "bridge profiles",
            render_plugins_bridge_profiles_text(execution),
        ),
        PluginsCommandExecution::BridgeTemplate(execution) => (
            "bridge template",
            render_plugins_bridge_template_text(execution),
        ),
        PluginsCommandExecution::Preflight(execution) => (
            "plugins preflight",
            render_plugins_preflight_text(execution),
        ),
        PluginsCommandExecution::Actions(execution) => {
            ("operator actions", render_plugins_actions_text(execution))
        }
    };
    wrap_plugins_surface_text(title, body)
}

fn wrap_plugins_surface_text(title: &str, body: String) -> String {
    crate::render_operator_shell_surface_from_body(title, "operator plugins", body)
}

fn render_plugins_invoke_extension_text(execution: &PluginsInvokeExtensionExecution) -> String {
    let source_language = execution.source_language.as_deref().unwrap_or("-");
    let response_payload = serde_json::to_string_pretty(&execution.response_payload)
        .unwrap_or_else(|_| execution.response_payload.to_string());
    let runtime_evidence = serde_json::to_string_pretty(&execution.runtime_evidence)
        .unwrap_or_else(|_| execution.runtime_evidence.to_string());

    [
        format!(
            "plugins invoke-extension package_root={} plugin_id={} bridge_kind={} source_language={} method={}",
            execution.package_root,
            execution.plugin_id,
            execution.bridge_kind,
            source_language,
            execution.method
        ),
        "payload:".to_owned(),
        execution.payload.to_string(),
        "response_payload:".to_owned(),
        response_payload,
        "runtime_evidence:".to_owned(),
        runtime_evidence,
    ]
    .join("\n")
}

fn render_plugins_invoke_host_hook_text(execution: &PluginsInvokeHostHookExecution) -> String {
    let source_language = execution.source_language.as_deref().unwrap_or("-");
    let response_payload = serde_json::to_string_pretty(&execution.response_payload)
        .unwrap_or_else(|_| execution.response_payload.to_string());
    let runtime_evidence = serde_json::to_string_pretty(&execution.runtime_evidence)
        .unwrap_or_else(|_| execution.runtime_evidence.to_string());

    [
        format!(
            "plugins invoke-host-hook package_root={} plugin_id={} extension_family={} extension_trust_lane={} bridge_kind={} source_language={} hook={} dispatched_method={}",
            execution.package_root,
            execution.plugin_id,
            display_text_or_dash(execution.extension_family.as_deref()),
            display_text_or_dash(execution.extension_trust_lane.as_deref()),
            execution.bridge_kind,
            source_language,
            execution.hook,
            execution.dispatched_method
        ),
        "payload:".to_owned(),
        execution.payload.to_string(),
        "response_payload:".to_owned(),
        response_payload,
        "runtime_evidence:".to_owned(),
        runtime_evidence,
    ]
    .join("\n")
}

fn render_plugins_invoke_tui_surface_text(execution: &PluginsInvokeTuiSurfaceExecution) -> String {
    let source_language = execution.source_language.as_deref().unwrap_or("-");
    let response_payload = serde_json::to_string_pretty(&execution.response_payload)
        .unwrap_or_else(|_| execution.response_payload.to_string());
    let runtime_evidence = serde_json::to_string_pretty(&execution.runtime_evidence)
        .unwrap_or_else(|_| execution.runtime_evidence.to_string());

    [
        format!(
            "plugins invoke-tui-surface package_root={} plugin_id={} extension_family={} extension_trust_lane={} bridge_kind={} source_language={} tui_surface={} dispatched_method={}",
            execution.package_root,
            execution.plugin_id,
            display_text_or_dash(execution.extension_family.as_deref()),
            display_text_or_dash(execution.extension_trust_lane.as_deref()),
            execution.bridge_kind,
            source_language,
            execution.tui_surface,
            execution.dispatched_method
        ),
        "payload:".to_owned(),
        execution.payload.to_string(),
        "response_payload:".to_owned(),
        response_payload,
        "runtime_evidence:".to_owned(),
        runtime_evidence,
    ]
    .join("\n")
}

fn render_plugins_init_text(execution: &PluginsInitExecution) -> String {
    let source_language = execution.source_language.as_deref().unwrap_or("-");
    let mut lines = vec![format!(
        "plugins init package_root={} plugin_id={} provider_id={} connector_name={}",
        execution.package_root,
        execution.plugin_id,
        execution.provider_id,
        execution.connector_name
    )];
    lines.push(format!(
        "- bridge_kind={} source_language={} adapter_family={} entrypoint={}",
        execution.bridge_kind, source_language, execution.adapter_family, execution.entrypoint
    ));
    if !execution.runtime_files_written.is_empty() {
        lines.push(format!(
            "- runtime_files_written={}",
            execution.runtime_files_written.join(",")
        ));
    }
    if let Some(smoke_test_command) = execution.smoke_test_command.as_deref() {
        lines.push(format!("- smoke_test_command={smoke_test_command}"));
    }
    lines.push(format!("- manifest={}", execution.manifest_path));
    lines.push(format!("- readme={}", execution.readme_path));
    lines.push(format!(
        "- next_steps=loong plugins doctor --root \"{}\" --profile sdk-release",
        execution.package_root
    ));
    lines.push(format!(
        "- operator_actions=loong plugins actions --root \"{}\" --profile sdk-release",
        execution.package_root
    ));
    lines.join("\n")
}

fn populate_native_extension_authoring_guidance(result: &mut PluginInventoryResult) {
    if result.authoring_guidance.is_none()
        && result.bridge_kind == PluginBridgeKind::ProcessStdio.as_str()
    {
        result.authoring_guidance =
            crate::native_extension_authoring::process_stdio_native_extension_authoring_guidance(
                result.package_root.as_str(),
                result.plugin_id.as_str(),
                result.source_language.as_deref(),
                &result.native_extension,
            );
    }
}

fn plugin_native_extension_authoring_guidance(
    result: &PluginInventoryResult,
) -> Option<crate::PluginNativeExtensionAuthoringGuidance> {
    result.authoring_guidance.clone().or_else(|| {
        if result.bridge_kind != PluginBridgeKind::ProcessStdio.as_str() {
            return None;
        }

        crate::native_extension_authoring::process_stdio_native_extension_authoring_guidance(
            result.package_root.as_str(),
            result.plugin_id.as_str(),
            result.source_language.as_deref(),
            &result.native_extension,
        )
    })
}

fn render_plugins_inventory_text(execution: &PluginsInventoryExecution) -> String {
    let mut lines = vec![format!(
        "plugins inventory query={} roots={} returned_plugins={} ready={} setup_incomplete={} blocked={} deferred={} loaded={}",
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.returned_results,
        execution.summary.ready_plugins,
        execution.summary.setup_incomplete_plugins,
        execution.summary.blocked_plugins,
        execution.summary.deferred_plugins,
        execution.summary.loaded_plugins
    )];
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} bridge={} capabilities={} language={} setup_surface={} activation_status={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution),
        format_rollup_map(&execution.summary.capability_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.setup_surface_distribution),
        format_rollup_map(&execution.summary.activation_status_distribution)
    ));
    for result in &execution.results {
        let activation_status = inventory_result_status_label(result);
        let setup_surface = inventory_result_setup_surface_label(result);
        let capabilities = format_csv_or_dash(&result.capabilities);
        let source_language = result.source_language.as_deref().unwrap_or("-");
        let manifest_path = display_text_or_dash(result.package_manifest_path.as_deref());
        let setup_mode = display_text_or_dash(result.setup_mode.as_deref());
        let host_api = result
            .compatibility
            .as_ref()
            .and_then(|compatibility| compatibility.host_api.as_deref());
        let host_version_req = result
            .compatibility
            .as_ref()
            .and_then(|compatibility| compatibility.host_version_req.as_deref());
        let required_env_vars = format_csv_or_dash(&result.setup_required_env_vars);
        let required_config_keys = format_csv_or_dash(&result.setup_required_config_keys);
        let runtime_health = result
            .runtime_health
            .as_ref()
            .map(|health| health.status.as_str());
        let native_extension = &result.native_extension;
        let attestation = result
            .activation_attestation
            .as_ref()
            .map(|attestation| attestation.integrity.as_str());
        lines.push(format!(
            "- plugin={} provider={} status={} loaded={} deferred={} bridge={} capabilities={} language={} setup_surface={}",
            result.plugin_id,
            result.provider_id,
            activation_status,
            result.loaded,
            result.deferred,
            result.bridge_kind,
            capabilities,
            source_language,
            setup_surface
        ));
        lines.push(format!(
            "  manifest={} setup_mode={} required_env={} required_config={} host_api={} host_version_req={}",
            manifest_path,
            setup_mode,
            required_env_vars,
            required_config_keys,
            display_text_or_dash(host_api),
            display_text_or_dash(host_version_req)
        ));
        lines.push(format!(
            "  source={} bootstrap_hint={} runtime_health={} attestation={} summary={}",
            result.source_path,
            display_text_or_dash(result.bootstrap_hint.as_deref()),
            display_text_or_dash(runtime_health),
            display_text_or_dash(attestation),
            display_text_or_dash(result.summary.as_deref())
        ));
        let has_native_extension_projection = native_extension.contract.is_some()
            || native_extension.family.is_some()
            || native_extension.trust_lane.is_some()
            || !native_extension.methods.is_empty()
            || !native_extension.host_hooks.is_empty()
            || !native_extension.tui_surfaces.is_empty()
            || !native_extension.metadata_issues.is_empty();
        if has_native_extension_projection {
            lines.push(format!(
                "  native_extension contract={} family={} trust_lane={} methods={} host_hooks={} tui_surfaces={} metadata_issues={}",
                display_text_or_dash(native_extension.contract.as_deref()),
                display_text_or_dash(native_extension.family.as_deref()),
                display_text_or_dash(native_extension.trust_lane.as_deref()),
                format_csv_or_dash(&native_extension.methods),
                format_csv_or_dash(&native_extension.host_hooks),
                format_csv_or_dash(&native_extension.tui_surfaces),
                format_csv_or_dash(&native_extension.metadata_issues),
            ));
        }
        if let Some(guidance) = plugin_native_extension_authoring_guidance(result) {
            lines.push(format!(
                "  authoring validate={} operator_actions={}",
                guidance.validate_command, guidance.operator_actions_command
            ));
            lines.push(format!(
                "  authoring_smoke_test={}",
                guidance.smoke_test_command
            ));
        }
        if let Some(reason) = result.activation_reason.as_deref() {
            lines.push(format!("  activation_reason={reason}"));
        }
    }
    lines.join("\n")
}

fn render_plugins_doctor_text(execution: &PluginsDoctorExecution) -> String {
    let preflight_summary = &execution.preflight_summary;
    let mut lines = vec![format!(
        "plugins doctor profile={} query={} roots={} matched_plugins={} returned_plugins={} passed={} warned={} blocked={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.returned_results,
        execution.summary.passed_plugins,
        execution.summary.warned_plugins,
        execution.summary.blocked_plugins
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        preflight_summary.policy_source,
        display_text_or_dash(preflight_summary.policy_version.as_deref()),
        preflight_summary.policy_checksum,
        preflight_summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem bridge={} capabilities={} language={} setup_surface={} activation_status={}",
        format_rollup_map(&execution.summary.bridge_kind_distribution),
        format_rollup_map(&execution.summary.capability_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.setup_surface_distribution),
        format_rollup_map(&execution.summary.activation_status_distribution)
    ));
    lines.push(format!(
        "doctor_attention activation_ready={} setup_incomplete={} deferred={} loaded={} attention={} remediation_counts={}",
        execution.summary.activation_ready_plugins,
        execution.summary.setup_incomplete_plugins,
        execution.summary.deferred_plugins,
        execution.summary.loaded_plugins,
        execution.summary.packages_requiring_author_attention,
        format_rollup_map(&execution.summary.remediation_counts)
    ));
    lines.push(format!(
        "doctor_actions recommended={} operator_actions={} packages_with_operator_actions={} operator_plan_by_kind={}",
        execution.summary.total_recommended_actions,
        execution.summary.total_operator_actions,
        execution.summary.packages_with_operator_actions,
        format_rollup_map(&preflight_summary.operator_action_counts_by_kind)
    ));
    lines.extend(render_bridge_profile_fit_lines(preflight_summary));
    for result in &execution.results {
        lines.extend(render_plugin_doctor_result_lines(result));
    }
    lines.join("\n")
}

fn render_plugins_bridge_profiles_text(execution: &PluginsBridgeProfilesExecution) -> String {
    let mut lines = vec![format!(
        "plugins bridge-profiles returned_profiles={}",
        execution.profiles.len()
    )];
    for profile in &execution.profiles {
        lines.push(format!(
            "- profile={} version={} source={} checksum={} sha256={}",
            profile.profile_id,
            profile.policy_version.as_deref().unwrap_or("-"),
            profile.source,
            profile.checksum,
            profile.sha256
        ));
        lines.push(format!(
            "  bridges={} compatibility={} shims={} execute_process_stdio={} execute_http_json={} enforce_supported={} enforce_execution_success={}",
            format_csv_or_dash(&profile.supported_bridges),
            format_csv_or_dash(&profile.supported_compatibility_modes),
            format_csv_or_dash(&profile.supported_compatibility_shims),
            profile.execute_process_stdio,
            profile.execute_http_json,
            profile.enforce_supported,
            profile.enforce_execution_success
        ));
        for shim in &profile.shim_support_profiles {
            lines.push(format!(
                "  shim={} family={} version={} dialects={} bridges={} adapter_families={} languages={}",
                shim.shim_id,
                shim.shim_family,
                display_text_or_dash(shim.version.as_deref()),
                format_csv_or_dash(&shim.supported_dialects),
                format_csv_or_dash(&shim.supported_bridges),
                format_csv_or_dash(&shim.supported_adapter_families),
                format_csv_or_dash(&shim.supported_source_languages)
            ));
        }
    }
    lines.join("\n")
}

fn render_plugins_bridge_template_text(execution: &PluginsBridgeTemplateExecution) -> String {
    let mut lines = vec![format!(
        "plugins bridge-template profile={} query={} roots={} matched_plugins={} template_kind={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.template_kind
    )];
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    lines.push(format!(
        "template profile={} source={} version={} checksum={} sha256={} output={}",
        execution.template_profile_id,
        execution.template_source,
        display_text_or_dash(execution.template_policy_version.as_deref()),
        execution.template_checksum,
        execution.template_sha256,
        display_text_or_dash(execution.output_path.as_deref())
    ));
    lines.push(format!(
        "template_delta base_profile={} base_source={} base_version={} checksum={} sha256={} output={}",
        execution.delta_artifact.base_profile_id,
        execution.delta_artifact.base_source,
        display_text_or_dash(execution.delta_artifact.base_policy_version.as_deref()),
        execution.delta_artifact.checksum,
        execution.delta_artifact.sha256,
        display_text_or_dash(execution.delta_output_path.as_deref())
    ));
    lines.push(format!(
        "template_delta_support bridges={} compatibility={} adapter_families={} shims={} shim_profiles={} unresolved={}",
        format_csv_or_dash(&execution.delta_artifact.delta.supported_bridges),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_compatibility_modes),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_adapter_families),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_compatibility_shims),
        format_bridge_shim_profile_delta_artifact(&execution.delta_artifact.delta.shim_profile_additions),
        format_csv_or_dash(&execution.delta_artifact.delta.unresolved_blocking_reasons)
    ));
    lines.push(format!(
        "template_support bridges={} compatibility={} shims={} execute_process_stdio={} execute_http_json={} enforce_supported={} enforce_execution_success={}",
        execution
            .template
            .supported_bridges
            .iter()
            .map(|bridge| bridge.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        execution
            .template
            .supported_compatibility_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        execution
            .template
            .supported_compatibility_shims
            .iter()
            .map(|shim| format!("{}:{}", shim.shim_id, shim.family))
            .collect::<Vec<_>>()
            .join(","),
        execution.template.execute_process_stdio,
        execution.template.execute_http_json,
        execution.template.enforce_supported,
        execution.template.enforce_execution_success
    ));
    lines.join("\n")
}

fn render_plugins_preflight_text(execution: &PluginsPreflightExecution) -> String {
    let mut lines = vec![format!(
        "plugins preflight profile={} query={} roots={} matched_plugins={} returned_plugins={} passed={} warned={} blocked={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.summary.returned_plugins,
        execution.summary.passed_plugins,
        execution.summary.warned_plugins,
        execution.summary.blocked_plugins
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        execution.summary.policy_source,
        execution.summary.policy_version.as_deref().unwrap_or("-"),
        execution.summary.policy_checksum,
        execution.summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} dialect={} compatibility={} language={} bridge={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.dialect_distribution),
        format_rollup_map(&execution.summary.compatibility_mode_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution)
    ));
    lines.push(format!(
        "diagnostics total={} blocking={} error={} warning={} info={}",
        execution.summary.total_diagnostics,
        execution.summary.blocking_diagnostics,
        execution.summary.error_diagnostics,
        execution.summary.warning_diagnostics,
        execution.summary.info_diagnostics
    ));
    lines.push(format!(
        "operator_actions total={} by_surface={} by_kind={} reload={} no_reload={}",
        execution.summary.operator_action_plan.len(),
        format_rollup_map(&execution.summary.operator_action_counts_by_surface),
        format_rollup_map(&execution.summary.operator_action_counts_by_kind),
        execution.summary.operator_actions_requiring_reload,
        execution.summary.operator_actions_without_reload
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    for result in &execution.results {
        let plugin = &result.plugin;
        let action_kinds =
            format_preflight_result_operator_action_kinds(&result.recommended_actions);
        lines.push(format!(
            "- plugin={} provider={} verdict={} baseline={} activation_ready={} loaded={} actions={}",
            plugin.plugin_id,
            plugin.provider_id,
            result.verdict,
            result.baseline_verdict,
            result.activation_ready,
            plugin.loaded,
            action_kinds
        ));
    }
    lines.join("\n")
}

fn render_plugin_doctor_result_lines(result: &PluginPreflightResult) -> Vec<String> {
    let plugin = &result.plugin;
    let activation_status = inventory_result_status_label(plugin);
    let setup_surface = inventory_result_setup_surface_label(plugin);
    let source_language = plugin.source_language.as_deref().unwrap_or("-");
    let capabilities = format_csv_or_dash(&plugin.capabilities);
    let manifest_path = display_text_or_dash(plugin.package_manifest_path.as_deref());
    let setup_mode = display_text_or_dash(plugin.setup_mode.as_deref());
    let required_env_vars = format_csv_or_dash(&plugin.setup_required_env_vars);
    let required_config_keys = format_csv_or_dash(&plugin.setup_required_config_keys);
    let setup_remediation = display_text_or_dash(plugin.setup_remediation.as_deref());
    let runtime_health = plugin
        .runtime_health
        .as_ref()
        .map(|health| health.status.as_str());
    let native_extension = &plugin.native_extension;
    let attestation = plugin
        .activation_attestation
        .as_ref()
        .map(|value| value.integrity.as_str());
    let effective_flags = format_csv_or_dash(&result.effective_policy_flags);
    let remediation_classes = format_preflight_remediation_classes(&result.remediation_classes);
    let operator_action_kinds =
        format_preflight_result_operator_action_kinds(&result.recommended_actions);
    let blocking_diagnostics = format_csv_or_dash(&result.blocking_diagnostic_codes);
    let advisory_diagnostics = format_csv_or_dash(&result.advisory_diagnostic_codes);
    let recommended_actions =
        format_preflight_result_recommended_actions(&result.recommended_actions);

    let mut lines = vec![format!(
        "- plugin={} provider={} verdict={} activation_status={} loaded={} deferred={} bridge={} capabilities={} language={} setup_surface={}",
        plugin.plugin_id,
        plugin.provider_id,
        result.verdict,
        activation_status,
        plugin.loaded,
        plugin.deferred,
        plugin.bridge_kind,
        capabilities,
        source_language,
        setup_surface
    )];
    lines.push(format!(
        "  manifest={} setup_mode={} required_env={} required_config={} setup_remediation={}",
        manifest_path, setup_mode, required_env_vars, required_config_keys, setup_remediation
    ));
    lines.push(format!(
        "  source={} activation_ready={} runtime_health={} attestation={} summary={}",
        plugin.source_path,
        result.activation_ready,
        display_text_or_dash(runtime_health),
        display_text_or_dash(attestation),
        display_text_or_dash(plugin.summary.as_deref())
    ));
    lines.push(format!(
        "  policy_summary={} effective_flags={} remediation_classes={} operator_actions={}",
        result.policy_summary, effective_flags, remediation_classes, operator_action_kinds
    ));
    let has_native_extension_projection = native_extension.contract.is_some()
        || native_extension.family.is_some()
        || native_extension.trust_lane.is_some()
        || !native_extension.methods.is_empty()
        || !native_extension.host_hooks.is_empty()
        || !native_extension.tui_surfaces.is_empty()
        || !native_extension.metadata_issues.is_empty();
    if has_native_extension_projection {
        lines.push(format!(
            "  native_extension contract={} family={} trust_lane={} methods={} host_hooks={} tui_surfaces={} metadata_issues={}",
            display_text_or_dash(native_extension.contract.as_deref()),
            display_text_or_dash(native_extension.family.as_deref()),
            display_text_or_dash(native_extension.trust_lane.as_deref()),
            format_csv_or_dash(&native_extension.methods),
            format_csv_or_dash(&native_extension.host_hooks),
            format_csv_or_dash(&native_extension.tui_surfaces),
            format_csv_or_dash(&native_extension.metadata_issues),
        ));
    }
    if let Some(guidance) = plugin_native_extension_authoring_guidance(plugin) {
        lines.push(format!(
            "  authoring validate={} operator_actions={}",
            guidance.validate_command, guidance.operator_actions_command
        ));
        lines.push(format!(
            "  authoring_smoke_test={}",
            guidance.smoke_test_command
        ));
    }
    lines.push(format!(
        "  blocking_diagnostics={} advisory_diagnostics={}",
        blocking_diagnostics, advisory_diagnostics
    ));
    if let Some(reason) = plugin.activation_reason.as_deref() {
        lines.push(format!("  activation_reason={reason}"));
    }
    if recommended_actions != "-" {
        lines.push(format!("  recommended_actions={recommended_actions}"));
    }
    lines
}

fn format_preflight_remediation_classes(
    values: &[crate::PluginPreflightRemediationClass],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    let mut classes = values
        .iter()
        .map(|value| value.as_str().to_owned())
        .collect::<Vec<_>>();
    classes.sort();
    classes.dedup();
    classes.join(",")
}

fn format_preflight_result_operator_action_kinds(
    values: &[crate::PluginPreflightRecommendedAction],
) -> String {
    let kinds = values
        .iter()
        .filter_map(|value| value.operator_action.as_ref())
        .map(|value| value.kind.as_str().to_owned())
        .collect::<BTreeSet<_>>();

    if kinds.is_empty() {
        return "-".to_owned();
    }

    kinds.into_iter().collect::<Vec<_>>().join(",")
}

fn format_preflight_result_recommended_actions(
    values: &[crate::PluginPreflightRecommendedAction],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    let mut rendered = Vec::new();
    for value in values {
        let remediation_class = value.remediation_class.as_str();
        let mut parts = vec![remediation_class.to_owned(), value.summary.clone()];
        if let Some(action) = value.operator_action.as_ref() {
            let kind = action.kind.as_str();
            parts.push(format!("action={kind}"));
        }
        rendered.push(parts.join("|"));
    }
    rendered.join("; ")
}

fn render_plugins_actions_text(execution: &PluginsActionsExecution) -> String {
    let mut lines = vec![format!(
        "plugins actions profile={} query={} roots={} total_actions={} matched_actions={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.total_actions,
        execution.matched_actions
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        execution.summary.policy_source,
        execution.summary.policy_version.as_deref().unwrap_or("-"),
        execution.summary.policy_checksum,
        execution.summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} dialect={} compatibility={} language={} bridge={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.dialect_distribution),
        format_rollup_map(&execution.summary.compatibility_mode_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution)
    ));
    lines.push(format!(
        "filters surface={} kind={} requires_reload={}",
        format_csv_or_dash(&execution.filters.surface),
        format_csv_or_dash(&execution.filters.kind),
        execution
            .filters
            .requires_reload
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned())
    ));
    lines.push(format!(
        "filtered_counts by_surface={} by_kind={} reload={} no_reload={}",
        format_rollup_map(&execution.filtered_action_counts_by_surface),
        format_rollup_map(&execution.filtered_action_counts_by_kind),
        execution.filtered_actions_requiring_reload,
        execution.filtered_actions_without_reload
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    for item in &execution.actions {
        let remediation_summary = item
            .supporting_remediations
            .iter()
            .map(|support| {
                let mut parts = vec![support.remediation_class.clone()];
                if let Some(code) = support.diagnostic_code.as_deref() {
                    parts.push(format!("code={code}"));
                }
                if let Some(field_path) = support.field_path.as_deref() {
                    parts.push(format!("field={field_path}"));
                }
                if support.blocking {
                    parts.push("blocking=true".to_owned());
                }
                parts.join("|")
            })
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!(
            "- action_id={} surface={} kind={} plugin={} provider={} reload={} follow_up={} supports={} blocked={} warned={} passed={}",
            item.action.action_id,
            item.action.surface,
            item.action.kind,
            item.action.target_plugin_id,
            display_text_or_dash(item.action.target_provider_id.as_deref()),
            item.action.requires_reload,
            display_text_or_dash(item.action.follow_up_profile.as_deref()),
            item.supporting_results,
            item.blocked_results,
            item.warned_results,
            item.passed_results
        ));
        lines.push(format!(
            "  source={} manifest={} remediations={}",
            item.action.target_source_path,
            display_text_or_dash(item.action.target_manifest_path.as_deref()),
            remediation_summary
        ));
    }
    lines.join("\n")
}

#[derive(Debug, Clone)]
struct ResolvedPluginScanSource {
    scan_roots: Vec<String>,
    query: String,
    limit: usize,
    bridge_support: Option<ResolvedBridgeSupportSelection>,
}

impl ResolvedPluginScanSource {
    fn bridge_support_source(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .map(|selection| selection.policy.source.clone())
    }

    fn bridge_support_sha256(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .map(|selection| selection.policy.sha256.clone())
    }

    fn bridge_support_delta_source(&self) -> Option<String> {
        self.bridge_support
            .as_ref()
            .and_then(|selection| selection.delta_source.clone())
    }

    fn bridge_support_delta_sha256(&self) -> Option<String> {
        self.bridge_support.as_ref().and_then(|selection| {
            selection
                .delta_artifact
                .as_ref()
                .map(|artifact| artifact.sha256.clone())
        })
    }
}

#[derive(Debug, Clone)]
struct PluginInventoryContext {
    scan_roots: Vec<String>,
    query: String,
    limit: usize,
    bridge_support_source: Option<String>,
    bridge_support_sha256: Option<String>,
    bridge_support_delta_source: Option<String>,
    bridge_support_delta_sha256: Option<String>,
    spec: RunnerSpec,
}

impl PluginInventoryContext {
    fn bridge_support_provenance(&self) -> Option<PluginsBridgeSupportProvenanceView> {
        PluginsBridgeSupportProvenanceView::from_fields(
            self.bridge_support_source.as_deref(),
            self.bridge_support_sha256.as_deref(),
            self.bridge_support_delta_source.as_deref(),
            self.bridge_support_delta_sha256.as_deref(),
        )
    }
}

#[derive(Debug, Clone)]
struct PluginPreflightContext {
    scan_roots: Vec<String>,
    query: String,
    limit: usize,
    profile: String,
    bridge_support_source: Option<String>,
    bridge_support_sha256: Option<String>,
    bridge_support_delta_source: Option<String>,
    bridge_support_delta_sha256: Option<String>,
    spec: RunnerSpec,
}

impl PluginPreflightContext {
    fn bridge_support_provenance(&self) -> Option<PluginsBridgeSupportProvenanceView> {
        PluginsBridgeSupportProvenanceView::from_fields(
            self.bridge_support_source.as_deref(),
            self.bridge_support_sha256.as_deref(),
            self.bridge_support_delta_source.as_deref(),
            self.bridge_support_delta_sha256.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct PluginGovernanceSurfaceContextSpec {
    pack_id: &'static str,
    agent_id: &'static str,
    operator_surface: &'static str,
    surface_label: &'static str,
}

fn build_plugin_inventory_context(
    source: &PluginScanSourceArgs,
    include_ready: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> CliResult<PluginInventoryContext> {
    let default_limit = default_plugin_inventory_limit();
    let resolved = resolve_plugin_scan_source(source, default_limit, 100, "plugins inventory")?;

    let mut spec = RunnerSpec::template();
    spec.pack = VerticalPackManifest {
        pack_id: "plugin-inventory".to_owned(),
        domain: "ops".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
        metadata: BTreeMap::from([("operator_surface".to_owned(), "plugin_inventory".to_owned())]),
    };
    spec.agent_id = "agent-plugin-inventory".to_owned();
    spec.ttl_s = 120;
    spec.approval = Some(HumanApprovalSpec {
        mode: HumanApprovalMode::Disabled,
        ..HumanApprovalSpec::default()
    });
    spec.defaults = None;
    spec.self_awareness = None;
    spec.plugin_scan = Some(PluginScanSpec {
        enabled: true,
        roots: resolved.scan_roots.clone(),
    });
    spec.bridge_support = resolved
        .bridge_support
        .as_ref()
        .map(|selection| selection.policy.profile.clone());
    spec.bootstrap = None;
    spec.auto_provision = None;
    spec.hotfixes = Vec::new();
    spec.operation = OperationSpec::PluginInventory {
        query: resolved.query.clone(),
        limit: resolved.limit,
        include_ready,
        include_blocked,
        include_deferred,
        include_examples,
    };
    let bridge_support_source = resolved.bridge_support_source();
    let bridge_support_sha256 = resolved.bridge_support_sha256();
    let bridge_support_delta_source = resolved.bridge_support_delta_source();
    let bridge_support_delta_sha256 = resolved.bridge_support_delta_sha256();

    Ok(PluginInventoryContext {
        scan_roots: resolved.scan_roots,
        query: resolved.query,
        limit: resolved.limit,
        bridge_support_source,
        bridge_support_sha256,
        bridge_support_delta_source,
        bridge_support_delta_sha256,
        spec,
    })
}

fn build_plugin_doctor_context(
    source: &PluginDoctorSourceArgs,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
) -> CliResult<PluginPreflightContext> {
    let policy_signature = build_policy_signature_spec(
        source.policy_signature_algorithm.as_str(),
        source.policy_signature_public_key_base64.as_deref(),
        source.policy_signature_base64.as_deref(),
    )?;
    let profile = source.profile.as_profile();
    let surface_spec = PluginGovernanceSurfaceContextSpec {
        pack_id: "plugin-doctor",
        agent_id: "agent-plugin-doctor",
        operator_surface: "plugin_doctor",
        surface_label: "plugins doctor",
    };

    build_plugin_preflight_context_from_parts(
        &source.scan,
        profile,
        source.policy_path.clone(),
        source.policy_sha256.clone(),
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        false,
        surface_spec,
    )
}

fn build_plugin_preflight_context(
    source: &PluginGovernanceSourceArgs,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> CliResult<PluginPreflightContext> {
    let policy_signature = build_policy_signature_spec(
        source.policy_signature_algorithm.as_str(),
        source.policy_signature_public_key_base64.as_deref(),
        source.policy_signature_base64.as_deref(),
    )?;
    let profile = source.profile.as_profile();
    let surface_spec = PluginGovernanceSurfaceContextSpec {
        pack_id: "plugin-governance",
        agent_id: "agent-plugin-governance",
        operator_surface: "plugin_governance",
        surface_label: "plugins governance",
    };

    build_plugin_preflight_context_from_parts(
        &source.scan,
        profile,
        source.policy_path.clone(),
        source.policy_sha256.clone(),
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        include_examples,
        surface_spec,
    )
}

fn build_plugin_preflight_context_from_parts(
    scan: &PluginScanSourceArgs,
    profile: PluginPreflightProfile,
    policy_path: Option<String>,
    policy_sha256: Option<String>,
    policy_signature: Option<SecurityProfileSignatureSpec>,
    include_passed: bool,
    include_warned: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
    surface_spec: PluginGovernanceSurfaceContextSpec,
) -> CliResult<PluginPreflightContext> {
    let default_limit = default_plugin_preflight_limit();
    let resolved =
        resolve_plugin_scan_source(scan, default_limit, 500, surface_spec.surface_label)?;

    let mut spec = RunnerSpec::template();
    spec.pack = VerticalPackManifest {
        pack_id: surface_spec.pack_id.to_owned(),
        domain: "ops".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
        metadata: BTreeMap::from([(
            "operator_surface".to_owned(),
            surface_spec.operator_surface.to_owned(),
        )]),
    };
    spec.agent_id = surface_spec.agent_id.to_owned();
    spec.ttl_s = 120;
    spec.approval = Some(HumanApprovalSpec {
        mode: HumanApprovalMode::Disabled,
        ..HumanApprovalSpec::default()
    });
    spec.defaults = None;
    spec.self_awareness = None;
    spec.plugin_scan = Some(PluginScanSpec {
        enabled: true,
        roots: resolved.scan_roots.clone(),
    });
    spec.bridge_support = resolved
        .bridge_support
        .as_ref()
        .map(|selection| selection.policy.profile.clone());
    spec.bootstrap = None;
    spec.auto_provision = None;
    spec.hotfixes = Vec::new();
    spec.operation = OperationSpec::PluginPreflight {
        query: resolved.query.clone(),
        limit: resolved.limit,
        profile,
        policy_path,
        policy_sha256,
        policy_signature,
        include_passed,
        include_warned,
        include_blocked,
        include_deferred,
        include_examples,
    };
    let bridge_support_source = resolved.bridge_support_source();
    let bridge_support_sha256 = resolved.bridge_support_sha256();
    let bridge_support_delta_source = resolved.bridge_support_delta_source();
    let bridge_support_delta_sha256 = resolved.bridge_support_delta_sha256();

    Ok(PluginPreflightContext {
        scan_roots: resolved.scan_roots,
        query: resolved.query,
        limit: resolved.limit,
        profile: profile.as_str().to_owned(),
        bridge_support_source,
        bridge_support_sha256,
        bridge_support_delta_source,
        bridge_support_delta_sha256,
        spec,
    })
}

fn resolve_plugin_scan_source(
    source: &PluginScanSourceArgs,
    default_limit: usize,
    max_limit: usize,
    surface_label: &str,
) -> CliResult<ResolvedPluginScanSource> {
    let roots = normalize_scan_roots(&source.roots, surface_label)?;
    let requested_limit = source.limit.unwrap_or(default_limit);
    let limit = validate_plugin_limit(requested_limit, max_limit, surface_label)?;
    let bridge_support = resolve_bridge_support_selection(
        source.bridge_support.as_deref(),
        source.bridge_profile.map(PluginBridgeProfileArg::as_str),
        source.bridge_support_delta.as_deref(),
        source.bridge_support_sha256.as_deref(),
        source.bridge_support_delta_sha256.as_deref(),
    )?;

    Ok(ResolvedPluginScanSource {
        scan_roots: roots,
        query: source.query.clone(),
        limit,
        bridge_support,
    })
}

fn load_bridge_profile_views(
    requested: &[PluginBridgeProfileArg],
) -> CliResult<Vec<PluginsBridgeProfileExecutionView>> {
    let requested = if requested.is_empty() {
        vec![
            PluginBridgeProfileArg::NativeBalanced,
            PluginBridgeProfileArg::OpenclawEcosystemBalanced,
        ]
    } else {
        requested.to_vec()
    };

    let mut views = Vec::new();
    let mut seen = BTreeSet::new();
    for profile in requested {
        let profile_id = profile.as_str();
        if !seen.insert(profile_id.to_owned()) {
            continue;
        }
        let resolved =
            resolve_bridge_support_policy(None, Some(profile_id), None)?.ok_or_else(|| {
                format!("bundled bridge support profile `{profile_id}` was not resolved")
            })?;
        let mut supported_bridges = resolved
            .profile
            .supported_bridges
            .iter()
            .map(|bridge| bridge.as_str().to_owned())
            .collect::<Vec<_>>();
        supported_bridges.sort();

        let mut supported_compatibility_modes = resolved
            .profile
            .supported_compatibility_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect::<Vec<_>>();
        supported_compatibility_modes.sort();

        let mut supported_compatibility_shims = resolved
            .profile
            .supported_compatibility_shims
            .iter()
            .map(|shim| format!("{}:{}", shim.shim_id, shim.family))
            .collect::<Vec<_>>();
        supported_compatibility_shims.sort();

        let mut shim_support_profiles = resolved
            .profile
            .supported_compatibility_shim_profiles
            .iter()
            .map(|profile| {
                let mut supported_dialects = profile
                    .supported_dialects
                    .iter()
                    .map(|dialect| dialect.as_str().to_owned())
                    .collect::<Vec<_>>();
                supported_dialects.sort();

                let mut supported_bridges = profile
                    .supported_bridges
                    .iter()
                    .map(|bridge| bridge.as_str().to_owned())
                    .collect::<Vec<_>>();
                supported_bridges.sort();

                let mut supported_adapter_families = profile
                    .supported_adapter_families
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                supported_adapter_families.sort();

                let mut supported_source_languages = profile
                    .supported_source_languages
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                supported_source_languages.sort();

                PluginsBridgeShimSupportProfileView {
                    shim_id: profile.shim.shim_id.clone(),
                    shim_family: profile.shim.family.clone(),
                    version: profile.version.clone(),
                    supported_dialects,
                    supported_bridges,
                    supported_adapter_families,
                    supported_source_languages,
                }
            })
            .collect::<Vec<_>>();
        shim_support_profiles.sort_by(|left, right| {
            (
                left.shim_id.as_str(),
                left.shim_family.as_str(),
                left.version.as_deref().unwrap_or_default(),
            )
                .cmp(&(
                    right.shim_id.as_str(),
                    right.shim_family.as_str(),
                    right.version.as_deref().unwrap_or_default(),
                ))
        });

        views.push(PluginsBridgeProfileExecutionView {
            profile_id: profile_id.to_owned(),
            source: resolved.source,
            policy_version: resolved.profile.policy_version.clone(),
            checksum: resolved.checksum,
            sha256: resolved.sha256,
            supported_bridges,
            supported_compatibility_modes,
            supported_compatibility_shims,
            shim_support_profiles,
            execute_process_stdio: resolved.profile.execute_process_stdio,
            execute_http_json: resolved.profile.execute_http_json,
            enforce_supported: resolved.profile.enforce_supported,
            enforce_execution_success: resolved.profile.enforce_execution_success,
        });
    }

    Ok(views)
}

fn normalize_scan_roots(roots: &[String], surface_label: &str) -> CliResult<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_owned()) {
            normalized.push(trimmed.to_owned());
        }
    }
    if normalized.is_empty() {
        return Err(format!(
            "{surface_label} requires at least one non-empty --root"
        ));
    }
    Ok(normalized)
}

fn validate_plugin_limit(limit: usize, max_limit: usize, surface_label: &str) -> CliResult<usize> {
    if !(1..=max_limit).contains(&limit) {
        return Err(format!(
            "{surface_label} limit must be between 1 and {max_limit}"
        ));
    }
    Ok(limit)
}

fn build_policy_signature_spec(
    algorithm: &str,
    public_key_base64: Option<&str>,
    signature_base64: Option<&str>,
) -> CliResult<Option<SecurityProfileSignatureSpec>> {
    match (public_key_base64, signature_base64) {
        (None, None) => Ok(None),
        (Some(_), None) => {
            Err("plugins governance policy signature requires --policy-signature-base64".to_owned())
        }
        (None, Some(_)) => Err(
            "plugins governance policy signature requires --policy-signature-public-key-base64"
                .to_owned(),
        ),
        (Some(public_key_base64), Some(signature_base64)) => {
            Ok(Some(SecurityProfileSignatureSpec {
                algorithm: algorithm.to_owned(),
                public_key_base64: public_key_base64.to_owned(),
                signature_base64: signature_base64.to_owned(),
            }))
        }
    }
}

fn decode_preflight_bridge_profile_recommendation(
    report: &SpecRunReport,
) -> CliResult<Option<PluginPreflightBridgeProfileRecommendation>> {
    let recommendation_value = report
        .outcome
        .get("summary")
        .and_then(|summary| summary.get("bridge_profile_recommendation"))
        .cloned()
        .unwrap_or(Value::Null);

    serde_json::from_value(recommendation_value).map_err(|error| {
        format!("decode plugin preflight bridge profile recommendation failed: {error}")
    })
}

fn decode_plugin_inventory_results(
    report: &SpecRunReport,
) -> CliResult<Vec<PluginInventoryResult>> {
    let results_value = report
        .outcome
        .get("results")
        .cloned()
        .unwrap_or(Value::Null);

    serde_json::from_value(results_value)
        .map_err(|error| format!("decode plugin inventory results failed: {error}"))
}

fn summarize_plugin_inventory_results(
    results: &[PluginInventoryResult],
) -> PluginsInventorySummaryView {
    let returned_plugins = results.len();
    let mut ready_plugins = 0;
    let mut setup_incomplete_plugins = 0;
    let mut blocked_plugins = 0;
    let mut deferred_plugins = 0;
    let mut loaded_plugins = 0;
    let mut source_kind_distribution = BTreeMap::new();
    let mut bridge_kind_distribution = BTreeMap::new();
    let mut capability_distribution = BTreeMap::new();
    let mut source_language_distribution = BTreeMap::new();
    let mut setup_surface_distribution = BTreeMap::new();
    let mut activation_status_distribution = BTreeMap::new();

    for result in results {
        let activation_status = result.activation_status.as_deref();

        if activation_status == Some("ready") {
            ready_plugins += 1;
        }
        if activation_status == Some("setup_incomplete") {
            setup_incomplete_plugins += 1;
        }
        if activation_status.is_some_and(plugin_inventory_status_is_blocked) {
            blocked_plugins += 1;
        }
        if result.deferred {
            deferred_plugins += 1;
        }
        if result.loaded {
            loaded_plugins += 1;
        }

        increment_rollup_count(&mut source_kind_distribution, result.source_kind.as_str());
        increment_rollup_count(&mut bridge_kind_distribution, result.bridge_kind.as_str());
        for capability in &result.capabilities {
            increment_rollup_count(&mut capability_distribution, capability.as_str());
        }

        let source_language = result.source_language.as_deref().unwrap_or("unknown");
        increment_rollup_count(&mut source_language_distribution, source_language);

        let setup_surface = inventory_result_setup_surface_label(result);
        increment_rollup_count(&mut setup_surface_distribution, setup_surface);

        let status_label = inventory_result_status_label(result);
        increment_rollup_count(&mut activation_status_distribution, status_label);
    }

    PluginsInventorySummaryView {
        returned_plugins,
        ready_plugins,
        setup_incomplete_plugins,
        blocked_plugins,
        deferred_plugins,
        loaded_plugins,
        source_kind_distribution,
        bridge_kind_distribution,
        capability_distribution,
        source_language_distribution,
        setup_surface_distribution,
        activation_status_distribution,
    }
}

fn plugin_inventory_status_is_blocked(status: &str) -> bool {
    if status == "ready" {
        return false;
    }

    if status == "setup_incomplete" {
        return false;
    }

    true
}

fn inventory_result_status_label(result: &PluginInventoryResult) -> &str {
    let activation_status = result.activation_status.as_deref();
    let has_activation_status = activation_status.is_some_and(|status| !status.is_empty());

    if has_activation_status {
        return activation_status.unwrap_or("unknown");
    }

    if result.deferred {
        return "deferred";
    }

    "unknown"
}

fn inventory_result_setup_surface_label(result: &PluginInventoryResult) -> &str {
    let setup_surface = result.setup_surface.as_deref();
    let has_setup_surface = setup_surface.is_some_and(|value| !value.is_empty());

    if has_setup_surface {
        return setup_surface.unwrap_or("none");
    }

    let setup_mode = result.setup_mode.as_deref();
    let has_setup_mode = setup_mode.is_some_and(|value| !value.is_empty());

    if has_setup_mode {
        return "unspecified";
    }

    "none"
}

fn increment_rollup_count(values: &mut BTreeMap<String, usize>, key: &str) {
    let entry = values.entry(key.to_owned()).or_default();
    let next_value = entry.saturating_add(1);
    *entry = next_value;
}

impl PluginsBridgeSupportProvenanceView {
    fn from_fields(
        source: Option<&str>,
        sha256: Option<&str>,
        delta_source: Option<&str>,
        delta_sha256: Option<&str>,
    ) -> Option<Self> {
        if source.is_none() && sha256.is_none() && delta_source.is_none() && delta_sha256.is_none()
        {
            return None;
        }

        Some(Self {
            source: source.map(str::to_owned),
            sha256: sha256.map(str::to_owned),
            delta_source: delta_source.map(str::to_owned),
            delta_sha256: delta_sha256.map(str::to_owned),
        })
    }
}

fn decode_preflight_summary(
    report: &SpecRunReport,
    bridge_support_provenance: Option<PluginsBridgeSupportProvenanceView>,
) -> CliResult<PluginsPreflightSummaryView> {
    let summary_value = report
        .outcome
        .get("summary")
        .cloned()
        .ok_or_else(|| "decode plugin preflight summary failed: missing summary".to_owned())?;
    let mut summary: PluginsPreflightSummaryView = serde_json::from_value(summary_value)
        .map_err(|error| format!("decode plugin preflight summary failed: {error}"))?;
    summary.bridge_support_provenance = bridge_support_provenance;
    Ok(summary)
}

fn decode_preflight_results(report: &SpecRunReport) -> CliResult<Vec<PluginPreflightResult>> {
    let results_value = report
        .outcome
        .get("results")
        .cloned()
        .unwrap_or(Value::Null);

    serde_json::from_value(results_value)
        .map_err(|error| format!("decode plugin preflight results failed: {error}"))
}

fn summarize_plugin_doctor_results(
    results: &[PluginPreflightResult],
    preflight_summary: &PluginsPreflightSummaryView,
) -> PluginsDoctorSummaryView {
    let mut activation_ready_plugins: usize = 0;
    let mut setup_incomplete_plugins: usize = 0;
    let mut deferred_plugins: usize = 0;
    let mut loaded_plugins: usize = 0;
    let mut packages_with_operator_actions: usize = 0;
    let mut total_recommended_actions: usize = 0;
    let mut total_operator_actions: usize = 0;
    let mut bridge_kind_distribution = BTreeMap::new();
    let mut capability_distribution = BTreeMap::new();
    let mut source_language_distribution = BTreeMap::new();
    let mut setup_surface_distribution = BTreeMap::new();
    let mut activation_status_distribution = BTreeMap::new();

    for result in results {
        let plugin = &result.plugin;

        if result.activation_ready {
            activation_ready_plugins = activation_ready_plugins.saturating_add(1);
        }

        if plugin.activation_status.as_deref() == Some("setup_incomplete") {
            setup_incomplete_plugins = setup_incomplete_plugins.saturating_add(1);
        }

        if plugin.deferred {
            deferred_plugins = deferred_plugins.saturating_add(1);
        }

        if plugin.loaded {
            loaded_plugins = loaded_plugins.saturating_add(1);
        }

        let recommended_action_count = result.recommended_actions.len();
        total_recommended_actions =
            total_recommended_actions.saturating_add(recommended_action_count);

        let operator_action_count = count_preflight_result_operator_actions(result);
        total_operator_actions = total_operator_actions.saturating_add(operator_action_count);

        if operator_action_count > 0 {
            packages_with_operator_actions = packages_with_operator_actions.saturating_add(1);
        }

        increment_rollup_count(&mut bridge_kind_distribution, plugin.bridge_kind.as_str());
        for capability in &plugin.capabilities {
            increment_rollup_count(&mut capability_distribution, capability.as_str());
        }

        let source_language = plugin.source_language.as_deref().unwrap_or("unknown");
        increment_rollup_count(&mut source_language_distribution, source_language);

        let setup_surface = inventory_result_setup_surface_label(plugin);
        increment_rollup_count(&mut setup_surface_distribution, setup_surface);

        let activation_status = inventory_result_status_label(plugin);
        increment_rollup_count(&mut activation_status_distribution, activation_status);
    }

    let packages_requiring_author_attention = preflight_summary
        .warned_plugins
        .saturating_add(preflight_summary.blocked_plugins);

    PluginsDoctorSummaryView {
        matched_plugins: preflight_summary.matched_plugins,
        returned_plugins: results.len(),
        passed_plugins: preflight_summary.passed_plugins,
        warned_plugins: preflight_summary.warned_plugins,
        blocked_plugins: preflight_summary.blocked_plugins,
        activation_ready_plugins,
        setup_incomplete_plugins,
        deferred_plugins,
        loaded_plugins,
        packages_requiring_author_attention,
        packages_with_operator_actions,
        total_recommended_actions,
        total_operator_actions,
        remediation_counts: preflight_summary.remediation_counts.clone(),
        bridge_kind_distribution,
        capability_distribution,
        source_language_distribution,
        setup_surface_distribution,
        activation_status_distribution,
    }
}

fn count_preflight_result_operator_actions(result: &PluginPreflightResult) -> usize {
    let mut count = 0_usize;
    for action in &result.recommended_actions {
        if action.operator_action.is_some() {
            count = count.saturating_add(1);
        }
    }
    count
}

fn action_matches_filters(
    item: &PluginsActionPlanItemView,
    filters: &PluginActionFiltersView,
) -> bool {
    (filters.surface.is_empty()
        || filters
            .surface
            .iter()
            .any(|surface| surface == &item.action.surface))
        && (filters.kind.is_empty() || filters.kind.iter().any(|kind| kind == &item.action.kind))
        && filters
            .requires_reload
            .is_none_or(|requires_reload| item.action.requires_reload == requires_reload)
}

fn summarize_filtered_actions(
    actions: &[PluginsActionPlanItemView],
) -> (
    BTreeMap<String, usize>,
    BTreeMap<String, usize>,
    usize,
    usize,
) {
    let mut by_surface = BTreeMap::new();
    let mut by_kind = BTreeMap::new();
    let mut requiring_reload = 0_usize;
    let mut without_reload = 0_usize;
    for item in actions {
        *by_surface.entry(item.action.surface.clone()).or_default() += 1;
        *by_kind.entry(item.action.kind.clone()).or_default() += 1;
        if item.action.requires_reload {
            requiring_reload = requiring_reload.saturating_add(1);
        } else {
            without_reload = without_reload.saturating_add(1);
        }
    }
    (by_surface, by_kind, requiring_reload, without_reload)
}

fn display_text_or_dash(value: Option<&str>) -> &str {
    match value {
        Some(value) if !value.is_empty() => value,
        _ => "-",
    }
}

fn format_csv_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn format_rollup_map(values: &BTreeMap<String, usize>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn write_bridge_support_template(path: &str, template: &BridgeSupportSpec) -> CliResult<()> {
    let rendered = serde_json::to_string_pretty(template)
        .map_err(|error| format!("serialize bridge support template failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create bridge template parent directory `{}` failed: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, rendered)
        .map_err(|error| format!("write bridge support template `{path}` failed: {error}"))
}

fn write_bridge_support_delta_artifact(
    path: &str,
    artifact: &MaterializedBridgeSupportDeltaArtifact,
) -> CliResult<()> {
    let rendered = serde_json::to_string_pretty(artifact)
        .map_err(|error| format!("serialize bridge support delta artifact failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create bridge delta parent directory `{}` failed: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, rendered)
        .map_err(|error| format!("write bridge support delta artifact `{path}` failed: {error}"))
}

fn render_bridge_profile_fit_lines(summary: &PluginsPreflightSummaryView) -> Vec<String> {
    let mut lines = vec![format!(
        "bridge_profiles active={} recommended={} recommended_source={} active_matches={} active_support_fits_all={}",
        display_text_or_dash(summary.active_bridge_profile.as_deref()),
        display_text_or_dash(summary.recommended_bridge_profile.as_deref()),
        display_text_or_dash(summary.recommended_bridge_profile_source.as_deref()),
        summary
            .active_bridge_profile_matches_recommended
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        summary
            .active_bridge_support_fits_all_plugins
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned())
    )];

    for fit in &summary.bridge_profile_fits {
        lines.push(format!(
            "bridge_profile_fit profile={} version={} fits_all={} supported={} blocked={} reasons={} sample_blocked_plugins={}",
            fit.profile_id,
            display_text_or_dash(fit.policy_version.as_deref()),
            fit.fits_all_plugins,
            fit.supported_plugins,
            fit.blocked_plugins,
            format_rollup_map(&fit.blocking_reasons),
            format_csv_or_dash(&fit.sample_blocked_plugins)
        ));
    }

    if let Some(recommendation) = summary.bridge_profile_recommendation.as_ref() {
        lines.push(format!(
            "bridge_profile_recommendation kind={} target={} source={} version={} summary={}",
            recommendation.kind,
            recommendation.target_profile_id,
            recommendation.target_profile_source,
            display_text_or_dash(recommendation.target_policy_version.as_deref()),
            recommendation.summary
        ));
        if let Some(delta) = recommendation.delta.as_ref() {
            lines.push(format!(
                "bridge_profile_delta bridges={} compatibility={} adapter_families={} shims={} shim_profiles={} unresolved={}",
                format_csv_or_dash(&delta.supported_bridges),
                format_csv_or_dash(&delta.supported_compatibility_modes),
                format_csv_or_dash(&delta.supported_adapter_families),
                format_csv_or_dash(&delta.supported_compatibility_shims),
                format_shim_profile_deltas(&delta.shim_profile_additions),
                format_csv_or_dash(&delta.unresolved_blocking_reasons)
            ));
        }
    }

    lines
}

fn format_shim_profile_deltas(values: &[PluginsBridgeShimProfileDeltaView]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}:dialects={}|bridges={}|adapter_families={}|languages={}",
                value.shim_id,
                value.shim_family,
                format_csv_or_dash(&value.supported_dialects),
                format_csv_or_dash(&value.supported_bridges),
                format_csv_or_dash(&value.supported_adapter_families),
                format_csv_or_dash(&value.supported_source_languages)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn format_bridge_shim_profile_delta_artifact(
    values: &[crate::PluginPreflightBridgeShimProfileDelta],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}:dialects={}|bridges={}|adapter_families={}|languages={}",
                value.shim_id,
                value.shim_family,
                format_csv_or_dash(&value.supported_dialects),
                format_csv_or_dash(&value.supported_bridges),
                format_csv_or_dash(&value.supported_adapter_families),
                format_csv_or_dash(&value.supported_source_languages)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE, PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE,
        PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION,
    };
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("{prefix}-{nanos}"))
            .display()
            .to_string()
    }

    #[test]
    fn wrap_plugins_surface_text_uses_operator_header() {
        let rendered = wrap_plugins_surface_text("plugins inventory", "plugin=demo".to_owned());

        assert!(
            rendered
                .lines()
                .any(|line| line.starts_with("LOONG") || line.contains(" loong ")),
            "plugins text should use the shared ratatui operator shell header: {rendered}"
        );
        assert!(rendered.contains("plugins inventory"));
        assert!(rendered.contains("plugin=demo"));
    }

    fn write_openclaw_weather_sdk_package(plugin_root: &str) {
        let package_root = format!("{plugin_root}/weather-sdk");
        fs::create_dir_all(format!("{package_root}/dist")).expect("create package root");
        fs::write(
            format!("{package_root}/openclaw.plugin.json"),
            r#"
{
  "id": "weather-sdk",
  "name": "Weather SDK",
  "description": "OpenClaw weather integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["weather"],
  "channels": ["weather"],
  "skills": ["forecast"],
  "configSchema": {}
}
"#,
        )
        .expect("write openclaw manifest");
        fs::write(
            format!("{package_root}/package.json"),
            r#"
{
  "name": "@acme/weather-sdk",
  "version": "1.2.3",
  "description": "Weather provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js",
    "channel": {
      "id": "weather",
      "label": "Weather",
      "aliases": ["forecast"]
    }
  }
}
"#,
        )
        .expect("write package json");
        fs::write(format!("{package_root}/dist/index.js"), "export {};\n").expect("write entry");
        fs::write(format!("{package_root}/dist/setup.js"), "export {};\n")
            .expect("write setup entry");
    }

    fn write_openclaw_weather_sdk_python_package(plugin_root: &str) {
        let package_root = format!("{plugin_root}/weather-sdk");
        fs::create_dir_all(format!("{package_root}/dist")).expect("create package root");
        fs::write(
            format!("{package_root}/openclaw.plugin.json"),
            r#"
{
  "id": "weather-sdk",
  "name": "Weather SDK",
  "description": "OpenClaw weather integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["weather"],
  "channels": ["weather"],
  "skills": ["forecast"],
  "configSchema": {}
}
"#,
        )
        .expect("write openclaw manifest");
        fs::write(
            format!("{package_root}/package.json"),
            r#"
{
  "name": "@acme/weather-sdk",
  "version": "1.2.3",
  "description": "Weather provider package",
  "openclaw": {
    "extensions": ["dist/index.py"],
    "setupEntry": "dist/setup.py",
    "channel": {
      "id": "weather",
      "label": "Weather",
      "aliases": ["forecast"]
    }
  }
}
"#,
        )
        .expect("write package json");
        fs::write(
            format!("{package_root}/dist/index.py"),
            "def invoke():\n    return {}\n",
        )
        .expect("write entry");
        fs::write(
            format!("{package_root}/dist/setup.py"),
            "def setup():\n    return {}\n",
        )
        .expect("write setup entry");
    }

    fn write_host_hook_declared_native_extension_package(package_root: &str) {
        fs::create_dir_all(package_root).expect("create host-hook package root");
        let args_json = serde_json::to_string(&vec![format!("{package_root}/index.js")])
            .expect("serialize host-hook args");
        let manifest = serde_json::json!({
            "api_version": "v1alpha1",
            "version": "0.1.0",
            "plugin_id": "host-hook-extension",
            "provider_id": "host-hook-extension",
            "connector_name": "host-hook-extension",
            "capabilities": ["InvokeConnector"],
            "metadata": {
                "bridge_kind": "process_stdio",
                "adapter_family": "javascript-stdio-adapter",
                "entrypoint": "stdin/stdout::invoke",
                "source_language": "javascript",
                "command": "node",
                "args_json": args_json,
                "process_timeout_ms": "15000",
                "loong_extension_contract": "process_stdio_json_line_v1",
                "loong_extension_family": "governed_native_runtime_extension",
                "loong_extension_trust_lane": "governed_sidecar",
                "loong_extension_methods_json": "[\"extension/event\"]",
                "loong_extension_host_hooks_json": "[\"turn_start\",\"turn_end\"]"
            },
            "summary": "Reserved host hook declaration example"
        });
        fs::write(
            format!("{package_root}/loong.plugin.json"),
            serde_json::to_string_pretty(&manifest).expect("serialize host-hook manifest"),
        )
        .expect("write host-hook package manifest");
        crate::test_support::write_executable_script_atomically(
            Path::new(&format!("{package_root}/index.js")),
            "#!/usr/bin/env node\nprocess.stdin.resume();\n",
        );
    }

    fn write_trusted_host_extension_package(package_root: &str) {
        fs::create_dir_all(package_root).expect("create trusted-host package root");
        let args_json = serde_json::to_string(&vec![format!("{package_root}/index.js")])
            .expect("serialize trusted-host args");
        let manifest = serde_json::json!({
            "api_version": "v1alpha1",
            "version": "0.1.0",
            "plugin_id": "trusted-host-extension",
            "provider_id": "trusted-host-extension",
            "connector_name": "trusted-host-extension",
            "capabilities": ["InvokeConnector"],
            "metadata": {
                "bridge_kind": "process_stdio",
                "adapter_family": "javascript-stdio-adapter",
                "entrypoint": "stdin/stdout::invoke",
                "source_language": "javascript",
                "command": "node",
                "args_json": args_json,
                "process_timeout_ms": "15000",
                "loong_extension_contract": "process_stdio_json_line_v1",
                "loong_extension_family": "trusted_host_extension",
                "loong_extension_trust_lane": "trusted_host",
                "loong_extension_methods_json": "[\"extension/event\"]",
                "loong_extension_host_hooks_json": "[\"turn_start\",\"turn_end\"]",
                "loong_extension_tui_surfaces_json": "[\"command_palette\"]"
            },
            "summary": "Trusted host read-only hook probe example"
        });
        fs::write(
            format!("{package_root}/loong.plugin.json"),
            serde_json::to_string_pretty(&manifest).expect("serialize trusted-host manifest"),
        )
        .expect("write trusted-host package manifest");
        crate::test_support::write_executable_script_atomically(
            Path::new(&format!("{package_root}/index.js")),
            "#!/usr/bin/env node\nfunction buildExtensionPayload(operation, payload) {\n  if (operation === 'extension/event') {\n    return { ok: true, handled_event: payload.event ?? 'unknown', handled_hook: payload.host_hook ?? 'unknown', handled_tui_surface: payload.host_tui_surface ?? 'unknown', received_hook_payload: payload.hook_payload ?? null, received_surface_payload: payload.surface_payload ?? null };\n  }\n  return { error: `unsupported method: ${operation}` };\n}\nfunction emitResponse(line) {\n  const trimmed = line.trim();\n  if (!trimmed) return;\n  const request = JSON.parse(trimmed);\n  const payload = request.payload ?? {};\n  const response = { method: request.method ?? '', id: request.id ?? null, payload: buildExtensionPayload(payload.operation ?? '', payload.payload ?? {}) };\n  process.stdout.write(`${JSON.stringify(response)}\\n`);\n}\nprocess.stdin.setEncoding('utf8');\nlet buffered = '';\nprocess.stdin.on('data', (chunk) => { buffered += chunk; let newlineIndex = buffered.indexOf('\\n'); while (newlineIndex !== -1) { const line = buffered.slice(0, newlineIndex); buffered = buffered.slice(newlineIndex + 1); emitResponse(line); newlineIndex = buffered.indexOf('\\n'); } });\nprocess.stdin.on('end', () => { if (buffered.trim()) emitResponse(buffered); });\nprocess.stdin.resume();\n",
        );
    }

    fn plugin_scan_source(plugin_root: &str, query: &str) -> PluginScanSourceArgs {
        PluginScanSourceArgs {
            roots: vec![plugin_root.to_owned()],
            query: query.to_owned(),
            limit: Some(10),
            bridge_support: None,
            bridge_profile: None,
            bridge_support_delta: None,
            bridge_support_sha256: None,
            bridge_support_delta_sha256: None,
        }
    }

    fn plugin_governance_source(plugin_root: &str, query: &str) -> PluginGovernanceSourceArgs {
        PluginGovernanceSourceArgs {
            scan: plugin_scan_source(plugin_root, query),
            profile: PluginPreflightProfileArg::RuntimeActivation,
            policy_path: None,
            policy_sha256: None,
            policy_signature_public_key_base64: None,
            policy_signature_base64: None,
            policy_signature_algorithm: "ed25519".to_owned(),
        }
    }

    fn plugin_doctor_source(plugin_root: &str, query: &str) -> PluginDoctorSourceArgs {
        PluginDoctorSourceArgs {
            scan: plugin_scan_source(plugin_root, query),
            profile: PluginPreflightProfileArg::SdkRelease,
            policy_path: None,
            policy_sha256: None,
            policy_signature_public_key_base64: None,
            policy_signature_base64: None,
            policy_signature_algorithm: "ed25519".to_owned(),
        }
    }

    #[test]
    fn build_policy_signature_spec_requires_complete_pair() {
        let error = build_policy_signature_spec("ed25519", Some("pub"), None)
            .expect_err("incomplete signature should fail");
        assert!(error.contains("--policy-signature-base64"));

        let error = build_policy_signature_spec("ed25519", None, Some("sig"))
            .expect_err("missing public key should fail");
        assert!(error.contains("--policy-signature-public-key-base64"));
    }

    #[test]
    fn normalize_scan_roots_deduplicates_and_rejects_empty_input() {
        let roots = normalize_scan_roots(
            &[
                " /tmp/a ".to_owned(),
                "/tmp/a".to_owned(),
                "  ".to_owned(),
                "/tmp/b".to_owned(),
            ],
            "plugins inventory",
        )
        .expect("roots should normalize");
        assert_eq!(roots, vec!["/tmp/a".to_owned(), "/tmp/b".to_owned()]);

        let error = normalize_scan_roots(&["   ".to_owned()], "plugins inventory")
            .expect_err("empty roots should fail");
        assert!(error.contains("--root"));
    }

    #[test]
    fn summarize_filtered_actions_counts_surface_kind_and_reload() {
        let action = PluginsActionPlanItemView {
            action: PluginsActionView {
                action_id: "a".repeat(64),
                surface: "host_runtime".to_owned(),
                kind: "quarantine_loaded_provider".to_owned(),
                target_plugin_id: "sample".to_owned(),
                target_provider_id: Some("sample".to_owned()),
                target_source_path: "/tmp/sample".to_owned(),
                target_manifest_path: None,
                follow_up_profile: None,
                requires_reload: true,
            },
            supporting_results: 1,
            blocked_results: 1,
            warned_results: 0,
            passed_results: 0,
            supporting_remediations: Vec::new(),
        };
        let (by_surface, by_kind, requiring_reload, without_reload) =
            summarize_filtered_actions(&[action]);
        assert_eq!(by_surface.get("host_runtime").copied(), Some(1));
        assert_eq!(by_kind.get("quarantine_loaded_provider").copied(), Some(1));
        assert_eq!(requiring_reload, 1);
        assert_eq!(without_reload, 0);
    }

    #[tokio::test]
    async fn execute_plugins_bridge_profiles_returns_bundled_profiles() {
        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::BridgeProfiles(PluginBridgeProfilesCommand {
                profiles: vec![PluginBridgeProfileArg::OpenclawEcosystemBalanced],
            }),
        })
        .await
        .expect("plugins bridge-profiles should execute");

        let PluginsCommandExecution::BridgeProfiles(execution) = execution else {
            panic!("expected bridge profiles execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(
            execution.schema.purpose,
            PLUGINS_BRIDGE_PROFILES_SCHEMA_PURPOSE
        );
        assert_eq!(execution.returned_profiles, 1);
        assert_eq!(
            execution.profiles[0].profile_id,
            "openclaw-ecosystem-balanced"
        );
        assert_eq!(
            execution.profiles[0].source,
            "bundled:bridge-support-openclaw-ecosystem-balanced.json"
        );
        assert!(
            execution.profiles[0]
                .supported_compatibility_modes
                .iter()
                .any(|mode| mode == "openclaw_modern")
        );
        assert!(
            execution.profiles[0]
                .shim_support_profiles
                .iter()
                .any(|profile| {
                    profile.shim_id == "openclaw-modern-compat"
                        && profile
                            .supported_source_languages
                            .iter()
                            .any(|language| language == "typescript")
                })
        );
    }

    #[tokio::test]
    async fn execute_plugins_inventory_surfaces_manifest_first_openclaw_package_truth() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-inventory-openclaw");
        write_openclaw_weather_sdk_package(&plugin_root);

        let mut source = plugin_scan_source(&plugin_root, "weather-sdk");
        source.limit = None;
        source.bridge_profile = Some(PluginBridgeProfileArg::OpenclawEcosystemBalanced);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Inventory(PluginInventoryCommand {
                source,
                include_ready: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins inventory should execute");

        let PluginsCommandExecution::Inventory(execution) = execution else {
            panic!("expected inventory execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_INVENTORY_SCHEMA_PURPOSE);
        assert_eq!(execution.limit, default_plugin_inventory_limit());
        assert_eq!(execution.returned_results, 1);
        assert_eq!(execution.summary.returned_plugins, 1);
        assert_eq!(execution.summary.ready_plugins, 0);
        assert_eq!(execution.summary.setup_incomplete_plugins, 1);
        assert_eq!(execution.summary.blocked_plugins, 0);
        assert_eq!(execution.summary.deferred_plugins, 1);
        assert_eq!(execution.summary.loaded_plugins, 0);
        assert_eq!(
            execution
                .summary
                .bridge_kind_distribution
                .get("process_stdio")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .source_language_distribution
                .get("javascript")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .setup_surface_distribution
                .get("channel")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .activation_status_distribution
                .get("setup_incomplete")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution.bridge_support_source.as_deref(),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        let result = &execution.results[0];
        assert_eq!(result.plugin_id, "weather-sdk");
        assert_eq!(result.provider_id, "weather-sdk");
        assert_eq!(result.bridge_kind, "process_stdio");
        assert_eq!(result.source_language.as_deref(), Some("javascript"));
        assert_eq!(result.setup_mode.as_deref(), Some("governed_entry"));
        assert_eq!(result.setup_surface.as_deref(), Some("channel"));
        assert_eq!(
            result.activation_status.as_deref(),
            Some("setup_incomplete")
        );
        assert!(result.deferred);
        assert!(
            result
                .setup_required_config_keys
                .iter()
                .any(|key| key == "plugins.entries.weather-sdk")
        );
    }

    #[tokio::test]
    async fn execute_plugins_inventory_surfaces_trusted_host_extension_declarations() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-inventory-trusted-host");
        write_trusted_host_extension_package(&plugin_root);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Inventory(PluginInventoryCommand {
                source: plugin_scan_source(&plugin_root, "trusted-host-extension"),
                include_ready: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins inventory should execute");

        let PluginsCommandExecution::Inventory(execution) = execution else {
            panic!("expected inventory execution");
        };
        let result = &execution.results[0];
        assert_eq!(
            result.native_extension.family.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        );
        assert_eq!(
            result.native_extension.trust_lane.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
        );
        assert_eq!(
            result.native_extension.methods,
            vec!["extension/event".to_owned()]
        );
        assert_eq!(
            result.native_extension.host_hooks,
            vec!["turn_start".to_owned(), "turn_end".to_owned()]
        );
        assert_eq!(
            result.native_extension.tui_surfaces,
            vec!["command_palette".to_owned()]
        );
        assert!(
            result.native_extension.metadata_issues.is_empty(),
            "trusted-host inventory projection should stay clean: {:?}",
            result.native_extension.metadata_issues
        );

        let rendered = render_plugins_inventory_text(&execution);
        assert!(rendered.contains("native_extension contract=process_stdio_json_line_v1"));
        assert!(rendered.contains("family=trusted_host_extension"));
        assert!(rendered.contains("trust_lane=trusted_host"));
        assert!(rendered.contains("methods=extension/event"));
        assert!(rendered.contains("host_hooks=turn_start,turn_end"));
        assert!(rendered.contains("tui_surfaces=command_palette"));
        assert!(rendered.contains("authoring validate=loong plugins doctor --root"));
        assert!(rendered.contains("operator_actions=loong plugins actions --root"));
        assert!(rendered.contains("authoring_smoke_test=loong plugins invoke-host-hook"));

        let encoded = serde_json::to_value(&execution).expect("serialize inventory execution");
        assert_eq!(
            encoded["results"][0]["native_extension"]["family"],
            serde_json::json!("trusted_host_extension")
        );
        assert_eq!(
            encoded["results"][0]["native_extension"]["host_hooks"],
            serde_json::json!(["turn_start", "turn_end"])
        );
        assert_eq!(
            encoded["results"][0]["native_extension"]["tui_surfaces"],
            serde_json::json!(["command_palette"])
        );
        assert_eq!(
            encoded["results"][0]["authoring_guidance"]["operator_actions_command"],
            serde_json::json!(format!(
                "loong plugins actions --root \"{}\" --profile sdk-release",
                result.package_root
            ))
        );
    }

    #[tokio::test]
    async fn execute_plugins_doctor_defaults_to_sdk_release_and_surfaces_author_actions() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-doctor-openclaw");
        write_openclaw_weather_sdk_package(&plugin_root);

        let mut source = plugin_doctor_source(&plugin_root, "weather-sdk");
        source.scan.bridge_profile = Some(PluginBridgeProfileArg::OpenclawEcosystemBalanced);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Doctor(PluginDoctorCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
            }),
        })
        .await
        .expect("plugins doctor should execute");

        let PluginsCommandExecution::Doctor(execution) = execution else {
            panic!("expected doctor execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_DOCTOR_SCHEMA_PURPOSE);
        assert_eq!(execution.profile, "sdk_release");
        assert_eq!(execution.returned_results, 1);
        assert_eq!(execution.summary.matched_plugins, 1);
        assert_eq!(execution.summary.returned_plugins, 1);
        assert_eq!(execution.summary.passed_plugins, 0);
        assert_eq!(execution.summary.warned_plugins, 0);
        assert_eq!(execution.summary.blocked_plugins, 1);
        assert_eq!(execution.summary.activation_ready_plugins, 0);
        assert_eq!(execution.summary.setup_incomplete_plugins, 1);
        assert_eq!(execution.summary.deferred_plugins, 1);
        assert_eq!(execution.summary.loaded_plugins, 0);
        assert_eq!(execution.summary.packages_requiring_author_attention, 1);
        assert_eq!(
            execution.summary.packages_with_operator_actions, 1,
            "doctor should surface at least one actionable operator follow-up"
        );
        assert!(
            execution.summary.total_recommended_actions > 0,
            "doctor should expose recommended actions"
        );
        assert!(
            execution.summary.total_operator_actions > 0,
            "doctor should expose operator actions"
        );
        assert_eq!(
            execution
                .summary
                .setup_surface_distribution
                .get("channel")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .activation_status_distribution
                .get("setup_incomplete")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .remediation_counts
                .get("resolve_activation_blockers")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .preflight_summary
                .operator_action_counts_by_kind
                .get("review_diagnostics")
                .copied(),
            Some(1)
        );
        let result = &execution.results[0];
        assert_eq!(result.profile, "sdk_release");
        assert_eq!(result.verdict, "block");
        assert_eq!(result.plugin.plugin_id, "weather-sdk");
        assert_eq!(result.plugin.setup_mode.as_deref(), Some("governed_entry"));
        assert_eq!(result.plugin.setup_surface.as_deref(), Some("channel"));
        assert!(
            result
                .recommended_actions
                .iter()
                .any(|action| action.operator_action.is_some())
        );
    }

    #[tokio::test]
    async fn execute_plugins_doctor_surfaces_trusted_host_extension_declarations() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-doctor-trusted-host");
        write_trusted_host_extension_package(&plugin_root);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Doctor(PluginDoctorCommand {
                source: plugin_doctor_source(&plugin_root, "trusted-host-extension"),
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
            }),
        })
        .await
        .expect("plugins doctor should execute");

        let PluginsCommandExecution::Doctor(execution) = execution else {
            panic!("expected doctor execution");
        };
        let result = &execution.results[0].plugin;
        assert_eq!(
            result.native_extension.family.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        );
        assert_eq!(
            result.native_extension.trust_lane.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
        );
        assert_eq!(
            result.native_extension.host_hooks,
            vec!["turn_start".to_owned(), "turn_end".to_owned()]
        );
        assert_eq!(
            result.native_extension.tui_surfaces,
            vec!["command_palette".to_owned()]
        );
        assert!(
            result.native_extension.metadata_issues.is_empty(),
            "trusted-host doctor projection should stay clean: {:?}",
            result.native_extension.metadata_issues
        );

        let rendered = render_plugins_doctor_text(&execution);
        assert!(rendered.contains("native_extension contract=process_stdio_json_line_v1"));
        assert!(rendered.contains("family=trusted_host_extension"));
        assert!(rendered.contains("trust_lane=trusted_host"));
        assert!(rendered.contains("host_hooks=turn_start,turn_end"));
        assert!(rendered.contains("tui_surfaces=command_palette"));
        assert!(rendered.contains("authoring validate=loong plugins doctor --root"));
        assert!(rendered.contains("operator_actions=loong plugins actions --root"));
        assert!(rendered.contains("authoring_smoke_test=loong plugins invoke-host-hook"));

        let encoded = serde_json::to_value(&execution).expect("serialize doctor execution");
        assert_eq!(
            encoded["results"][0]["plugin"]["native_extension"]["family"],
            serde_json::json!("trusted_host_extension")
        );
        assert_eq!(
            encoded["results"][0]["plugin"]["native_extension"]["tui_surfaces"],
            serde_json::json!(["command_palette"])
        );
        assert_eq!(
            encoded["results"][0]["plugin"]["authoring_guidance"]["smoke_test_command"],
            serde_json::json!(format!(
                "loong plugins invoke-host-hook --root \"{}\" --plugin-id \"{}\" --hook turn_start --payload '{{}}' --allow-command node",
                result.package_root, result.plugin_id
            ))
        );
    }

    #[tokio::test]
    async fn execute_plugins_actions_filters_operator_action_plan() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-actions");
        fs::create_dir_all(&plugin_root).expect("create plugin root");
        fs::write(
            format!("{plugin_root}/search_a.py"),
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-a",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write plugin a");
        fs::write(
            format!("{plugin_root}/search_b.py"),
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-b",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write plugin b");

        let source = plugin_governance_source(&plugin_root, "");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Actions(PluginActionsCommand {
                source,
                include_passed: false,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                surface: vec![PluginActionSurfaceArg::PluginPackage],
                kind: vec![PluginActionKindArg::ResolveSlotOwnership],
                requires_reload: Some(true),
            }),
        })
        .await
        .expect("plugins actions should execute");

        let PluginsCommandExecution::Actions(execution) = execution else {
            panic!("expected actions execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_ACTIONS_SCHEMA_PURPOSE);
        assert_eq!(execution.total_actions, 4);
        assert_eq!(execution.matched_actions, 2);
        assert_eq!(execution.bridge_support_provenance, None);
        assert_eq!(
            execution.summary.schema_version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.surface,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE
        );
        assert_eq!(
            execution.summary.schema.purpose,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE
        );
        assert_eq!(execution.summary.bridge_support_provenance, None);
        assert_eq!(
            execution
                .filtered_action_counts_by_kind
                .get("resolve_slot_ownership")
                .copied(),
            Some(2)
        );
        assert!(execution.actions.iter().all(|item| {
            item.action.surface == "plugin_package"
                && item.action.kind == "resolve_slot_ownership"
                && item.action.requires_reload
        }));
    }

    #[tokio::test]
    async fn execute_plugins_preflight_uses_bundled_openclaw_bridge_profile() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-openclaw");
        write_openclaw_weather_sdk_package(&plugin_root);

        let mut source = plugin_governance_source(&plugin_root, "weather-sdk");
        source.scan.bridge_profile = Some(PluginBridgeProfileArg::OpenclawEcosystemBalanced);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Preflight(PluginPreflightCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins preflight should execute");

        let PluginsCommandExecution::Preflight(execution) = execution else {
            panic!("expected preflight execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_PREFLIGHT_SCHEMA_PURPOSE);
        let provenance = execution
            .bridge_support_provenance
            .as_ref()
            .expect("bundled bridge profile should emit provenance");
        assert_eq!(
            execution.summary.schema_version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.surface,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE
        );
        assert_eq!(
            execution.summary.schema.purpose,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE
        );
        assert_eq!(
            execution.bridge_support_source.as_deref(),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(
            provenance.source.as_deref(),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(provenance.delta_source, None);
        assert_eq!(provenance.delta_sha256, None);
        assert_eq!(
            execution
                .summary
                .bridge_support_provenance
                .as_ref()
                .and_then(|value| value.source.as_deref()),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(execution.summary.blocked_plugins, 1);
        assert_eq!(execution.summary.warned_plugins, 0);
        assert_eq!(
            execution
                .summary
                .dialect_distribution
                .get("openclaw_modern_manifest")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .compatibility_mode_distribution
                .get("openclaw_modern")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .source_language_distribution
                .get("javascript")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution
                .summary
                .bridge_kind_distribution
                .get("process_stdio")
                .copied(),
            Some(1)
        );
        assert_eq!(
            execution.summary.active_bridge_profile.as_deref(),
            Some("openclaw-ecosystem-balanced")
        );
        assert_eq!(
            execution.summary.recommended_bridge_profile.as_deref(),
            Some("openclaw-ecosystem-balanced")
        );
        assert_eq!(
            execution.summary.active_bridge_profile_matches_recommended,
            Some(true)
        );
        assert_eq!(
            execution.summary.active_bridge_support_fits_all_plugins,
            Some(true)
        );
        assert!(execution.summary.bridge_profile_fits.iter().any(|fit| {
            fit.profile_id == "openclaw-ecosystem-balanced"
                && fit.fits_all_plugins
                && fit.supported_plugins == 1
                && fit.blocked_plugins == 0
        }));
        assert!(
            execution.summary.bridge_profile_recommendation.is_none(),
            "active bundled profile already matches recommendation"
        );
        assert_eq!(execution.results.len(), 1);
        let first_result = &execution.results[0];
        let plugin = &first_result.plugin;
        let activation_status = plugin.activation_status.as_deref();
        let activation_reason = plugin
            .activation_reason
            .as_deref()
            .expect("expected plugin activation reason");

        assert_eq!(activation_status, Some("setup_incomplete"));
        assert_eq!(first_result.verdict, "block");
        assert!(activation_reason.contains("plugins.entries.weather-sdk"));
        assert!(
            first_result
                .policy_flags
                .iter()
                .any(|flag| flag == "activation_blocked")
        );
    }

    #[tokio::test]
    async fn execute_plugins_preflight_recommends_openclaw_bridge_profile_without_active_profile() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-openclaw-recommend");
        write_openclaw_weather_sdk_package(&plugin_root);

        let source = plugin_governance_source(&plugin_root, "weather-sdk");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Preflight(PluginPreflightCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins preflight should execute");

        let PluginsCommandExecution::Preflight(execution) = execution else {
            panic!("expected preflight execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_PREFLIGHT_SCHEMA_PURPOSE);
        assert_eq!(execution.bridge_support_provenance, None);
        assert_eq!(execution.bridge_support_source, None);
        assert_eq!(
            execution.summary.schema_version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(execution.summary.bridge_support_provenance, None);
        assert_eq!(execution.summary.active_bridge_profile, None);
        assert_eq!(
            execution.summary.recommended_bridge_profile.as_deref(),
            Some("openclaw-ecosystem-balanced")
        );
        assert_eq!(
            execution
                .summary
                .recommended_bridge_profile_source
                .as_deref(),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(
            execution.summary.active_bridge_profile_matches_recommended,
            Some(false)
        );
        assert_eq!(
            execution.summary.active_bridge_support_fits_all_plugins,
            None
        );
        let recommendation = execution
            .summary
            .bridge_profile_recommendation
            .as_ref()
            .expect("adopt recommendation should be present");
        assert_eq!(recommendation.kind, "adopt_bundled_profile");
        assert_eq!(
            recommendation.target_profile_id,
            "openclaw-ecosystem-balanced"
        );
        assert!(recommendation.delta.is_none());
        assert!(execution.summary.bridge_profile_fits.iter().any(|fit| {
            fit.profile_id == "native-balanced"
                && !fit.fits_all_plugins
                && fit.blocked_plugins == 1
                && fit
                    .blocking_reasons
                    .get("unsupported_compatibility_mode")
                    .copied()
                    == Some(1)
        }));
    }

    #[tokio::test]
    async fn execute_plugins_preflight_recommends_custom_bridge_profile_delta_for_python_openclaw_plugins()
     {
        let plugin_root = unique_temp_dir("loong-plugins-cli-openclaw-python-delta");
        write_openclaw_weather_sdk_python_package(&plugin_root);

        let source = plugin_governance_source(&plugin_root, "weather-sdk");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Preflight(PluginPreflightCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins preflight should execute");

        let PluginsCommandExecution::Preflight(execution) = execution else {
            panic!("expected preflight execution");
        };
        assert_eq!(execution.summary.recommended_bridge_profile, None);
        assert_eq!(
            execution.summary.active_bridge_support_fits_all_plugins,
            None
        );
        let recommendation = execution
            .summary
            .bridge_profile_recommendation
            .as_ref()
            .expect("custom delta recommendation should be present");
        assert_eq!(recommendation.kind, "author_bridge_profile_delta");
        assert_eq!(
            recommendation.target_profile_id,
            "openclaw-ecosystem-balanced"
        );
        let delta = recommendation
            .delta
            .as_ref()
            .expect("custom delta recommendation should include a delta");
        assert_eq!(delta.supported_compatibility_modes, Vec::<String>::new());
        assert_eq!(delta.supported_compatibility_shims, Vec::<String>::new());
        assert_eq!(delta.shim_profile_additions.len(), 1);
        assert_eq!(
            delta.shim_profile_additions[0].supported_source_languages,
            vec!["python".to_owned()]
        );
    }

    #[tokio::test]
    async fn execute_plugins_preflight_accepts_bridge_support_delta_artifact_and_suppresses_repeat_delta_recommendation()
     {
        let plugin_root = unique_temp_dir("loong-plugins-cli-openclaw-python-active-delta");
        write_openclaw_weather_sdk_python_package(&plugin_root);
        let delta_path = format!("{plugin_root}/bridge-support.delta.json");
        let artifact = materialize_bridge_support_delta_artifact(
            "openclaw-ecosystem-balanced",
            Some(&crate::PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![crate::PluginPreflightBridgeShimProfileDelta {
                    shim_id: "openclaw-modern-compat".to_owned(),
                    shim_family: "openclaw-modern-compat".to_owned(),
                    supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                    supported_bridges: vec!["process_stdio".to_owned()],
                    supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                    supported_source_languages: vec!["python".to_owned()],
                }],
                unresolved_blocking_reasons: Vec::new(),
            }),
        )
        .expect("delta artifact should materialize");
        fs::write(
            &delta_path,
            serde_json::to_string_pretty(&artifact).expect("serialize delta artifact"),
        )
        .expect("write delta artifact");

        let mut source = plugin_governance_source(&plugin_root, "weather-sdk");
        source.scan.bridge_support_delta = Some(delta_path.clone());
        source.scan.bridge_support_delta_sha256 = Some(artifact.sha256.clone());

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Preflight(PluginPreflightCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("plugins preflight should execute with delta artifact");

        let PluginsCommandExecution::Preflight(execution) = execution else {
            panic!("expected preflight execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_PREFLIGHT_SCHEMA_PURPOSE);
        let expected_bridge_support_source = format!("delta:{delta_path}");
        let provenance = execution
            .bridge_support_provenance
            .as_ref()
            .expect("delta-backed bridge policy should emit provenance");
        assert_eq!(
            execution.summary.schema_version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.bridge_support_source.as_deref(),
            Some(expected_bridge_support_source.as_str())
        );
        assert_eq!(
            provenance.source.as_deref(),
            Some(expected_bridge_support_source.as_str())
        );
        assert_eq!(
            execution.bridge_support_delta_source.as_deref(),
            Some(delta_path.as_str())
        );
        assert_eq!(
            provenance.delta_source.as_deref(),
            Some(delta_path.as_str())
        );
        assert_eq!(
            execution.bridge_support_delta_sha256.as_deref(),
            Some(artifact.sha256.as_str())
        );
        assert_eq!(
            provenance.delta_sha256.as_deref(),
            Some(artifact.sha256.as_str())
        );
        assert_eq!(
            execution
                .summary
                .bridge_support_provenance
                .as_ref()
                .and_then(|value| value.delta_source.as_deref()),
            Some(delta_path.as_str())
        );
        assert_eq!(execution.summary.active_bridge_profile, None);
        assert_eq!(execution.summary.recommended_bridge_profile, None);
        assert_eq!(
            execution.summary.active_bridge_support_fits_all_plugins,
            Some(true)
        );
        assert!(
            execution.summary.bridge_profile_recommendation.is_none(),
            "active delta-backed bridge policy should suppress repeat delta recommendation"
        );
    }

    #[tokio::test]
    async fn execute_plugins_bridge_template_materializes_aligned_active_profile() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-bridge-template-aligned");
        write_openclaw_weather_sdk_package(&plugin_root);

        let mut source = plugin_governance_source(&plugin_root, "weather-sdk");
        source.scan.bridge_profile = Some(PluginBridgeProfileArg::OpenclawEcosystemBalanced);

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::BridgeTemplate(PluginBridgeTemplateCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                output: None,
                delta_output: None,
            }),
        })
        .await
        .expect("plugins bridge-template should execute");

        let PluginsCommandExecution::BridgeTemplate(execution) = execution else {
            panic!("expected bridge template execution");
        };
        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(
            execution.schema.purpose,
            PLUGINS_BRIDGE_TEMPLATE_SCHEMA_PURPOSE
        );
        assert_eq!(
            execution.summary.schema_version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution.summary.schema.version,
            PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION
        );
        assert_eq!(
            execution
                .bridge_support_provenance
                .as_ref()
                .and_then(|value| value.source.as_deref()),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(
            execution
                .summary
                .bridge_support_provenance
                .as_ref()
                .and_then(|value| value.source.as_deref()),
            Some("bundled:bridge-support-openclaw-ecosystem-balanced.json")
        );
        assert_eq!(execution.template_kind, "active_aligned_profile");
        assert_eq!(execution.template_profile_id, "openclaw-ecosystem-balanced");
        assert_eq!(
            execution.template_policy_version.as_deref(),
            Some("openclaw-ecosystem-balanced@1")
        );
        assert_eq!(
            execution.delta_artifact.base_profile_id,
            "openclaw-ecosystem-balanced"
        );
        assert_eq!(
            execution.delta_artifact.delta,
            crate::PluginPreflightBridgeProfileDelta::default()
        );
        assert!(
            execution
                .template
                .supported_compatibility_modes
                .iter()
                .any(|mode| mode.as_str() == "openclaw_modern")
        );
    }

    #[tokio::test]
    async fn execute_plugins_bridge_template_materializes_custom_delta_and_writes_output() {
        let plugin_root = unique_temp_dir("loong-plugins-cli-bridge-template-delta");
        write_openclaw_weather_sdk_python_package(&plugin_root);
        let output_path = format!("{plugin_root}/generated/bridge-support.json");
        let delta_output_path = format!("{plugin_root}/generated/bridge-support.delta.json");

        let source = plugin_governance_source(&plugin_root, "weather-sdk");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::BridgeTemplate(PluginBridgeTemplateCommand {
                source,
                include_passed: true,
                include_warned: true,
                include_blocked: true,
                include_deferred: true,
                output: Some(output_path.clone()),
                delta_output: Some(delta_output_path.clone()),
            }),
        })
        .await
        .expect("plugins bridge-template should execute");

        let PluginsCommandExecution::BridgeTemplate(execution) = execution else {
            panic!("expected bridge template execution");
        };
        assert_eq!(execution.template_kind, "derived_custom_profile");
        assert_eq!(execution.template_profile_id, "openclaw-ecosystem-balanced");
        assert_eq!(
            execution.template_policy_version.as_deref(),
            Some("custom-derived-from-openclaw-ecosystem-balanced")
        );
        assert_eq!(
            execution.delta_output_path.as_deref(),
            Some(delta_output_path.as_str())
        );
        assert_eq!(
            execution.delta_artifact.base_profile_id,
            "openclaw-ecosystem-balanced"
        );
        assert!(
            execution
                .template
                .supported_compatibility_shim_profiles
                .iter()
                .any(|profile| {
                    profile.shim.shim_id == "openclaw-modern-compat"
                        && profile.supported_source_languages.contains("python")
                })
        );
        assert_eq!(execution.output_path.as_deref(), Some(output_path.as_str()));
        assert_eq!(
            execution.delta_artifact.delta.shim_profile_additions[0].supported_source_languages,
            vec!["python".to_owned()]
        );

        let rendered = fs::read_to_string(&output_path).expect("bridge template file should exist");
        let template: BridgeSupportSpec =
            serde_json::from_str(&rendered).expect("written bridge template should decode");
        assert_eq!(
            template.policy_version.as_deref(),
            Some("custom-derived-from-openclaw-ecosystem-balanced")
        );
        assert!(
            template
                .supported_compatibility_shim_profiles
                .iter()
                .any(|profile| {
                    profile.shim.shim_id == "openclaw-modern-compat"
                        && profile.supported_source_languages.contains("python")
                })
        );

        let rendered_delta = fs::read_to_string(&delta_output_path)
            .expect("bridge delta artifact file should exist");
        let delta_artifact: MaterializedBridgeSupportDeltaArtifact =
            serde_json::from_str(&rendered_delta)
                .expect("written bridge delta artifact should decode");
        assert_eq!(
            delta_artifact.base_profile_id,
            "openclaw-ecosystem-balanced"
        );
        assert_eq!(
            delta_artifact.delta.shim_profile_additions[0].supported_source_languages,
            vec!["python".to_owned()]
        );
    }

    #[tokio::test]
    async fn execute_plugins_init_scaffolds_http_json_package_manifest() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-http");
        let package_root = format!("{temp_root}/tavily-search");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root: package_root.clone(),
                plugin_id: "tavily-search".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::HttpJson,
                source_language: None,
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "0.1.0".to_owned(),
                summary: Some("Tavily-backed search package".to_owned()),
            }),
        })
        .await
        .expect("plugins init should scaffold an http json package");

        let PluginsCommandExecution::Init(execution) = execution else {
            panic!("expected init execution");
        };

        assert_eq!(execution.schema_version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.version, PLUGINS_COMMAND_SCHEMA_VERSION);
        assert_eq!(execution.schema.surface, PLUGINS_COMMAND_SCHEMA_SURFACE);
        assert_eq!(execution.schema.purpose, PLUGINS_INIT_SCHEMA_PURPOSE);
        assert_eq!(execution.package_root, package_root);
        assert_eq!(execution.plugin_id, "tavily-search");
        assert_eq!(execution.provider_id, "tavily-search");
        assert_eq!(execution.connector_name, "tavily-search");
        assert_eq!(execution.bridge_kind, "http_json");
        assert_eq!(execution.source_language, None);
        assert_eq!(execution.adapter_family, "http-adapter");
        assert_eq!(execution.entrypoint, "https://localhost/invoke");
        assert_eq!(execution.version, "0.1.0");
        assert_eq!(execution.files_written.len(), 2);

        let manifest_path = execution.manifest_path.clone();
        let readme_path = execution.readme_path.clone();

        let rendered_manifest =
            fs::read_to_string(&manifest_path).expect("scaffold manifest should exist");
        let manifest: crate::kernel::PluginManifest =
            serde_json::from_str(&rendered_manifest).expect("scaffold manifest should decode");

        assert_eq!(
            manifest.api_version.as_deref(),
            Some(crate::kernel::CURRENT_PLUGIN_MANIFEST_API_VERSION)
        );
        assert_eq!(manifest.version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.plugin_id, "tavily-search");
        assert_eq!(manifest.provider_id, "tavily-search");
        assert_eq!(manifest.connector_name, "tavily-search");
        assert_eq!(
            manifest.summary.as_deref(),
            Some("Tavily-backed search package")
        );
        assert!(
            manifest.capabilities.contains(&Capability::InvokeConnector),
            "scaffold manifest should include invoke_connector"
        );
        assert_eq!(
            manifest.metadata.get("bridge_kind").map(String::as_str),
            Some("http_json")
        );
        assert_eq!(
            manifest.metadata.get("adapter_family").map(String::as_str),
            Some("http-adapter")
        );
        assert_eq!(
            manifest.metadata.get("entrypoint").map(String::as_str),
            Some("https://localhost/invoke")
        );
        let expected_host_version_req = format!(">={}", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            manifest
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_api.as_deref()),
            Some(crate::kernel::CURRENT_PLUGIN_HOST_API)
        );
        assert_eq!(
            manifest
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_version_req.as_deref()),
            Some(expected_host_version_req.as_str())
        );

        let rendered_readme =
            fs::read_to_string(&readme_path).expect("scaffold readme should exist");
        assert!(
            rendered_readme.contains("loong plugins doctor --root"),
            "README should point authors to doctor: {rendered_readme}"
        );
        assert!(
            rendered_readme.contains("loong plugins actions --root"),
            "README should point authors to actions: {rendered_readme}"
        );

        let scanner = crate::kernel::PluginScanner::new();
        let scan_report = scanner
            .scan_path(&execution.package_root)
            .expect("scaffold package should scan cleanly");
        let translator = crate::kernel::PluginTranslator::new();
        let translation_report = translator.translate_scan_report(&scan_report);
        let ir = &translation_report.entries[0];

        assert_eq!(translation_report.translated_plugins, 1);
        assert_eq!(
            ir.runtime.bridge_kind,
            crate::kernel::PluginBridgeKind::HttpJson
        );
        assert_eq!(ir.runtime.adapter_family, "http-adapter");
        assert_eq!(ir.runtime.entrypoint_hint, "https://localhost/invoke");
    }

    #[tokio::test]
    async fn execute_plugins_init_requires_source_language_for_process_stdio() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-process-language");
        let package_root = format!("{temp_root}/tavily-search");

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "tavily-search".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: None,
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "0.1.0".to_owned(),
                summary: None,
            }),
        })
        .await
        .expect_err("process stdio scaffold should require source language");

        assert!(error.contains("--source-language"));
        assert!(error.contains("process_stdio"));
    }

    #[tokio::test]
    async fn execute_plugins_init_rejects_invalid_semver_version() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-invalid-version");
        let package_root = format!("{temp_root}/tavily-search");

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "tavily-search".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::HttpJson,
                source_language: None,
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "not-semver".to_owned(),
                summary: None,
            }),
        })
        .await
        .expect_err("plugins init should reject invalid semver");

        assert!(error.contains("--version"));
        assert!(error.contains("semver"));
    }

    #[tokio::test]
    async fn execute_plugins_init_process_stdio_scaffold_retains_source_language() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-process");
        let package_root = format!("{temp_root}/weather-python");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "weather-python".to_owned(),
                provider_id: Some("weather".to_owned()),
                connector_name: Some("weather-stdio".to_owned()),
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("py".to_owned()),
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "0.2.0".to_owned(),
                summary: Some("Python weather bridge".to_owned()),
            }),
        })
        .await
        .expect("process stdio scaffold should succeed");

        let PluginsCommandExecution::Init(execution) = execution else {
            panic!("expected init execution");
        };

        assert_eq!(execution.bridge_kind, "process_stdio");
        assert_eq!(execution.source_language.as_deref(), Some("python"));
        assert_eq!(execution.adapter_family, "python-stdio-adapter");
        assert_eq!(execution.entrypoint, "stdin/stdout::invoke");
        assert!(execution.smoke_test_command.is_some());
        assert!(!execution.runtime_files_written.is_empty());

        let rendered_manifest =
            fs::read_to_string(&execution.manifest_path).expect("manifest should exist");
        let manifest: crate::kernel::PluginManifest =
            serde_json::from_str(&rendered_manifest).expect("manifest should decode");

        assert_eq!(
            manifest.metadata.get("source_language").map(String::as_str),
            Some("python")
        );
        assert_eq!(
            manifest.metadata.get("adapter_family").map(String::as_str),
            Some("python-stdio-adapter")
        );

        let scanner = crate::kernel::PluginScanner::new();
        let scan_report = scanner
            .scan_path(&execution.package_root)
            .expect("scaffold package should scan cleanly");
        let translator = crate::kernel::PluginTranslator::new();
        let translation_report = translator.translate_scan_report(&scan_report);
        let ir = &translation_report.entries[0];

        assert_eq!(ir.runtime.source_language, "python");
        assert_eq!(
            ir.runtime.bridge_kind,
            crate::kernel::PluginBridgeKind::ProcessStdio
        );
        assert_eq!(ir.runtime.adapter_family, "python-stdio-adapter");
        assert_eq!(ir.runtime.entrypoint_hint, "stdin/stdout::invoke");
    }

    #[tokio::test]
    async fn execute_plugins_init_persists_additive_declared_capabilities() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-capabilities");
        let package_root = format!("{temp_root}/weather-python");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "weather-python".to_owned(),
                provider_id: Some("weather".to_owned()),
                connector_name: Some("weather-stdio".to_owned()),
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("py".to_owned()),
                capabilities: vec!["observe_telemetry".to_owned()],
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "0.2.0".to_owned(),
                summary: Some("Python weather bridge".to_owned()),
            }),
        })
        .await
        .expect("process stdio scaffold with additive capabilities should succeed");

        let PluginsCommandExecution::Init(execution) = execution else {
            panic!("expected init execution");
        };

        let inventory_execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Inventory(PluginInventoryCommand {
                source: PluginScanSourceArgs {
                    roots: vec![execution.package_root.clone()],
                    query: String::new(),
                    limit: None,
                    bridge_support: None,
                    bridge_profile: None,
                    bridge_support_delta: None,
                    bridge_support_sha256: None,
                    bridge_support_delta_sha256: None,
                },
                include_ready: true,
                include_blocked: true,
                include_deferred: true,
                include_examples: false,
            }),
        })
        .await
        .expect("inventory should read scaffolded plugin capabilities");

        let PluginsCommandExecution::Inventory(inventory_execution) = inventory_execution else {
            panic!("expected inventory execution");
        };

        assert_eq!(
            inventory_execution.results[0].capabilities,
            vec![
                "invoke_connector".to_owned(),
                "observe_telemetry".to_owned()
            ]
        );
        assert_eq!(
            inventory_execution
                .summary
                .capability_distribution
                .get("invoke_connector"),
            Some(&1)
        );
        assert_eq!(
            inventory_execution
                .summary
                .capability_distribution
                .get("observe_telemetry"),
            Some(&1)
        );
    }

    #[test]
    fn write_plugin_scaffold_files_rolls_back_manifest_when_readme_write_fails() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-rollback");
        let package_root = Path::new(temp_root.as_str());
        let manifest_path = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        let readme_path = package_root.join(PLUGINS_INIT_README_FILE_NAME);
        let expected_host_version_req = format!(">={}", env!("CARGO_PKG_VERSION"));

        fs::create_dir_all(package_root).expect("create package root");
        fs::create_dir(&readme_path).expect("create readme directory");

        let manifest_contents = format!(
            "{{\"compatibility\":{{\"host_version_req\":\"{expected_host_version_req}\"}}}}"
        );
        let error = write_plugin_scaffold_files(
            package_root,
            &manifest_path,
            manifest_contents.as_str(),
            &readme_path,
            "# scaffold",
            None,
        )
        .expect_err("readme directory should force scaffold rollback");

        assert!(error.contains("write scaffold readme"));
        assert!(
            !manifest_path.exists(),
            "manifest should be removed after readme write failure"
        );
        assert!(
            readme_path.is_dir(),
            "readme test fixture directory should remain in place"
        );
    }

    #[tokio::test]
    async fn execute_plugins_init_scaffolds_trusted_host_extension_package() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-trusted-host");
        let package_root = format!("{temp_root}/weather-host");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "weather-host".to_owned(),
                provider_id: Some("weather".to_owned()),
                connector_name: Some("weather-stdio".to_owned()),
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("js".to_owned()),
                capabilities: Vec::new(),
                host_hooks: vec!["turn_start".to_owned()],
                tui_surfaces: vec!["command_palette".to_owned()],
                version: "0.2.0".to_owned(),
                summary: Some("Trusted host weather hook".to_owned()),
            }),
        })
        .await
        .expect("trusted host scaffold should succeed");

        let PluginsCommandExecution::Init(execution) = execution else {
            panic!("expected init execution");
        };
        assert!(
            execution
                .smoke_test_command
                .as_deref()
                .is_some_and(|command| command.contains("plugins invoke-host-hook"))
        );
        assert!(!execution.runtime_files_written.is_empty());

        let rendered_manifest =
            fs::read_to_string(&execution.manifest_path).expect("manifest should exist");
        let manifest: crate::kernel::PluginManifest =
            serde_json::from_str(&rendered_manifest).expect("manifest should decode");
        assert_eq!(
            manifest
                .metadata
                .get("loong_extension_family")
                .map(String::as_str),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        );
        assert_eq!(
            manifest
                .metadata
                .get("loong_extension_tui_surfaces_json")
                .map(String::as_str),
            Some("[\"command_palette\"]")
        );
        assert_eq!(
            manifest
                .metadata
                .get("loong_extension_host_hooks_json")
                .map(String::as_str),
            Some("[\"turn_start\"]")
        );
    }

    #[tokio::test]
    async fn execute_plugins_init_trusted_host_scaffold_smoke_probe_succeeds() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-trusted-host-smoke");
        let package_root = format!("{temp_root}/weather-host");

        let execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root: package_root.clone(),
                plugin_id: "weather-host".to_owned(),
                provider_id: Some("weather".to_owned()),
                connector_name: Some("weather-stdio".to_owned()),
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("js".to_owned()),
                capabilities: Vec::new(),
                host_hooks: vec!["turn_start".to_owned()],
                tui_surfaces: Vec::new(),
                version: "0.2.0".to_owned(),
                summary: Some("Trusted host weather hook".to_owned()),
            }),
        })
        .await
        .expect("trusted host scaffold should succeed");

        let PluginsCommandExecution::Init(execution) = execution else {
            panic!("expected init execution");
        };

        let hook_execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::InvokeHostHook(PluginInvokeHostHookCommand {
                root: execution.package_root.clone(),
                plugin_id: "weather-host".to_owned(),
                hook: "turn_start".to_owned(),
                payload: "{\"turn_id\":\"demo-turn\"}".to_owned(),
                allow_commands: vec!["node".to_owned()],
            }),
        })
        .await
        .expect("scaffolded trusted host package should probe successfully");

        let PluginsCommandExecution::InvokeHostHook(hook_execution) = hook_execution else {
            panic!("expected invoke-host-hook execution");
        };
        assert_eq!(
            hook_execution.response_payload["handled_hook"],
            serde_json::json!("turn_start")
        );
    }

    #[tokio::test]
    async fn execute_plugins_init_rejects_unsupported_trusted_host_hook() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-bad-hook");
        let package_root = format!("{temp_root}/weather-host");

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "weather-host".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("js".to_owned()),
                capabilities: Vec::new(),
                host_hooks: vec!["provider_request".to_owned()],
                tui_surfaces: Vec::new(),
                version: "0.2.0".to_owned(),
                summary: None,
            }),
        })
        .await
        .expect_err("unsupported host hook should be rejected");

        assert!(error.contains("--host-hook"));
    }

    #[tokio::test]
    async fn execute_plugins_init_rejects_unsupported_trusted_tui_surface() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-bad-surface");
        let package_root = format!("{temp_root}/weather-host");

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root,
                plugin_id: "weather-host".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::ProcessStdio,
                source_language: Some("js".to_owned()),
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: vec!["sidebar_widget".to_owned()],
                version: "0.2.0".to_owned(),
                summary: None,
            }),
        })
        .await
        .expect_err("unsupported tui surface should be rejected");

        assert!(error.contains("--tui-surface"));
    }

    #[tokio::test]
    async fn execute_plugins_init_rejects_non_empty_package_root() {
        let temp_root = unique_temp_dir("loong-plugins-cli-init-non-empty");
        let package_root = format!("{temp_root}/existing");

        fs::create_dir_all(&package_root).expect("create package root");
        fs::write(format!("{package_root}/README.md"), "occupied").expect("write occupied file");

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::Init(PluginInitCommand {
                package_root: package_root.clone(),
                plugin_id: "occupied".to_owned(),
                provider_id: None,
                connector_name: None,
                bridge_kind: PluginInitBridgeKindArg::HttpJson,
                source_language: None,
                capabilities: Vec::new(),
                host_hooks: Vec::new(),
                tui_surfaces: Vec::new(),
                version: "0.1.0".to_owned(),
                summary: None,
            }),
        })
        .await
        .expect_err("non-empty package root should be rejected");

        assert!(error.contains("empty"));
        assert!(error.contains(&package_root));
    }

    #[tokio::test]
    async fn execute_plugins_invoke_extension_runs_process_stdio_extension() {
        let temp_root = unique_temp_dir("loong-plugins-cli-invoke-extension");
        let package_root = format!("{temp_root}/trusted-host-extension");
        write_trusted_host_extension_package(&package_root);

        let invoke_execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::InvokeExtension(PluginInvokeExtensionCommand {
                root: package_root,
                plugin_id: "trusted-host-extension".to_owned(),
                method: "extension/event".to_owned(),
                payload: "{\"event\":\"session_start\"}".to_owned(),
                allow_commands: vec!["node".to_owned()],
            }),
        })
        .await
        .expect("invoke-extension should execute process_stdio extension");

        let PluginsCommandExecution::InvokeExtension(invoke_execution) = invoke_execution else {
            panic!("expected invoke-extension execution");
        };
        assert_eq!(
            invoke_execution.response_payload["handled_event"],
            serde_json::json!("session_start")
        );
    }

    #[tokio::test]
    async fn execute_plugins_invoke_host_hook_runs_trusted_host_extension_probe() {
        let temp_root = unique_temp_dir("loong-plugins-cli-invoke-host-hook");
        let package_root = format!("{temp_root}/trusted-host-extension");
        write_trusted_host_extension_package(&package_root);

        let hook_execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::InvokeHostHook(PluginInvokeHostHookCommand {
                root: package_root,
                plugin_id: "trusted-host-extension".to_owned(),
                hook: "turn_start".to_owned(),
                payload: "{\"turn_id\":\"demo-turn\"}".to_owned(),
                allow_commands: vec!["node".to_owned()],
            }),
        })
        .await
        .expect("invoke-host-hook should execute trusted host extension");

        let PluginsCommandExecution::InvokeHostHook(hook_execution) = hook_execution else {
            panic!("expected invoke-host-hook execution");
        };
        assert_eq!(
            hook_execution.extension_family.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        );
        assert_eq!(
            hook_execution.extension_trust_lane.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE)
        );
        assert_eq!(hook_execution.dispatched_method, "extension/event");
        assert_eq!(hook_execution.hook, "turn_start");
        assert_eq!(
            hook_execution.response_payload["handled_event"],
            serde_json::json!("turn_start")
        );
        assert_eq!(
            hook_execution.response_payload["handled_hook"],
            serde_json::json!("turn_start")
        );
        assert_eq!(
            hook_execution.response_payload["received_hook_payload"]["turn_id"],
            serde_json::json!("demo-turn")
        );
    }

    #[tokio::test]
    async fn execute_plugins_invoke_tui_surface_runs_trusted_host_extension_probe() {
        let temp_root = unique_temp_dir("loong-plugins-cli-invoke-tui-surface");
        let package_root = format!("{temp_root}/trusted-host-extension");
        write_trusted_host_extension_package(&package_root);

        let surface_execution = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::InvokeTuiSurface(PluginInvokeTuiSurfaceCommand {
                root: package_root,
                plugin_id: "trusted-host-extension".to_owned(),
                tui_surface: "command_palette".to_owned(),
                payload: "{\"query\":\":ext\"}".to_owned(),
                allow_commands: vec!["node".to_owned()],
            }),
        })
        .await
        .expect("invoke-tui-surface should execute trusted host extension");

        let PluginsCommandExecution::InvokeTuiSurface(surface_execution) = surface_execution else {
            panic!("expected invoke-tui-surface execution");
        };
        assert_eq!(
            surface_execution.extension_family.as_deref(),
            Some(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY)
        );
        assert_eq!(surface_execution.tui_surface, "command_palette");
        assert_eq!(
            surface_execution.response_payload["handled_event"],
            serde_json::json!("tui_surface")
        );
        assert_eq!(
            surface_execution.response_payload["handled_tui_surface"],
            serde_json::json!("command_palette")
        );
        assert_eq!(
            surface_execution.response_payload["received_surface_payload"]["query"],
            serde_json::json!(":ext")
        );
    }

    #[tokio::test]
    async fn execute_plugins_invoke_host_hook_rejects_governed_sidecar_extension() {
        let temp_root = unique_temp_dir("loong-plugins-cli-invoke-host-hook-governed");
        let package_root = format!("{temp_root}/host-hook-extension");
        write_host_hook_declared_native_extension_package(&package_root);

        let error = execute_plugins_command(PluginsCommandOptions {
            json: false,
            command: PluginsCommands::InvokeHostHook(PluginInvokeHostHookCommand {
                root: package_root,
                plugin_id: "host-hook-extension".to_owned(),
                hook: "turn_start".to_owned(),
                payload: "{}".to_owned(),
                allow_commands: vec!["node".to_owned()],
            }),
        })
        .await
        .expect_err("invoke-host-hook should reject governed sidecar packages");

        assert!(error.contains(crate::kernel::TRUSTED_HOST_EXTENSION_FAMILY));
        assert!(error.contains(crate::kernel::TRUSTED_HOST_EXTENSION_TRUST_LANE));
    }

    #[tokio::test]
    async fn checked_in_trusted_host_examples_probe_successfully() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root");
        for (relative_root, plugin_id, allow_command) in [
            (
                "examples/plugins-process/native-extension-trusted-host-javascript",
                "trusted-host-extension-javascript-example",
                "node",
            ),
            (
                "examples/plugins-process/native-extension-trusted-host-go",
                "trusted-host-extension-go-example",
                "go",
            ),
            (
                "examples/plugins-process/native-extension-trusted-host-rust",
                "trusted-host-extension-rust-example",
                "cargo",
            ),
        ] {
            let package_root = repo_root.join(relative_root).display().to_string();

            let hook_execution = execute_plugins_command(PluginsCommandOptions {
                json: false,
                command: PluginsCommands::InvokeHostHook(PluginInvokeHostHookCommand {
                    root: package_root.clone(),
                    plugin_id: plugin_id.to_owned(),
                    hook: "turn_start".to_owned(),
                    payload: "{\"turn_id\":\"demo-turn\"}".to_owned(),
                    allow_commands: vec![allow_command.to_owned()],
                }),
            })
            .await
            .unwrap_or_else(|error| {
                panic!("{plugin_id} should probe host hook successfully: {error}")
            });

            let PluginsCommandExecution::InvokeHostHook(hook_execution) = hook_execution else {
                panic!("expected invoke-host-hook execution");
            };
            assert_eq!(
                hook_execution.response_payload["handled_hook"],
                serde_json::json!("turn_start")
            );

            let surface_execution = execute_plugins_command(PluginsCommandOptions {
                json: false,
                command: PluginsCommands::InvokeTuiSurface(PluginInvokeTuiSurfaceCommand {
                    root: package_root,
                    plugin_id: plugin_id.to_owned(),
                    tui_surface: "command_palette".to_owned(),
                    payload: "{\"query\":\":ext\"}".to_owned(),
                    allow_commands: vec![allow_command.to_owned()],
                }),
            })
            .await
            .unwrap_or_else(|error| {
                panic!("{plugin_id} should probe tui surface successfully: {error}")
            });

            let PluginsCommandExecution::InvokeTuiSurface(surface_execution) = surface_execution
            else {
                panic!("expected invoke-tui-surface execution");
            };
            assert_eq!(
                surface_execution.response_payload["handled_tui_surface"],
                serde_json::json!("command_palette")
            );
        }
    }
}
