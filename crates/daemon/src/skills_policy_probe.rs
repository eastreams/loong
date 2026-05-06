use std::collections::BTreeSet;
use std::path::PathBuf;

use loong_app as mvp;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectiveSkillsPolicyProbe {
    pub(crate) policy: mvp::tools::runtime_config::SkillsRuntimePolicy,
    pub(crate) override_active: bool,
}

pub(crate) fn resolve_effective_skills_policy(
    tool_runtime: &mvp::tools::runtime_config::ToolRuntimeConfig,
) -> Result<EffectiveSkillsPolicyProbe, String> {
    let outcome = mvp::tools::skills_policy_get_with_config(tool_runtime)
        .map_err(|error| format!("resolve effective skills policy failed: {error}"))?;
    let payload = outcome.payload;
    let policy = skills_policy_from_payload(&payload)?;
    let override_active = payload
        .get("override_active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let probe = EffectiveSkillsPolicyProbe {
        policy,
        override_active,
    };
    Ok(probe)
}

fn skills_policy_from_payload(
    payload: &Value,
) -> Result<mvp::tools::runtime_config::SkillsRuntimePolicy, String> {
    let policy = payload
        .get("policy")
        .and_then(Value::as_object)
        .ok_or_else(|| "skills policy payload missing `policy` object".to_owned())?;

    let enabled = policy
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "skills policy payload missing boolean `enabled`".to_owned())?;

    let require_download_approval = policy
        .get("require_download_approval")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            "skills policy payload missing boolean `require_download_approval`".to_owned()
        })?;

    let allowed_domains = json_string_set(policy.get("allowed_domains"), "policy.allowed_domains")?;
    let blocked_domains = json_string_set(policy.get("blocked_domains"), "policy.blocked_domains")?;

    let install_root = policy
        .get("install_root")
        .and_then(Value::as_str)
        .map(PathBuf::from);

    let auto_expose_installed = policy
        .get("auto_expose_installed")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            "skills policy payload missing boolean `auto_expose_installed`".to_owned()
        })?;

    let resolved = mvp::tools::runtime_config::SkillsRuntimePolicy {
        enabled,
        require_download_approval,
        allowed_domains,
        blocked_domains,
        install_root,
        auto_expose_installed,
    };
    Ok(resolved)
}

fn json_string_set(value: Option<&Value>, field_name: &str) -> Result<BTreeSet<String>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("skills policy `{field_name}` must be an array"))?;

    let mut collected = BTreeSet::new();

    for raw_value in values {
        let string_value = raw_value
            .as_str()
            .ok_or_else(|| format!("skills policy `{field_name}` must contain only strings"))?;
        let owned_value = string_value.to_owned();
        collected.insert(owned_value);
    }

    Ok(collected)
}
