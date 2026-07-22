use super::*;

pub(crate) fn reject_disabled_cli_channel(config: &LoongConfig) -> CliResult<()> {
    if config.cli.enabled {
        return Ok(());
    }

    Err("CLI channel is disabled by config.cli.enabled=false".to_owned())
}

pub(super) fn ensure_cli_channel_enabled_for_entrypoint(
    config_path: Option<&str>,
) -> CliResult<()> {
    let resolved_config_path = config_path
        .map(config::expand_path)
        .unwrap_or_else(config::default_config_path);
    let config_exists = resolved_config_path.try_exists().map_err(|error| {
        format!(
            "failed to access config path {}: {error}",
            resolved_config_path.display()
        )
    })?;
    if !config_exists {
        return Ok(());
    }

    let (_resolved_path, config) = config::load(config_path)?;
    reject_disabled_cli_channel(&config)
}

/// Assemble a CLI turn runtime starting from a config path on disk.
///
/// This is the highest-level bootstrap used by `chat`/`ask`: it loads the
/// config, resolves startup session selection, exports runtime
/// environment variables, bootstraps the shared runtime, and delegates the
/// final session/memory assembly to the lower-level helpers below.
pub(crate) fn initialize_cli_turn_runtime(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_scope: &'static str,
) -> CliResult<CliTurnRuntime> {
    let (resolved_path, config) = config::load(config_path)?;
    initialize_cli_turn_runtime_with_loaded_config(
        resolved_path,
        config,
        session_hint,
        options,
        kernel_scope,
        CliSessionRequirement::AllowImplicitDefault,
        true,
    )
}

/// Assemble a CLI turn runtime when the caller already owns a resolved config.
///
/// Compared with `initialize_cli_turn_runtime`, this skips config loading but
/// still normalizes the runtime workspace root, optionally exports runtime
/// environment variables, bootstraps the shared runtime, and materializes the
/// owned Session only after final session selection.
pub(crate) fn initialize_cli_turn_runtime_with_loaded_config(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_scope: &'static str,
    session_requirement: CliSessionRequirement,
    initialize_runtime_environment: bool,
) -> CliResult<CliTurnRuntime> {
    let mut config = config;
    // Interactive chat surfaces should anchor tool-relative filesystem access
    // to the launch directory when possible, rather than forcing every turn to
    // inherit the static configured file root.
    let runtime_workspace_root = std::env::current_dir()
        .ok()
        .unwrap_or_else(|| config.tools.resolved_file_root());
    let runtime_workspace_root =
        dunce::canonicalize(&runtime_workspace_root).unwrap_or(runtime_workspace_root);
    let runtime_workspace_root = runtime_workspace_root.display().to_string();
    config.tools.runtime_workspace_root = Some(runtime_workspace_root);

    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));
    }
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config)?;
    assemble_cli_turn_runtime(
        resolved_path,
        config,
        session_hint,
        options,
        session_requirement,
        move |config, session_id| {
            let session = crate::Session::from_config(
                runtime.as_ref(),
                config,
                session_id,
                kernel_scope,
                loong_contracts::GovernedSessionMode::MutatingCapable,
            )?;
            Ok((runtime, session))
        },
    )
}

/// Final assembly step for CLI/chat turn state once config and kernel authority
/// are already available.
///
/// This helper resolves ACP defaults, prepares memory/sqlite state, derives the
/// effective session id/address, and constructs the `CliTurnRuntime`. It
/// deliberately does not mutate process environment variables or bootstrap a
/// second Runtime; callers use it when those concerns were already handled by
/// an outer runtime surface.
pub(crate) fn initialize_cli_turn_runtime_with_loaded_config_and_runtime(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    runtime: Arc<loong_runtime::runtime::Runtime<crate::RuntimeContextFactory>>,
    agent_id: impl Into<String>,
    session_requirement: CliSessionRequirement,
) -> CliResult<CliTurnRuntime> {
    let agent_id = agent_id.into();
    assemble_cli_turn_runtime(
        resolved_path,
        config,
        session_hint,
        options,
        session_requirement,
        move |config, session_id| {
            let session = crate::Session::from_config(
                runtime.as_ref(),
                config,
                session_id,
                agent_id,
                loong_contracts::GovernedSessionMode::MutatingCapable,
            )?;
            Ok((runtime, session))
        },
    )
}

