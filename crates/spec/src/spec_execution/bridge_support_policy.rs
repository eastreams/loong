use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use kernel::{PluginCompatibilityShim, PluginCompatibilityShimSupport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::spec_runtime::*;
use crate::{
    BUNDLED_BRIDGE_SUPPORT_NATIVE_BALANCED, BUNDLED_BRIDGE_SUPPORT_OPENCLAW_ECOSYSTEM_BALANCED,
};

pub const BUNDLED_BRIDGE_SUPPORT_PROFILE_IDS: &[&str] =
    &["native-balanced", "openclaw-ecosystem-balanced"];

#[derive(Debug, Clone)]
pub struct ResolvedBridgeSupportPolicy {
    pub profile: BridgeSupportSpec,
    pub source: String,
    pub checksum: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct MaterializedBridgeSupportTemplate {
    pub base_profile_id: String,
    pub base_source: String,
    pub source: String,
    pub derived: bool,
    pub profile: BridgeSupportSpec,
    pub checksum: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedBridgeSupportSelection {
    pub policy: ResolvedBridgeSupportPolicy,
    pub delta_source: Option<String>,
    pub delta_artifact: Option<MaterializedBridgeSupportDeltaArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaterializedBridgeSupportDeltaArtifact {
    pub base_profile_id: String,
    pub base_source: String,
    pub base_policy_version: Option<String>,
    pub delta: PluginPreflightBridgeProfileDelta,
    pub checksum: String,
    pub sha256: String,
}

pub(super) fn bridge_support_policy_checksum(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    super::fnv1a64_hex(encoded.as_bytes())
}

pub fn bridge_support_policy_sha256(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    let digest = Sha256::digest(encoded.as_bytes());
    super::hex_lower(&digest)
}

pub fn resolve_bridge_support_policy(
    path: Option<&str>,
    profile: Option<&str>,
    expected_sha256: Option<&str>,
) -> Result<Option<ResolvedBridgeSupportPolicy>, String> {
    validate_bridge_support_policy_request(path, profile, expected_sha256)?;

    let Some((profile, source)) = (match (path, profile) {
        (Some(path), None) => {
            let profile = load_bridge_support_policy_from_path(path).map_err(|error| {
                format!("failed to load bridge support policy at {path}: {error}")
            })?;
            Some((profile, path.to_owned()))
        }
        (None, Some(profile)) => Some(load_bundled_bridge_support_policy(profile)?),
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err("bridge support request cannot set both path and profile".to_owned());
        }
    }) else {
        return Ok(None);
    };

    let actual_sha256 = bridge_support_policy_sha256(&profile);
    if let Some(expected_sha256) = expected_sha256
        && !expected_sha256.eq_ignore_ascii_case(&actual_sha256)
    {
        return Err(format!(
            "bridge support policy sha256 mismatch for {source}: expected {expected_sha256}, got {actual_sha256}"
        ));
    }

    Ok(Some(ResolvedBridgeSupportPolicy {
        checksum: bridge_support_policy_checksum(&profile),
        sha256: actual_sha256,
        profile,
        source,
    }))
}

pub fn resolve_bridge_support_selection(
    path: Option<&str>,
    profile: Option<&str>,
    delta_path: Option<&str>,
    expected_sha256: Option<&str>,
    expected_delta_sha256: Option<&str>,
) -> Result<Option<ResolvedBridgeSupportSelection>, String> {
    validate_bridge_support_selection_request(
        path,
        profile,
        delta_path,
        expected_sha256,
        expected_delta_sha256,
    )?;

    match (path, profile, delta_path) {
        (None, None, None) => Ok(None),
        (Some(path), None, None) => {
            resolve_bridge_support_policy(Some(path), None, expected_sha256).map(|resolved| {
                resolved.map(|policy| ResolvedBridgeSupportSelection {
                    policy,
                    delta_source: None,
                    delta_artifact: None,
                })
            })
        }
        (None, Some(profile), None) => {
            resolve_bridge_support_policy(None, Some(profile), expected_sha256).map(|resolved| {
                resolved.map(|policy| ResolvedBridgeSupportSelection {
                    policy,
                    delta_source: None,
                    delta_artifact: None,
                })
            })
        }
        (None, None, Some(delta_path)) => {
            let loaded =
                load_bridge_support_delta_artifact_from_path(delta_path).map_err(|error| {
                    format!("failed to load bridge support delta artifact at {delta_path}: {error}")
                })?;
            let canonical = materialize_bridge_support_delta_artifact(
                loaded.base_profile_id.as_str(),
                Some(&loaded.delta),
            )?;
            validate_loaded_bridge_support_delta_artifact(delta_path, &loaded, &canonical)?;

            if let Some(expected_delta_sha256) = expected_delta_sha256
                && !expected_delta_sha256.eq_ignore_ascii_case(&canonical.sha256)
            {
                return Err(format!(
                    "bridge support delta artifact sha256 mismatch for {delta_path}: expected {expected_delta_sha256}, got {}",
                    canonical.sha256
                ));
            }

            let template = materialize_bridge_support_template(
                canonical.base_profile_id.as_str(),
                Some(&canonical.delta),
            )?;
            if let Some(expected_sha256) = expected_sha256
                && !expected_sha256.eq_ignore_ascii_case(&template.sha256)
            {
                return Err(format!(
                    "materialized bridge support policy sha256 mismatch for delta artifact {delta_path}: expected {expected_sha256}, got {}",
                    template.sha256
                ));
            }

            Ok(Some(ResolvedBridgeSupportSelection {
                policy: ResolvedBridgeSupportPolicy {
                    profile: template.profile,
                    source: format!("delta:{delta_path}"),
                    checksum: template.checksum,
                    sha256: template.sha256,
                },
                delta_source: Some(delta_path.to_owned()),
                delta_artifact: Some(canonical),
            }))
        }
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => Err(
            "bridge support selection must choose exactly one of path, profile, or delta_path"
                .to_owned(),
        ),
    }
}

pub fn materialize_bridge_support_template(
    profile_id: &str,
    delta: Option<&PluginPreflightBridgeProfileDelta>,
) -> Result<MaterializedBridgeSupportTemplate, String> {
    let resolved = resolve_bridge_support_policy(None, Some(profile_id), None)?
        .ok_or_else(|| format!("bundled bridge support profile `{profile_id}` was not resolved"))?;
    let mut profile = resolved.profile.clone();
    let derived = delta.is_some();
    if let Some(delta) = delta {
        apply_bridge_support_profile_delta(&mut profile, delta)?;
        profile.policy_version = Some(format!("custom-derived-from-{profile_id}"));
        profile.expected_checksum = None;
        profile.expected_sha256 = None;
    }

    let checksum = bridge_support_policy_checksum(&profile);
    let sha256 = bridge_support_policy_sha256(&profile);

    Ok(MaterializedBridgeSupportTemplate {
        base_profile_id: profile_id.to_owned(),
        base_source: resolved.source.clone(),
        source: if derived {
            format!("derived:{profile_id}")
        } else {
            resolved.source
        },
        derived,
        profile,
        checksum,
        sha256,
    })
}

pub fn materialize_bridge_support_delta_artifact(
    profile_id: &str,
    delta: Option<&PluginPreflightBridgeProfileDelta>,
) -> Result<MaterializedBridgeSupportDeltaArtifact, String> {
    let resolved = resolve_bridge_support_policy(None, Some(profile_id), None)?
        .ok_or_else(|| format!("bundled bridge support profile `{profile_id}` was not resolved"))?;
    let delta = normalize_bridge_support_profile_delta(delta)?;
    let canonical = bridge_support_profile_delta_canonical_json(
        profile_id,
        resolved.profile.policy_version.as_deref(),
        &delta,
    );
    let checksum = super::fnv1a64_hex(canonical.as_bytes());
    let sha256 = super::hex_lower(&Sha256::digest(canonical.as_bytes()));

    Ok(MaterializedBridgeSupportDeltaArtifact {
        base_profile_id: profile_id.to_owned(),
        base_source: resolved.source,
        base_policy_version: resolved.profile.policy_version,
        delta,
        checksum,
        sha256,
    })
}

pub fn load_bridge_support_delta_artifact_from_path(
    path: &str,
) -> Result<MaterializedBridgeSupportDeltaArtifact, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read bridge support delta artifact failed: {error}"))?;
    serde_json::from_str::<MaterializedBridgeSupportDeltaArtifact>(&raw)
        .map_err(|error| format!("parse bridge support delta artifact failed: {error}"))
}

fn validate_bridge_support_policy_request(
    path: Option<&str>,
    profile: Option<&str>,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    validate_bridge_support_selection_request(path, profile, None, expected_sha256, None)
}

fn validate_bridge_support_selection_request(
    path: Option<&str>,
    profile: Option<&str>,
    delta_path: Option<&str>,
    expected_sha256: Option<&str>,
    expected_delta_sha256: Option<&str>,
) -> Result<(), String> {
    let selector_count = usize::from(path.is_some())
        + usize::from(profile.is_some())
        + usize::from(delta_path.is_some());
    if selector_count > 1 {
        return Err(
            "bridge support policy accepts either a file path, a bundled profile, or a delta artifact, not multiple selectors"
                .to_owned(),
        );
    }
    if expected_sha256.is_some() && selector_count == 0 {
        return Err(
            "bridge support policy sha256 requires a file path, bundled profile, or delta artifact"
                .to_owned(),
        );
    }
    if expected_delta_sha256.is_some() && delta_path.is_none() {
        return Err(
            "bridge support delta artifact sha256 requires --bridge-support-delta".to_owned(),
        );
    }
    Ok(())
}

fn validate_loaded_bridge_support_delta_artifact(
    path: &str,
    loaded: &MaterializedBridgeSupportDeltaArtifact,
    canonical: &MaterializedBridgeSupportDeltaArtifact,
) -> Result<(), String> {
    if loaded.base_source != canonical.base_source {
        return Err(format!(
            "bridge support delta artifact {path} base source mismatch: expected {}, got {}",
            canonical.base_source, loaded.base_source
        ));
    }
    if loaded.base_policy_version != canonical.base_policy_version {
        return Err(format!(
            "bridge support delta artifact {path} base policy version mismatch: expected {}, got {}",
            canonical.base_policy_version.as_deref().unwrap_or("-"),
            loaded.base_policy_version.as_deref().unwrap_or("-")
        ));
    }
    if loaded.delta != canonical.delta {
        return Err(format!(
            "bridge support delta artifact {path} is not canonical for bundled profile `{}`; regenerate it with `loong plugins bridge-template --delta-output`",
            canonical.base_profile_id
        ));
    }
    if !loaded.checksum.eq_ignore_ascii_case(&canonical.checksum) {
        return Err(format!(
            "bridge support delta artifact checksum mismatch for {path}: expected {}, got {}",
            canonical.checksum, loaded.checksum
        ));
    }
    if !loaded.sha256.eq_ignore_ascii_case(&canonical.sha256) {
        return Err(format!(
            "bridge support delta artifact sha256 mismatch for {path}: expected {}, got {}",
            canonical.sha256, loaded.sha256
        ));
    }
    Ok(())
}

fn apply_bridge_support_profile_delta(
    profile: &mut BridgeSupportSpec,
    delta: &PluginPreflightBridgeProfileDelta,
) -> Result<(), String> {
    let mut supported_bridges = profile
        .supported_bridges
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for bridge in &delta.supported_bridges {
        let parsed = parse_bridge_kind_label(bridge)
            .ok_or_else(|| format!("unsupported bridge support delta bridge `{bridge}`"))?;
        supported_bridges.insert(parsed);
    }
    profile.supported_bridges = supported_bridges.into_iter().collect();

    let mut supported_adapter_families = profile
        .supported_adapter_families
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();
    for adapter_family in &delta.supported_adapter_families {
        let normalized = adapter_family.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            supported_adapter_families.insert(normalized);
        }
    }
    profile.supported_adapter_families = supported_adapter_families.into_iter().collect();

    let mut supported_compatibility_modes = profile
        .supported_compatibility_modes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    for compatibility_mode in &delta.supported_compatibility_modes {
        let parsed = parse_plugin_activation_runtime_mode(compatibility_mode).ok_or_else(|| {
            format!("unsupported bridge support delta compatibility mode `{compatibility_mode}`")
        })?;
        supported_compatibility_modes.insert(parsed);
    }
    profile.supported_compatibility_modes = supported_compatibility_modes.into_iter().collect();

    let mut supported_compatibility_shims = profile
        .supported_compatibility_shims
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for compatibility_shim in &delta.supported_compatibility_shims {
        supported_compatibility_shims.insert(parse_bridge_support_delta_shim(
            compatibility_shim.as_str(),
        )?);
    }

    let mut shim_profiles = profile
        .supported_compatibility_shim_profiles
        .iter()
        .cloned()
        .map(PluginCompatibilityShimSupport::normalized)
        .map(|profile| {
            let key = compatibility_shim_profile_key(&profile.shim, profile.version.as_deref());
            (key, profile)
        })
        .collect::<BTreeMap<_, _>>();

    for addition in &delta.shim_profile_additions {
        let shim = PluginCompatibilityShim {
            shim_id: addition.shim_id.trim().to_owned(),
            family: addition.shim_family.trim().to_owned(),
        };
        if shim.shim_id.is_empty() || shim.family.is_empty() {
            return Err(
                "bridge support delta shim_profile_additions require shim_id and shim_family"
                    .to_owned(),
            );
        }
        supported_compatibility_shims.insert(shim.clone());
        let mut matching_keys = shim_profiles
            .keys()
            .filter(|(existing_shim, _)| *existing_shim == shim)
            .cloned()
            .collect::<Vec<_>>();
        if matching_keys.is_empty() {
            let fallback_key = compatibility_shim_profile_key(&shim, None);
            matching_keys.push(fallback_key);
        }

        for matching_key in matching_keys {
            let entry = shim_profiles.entry(matching_key).or_insert_with(|| {
                PluginCompatibilityShimSupport {
                    shim: shim.clone(),
                    version: None,
                    supported_dialects: BTreeSet::new(),
                    supported_bridges: BTreeSet::new(),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::new(),
                }
            });

            for dialect in &addition.supported_dialects {
                let parsed = parse_plugin_activation_runtime_dialect(dialect).ok_or_else(|| {
                    format!("unsupported bridge support delta dialect `{dialect}`")
                })?;
                entry.supported_dialects.insert(parsed);
            }
            for bridge in &addition.supported_bridges {
                let parsed = parse_bridge_kind_label(bridge)
                    .ok_or_else(|| format!("unsupported bridge support delta bridge `{bridge}`"))?;
                entry.supported_bridges.insert(parsed);
            }
            for adapter_family in &addition.supported_adapter_families {
                let normalized = adapter_family.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    entry.supported_adapter_families.insert(normalized);
                }
            }
            for source_language in &addition.supported_source_languages {
                let normalized = normalize_runtime_source_language(source_language);
                if normalized != "unknown" {
                    entry.supported_source_languages.insert(normalized);
                }
            }
        }
    }

    profile.supported_compatibility_shims = supported_compatibility_shims.into_iter().collect();
    profile.supported_compatibility_shim_profiles = shim_profiles
        .into_values()
        .map(PluginCompatibilityShimSupport::normalized)
        .collect();
    profile
        .supported_compatibility_shim_profiles
        .sort_by(|left, right| {
            (
                left.shim.shim_id.as_str(),
                left.shim.family.as_str(),
                left.version.as_deref().unwrap_or_default(),
            )
                .cmp(&(
                    right.shim.shim_id.as_str(),
                    right.shim.family.as_str(),
                    right.version.as_deref().unwrap_or_default(),
                ))
        });

    Ok(())
}

