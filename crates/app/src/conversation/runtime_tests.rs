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
use crate::test_utils::unique_temp_dir;
use crate::tools::ToolView;
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
        _ctx: &crate::Context<'_>,
        _include_system_prompt: bool,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongConfig,
        _messages: &[Value],
        _ctx: &crate::Context<'_>,
    ) -> CliResult<String> {
        Ok(String::new())
    }

    async fn request_turn(
        &self,
        _config: &LoongConfig,
        _turn_id: &str,
        _messages: &[Value],
        _ctx: &crate::Context<'_>,
    ) -> CliResult<ProviderTurn> {
        Ok(ProviderTurn::default())
    }

    async fn request_turn_streaming(
        &self,
        _config: &LoongConfig,
        _turn_id: &str,
        _messages: &[Value],
        _ctx: &crate::Context<'_>,
        _on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        Ok(ProviderTurn::default())
    }

    async fn persist_turn(
        &self,
        _role: &str,
        _content: &str,
        _ctx: &crate::Context<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
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

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn hosted_runtime_build_context_delegates_to_inner_runtime() {
    #[derive(Clone)]
    struct BuildContextAwareRuntime;

    #[async_trait]
    impl ConversationRuntime for BuildContextAwareRuntime {
        async fn build_context(
            &self,
            _config: &LoongConfig,
            _ctx: &crate::Context<'_>,
            _include_system_prompt: bool,
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
                runtime_self_continuity: None,
            };

            Ok(assembled)
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _ctx: &crate::Context<'_>,
            _include_system_prompt: bool,
        ) -> CliResult<Vec<Value>> {
            Err("build_messages should not be used when build_context is delegated".to_owned())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _ctx: &crate::Context<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _turn_id: &str,
            _messages: &[Value],
            _ctx: &crate::Context<'_>,
        ) -> CliResult<ProviderTurn> {
            Err("unused".to_owned())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _turn_id: &str,
            _messages: &[Value],
            _ctx: &crate::Context<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Err("unused".to_owned())
        }

        async fn persist_turn(
            &self,
            _role: &str,
            _content: &str,
            _ctx: &crate::Context<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    let config = LoongConfig::default();
    let hosted_runtime = HostedConversationRuntime::new(BuildContextAwareRuntime);
    let harness = TurnTestHarness::new();
    let ctx = harness.context();

    let assembled = hosted_runtime
        .build_context(&config, &ctx, true)
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

#[tokio::test]
async fn default_runtime_build_context_rehydrates_active_skills() {
    let session_id = "session-active-external-skills";
    let harness = TurnTestHarness::with_capabilities(std::collections::BTreeSet::from([
        loong_contracts::Capability::InvokeTool,
        loong_contracts::Capability::FilesystemRead,
        loong_contracts::Capability::FilesystemWrite,
        loong_contracts::Capability::MemoryRead,
    ]));
    let runtime = crate::conversation::DefaultConversationRuntime::new();
    let sqlite_path = harness.temp_dir.join("memory.sqlite3");
    let workspace_root = harness.temp_dir.join("workspace");
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

    let session = crate::Session::from_config(
        harness.runtime.as_ref(),
        &config,
        session_id,
        harness.session.agent_id(),
        harness.session.session_mode,
    )
    .expect("load session context");
    let ctx = crate::Context::new(harness.runtime.as_ref(), &session)
        .expect("loaded Session must remain bound to the harness Runtime");

    let assembled = runtime
        .build_context(&config, &ctx, true)
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
async fn session_rematerialization_excludes_active_skill_blocked_tools() {
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
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config)
        .expect("bootstrap runtime before session materialization");
    let session = crate::Session::from_config(
        runtime.as_ref(),
        &config,
        session_id,
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("materialize session before policy update");
    assert!(session.tool_view.contains("web"));

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

    let session = session
        .rematerialize(runtime.as_ref(), &config)
        .expect("rematerialize updated session projection");
    let tool_view = &session.tool_view;

    assert!(
        !tool_view.contains("web"),
        "blocked hidden tool should also remove its direct surface"
    );
    assert!(
        tool_view.contains("read"),
        "unrelated direct tools should remain visible"
    );
}

#[test]
fn session_rematerialization_cannot_expand_tool_authority() {
    let root = unique_temp_dir("session-rematerialization-tool-ceiling");
    let mut config = LoongConfig::default();
    config.memory.sqlite_path = root.join("memory.db").display().to_string();
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config)
        .expect("bootstrap runtime before session rematerialization");
    let session = crate::Session::root(
        runtime.as_ref(),
        "test-agent",
        "narrow-session",
        loong_contracts::GovernedSessionMode::MutatingCapable,
        loong_contracts::Capabilities::from([
            loong_contracts::Capability::InvokeTool,
            loong_contracts::Capability::FilesystemRead,
        ]),
        tool_runtime_config,
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        ToolView::from_legacy_paths(["read"]),
        None,
        None,
    )
    .expect("narrow session");

    let rematerialized = session
        .rematerialize(runtime.as_ref(), &config)
        .expect("rematerialize narrow session");

    assert!(rematerialized.tool_view.contains("read"));
    assert!(!rematerialized.tool_view.contains("memory_search"));
}
