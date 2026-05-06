use super::*;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::active_skills::{
    ACTIVE_SKILLS_EVENT_KIND, ActiveSkill, ActiveSkillsState,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};
use crate::test_support::TurnTestHarness;
use crate::test_support::unique_temp_dir;
#[cfg(feature = "memory-sqlite")]
use serde_json::json;
#[cfg(feature = "memory-sqlite")]
use std::sync::Arc;

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct NoopTestSpawner;

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for NoopTestSpawner {
    async fn spawn(&self, _request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
struct SpawnerAwareRuntime {
    async_delegate_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
    background_task_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl ConversationRuntime for SpawnerAwareRuntime {
    fn tool_view(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        Ok(crate::tools::runtime_tool_view())
    }

    fn async_delegate_spawner(
        &self,
        _config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        self.async_delegate_spawner.clone()
    }

    fn background_task_spawner(
        &self,
        _config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        self.background_task_spawner.clone()
    }

    async fn build_messages(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        Ok(ProviderTurn::default())
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &ToolView,
        _binding: ConversationRuntimeBinding<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        Ok(ProviderTurn::default())
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[test]
fn provider_runtime_binding_maps_direct_conversation_binding_to_advisory_only() {
    assert!(matches!(
        provider_runtime_binding(ConversationRuntimeBinding::direct()),
        provider::ProviderRuntimeBinding::AdvisoryOnly
    ));
}

#[test]
fn provider_runtime_binding_maps_kernel_conversation_binding_to_kernel() {
    let harness = TurnTestHarness::new();

    assert!(matches!(
        provider_runtime_binding(ConversationRuntimeBinding::kernel(&harness.kernel_ctx)),
        provider::ProviderRuntimeBinding::Kernel(kernel_ctx)
            if std::ptr::eq(kernel_ctx, &harness.kernel_ctx)
    ));
}

#[test]
fn normalize_turn_middleware_ids_preserves_first_occurrence_order() {
    let normalized = normalize_turn_middleware_ids(vec![
        "alpha".to_owned(),
        "beta".to_owned(),
        "alpha".to_owned(),
        "gamma".to_owned(),
        "beta".to_owned(),
    ]);

    assert_eq!(normalized, vec!["alpha", "beta", "gamma"]);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn hosted_runtime_overrides_background_task_spawner_without_changing_async_delegate_spawner() {
    let config = LoongConfig::default();
    let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let override_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let inner_runtime = SpawnerAwareRuntime {
        async_delegate_spawner: Some(inner_async_spawner.clone()),
        background_task_spawner: Some(inner_background_spawner),
    };

    let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
        .with_background_task_spawner(override_background_spawner.clone());

    let resolved_async_spawner = hosted_runtime
        .async_delegate_spawner(&config)
        .expect("async delegate spawner");
    let resolved_background_spawner = hosted_runtime
        .background_task_spawner(&config)
        .expect("background task spawner");

    assert!(Arc::ptr_eq(&resolved_async_spawner, &inner_async_spawner));
    assert!(Arc::ptr_eq(
        &resolved_background_spawner,
        &override_background_spawner
    ));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn hosted_runtime_overrides_async_delegate_spawner_without_changing_background_task_spawner() {
    let config = LoongConfig::default();
    let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let override_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
    let inner_runtime = SpawnerAwareRuntime {
        async_delegate_spawner: Some(inner_async_spawner),
        background_task_spawner: Some(inner_background_spawner.clone()),
    };

    let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
        .with_async_delegate_spawner(override_async_spawner.clone());

    let resolved_async_spawner = hosted_runtime
        .async_delegate_spawner(&config)
        .expect("async delegate spawner");
    let resolved_background_spawner = hosted_runtime
        .background_task_spawner(&config)
        .expect("background task spawner");

    assert!(Arc::ptr_eq(
        &resolved_async_spawner,
        &override_async_spawner
    ));
    assert!(Arc::ptr_eq(
        &resolved_background_spawner,
        &inner_background_spawner
    ));
}

#[test]
fn async_delegate_spawn_request_round_trips_runtime_self_continuity_json() {
    let execution = super::super::subagent::ConstrainedSubagentExecution {
        mode: super::super::subagent::ConstrainedSubagentMode::Async,
        isolation: super::super::subagent::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 2,
        active_children: 0,
        max_active_children: 2,
        timeout_seconds: 30,
        allow_shell_in_child: false,
        child_tool_allowlist: vec!["file.read".to_owned()],
        workspace_root: None,
        runtime_narrowing: ToolRuntimeNarrowing::default(),
        kernel_bound: false,
        identity: None,
        profile: Some(super::super::subagent::ConstrainedSubagentProfile::for_child_depth(1, 2)),
    };
    let continuity = RuntimeSelfContinuity {
        workspace_guidance: crate::workspace_guidance::WorkspaceGuidanceModel {
            entries: vec!["Keep continuity explicit.".to_owned()],
        },
        runtime_self: crate::runtime_self::RuntimeSelfModel::default(),
        resolved_identity: None,
        session_profile_projection: Some("delegate profile".to_owned()),
    };
    let request = AsyncDelegateSpawnRequest {
        child_session_id: "child-1".to_owned(),
        parent_session_id: "parent-1".to_owned(),
        task: "investigate".to_owned(),
        canonical_task_id: Some("task-1".to_owned()),
        label: Some("child".to_owned()),
        profile: Some(DelegateBuiltinProfile::Research),
        execution: execution.clone(),
        runtime_self_continuity: Some(continuity.clone()),
        timeout_seconds: 30,
        binding: OwnedConversationRuntimeBinding::direct(),
    };

    let encoded = request
        .runtime_self_continuity_json()
        .expect("serialize runtime self continuity");
    let round_tripped = async_delegate_spawn_request_from_serialized_parts(
        request.child_session_id.clone(),
        request.parent_session_id.clone(),
        request.task.clone(),
        request.canonical_task_id.clone(),
        request.label.clone(),
        request.profile,
        execution,
        encoded,
        request.timeout_seconds,
        OwnedConversationRuntimeBinding::direct(),
    )
    .expect("round-trip async delegate request");

    assert_eq!(
        round_tripped.runtime_self_continuity,
        Some(continuity),
        "runtime self continuity should survive serialization"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn hosted_runtime_build_context_delegates_to_inner_runtime() {
    #[derive(Clone)]
    struct BuildContextAwareRuntime;

    #[async_trait]
    impl ConversationRuntime for BuildContextAwareRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        async fn build_context(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<AssembledConversationContext> {
            let messages = vec![serde_json::json!({
                "role": "system",
                "content": "delegated"
            })];
            let prompt_fragment = PromptFragment::new(
                "fragment",
                PromptLane::RuntimeSelf,
                "runtime-self",
                "delegated fragment",
                ContextArtifactKind::RuntimeContract,
            );
            let assembled = AssembledConversationContext {
                messages,
                artifacts: Vec::new(),
                estimated_tokens: Some(7),
                prompt_fragments: vec![prompt_fragment],
                system_prompt_addition: Some("addition".to_owned()),
            };

            Ok(assembled)
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Err("build_messages should not be used when build_context is delegated".to_owned())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Err("unused".to_owned())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Err("unused".to_owned())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    let config = LoongConfig::default();
    let hosted_runtime = HostedConversationRuntime::new(BuildContextAwareRuntime);

    let assembled = hosted_runtime
        .build_context(
            &config,
            "session-1",
            true,
            ConversationRuntimeBinding::Direct,
        )
        .await
        .expect("delegated build_context");

    assert_eq!(assembled.messages.len(), 1);
    assert_eq!(assembled.estimated_tokens, Some(7));
    assert_eq!(assembled.prompt_fragments.len(), 1);
    assert_eq!(
        assembled.system_prompt_addition.as_deref(),
        Some("addition")
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn load_hosted_default_conversation_runtime_keeps_default_async_spawner_only() {
    let config = LoongConfig::default();
    let runtime = load_hosted_default_conversation_runtime(&config)
        .expect("load hosted default conversation runtime");

    let async_delegate_spawner = runtime.async_delegate_spawner(&config);
    let background_task_spawner = runtime.background_task_spawner(&config);

    assert!(async_delegate_spawner.is_some());
    assert!(background_task_spawner.is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn hosted_runtime_persist_turn_uses_explicit_memory_config_for_direct_binding() {
    let root = unique_temp_dir("hosted-runtime-explicit-memory");
    let sqlite_path = root.join("explicit-memory.db");
    let memory_config =
        crate::session::store::SessionStoreConfig::for_sqlite_path(sqlite_path.clone());
    let session_id = "hosted-runtime-explicit-session";
    let runtime = HostedConversationRuntime::new_with_memory_config(
        SpawnerAwareRuntime {
            async_delegate_spawner: None,
            background_task_spawner: None,
        },
        memory_config.clone(),
    );

    crate::session::store::ensure_session_store_ready(Some(sqlite_path), &memory_config)
        .expect("initialize explicit session store");

    runtime
        .persist_turn(
            session_id,
            "assistant",
            "persist via explicit memory config",
            ConversationRuntimeBinding::Direct,
        )
        .await
        .expect("persist hosted runtime turn");

    let turns = crate::session::store::window_session_turns(session_id, 8, &memory_config)
        .expect("load persisted turns");
    let persisted_turn = turns
        .iter()
        .find(|turn| {
            turn.role == "assistant" && turn.content == "persist via explicit memory config"
        })
        .expect("persisted assistant turn");

    assert_eq!(persisted_turn.role, "assistant");
    assert_eq!(persisted_turn.content, "persist via explicit memory config");
}

#[tokio::test]
async fn default_runtime_build_context_rehydrates_active_skills() {
    let runtime = DefaultConversationRuntime::default();
    let session_id = "session-active-external-skills";
    let root = unique_temp_dir("active-external-skills-runtime");
    let sqlite_path = root.join("memory.db");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");

    let mut config = LoongConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root.display().to_string());

    let memory_config =
        crate::session::store::session_store_config_from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_skills": ActiveSkillsState {
                    skills: vec![ActiveSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: vec!["shell.exec".to_owned()],
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

    let assembled = runtime
        .build_context(
            &config,
            session_id,
            true,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("build context");
    let system_content = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should be text");

    assert!(
        system_content.contains("[active_skills]"),
        "expected active skills marker, got: {system_content}"
    );
    assert!(
        system_content.contains("release-guard"),
        "expected skill id in system prompt, got: {system_content}"
    );
    assert!(
        system_content.contains("Release Guard"),
        "expected skill display name in system prompt, got: {system_content}"
    );
    assert!(
        system_content.contains("protect releases"),
        "expected skill instructions in system prompt, got: {system_content}"
    );
    assert!(
        system_content.contains("Allowed tools: shell.exec"),
        "expected allowed tool summary in system prompt, got: {system_content}"
    );
    assert!(
        system_content.contains("Blocked tools: web.fetch"),
        "expected blocked tool summary in system prompt, got: {system_content}"
    );
}

#[tokio::test]
async fn default_runtime_tool_view_excludes_active_skill_blocked_tools() {
    let runtime = DefaultConversationRuntime::default();
    let session_id = "session-active-external-skill-tool-block";
    let root = unique_temp_dir("active-external-skill-tool-block");
    let sqlite_path = root.join("memory.db");

    let mut config = LoongConfig::default();
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let base_tool_view = crate::tools::runtime_tool_view_from_loong_config(&config);
    assert!(
        base_tool_view.contains("web"),
        "default runtime should expose the direct web surface"
    );

    let memory_config =
        crate::session::store::session_store_config_from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_skills": ActiveSkillsState {
                    skills: vec![ActiveSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: Vec::new(),
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

    let tool_view = runtime
        .tool_view(&config, session_id, ConversationRuntimeBinding::direct())
        .expect("runtime tool view");

    assert!(
        !tool_view.contains("web"),
        "blocked hidden tool should also remove its direct surface"
    );
    assert!(
        tool_view.contains("read"),
        "unrelated direct tools should remain visible"
    );
}
