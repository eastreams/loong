#[cfg(feature = "channel-telegram")]
use std::time::Duration;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use async_trait::async_trait;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::context::{bootstrap_kernel_context_for_config, DEFAULT_TOKEN_TTL_S};
use crate::CliResult;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::KernelContext;

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::config::LoongClawConfig;
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use super::conversation::{ConversationTurnLoop, ProviderErrorMode, SessionContext};
#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
use crate::tools::runtime_tool_view_for_config;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelSendReceipt {
    pub channel: &'static str,
    pub target: String,
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
#[allow(dead_code)]
#[async_trait]
pub trait ChannelAdapter {
    fn name(&self) -> &str;
    async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>>;
    async fn send_text(&self, target: &str, text: &str) -> CliResult<()>;
}

#[allow(clippy::print_stdout)] // CLI startup banner
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
        validate_telegram_security_config(&config)?;
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));
        let kernel_ctx =
            bootstrap_kernel_context_for_config("channel-telegram", DEFAULT_TOKEN_TTL_S, &config)?;

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

#[allow(clippy::print_stdout)] // CLI output
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
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));

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
        validate_feishu_security_config(&config)?;
        crate::runtime_env::initialize_runtime_environment(&config, Some(&resolved_path));
        let kernel_ctx =
            bootstrap_kernel_context_for_config("channel-feishu", DEFAULT_TOKEN_TTL_S, &config)?;

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
pub(crate) async fn send_text_to_known_session(
    config: &LoongClawConfig,
    session_id: &str,
    text: &str,
) -> CliResult<ChannelSendReceipt> {
    let (channel, raw_target) = session_id
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("sessions_send_channel_unsupported: `{session_id}`"))?;
    let target = raw_target.trim();
    if target.is_empty() {
        return Err(format!("sessions_send_channel_unsupported: `{session_id}`"));
    }

    match channel {
        "telegram" => {
            #[cfg(not(feature = "channel-telegram"))]
            {
                let _ = config;
                let _ = text;
                return Err(
                    "telegram channel is disabled (enable feature `channel-telegram`)".to_owned(),
                );
            }

            #[cfg(feature = "channel-telegram")]
            {
                if !config.telegram.enabled {
                    return Err(
                        "sessions_send_channel_disabled: telegram channel is disabled by config"
                            .to_owned(),
                    );
                }
                let chat_id = target.parse::<i64>().map_err(|error| {
                    format!("sessions_send_invalid_telegram_target: `{target}`: {error}")
                })?;
                if !config.telegram.allowed_chat_ids.contains(&chat_id) {
                    return Err(format!(
                        "sessions_send_target_not_allowed: telegram target `{chat_id}` is not present in telegram.allowed_chat_ids"
                    ));
                }
                let token = config.telegram.bot_token().ok_or_else(|| {
                    "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                })?;
                let adapter = telegram::TelegramAdapter::new(config, token);
                adapter.send_text(target, text).await?;
                return Ok(ChannelSendReceipt {
                    channel: "telegram",
                    target: chat_id.to_string(),
                });
            }
        }
        "feishu" => {
            #[cfg(not(feature = "channel-feishu"))]
            {
                let _ = config;
                let _ = text;
                return Err(
                    "feishu channel is disabled (enable feature `channel-feishu`)".to_owned(),
                );
            }

            #[cfg(feature = "channel-feishu")]
            {
                if !config.feishu.enabled {
                    return Err(
                        "sessions_send_channel_disabled: feishu channel is disabled by config"
                            .to_owned(),
                    );
                }
                if !config
                    .feishu
                    .allowed_chat_ids
                    .iter()
                    .any(|allowed| allowed.trim() == target)
                {
                    return Err(format!(
                        "sessions_send_target_not_allowed: feishu target `{target}` is not present in feishu.allowed_chat_ids"
                    ));
                }
                feishu::run_feishu_send(config, target, text, false).await?;
                return Ok(ChannelSendReceipt {
                    channel: "feishu",
                    target: target.to_owned(),
                });
            }
        }
        _ => Err(format!("sessions_send_channel_unsupported: `{session_id}`")),
    }
}

#[cfg(not(any(feature = "channel-telegram", feature = "channel-feishu")))]
pub(crate) async fn send_text_to_known_session(
    _config: &crate::config::LoongClawConfig,
    session_id: &str,
    _text: &str,
) -> CliResult<ChannelSendReceipt> {
    Err(format!("sessions_send_channel_unsupported: `{session_id}`"))
}

#[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
pub(super) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    message: &ChannelInboundMessage,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let session_context = SessionContext::root_with_tool_view(
        &message.session_id,
        runtime_tool_view_for_config(&config.tools),
    );
    ConversationTurnLoop::new()
        .handle_turn_in_session(
            config,
            &session_context,
            &message.text,
            ProviderErrorMode::Propagate,
            kernel_ctx,
        )
        .await
}

#[cfg(feature = "channel-telegram")]
fn validate_telegram_security_config(config: &LoongClawConfig) -> CliResult<()> {
    if config.telegram.allowed_chat_ids.is_empty() {
        return Err(
            "telegram.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "channel-feishu")]
fn validate_feishu_security_config(config: &LoongClawConfig) -> CliResult<()> {
    let has_allowlist = config
        .feishu
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "feishu.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }

    let has_verification_token = config
        .feishu
        .verification_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_verification_token {
        return Err(
            "feishu.verification_token is missing; configure token or verification_token_env"
                .to_owned(),
        );
    }

    let has_encrypt_key = config
        .feishu
        .encrypt_key()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_encrypt_key {
        return Err("feishu.encrypt_key is missing; configure key or encrypt_key_env".to_owned());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_requires_allowlist() {
        let config = LoongClawConfig::default();
        let error = validate_telegram_security_config(&config)
            .expect_err("empty allowlist must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_accepts_configured_allowlist() {
        let mut config = LoongClawConfig::default();
        config.telegram.allowed_chat_ids = vec![123_i64];
        assert!(validate_telegram_security_config(&config).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_requires_secrets_and_allowlist() {
        let config = LoongClawConfig::default();
        let error =
            validate_feishu_security_config(&config).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_complete_configuration() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token-123".to_owned());
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key = Some("encrypt-key-123".to_owned());
        config.feishu.encrypt_key_env = None;

        assert!(validate_feishu_security_config(&config).is_ok());
    }
}
