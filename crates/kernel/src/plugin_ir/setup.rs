use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::plugin::PluginSetup;

/// Declares which setup requirements are already verified for a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginSetupReadinessContext {
    pub verified_env_vars: BTreeSet<String>,
    pub verified_config_keys: BTreeSet<String>,
}

/// Summarizes whether manifest-declared setup requirements are satisfied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupReadiness {
    pub ready: bool,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
}

impl Default for PluginSetupReadiness {
    fn default() -> Self {
        Self {
            ready: true,
            missing_required_env_vars: Vec::new(),
            missing_required_config_keys: Vec::new(),
        }
    }
}

/// Evaluates manifest-declared setup requirements against verified runtime context.
pub fn evaluate_plugin_setup_requirements(
    required_env_vars: &[String],
    required_config_keys: &[String],
    context: &PluginSetupReadinessContext,
) -> PluginSetupReadiness {
    let mut missing_required_env_vars = Vec::new();
    for required_env_var in required_env_vars {
        let env_var_is_verified =
            verified_env_var_names_contain(&context.verified_env_vars, required_env_var);
        if !env_var_is_verified {
            missing_required_env_vars.push(required_env_var.clone());
        }
    }

    let mut missing_required_config_keys = Vec::new();
    for required_config_key in required_config_keys {
        let config_key_is_verified = context.verified_config_keys.contains(required_config_key);
        if !config_key_is_verified {
            missing_required_config_keys.push(required_config_key.clone());
        }
    }

    let env_ready = missing_required_env_vars.is_empty();
    let config_ready = missing_required_config_keys.is_empty();
    let ready = env_ready && config_ready;

    PluginSetupReadiness {
        ready,
        missing_required_env_vars,
        missing_required_config_keys,
    }
}

pub(super) fn evaluate_plugin_setup_readiness(
    setup: Option<&PluginSetup>,
    context: &PluginSetupReadinessContext,
) -> PluginSetupReadiness {
    let Some(setup) = setup else {
        return PluginSetupReadiness::default();
    };

    evaluate_plugin_setup_requirements(
        &setup.required_env_vars,
        &setup.required_config_keys,
        context,
    )
}

pub(super) fn format_plugin_setup_incomplete_reason(readiness: &PluginSetupReadiness) -> String {
    let mut reasons = Vec::new();

    if !readiness.missing_required_env_vars.is_empty() {
        let missing_env_vars = readiness.missing_required_env_vars.join(", ");
        let env_reason = format!("missing required env vars: {missing_env_vars}");
        reasons.push(env_reason);
    }

    if !readiness.missing_required_config_keys.is_empty() {
        let missing_config_keys = readiness.missing_required_config_keys.join(", ");
        let config_reason = format!("missing required config keys: {missing_config_keys}");
        reasons.push(config_reason);
    }

    let combined_reasons = reasons.join("; ");
    format!("plugin setup is incomplete: {combined_reasons}")
}

fn verified_env_var_names_contain(
    verified_env_vars: &BTreeSet<String>,
    required_env_var: &str,
) -> bool {
    #[cfg(windows)]
    {
        verified_env_vars
            .iter()
            .any(|verified_env_var| verified_env_var.eq_ignore_ascii_case(required_env_var))
    }

    #[cfg(not(windows))]
    {
        verified_env_vars.contains(required_env_var)
    }
}
