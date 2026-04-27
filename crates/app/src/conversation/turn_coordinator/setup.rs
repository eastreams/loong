use super::*;

pub(super) fn require_production_kernel_binding<'a>(
    binding: ConversationRuntimeBinding<'a>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ConversationRuntimeBinding<'a>> {
    if binding.is_kernel_bound() {
        return Ok(binding);
    }

    let failed_event = ConversationTurnPhaseEvent::failed();
    observe_turn_phase(observer, failed_event);

    let error = PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING.to_owned();
    Err(error)
}

pub(super) fn lane_policy_from_config(_config: &LoongConfig) -> LaneArbiterPolicy {
    LaneArbiterPolicy {
        ..LaneArbiterPolicy::default()
    }
}

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub(super) fn build_default_runtime_or_observe_failure(
        config: &LoongConfig,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<DefaultTurnRuntime> {
        let runtime_result = DefaultConversationRuntime::from_config_or_env(config);
        let runtime = match runtime_result {
            Ok(runtime) => runtime,
            Err(error) => {
                let failed_event = ConversationTurnPhaseEvent::failed();
                observe_turn_phase(observer, failed_event);
                return Err(error);
            }
        };
        Ok(runtime)
    }

    pub(super) fn build_default_runtime_with_binding<'a>(
        config: &LoongConfig,
        binding: ConversationRuntimeBinding<'a>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<(DefaultTurnRuntime, ConversationRuntimeBinding<'a>)> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, observer)?;
        let effective_binding = binding;

        Ok((runtime, effective_binding))
    }

    pub(super) fn build_default_runtime_with_production_binding<'a>(
        config: &LoongConfig,
        binding: ConversationRuntimeBinding<'a>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<(DefaultTurnRuntime, ConversationRuntimeBinding<'a>)> {
        let production_binding = require_production_kernel_binding(binding, observer)?;
        let runtime = Self::build_default_runtime_or_observe_failure(config, observer)?;

        Ok((runtime, production_binding))
    }
}
