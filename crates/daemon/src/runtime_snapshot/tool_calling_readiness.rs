use crate::mvp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotToolCallingState {
    pub availability: String,
    pub structured_tool_schema_enabled: bool,
    pub effective_tool_schema_mode: String,
    pub active_model: String,
    pub reason: String,
}

pub fn collect_runtime_snapshot_tool_calling_state(
    config: &mvp::config::LoongConfig,
    visible_tool_count: usize,
) -> RuntimeSnapshotToolCallingState {
    let provider_readiness = mvp::provider::provider_tool_schema_readiness(config);
    let no_visible_tools = visible_tool_count == 0;

    if no_visible_tools {
        return RuntimeSnapshotToolCallingState {
            availability: "inactive".to_owned(),
            structured_tool_schema_enabled: provider_readiness.structured_tool_schema_enabled,
            effective_tool_schema_mode: provider_readiness.effective_tool_schema_mode,
            active_model: provider_readiness.active_model,
            reason: "no runtime-visible tools are enabled".to_owned(),
        };
    }

    if provider_readiness.structured_tool_schema_enabled {
        return RuntimeSnapshotToolCallingState {
            availability: "ready".to_owned(),
            structured_tool_schema_enabled: true,
            effective_tool_schema_mode: provider_readiness.effective_tool_schema_mode,
            active_model: provider_readiness.active_model,
            reason: "provider turns include structured tool definitions for the active model"
                .to_owned(),
        };
    }

    RuntimeSnapshotToolCallingState {
        availability: "degraded".to_owned(),
        structured_tool_schema_enabled: false,
        effective_tool_schema_mode: provider_readiness.effective_tool_schema_mode,
        active_model: provider_readiness.active_model,
        reason:
            "provider turns omit structured tool definitions for the active model; tool use relies on prompt-visible guidance only"
                .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::collect_runtime_snapshot_tool_calling_state;
    use crate::mvp;

    #[test]
    fn tool_calling_state_is_inactive_when_no_tools_are_visible() {
        let config = mvp::config::LoongConfig::default();

        let state = collect_runtime_snapshot_tool_calling_state(&config, 0);

        assert_eq!(state.availability, "inactive");
        assert_eq!(state.reason, "no runtime-visible tools are enabled");
    }

    #[test]
    fn tool_calling_state_is_degraded_when_tool_schema_is_disabled() {
        let config = mvp::config::LoongConfig {
            provider: mvp::config::ProviderConfig {
                tool_schema_mode: mvp::config::ProviderToolSchemaModeConfig::Disabled,
                ..mvp::config::ProviderConfig::default()
            },
            ..mvp::config::LoongConfig::default()
        };

        let state = collect_runtime_snapshot_tool_calling_state(&config, 2);

        assert_eq!(state.availability, "degraded");
        assert!(!state.structured_tool_schema_enabled);
        assert_eq!(state.effective_tool_schema_mode, "disabled");
    }
}
