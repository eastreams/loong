use super::*;
use futures_util::StreamExt;
use loong_contracts::SecretRef;

fn build_control_plane_router(manager: Arc<mvp::control_plane::ControlPlaneManager>) -> Router {
    super::build_control_plane_router(manager).expect("router")
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Option<Arc<mvp::control_plane::ControlPlaneRepositoryView>>,
    acp_view: Option<Arc<mvp::control_plane::ControlPlaneAcpView>>,
) -> Router {
    super::serve::build_control_plane_router_with_views(manager, repository_view, acp_view)
        .expect("router")
}

fn build_control_plane_router_with_turn_runtime(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    turn_runtime: Arc<ControlPlaneTurnRuntime>,
) -> Router {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    super::serve::build_control_plane_router_with_runtime(
        manager,
        None,
        None,
        Some(turn_runtime),
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

#[cfg(feature = "memory-sqlite")]
fn build_control_plane_router_with_turn_runtime_and_views(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    repository_view: Arc<mvp::control_plane::ControlPlaneRepositoryView>,
    acp_view: Arc<mvp::control_plane::ControlPlaneAcpView>,
    turn_runtime: Arc<ControlPlaneTurnRuntime>,
) -> Router {
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    let exposure_policy = default_loopback_exposure_policy();
    super::serve::build_control_plane_router_with_runtime(
        manager,
        Some(repository_view),
        Some(acp_view),
        Some(turn_runtime),
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

#[derive(Default)]
struct TestTurnBackendState {
    sink_calls: std::sync::atomic::AtomicUsize,
}

struct TestTurnBackend {
    id: &'static str,
    state: Arc<TestTurnBackendState>,
}

impl mvp::acp::AcpRuntimeBackend for TestTurnBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    fn metadata(&self) -> mvp::acp::AcpBackendMetadata {
        mvp::acp::AcpBackendMetadata::new(
            self.id(),
            [
                mvp::acp::AcpCapability::SessionLifecycle,
                mvp::acp::AcpCapability::TurnExecution,
                mvp::acp::AcpCapability::TurnEventStreaming,
            ],
            "Control-plane turn backend for daemon tests",
        )
    }

    fn ensure_session<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        request: &'life2 mvp::acp::AcpSessionBootstrap,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpSessionHandle>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            Ok(mvp::acp::AcpSessionHandle {
                session_key: request.session_key.clone(),
                backend_id: self.id().to_owned(),
                runtime_session_name: format!("test-runtime-{}", request.session_key),
                working_directory: request.working_directory.clone(),
                backend_session_id: Some(format!("backend-{}", request.session_key)),
                agent_session_id: Some(format!("agent-{}", request.session_key)),
                binding: request.binding.clone(),
            })
        })
    }

    fn run_turn<'life0, 'life1, 'life2, 'life3, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
        request: &'life3 mvp::acp::AcpTurnRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        'life3: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            Ok(mvp::acp::AcpTurnResult {
                output_text: format!("echo: {}", request.input),
                state: mvp::acp::AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
            })
        })
    }

    fn run_turn_with_sink<'life0, 'life1, 'life2, 'life3, 'life5, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
        request: &'life3 mvp::acp::AcpTurnRequest,
        _abort: Option<mvp::acp::AcpAbortSignal>,
        sink: Option<&'life5 dyn mvp::acp::AcpTurnEventSink>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = CliResult<mvp::acp::AcpTurnResult>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        'life3: 'async_trait,
        'life5: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            if let Some(sink) = sink {
                sink.on_event(&serde_json::json!({
                    "type": "text",
                    "content": format!("chunk:{}", request.input),
                }))?;
                sink.on_event(&serde_json::json!({
                    "type": "done",
                    "stopReason": "completed",
                }))?;
            }
            self.state
                .sink_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(mvp::acp::AcpTurnResult {
                output_text: format!("streamed: {}", request.input),
                state: mvp::acp::AcpSessionState::Ready,
                usage: Some(serde_json::json!({
                    "total_tokens": 7
                })),
                events: Vec::new(),
                stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
            })
        })
    }

    fn cancel<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { Ok(()) })
    }

    fn close<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        _config: &'life1 mvp::config::LoongConfig,
        _session: &'life2 mvp::acp::AcpSessionHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CliResult<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move { Ok(()) })
    }
}

fn turn_runtime_test_config(backend_id: &str) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.audit.mode = mvp::config::AuditMode::InMemory;
    config
}

fn seeded_turn_runtime(
    backend_id: &'static str,
    state: Arc<TestTurnBackendState>,
) -> Arc<ControlPlaneTurnRuntime> {
    mvp::acp::register_acp_backend(backend_id, {
        move || {
            Box::new(TestTurnBackend {
                id: backend_id,
                state: state.clone(),
            })
        }
    })
    .expect("register control-plane turn backend");
    let config = turn_runtime_test_config(backend_id);
    let temp_root = std::env::temp_dir().join(format!(
        "loong-control-plane-turn-runtime-{}-{}",
        backend_id,
        current_time_ms()
    ));
    std::fs::create_dir_all(&temp_root).expect("create control-plane turn runtime temp root");
    let resolved_path = temp_root.join("config.toml");
    mvp::config::write(
        Some(resolved_path.to_str().expect("utf8 config path")),
        &config,
        true,
    )
    .expect("write control-plane turn runtime config");
    let acp_manager = Arc::new(mvp::acp::AcpSessionManager::default());
    Arc::new(ControlPlaneTurnRuntime::with_manager(
        resolved_path,
        config,
        acp_manager,
    ))
}

fn remote_control_plane_config(shared_token: &str) -> mvp::config::LoongConfig {
    let mut config = mvp::config::LoongConfig::default();
    config.control_plane.allow_remote = true;
    config.control_plane.shared_token = Some(SecretRef::Inline(shared_token.to_owned()));
    config
}

