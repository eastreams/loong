use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const PROJECT_LOCAL_LOONG_EXTENSION_ROOT: &str = ".loong/extensions/";
pub(crate) const GLOBAL_LOONG_EXTENSION_ROOT: &str = "~/.loong/agent/extensions/";
pub(crate) const PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE: &str = "project_local_over_global";
pub(crate) const REVIEW_GLOBAL_DUPLICATE_ACTION: &str = "review_global_duplicate";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePluginDiscoveryGuidanceView {
    pub precedence_rule: String,
    pub project_local_root: String,
    pub global_root: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_plugin_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_conflicts: Vec<RuntimePluginShadowingConflictView>,
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
        recommended_action: has_shadowed_plugins.then(|| REVIEW_GLOBAL_DUPLICATE_ACTION.to_owned()),
        resolution_hint,
    })
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
        assert!(
            guidance
                .resolution_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("shadowed:"))
        );
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
