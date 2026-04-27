use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_observer_uses_streaming_request_and_emits_live_events() {
    let mut config = LoongConfig::default();
    config.provider.kind = crate::config::ProviderKind::Anthropic;

    let runtime = ObserverStreamingRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await
        .expect("observer turn should succeed");

    assert_eq!(reply, "final reply");

    let streaming_calls = runtime
        .streaming_calls
        .lock()
        .expect("streaming call lock should not be poisoned");
    assert_eq!(*streaming_calls, 1);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RequestingProvider,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert_eq!(token_events.len(), 1);
    assert_eq!(token_events[0].event_type, "text_delta");
    assert_eq!(token_events[0].delta.text.as_deref(), Some("draft"));
}

#[tokio::test]
async fn handle_turn_with_observer_falls_back_when_streaming_events_are_unsupported() {
    let mut config = LoongConfig::default();
    config.provider.kind = crate::config::ProviderKind::Openai;
    config.provider.wire_api = crate::config::ProviderWireApi::Responses;

    let runtime = ObserverFallbackRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await
        .expect("observer turn should succeed");

    assert_eq!(reply, "final reply");

    let request_turn_calls = runtime
        .request_turn_calls
        .lock()
        .expect("request-turn call lock should not be poisoned");
    assert_eq!(*request_turn_calls, 1);

    let request_turn_streaming_calls = runtime
        .request_turn_streaming_calls
        .lock()
        .expect("request-turn-streaming call lock should not be poisoned");
    assert_eq!(*request_turn_streaming_calls, 0);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RequestingProvider,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert!(
        token_events.is_empty(),
        "unsupported transports should not emit streaming token events: {token_events:#?}"
    );
}

#[tokio::test]
async fn handle_turn_with_observer_emits_lifecycle_for_explicit_acp_inline_message() {
    let config = LoongConfig::default();
    let runtime = ObserverStreamingRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::explicit();
    let address = ConversationSessionAddress::from_session_id("observer-session");
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await
        .expect("ACP inline reply should succeed");

    let expected_reply =
        format_provider_error_reply("ACP is disabled by policy (`acp.enabled=false`)");
    assert_eq!(reply, expected_reply);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );

    let tool_events = observer
        .tool_events
        .lock()
        .expect("tool event lock should not be poisoned");
    assert!(tool_events.is_empty());

    let token_events = observer
        .token_events
        .lock()
        .expect("token event lock should not be poisoned");
    assert!(token_events.is_empty());

    let streaming_calls = runtime
        .streaming_calls
        .lock()
        .expect("streaming call lock should not be poisoned");
    assert_eq!(*streaming_calls, 0);
}

