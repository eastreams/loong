pub(crate) mod acp;
pub(crate) mod core;
pub(crate) mod operator;
pub(crate) mod pairing;

pub use acp::*;
pub use core::*;
pub use operator::*;
pub use pairing::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;

    #[test]
    fn channel_inventory_read_model_includes_structured_channel_access_policies() {
        let mut config = app::config::LoongConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(loong_contracts::SecretRef::Inline("secret".to_owned()));
        config.feishu.allowed_chat_ids = vec!["*".to_owned()];
        config.feishu.allowed_sender_ids = vec!["ou_admin".to_owned()];

        let inventory = app::channel::channel_inventory(&config);
        let read_model = core::build_channel_inventory_read_model("/tmp/loong.toml", &inventory);
        let access_policy = read_model
            .channel_access_policies
            .iter()
            .find(|policy| policy.channel_id == "feishu")
            .expect("feishu access policy");

        assert_eq!(access_policy.conversation_config_key, "allowed_chat_ids");
        assert_eq!(access_policy.sender_config_key, "allowed_sender_ids");
        assert_eq!(
            access_policy.summary.conversation_mode,
            app::channel::ChannelAccessRestrictionMode::WildcardAllowlist
        );
        assert_eq!(
            access_policy.summary.allowed_conversations,
            vec!["*".to_owned()]
        );
        assert_eq!(
            access_policy.summary.allowed_senders,
            vec!["ou_admin".to_owned()]
        );
    }

    #[test]
    fn tool_surface_read_model_preserves_guidance_and_counts() {
        let surface = app::tools::ToolSurfaceState {
            surface_id: "read".to_owned(),
            prompt_snippet: "inspect files".to_owned(),
            usage_guidance: "prefer direct read before shell".to_owned(),
            tool_ids: vec!["read".to_owned(), "write".to_owned()],
        };

        let read_model = core::build_tool_surface_read_model(&surface);

        assert_eq!(read_model.surface_id, "read");
        assert_eq!(read_model.tool_count, 2);
        assert_eq!(read_model.visible_tool_names, vec!["read", "write"]);
        assert_eq!(read_model.tool_ids, vec!["read", "write"]);
        assert_eq!(read_model.usage_guidance, "prefer direct read before shell");
    }
}
