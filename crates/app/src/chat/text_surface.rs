use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CliChatStartupSummary {
    pub(super) config_path: String,
    pub(super) memory_label: String,
    pub(super) session_id: String,
    pub(super) context_engine_id: String,
    pub(super) context_engine_source: String,
    pub(super) acp_enabled: bool,
    pub(super) dispatch_enabled: bool,
    pub(super) conversation_routing: String,
    pub(super) allowed_channels: Vec<String>,
    pub(super) acp_backend_id: String,
    pub(super) acp_backend_source: String,
    pub(super) explicit_acp_request: bool,
    pub(super) event_stream_enabled: bool,
    pub(super) bootstrap_mcp_servers: Vec<String>,
    pub(super) working_directory: Option<String>,
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_cli_chat_startup(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<()> {
    let summary = build_cli_chat_startup_summary(runtime, options)?;
    for line in render_cli_chat_startup_lines(&summary) {
        println!("{line}");
    }
    Ok(())
}

fn build_cli_chat_startup_summary(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<CliChatStartupSummary> {
    let context_engine_selection = resolve_context_engine_selection(&runtime.config);
    let acp_selection = resolve_acp_backend_selection(&runtime.config);
    Ok(CliChatStartupSummary {
        config_path: runtime.resolved_path.display().to_string(),
        memory_label: runtime.memory_label.clone(),
        session_id: runtime.session_id.clone(),
        context_engine_id: context_engine_selection.id.to_owned(),
        context_engine_source: context_engine_selection.source.as_str().to_owned(),
        acp_enabled: runtime.config.acp.enabled,
        dispatch_enabled: runtime.config.acp.dispatch_enabled(),
        conversation_routing: runtime
            .config
            .acp
            .dispatch
            .conversation_routing
            .as_str()
            .to_owned(),
        allowed_channels: runtime.config.acp.dispatch.allowed_channel_ids()?,
        acp_backend_id: acp_selection.id.to_owned(),
        acp_backend_source: acp_selection.source.as_str().to_owned(),
        explicit_acp_request: runtime.explicit_acp_request,
        event_stream_enabled: options.acp_event_stream,
        bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
        working_directory: runtime
            .effective_working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

pub(super) fn render_cli_chat_startup_lines(summary: &CliChatStartupSummary) -> Vec<String> {
    let render_width = detect_cli_chat_render_width();
    render_cli_chat_startup_lines_with_width(summary, render_width)
}

pub(super) fn render_cli_chat_missing_config_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_missing_config_screen_spec(onboard_hint);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn build_cli_chat_missing_config_screen_spec(onboard_hint: &str) -> TuiScreenSpec {
    let intro_lines = vec![
        "Welcome to LoongClaw!".to_owned(),
        "No configuration found for interactive chat.".to_owned(),
    ];
    let sections = vec![TuiSectionSpec::ActionGroup {
        title: Some("setup command".to_owned()),
        inline_title_when_wide: true,
        items: vec![TuiActionSpec {
            label: "start setup".to_owned(),
            command: onboard_hint.to_owned(),
        }],
    }];
    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: "run setup wizard".to_owned(),
            detail_lines: vec!["Create a config now and return to interactive chat.".to_owned()],
            recommended: true,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: "skip for now".to_owned(),
            detail_lines: vec!["Exit chat now and keep the setup command for later.".to_owned()],
            recommended: false,
        },
    ];
    let footer_lines = vec!["Press Enter to accept y.".to_owned()];

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("setup required".to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    }
}

pub(super) fn render_cli_chat_missing_config_decline_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_missing_config_decline_message_spec(onboard_hint);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_missing_config_decline_message_spec(onboard_hint: &str) -> TuiMessageSpec {
    let setup_hint = format!("You can run '{onboard_hint}' later to get started.");
    let sections = vec![
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("setup skipped".to_owned()),
            lines: vec![setup_hint],
        },
        TuiSectionSpec::ActionGroup {
            title: Some("start later".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "setup command".to_owned(),
                command: onboard_hint.to_owned(),
            }],
        },
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("setup required".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

pub(super) fn render_cli_chat_startup_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_startup_screen_spec(summary);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn build_cli_chat_startup_screen_spec(summary: &CliChatStartupSummary) -> TuiScreenSpec {
    let allowed_channels = if summary.allowed_channels.is_empty() {
        "-".to_owned()
    } else {
        summary.allowed_channels.join(",")
    };
    let runtime_line = format!(
        "ACP enabled={} dispatch_enabled={} routing={} backend={} ({}) allowed_channels={allowed_channels}",
        summary.acp_enabled,
        summary.dispatch_enabled,
        summary.conversation_routing,
        summary.acp_backend_id,
        summary.acp_backend_source,
    );
    let mut sections = vec![
        TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "first prompt".to_owned(),
                command: DEFAULT_FIRST_PROMPT.to_owned(),
            }],
        },
        TuiSectionSpec::Narrative {
            title: None,
            lines: vec!["- type your request, or use /help for commands".to_owned()],
        },
        TuiSectionSpec::KeyValues {
            title: Some("session details".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "session".to_owned(),
                    value: summary.session_id.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "config".to_owned(),
                    value: summary.config_path.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "memory".to_owned(),
                    value: summary.memory_label.clone(),
                },
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("runtime details".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "context engine".to_owned(),
                    value: format!(
                        "{} ({})",
                        summary.context_engine_id, summary.context_engine_source
                    ),
                },
                TuiKeyValueSpec::Plain {
                    key: "acp".to_owned(),
                    value: runtime_line,
                },
            ],
        },
    ];

    if summary.explicit_acp_request
        || summary.event_stream_enabled
        || !summary.bootstrap_mcp_servers.is_empty()
        || summary.working_directory.is_some()
    {
        let bootstrap_label = if summary.bootstrap_mcp_servers.is_empty() {
            "-".to_owned()
        } else {
            summary.bootstrap_mcp_servers.join(",")
        };
        let cwd_label = summary.working_directory.as_deref().unwrap_or("-");
        let override_lines = vec![
            format!("explicit request: {}", summary.explicit_acp_request),
            format!("event stream: {}", summary.event_stream_enabled),
            format!("bootstrap MCP servers: {bootstrap_label}"),
            format!("working directory: {cwd_label}"),
        ];
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("acp overrides".to_owned()),
            lines: override_lines,
        });
    }

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("chat ready".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

pub(super) fn render_cli_chat_help_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = build_cli_chat_help_message_spec();
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_help_message_spec() -> TuiMessageSpec {
    let command_items = vec![
        TuiKeyValueSpec::Plain {
            key: "/help".to_owned(),
            value: "show chat commands".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/history".to_owned(),
            value: "print the current session sliding window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/fast_lane_summary [limit]".to_owned(),
            value: "summarize fast-lane batch execution events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/safe_lane_summary [limit]".to_owned(),
            value: "summarize safe-lane runtime events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_summary [limit]".to_owned(),
            value: "summarize durable turn finalization state".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_repair".to_owned(),
            value: "repair durable turn finalization tail when safe".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/exit".to_owned(),
            value: "quit chat".to_owned(),
        },
    ];
    let note_lines = vec![
        "Type any non-command text to send a normal assistant turn.".to_owned(),
        "Use /history to inspect the active memory window when a reply feels off.".to_owned(),
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("commands".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("slash commands".to_owned()),
                items: command_items,
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("usage notes".to_owned()),
                lines: note_lines,
            },
        ],
        footer_lines: Vec::new(),
    }
}