fn compatibility_shim_profile_key(
    shim: &PluginCompatibilityShim,
    version: Option<&str>,
) -> (PluginCompatibilityShim, Option<String>) {
    let normalized_version = version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    (shim.clone(), normalized_version)
}

fn normalize_bridge_support_profile_delta(
    delta: Option<&PluginPreflightBridgeProfileDelta>,
) -> Result<PluginPreflightBridgeProfileDelta, String> {
    let Some(delta) = delta else {
        return Ok(PluginPreflightBridgeProfileDelta::default());
    };

    let mut supported_bridges = BTreeSet::new();
    for bridge in &delta.supported_bridges {
        let parsed = parse_bridge_kind_label(bridge)
            .ok_or_else(|| format!("unsupported bridge support delta bridge `{bridge}`"))?;
        supported_bridges.insert(parsed.as_str().to_owned());
    }

    let supported_adapter_families = delta
        .supported_adapter_families
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();

    let mut supported_compatibility_modes = BTreeSet::new();
    for compatibility_mode in &delta.supported_compatibility_modes {
        let parsed = parse_plugin_activation_runtime_mode(compatibility_mode).ok_or_else(|| {
            format!("unsupported bridge support delta compatibility mode `{compatibility_mode}`")
        })?;
        supported_compatibility_modes.insert(parsed.as_str().to_owned());
    }

    let mut supported_compatibility_shims = BTreeSet::new();
    for shim in &delta.supported_compatibility_shims {
        supported_compatibility_shims.insert(format_bridge_support_delta_shim(
            &parse_bridge_support_delta_shim(shim)?,
        ));
    }

    let shim_profile_additions =
        normalize_bridge_support_shim_profile_additions(&delta.shim_profile_additions)?;

    let unresolved_blocking_reasons = delta
        .unresolved_blocking_reasons
        .iter()
        .map(|reason| reason.trim().to_owned())
        .filter(|reason| !reason.is_empty())
        .collect::<BTreeSet<_>>();

    Ok(PluginPreflightBridgeProfileDelta {
        supported_bridges: supported_bridges.into_iter().collect(),
        supported_adapter_families: supported_adapter_families.into_iter().collect(),
        supported_compatibility_modes: supported_compatibility_modes.into_iter().collect(),
        supported_compatibility_shims: supported_compatibility_shims.into_iter().collect(),
        shim_profile_additions,
        unresolved_blocking_reasons: unresolved_blocking_reasons.into_iter().collect(),
    })
}