fn non_loopback_bind_addr() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 4317))
}

fn build_remote_control_plane_router(
    manager: Arc<mvp::control_plane::ControlPlaneManager>,
    shared_token: &str,
) -> Router {
    let config = remote_control_plane_config(shared_token);
    let bind_addr = non_loopback_bind_addr();
    let exposure_policy =
        build_control_plane_exposure_policy(bind_addr, Some(&config)).expect("policy");
    let pairing_registry = Arc::new(mvp::control_plane::ControlPlanePairingRegistry::new());
    super::serve::build_control_plane_router_with_runtime(
        manager,
        None,
        None,
        None,
        pairing_registry,
        exposure_policy,
    )
    .expect("router")
}

async fn connect_token(
    router: &Router,
    scopes: std::collections::BTreeSet<ControlPlaneScope>,
) -> String {
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: None,
    };

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let connect: ControlPlaneConnectResponse = serde_json::from_slice(&body).expect("connect json");
    connect.connection_token
}

fn bearer_request(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method(method)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request")
}

async fn issue_challenge(router: &Router) -> ControlPlaneChallengeResponse {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/challenge")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("challenge response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&body).expect("challenge json")
}

fn signed_device_for_request(
    client_id: &str,
    role: ControlPlaneRole,
    scopes: std::collections::BTreeSet<ControlPlaneScope>,
    challenge: &ControlPlaneChallengeResponse,
) -> loong_protocol::ControlPlaneDeviceIdentity {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let device_template = loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-1".to_owned(),
        public_key: String::new(),
        signature: String::new(),
        signed_at_ms: challenge.issued_at_ms,
        nonce: challenge.nonce.clone(),
    };
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: client_id.to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device_template.clone()),
    };
    let message = control_plane_device_signature_message(&request, &device_template);
    let signature = signing_key.sign(&message);
    loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-1".to_owned(),
        public_key: base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes()),
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
        signed_at_ms: challenge.issued_at_ms,
        nonce: challenge.nonce.clone(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn isolated_memory_config(test_name: &str) -> mvp::session::store::SessionStoreConfig {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ISOLATED_MEMORY_CONFIG_ID: AtomicU64 = AtomicU64::new(1);
    let nonce = NEXT_ISOLATED_MEMORY_CONFIG_ID.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "loong-control-plane-server-{test_name}-{}-{nonce}",
        std::process::id(),
    ));
    let _ = std::fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(base.join("memory.sqlite3-wal"));
    let _ = std::fs::remove_file(base.join("memory.sqlite3-shm"));
    mvp::session::store::SessionStoreConfig {
        sqlite_path: Some(db_path),
        runtime_config: None,
    }
}

#[cfg(feature = "memory-sqlite")]
fn seeded_repository_view(test_name: &str) -> Arc<mvp::control_plane::ControlPlaneRepositoryView> {
    let config = isolated_memory_config(test_name);
    let repo = mvp::session::repository::SessionRepository::new(&config).expect("repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: serde_json::json!({
            "task": "research control plane parity",
            "label": "Child",
            "execution": {
                "mode": "async",
                "depth": 1,
                "max_depth": 3,
                "active_children": 0,
                "max_active_children": 2,
                "timeout_seconds": 90,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["file.read"],
                "workspace_root": "/tmp/loong/control-plane/child-session",
                "kernel_bound": false,
                "runtime_narrowing": {}
            }
        }),
    })
    .expect("append child event");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-visible".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-visible".to_owned(),
        tool_call_id: "call-visible".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-visible",
        }),
    })
    .expect("create visible approval");
    repo.upsert_session_tool_policy(mvp::session::repository::NewSessionToolPolicyRecord {
        session_id: "child-session".to_owned(),
        requested_tool_ids: vec!["file.read".to_owned()],
        runtime_narrowing: mvp::tools::runtime_config::ToolRuntimeNarrowing::default(),
    })
    .expect("create visible tool policy");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create hidden root");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-hidden".to_owned(),
        session_id: "hidden-root".to_owned(),
        turn_id: "turn-hidden".to_owned(),
        tool_call_id: "call-hidden".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate_async",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-hidden",
        }),
    })
    .expect("create hidden approval");

    Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
        config,
        mvp::config::ToolConfig::default(),
        "root-session",
    ))
}

#[cfg(feature = "memory-sqlite")]
fn seeded_control_plane_views(
    test_name: &str,
) -> (
    Arc<mvp::control_plane::ControlPlaneRepositoryView>,
    Arc<mvp::control_plane::ControlPlaneAcpView>,
) {
    let memory_config = isolated_memory_config(test_name);
    let repo =
        mvp::session::repository::SessionRepository::new(&memory_config).expect("repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: serde_json::json!({
            "status": "started",
        }),
    })
    .expect("append child event");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-visible".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-visible".to_owned(),
        tool_call_id: "call-visible".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-visible",
        }),
    })
    .expect("create visible approval");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create hidden root");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-hidden".to_owned(),
        session_id: "hidden-root".to_owned(),
        turn_id: "turn-hidden".to_owned(),
        tool_call_id: "call-hidden".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: serde_json::json!({
            "tool": "delegate_async",
        }),
        governance_snapshot_json: serde_json::json!({
            "reason": "governed_tool_requires_approval",
            "rule_id": "approval-hidden",
        }),
    })
    .expect("create hidden approval");

    let mut config = mvp::config::LoongConfig::default();
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    config.acp.enabled = true;

    let store = mvp::acp::AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
    mvp::acp::AcpSessionStore::upsert(
        &store,
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:child-session".to_owned(),
            conversation_id: Some("conversation-visible".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "child-session".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc-visible".to_owned()),
                participant_id: None,
                thread_id: Some("thread-visible".to_owned()),
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::ExplicitRequest),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-visible".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-visible".to_owned()),
            agent_session_id: Some("agent-visible".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Ready,
            last_activity_ms: 100,
            last_error: None,
        },
    )
    .expect("seed visible ACP session");
    mvp::acp::AcpSessionStore::upsert(
        &store,
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:hidden-root".to_owned(),
            conversation_id: Some("conversation-hidden".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "hidden-root".to_owned(),
                channel_id: Some("telegram".to_owned()),
                account_id: None,
                conversation_id: Some("hidden".to_owned()),
                participant_id: None,
                thread_id: None,
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-hidden".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-hidden".to_owned()),
            agent_session_id: Some("agent-hidden".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Review),
            state: mvp::acp::AcpSessionState::Busy,
            last_activity_ms: 200,
            last_error: Some("hidden".to_owned()),
        },
    )
    .expect("seed hidden ACP session");

    (
        Arc::new(mvp::control_plane::ControlPlaneRepositoryView::new(
            memory_config,
            mvp::config::ToolConfig::default(),
            "root-session",
        )),
        Arc::new(mvp::control_plane::ControlPlaneAcpView::new(
            config,
            "root-session",
        )),
    )
}