#[tokio::test]
async fn handle_turn_with_ingress_and_observer_marks_failed_when_runtime_bootstrap_fails() {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-observer-runtime-ingress".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await;
    let _error = result.expect_err("missing runtime bootstrap should fail");

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[tokio::test]
async fn handle_turn_with_observer_marks_failed_when_runtime_bootstrap_fails() {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-observer-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_turn_with_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await;
    let _error = result.expect_err("missing runtime bootstrap should fail");

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();
    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[tokio::test]
async fn handle_production_turn_with_observer_rejects_direct_binding_before_runtime_bootstrap() {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-observer-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_production_turn_with_address_and_acp_options_and_observer_with_manager(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            Some(observer_handle),
            None,
            None,
        )
        .await;
    let error = result.expect_err("direct production binding should fail");

    assert_eq!(
        error,
        PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
    );

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();

    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[tokio::test]
async fn handle_production_turn_with_runtime_rejects_direct_binding_before_provider_request() {
    let mut config = LoongConfig::default();
    config.provider.kind = crate::config::ProviderKind::Anthropic;

    let runtime = ObserverStreamingRuntime::default();
    let coordinator = ConversationTurnCoordinator::new();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("observer-session");

    let result = coordinator
        .handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "say hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            Some(observer_handle),
        )
        .await;
    let error = result.expect_err("direct production binding should fail");

    assert_eq!(
        error,
        PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
    );

    let streaming_calls = runtime
        .streaming_calls
        .lock()
        .expect("streaming call lock should not be poisoned");

    assert_eq!(*streaming_calls, 0);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();

    assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
}

#[tokio::test]
async fn compact_production_session_rejects_direct_binding_before_runtime_bootstrap() {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let result = coordinator
        .compact_production_session(
            &config,
            "maintenance-session",
            ConversationRuntimeBinding::direct(),
        )
        .await;
    let error = result.expect_err("direct production maintenance binding should fail");

    assert_eq!(
        error,
        PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
    );
}

#[tokio::test]
async fn repair_production_turn_checkpoint_tail_rejects_direct_binding_before_runtime_bootstrap() {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let result = coordinator
        .repair_production_turn_checkpoint_tail(
            &config,
            "maintenance-session",
            ConversationRuntimeBinding::direct(),
        )
        .await;
    let error = result.expect_err("direct production maintenance binding should fail");

    assert_eq!(
        error,
        PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
    );
}

#[tokio::test]
async fn load_production_turn_checkpoint_diagnostics_rejects_direct_binding_before_runtime_bootstrap()
 {
    let mut config = LoongConfig::default();
    config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

    let coordinator = ConversationTurnCoordinator::new();
    let limit = config.memory.sliding_window;
    let result = coordinator
        .load_production_turn_checkpoint_diagnostics_with_limit(
            &config,
            "maintenance-session",
            limit,
            ConversationRuntimeBinding::direct(),
        )
        .await;
    let error = result.expect_err("direct production maintenance binding should fail");

    assert_eq!(
        error,
        PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
    );
}

#[test]
fn generic_direct_capable_coordinator_entrypoints_are_not_public_api() {
    let source = include_str!("../../turn_coordinator.rs");
    let function_names = [
        "compact_session_with_runtime",
        "handle_turn_with_ingress",
        "handle_turn_with_acp_options",
        "probe_turn_checkpoint_tail_runtime_gate",
        "probe_turn_checkpoint_tail_runtime_gate_with_limit",
        "handle_turn_with_acp_event_sink",
        "handle_turn_with_address",
        "handle_turn_with_address_and_acp_event_sink",
        "handle_turn_with_address_and_acp_options_and_ingress",
        "handle_turn_with_address_and_acp_options",
        "handle_turn_with_address_and_acp_options_and_ingress_and_observer",
        "handle_turn_with_address_and_acp_options_and_observer",
        "handle_turn_with_runtime",
        "handle_turn_with_runtime_and_ingress",
        "repair_turn_checkpoint_tail_with_runtime",
        "probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit",
        "handle_turn_with_runtime_and_acp_options",
        "handle_turn_with_runtime_and_acp_event_sink",
        "handle_turn_with_runtime_and_address",
        "handle_turn_with_runtime_and_address_and_acp_options",
        "handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer",
        "handle_turn_with_runtime_and_address_and_acp_event_sink",
    ];
    let signature_pattern = Regex::new(
        r"(?m)^(?:(?:\s*///[^\n]*\n)|(?:\s*#\[[^\n]+\]\s*\n))*\s*(?P<visibility>pub(?:\([^)]*\))?\s+)?async\s+fn\s+(?P<name>[A-Za-z0-9_]+)\b",
    )
    .expect("signature regex should compile");
    let mut public_function_names = Vec::new();

    for captures in signature_pattern.captures_iter(source) {
        let Some(name_match) = captures.name("name") else {
            continue;
        };
        let visibility = captures
            .name("visibility")
            .map(|match_value| match_value.as_str().trim())
            .unwrap_or("");
        if visibility != "pub" {
            continue;
        }

        let function_name = name_match.as_str().to_owned();
        public_function_names.push(function_name);
    }

    for function_name in function_names {
        let is_public = public_function_names
            .iter()
            .any(|public_name| public_name == function_name);
        assert!(
            !is_public,
            "generic direct-capable coordinator seam should stay internal: {function_name}"
        );
    }
}
