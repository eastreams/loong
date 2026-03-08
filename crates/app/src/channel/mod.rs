#[cfg(feature = "channel-telegram")]
use std::time::Duration;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use async_trait::async_trait;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::context::{bootstrap_kernel_context, DEFAULT_TOKEN_TTL_S};
use crate::CliResult;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::KernelContext;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::config::LoongClawConfig;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::conversation::{ConversationOrchestrator, ProviderErrorMode};

#[cfg(feature = "channel-feishu")]
mod feishu;
#[cfg(feature = "channel-telegram")]
mod telegram;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[derive(Debug, Clone)]
pub struct ChannelInboundMessage {
    pub session_id: String,
    #[allow(dead_code)]
    pub reply_target: String,
    pub text: String,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>>;
    async fn send_text(&self, target: &str, text: &str) -> CliResult<()>;
}

pub async fn run_telegram_channel(config_path: Option<&str>, once: bool) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, once);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let (resolved_path, config) = super::config::load(config_path)?;
        if !config.telegram.enabled {
            return Err("telegram channel is disabled by config.telegram.enabled=false".to_owned());
        }
        apply_runtime_env(&config);
        let kernel_ctx = bootstrap_kernel_context("channel-telegram", DEFAULT_TOKEN_TTL_S)?;

        let token = config.telegram.bot_token().ok_or_else(|| {
            "telegram bot token missing (set telegram.bot_token or env)".to_owned()
        })?;
        let mut adapter = telegram::TelegramAdapter::new(&config, token);

        println!(
            "{} channel started (config={}, timeout={}s)",
            adapter.name(),
            resolved_path.display(),
            config.telegram.polling_timeout_s
        );

        loop {
            let batch = adapter.receive_batch().await?;
            if batch.is_empty() && once {
                break;
            }
            for message in batch {
                let reply =
                    process_inbound_with_provider(&config, &message, Some(&kernel_ctx)).await?;
                adapter.send_text(&message.reply_target, &reply).await?;
            }
            if once {
                break;
            }
            sleep(Duration::from_millis(250)).await;
        }
        Ok(())
    }
}

pub async fn run_feishu_send(
    config_path: Option<&str>,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, receive_id, text, as_card);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let (resolved_path, config) = super::config::load(config_path)?;
        if !config.feishu.enabled {
            return Err("feishu channel is disabled by config.feishu.enabled=false".to_owned());
        }
        apply_runtime_env(&config);

        feishu::run_feishu_send(&config, receive_id, text, as_card).await?;

        println!(
            "feishu message sent (config={}, receive_id_type={})",
            resolved_path.display(),
            config.feishu.receive_id_type
        );
        Ok(())
    }
}

pub async fn run_feishu_channel(
    config_path: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, bind_override, path_override);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let (resolved_path, config) = super::config::load(config_path)?;
        if !config.feishu.enabled {
            return Err("feishu channel is disabled by config.feishu.enabled=false".to_owned());
        }
        apply_runtime_env(&config);
        let kernel_ctx = bootstrap_kernel_context("channel-feishu", DEFAULT_TOKEN_TTL_S)?;

        feishu::run_feishu_channel(
            &config,
            &resolved_path,
            bind_override,
            path_override,
            kernel_ctx,
        )
        .await
    }
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    ConversationOrchestrator::new()
        .handle_turn(
            config,
            &message.session_id,
            &message.text,
            ProviderErrorMode::Propagate,
            kernel_ctx,
        )
        .await
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
fn apply_runtime_env(config: &LoongClawConfig) {
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
}