#[test]
fn default_bind_addr_is_loopback() {
    let addr = default_control_plane_bind_addr(0);
    assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
    assert_eq!(addr.port(), 0);
}

#[test]
fn resolve_control_plane_bind_addr_accepts_explicit_override() {
    let bind_addr = resolve_control_plane_bind_addr(Some("0.0.0.0:4317"), 0).expect("bind addr");
    assert_eq!(bind_addr, non_loopback_bind_addr());
}

#[test]
fn non_loopback_exposure_requires_explicit_remote_opt_in() {
    let config = mvp::config::LoongConfig::default();
    let error = build_control_plane_exposure_policy(non_loopback_bind_addr(), Some(&config))
        .expect_err("remote bind should require explicit opt-in");
    assert!(error.contains("control_plane.allow_remote=true"));
}

#[test]
fn non_loopback_exposure_requires_shared_token() {
    let mut config = mvp::config::LoongConfig::default();
    config.control_plane.allow_remote = true;
    let error = build_control_plane_exposure_policy(non_loopback_bind_addr(), Some(&config))
        .expect_err("remote bind should require shared token");
    assert!(error.contains("control_plane.shared_token"));
}

#[tokio::test]
async fn readyz_returns_ok() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("readyz response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn control_challenge_returns_nonce_payload() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let challenge = issue_challenge(&router).await;
    assert!(challenge.nonce.starts_with("cpc-"));
    assert!(challenge.expires_at_ms >= challenge.issued_at_ms);
}

#[tokio::test]
async fn healthz_returns_snapshot_json() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    manager.set_presence_count(2);
    manager.set_session_count(3);
    manager.set_pending_approval_count(1);
    manager.set_acp_session_count(4);
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("healthz response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let snapshot: ControlPlaneSnapshotResponse =
        serde_json::from_slice(&body).expect("snapshot json");
    assert!(snapshot.snapshot.runtime_ready);
    assert_eq!(snapshot.snapshot.presence_count, 2);
    assert_eq!(snapshot.snapshot.session_count, 3);
    assert_eq!(snapshot.snapshot.pending_approval_count, 1);
    assert_eq!(snapshot.snapshot.acp_session_count, 4);
}

#[tokio::test]
async fn control_connect_returns_protocol_response() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let connect: ControlPlaneConnectResponse = serde_json::from_slice(&body).expect("connect json");
    assert_eq!(connect.protocol, CONTROL_PLANE_PROTOCOL_VERSION);
    assert_eq!(connect.principal.client_id, "cli");
    assert_eq!(connect.principal.role, ControlPlaneRole::Operator);
    assert!(connect.connection_token.starts_with("cpt-"));
    assert!(connect.connection_token_expires_at_ms > 0);
    assert!(connect.snapshot.runtime_ready);
    assert_eq!(
        connect.policy.tick_interval_ms,
        CONTROL_PLANE_TICK_INTERVAL_MS
    );
}

#[tokio::test]
async fn remote_control_connect_requires_shared_token_for_non_device_operator() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_remote_control_plane_router(manager, "bootstrap-token");
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&body).expect("error json");
    assert_eq!(
        error.code,
        ControlPlaneConnectErrorCode::SharedTokenRequired
    );
}

#[tokio::test]
async fn remote_control_connect_rejects_invalid_shared_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_remote_control_plane_router(manager, "bootstrap-token");
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loong_protocol::ControlPlaneAuthClaims {
            token: Some("wrong-token".to_owned()),
            device_token: None,
            bootstrap_token: None,
            password: None,
        }),
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&body).expect("error json");
    assert_eq!(error.code, ControlPlaneConnectErrorCode::SharedTokenInvalid);
}

#[tokio::test]
async fn remote_control_connect_accepts_valid_shared_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_remote_control_plane_router(manager, "bootstrap-token");
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loong_protocol::ControlPlaneAuthClaims {
            token: Some("bootstrap-token".to_owned()),
            device_token: None,
            bootstrap_token: None,
            password: None,
        }),
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let connect: ControlPlaneConnectResponse = serde_json::from_slice(&body).expect("connect json");
    assert_eq!(connect.principal.client_id, "cli");
}

#[tokio::test]
async fn remote_control_connect_clamps_bootstrap_scopes_to_safe_subset() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_remote_control_plane_router(manager, "bootstrap-token");
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::from([
            ControlPlaneScope::OperatorRead,
            ControlPlaneScope::OperatorAdmin,
            ControlPlaneScope::OperatorPairing,
        ]),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loong_protocol::ControlPlaneAuthClaims {
            token: Some("bootstrap-token".to_owned()),
            device_token: None,
            bootstrap_token: None,
            password: None,
        }),
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let connect: ControlPlaneConnectResponse = serde_json::from_slice(&body).expect("connect json");
    assert!(
        connect
            .principal
            .scopes
            .contains(&ControlPlaneScope::OperatorRead)
    );
    assert!(
        connect
            .principal
            .scopes
            .contains(&ControlPlaneScope::OperatorPairing)
    );
    assert!(
        !connect
            .principal
            .scopes
            .contains(&ControlPlaneScope::OperatorAdmin)
    );
}