fn normalize_bridge_support_shim_profile_additions(
    additions: &[PluginPreflightBridgeShimProfileDelta],
) -> Result<Vec<PluginPreflightBridgeShimProfileDelta>, String> {
    let mut merged_by_shim = BTreeMap::new();

    for addition in additions {
        let normalized = normalize_bridge_support_shim_profile_delta(addition)?;
        let key = (normalized.shim_id.clone(), normalized.shim_family.clone());
        let entry = merged_by_shim.entry(key.clone()).or_insert_with(|| {
            PluginPreflightBridgeShimProfileDelta {
                shim_id: key.0.clone(),
                shim_family: key.1.clone(),
                supported_dialects: Vec::new(),
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_source_languages: Vec::new(),
            }
        });

        merge_bridge_support_shim_profile_delta(entry, &normalized);
    }

    Ok(merged_by_shim.into_values().collect())
}

fn merge_bridge_support_shim_profile_delta(
    target: &mut PluginPreflightBridgeShimProfileDelta,
    addition: &PluginPreflightBridgeShimProfileDelta,
) {
    merge_sorted_unique_strings(&mut target.supported_dialects, &addition.supported_dialects);
    merge_sorted_unique_strings(&mut target.supported_bridges, &addition.supported_bridges);
    merge_sorted_unique_strings(
        &mut target.supported_adapter_families,
        &addition.supported_adapter_families,
    );
    merge_sorted_unique_strings(
        &mut target.supported_source_languages,
        &addition.supported_source_languages,
    );
}

