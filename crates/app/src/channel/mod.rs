#[cfg(test)]
use crate::config::LoongClawConfig;

pub(crate) mod access_policy;
mod catalog;
mod commands;
mod core;
#[cfg(feature = "channel-dingtalk")]
mod dingtalk;
#[cfg(feature = "channel-discord")]
mod discord;
#[cfg(feature = "channel-email")]
mod email;
#[cfg(feature = "feishu-integration")]
pub mod feishu;
#[cfg(feature = "channel-google-chat")]
mod google_chat;
mod http;
#[cfg(feature = "channel-imessage")]
mod imessage;
#[cfg(feature = "channel-irc")]
mod irc;
#[cfg(feature = "channel-line")]
mod line;
#[cfg(feature = "channel-matrix")]
mod matrix;
#[cfg(feature = "channel-mattermost")]
mod mattermost;
#[cfg(feature = "channel-nextcloud-talk")]
mod nextcloud_talk;
#[cfg(feature = "channel-nostr")]
mod nostr;
mod registry;
mod runtime;
pub(crate) mod sdk;
#[cfg(feature = "channel-signal")]
mod signal;
#[cfg(feature = "channel-signal")]
mod signal_command;
#[cfg(feature = "channel-slack")]
mod slack;
#[cfg(feature = "channel-synology-chat")]
mod synology_chat;
#[cfg(feature = "channel-teams")]
mod teams;
#[cfg(feature = "channel-telegram")]
mod telegram;
#[cfg(feature = "channel-tlon")]
mod tlon;
mod tlon_command;
/// Channel API traits for platform-agnostic abstraction
pub mod traits;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
#[cfg(feature = "channel-twitch")]
mod twitch;
#[cfg(feature = "channel-twitch")]
mod twitch_command;
#[cfg(feature = "channel-webhook")]
mod webhook;
#[cfg(feature = "channel-wecom")]
mod wecom;
#[cfg(feature = "channel-whatsapp")]
mod whatsapp;

#[cfg(feature = "channel-twitch")]
pub use self::twitch_command::run_twitch_send;
pub use registry::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCapability,
    ChannelCatalogCommandFamilyDescriptor, ChannelCatalogEntry, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCommandFamilyDescriptor,
    ChannelConfiguredAccountAccessPolicy, ChannelDiscoveredPluginBridge,
    ChannelDiscoveredPluginBridgeStatus, ChannelDoctorCheckSpec, ChannelDoctorCheckTrigger,
    ChannelDoctorOperationSpec, ChannelInventory, ChannelOnboardingDescriptor,
    ChannelOnboardingStrategy, ChannelOperationDescriptor, ChannelOperationHealth,
    ChannelOperationStatus, ChannelPluginBridgeContract, ChannelPluginBridgeDiscovery,
    ChannelPluginBridgeDiscoveryAmbiguityStatus, ChannelPluginBridgeDiscoveryStatus,
    ChannelPluginBridgeManifestStatus, ChannelPluginBridgeManifestValidation,
    ChannelPluginBridgeSelectionStatus, ChannelPluginBridgeStableTarget,
    ChannelRuntimeCommandDescriptor, ChannelStatusSnapshot, ChannelSurface,
    DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR, DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR, FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    FEISHU_COMMAND_FAMILY_DESCRIPTOR, FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
    GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR, IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR, LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR, MATRIX_COMMAND_FAMILY_DESCRIPTOR,
    MATRIX_RUNTIME_COMMAND_DESCRIPTOR, MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR, NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR, SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR, TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
    TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR, TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR, WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, WECOM_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_RUNTIME_COMMAND_DESCRIPTOR, WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    WHATSAPP_COMMAND_FAMILY_DESCRIPTOR, WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR,
    catalog_only_channel_entries, channel_inventory, channel_status_snapshots,
    list_channel_catalog, normalize_channel_catalog_id, normalize_channel_platform,
    resolve_channel_catalog_command_family_descriptor, resolve_channel_catalog_entry,
    resolve_channel_catalog_operation, resolve_channel_command_family_descriptor,
    resolve_channel_doctor_operation_spec, resolve_channel_onboarding_descriptor,
    resolve_channel_operation_descriptor, resolve_channel_runtime_command_descriptor,
    validate_plugin_channel_bridge_manifest,
};
pub use runtime::state::ChannelOperationRuntime;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub use runtime::turn_feedback::ChannelTurnFeedbackPolicy;
pub use sdk::{
    ChannelDescriptor, ChannelRuntimeKind, background_channel_runtime_descriptors,
    channel_descriptor, is_background_channel_surface_enabled, service_channel_descriptors,
};
pub(crate) use sdk::{collect_channel_validation_issues, enabled_channel_ids};
pub use tlon_command::run_tlon_send;

mod types;
pub use access_policy::{ChannelAccessRestrictionMode, ChannelInboundAccessPolicySummary};
pub use types::ChannelOutboundTargetKind as ChannelCatalogTargetKind;
pub use types::{
    ChannelAdapter, ChannelDelivery, ChannelDeliveryFeishuCallback, ChannelDeliveryResource,
    ChannelInboundMessage, ChannelOutboundDeliveryOptions, ChannelOutboundMessage,
    ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
    ChannelStreamingMode, FeishuChannelSendRequest,
};