#[tokio::test]
async fn control_connect_rejects_protocol_mismatch() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION + 1,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION + 1,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: None,
        },
        role: ControlPlaneRole::Operator,
        scopes: std::collections::BTreeSet::new(),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: None,
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn control_connect_requires_pairing_before_signed_device_can_connect() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);
    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("connect response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&body).expect("connect error json");
    assert_eq!(error.code, ControlPlaneConnectErrorCode::PairingRequired);
    assert!(error.pairing_request_id.is_some());
}

#[tokio::test]
async fn control_connect_rejects_reused_device_challenge() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);
    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let first = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("first connect response");
    assert_eq!(first.status(), StatusCode::FORBIDDEN);

    let second = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("second connect response");
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pairing_resolve_approves_device_and_connect_accepts_device_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);

    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let pairing_request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: scopes.clone(),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device.clone()),
    };

    let pairing_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&pairing_request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("pairing response");
    assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
    let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let pairing_error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&pairing_body).expect("pairing error json");
    let pairing_request_id = pairing_error
        .pairing_request_id
        .expect("pairing request id");

    let operator_token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;
    let resolve_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/pairing/resolve")
                .method("POST")
                .header("authorization", format!("Bearer {operator_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id,
                        approve: true,
                    })
                    .expect("encode resolve request"),
                ))
                .expect("request"),
        )
        .await
        .expect("resolve response");
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolve: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&resolve_body).expect("resolve json");
    let device_token = resolve.device_token.expect("device token");

    let reconnect_challenge = issue_challenge(&router).await;
    let reconnect_device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &reconnect_challenge,
    );
    let reconnect_request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loong_protocol::ControlPlaneAuthClaims {
            token: None,
            device_token: Some(device_token),
            bootstrap_token: None,
            password: None,
        }),
        device: Some(reconnect_device),
    };

    let reconnect = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&reconnect_request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("reconnect response");
    assert_eq!(reconnect.status(), StatusCode::OK);
}

#[tokio::test]
async fn control_connect_requires_repairing_for_scope_upgrade() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);

    let initial_scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let challenge = issue_challenge(&router).await;
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        initial_scopes.clone(),
        &challenge,
    );
    let initial_request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: initial_scopes.clone(),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let pairing_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&initial_request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("pairing response");
    let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let pairing_error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&pairing_body).expect("pairing error json");
    let pairing_request_id = pairing_error
        .pairing_request_id
        .expect("pairing request id");

    let operator_token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;
    let resolve_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/pairing/resolve")
                .method("POST")
                .header("authorization", format!("Bearer {operator_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id,
                        approve: true,
                    })
                    .expect("encode resolve request"),
                ))
                .expect("request"),
        )
        .await
        .expect("resolve response");
    let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolve: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&resolve_body).expect("resolve json");
    let device_token = resolve.device_token.expect("device token");

    let upgraded_scopes = std::collections::BTreeSet::from([
        ControlPlaneScope::OperatorRead,
        ControlPlaneScope::OperatorAcp,
    ]);
    let upgrade_challenge = issue_challenge(&router).await;
    let upgrade_device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        upgraded_scopes.clone(),
        &upgrade_challenge,
    );
    let upgrade_request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: upgraded_scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loong_protocol::ControlPlaneAuthClaims {
            token: None,
            device_token: Some(device_token),
            bootstrap_token: None,
            password: None,
        }),
        device: Some(upgrade_device),
    };

    let upgrade_response = router
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&upgrade_request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("upgrade response");
    assert_eq!(upgrade_response.status(), StatusCode::FORBIDDEN);
    let upgrade_body = to_bytes(upgrade_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let upgrade_error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&upgrade_body).expect("upgrade error json");
    assert_eq!(
        upgrade_error.code,
        ControlPlaneConnectErrorCode::PairingRequired
    );
    assert!(upgrade_error.pairing_request_id.is_some());
}

#[tokio::test]
async fn pairing_list_surfaces_pending_request_for_unpaired_device() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);

    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let pairing_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("pairing response");
    assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);

    let operator_token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;
    let list_response = router
        .oneshot(bearer_request(
            "GET",
            "/pairing/list?status=pending&limit=10",
            &operator_token,
        ))
        .await
        .expect("pairing list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let list: ControlPlanePairingListResponse =
        serde_json::from_slice(&list_body).expect("pairing list json");
    assert_eq!(list.matched_count, 1);
    assert_eq!(list.returned_count, 1);
    assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Pending);
    assert_eq!(list.requests[0].device_id, "device-1");
}

#[tokio::test]
async fn pairing_list_surfaces_approved_request_after_resolution() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);

    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: scopes.clone(),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let pairing_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("pairing response");
    assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
    let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let pairing_error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&pairing_body).expect("pairing error json");
    let pairing_request_id = pairing_error
        .pairing_request_id
        .expect("pairing request id");

    let operator_token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;
    let resolve_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/pairing/resolve")
                .method("POST")
                .header("authorization", format!("Bearer {operator_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id: pairing_request_id.clone(),
                        approve: true,
                    })
                    .expect("encode resolve request"),
                ))
                .expect("request"),
        )
        .await
        .expect("resolve response");
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolve: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&resolve_body).expect("resolve json");
    assert_eq!(resolve.request.status, ControlPlanePairingStatus::Approved,);
    assert_eq!(resolve.request.pairing_request_id, pairing_request_id);
    assert_eq!(resolve.request.requested_scopes, scopes);
    assert!(resolve.request.resolved_at_ms.is_some());
    assert!(resolve.device_token.is_some());

    let list_response = router
        .oneshot(bearer_request(
            "GET",
            "/pairing/list?status=approved&limit=10",
            &operator_token,
        ))
        .await
        .expect("pairing list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let list: ControlPlanePairingListResponse =
        serde_json::from_slice(&list_body).expect("pairing list json");
    assert_eq!(list.matched_count, 1);
    assert_eq!(list.returned_count, 1);
    assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Approved);
    assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
    assert_eq!(list.requests[0].requested_scopes, scopes);
    assert!(list.requests[0].resolved_at_ms.is_some());
}

