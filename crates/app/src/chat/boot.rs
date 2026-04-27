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
/// config, permits implicit default-session resolution, exports runtime
/// environment variables, bootstraps a fresh kernel context, and delegates the
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
/// environment variables, bootstraps a fresh kernel context, and then delegates
/// the final session/memory assembly to
/// `initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx`.
///
/// Use the `_and_kernel_ctx` variant when the caller must reuse an existing
/// kernel authority—such as channel-triggered turns—rather than minting a new
/// token for the same logical operation.
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
    let runtime_kernel =
        crate::runtime_bridge::RuntimeKernelOwner::bootstrap(kernel_scope, &config)?;
    let kernel_ctx = runtime_kernel.cloned_kernel_context();
    initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx(
        resolved_path,
        config,
        session_hint,
        options,
        kernel_ctx,
        session_requirement,
    )
}

/// Final assembly step for CLI/chat turn state once config and kernel authority
/// are already available.
///
/// This helper resolves ACP defaults, prepares memory/sqlite state, derives the
/// effective session id/address, and constructs the `CliTurnRuntime`. It
/// deliberately does not mutate process environment variables or bootstrap a
/// new kernel context; callers use it when those concerns were already handled
/// by an outer runtime surface.
pub(crate) fn initialize_cli_turn_runtime_with_loaded_config_and_kernel_ctx(
    resolved_path: PathBuf,
    config: LoongConfig,
    session_hint: Option<&str>,
    options: &CliChatOptions,
    kernel_ctx: crate::KernelContext,
    session_requirement: CliSessionRequirement,
) -> CliResult<CliTurnRuntime> {
    let explicit_acp_request = options.requests_explicit_acp();
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
    let session_id =
        resolve_cli_runtime_session_id(session_hint, session_requirement, &memory_config)?;

    #[cfg(not(feature = "memory-sqlite"))]
    let session_id = resolve_cli_session_id(session_hint, session_requirement)?;

    let session_address = ConversationSessionAddress::from_session_id(session_id.clone());
    let runtime_kernel = crate::runtime_bridge::RuntimeKernelOwner::new(kernel_ctx);

    Ok(CliTurnRuntime {
        resolved_path,
        config,
        session_id,
        session_address,
        turn_coordinator: ConversationTurnCoordinator::new(),
        runtime_kernel,
        explicit_acp_request,
        effective_bootstrap_mcp_servers,
        effective_working_directory,
        memory_label,
        #[cfg(feature = "memory-sqlite")]
        memory_config,
    })
}

fn resolve_cli_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
) -> CliResult<String> {
    match session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(session_id) => Ok(session_id.to_owned()),
        None => match session_requirement {
            CliSessionRequirement::AllowImplicitDefault => Ok("default".to_owned()),
            CliSessionRequirement::RequireExplicit => {
                Err("concurrent CLI host requires an explicit session id".to_owned())
            }
        },
    }
}

#[cfg(feature = "memory-sqlite")]
fn resolve_cli_runtime_session_id(
    session_hint: Option<&str>,
    session_requirement: CliSessionRequirement,
    memory_config: &SessionStoreConfig,
) -> CliResult<String> {
    let session_id = resolve_cli_session_id(session_hint, session_requirement)?;
    let should_resolve_latest = session_requirement == CliSessionRequirement::AllowImplicitDefault
        && session_id == LATEST_SESSION_SELECTOR;

    if !should_resolve_latest {
        return Ok(session_id);
    }

    let latest_session_id = latest_resumable_root_session_id(memory_config)?;
    let latest_session_id = latest_session_id.ok_or_else(|| {
        "CLI session selector `latest` did not find any resumable root session".to_owned()
    })?;

    Ok(latest_session_id)
}