/// Shares CLI assembly while preserving the required ownership order:
/// session selection completes before either runtime-issued or inherited
/// authority is attached to the resulting `CliTurnRuntime`.
fn assemble_cli_turn_runtime<F>(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    session_requirement: CliSessionRequirement,
    build_runtime_session: F,
) -> CliResult<CliTurnRuntime>
where
    F: FnOnce(
        &LoongConfig,
        &str,
    ) -> CliResult<(
        Arc<loong_runtime::runtime::Runtime<crate::RuntimeContextFactory>>,
        crate::Session,
    )>,
{
    let effective_bootstrap_mcp_servers = config
        .acp
        .dispatch
        .bootstrap_mcp_server_names_with_additions(&options.acp_bootstrap_mcp_servers)?;
    let effective_working_directory = options
        .acp_working_directory
        .clone()
        .or_else(|| config.acp.dispatch.resolved_working_directory());

    #[cfg(feature = "memory-sqlite")]
    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);

    #[cfg(feature = "memory-sqlite")]
    let memory_label = {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let initialized = store::ensure_session_store_ready(Some(sqlite_path), &memory_config)
            .map_err(|error| format!("failed to initialize sqlite memory: {error}"))?;
        initialized.display().to_string()
    };

    #[cfg(not(feature = "memory-sqlite"))]
    let memory_label = "disabled".to_owned();

    #[cfg(feature = "memory-sqlite")]
    let (session_id, session_origin) = resolve_or_create_cli_runtime_session_id(
        session_hint,
        session_requirement,
        &memory_config,
    )?;

    #[cfg(not(feature = "memory-sqlite"))]
    let (session_id, session_origin) =
        resolve_or_create_cli_runtime_session_id(session_hint, session_requirement, ())?;

    let (runtime, session) = build_runtime_session(&config, session_id.as_str())?;
    let legacy_tools = crate::conversation::DefaultLegacyToolDispatcher::with_config(
        Arc::clone(&runtime),
        &session,
        crate::session::store::session_store_config_from_memory_config(&config.memory),
        config.clone(),
    )?;
    let session_address = ConversationSessionAddress::from_session_id(session.session_id());
    Ok(CliTurnRuntime {
        resolved_path,
        config_present: true,
        config,
        session_origin,
        session_address,
        turn_coordinator: ConversationTurnCoordinator::new(),
        runtime,
        session,
        legacy_tools,
        effective_bootstrap_mcp_servers,
        effective_working_directory,
        memory_label,
        #[cfg(feature = "memory-sqlite")]
        memory_config,
    })
}

#[cfg(not(feature = "memory-sqlite"))]
fn resolve_or_create_cli_runtime_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
    _memory_store_unavailable: (),
) -> CliResult<(String, crate::chat::CliRuntimeSessionOrigin)> {
    let normalized = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (normalized, session_requirement) {
        (None, CliSessionRequirement::AllowImplicitDefault) => Err(
            "CLI startup session creation requires sqlite-backed memory; enable feature `memory-sqlite`".to_owned(),
        ),
        (None, CliSessionRequirement::RequireExplicit) => {
            Err("concurrent CLI host requires an explicit session id".to_owned())
        }
        (Some(session_id), _) if session_id == LATEST_SESSION_SELECTOR => Err(
            "CLI session selector `latest` requires sqlite-backed memory; enable feature `memory-sqlite`".to_owned(),
        ),
        (Some(session_id), _) => Err(format!(
            "CLI session `{session_id}` cannot be validated because sqlite-backed memory is disabled"
        )),
    }
}