pub(super) fn render_cli_chat_history_lines_with_width(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_history_message_spec(session_id, limit, history_lines);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_history_message_spec(
    session_id: &str,
    limit: usize,
    history_lines: &[String],
) -> TuiMessageSpec {
    let caption = format!("session={session_id} limit={limit}");
    let history_section = TuiSectionSpec::Narrative {
        title: Some("sliding window".to_owned()),
        lines: history_lines.to_vec(),
    };

    TuiMessageSpec {
        role: "history".to_owned(),
        caption: Some(caption),
        sections: vec![history_section],
        footer_lines: Vec::new(),
    }
}

pub(super) fn render_cli_chat_assistant_lines_with_width(
    assistant_text: &str,
    width: usize,
) -> Vec<String> {
    if let Some(screen_spec) = build_cli_chat_approval_screen_spec(assistant_text) {
        return render_tui_screen_spec(&screen_spec, width, false);
    }
    let message_spec = build_cli_chat_assistant_message_spec(assistant_text);
    render_tui_message_spec(&message_spec, width)
}

fn build_cli_chat_assistant_message_spec(assistant_text: &str) -> TuiMessageSpec {
    let sections = parse_cli_chat_markdown_sections(assistant_text);

    TuiMessageSpec {
        role: crate::config::CLI_COMMAND_NAME.to_owned(),
        caption: Some("reply".to_owned()),
        sections,
        footer_lines: Vec::new(),
    }
}

fn build_cli_chat_approval_screen_spec(assistant_text: &str) -> Option<TuiScreenSpec> {
    let parsed = parse_approval_prompt_view(assistant_text)?;
    let mut intro_lines = Vec::new();
    if let Some(preface) = parsed
        .preface
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        intro_lines.extend(preface.lines().map(|line| line.to_owned()));
    }

    let title = parsed.title();

    let mut sections = Vec::new();
    if let Some(reason) = parsed.reason.as_deref() {
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some(parsed.pause_reason_title()),
            lines: vec![reason.to_owned()],
        });
    }

    let mut kv_items = Vec::new();
    if let Some(tool_name) = parsed.tool_name.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.tool_label(),
            value: tool_name.to_owned(),
        });
    }
    if let Some(request_id) = parsed.request_id.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.request_id_label(),
            value: request_id.to_owned(),
        });
    }
    if !kv_items.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some(parsed.request_section_title()),
            items: kv_items,
        });
    }

    let choices = parsed
        .actions
        .iter()
        .map(|action| TuiChoiceSpec {
            key: action.numeric_alias.clone(),
            label: action.label.clone(),
            detail_lines: action.detail_lines.clone(),
            recommended: action.recommended,
        })
        .collect::<Vec<_>>();

    let footer_lines = if parsed.actions.is_empty() {
        Vec::new()
    } else if parsed.locale.is_cjk() {
        vec![
            format!("也可以直接回复：{}", parsed.action_commands_text()),
            format!("数字别名：{}", parsed.action_numeric_aliases_text()),
        ]
    } else {
        vec![
            format!("You can also reply with: {}", parsed.action_commands_text()),
            format!("Numeric aliases: {}", parsed.action_numeric_aliases_text()),
        ]
    };

    Some(TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(parsed.subtitle()),
        title,
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    })
}