fn merge_sorted_unique_strings(target: &mut Vec<String>, values: &[String]) {
    let mut merged = target.iter().cloned().collect::<BTreeSet<_>>();

    for value in values {
        merged.insert(value.clone());
    }

    *target = merged.into_iter().collect();
}

fn normalize_bridge_support_shim_profile_delta(
    addition: &PluginPreflightBridgeShimProfileDelta,
) -> Result<PluginPreflightBridgeShimProfileDelta, String> {
    let shim_id = addition.shim_id.trim().to_owned();
    let shim_family = addition.shim_family.trim().to_owned();
    if shim_id.is_empty() || shim_family.is_empty() {
        return Err(
            "bridge support delta shim_profile_additions require shim_id and shim_family"
                .to_owned(),
        );
    }

    let mut supported_dialects = BTreeSet::new();
    for dialect in &addition.supported_dialects {
        let parsed = parse_plugin_activation_runtime_dialect(dialect)
            .ok_or_else(|| format!("unsupported bridge support delta dialect `{dialect}`"))?;
        supported_dialects.insert(parsed.as_str().to_owned());
    }

    let mut supported_bridges = BTreeSet::new();
    for bridge in &addition.supported_bridges {
        let parsed = parse_bridge_kind_label(bridge)
            .ok_or_else(|| format!("unsupported bridge support delta bridge `{bridge}`"))?;
        supported_bridges.insert(parsed.as_str().to_owned());
    }

    let supported_adapter_families = addition
        .supported_adapter_families
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>();

    let supported_source_languages = addition
        .supported_source_languages
        .iter()
        .map(|value| normalize_runtime_source_language(value))
        .filter(|value| value != "unknown")
        .collect::<BTreeSet<_>>();

    Ok(PluginPreflightBridgeShimProfileDelta {
        shim_id,
        shim_family,
        supported_dialects: supported_dialects.into_iter().collect(),
        supported_bridges: supported_bridges.into_iter().collect(),
        supported_adapter_families: supported_adapter_families.into_iter().collect(),
        supported_source_languages: supported_source_languages.into_iter().collect(),
    })
}

fn format_bridge_support_delta_shim(shim: &PluginCompatibilityShim) -> String {
    format!("{}:{}", shim.shim_id, shim.family)
}

fn parse_bridge_support_delta_shim(raw: &str) -> Result<PluginCompatibilityShim, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("bridge support delta compatibility shim cannot be empty".to_owned());
    }

    let mut segments = trimmed.splitn(2, ':');
    let shim_id = segments.next().unwrap_or_default().trim();
    let family = segments.next().map(str::trim).unwrap_or(shim_id);
    if shim_id.is_empty() || family.is_empty() {
        return Err(format!(
            "bridge support delta compatibility shim `{trimmed}` must be `shim_id[:family]`"
        ));
    }

    Ok(PluginCompatibilityShim {
        shim_id: shim_id.to_owned(),
        family: family.to_owned(),
    })
}

pub fn load_bridge_support_policy_from_path(path: &str) -> Result<BridgeSupportSpec, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read bridge support policy failed: {error}"))?;
    serde_json::from_str::<BridgeSupportSpec>(&raw)
        .map_err(|error| format!("parse bridge support policy failed: {error}"))
}

pub fn load_bundled_bridge_support_policy(
    profile: &str,
) -> Result<(BridgeSupportSpec, String), String> {
    match profile {
        "native-balanced" => Ok((
            bundled_bridge_support_native_balanced()?,
            "bundled:bridge-support-native-balanced.json".to_owned(),
        )),
        "openclaw-ecosystem-balanced" => Ok((
            bundled_bridge_support_openclaw_ecosystem_balanced()?,
            "bundled:bridge-support-openclaw-ecosystem-balanced.json".to_owned(),
        )),
        other => Err(format!(
            "unknown bridge support bundled profile `{other}` (expected one of: native-balanced, openclaw-ecosystem-balanced)"
        )),
    }
}

