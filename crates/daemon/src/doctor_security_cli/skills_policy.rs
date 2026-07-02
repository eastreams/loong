use loong_app as app;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectiveSkillsPolicyProbe {
    pub(crate) policy: app::tools::runtime_config::SkillsRuntimePolicy,
    pub(crate) override_active: bool,
}

pub(crate) fn resolve_effective_skills_policy(
    tool_runtime: &app::tools::runtime_config::ToolRuntimeConfig,
) -> Result<EffectiveSkillsPolicyProbe, String> {
    let (policy, override_active) =
        app::tools::effective_skills_policy_with_config(tool_runtime)
            .map_err(|error| format!("resolve effective skills policy failed: {error}"))?;
    Ok(EffectiveSkillsPolicyProbe {
        policy,
        override_active,
    })
}