#[tokio::test]
async fn pairing_list_surfaces_rejected_request_after_resolution() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    let router = build_control_plane_router(manager);

    let challenge = issue_challenge(&router).await;
    let scopes = std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let device = signed_device_for_request(
        "cli",
        ControlPlaneRole::Operator,
        scopes.clone(),
        &challenge,
    );
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Loong CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: scopes.clone(),
        caps: std::collections::BTreeSet::new(),
        commands: std::collections::BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: None,
        device: Some(device),
    };

    let pairing_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/control/connect")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode request"),
                ))
                .expect("request"),
        )
        .await
        .expect("pairing response");
    assert_eq!(pairing_response.status(), StatusCode::FORBIDDEN);
    let pairing_body = to_bytes(pairing_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let pairing_error: ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&pairing_body).expect("pairing error json");
    let pairing_request_id = pairing_error
        .pairing_request_id
        .expect("pairing request id");

    let operator_token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;
    let resolve_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/pairing/resolve")
                .method("POST")
                .header("authorization", format!("Bearer {operator_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id: pairing_request_id.clone(),
                        approve: false,
                    })
                    .expect("encode resolve request"),
                ))
                .expect("request"),
        )
        .await
        .expect("resolve response");
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolve: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&resolve_body).expect("resolve json");
    assert_eq!(resolve.request.status, ControlPlanePairingStatus::Rejected,);
    assert_eq!(resolve.request.pairing_request_id, pairing_request_id);
    assert_eq!(resolve.request.requested_scopes, scopes);
    assert!(resolve.request.resolved_at_ms.is_some());
    assert!(resolve.device_token.is_none());

    let list_response = router
        .oneshot(bearer_request(
            "GET",
            "/pairing/list?status=rejected&limit=10",
            &operator_token,
        ))
        .await
        .expect("pairing list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = to_bytes(list_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let list: ControlPlanePairingListResponse =
        serde_json::from_slice(&list_body).expect("pairing list json");
    assert_eq!(list.matched_count, 1);
    assert_eq!(list.returned_count, 1);
    assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Rejected);
    assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
    assert_eq!(list.requests[0].requested_scopes, scopes);
    assert!(list.requests[0].resolved_at_ms.is_some());
}

#[tokio::test]
async fn control_snapshot_returns_snapshot_payload() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    manager.set_session_count(7);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/snapshot", &token))
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let snapshot: ControlPlaneSnapshotResponse =
        serde_json::from_slice(&body).expect("snapshot json");
    assert_eq!(snapshot.snapshot.session_count, 7);
}

#[tokio::test]
async fn control_events_returns_recent_events_with_limit() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
    let _ = manager.record_session_message(serde_json::json!({ "idx": 3 }), true);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/events?limit=2", &token))
        .await
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 2);
    assert_eq!(events.events[0].seq, 1);
    assert_eq!(events.events[1].seq, 2);
    assert_eq!(events.events[0].payload["idx"], 1);
    assert_eq!(events.events[1].payload["idx"], 2);
}

#[tokio::test]
async fn control_events_can_include_targeted_records_when_requested() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_session_message(serde_json::json!({ "kind": "broadcast" }), false);
    let _ = manager.record_session_message(serde_json::json!({ "kind": "targeted" }), true);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/events?limit=10&include_targeted=true",
            &token,
        ))
        .await
        .expect("events response");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 2);
    assert_eq!(events.events[1].payload["kind"], "targeted");
}

#[tokio::test]
async fn control_events_supports_after_seq_long_poll() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager.clone());
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;

    let request_future = {
        let router = router.clone();
        let token = token.clone();
        tokio::spawn(async move {
            router
                .oneshot(bearer_request(
                    "GET",
                    "/control/events?after_seq=1&timeout_ms=1000",
                    &token,
                ))
                .await
        })
    };

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

    let response = request_future
        .await
        .expect("join")
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.events[0].payload["idx"], 2);
    assert_eq!(events.events[0].seq, 2);
}

#[tokio::test]
async fn control_events_after_seq_returns_empty_on_timeout() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;

    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/events?after_seq=1&timeout_ms=20",
            &token,
        ))
        .await
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert!(events.events.is_empty());
}

#[tokio::test]
async fn control_subscribe_rejects_missing_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/subscribe")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("subscribe response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn control_subscribe_returns_sse_content_type() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/subscribe?after_seq=0",
            &token,
        ))
        .await
        .expect("subscribe response");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(content_type.starts_with("text/event-stream"));
}

#[tokio::test]
async fn control_subscribe_stream_yields_backlog_event() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
    let stream = control_plane_subscribe_stream(manager, 1, true);
    let mut stream = Box::pin(stream);
    let next = stream.next().await.expect("stream item");
    let event = next.expect("event");
    let event_debug = format!("{event:?}");
    assert!(!event_debug.is_empty());
}

