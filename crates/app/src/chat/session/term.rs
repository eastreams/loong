use super::*;

pub(super) fn terminal_surface_allowed(stdout_is_tty: bool, stdin_is_tty: bool) -> bool {
    if !stdout_is_tty {
        return false;
    }

    if !stdin_is_tty {
        return false;
    }

    true
}

pub(crate) fn interactive_terminal_surface_supported() -> bool {
    let stdout_is_tty = terminal_surface_supported();
    let stdin_is_tty = stdin_is_tty();

    terminal_surface_allowed(stdout_is_tty, stdin_is_tty)
}

pub(crate) async fn run_cli_chat_surface(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
) -> CliResult<()> {
    let runtime = initialize_cli_turn_runtime(config_path, session_hint, options, "cli-chat")?;
    let surface = ChatSessionSurface::new(runtime, options.clone())?;
    surface.run().await
}

pub(crate) fn run_concurrent_cli_host_surface(options: &ConcurrentCliHostOptions) -> CliResult<()> {
    reject_disabled_cli_channel(&options.config)?;
    let chat_options = CliChatOptions::default();
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        options.resolved_path.clone(),
        options.config.clone(),
        Some(options.session_id.as_str()),
        &chat_options,
        "cli-chat-concurrent",
        CliSessionRequirement::RequireExplicit,
        options.initialize_runtime_environment,
    )?;
    let host_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize concurrent CLI host runtime: {error}"))?;
    let surface = ChatSessionSurface::new(runtime, chat_options)?;
    host_runtime.block_on(async {
        surface
            .run_with_shutdown(Some(options.shutdown.clone()))
            .await
    })
}

pub(super) struct ChatSessionSurface {
    pub(super) runtime: CliTurnRuntime,
    pub(super) options: CliChatOptions,
    pub(super) term: Term,
    pub(super) state: Arc<Mutex<SurfaceState>>,
}

pub(super) fn sync_live_surface_snapshot(live: &mut LiveSurfaceModel) {
    let snapshot = build_cli_chat_live_surface_snapshot(&live.state);
    live.snapshot = snapshot;
}

pub(super) fn fallback_live_surface_snapshot() -> CliChatLiveSurfaceSnapshot {
    CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::Preparing,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
        first_token_latency_ms: None,
        draft_preview: None,
        tools: Vec::new(),
    }
}

pub(super) struct SurfaceGuard {
    pub(super) term: Term,
}

impl SurfaceGuard {
    pub(super) fn new(term: &Term) -> CliResult<Self> {
        term.write_str(ALT_SCREEN_ENTER)
            .map_err(|error| format!("failed to enter alternate screen: {error}"))?;
        term.hide_cursor()
            .map_err(|error| format!("failed to hide cursor: {error}"))?;
        term.clear_screen()
            .map_err(|error| format!("failed to clear screen: {error}"))?;
        Ok(Self { term: term.clone() })
    }
}

impl Drop for SurfaceGuard {
    fn drop(&mut self) {
        let _ = self.term.show_cursor();
        let _ = self
            .term
            .write_str(terminal_surface_restore_sequence().as_str());
        let _ = self.term.flush();
    }
}

pub(super) fn terminal_surface_restore_sequence() -> String {
    [
        BRACKETED_PASTE_DISABLE,
        CURSOR_KEYS_NORMAL,
        KEYPAD_NORMAL,
        ANSI_RESET,
        ALT_SCREEN_EXIT,
    ]
    .join("")
}