fn bundled_bridge_support_native_balanced() -> Result<BridgeSupportSpec, String> {
    BUNDLED_BRIDGE_SUPPORT_NATIVE_BALANCED
        .get_or_init(|| {
            let raw = include_str!("../../config/bridge-support-native-balanced.json");
            serde_json::from_str(raw).map_err(|error| {
                format!("bundled native bridge support profile should parse: {error}")
            })
        })
        .clone()
}

fn bundled_bridge_support_openclaw_ecosystem_balanced() -> Result<BridgeSupportSpec, String> {
    BUNDLED_BRIDGE_SUPPORT_OPENCLAW_ECOSYSTEM_BALANCED
        .get_or_init(|| {
            let raw = include_str!("../../config/bridge-support-openclaw-ecosystem-balanced.json");
            serde_json::from_str(raw).map_err(|error| {
                format!("bundled openclaw ecosystem bridge support profile should parse: {error}")
            })
        })
        .clone()
}

fn bridge_support_policy_canonical_json(bridge: &BridgeSupportSpec) -> String {
    let mut bridges = bridge.supported_bridges.clone();
    bridges.sort();

    let mut adapter_families = bridge.supported_adapter_families.clone();
    adapter_families.sort();
    let mut compatibility_modes = bridge.supported_compatibility_modes.clone();
    compatibility_modes.sort();
    let mut compatibility_shims = bridge.supported_compatibility_shims.clone();
    compatibility_shims.sort();
    let compatibility_shim_profiles =
        canonical_compatibility_shim_profiles(&bridge.supported_compatibility_shim_profiles);
    let mut allowed_commands = bridge.allowed_process_commands.clone();
    allowed_commands.sort();
    let security_scan = canonical_security_scan_value(bridge.security_scan.as_ref());

    let canonical = json!({
        "enabled": bridge.enabled,
        "supported_bridges": bridges,
        "supported_adapter_families": adapter_families,
        "supported_compatibility_modes": compatibility_modes,
        "supported_compatibility_shims": compatibility_shims,
        "supported_compatibility_shim_profiles": compatibility_shim_profiles,
        "enforce_supported": bridge.enforce_supported,
        "execute_process_stdio": bridge.execute_process_stdio,
        "execute_http_json": bridge.execute_http_json,
        "allowed_process_commands": allowed_commands,
        "enforce_execution_success": bridge.enforce_execution_success,
        "security_scan": security_scan,
    });

    serde_json::to_string(&canonical).unwrap_or_default()
}

fn bridge_support_profile_delta_canonical_json(
    profile_id: &str,
    policy_version: Option<&str>,
    delta: &PluginPreflightBridgeProfileDelta,
) -> String {
    let canonical = json!({
        "base_profile_id": profile_id,
        "base_policy_version": policy_version,
        "delta": {
            "supported_bridges": delta.supported_bridges,
            "supported_adapter_families": delta.supported_adapter_families,
            "supported_compatibility_modes": delta.supported_compatibility_modes,
            "supported_compatibility_shims": delta.supported_compatibility_shims,
            "shim_profile_additions": delta.shim_profile_additions,
            "unresolved_blocking_reasons": delta.unresolved_blocking_reasons,
        },
    });

    serde_json::to_string(&canonical).unwrap_or_default()
}

fn canonical_compatibility_shim_profiles(
    profiles: &[PluginCompatibilityShimSupport],
) -> Vec<Value> {
    let mut normalized = profiles
        .iter()
        .cloned()
        .map(PluginCompatibilityShimSupport::normalized)
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| {
        (
            left.shim.shim_id.as_str(),
            left.shim.family.as_str(),
            left.version.as_deref().unwrap_or_default(),
        )
            .cmp(&(
                right.shim.shim_id.as_str(),
                right.shim.family.as_str(),
                right.version.as_deref().unwrap_or_default(),
            ))
    });

    normalized
        .into_iter()
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

            let supported_adapter_families = profile
                .supported_adapter_families
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let supported_source_languages = profile
                .supported_source_languages
                .iter()
                .cloned()
                .collect::<Vec<_>>();

            json!({
                "shim": {
                    "shim_id": profile.shim.shim_id,
                    "family": profile.shim.family,
                },
                "version": profile.version,
                "supported_dialects": supported_dialects,
                "supported_bridges": supported_bridges,
                "supported_adapter_families": supported_adapter_families,
                "supported_source_languages": supported_source_languages,
            })
        })
        .collect()
}

fn canonical_security_scan_value(security_scan: Option<&SecurityScanSpec>) -> Value {
    let Some(scan) = security_scan else {
        return Value::Null;
    };

    let mut keywords = scan.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = scan.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = scan.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = scan
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let profile_signature =
        canonical_security_scan_profile_signature_value(scan.profile_signature.as_ref());
    let siem_export = canonical_security_scan_siem_export_value(scan.siem_export.as_ref());
    let runtime = canonical_security_scan_runtime_value(&scan.runtime);

    json!({
        "enabled": scan.enabled,
        "block_on_high": scan.block_on_high,
        "profile_path": scan.profile_path,
        "profile_sha256": scan.profile_sha256,
        "profile_signature": profile_signature,
        "siem_export": siem_export,
        "runtime": runtime,
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": scan.wasm.enabled,
            "max_module_bytes": scan.wasm.max_module_bytes,
            "allow_wasi": scan.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": scan.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        },
    })
}

fn canonical_security_scan_profile_signature_value(
    signature: Option<&SecurityProfileSignatureSpec>,
) -> Value {
    let Some(signature) = signature else {
        return Value::Null;
    };
    json!({
        "algorithm": signature.algorithm.trim().to_ascii_lowercase(),
        "public_key_base64": signature.public_key_base64,
        "signature_base64": signature.signature_base64,
    })
}