#[tokio::test]
async fn control_subscribe_stream_yields_live_event_after_wait() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let stream = control_plane_subscribe_stream(manager.clone(), 1, true);
    let mut stream = Box::pin(stream);

    let waiter = tokio::spawn(async move { stream.next().await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

    let next = waiter.await.expect("join").expect("stream item");
    let event = next.expect("event");
    let event_debug = format!("{event:?}");
    assert!(!event_debug.is_empty());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn control_snapshot_uses_repository_backed_session_counts_when_available() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    manager.set_session_count(99);
    let (repository_view, acp_view) = seeded_control_plane_views("snapshot-repo");
    let router =
        build_control_plane_router_with_views(manager, Some(repository_view), Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/snapshot", &token))
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let snapshot: ControlPlaneSnapshotResponse =
        serde_json::from_slice(&body).expect("snapshot json");
    assert_eq!(snapshot.snapshot.session_count, 2);
    assert_eq!(snapshot.snapshot.pending_approval_count, 1);
    assert_eq!(snapshot.snapshot.acp_session_count, 1);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_list_returns_visible_repository_sessions() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("session-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/session/list?limit=10", &token))
        .await
        .expect("session list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let sessions: ControlPlaneSessionListResponse =
        serde_json::from_slice(&body).expect("session list json");
    assert_eq!(sessions.current_session_id, "root-session");
    assert_eq!(sessions.matched_count, 2);
    assert_eq!(sessions.returned_count, 2);
    assert!(
        sessions
            .sessions
            .iter()
            .any(|session| session.session_id == "root-session")
    );
    let child = sessions
        .sessions
        .iter()
        .find(|session| session.session_id == "child-session")
        .expect("child session");
    assert_eq!(child.workflow.workflow_id, "root-session");
    assert_eq!(
        child.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(child.workflow.phase.as_deref(), Some("execute"));
    assert_eq!(
        child
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .mode,
        "advisory_only"
    );
    assert!(
        !sessions
            .sessions
            .iter()
            .any(|session| session.session_id == "hidden-root")
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_read_returns_repository_observation_for_visible_session() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("session-read")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/session/read?session_id=child-session&recent_event_limit=10",
            &token,
        ))
        .await
        .expect("session read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let session: ControlPlaneSessionReadResponse =
        serde_json::from_slice(&body).expect("session read json");
    assert_eq!(session.current_session_id, "root-session");
    assert_eq!(session.observation.session.session_id, "child-session");
    assert_eq!(
        session.observation.session.workflow.workflow_id,
        "root-session"
    );
    assert_eq!(
        session.observation.session.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(
        session.observation.session.workflow.phase.as_deref(),
        Some("execute")
    );
    assert_eq!(
        session
            .observation
            .session
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .execution_surface,
        "delegate.async"
    );
    assert_eq!(session.observation.recent_events.len(), 1);
    assert_eq!(
        session.observation.recent_events[0].event_kind,
        "delegate_started"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_list_returns_visible_background_tasks() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
        .await
        .expect("task list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let tasks: ControlPlaneTaskListResponse =
        serde_json::from_slice(&body).expect("task list json");
    assert_eq!(tasks.current_session_id, "root-session");
    assert_eq!(tasks.matched_count, 1);
    assert_eq!(tasks.returned_count, 1);
    let task = tasks.tasks.first().expect("task summary");
    assert_eq!(task.task_id, "child-session");
    assert_eq!(task.workflow.workflow_id, "root-session");
    assert_eq!(
        task.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(task.workflow.phase.as_deref(), Some("execute"));
    assert_eq!(
        task.workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .task_id,
        "child-session"
    );
    assert_eq!(task.delegate_mode.as_deref(), Some("async"));
    assert_eq!(task.requested_tool_ids, vec!["file.read".to_owned()]);
    assert_eq!(task.visible_requested_tool_ids, vec!["read".to_owned()]);
    assert_eq!(task.effective_tool_ids, vec!["file.read".to_owned()]);
    assert_eq!(task.visible_effective_tool_ids, vec!["read".to_owned()]);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_read_returns_visible_background_task_detail() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-read")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/task/read?task_id=child-session",
            &token,
        ))
        .await
        .expect("task read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let task: ControlPlaneTaskReadResponse = serde_json::from_slice(&body).expect("task read json");
    assert_eq!(task.current_session_id, "root-session");
    assert_eq!(task.task.task_id, "child-session");
    assert_eq!(task.task.workflow.workflow_id, "root-session");
    assert_eq!(
        task.task
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .worktree
            .as_ref()
            .expect("worktree binding")
            .worktree_id,
        "child-session"
    );
    assert_eq!(task.task.delegate_phase.as_deref(), Some("running"));
    assert_eq!(task.task.approval_request_count, 1);
    assert_eq!(task.task.requested_tool_ids, vec!["file.read".to_owned()]);
    assert_eq!(
        task.task.visible_requested_tool_ids,
        vec!["read".to_owned()]
    );
    assert_eq!(task.task.effective_tool_ids, vec!["file.read".to_owned()]);
    assert_eq!(
        task.task.visible_effective_tool_ids,
        vec!["read".to_owned()]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_routes_reject_insufficient_scope() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-scope")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;

    let list_response = router
        .clone()
        .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
        .await
        .expect("task list response");
    assert_eq!(list_response.status(), StatusCode::FORBIDDEN);

    let read_response = router
        .oneshot(bearer_request(
            "GET",
            "/task/read?task_id=child-session",
            &token,
        ))
        .await
        .expect("task read response");
    assert_eq!(read_response.status(), StatusCode::FORBIDDEN);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_list_returns_only_visible_requests() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("approval-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorApprovals]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/approval/list?status=pending&limit=10",
            &token,
        ))
        .await
        .expect("approval list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let approvals: ControlPlaneApprovalListResponse =
        serde_json::from_slice(&body).expect("approval list json");
    assert_eq!(approvals.current_session_id, "root-session");
    assert_eq!(approvals.matched_count, 1);
    assert_eq!(approvals.returned_count, 1);
    assert_eq!(approvals.approvals[0].approval_request_id, "apr-visible");
    assert_eq!(
        approvals.approvals[0].status,
        ControlPlaneApprovalRequestStatus::Pending
    );
    assert_eq!(
        approvals.approvals[0].reason.as_deref(),
        Some("governed_tool_requires_approval")
    );
    assert_eq!(
        approvals.approvals[0].visible_tool_name.as_deref(),
        Some("delegate")
    );
    assert_eq!(
        approvals.approvals[0].request_summary.as_ref(),
        Some(&serde_json::json!({
            "tool": "delegate",
            "request": {}
        }))
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn acp_session_list_returns_only_visible_sessions() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let (_repository_view, acp_view) = seeded_control_plane_views("acp-list");
    let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/acp/session/list?limit=10", &token))
        .await
        .expect("ACP session list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let sessions: ControlPlaneAcpSessionListResponse =
        serde_json::from_slice(&body).expect("ACP session list json");
    assert_eq!(sessions.current_session_id, "root-session");
    assert_eq!(sessions.matched_count, 1);
    assert_eq!(sessions.returned_count, 1);
    assert_eq!(
        sessions.sessions[0].session_key,
        "agent:codex:child-session"
    );
    assert_eq!(
        sessions.sessions[0]
            .binding
            .as_ref()
            .expect("binding")
            .route_session_id,
        "child-session"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn acp_session_read_returns_live_status_for_visible_session() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let (_repository_view, acp_view) = seeded_control_plane_views("acp-read");
    let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/acp/session/read?session_key=agent:codex:child-session",
            &token,
        ))
        .await
        .expect("ACP session read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let session: ControlPlaneAcpSessionReadResponse =
        serde_json::from_slice(&body).expect("ACP session read json");
    assert_eq!(session.current_session_id, "root-session");
    assert_eq!(session.metadata.session_key, "agent:codex:child-session");
    assert_eq!(session.status.session_key, "agent:codex:child-session");
    assert_eq!(session.status.state, ControlPlaneAcpSessionState::Ready);
    assert_eq!(
        session.status.mode,
        Some(ControlPlaneAcpSessionMode::Interactive)
    );
    assert!(
        session
            .status
            .last_error
            .as_deref()
            .is_some_and(|error| error.starts_with("status_unavailable:")),
        "expected ACP session read to degrade with status_unavailable when backend is absent"
    );
}

#[tokio::test]
async fn control_snapshot_rejects_missing_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/snapshot")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn turn_submit_returns_service_unavailable_without_runtime() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
    )
    .await;
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "session-1".to_owned(),
        input: "hello".to_owned(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let response = router
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn turn_submit_rejects_insufficient_scope() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str =
        Box::leak(format!("control-plane-turn-scope-{}", current_time_ms()).into_boxed_str());
    let turn_runtime = seeded_turn_runtime(backend_id, state);
    let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "session-1".to_owned(),
        input: "hello".to_owned(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let response = router
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn turn_submit_rejects_hidden_session_visibility() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let backend_state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str =
        Box::leak(format!("control-plane-turn-hidden-{}", current_time_ms()).into_boxed_str());
    let turn_runtime = seeded_turn_runtime(backend_id, backend_state);
    let (repository_view, acp_view) = seeded_control_plane_views("turn-hidden-session");
    let router = build_control_plane_router_with_turn_runtime_and_views(
        manager,
        repository_view,
        acp_view,
        turn_runtime,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
    )
    .await;
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "hidden-root".to_owned(),
        input: "hello".to_owned(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let response = router
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn turn_result_and_stream_reject_hidden_session_visibility() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let backend_state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str = Box::leak(
        format!("control-plane-turn-hidden-result-{}", current_time_ms()).into_boxed_str(),
    );
    let turn_runtime = seeded_turn_runtime(backend_id, backend_state);
    let turn_snapshot = turn_runtime.registry.issue_turn("hidden-root");
    let turn_id = turn_snapshot.turn_id.clone();
    let (repository_view, acp_view) = seeded_control_plane_views("turn-hidden-result");
    let result_router = build_control_plane_router_with_turn_runtime_and_views(
        manager,
        repository_view,
        acp_view,
        turn_runtime.clone(),
    );
    let token = connect_token(
        &result_router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let result_response = result_router
        .clone()
        .oneshot(bearer_request(
            "GET",
            format!("/turn/result?turn_id={turn_id}").as_str(),
            &token,
        ))
        .await
        .expect("turn result response");
    assert_eq!(result_response.status(), StatusCode::FORBIDDEN);
    let stream_response = result_router
        .oneshot(bearer_request(
            "GET",
            format!("/turn/stream?turn_id={turn_id}").as_str(),
            &token,
        ))
        .await
        .expect("turn stream response");
    assert_eq!(stream_response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn turn_submit_and_result_fetch_complete_with_streamed_backend() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str =
        Box::leak(format!("control-plane-turn-success-{}", current_time_ms()).into_boxed_str());
    let turn_runtime = seeded_turn_runtime(backend_id, state.clone());
    let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
    )
    .await;
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "session-1".to_owned(),
        input: "hello".to_owned(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let submit_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
    let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
        .await
        .expect("submit body");
    let submit: ControlPlaneTurnSubmitResponse =
        serde_json::from_slice(&submit_body).expect("submit json");
    assert_eq!(submit.turn.status, ControlPlaneTurnStatus::Running);

    let turn_id = submit.turn.turn_id.clone();
    let mut final_result = None;
    for _ in 0..20 {
        let result_response = router
            .clone()
            .oneshot(bearer_request(
                "GET",
                format!("/turn/result?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn result response");
        assert_eq!(result_response.status(), StatusCode::OK);
        let result_body = to_bytes(result_response.into_body(), usize::MAX)
            .await
            .expect("result body");
        let result: ControlPlaneTurnResultResponse =
            serde_json::from_slice(&result_body).expect("result json");
        if result.turn.status.is_terminal() {
            final_result = Some(result);
            break;
        }
        tokio::task::yield_now().await;
    }

    let final_result = final_result.expect("turn should reach a terminal state");
    assert_eq!(
        final_result.turn.status,
        ControlPlaneTurnStatus::Completed,
        "turn result error: {:?}",
        final_result.error
    );
    assert_eq!(final_result.output_text.as_deref(), Some("streamed: hello"));
    assert_eq!(final_result.stop_reason.as_deref(), Some("completed"));
    assert_eq!(
        final_result
            .usage
            .as_ref()
            .and_then(|usage| usage.get("total_tokens")),
        Some(&serde_json::json!(7))
    );
    assert!(
        final_result.turn.event_count >= 3,
        "expected runtime events plus terminal event"
    );
    assert_eq!(
        state.sink_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn turn_stream_replays_buffered_runtime_and_terminal_events() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str =
        Box::leak(format!("control-plane-turn-stream-{}", current_time_ms()).into_boxed_str());
    let turn_runtime = seeded_turn_runtime(backend_id, state);
    let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
    )
    .await;
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "session-stream".to_owned(),
        input: "stream me".to_owned(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let submit_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
        .await
        .expect("submit body");
    let submit: ControlPlaneTurnSubmitResponse =
        serde_json::from_slice(&submit_body).expect("submit json");
    let turn_id = submit.turn.turn_id;

    for _ in 0..20 {
        let result_response = router
            .clone()
            .oneshot(bearer_request(
                "GET",
                format!("/turn/result?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn result response");
        let result_body = to_bytes(result_response.into_body(), usize::MAX)
            .await
            .expect("result body");
        let result: ControlPlaneTurnResultResponse =
            serde_json::from_slice(&result_body).expect("result json");
        if result.turn.status.is_terminal() {
            break;
        }
        tokio::task::yield_now().await;
    }

    let stream_response = router
        .oneshot(bearer_request(
            "GET",
            format!("/turn/stream?turn_id={turn_id}").as_str(),
            &token,
        ))
        .await
        .expect("turn stream response");
    assert_eq!(stream_response.status(), StatusCode::OK);
    let stream_body = to_bytes(stream_response.into_body(), usize::MAX)
        .await
        .expect("stream body");
    let stream_text = String::from_utf8(stream_body.to_vec()).expect("utf8 stream body");
    assert!(stream_text.contains("event: turn.event"));
    assert!(stream_text.contains("event: turn.terminal"));
    assert!(stream_text.contains("\"type\":\"text\""));
    assert!(stream_text.contains("chunk:stream me"));
    assert!(stream_text.contains("\"event_type\":\"turn.completed\""));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_list_rejects_insufficient_scope() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("approval-list-scope")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/approval/list?status=pending&limit=10",
            &token,
        ))
        .await
        .expect("approval list response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn turn_submit_preserves_input_whitespace() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let state = Arc::new(TestTurnBackendState::default());
    let backend_id: &'static str =
        Box::leak(format!("control-plane-turn-whitespace-{}", current_time_ms()).into_boxed_str());
    let turn_runtime = seeded_turn_runtime(backend_id, state);
    let router = build_control_plane_router_with_turn_runtime(manager, turn_runtime);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAdmin]),
    )
    .await;
    let input = "  hello\n\n```rust\nfn main() {}\n```\n".to_owned();
    let request = ControlPlaneTurnSubmitRequest {
        session_id: "session-whitespace".to_owned(),
        input: input.clone(),
        channel_id: None,
        account_id: None,
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        working_directory: None,
        metadata: std::collections::BTreeMap::new(),
    };
    let submit_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/turn/submit")
                .method("POST")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&request).expect("encode turn submit request"),
                ))
                .expect("request"),
        )
        .await
        .expect("turn submit response");
    assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
    let submit_body = to_bytes(submit_response.into_body(), usize::MAX)
        .await
        .expect("submit body");
    let submit: ControlPlaneTurnSubmitResponse =
        serde_json::from_slice(&submit_body).expect("submit json");
    let turn_id = submit.turn.turn_id;
    let mut final_result = None;
    for _ in 0..20 {
        let result_response = router
            .clone()
            .oneshot(bearer_request(
                "GET",
                format!("/turn/result?turn_id={turn_id}").as_str(),
                &token,
            ))
            .await
            .expect("turn result response");
        let result_body = to_bytes(result_response.into_body(), usize::MAX)
            .await
            .expect("result body");
        let result: ControlPlaneTurnResultResponse =
            serde_json::from_slice(&result_body).expect("result json");
        if result.turn.status.is_terminal() {
            final_result = Some(result);
            break;
        }
        tokio::task::yield_now().await;
    }
    let final_result = final_result.expect("turn should reach a terminal state");
    let expected_output = format!("streamed: {input}");
    assert_eq!(
        final_result.output_text.as_deref(),
        Some(expected_output.as_str()),
        "turn result error: {:?}",
        final_result.error
    );
}

#[tokio::test]
async fn turn_stream_stops_when_retention_prunes_completed_turn() {
    let registry = Arc::new(mvp::control_plane::ControlPlaneTurnRegistry::new());
    let turn = registry.issue_turn("session-pruned");
    let turn_id = turn.turn_id.clone();
    registry
        .complete_success(turn_id.as_str(), "done", Some("completed"), None)
        .expect("complete pruned turn");
    let initial_state =
        initial_turn_stream_state(registry.clone(), turn_id.as_str(), 1).expect("state");
    for index in 0..300 {
        let session_id = format!("session-retained-{index}");
        let output_text = format!("output-{index}");
        let retained_turn = registry.issue_turn(session_id.as_str());
        registry
            .complete_success(
                retained_turn.turn_id.as_str(),
                output_text.as_str(),
                Some("completed"),
                None,
            )
            .expect("complete retained turn");
    }
    let next_item = next_turn_sse_item(initial_state).await;
    assert!(next_item.is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn acp_session_list_rejects_insufficient_scope() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let (_repository_view, acp_view) = seeded_control_plane_views("acp-list-scope");
    let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/acp/session/list?limit=10", &token))
        .await
        .expect("ACP session list response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
