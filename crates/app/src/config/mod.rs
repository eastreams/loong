mod channels;
mod provider;
mod runtime;
mod shared;
mod tools_memory;

#[allow(unused_imports)]
pub use channels::{CliChannelConfig, FeishuChannelConfig, TelegramChannelConfig};
#[allow(unused_imports)]
pub use provider::{ProviderConfig, ProviderKind, ReasoningEffort};
#[allow(unused_imports)]
pub use runtime::{
    default_loongclaw_home, load, write_template, ConversationConfig, ConversationTurnLoopConfig,
    LoongClawConfig,
};
#[allow(unused_imports)]
pub use tools_memory::{MemoryConfig, ToolConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

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
            kind: ProviderKind::Volcengine,
            endpoint: Some("https://example.volcengine.com/chat/completions".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://example.volcengine.com/chat/completions"
        );
    }

    #[test]
    fn provider_kinds_are_sorted_alphabetically() {
        let kinds = ProviderKind::all_sorted();
        let mut ids = Vec::new();
        for kind in kinds {
            ids.push(kind.as_str());
        }
        assert_eq!(
            ids,
            vec![
                "anthropic",
                "kimi",
                "minimax",
                "ollama",
                "openai",
                "openrouter",
                "volcengine",
                "xai",
                "zai",
                "zhipu"
            ]
        );
        let unique = ids.iter().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn endpoint_resolution_for_supported_provider_profiles_is_stable() {
        let cases = vec![
            (
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1/chat/completions",
            ),
            (
                ProviderKind::Kimi,
                "https://api.moonshot.cn/v1/chat/completions",
            ),
            (
                ProviderKind::Minimax,
                "https://api.minimaxi.com/v1/chat/completions",
            ),
            (
                ProviderKind::Ollama,
                "http://127.0.0.1:11434/v1/chat/completions",
            ),
            (
                ProviderKind::Openai,
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                ProviderKind::Openrouter,
                "https://openrouter.ai/api/v1/chat/completions",
            ),
            (
                ProviderKind::Volcengine,
                "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            ),
            (ProviderKind::Xai, "https://api.x.ai/v1/chat/completions"),
            (
                ProviderKind::Zai,
                "https://api.z.ai/api/paas/v4/chat/completions",
            ),
            (
                ProviderKind::Zhipu,
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            ),
        ];
        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.endpoint(), expected, "kind={kind:?}");
        }
    }

    #[test]
    fn switching_provider_kind_uses_profile_defaults() {
        let config = ProviderConfig {
            kind: ProviderKind::Openrouter,
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.endpoint(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            config.default_api_key_env().as_deref(),
            Some("OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn switching_provider_kind_keeps_profile_defaults_with_partial_template_edits() {
        let with_empty_path = ProviderConfig {
            kind: ProviderKind::Volcengine,
            chat_completions_path: String::new(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            with_empty_path.endpoint(),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );

        let with_empty_base = ProviderConfig {
            kind: ProviderKind::Volcengine,
            base_url: String::new(),
            ..ProviderConfig::default()
        };
        assert_eq!(
            with_empty_base.endpoint(),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn openai_codex_oauth_can_override_api_key_auth() {
        let config = ProviderConfig {
            kind: ProviderKind::Openai,
            oauth_access_token: Some("oauth-token".to_owned()),
            api_key: Some("api-key-should-not-win".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.default_oauth_access_token_env().as_deref(),
            Some("OPENAI_CODEX_OAUTH_TOKEN")
        );
        assert_eq!(
            config.authorization_header(),
            Some("Bearer oauth-token".to_owned())
        );
    }

    #[test]
    fn volcengine_coding_plan_oauth_can_override_api_key_auth() {
        let config = ProviderConfig {
            kind: ProviderKind::Volcengine,
            oauth_access_token: Some("vc-oauth-token".to_owned()),
            api_key: Some("api-key-should-not-win".to_owned()),
            ..ProviderConfig::default()
        };
        assert_eq!(
            config.default_oauth_access_token_env().as_deref(),
            Some("VOLCENGINE_CODING_PLAN_OAUTH_TOKEN")
        );
        assert_eq!(
            config.authorization_header(),
            Some("Bearer vc-oauth-token".to_owned())
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_legacy_volcengine_custom_alias() {
        let raw = r#"
[provider]
kind = "volcengine_custom"
model = "model-example"
"#;
        let parsed =
            toml::from_str::<LoongClawConfig>(raw).expect("parse legacy kind alias should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Volcengine);
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn provider_kind_keeps_legacy_compatible_aliases() {
        let raw = r#"
[provider]
kind = "xai_compatible"
model = "model-example"
"#;
        let parsed =
            toml::from_str::<LoongClawConfig>(raw).expect("parse compatible alias should pass");
        assert_eq!(parsed.provider.kind, ProviderKind::Xai);
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

    #[test]
    fn provider_default_model_uses_auto_discovery() {
        let config = ProviderConfig::default();
        assert_eq!(config.model, "auto");
        assert!(config.model_selection_requires_fetch());
    }

    #[test]
    fn turn_loop_policy_defaults_are_stable() {
        let config = LoongClawConfig::default();
        assert_eq!(config.conversation.turn_loop.max_rounds, 4);
        assert_eq!(config.conversation.turn_loop.max_tool_steps_per_round, 1);
        assert_eq!(
            config.conversation.turn_loop.max_repeated_tool_call_rounds,
            2
        );
        assert_eq!(config.conversation.turn_loop.max_ping_pong_cycles, 2);
        assert_eq!(
            config.conversation.turn_loop.max_same_tool_failure_rounds,
            3
        );
        assert_eq!(
            config
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars,
            8_000
        );
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn turn_loop_policy_can_be_overridden_from_toml() {
        let raw = r#"
[conversation.turn_loop]
max_rounds = 6
max_tool_steps_per_round = 3
max_repeated_tool_call_rounds = 5
max_ping_pong_cycles = 4
max_same_tool_failure_rounds = 7
max_followup_tool_payload_chars = 1200
"#;
        let parsed =
            toml::from_str::<LoongClawConfig>(raw).expect("parse turn-loop config should pass");
        assert_eq!(parsed.conversation.turn_loop.max_rounds, 6);
        assert_eq!(parsed.conversation.turn_loop.max_tool_steps_per_round, 3);
        assert_eq!(
            parsed.conversation.turn_loop.max_repeated_tool_call_rounds,
            5
        );
        assert_eq!(parsed.conversation.turn_loop.max_ping_pong_cycles, 4);
        assert_eq!(
            parsed.conversation.turn_loop.max_same_tool_failure_rounds,
            7
        );
        assert_eq!(
            parsed
                .conversation
                .turn_loop
                .max_followup_tool_payload_chars,
            1200
        );
    }

    #[test]
    fn models_endpoint_resolution_for_supported_provider_profiles_is_stable() {
        let cases = vec![
            (
                ProviderKind::Anthropic,
                "https://api.anthropic.com/v1/models",
            ),
            (ProviderKind::Kimi, "https://api.moonshot.cn/v1/models"),
            (ProviderKind::Minimax, "https://api.minimaxi.com/v1/models"),
            (ProviderKind::Ollama, "http://127.0.0.1:11434/v1/models"),
            (ProviderKind::Openai, "https://api.openai.com/v1/models"),
            (
                ProviderKind::Openrouter,
                "https://openrouter.ai/api/v1/models",
            ),
            (
                ProviderKind::Volcengine,
                "https://ark.cn-beijing.volces.com/api/v3/models",
            ),
            (ProviderKind::Xai, "https://api.x.ai/v1/models"),
            (ProviderKind::Zai, "https://api.z.ai/api/paas/v4/models"),
            (
                ProviderKind::Zhipu,
                "https://open.bigmodel.cn/api/paas/v4/models",
            ),
        ];
        for (kind, expected) in cases {
            let config = ProviderConfig {
                kind,
                ..ProviderConfig::default()
            };
            assert_eq!(config.models_endpoint(), expected, "kind={kind:?}");
        }
    }
}
