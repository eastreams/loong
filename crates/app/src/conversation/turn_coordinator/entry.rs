use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    async fn handle_turn_with_session_and_acp_options_and_ingress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        let prepared = Self::build_default_runtime_with_binding(config, binding, None)?;
        let runtime = prepared.0;
        let effective_binding = prepared.1;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            &address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            effective_binding,
            ingress,
            None,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_ingress_and_observer(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            binding,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        let prepared =
            Self::build_default_runtime_with_binding(config, binding, observer.as_ref())?;
        let runtime = prepared.0;
        let effective_binding = prepared.1;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            effective_binding,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            binding,
            None,
            observer,
        )
        .await
    }

    pub async fn handle_production_turn_with_address_and_acp_options_and_observer(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_production_turn_with_address_and_acp_options_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            binding,
            observer,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn handle_production_turn_with_address_and_acp_options_and_observer_with_manager(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            require_production_kernel_binding(binding, observer.as_ref())?,
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
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_runtime_and_session_and_acp_options_and_ingress(
            config,
            session_id,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
            None,
        )
        .await
    }

    async fn handle_turn_with_runtime_and_session_and_acp_options_and_ingress<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            &address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            ingress,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_outcome(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            ingress,
            observer,
            retry_progress,
            acp_manager,
        )
        .await
        .map(|outcome| outcome.reply)
    }

    pub async fn handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
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
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
        acp_manager: Option<Arc<crate::acp::AcpSessionManager>>,
    ) -> CliResult<String> {
        let production_binding = require_production_kernel_binding(binding, observer.as_ref())?;

        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            production_binding,
            ingress,
            observer,
            retry_progress,
            acp_manager,
        )
        .await
    }
}
