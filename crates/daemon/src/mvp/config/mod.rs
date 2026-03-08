mod channels;
mod provider;
mod runtime;
mod shared;
mod tools_memory;

#[allow(unused_imports)]
pub use channels::{CliChannelConfig, FeishuChannelConfig, TelegramChannelConfig};
#[allow(unused_imports)]
pub use provider::{ProviderConfig, ProviderKind};
#[allow(unused_imports)]
pub use runtime::{default_loongclaw_home, load, write_template, LoongClawConfig};
#[allow(unused_imports)]
pub use tools_memory::{MemoryConfig, ToolConfig};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_resolution_for_openai_compatible_is_stable() {
        let config = ProviderConfig {
            base_url: "https://api.openai.com/".to_owned(),
            chat_completions_path: "/v1/chat/completions".to_owned(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_resolution_for_volcengine_prefers_explicit_endpoint() {
        let config = ProviderConfig {
            kind: ProviderKind::VolcengineCustom,
            endpoint: Some("https://example.volcengine.com/chat/completions".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://example.volcengine.com/chat/completions"
        );
    }

    #[test]
    #[cfg(feature = "channel-telegram")]
    fn telegram_token_prefers_inline_secret() {
        let config = TelegramChannelConfig {
            bot_token: Some("inline-token".to_owned()),
            bot_token_env: Some("SHOULD_NOT_BE_READ".to_owned()),
            ..TelegramChannelConfig::default()
        };
        assert_eq!(config.bot_token().as_deref(), Some("inline-token"));
    }

    #[test]
    fn feishu_defaults_are_stable() {
        let config = FeishuChannelConfig::default();
        assert_eq!(config.base_url, "https://open.feishu.cn");
        assert_eq!(config.receive_id_type, "chat_id");
        assert_eq!(config.webhook_bind, "127.0.0.1:8080");
        assert_eq!(config.webhook_path, "/feishu/events");
        assert_eq!(
            config.encrypt_key_env.as_deref(),
            Some("FEISHU_ENCRYPT_KEY")
        );
        assert!(config.ignore_bot_messages);
    }

    #[test]
    fn provider_retry_defaults_are_stable() {
        let config = ProviderConfig::default();
        assert_eq!(config.request_timeout_ms, 30_000);
        assert_eq!(config.retry_max_attempts, 3);
        assert_eq!(config.retry_initial_backoff_ms, 300);
        assert_eq!(config.retry_max_backoff_ms, 3_000);
    }
}