fn canonical_security_scan_siem_export_value(export: Option<&SecuritySiemExportSpec>) -> Value {
    let Some(export) = export else {
        return Value::Null;
    };
    json!({
        "enabled": export.enabled,
        "path": export.path,
        "include_findings": export.include_findings,
        "max_findings_per_record": export.max_findings_per_record,
        "fail_on_error": export.fail_on_error,
    })
}

fn canonical_security_scan_runtime_value(runtime: &SecurityRuntimeExecutionSpec) -> Value {
    let mut allowed_path_prefixes = runtime.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();
    let mut guest_readable_config_keys = runtime.guest_readable_config_keys.clone();
    guest_readable_config_keys.sort();
    let bridge_circuit_breaker = json!({
        "enabled": runtime.bridge_circuit_breaker.enabled,
        "failure_threshold": runtime.bridge_circuit_breaker.failure_threshold,
        "cooldown_ms": runtime.bridge_circuit_breaker.cooldown_ms,
        "half_open_max_calls": runtime.bridge_circuit_breaker.half_open_max_calls,
        "success_threshold": runtime.bridge_circuit_breaker.success_threshold,
    });

    json!({
        "execute_wasm_component": runtime.execute_wasm_component,
        "allowed_path_prefixes": allowed_path_prefixes,
        "guest_readable_config_keys": guest_readable_config_keys,
        "max_component_bytes": runtime.max_component_bytes,
        "max_output_bytes": runtime.max_output_bytes,
        "fuel_limit": runtime.fuel_limit,
        "bridge_circuit_breaker": bridge_circuit_breaker,
        "timeout_ms": runtime.timeout_ms,
    })
}

fn canonical_security_scan_profile_value(profile: &SecurityScanProfile) -> Value {
    let mut keywords = profile.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = profile.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = profile.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = profile
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();

    json!({
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": profile.wasm.enabled,
            "max_module_bytes": profile.wasm.max_module_bytes,
            "allow_wasi": profile.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": profile.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        }
    })
}

pub fn security_scan_profile_sha256(profile: &SecurityScanProfile) -> String {
    let encoded = security_scan_profile_message(profile);
    let digest = Sha256::digest(&encoded);
    super::hex_lower(&digest)
}