pub use runtime::serve::ChannelServeStopHandle;
#[cfg(all(
    test,
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix",
        feature = "channel-wecom",
        feature = "channel-whatsapp"
    )
))]
use runtime::serve::{
    with_channel_serve_runtime_in_dir, with_channel_serve_runtime_with_stop_in_dir,
};

mod dispatch;
#[cfg(test)]
use crate::CliResult;
#[cfg(test)]
use crate::conversation::ConversationIngressPrivateContext;
#[cfg(test)]
use commands::context::render_channel_route_notice;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
pub(crate) use dispatch::process_inbound_with_provider;
#[cfg(all(test, feature = "config-toml"))]
use dispatch::reload_channel_turn_config;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
pub use dispatch::run_channel_serve_runtime_probe_for_test;
#[cfg(feature = "channel-feishu")]
pub use dispatch::run_feishu_channel_with_stop;
#[cfg(feature = "channel-matrix")]
pub use dispatch::run_matrix_channel_with_stop;
#[cfg(feature = "channel-telegram")]
pub use dispatch::run_telegram_channel_with_stop;
#[cfg(feature = "channel-wecom")]
pub use dispatch::run_wecom_channel_with_stop;
#[cfg(feature = "channel-whatsapp")]
pub use dispatch::run_whatsapp_channel_with_stop;
pub(crate) use dispatch::send_text_to_known_session;
#[cfg(all(test, feature = "channel-matrix"))]
use dispatch::validate_matrix_security_config;
#[cfg(all(test, feature = "channel-feishu"))]
use dispatch::{build_feishu_command_context, validate_feishu_security_config};
#[cfg(all(test, feature = "channel-telegram"))]
use dispatch::{build_telegram_command_context, validate_telegram_security_config};
#[cfg(all(
    test,
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix",
        feature = "channel-wecom",
        feature = "channel-whatsapp"
    )
))]
use dispatch::{channel_message_ingress_context, process_inbound_with_runtime_and_feedback};
pub use dispatch::{
    load_channel_operation_runtime_for_account_from_dir_for_test, run_background_channel_with_stop,
    run_dingtalk_send, run_discord_send, run_email_send, run_feishu_channel, run_feishu_send,
    run_google_chat_send, run_imessage_send, run_irc_send, run_line_send, run_matrix_channel,
    run_matrix_send, run_mattermost_send, run_nextcloud_talk_send, run_nostr_send, run_signal_send,
    run_slack_send, run_synology_chat_send, run_teams_send, run_telegram_channel,
    run_telegram_send, run_webhook_send, run_wecom_channel, run_wecom_send, run_whatsapp_channel,
    run_whatsapp_send,
};
#[cfg(all(
    test,
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix",
        feature = "channel-wecom",
        feature = "channel-whatsapp"
    )
))]
use runtime::serve::ChannelServeRuntimeSpec;
#[cfg(all(
    test,
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix",
        feature = "channel-wecom",
        feature = "channel-whatsapp"
    )
))]
use types::process_channel_batch;
#[cfg(all(
    test,
    any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix",
        feature = "channel-wecom"
    )
))]
use types::{KnownChannelSessionSendTarget, parse_known_channel_session_send_target};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_runtime_dir(suffix: &str) -> PathBuf {
        let unique = format!(
            "loongclaw-channel-mod-{suffix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn now_ms_for_test() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[derive(Default)]
    struct ChannelTraceRuntime {
        request_turn_calls: Arc<Mutex<usize>>,
        request_turn_kernel_bindings: Arc<Mutex<Vec<bool>>>,
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[async_trait]
    impl crate::conversation::ConversationRuntime for ChannelTraceRuntime {
        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: crate::conversation::ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            let mut messages = Vec::new();

            if include_system_prompt {
                let system_message = serde_json::json!({
                    "role": "system",
                    "content": "system",
                });
                messages.push(system_message);
            }

            let user_message = serde_json::json!({
                "role": "user",
                "content": "hello",
            });
            messages.push(user_message);

            Ok(messages)
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: crate::conversation::ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Err("request_completion should not be used in channel trace runtime tests".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            binding: crate::conversation::ConversationRuntimeBinding<'_>,
        ) -> CliResult<crate::conversation::ProviderTurn> {
            self.request_turn_kernel_bindings
                .lock()
                .expect("request turn kernel binding log")
                .push(binding.is_kernel_bound());

            let mut request_turn_calls = self
                .request_turn_calls
                .lock()
                .expect("request turn call count");
            *request_turn_calls += 1;
            let current_call = *request_turn_calls;
            drop(request_turn_calls);

            if current_call == 1 {
                let tool_intent = crate::conversation::ToolIntent {
                    tool_name: "tool.search".to_owned(),
                    args_json: serde_json::json!({
                        "query": "qzxwvvvjjjjkkk",
                    }),
                    source: "provider_test".to_owned(),
                    session_id: String::new(),
                    turn_id: String::new(),
                    tool_call_id: "call-1".to_owned(),
                };
                return Ok(crate::conversation::ProviderTurn {
                    assistant_text: String::new(),
                    tool_intents: vec![tool_intent],
                    raw_meta: Value::Null,
                });
            }

            Ok(crate::conversation::ProviderTurn {
                assistant_text: "final reply".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            })
        }

        async fn request_turn_streaming(
            &self,
            config: &LoongClawConfig,
            session_id: &str,
            turn_id: &str,
            messages: &[Value],
            tool_view: &crate::tools::ToolView,
            binding: crate::conversation::ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<crate::conversation::ProviderTurn> {
            self.request_turn(config, session_id, turn_id, messages, tool_view, binding)
                .await
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: crate::conversation::ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[derive(Default)]
    struct RecordingAdapter {
        sent: Arc<Mutex<Vec<(ChannelOutboundTarget, ChannelOutboundMessage)>>>,
        acked: Arc<Mutex<Vec<Option<String>>>>,
        completed_batches: Arc<Mutex<usize>>,
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
            Ok(Vec::new())
        }

        async fn send_message(
            &self,
            target: &ChannelOutboundTarget,
            message: &ChannelOutboundMessage,
        ) -> CliResult<()> {
            self.sent
                .lock()
                .expect("sent log")
                .push((target.clone(), message.clone()));
            Ok(())
        }

        async fn ack_inbound(&mut self, message: &ChannelInboundMessage) -> CliResult<()> {
            self.acked
                .lock()
                .expect("ack log")
                .push(message.delivery.ack_cursor.clone());
            Ok(())
        }

        async fn complete_batch(&mut self) -> CliResult<()> {
            *self
                .completed_batches
                .lock()
                .expect("completed batch count") += 1;
            Ok(())
        }
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn channel_adapter_default_ack_hooks_are_noop() {
        #[derive(Default)]
        struct NoopAdapter;

        #[async_trait]
        impl ChannelAdapter for NoopAdapter {
            fn name(&self) -> &str {
                "noop"
            }

            async fn receive_batch(&mut self) -> CliResult<Vec<ChannelInboundMessage>> {
                Ok(Vec::new())
            }

            async fn send_message(
                &self,
                _target: &ChannelOutboundTarget,
                _message: &ChannelOutboundMessage,
            ) -> CliResult<()> {
                Ok(())
            }
        }

        let mut adapter = NoopAdapter;
        let message = ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("2".to_owned()),
                source_message_id: Some("42".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        };

        adapter
            .ack_inbound(&message)
            .await
            .expect("default ack hook should succeed");
        adapter
            .complete_batch()
            .await
            .expect("default batch completion hook should succeed");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn process_channel_batch_acknowledges_after_successful_delivery() {
        let mut adapter = RecordingAdapter::default();
        let batch = vec![ChannelInboundMessage {
            session: ChannelSession::new(ChannelPlatform::Telegram, "1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: Some("101".to_owned()),
                source_message_id: Some("55".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        }];

        let had_messages = process_channel_batch(
            &mut adapter,
            batch,
            None,
            |message: ChannelInboundMessage, _turn_feedback_policy| {
                Box::pin(async move { Ok(format!("reply: {}", message.text)) })
            },
        )
        .await
        .expect("batch should process");

        assert!(had_messages);
        assert_eq!(
            adapter.sent.lock().expect("sent log").as_slice(),
            &[(
                ChannelOutboundTarget::telegram_chat(1),
                ChannelOutboundMessage::Text("reply: hello".to_owned()),
            )]
        );
        assert_eq!(
            adapter.acked.lock().expect("ack log").as_slice(),
            &[Some("101".to_owned())]
        );
        assert_eq!(*adapter.completed_batches.lock().expect("completed"), 1);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn process_inbound_with_runtime_and_feedback_appends_significant_trace() {
        let mut config = LoongClawConfig::default();
        config.provider.kind = crate::config::ProviderKind::Openai;
        config.telegram = serde_json::from_value(serde_json::json!({
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "ops-bot",
                    "bot_token": "test-token"
                }
            }
        }))
        .expect("deserialize telegram channel config");

        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(ChannelPlatform::Telegram, "ops-bot", "chat-1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("msg-1".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        };
        let runtime = ChannelTraceRuntime::default();
        let kernel_ctx = crate::context::bootstrap_test_kernel_context("channel-test", 60)
            .expect("bootstrap test kernel context");

        let reply = process_inbound_with_runtime_and_feedback(
            &config,
            &runtime,
            &message,
            crate::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
            ChannelTurnFeedbackPolicy::final_trace_significant(),
        )
        .await
        .expect("channel trace reply should succeed");

        assert!(reply.contains("final reply"));
        assert!(reply.contains("execution trace:"));
        assert!(
            reply.contains("tool.search completed: returned "),
            "tool search completion trace should include a summarized result count: {reply}"
        );

        let request_turn_calls = runtime
            .request_turn_calls
            .lock()
            .expect("request turn call count");
        assert_eq!(*request_turn_calls, 2);

        let request_turn_kernel_bindings = runtime
            .request_turn_kernel_bindings
            .lock()
            .expect("request turn kernel binding log");
        assert_eq!(request_turn_kernel_bindings.as_slice(), &[true, true]);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn process_inbound_with_runtime_and_feedback_can_disable_trace_rendering() {
        let mut config = LoongClawConfig::default();
        config.provider.kind = crate::config::ProviderKind::Openai;
        config.telegram = serde_json::from_value(serde_json::json!({
            "default_account": "Work Bot",
            "accounts": {
                "Work Bot": {
                    "account_id": "ops-bot",
                    "bot_token": "test-token"
                }
            }
        }))
        .expect("deserialize telegram channel config");

        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(ChannelPlatform::Telegram, "ops-bot", "chat-1"),
            reply_target: ChannelOutboundTarget::telegram_chat(1),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("msg-1".to_owned()),
                sender_principal_key: None,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        };
        let runtime = ChannelTraceRuntime::default();
        let kernel_ctx = crate::context::bootstrap_test_kernel_context("channel-test", 60)
            .expect("bootstrap test kernel context");

        let reply = process_inbound_with_runtime_and_feedback(
            &config,
            &runtime,
            &message,
            crate::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
            ChannelTurnFeedbackPolicy::disabled(),
        )
        .await
        .expect("channel reply should succeed when trace rendering is disabled");

        assert_eq!(reply, "final reply");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn channel_adapter_send_text_defaults_to_text_outbound_message() {
        let adapter = RecordingAdapter::default();
        let target = ChannelOutboundTarget::telegram_chat(1);

        adapter
            .send_text(&target, "hello default wrapper")
            .await
            .expect("default send_text wrapper should succeed");

        assert_eq!(
            adapter.sent.lock().expect("sent log").as_slice(),
            &[(
                ChannelOutboundTarget::telegram_chat(1),
                ChannelOutboundMessage::Text("hello default wrapper".to_owned()),
            )]
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_outbound_target_tracks_delivery_options() {
        let target = ChannelOutboundTarget::feishu_receive_id("ou_demo")
            .with_feishu_receive_id_type("open_id")
            .with_idempotency_key("send-uuid-1")
            .with_feishu_reply_in_thread(true);

        assert_eq!(target.feishu_receive_id_type(), Some("open_id"));
        assert_eq!(target.idempotency_key(), Some("send-uuid-1"));
        assert_eq!(target.feishu_reply_in_thread(), Some(true));
    }

    #[test]
    #[cfg(feature = "config-toml")]
    fn reload_channel_turn_config_refreshes_provider_state_without_mutating_channel_settings() {
        let path = std::env::temp_dir().join(format!(
            "loongclaw-channel-provider-reload-{}.toml",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let path_string = path.display().to_string();

        let mut in_memory = LoongClawConfig::default();
        in_memory.telegram.enabled = true;
        in_memory.telegram.allowed_chat_ids = vec![1001];
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        in_memory.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai,
            },
        );

        let mut on_disk = in_memory.clone();
        on_disk.telegram.allowed_chat_ids = vec![2002];
        let mut deepseek =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
        deepseek.model = "deepseek-chat".to_owned();
        on_disk.providers.insert(
            "deepseek-chat".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: deepseek.clone(),
            },
        );
        on_disk.provider = deepseek;
        on_disk.active_provider = Some("deepseek-chat".to_owned());
        crate::config::write(Some(&path_string), &on_disk, true).expect("write config fixture");

        let reloaded =
            reload_channel_turn_config(&in_memory, Some(path.as_path())).expect("reload");
        assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));
        assert_eq!(reloaded.provider.model, "deepseek-chat");
        assert_eq!(reloaded.telegram.allowed_chat_ids, vec![1001]);

        let _ = std::fs::remove_file(path);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_is_stable() {
        let session = ChannelSession::new(ChannelPlatform::Telegram, "123");
        assert_eq!(session.session_key(), "telegram:123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_thread_id_when_present() {
        let session = ChannelSession::with_thread(ChannelPlatform::Feishu, "oc_123", "om_thread_1");
        assert_eq!(session.session_key(), "feishu:oc_123:om_thread_1");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_identity_when_present() {
        let session = ChannelSession::with_account(ChannelPlatform::Telegram, "bot_123456", "123");
        assert_eq!(session.session_key(), "telegram:bot_123456:123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_configured_account_identity_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "feishu_shared", "oc_123")
                .with_configured_account_id("work");
        assert_eq!(
            session.session_key(),
            "feishu:cfg=work:feishu_shared:oc_123"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_participant_id_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_participant_id("ou_sender_1");
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:ou_sender_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_identity_and_thread_when_present() {
        let session = ChannelSession::with_account_and_thread(
            ChannelPlatform::Feishu,
            "lark_cli_a1b2c3",
            "oc_123",
            "om_thread_1",
        );
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:om_thread_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_session_key_includes_account_participant_and_thread_when_present() {
        let session =
            ChannelSession::with_account(ChannelPlatform::Feishu, "lark_cli_a1b2c3", "oc_123")
                .with_participant_id("ou_sender_1")
                .with_thread_id("om_root_1");
        assert_eq!(
            session.session_key(),
            "feishu:lark_cli_a1b2c3:oc_123:ou_sender_1:om_root_1"
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_message_ingress_context_preserves_sender_and_thread_metadata() {
        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(
                ChannelPlatform::Feishu,
                "lark_cli_a1b2c3",
                "oc_123",
            )
            .with_configured_account_id("work")
            .with_participant_id("ou_sender_1")
            .with_thread_id("om_root_1"),
            reply_target: ChannelOutboundTarget::feishu_message_reply("om_message_9"),
            text: "hello".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("om_message_9".to_owned()),
                sender_principal_key: Some("feishu:user:ou_sender_1".to_owned()),
                thread_root_id: Some("om_root_1".to_owned()),
                parent_message_id: Some("om_parent_1".to_owned()),
                resources: vec![
                    ChannelDeliveryResource {
                        resource_type: "image".to_owned(),
                        file_key: "img_v2_123".to_owned(),
                        file_name: None,
                    },
                    ChannelDeliveryResource {
                        resource_type: "file".to_owned(),
                        file_key: "file_v2_456".to_owned(),
                        file_name: Some("report.pdf".to_owned()),
                    },
                ],
                feishu_callback: None,
            },
        };

        let ingress = channel_message_ingress_context(&message).expect("ingress context");
        assert_eq!(ingress.channel.platform, "feishu");
        assert_eq!(
            ingress.channel.account_id.as_deref(),
            Some("lark_cli_a1b2c3")
        );
        assert_eq!(
            ingress.channel.configured_account_id.as_deref(),
            Some("work")
        );
        assert_eq!(ingress.channel.conversation_id, "oc_123");
        assert_eq!(
            ingress.channel.participant_id.as_deref(),
            Some("ou_sender_1")
        );
        assert_eq!(ingress.channel.thread_id.as_deref(), Some("om_root_1"));
        assert_eq!(
            ingress.delivery.sender_identity_key.as_deref(),
            Some("feishu:user:ou_sender_1")
        );
        assert_eq!(
            ingress.delivery.thread_root_id.as_deref(),
            Some("om_root_1")
        );
        assert_eq!(
            ingress.delivery.parent_message_id.as_deref(),
            Some("om_parent_1")
        );
        assert_eq!(ingress.delivery.resources.len(), 2);
        assert_eq!(ingress.delivery.resources[0].resource_type, "image");
        assert_eq!(ingress.delivery.resources[0].file_key, "img_v2_123");
        assert_eq!(ingress.delivery.resources[1].resource_type, "file");
        assert_eq!(ingress.delivery.resources[1].file_key, "file_v2_456");
        assert_eq!(
            ingress.delivery.resources[1].file_name.as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            ingress.private,
            ConversationIngressPrivateContext::default()
        );
    }

    #[cfg(any(feature = "channel-telegram", feature = "channel-feishu"))]
    #[test]
    fn channel_message_ingress_context_preserves_private_feishu_callback_metadata() {
        let message = ChannelInboundMessage {
            session: ChannelSession::with_account(
                ChannelPlatform::Feishu,
                "feishu_main",
                "oc_demo",
            )
            .with_configured_account_id("work")
            .with_participant_id("ou_operator_1")
            .with_thread_id("om_callback_1"),
            reply_target: ChannelOutboundTarget::feishu_message_reply("om_callback_1"),
            text: "[feishu_card_callback]".to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id: Some("om_callback_1".to_owned()),
                sender_principal_key: Some("feishu:user:ou_operator_1".to_owned()),
                thread_root_id: Some("om_callback_1".to_owned()),
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: Some(ChannelDeliveryFeishuCallback {
                    callback_token: Some("callback-secret-2".to_owned()),
                    open_message_id: Some("om_callback_1".to_owned()),
                    open_chat_id: Some("oc_demo".to_owned()),
                    operator_open_id: Some("ou_operator_1".to_owned()),
                    deferred_context_id: Some("evt_callback_2".to_owned()),
                }),
            },
        };

        let ingress = channel_message_ingress_context(&message).expect("ingress context");

        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.callback_token.as_deref()),
            Some("callback-secret-2")
        );
        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.operator_open_id.as_deref()),
            Some("ou_operator_1")
        );
        assert_eq!(
            ingress
                .private
                .feishu_callback
                .as_ref()
                .and_then(|value| value.deferred_context_id.as_deref()),
            Some("evt_callback_2")
        );
        assert!(
            !ingress
                .as_event_payload()
                .to_string()
                .contains("callback-secret-2")
        );
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn channel_outbound_target_preserves_platform_kind_and_id() {
        let target = ChannelOutboundTarget::new(
            ChannelPlatform::Feishu,
            ChannelOutboundTargetKind::MessageReply,
            "om_123",
        );
        assert_eq!(target.platform, ChannelPlatform::Feishu);
        assert_eq!(target.kind, ChannelOutboundTargetKind::MessageReply);
        assert_eq!(target.id, "om_123");
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn render_channel_route_notice_warns_on_implicit_multi_account_fallback() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: None,
            configured_account_count: 2,
            selected_configured_account_id: "alerts".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        let rendered =
            render_channel_route_notice("telegram", &route).expect("fallback route should warn");

        assert!(rendered.contains("telegram"));
        assert!(rendered.contains("alerts"));
        assert!(rendered.contains("--account"));
        assert!(rendered.contains("telegram.default_account"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn render_channel_route_notice_normalizes_hyphenated_config_keys() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: None,
            configured_account_count: 2,
            selected_configured_account_id: "alerts".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        let rendered =
            render_channel_route_notice("google-chat", &route).expect("fallback route should warn");

        assert!(rendered.contains("google-chat"));
        assert!(rendered.contains("google_chat.default_account"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[test]
    fn render_channel_route_notice_is_silent_for_explicit_account_selection() {
        let route = crate::config::ChannelResolvedAccountRoute {
            requested_account_id: Some("work".to_owned()),
            configured_account_count: 2,
            selected_configured_account_id: "work".to_owned(),
            default_account_source: crate::config::ChannelDefaultAccountSelectionSource::Fallback,
        };

        assert!(render_channel_route_notice("telegram", &route).is_none());
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_command_context_preserves_route_metadata() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "bot_token": "123456:work-token",
                        "allowed_chat_ids": [1001]
                    },
                    "Alerts": {
                        "bot_token": "654321:alerts-token",
                        "allowed_chat_ids": [2002]
                    }
                }
            }
        }))
        .expect("deserialize telegram context config");

        let context =
            build_telegram_command_context(PathBuf::from("/tmp/loongclaw.toml"), config, None)
                .expect("build telegram command context");

        assert_eq!(context.resolved_path, PathBuf::from("/tmp/loongclaw.toml"));
        assert_eq!(context.resolved.configured_account_id, "alerts");
        assert!(context.route.selected_by_default());
        assert!(context.route.uses_implicit_fallback_default());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_rejects_disabled_resolved_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Primary": {
                        "enabled": false,
                        "app_id": "cli_primary",
                        "app_secret": "secret"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let error = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("Primary"),
        )
        .expect_err("disabled feishu account should fail");

        assert!(error.contains("disabled"));
        assert!(error.contains("primary"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_accepts_unique_runtime_account_alias() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_work",
                        "app_secret": "secret"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let context = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("feishu_shared"),
        )
        .expect("unique runtime account alias should resolve");

        assert_eq!(context.resolved.configured_account_id, "work");
        assert_eq!(context.resolved.account.id, "feishu_shared");
        assert_eq!(
            context.route.requested_account_id.as_deref(),
            Some("feishu_shared")
        );
        assert!(!context.route.selected_by_default());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_command_context_reports_ambiguous_runtime_account_alias() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "feishu": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_work",
                        "app_secret": "secret-work"
                    },
                    "Alerts": {
                        "account_id": "feishu_shared",
                        "app_id": "cli_alerts",
                        "app_secret": "secret-alerts"
                    }
                }
            }
        }))
        .expect("deserialize feishu context config");

        let error = build_feishu_command_context(
            PathBuf::from("/tmp/loongclaw.toml"),
            config,
            Some("feishu_shared"),
        )
        .expect_err("ambiguous runtime account alias should fail");

        let error = error.to_ascii_lowercase();
        assert!(error.contains("requested feishu runtime account `feishu_shared` is ambiguous"));
        assert!(error.contains("work"));
        assert!(error.contains("alerts"));
        assert!(error.contains("use configured_account_id `alerts` or `work` to disambiguate"));
        assert!(error.contains("--account"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_tracks_running_state_and_shutdown() {
        let runtime_dir = temp_runtime_dir("serve-runtime-wrapper");
        let runtime_dir_for_body = runtime_dir.clone();
        let operation = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "bot_123456",
            account_label: "bot:123456",
        };

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            operation,
            |runtime| async move {
                let live = runtime::state::load_channel_operation_runtime_for_account_from_dir(
                    runtime_dir_for_body.as_path(),
                    ChannelPlatform::Telegram,
                    "serve",
                    "bot_123456",
                    0,
                )
                .expect("runtime should exist while serve body is running");
                assert!(live.running);
                assert_eq!(live.pid, Some(9191));
                assert_eq!(live.account_id.as_deref(), Some("bot_123456"));
                assert_eq!(live.account_label.as_deref(), Some("bot:123456"));
                assert!(
                    Arc::strong_count(&runtime) >= 1,
                    "runtime handle should stay alive in serve body"
                );
                Ok::<_, String>("ok".to_owned())
            },
        )
        .await
        .expect("serve runtime wrapper should succeed");

        assert_eq!(result, "ok");

        let finished = runtime::state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            0,
        )
        .expect("runtime should remain readable after shutdown");
        assert!(!finished.running);
        assert_eq!(finished.pid, Some(9191));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_shuts_down_cleanly_after_cooperative_stop() {
        let runtime_dir = temp_runtime_dir("serve-runtime-cooperative-stop");
        let runtime_dir_for_wrapper = runtime_dir.clone();
        let runtime_dir_for_body = runtime_dir.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let entered_for_body = entered.clone();
        let stop = ChannelServeStopHandle::new();
        let stop_for_body = stop.clone();
        let operation = ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: "bot_123456",
            account_label: "bot:123456",
        };

        let wrapper = tokio::spawn(async move {
            with_channel_serve_runtime_with_stop_in_dir(
                runtime_dir_for_wrapper.as_path(),
                9191,
                operation,
                stop_for_body,
                move |_runtime, stop| {
                    let runtime_dir_for_body = runtime_dir_for_body.clone();
                    async move {
                        let live =
                            runtime::state::load_channel_operation_runtime_for_account_from_dir(
                                runtime_dir_for_body.as_path(),
                                ChannelPlatform::Telegram,
                                "serve",
                                "bot_123456",
                                0,
                            )
                            .expect("runtime should exist while serve body is running");
                        assert!(live.running);
                        assert_eq!(live.pid, Some(9191));
                        entered_for_body.notify_one();
                        stop.wait().await;
                        Ok(())
                    }
                },
            )
            .await
        });

        entered.notified().await;
        stop.request_stop();

        wrapper
            .await
            .expect("cooperative stop wrapper join should succeed")
            .expect("cooperative stop wrapper should succeed");

        let finished = runtime::state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            0,
        )
        .expect("runtime should remain readable after shutdown");
        assert!(!finished.running);
        assert_eq!(finished.pid, Some(9191));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_rejects_duplicate_running_instance() {
        let runtime_dir = temp_runtime_dir("serve-runtime-duplicate");
        let now = now_ms_for_test();
        runtime::state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now),
            Some(now),
            Some(7001),
        )
        .expect("seed running runtime state");

        let error = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect_err("duplicate running instance should be rejected");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("bot_123456"));
        assert!(error.contains("7001"));
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_allows_takeover_when_previous_instance_is_stale() {
        let runtime_dir = temp_runtime_dir("serve-runtime-stale-takeover");
        let now = now_ms_for_test();
        runtime::state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            true,
            true,
            1,
            Some(now.saturating_sub(60_000)),
            Some(now.saturating_sub(60_000)),
            Some(7001),
        )
        .expect("seed stale runtime state");

        let result = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect("stale runtime should not block startup");

        assert_eq!(result, "ok");

        let runtime = runtime::state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            now_ms_for_test(),
        )
        .expect("runtime should remain readable after takeover");
        assert_eq!(runtime.pid, Some(9191));
        assert_eq!(runtime.instance_count, 1);
        assert_eq!(runtime.running_instances, 0);
        assert_eq!(runtime.stale_instances, 0);
        let entries = std::fs::read_dir(runtime_dir.as_path())
            .expect("list runtime dir after stale takeover")
            .map(|entry| {
                entry
                    .expect("runtime entry after stale takeover")
                    .file_name()
                    .into_string()
                    .expect("utf-8 runtime file name after stale takeover")
            })
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
    }

    #[cfg(any(
        feature = "channel-telegram",
        feature = "channel-feishu",
        feature = "channel-matrix"
    ))]
    #[tokio::test]
    async fn with_channel_serve_runtime_rejects_active_legacy_owner_after_inactive_account_prune() {
        let runtime_dir = temp_runtime_dir("serve-runtime-legacy-owner");
        let now = now_ms_for_test();
        runtime::state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            7001,
            false,
            false,
            0,
            Some(now.saturating_sub(5_000)),
            Some(now.saturating_sub(5_000)),
            Some(7001),
        )
        .expect("seed inactive account-scoped runtime state");
        runtime::state::write_runtime_state_for_test_with_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            8118,
            true,
            true,
            1,
            Some(now),
            Some(now),
            Some(8118),
        )
        .expect("seed active legacy runtime state");

        let error = with_channel_serve_runtime_in_dir(
            runtime_dir.as_path(),
            9191,
            ChannelServeRuntimeSpec {
                platform: ChannelPlatform::Telegram,
                operation_id: CHANNEL_OPERATION_SERVE_ID,
                account_id: "bot_123456",
                account_label: "bot:123456",
            },
            |_runtime| async move { Ok::<_, String>("ok".to_owned()) },
        )
        .await
        .expect_err("active legacy owner should still block startup");

        assert!(error.contains("already has an active serve runtime"));
        assert!(error.contains("8118"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_requires_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        let error = validate_telegram_security_config(&resolved)
            .expect_err("empty allowlist must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn telegram_security_validation_accepts_configured_allowlist() {
        let mut config = LoongClawConfig::default();
        config.telegram.allowed_chat_ids = vec![123_i64];
        let resolved = config
            .telegram
            .resolve_account(None)
            .expect("resolve telegram account");
        assert!(validate_telegram_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_requires_secrets_and_allowlist() {
        let config = LoongClawConfig::default();
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        let error =
            validate_feishu_security_config(&resolved).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_chat_ids"));
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_complete_configuration() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some(loongclaw_contracts::SecretRef::Inline(
            "token-123".to_owned(),
        ));
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key = Some(loongclaw_contracts::SecretRef::Inline(
            "encrypt-key-123".to_owned(),
        ));
        config.feishu.encrypt_key_env = None;

        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        assert!(validate_feishu_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-feishu")]
    #[test]
    fn feishu_security_validation_accepts_websocket_mode_without_webhook_secrets() {
        let mut config = LoongClawConfig::default();
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.mode = Some(crate::config::FeishuChannelServeMode::Websocket);

        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve feishu account");
        assert!(validate_feishu_security_config(&resolved).is_ok());
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn channel_session_key_encodes_matrix_segments_with_colons() {
        let session = ChannelSession::with_account(
            ChannelPlatform::Matrix,
            "@ops-bot:example.org",
            "!ops:example.org",
        )
        .with_participant_id("@alice:example.org")
        .with_thread_id("$event:example.org");

        assert_eq!(
            session.session_key(),
            "matrix:~b64~QG9wcy1ib3Q6ZXhhbXBsZS5vcmc:~b64~IW9wczpleGFtcGxlLm9yZw:~b64~QGFsaWNlOmV4YW1wbGUub3Jn:~b64~JGV2ZW50OmV4YW1wbGUub3Jn"
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn parse_known_channel_session_send_target_decodes_matrix_route_segments() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "matrix": {
                "enabled": true,
                "accounts": {
                    "Ops": {
                        "account_id": "@ops-bot:example.org",
                        "access_token": "matrix-token",
                        "base_url": "https://matrix.example.org",
                        "allowed_room_ids": ["!ops:example.org"]
                    }
                }
            }
        }))
        .expect("deserialize matrix config");
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve default matrix account");
        let account_id = resolved.account.id;
        let session_id = ChannelSession::with_account(
            ChannelPlatform::Matrix,
            account_id.as_str(),
            "!ops:example.org",
        )
        .session_key();

        let parsed = parse_known_channel_session_send_target(&config, session_id.as_str())
            .expect("parse matrix session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Matrix {
                account_id: Some(account_id),
                room_id: "!ops:example.org".to_owned(),
            }
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn parse_known_channel_session_send_target_accepts_legacy_matrix_account_aliases() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "matrix": {
                "enabled": true,
                "accounts": {
                    "Ops": {
                        "account_id": "@ops-bot:example.org",
                        "access_token": "matrix-token",
                        "base_url": "https://matrix.example.org",
                        "allowed_room_ids": ["!ops:example.org"]
                    }
                }
            }
        }))
        .expect("deserialize matrix config");

        let parsed = parse_known_channel_session_send_target(
            &config,
            "matrix:~b64~QG9wcy1ib3Q6ZXhhbXBsZS5vcmc:~b64~IW9wczpleGFtcGxlLm9yZw",
        )
        .expect("parse matrix session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Matrix {
                account_id: Some("ops-bot-example-org".to_owned()),
                room_id: "!ops:example.org".to_owned(),
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_matches_normalized_runtime_account_identity() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "ops": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:telegram-test-token",
                        "allowed_chat_ids": [123]
                    }
                }
            }
        }))
        .expect("deserialize telegram config");

        let parsed = parse_known_channel_session_send_target(&config, "telegram:Ops-Bot:123")
            .expect("parse telegram session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: Some("ops-bot".to_owned()),
                chat_id: "123".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_treats_single_segment_telegram_scope_as_chat_id() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "bot_token": "123456:telegram-test-token",
                "allowed_chat_ids": [123]
            }
        }))
        .expect("deserialize telegram config");

        let parsed = parse_known_channel_session_send_target(&config, "telegram:123")
            .expect("parse telegram session send target");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: None,
                chat_id: "123".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-telegram")]
    #[test]
    fn parse_known_channel_session_send_target_honors_configured_account_marker() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "work",
                "accounts": {
                    "work": {
                        "bot_token": "123456:work-token",
                        "allowed_chat_ids": [123]
                    },
                    "alerts": {
                        "bot_token": "654321:alerts-token",
                        "allowed_chat_ids": [456]
                    }
                }
            }
        }))
        .expect("deserialize telegram multi-account config");

        let parsed =
            parse_known_channel_session_send_target(&config, "telegram:cfg=alerts:bot_654321:456")
                .expect("parse telegram session with configured marker");

        assert_eq!(
            parsed,
            KnownChannelSessionSendTarget::Telegram {
                account_id: Some("alerts".to_owned()),
                chat_id: "456".to_owned(),
                thread_id: None,
            }
        );
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_requires_room_allowlist_and_transport() {
        let config = LoongClawConfig::default();
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account");
        let error =
            validate_matrix_security_config(&resolved).expect_err("empty config must be rejected");
        assert!(error.contains("allowed_room_ids"));

        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with allowlist");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("base url and token are still required");
        assert!(error.contains("base_url"));
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_rejects_invalid_base_url() {
        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.user_id = Some("@ops-bot:example.org".to_owned());
        config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "matrix-token".to_owned(),
        ));
        config.matrix.base_url = Some("not a url".to_owned());

        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with invalid base url");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("invalid base url must be rejected");
        assert!(error.contains("invalid matrix base_url"));
    }

    #[cfg(feature = "channel-matrix")]
    #[test]
    fn matrix_security_validation_requires_user_id_when_ignoring_self_messages() {
        let mut config = LoongClawConfig::default();
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "matrix-token".to_owned(),
        ));
        config.matrix.base_url = Some("https://matrix.example.org".to_owned());
        config.matrix.ignore_self_messages = true;

        let resolved = config
            .matrix
            .resolve_account(None)
            .expect("resolve matrix account with self-filter enabled");
        let error = validate_matrix_security_config(&resolved)
            .expect_err("user_id is required when self-filtering is enabled");
        assert!(error.contains("matrix.user_id"));
    }

    #[test]
    fn channel_streaming_mode_default_is_off() {
        assert_eq!(ChannelStreamingMode::default(), ChannelStreamingMode::Off);
    }
}
