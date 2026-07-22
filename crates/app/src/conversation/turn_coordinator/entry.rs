use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    async fn handle_acp_entry_decision<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        ctx: &Context<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<Option<String>> {
        let acp_entry_decision =
            evaluate_acp_conversation_turn_entry_for_address(config, address, acp_options)?;
        match acp_entry_decision {
            AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled => {
                #[cfg(feature = "memory-sqlite")]
                persist_task_progress_event_best_effort(
                    config,
                    address.session_id.as_str(),
                    "turn_started",
                    active_task_progress_record(config, address.session_id.as_str(), user_input),
                );
                observe_turn_phase(observer, ConversationTurnPhaseEvent::preparing());
                let error = "ACP is disabled by policy (`acp.enabled=false`)".to_owned();
                let reply = match error_mode {
                    ProviderErrorMode::Propagate => return Err(error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&error);
                        persist_reply_turns_raw(runtime, user_input, &synthetic, ctx).await?;
                        synthetic
                    }
                };
                Ok(Some(reply))
            }
            AcpConversationTurnEntryDecision::RouteViaAcp => {
                #[cfg(feature = "memory-sqlite")]
                persist_task_progress_event_best_effort(
                    config,
                    address.session_id.as_str(),
                    "turn_started",
                    active_task_progress_record(config, address.session_id.as_str(), user_input),
                );
                observe_turn_phase(observer, ConversationTurnPhaseEvent::preparing());
                let reply = self
                    .handle_turn_via_acp_with_manager(
                        config,
                        address,
                        user_input,
                        error_mode,
                        runtime,
                        acp_options,
                        ctx,
                        acp_manager,
                    )
                    .await?;
                Ok(Some(reply))
            }
            AcpConversationTurnEntryDecision::StayOnProvider => Ok(None),
        }
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_ingress_and_observer(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            acp_options,
            legacy_tools,
            ingress,
            observer,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        let runtime = match DefaultConversationRuntime::from_config_or_env(config) {
            Ok(runtime) => runtime,
            Err(error) => {
                observe_turn_phase(observer.as_ref(), ConversationTurnPhaseEvent::failed());
                return Err(error);
            }
        };
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            legacy_tools,
            ingress,
            observer,
            retry_progress,
            acp_manager,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_observer(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            acp_options,
            legacy_tools,
            None,
            observer,
        )
        .await
    }

    pub async fn handle_production_turn_with_address_and_acp_options_and_observer(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_production_turn_with_address_and_acp_options_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            acp_options,
            legacy_tools,
            observer,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_production_turn_with_address_and_acp_options_and_observer_with_manager(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            acp_options,
            legacy_tools,
            None,
            observer,
            retry_progress,
            acp_manager,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        legacy_tools: &DefaultLegacyToolDispatcher,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id(ctx.session().session_id());
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            &address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            legacy_tools,
            None,
            None,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            legacy_tools,
            None,
            None,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            legacy_tools,
            ingress,
            observer,
            retry_progress,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        // Address carries channel/thread routing data, but its canonical
        // Session identity must be the one already owned by Context.
        if address.session_id != ctx.session().session_id() {
            return Err(format!(
                "conversation address session `{}` does not match Context Session `{}`",
                address.session_id,
                ctx.session().session_id()
            ));
        }
        if let Some(reply) = self
            .handle_acp_entry_decision(
                config,
                address,
                user_input,
                error_mode,
                runtime,
                acp_options,
                ctx,
                observer.as_ref(),
                acp_manager,
            )
            .await?
        {
            observe_non_provider_turn_terminal_success_phases(observer.as_ref());
            return Ok(reply);
        }

        self.handle_turn_with_runtime_and_address_and_ingress_and_observer_outcome(
            config,
            ctx,
            user_input,
            error_mode,
            runtime,
            legacy_tools,
            ingress,
            observer,
            retry_progress,
        )
        .await
        .map(|outcome| outcome.reply)
    }

    pub async fn handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            legacy_tools,
            ingress,
            observer,
            None,
            None,
        )
        .await
    }

    pub async fn handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            ctx,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            legacy_tools,
            ingress,
            observer,
            retry_progress,
            acp_manager,
        )
        .await
    }
}
