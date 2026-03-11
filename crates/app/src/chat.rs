use std::collections::BTreeSet;
use std::io::{self, Write};

use loongclaw_contracts::Capability;

use crate::context::{bootstrap_kernel_context, DEFAULT_TOKEN_TTL_S};
use crate::CliResult;

use super::config::{self, LoongClawConfig};
use super::conversation::{ConversationTurnLoop, ProviderErrorMode};
#[cfg(feature = "memory-sqlite")]
use super::memory;
#[cfg(feature = "memory-sqlite")]
use super::memory::runtime_config::MemoryRuntimeConfig;

#[allow(clippy::print_stdout)] // CLI REPL output
pub async fn run_cli_chat(config_path: Option<&str>, session_hint: Option<&str>) -> CliResult<()> {
    let (resolved_path, config) = config::load(config_path)?;
    if !config.cli.enabled {
        return Err("CLI channel is disabled by config.cli.enabled=false".to_owned());
    }

    export_runtime_env(&config);
    let kernel_ctx = bootstrap_kernel_context("cli-chat", DEFAULT_TOKEN_TTL_S)?;

    #[cfg(feature = "memory-sqlite")]
    let memory_config = MemoryRuntimeConfig {
        sqlite_path: Some(config.memory.resolved_sqlite_path()),
    };

    #[cfg(feature = "memory-sqlite")]
    {
        let sqlite_path = config.memory.resolved_sqlite_path();
        let initialized = memory::ensure_memory_db_ready(Some(sqlite_path.clone()), &memory_config)
            .map_err(|error| format!("failed to initialize sqlite memory: {error}"))?;
        println!(
            "loongclaw chat started (config={}, memory={})",
            resolved_path.display(),
            initialized.display()
        );
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        println!(
            "loongclaw chat started (config={}, memory=disabled)",
            resolved_path.display()
        );
    }

    let session_id = session_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();
    println!("session={session_id} (type /help for commands, /exit to quit)");
    let turn_loop = ConversationTurnLoop::new();

    loop {
        print!("you> ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;
        let mut line = String::new();
        let read = io::stdin()
            .read_line(&mut line)
            .map_err(|error| format!("read stdin failed: {error}"))?;
        if read == 0 {
            println!();
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if is_exit_command(&config, input) {
            break;
        }
        if input == "/help" {
            print_help();
            continue;
        }
        if input == "/history" {
            #[cfg(feature = "memory-sqlite")]
            print_history(
                &session_id,
                config.memory.sliding_window,
                Some(&kernel_ctx),
                &memory_config,
            )
            .await?;
            #[cfg(not(feature = "memory-sqlite"))]
            print_history(&session_id, config.memory.sliding_window, Some(&kernel_ctx)).await?;
            continue;
        }

        let assistant_text = turn_loop
            .handle_turn(
                &config,
                &session_id,
                input,
                ProviderErrorMode::InlineMessage,
                Some(&kernel_ctx),
            )
            .await?;

        println!("loongclaw> {assistant_text}");
    }

    println!("bye.");
    Ok(())
}

fn is_exit_command(config: &LoongClawConfig, input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    config
        .cli
        .exit_commands
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .any(|value| !value.is_empty() && value == lower)
}

#[allow(clippy::print_stdout)] // CLI output
fn print_help() {
    println!("/help    show this help");
    println!("/history print current session sliding window");
    println!("/exit    quit chat");
}

#[allow(clippy::print_stdout)] // CLI output
async fn print_history(
    session_id: &str,
    limit: usize,
    kernel_ctx: Option<&crate::KernelContext>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<()> {
    #[cfg(feature = "memory-sqlite")]
    {
        if let Some(ctx) = kernel_ctx {
            let request = memory::build_window_request(session_id, limit);
            let caps = BTreeSet::from([Capability::MemoryRead]);
            let outcome = ctx
                .kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("load history via kernel failed: {error}"))?;
            let turns = memory::decode_window_turns(&outcome.payload);
            if turns.is_empty() {
                println!("(no history yet)");
                return Ok(());
            }
            for turn in turns {
                println!(
                    "[{}] {}: {}",
                    turn.ts.unwrap_or_default(),
                    turn.role,
                    turn.content
                );
            }
            return Ok(());
        }

        let turns = memory::window_direct(session_id, limit, memory_config)
            .map_err(|error| format!("load history failed: {error}"))?;
        if turns.is_empty() {
            println!("(no history yet)");
            return Ok(());
        }
        for turn in turns {
            println!("[{}] {}: {}", turn.ts, turn.role, turn.content);
        }
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, kernel_ctx);
        println!("history unavailable: memory-sqlite feature disabled");
        Ok(())
    }
}

fn export_runtime_env(config: &LoongClawConfig) {
    std::env::set_var(
        "LOONGCLAW_SQLITE_PATH",
        config.memory.resolved_sqlite_path().display().to_string(),
    );
    std::env::set_var(
        "LOONGCLAW_SLIDING_WINDOW",
        config.memory.sliding_window.to_string(),
    );
    std::env::set_var(
        "LOONGCLAW_SHELL_ALLOWLIST",
        config.tools.shell_allowlist.join(","),
    );
    std::env::set_var(
        "LOONGCLAW_FILE_ROOT",
        config.tools.resolved_file_root().display().to_string(),
    );

    // Populate the typed tool runtime config so executors never hit env vars
    // on the hot path.  Ignore the error if already initialised (e.g. tests).
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig {
        shell_allowlist: config
            .tools
            .shell_allowlist
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect(),
        file_root: Some(config.tools.resolved_file_root()),
    };
    let _ = crate::tools::runtime_config::init_tool_runtime_config(tool_rt);

    // Populate the typed memory runtime config (same pattern as tool config).
    let memory_rt = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(config.memory.resolved_sqlite_path()),
    };
    let _ = crate::memory::runtime_config::init_memory_runtime_config(memory_rt);
}