#[cfg(all(test, not(feature = "memory-sqlite")))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolve_cli_runtime_session_id_rejects_implicit_startup_without_sqlite() {
        let error = match resolve_or_create_cli_runtime_session_id(
            None,
            CliSessionRequirement::AllowImplicitDefault,
            (),
        ) {
            Ok(_) => panic!("implicit startup should require sqlite-backed memory"),
            Err(error) => error,
        };

        assert!(
            error.contains("sqlite-backed memory"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolve_cli_runtime_session_id_rejects_latest_without_sqlite() {
        let error = match resolve_or_create_cli_runtime_session_id(
            Some(LATEST_SESSION_SELECTOR),
            CliSessionRequirement::RequireExplicit,
            (),
        ) {
            Ok(_) => panic!("latest selector should require sqlite-backed memory"),
            Err(error) => error,
        };

        assert!(error.contains("latest"), "unexpected error: {error}");
    }

    #[test]
    fn resolve_cli_runtime_session_id_rejects_literal_without_sqlite_validation() {
        let error = match resolve_or_create_cli_runtime_session_id(
            Some("missing-session"),
            CliSessionRequirement::AllowImplicitDefault,
            (),
        ) {
            Ok(_) => panic!("explicit literal should not be accepted without validation"),
            Err(error) => error,
        };

        assert!(
            error.contains("cannot be validated"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cli_runtime_bootstrap_rejects_implicit_startup_without_sqlite() {
        let config = LoongConfig::default();
        let runtime =
            crate::runtime::bootstrap_runtime_with_config(&config).expect("bootstrap test runtime");
        let result = initialize_cli_turn_runtime_with_loaded_config_and_runtime(
            PathBuf::from("/tmp/loong.toml"),
            config,
            None,
            &CliChatOptions::default(),
            runtime,
            "cli-runtime-no-sqlite",
            CliSessionRequirement::AllowImplicitDefault,
        );

        let error = match result {
            Ok(_) => panic!("bootstrap path should reject implicit startup without sqlite"),
            Err(error) => error,
        };

        assert!(
            error.contains("sqlite-backed memory"),
            "unexpected error: {error}"
        );
    }
}

#[cfg(feature = "memory-sqlite")]
fn resolve_or_create_cli_runtime_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
    memory_config: &SessionStoreConfig,
) -> CliResult<(String, crate::chat::CliRuntimeSessionOrigin)> {
    let normalized = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (normalized, session_requirement) {
        (None, CliSessionRequirement::AllowImplicitDefault) => {
            create_cli_startup_root_session(memory_config).map(|session_id| {
                (
                    session_id,
                    crate::chat::CliRuntimeSessionOrigin::CreatedThisRun,
                )
            })
        }
        (None, CliSessionRequirement::RequireExplicit) => {
            Err("concurrent CLI host requires an explicit session id".to_owned())
        }
        (Some(session_id), _) if session_id == LATEST_SESSION_SELECTOR => {
            resolve_latest_cli_session_id(memory_config)
                .map(|session_id| (session_id, crate::chat::CliRuntimeSessionOrigin::Existing))
        }
        (Some(session_id), _) => ensure_existing_cli_session_id(session_id, memory_config)
            .map(|session_id| (session_id, crate::chat::CliRuntimeSessionOrigin::Existing)),
    }
}

#[cfg(feature = "memory-sqlite")]
fn create_cli_startup_root_session(memory_config: &SessionStoreConfig) -> CliResult<String> {
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    let session_id = format!("cli-chat-{}", uuid::Uuid::new_v4().simple());
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: session_id.clone(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some(session_id.clone()),
        state: crate::session::repository::SessionState::Ready,
    })?;
    Ok(session_id)
}

#[cfg(feature = "memory-sqlite")]
fn resolve_latest_cli_session_id(memory_config: &SessionStoreConfig) -> CliResult<String> {
    let latest_session_id = latest_resumable_root_session_id(memory_config)?;
    latest_session_id.ok_or_else(|| {
        "CLI session selector `latest` did not find any resumable root session".to_owned()
    })
}

#[cfg(feature = "memory-sqlite")]
fn ensure_existing_cli_session_id(
    session_id: &str,
    memory_config: &SessionStoreConfig,
) -> CliResult<String> {
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    repo.load_session(session_id)?
        .ok_or_else(|| format!("CLI session `{session_id}` does not exist"))?;
    Ok(session_id.to_owned())
}