pub fn security_scan_profile_message(profile: &SecurityScanProfile) -> Vec<u8> {
    let canonical = canonical_security_scan_profile_value(profile);
    serde_json::to_vec(&canonical).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::spec_runtime::{
        BridgeSupportSpec, PluginPreflightBridgeProfileDelta, PluginPreflightBridgeShimProfileDelta,
    };
    use kernel::{
        PluginBridgeKind, PluginCompatibilityMode, PluginCompatibilityShim,
        PluginCompatibilityShimSupport, PluginContractDialect,
    };

    fn unique_temp_path(prefix: &str, suffix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("{prefix}-{nanos}.{suffix}"))
            .display()
            .to_string()
    }

    #[test]
    fn bridge_support_policy_sha_includes_compatibility_shims() {
        let mut baseline = BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: vec!["javascript-stdio-adapter".to_owned()],
            supported_compatibility_modes: vec![PluginCompatibilityMode::OpenClawModern],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: Some("test".to_owned()),
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["node".to_owned()],
            enforce_execution_success: false,
            security_scan: None,
        };
        let without_shim = bridge_support_policy_sha256(&baseline);

        baseline.supported_compatibility_shims = vec![PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        }];
        let with_shim = bridge_support_policy_sha256(&baseline);

        assert_ne!(without_shim, with_shim);
    }

    #[test]
    fn bridge_support_policy_sha_includes_compatibility_shim_profiles() {
        let mut baseline = BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: vec!["javascript-stdio-adapter".to_owned()],
            supported_compatibility_modes: vec![PluginCompatibilityMode::OpenClawModern],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: Some("test".to_owned()),
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["node".to_owned()],
            enforce_execution_success: false,
            security_scan: None,
        };
        let without_profile = bridge_support_policy_sha256(&baseline);

        baseline.supported_compatibility_shim_profiles = vec![PluginCompatibilityShimSupport {
            shim: PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            },
            version: Some("openclaw-modern@1".to_owned()),
            supported_dialects: std::collections::BTreeSet::from([
                PluginContractDialect::OpenClawModernManifest,
            ]),
            supported_bridges: std::collections::BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: std::collections::BTreeSet::new(),
            supported_source_languages: std::collections::BTreeSet::from(["javascript".to_owned()]),
        }];
        let with_profile = bridge_support_policy_sha256(&baseline);

        baseline.supported_compatibility_shim_profiles[0].version =
            Some("openclaw-modern@2".to_owned());
        let with_profile_v2 = bridge_support_policy_sha256(&baseline);

        assert_ne!(without_profile, with_profile);
        assert_ne!(with_profile, with_profile_v2);
    }

    #[test]
    fn bridge_support_policy_sha_changes_when_enabled_flag_changes() {
        let mut baseline = BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: Some("test".to_owned()),
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["node".to_owned()],
            enforce_execution_success: false,
            security_scan: None,
        };
        let enabled_sha = bridge_support_policy_sha256(&baseline);

        baseline.enabled = false;
        let disabled_sha = bridge_support_policy_sha256(&baseline);

        assert_ne!(enabled_sha, disabled_sha);
    }

    #[test]
    fn apply_bridge_support_profile_delta_preserves_versioned_shim_profiles() {
        let shim = PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        };
        let mut baseline = BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::OpenClawModern],
            supported_compatibility_shims: vec![shim.clone()],
            supported_compatibility_shim_profiles: vec![
                PluginCompatibilityShimSupport {
                    shim: shim.clone(),
                    version: Some("openclaw-modern@1".to_owned()),
                    supported_dialects: BTreeSet::new(),
                    supported_bridges: BTreeSet::new(),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["python".to_owned()]),
                },
                PluginCompatibilityShimSupport {
                    shim,
                    version: Some("openclaw-modern@2".to_owned()),
                    supported_dialects: BTreeSet::new(),
                    supported_bridges: BTreeSet::new(),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["javascript".to_owned()]),
                },
            ],
            enforce_supported: true,
            policy_version: Some("test".to_owned()),
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        };
        let delta = PluginPreflightBridgeProfileDelta {
            supported_bridges: Vec::new(),
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: Vec::new(),
            supported_compatibility_shims: Vec::new(),
            shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                shim_id: "openclaw-modern-compat".to_owned(),
                shim_family: "openclaw-modern-compat".to_owned(),
                supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                supported_bridges: vec!["process_stdio".to_owned()],
                supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                supported_source_languages: vec!["ruby".to_owned()],
            }],
            unresolved_blocking_reasons: Vec::new(),
        };

        apply_bridge_support_profile_delta(&mut baseline, &delta)
            .expect("delta should preserve versioned shim profiles");

        assert_eq!(baseline.supported_compatibility_shim_profiles.len(), 2);
        assert!(
            baseline
                .supported_compatibility_shim_profiles
                .iter()
                .any(|profile| {
                    profile.version.as_deref() == Some("openclaw-modern@1")
                        && profile.supported_source_languages.contains("python")
                        && profile.supported_source_languages.contains("ruby")
                })
        );
        assert!(
            baseline
                .supported_compatibility_shim_profiles
                .iter()
                .any(|profile| {
                    profile.version.as_deref() == Some("openclaw-modern@2")
                        && profile.supported_source_languages.contains("javascript")
                        && profile.supported_source_languages.contains("ruby")
                })
        );
    }

    #[test]
    fn resolve_bridge_support_policy_accepts_bundled_profile_with_matching_sha() {
        let (bundled, source) = load_bundled_bridge_support_policy("openclaw-ecosystem-balanced")
            .expect("bundled bridge support profile should resolve");
        let sha256 = bridge_support_policy_sha256(&bundled);

        let resolved = resolve_bridge_support_policy(
            None,
            Some("openclaw-ecosystem-balanced"),
            Some(sha256.as_str()),
        )
        .expect("bundled bridge support policy should resolve")
        .expect("bundled bridge support policy should be present");

        assert_eq!(resolved.source, source);
        assert_eq!(resolved.sha256, sha256);
        assert_eq!(
            resolved.profile.policy_version.as_deref(),
            Some("openclaw-ecosystem-balanced@1")
        );
    }

    #[test]
    fn resolve_bridge_support_policy_rejects_mixed_path_and_profile() {
        let error = resolve_bridge_support_policy(
            Some("/tmp/bridge-support.json"),
            Some("native-balanced"),
            None,
        )
        .expect_err("mixed bridge support policy selectors should fail");

        assert!(error.contains("delta artifact"));
        assert!(error.contains("not multiple selectors"));
    }

    #[test]
    fn resolve_bridge_support_policy_rejects_unknown_bundled_profile() {
        let error = resolve_bridge_support_policy(None, Some("unknown"), None)
            .expect_err("unknown bundled bridge support profile should fail");

        assert!(error.contains("unknown bridge support bundled profile"));
        assert!(error.contains("native-balanced"));
    }

    #[test]
    fn materialize_bridge_support_template_returns_bundled_profile_when_delta_is_empty() {
        let template = materialize_bridge_support_template("native-balanced", None)
            .expect("bundled template should materialize");

        assert!(!template.derived);
        assert_eq!(template.base_profile_id, "native-balanced");
        assert_eq!(
            template.base_source,
            "bundled:bridge-support-native-balanced.json"
        );
        assert_eq!(
            template.source,
            "bundled:bridge-support-native-balanced.json"
        );
        assert_eq!(
            template.profile.policy_version.as_deref(),
            Some("native-balanced@1")
        );
        assert_eq!(
            template.sha256,
            bridge_support_policy_sha256(&template.profile)
        );
    }

    #[test]
    fn materialize_bridge_support_template_merges_custom_delta_into_base_profile() {
        let template = materialize_bridge_support_template(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
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
        .expect("delta template should materialize");

        assert!(template.derived);
        assert_eq!(template.base_profile_id, "openclaw-ecosystem-balanced");
        assert_eq!(template.source, "derived:openclaw-ecosystem-balanced");
        assert_eq!(
            template.profile.policy_version.as_deref(),
            Some("custom-derived-from-openclaw-ecosystem-balanced")
        );
        assert!(
            template
                .profile
                .supported_compatibility_shim_profiles
                .iter()
                .any(|profile| {
                    profile.shim.shim_id == "openclaw-modern-compat"
                        && profile.supported_source_languages.contains("python")
                })
        );
    }

    #[test]
    fn materialize_bridge_support_delta_artifact_normalizes_and_hashes_delta() {
        let artifact = materialize_bridge_support_delta_artifact(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: vec!["process_stdio".to_owned(), "process_stdio".to_owned()],
                supported_adapter_families: vec![
                    " OpenClaw-Modern-Compat ".to_owned(),
                    "openclaw-modern-compat".to_owned(),
                ],
                supported_compatibility_modes: vec![
                    "openclaw_modern".to_owned(),
                    "openclaw_modern".to_owned(),
                ],
                supported_compatibility_shims: vec![
                    "openclaw-modern-compat".to_owned(),
                    "openclaw-modern-compat:openclaw-modern-compat".to_owned(),
                ],
                shim_profile_additions: vec![
                    PluginPreflightBridgeShimProfileDelta {
                        shim_id: " openclaw-modern-compat ".to_owned(),
                        shim_family: " openclaw-modern-compat ".to_owned(),
                        supported_dialects: vec![
                            "openclaw_modern_manifest".to_owned(),
                            "openclaw_modern_manifest".to_owned(),
                        ],
                        supported_bridges: vec![
                            "process_stdio".to_owned(),
                            "process_stdio".to_owned(),
                        ],
                        supported_adapter_families: vec![
                            " OpenClaw-Modern-Compat ".to_owned(),
                            "openclaw-modern-compat".to_owned(),
                        ],
                        supported_source_languages: vec!["Python".to_owned(), "python".to_owned()],
                    },
                    PluginPreflightBridgeShimProfileDelta {
                        shim_id: "openclaw-modern-compat".to_owned(),
                        shim_family: "openclaw-modern-compat".to_owned(),
                        supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                        supported_bridges: vec!["process_stdio".to_owned()],
                        supported_adapter_families: vec![
                            "openclaw-modern-extra".to_owned(),
                            "openclaw-modern-compat".to_owned(),
                        ],
                        supported_source_languages: vec!["javascript".to_owned()],
                    },
                ],
                unresolved_blocking_reasons: vec![
                    "shim_support_profile_mismatch".to_owned(),
                    "shim_support_profile_mismatch".to_owned(),
                ],
            }),
        )
        .expect("delta artifact should materialize");

        assert_eq!(artifact.base_profile_id, "openclaw-ecosystem-balanced");
        assert_eq!(
            artifact.base_source,
            "bundled:bridge-support-openclaw-ecosystem-balanced.json"
        );
        assert_eq!(
            artifact.base_policy_version.as_deref(),
            Some("openclaw-ecosystem-balanced@1")
        );
        assert_eq!(
            artifact.delta.supported_adapter_families,
            vec!["openclaw-modern-compat".to_owned()]
        );
        assert_eq!(
            artifact.delta.supported_compatibility_shims,
            vec!["openclaw-modern-compat:openclaw-modern-compat".to_owned()]
        );
        assert_eq!(artifact.delta.shim_profile_additions.len(), 1);
        assert_eq!(
            artifact.delta.shim_profile_additions[0].supported_adapter_families,
            vec![
                "openclaw-modern-compat".to_owned(),
                "openclaw-modern-extra".to_owned(),
            ]
        );
        assert_eq!(
            artifact.delta.shim_profile_additions[0].supported_source_languages,
            vec!["javascript".to_owned(), "python".to_owned()]
        );
        assert!(!artifact.checksum.is_empty());
        assert_eq!(artifact.sha256.len(), 64);
    }

    #[test]
    fn resolve_bridge_support_selection_materializes_canonical_delta_artifact() {
        let artifact = materialize_bridge_support_delta_artifact(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
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
        let path = unique_temp_path("loong-bridge-delta", "json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&artifact).expect("serialize artifact"),
        )
        .expect("write artifact");

        let resolved = resolve_bridge_support_selection(
            None,
            None,
            Some(path.as_str()),
            None,
            Some(artifact.sha256.as_str()),
        )
        .expect("delta artifact should resolve")
        .expect("selection should be present");

        assert_eq!(resolved.policy.source, format!("delta:{path}"));
        assert_eq!(
            resolved.policy.profile.policy_version.as_deref(),
            Some("custom-derived-from-openclaw-ecosystem-balanced")
        );
        assert_eq!(
            resolved
                .delta_artifact
                .as_ref()
                .expect("delta artifact should be retained")
                .sha256,
            artifact.sha256
        );
    }

    #[test]
    fn resolve_bridge_support_selection_rejects_stale_delta_artifact_base_version() {
        let mut artifact = materialize_bridge_support_delta_artifact(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
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
        artifact.base_policy_version = Some("openclaw-ecosystem-balanced@0".to_owned());
        let path = unique_temp_path("loong-bridge-delta-stale", "json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&artifact).expect("serialize artifact"),
        )
        .expect("write artifact");

        let error = resolve_bridge_support_selection(None, None, Some(path.as_str()), None, None)
            .expect_err("stale delta artifact should fail");
        assert!(error.contains("base policy version mismatch"));
    }

    #[test]
    fn load_bridge_support_policy_rejects_unknown_fields() {
        let path = unique_temp_path("loong-bridge-support-unknown", "json");
        let value = serde_json::json!({
            "enabled": true,
            "supported_bridges": ["process_stdio"],
            "supported_adapter_families": [],
            "supported_compatibility_modes": ["native"],
            "supported_compatibility_shims": [],
            "supported_compatibility_shim_profiles": [],
            "enforce_supported": true,
            "policy_version": "test",
            "expected_checksum": null,
            "expected_sha256": null,
            "execute_process_stdio": true,
            "execute_http_json": false,
            "allowed_process_commands": [],
            "enforce_execution_success": false,
            "security_scan": null,
            "unexpected_field": true
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&value).expect("serialize bridge policy"),
        )
        .expect("write bridge policy");

        let error = load_bridge_support_policy_from_path(path.as_str())
            .expect_err("unknown bridge policy fields should fail");

        assert!(error.contains("unknown field"));
        assert!(error.contains("unexpected_field"));
    }

    #[test]
    fn load_bridge_support_delta_artifact_rejects_unknown_fields() {
        let artifact = materialize_bridge_support_delta_artifact(
            "openclaw-ecosystem-balanced",
            Some(&PluginPreflightBridgeProfileDelta {
                supported_bridges: Vec::new(),
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
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
        let path = unique_temp_path("loong-bridge-delta-unknown", "json");
        let mut value = serde_json::to_value(&artifact).expect("encode delta artifact");
        value["delta"]["unexpected_field"] = serde_json::json!(true);
        fs::write(
            &path,
            serde_json::to_string_pretty(&value).expect("serialize delta artifact"),
        )
        .expect("write delta artifact");

        let error = load_bridge_support_delta_artifact_from_path(path.as_str())
            .expect_err("unknown delta artifact fields should fail");

        assert!(error.contains("unknown field"));
        assert!(error.contains("unexpected_field"));
    }
}
