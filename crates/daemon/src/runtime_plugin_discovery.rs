use serde::{Deserialize, Serialize};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_hint: Option<String>,
}

pub fn build_runtime_plugin_discovery_guidance(
    roots_source: Option<&str>,
    shadowed_plugin_ids: &[String],
) -> Option<RuntimePluginDiscoveryGuidanceView> {
    if roots_source != Some("auto_discovered") {
        return None;
    }

    let shadowed_plugin_ids = shadowed_plugin_ids.to_vec();
    let has_shadowed_plugins = !shadowed_plugin_ids.is_empty();
    let resolution_hint = has_shadowed_plugins.then(|| {
        format!(
            "Project-local `{}` overrides `{}` for plugin ids: {}. Remove or rename the global duplicate if the override is accidental.",
            PROJECT_LOCAL_LOONG_EXTENSION_ROOT.trim_end_matches('/'),
            GLOBAL_LOONG_EXTENSION_ROOT.trim_end_matches('/'),
            shadowed_plugin_ids.join(", ")
        )
    });

    Some(RuntimePluginDiscoveryGuidanceView {
        precedence_rule: PROJECT_LOCAL_OVER_GLOBAL_PRECEDENCE_RULE.to_owned(),
        project_local_root: PROJECT_LOCAL_LOONG_EXTENSION_ROOT.to_owned(),
        global_root: GLOBAL_LOONG_EXTENSION_ROOT.to_owned(),
        shadowed_plugin_ids,
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
            build_runtime_plugin_discovery_guidance(Some("configured"), &[]),
            None
        );
        assert_eq!(
            build_runtime_plugin_discovery_guidance(Some("none"), &[]),
            None
        );
    }

    #[test]
    fn build_runtime_plugin_discovery_guidance_reports_project_local_override() {
        let guidance = build_runtime_plugin_discovery_guidance(
            Some("auto_discovered"),
            &[String::from("shared-extension")],
        )
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
        assert!(
            guidance
                .resolution_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("Project-local"))
        );
    }
}
