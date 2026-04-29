use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) const PROJECT_LOCAL_LOONG_EXTENSION_ROOT: &str = ".loong/extensions/";
pub(crate) const GLOBAL_LOONG_EXTENSION_ROOT: &str = "~/.loong/agent/extensions/";
pub(crate) const PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE: &str = "project_local_over_global";
pub(crate) const REVIEW_GLOBAL_DUPLICATE_ACTION: &str = "review_global_duplicate";
pub(crate) const INSPECT_EFFECTIVE_PACKAGE_ACTION: &str = "inspect_effective_package";
pub(crate) const INSPECT_SHADOWED_PACKAGE_ACTION: &str = "inspect_shadowed_package";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePluginDiscoveryGuidanceView {
    pub precedence_rule: String,
    pub project_local_root: String,
    pub global_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_plugin_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_conflicts: Vec<RuntimePluginShadowingConflictView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discovery_actions: Vec<RuntimePluginDiscoveryActionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePluginShadowingConflictView {
    pub plugin_id: String,
    pub effective_source_path: String,
    pub shadowed_source_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePluginDiscoveryActionView {
    pub kind: String,
    pub plugin_id: String,
    pub target_source_path: String,
    pub target_package_root: String,
    pub summary: String,
    pub command: String,
}

pub fn build_runtime_plugin_shadowing_conflicts<T, FId, FPath>(
    effective: &[T],
    shadowed_by_plugin_id: &BTreeMap<String, Vec<T>>,
    plugin_id_of: FId,
    source_path_of: FPath,
) -> Vec<RuntimePluginShadowingConflictView>
where
    FId: Fn(&T) -> &str,
    FPath: Fn(&T) -> &str,
{
    let effective_by_plugin_id = effective
        .iter()
        .map(|item| {
            (
                plugin_id_of(item).trim().to_owned(),
                source_path_of(item).to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    shadowed_by_plugin_id
        .iter()
        .filter_map(|(plugin_id, shadowed_items)| {
            let effective_source_path = effective_by_plugin_id.get(plugin_id)?.to_owned();
            let shadowed_source_paths = shadowed_items
                .iter()
                .map(&source_path_of)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            Some(RuntimePluginShadowingConflictView {
                plugin_id: plugin_id.clone(),
                effective_source_path,
                shadowed_source_paths,
            })
        })
        .collect()
}

pub fn build_runtime_plugin_discovery_guidance(
    roots_source: Option<&str>,
    shadowed_conflicts: Vec<RuntimePluginShadowingConflictView>,
) -> Option<RuntimePluginDiscoveryGuidanceView> {
    if roots_source != Some("auto_discovered") {
        return None;
    }

    let shadowed_plugin_ids = shadowed_conflicts
        .iter()
        .map(|conflict| conflict.plugin_id.clone())
        .collect::<Vec<_>>();
    let has_shadowed_plugins = !shadowed_plugin_ids.is_empty();
    let discovery_actions = shadowed_conflicts
        .iter()
        .flat_map(build_runtime_plugin_discovery_actions_for_conflict)
        .collect::<Vec<_>>();
    let resolution_hint = has_shadowed_plugins.then(|| {
        let conflict_examples = shadowed_conflicts
            .iter()
            .map(|conflict| {
                format!(
                    "{} => {} (shadowed: {})",
                    conflict.plugin_id,
                    conflict.effective_source_path,
                    conflict.shadowed_source_paths.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "Project-local `{}` overrides `{}` for conflicts: {}. Remove or rename the global duplicate if the override is accidental.",
            PROJECT_LOCAL_LOONG_EXTENSION_ROOT.trim_end_matches('/'),
            GLOBAL_LOONG_EXTENSION_ROOT.trim_end_matches('/'),
            conflict_examples
        )
    });

    Some(RuntimePluginDiscoveryGuidanceView {
        precedence_rule: PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE.to_owned(),
        project_local_root: PROJECT_LOCAL_LOONG_EXTENSION_ROOT.to_owned(),
        global_root: GLOBAL_LOONG_EXTENSION_ROOT.to_owned(),
        shadowed_plugin_ids,
        shadowed_conflicts,
        discovery_actions,
        recommended_action: has_shadowed_plugins.then(|| REVIEW_GLOBAL_DUPLICATE_ACTION.to_owned()),
        resolution_hint,
    })
}

pub fn build_runtime_plugin_discovery_next_steps(
    guidance: Option<&RuntimePluginDiscoveryGuidanceView>,
) -> Vec<String> {
    let Some(guidance) = guidance else {
        return Vec::new();
    };

    let mut seen_commands = BTreeSet::new();
    let mut steps = Vec::new();
    for action in &guidance.discovery_actions {
        if seen_commands.insert(action.command.clone()) {
            steps.push(format!("{}: {}", action.summary, action.command));
        }
    }
    steps
}

fn build_runtime_plugin_discovery_actions_for_conflict(
    conflict: &RuntimePluginShadowingConflictView,
) -> Vec<RuntimePluginDiscoveryActionView> {
    let command_name = crate::active_cli_command_name();
    let effective_package_root = package_root_from_source_path(&conflict.effective_source_path);
    let mut actions = vec![RuntimePluginDiscoveryActionView {
        kind: INSPECT_EFFECTIVE_PACKAGE_ACTION.to_owned(),
        plugin_id: conflict.plugin_id.clone(),
        target_source_path: conflict.effective_source_path.clone(),
        target_package_root: effective_package_root.clone(),
        summary: format!(
            "Inspect the effective project-local package for {}",
            conflict.plugin_id
        ),
        command: format!(
            "{} plugins doctor --root {} --profile sdk-release",
            command_name,
            crate::cli_handoff::shell_quote_argument(&effective_package_root)
        ),
    }];

    actions.extend(conflict.shadowed_source_paths.iter().map(|source_path| {
        let package_root = package_root_from_source_path(source_path);
        RuntimePluginDiscoveryActionView {
            kind: INSPECT_SHADOWED_PACKAGE_ACTION.to_owned(),
            plugin_id: conflict.plugin_id.clone(),
            target_source_path: source_path.clone(),
            target_package_root: package_root.clone(),
            summary: format!("Inspect the shadowed package for {}", conflict.plugin_id),
            command: format!(
                "{} plugins doctor --root {} --profile sdk-release",
                command_name,
                crate::cli_handoff::shell_quote_argument(&package_root)
            ),
        }
    }));

    actions
}

fn package_root_from_source_path(source_path: &str) -> String {
    Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(source_path))
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_runtime_plugin_discovery_guidance_only_applies_to_auto_discovered_roots() {
        assert_eq!(
            build_runtime_plugin_discovery_guidance(Some("configured"), Vec::new()),
            None
        );
        assert_eq!(
            build_runtime_plugin_discovery_guidance(Some("none"), Vec::new()),
            None
        );
    }

    #[test]
    fn build_runtime_plugin_discovery_guidance_reports_project_local_override() {
        let conflicts = vec![RuntimePluginShadowingConflictView {
            plugin_id: "shared-extension".to_owned(),
            effective_source_path: ".loong/extensions/search/loong.plugin.json".to_owned(),
            shadowed_source_paths: vec![
                "~/.loong/agent/extensions/search/loong.plugin.json".to_owned(),
            ],
        }];
        let guidance = build_runtime_plugin_discovery_guidance(Some("auto_discovered"), conflicts)
            .expect("auto-discovered roots should expose discovery guidance");

        assert_eq!(
            guidance.precedence_rule,
            PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE
        );
        assert_eq!(
            guidance.recommended_action.as_deref(),
            Some(REVIEW_GLOBAL_DUPLICATE_ACTION)
        );
        assert_eq!(guidance.shadowed_plugin_ids, vec!["shared-extension"]);
        assert_eq!(guidance.shadowed_conflicts.len(), 1);
        assert!(!guidance.discovery_actions.is_empty());
        assert_eq!(
            guidance.discovery_actions[0].kind,
            "inspect_effective_package"
        );
        assert!(
            guidance.discovery_actions[0]
                .command
                .contains("loong plugins doctor --root")
        );
        assert!(
            guidance
                .resolution_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("shadowed:"))
        );
    }

    #[test]
    fn build_runtime_plugin_discovery_next_steps_dedupes_repeated_commands() {
        let guidance = RuntimePluginDiscoveryGuidanceView {
            precedence_rule: PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE.to_owned(),
            project_local_root: PROJECT_LOCAL_LOONG_EXTENSION_ROOT.to_owned(),
            global_root: GLOBAL_LOONG_EXTENSION_ROOT.to_owned(),
            shadowed_plugin_ids: vec!["shared-extension".to_owned()],
            shadowed_conflicts: Vec::new(),
            discovery_actions: vec![
                RuntimePluginDiscoveryActionView {
                    kind: INSPECT_EFFECTIVE_PACKAGE_ACTION.to_owned(),
                    plugin_id: "shared-extension".to_owned(),
                    target_source_path: ".loong/extensions/search/loong.plugin.json".to_owned(),
                    target_package_root: ".loong/extensions/search".to_owned(),
                    summary: "Inspect the effective project-local package for shared-extension"
                        .to_owned(),
                    command: "loong plugins doctor --root '.loong/extensions/search' --profile sdk-release".to_owned(),
                },
                RuntimePluginDiscoveryActionView {
                    kind: INSPECT_EFFECTIVE_PACKAGE_ACTION.to_owned(),
                    plugin_id: "shared-extension".to_owned(),
                    target_source_path: ".loong/extensions/search/loong.plugin.json".to_owned(),
                    target_package_root: ".loong/extensions/search".to_owned(),
                    summary: "Inspect the effective project-local package for shared-extension"
                        .to_owned(),
                    command: "loong plugins doctor --root '.loong/extensions/search' --profile sdk-release".to_owned(),
                },
            ],
            recommended_action: Some(REVIEW_GLOBAL_DUPLICATE_ACTION.to_owned()),
            resolution_hint: None,
        };

        let steps = build_runtime_plugin_discovery_next_steps(Some(&guidance));

        assert_eq!(steps.len(), 1);
        assert!(steps[0].contains("Inspect the effective project-local package"));
    }

    #[test]
    fn build_runtime_plugin_shadowing_conflicts_groups_effective_and_shadowed_paths() {
        #[derive(Clone)]
        struct Candidate {
            plugin_id: &'static str,
            source_path: &'static str,
        }

        let conflicts = build_runtime_plugin_shadowing_conflicts(
            &[Candidate {
                plugin_id: "shared-extension",
                source_path: ".loong/extensions/search/loong.plugin.json",
            }],
            &BTreeMap::from([(
                "shared-extension".to_owned(),
                vec![Candidate {
                    plugin_id: "shared-extension",
                    source_path: "~/.loong/agent/extensions/search/loong.plugin.json",
                }],
            )]),
            |candidate| candidate.plugin_id,
            |candidate| candidate.source_path,
        );

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].plugin_id, "shared-extension");
        assert_eq!(
            conflicts[0].effective_source_path,
            ".loong/extensions/search/loong.plugin.json"
        );
        assert_eq!(
            conflicts[0].shadowed_source_paths,
            vec!["~/.loong/agent/extensions/search/loong.plugin.json"]
        );
    }
}
