use std::path::PathBuf;

use crate::CliResult;
use crate::KernelContext;
use crate::config::LoongClawConfig;
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};
use crate::conversation::{ConversationSessionAddress, ConversationTurnCoordinator};

/// Self-contained TUI runtime, mirroring the fields of `CliTurnRuntime`
/// needed for turn execution without depending on `chat.rs` internals.
#[derive(Clone)]
pub(crate) struct TuiRuntime {
    pub(super) resolved_path: PathBuf,
    pub(super) config: LoongClawConfig,
    pub(super) session_id: String,
    pub(super) session_address: ConversationSessionAddress,
    pub(super) turn_coordinator: ConversationTurnCoordinator,
    pub(super) kernel_ctx: KernelContext,
    pub(super) model_label: String,
}

/// Initialize a self-contained TUI runtime from config path and optional
/// session hint.  This is the TUI equivalent of
/// `initialize_cli_turn_runtime` but carries only the state the TUI
/// actually needs, and does not reference any private `chat.rs` types.
pub(crate) fn initialize(
    config_path: Option<&str>,
    session_hint: Option<&str>,
) -> CliResult<TuiRuntime> {
    let (resolved_path, config) = crate::config::load(config_path)?;

    // Session ID: use hint or fall back to "default".
    let session_id = session_hint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| "default".to_owned(), |s| s.to_owned());

    // Export runtime environment variables derived from config.
    crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));

    // Bootstrap kernel context (provider auth, capability token, etc.).
    let kernel_ctx = bootstrap_kernel_context_with_config("tui", DEFAULT_TOKEN_TTL_S, &config)?;

    let session_address = ConversationSessionAddress::from_session_id(session_id.clone());

    // Model label for the status bar — explicit model or "auto".
    let model_label = config
        .provider
        .resolved_model()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "auto".to_owned());

    Ok(TuiRuntime {
        resolved_path,
        config,
        session_id,
        session_address,
        turn_coordinator: ConversationTurnCoordinator::new(),
        kernel_ctx,
        model_label,
    })
}
