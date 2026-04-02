use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use serde_json::Map;
use serde_json::Value;

use crate::CliResult;
use crate::provider_credential_policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OnboardValueOrigin {
    CurrentSetup,
    DetectedStartingPoint,
    UserSelected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardWizardStep {
    Welcome,
    Authentication,
    RuntimeDefaults,
    Workspace,
    Protocols,
    EnvironmentCheck,
    ReviewAndWrite,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardOutcome {
    Success,
    SuccessWithWarnings,
    Blocked,
}

impl OnboardOutcome {
    pub const fn summary_label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::SuccessWithWarnings => "SuccessWithWarnings",
            Self::Blocked => "Blocked",
        }
    }

    pub const fn ready_label(self) -> &'static str {
        match self {
            Self::Success => "ready",
            Self::SuccessWithWarnings => "ready with warnings",
            Self::Blocked => "blocked after verification",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardInteractionMode {
    RichInteractive,
    PlainInteractive,
    NonInteractive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardWorkspaceDraft {
    pub sqlite_path: PathBuf,
    pub file_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardProtocolDraft {
    pub acp_enabled: bool,
    pub acp_backend: Option<String>,
    pub bootstrap_mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OnboardDraft {
    pub config: mvp::config::LoongClawConfig,
    pub output_path: PathBuf,
    pub origins: BTreeMap<&'static str, OnboardValueOrigin>,
    pub workspace: OnboardWorkspaceDraft,
    pub protocols: OnboardProtocolDraft,
}

impl OnboardDraft {
    pub const PROVIDER_CONFIG_KEY: &'static str = "provider.config";
    pub const PROVIDER_MODEL_KEY: &'static str = "provider.model";
    pub const PROVIDER_CREDENTIAL_KEY: &'static str = "provider.credential";
    pub const CLI_PROMPT_MODE_KEY: &'static str = "cli.prompt_mode";
    pub const CLI_PERSONALITY_KEY: &'static str = "cli.personality";
    pub const CLI_PROMPT_ADDENDUM_KEY: &'static str = "cli.prompt_addendum";
    pub const CLI_SYSTEM_PROMPT_KEY: &'static str = "cli.system_prompt";
    pub const CLI_ENABLED_KEY: &'static str = "cli.enabled";
    pub const MEMORY_PROFILE_KEY: &'static str = "memory.profile";
    pub const WEB_SEARCH_PROVIDER_KEY: &'static str = "tools.web_search.default_provider";
    pub const WEB_SEARCH_CREDENTIAL_KEY: &'static str = "tools.web_search.credential";
    pub const CHANNELS_ENABLED_KEY: &'static str = "channels.enabled";
    pub const CHANNELS_PAIRING_KEY: &'static str = "channels.pairing";
    pub const EXTERNAL_SKILLS_ENABLED_KEY: &'static str = "external_skills.enabled";
    pub const EXTERNAL_SKILLS_REQUIRE_APPROVAL_KEY: &'static str =
        "external_skills.require_download_approval";
    pub const EXTERNAL_SKILLS_AUTO_EXPOSE_KEY: &'static str =
        "external_skills.auto_expose_installed";
    pub const WORKSPACE_SQLITE_PATH_KEY: &'static str = "memory.sqlite_path";
    pub const WORKSPACE_FILE_ROOT_KEY: &'static str = "tools.file_root";
    pub const ACP_ENABLED_KEY: &'static str = "acp.enabled";
    pub const ACP_BACKEND_KEY: &'static str = "acp.backend";
    pub const ACP_BOOTSTRAP_MCP_SERVERS_KEY: &'static str = "acp.dispatch.bootstrap_mcp_servers";

    pub fn from_config(
        config: mvp::config::LoongClawConfig,
        output_path: PathBuf,
        initial_origin: Option<OnboardValueOrigin>,
    ) -> Self {
        let workspace = OnboardWorkspaceDraft {
            sqlite_path: config.memory.resolved_sqlite_path(),
            file_root: config.tools.resolved_file_root(),
        };
        let protocols = OnboardProtocolDraft {
            acp_enabled: config.acp.enabled,
            acp_backend: config.acp.backend.clone(),
            bootstrap_mcp_servers: config.acp.dispatch.bootstrap_mcp_servers.clone(),
        };
        let mut draft = Self {
            config,
            output_path,
            origins: BTreeMap::new(),
            workspace,
            protocols,
        };
        if let Some(origin) = initial_origin {
            draft.seed_origin(Self::WORKSPACE_SQLITE_PATH_KEY, origin);
            draft.seed_origin(Self::WORKSPACE_FILE_ROOT_KEY, origin);
            draft.seed_origin(Self::ACP_ENABLED_KEY, origin);
            draft.seed_origin(Self::ACP_BACKEND_KEY, origin);
            draft.seed_origin(Self::ACP_BOOTSTRAP_MCP_SERVERS_KEY, origin);
            draft.seed_origin(Self::PROVIDER_CONFIG_KEY, origin);
            draft.seed_origin(Self::PROVIDER_MODEL_KEY, origin);
            draft.seed_origin(Self::PROVIDER_CREDENTIAL_KEY, origin);
            draft.seed_origin(Self::CLI_PROMPT_MODE_KEY, origin);
            draft.seed_origin(Self::CLI_PERSONALITY_KEY, origin);
            draft.seed_origin(Self::CLI_PROMPT_ADDENDUM_KEY, origin);
            draft.seed_origin(Self::CLI_SYSTEM_PROMPT_KEY, origin);
            draft.seed_origin(Self::CLI_ENABLED_KEY, origin);
            draft.seed_origin(Self::MEMORY_PROFILE_KEY, origin);
            draft.seed_origin(Self::WEB_SEARCH_PROVIDER_KEY, origin);
            draft.seed_origin(Self::WEB_SEARCH_CREDENTIAL_KEY, origin);
            draft.seed_origin(Self::CHANNELS_ENABLED_KEY, origin);
            draft.seed_origin(Self::CHANNELS_PAIRING_KEY, origin);
            draft.seed_origin(Self::EXTERNAL_SKILLS_ENABLED_KEY, origin);
            draft.seed_origin(Self::EXTERNAL_SKILLS_REQUIRE_APPROVAL_KEY, origin);
            draft.seed_origin(Self::EXTERNAL_SKILLS_AUTO_EXPOSE_KEY, origin);
        }
        draft
    }

    pub fn origin_for(&self, key: &'static str) -> Option<OnboardValueOrigin> {
        self.origins.get(key).copied()
    }

    pub fn set_provider_config(&mut self, provider: mvp::config::ProviderConfig) {
        self.config.provider = provider;
        self.mark_user_selected(Self::PROVIDER_CONFIG_KEY);
    }

    pub fn set_provider_model(&mut self, model: String) {
        self.config.provider.model = model;
        self.mark_user_selected(Self::PROVIDER_MODEL_KEY);
    }

    pub fn set_provider_credential_env(&mut self, selected_api_key_env: String) {
        let selected_api_key_env = selected_api_key_env.trim();
        if selected_api_key_env.is_empty() {
            self.config.provider.clear_api_key_env_binding();
            self.config.provider.clear_oauth_access_token_env_binding();
        } else {
            self.config.provider.api_key = None;
            self.config.provider.oauth_access_token = None;
            match provider_credential_policy::selected_provider_credential_env_field(
                &self.config.provider,
                selected_api_key_env,
            ) {
                provider_credential_policy::ProviderCredentialEnvField::ApiKey => {
                    self.config.provider.clear_oauth_access_token_env_binding();
                    self.config
                        .provider
                        .set_api_key_env_binding(Some(selected_api_key_env.to_owned()));
                }
                provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken => {
                    self.config.provider.clear_api_key_env_binding();
                    self.config
                        .provider
                        .set_oauth_access_token_env_binding(Some(selected_api_key_env.to_owned()));
                }
            }
        }
        self.mark_user_selected(Self::PROVIDER_CREDENTIAL_KEY);
    }

    pub fn set_provider_oauth_access_token(&mut self, access_token: String) {
        let trimmed_access_token = access_token.trim();
        if trimmed_access_token.is_empty() {
            self.config.provider.oauth_access_token = None;
            self.config.provider.clear_oauth_access_token_env_binding();
            self.mark_user_selected(Self::PROVIDER_CREDENTIAL_KEY);
            return;
        }

        self.config.provider.api_key = None;
        self.config.provider.clear_api_key_env_binding();
        self.config.provider.clear_oauth_access_token_env_binding();
        self.config.provider.oauth_access_token =
            Some(SecretRef::Inline(trimmed_access_token.to_owned()));
        self.mark_user_selected(Self::PROVIDER_CREDENTIAL_KEY);
    }

    pub fn set_provider_runtime_profiles(
        &mut self,
        profiles: std::collections::BTreeMap<String, mvp::config::ProviderProfileConfig>,
        active_profile_id: String,
    ) -> CliResult<()> {
        let active_profile_id = active_profile_id.trim().to_owned();
        if active_profile_id.is_empty() {
            return Err("active provider profile id cannot be empty".to_owned());
        }

        self.config.providers = profiles;
        self.config.active_provider = Some(active_profile_id.clone());
        self.config.last_provider = None;
        let selected_profile_id = self
            .config
            .switch_active_provider(active_profile_id.as_str())?;
        self.config.active_provider = Some(selected_profile_id);
        self.mark_user_selected(Self::PROVIDER_CONFIG_KEY);
        self.mark_user_selected(Self::PROVIDER_MODEL_KEY);
        self.mark_user_selected(Self::PROVIDER_CREDENTIAL_KEY);
        Ok(())
    }

    pub fn use_native_prompt_pack(
        &mut self,
        personality: mvp::prompt::PromptPersonality,
        prompt_addendum: Option<String>,
    ) {
        self.config.cli.prompt_pack_id = Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
        self.config.cli.personality = Some(personality);
        self.config.cli.system_prompt_addendum = prompt_addendum;
        self.config.cli.refresh_native_system_prompt();
        self.mark_user_selected(Self::CLI_PROMPT_MODE_KEY);
        self.mark_user_selected(Self::CLI_PERSONALITY_KEY);
        self.mark_user_selected(Self::CLI_PROMPT_ADDENDUM_KEY);
        self.mark_user_selected(Self::CLI_SYSTEM_PROMPT_KEY);
    }

    pub fn restore_built_in_prompt(&mut self) {
        self.config.cli.prompt_pack_id = Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
        self.config.cli.personality = Some(mvp::prompt::PromptPersonality::default());
        self.config.cli.system_prompt_addendum = None;
        self.config.cli.refresh_native_system_prompt();
        self.mark_user_selected(Self::CLI_PROMPT_MODE_KEY);
        self.mark_user_selected(Self::CLI_PERSONALITY_KEY);
        self.mark_user_selected(Self::CLI_PROMPT_ADDENDUM_KEY);
        self.mark_user_selected(Self::CLI_SYSTEM_PROMPT_KEY);
    }

    pub fn set_inline_system_prompt(&mut self, system_prompt: String) {
        self.config.cli.prompt_pack_id = Some(String::new());
        self.config.cli.personality = None;
        self.config.cli.system_prompt_addendum = None;
        self.config.cli.system_prompt = system_prompt;
        self.mark_user_selected(Self::CLI_PROMPT_MODE_KEY);
        self.mark_user_selected(Self::CLI_PERSONALITY_KEY);
        self.mark_user_selected(Self::CLI_PROMPT_ADDENDUM_KEY);
        self.mark_user_selected(Self::CLI_SYSTEM_PROMPT_KEY);
    }

    pub fn set_cli_enabled(&mut self, enabled: bool) {
        self.config.cli.enabled = enabled;
        self.mark_user_selected(Self::CLI_ENABLED_KEY);
    }

    pub fn set_memory_profile(&mut self, profile: mvp::config::MemoryProfile) {
        self.config.memory.profile = profile;
        self.mark_user_selected(Self::MEMORY_PROFILE_KEY);
    }

    pub fn set_web_search_default_provider(&mut self, provider: String) {
        self.config.tools.web_search.default_provider = provider;
        self.mark_user_selected(Self::WEB_SEARCH_PROVIDER_KEY);
    }

    pub fn clear_web_search_credential(&mut self, provider: &str) {
        if self.set_web_search_credential_value(provider, None) {
            self.mark_user_selected(Self::WEB_SEARCH_CREDENTIAL_KEY);
        }
    }

    pub fn set_web_search_credential_env(&mut self, provider: &str, env_name: String) {
        if self.set_web_search_credential_value(provider, Some(format!("${{{}}}", env_name.trim())))
        {
            self.mark_user_selected(Self::WEB_SEARCH_CREDENTIAL_KEY);
        }
    }

    pub fn set_workspace_sqlite_path(&mut self, sqlite_path: PathBuf) {
        self.workspace.sqlite_path = sqlite_path.clone();
        self.config.memory.sqlite_path = sqlite_path.display().to_string();
        self.mark_user_selected(Self::WORKSPACE_SQLITE_PATH_KEY);
    }

    pub fn set_workspace_file_root(&mut self, file_root: PathBuf) {
        self.workspace.file_root = file_root.clone();
        self.config.tools.file_root = Some(file_root.display().to_string());
        self.mark_user_selected(Self::WORKSPACE_FILE_ROOT_KEY);
    }

    pub fn set_enabled_service_channels<I>(&mut self, channel_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        let normalized_ids = normalize_selected_service_channel_ids(channel_ids);
        let _ = set_selected_service_channels_in_config(&mut self.config, &normalized_ids);
        self.mark_user_selected(Self::CHANNELS_ENABLED_KEY);
    }

    pub fn set_channel_pairing_string_path(&mut self, path: &str, value: Option<String>) -> bool {
        let normalized_value = value
            .map(|raw_value| raw_value.trim().to_owned())
            .filter(|raw_value| !raw_value.is_empty());
        let updated = set_optional_string_path_in_config(&mut self.config, path, normalized_value);
        if updated {
            self.mark_user_selected(Self::CHANNELS_PAIRING_KEY);
        }
        updated
    }

    pub fn set_external_skills_runtime_enabled(&mut self, enabled: bool) {
        self.config.external_skills.enabled = enabled;
        self.config.external_skills.require_download_approval = true;
        self.config.external_skills.auto_expose_installed = false;
        self.mark_user_selected(Self::EXTERNAL_SKILLS_ENABLED_KEY);
        self.mark_user_selected(Self::EXTERNAL_SKILLS_REQUIRE_APPROVAL_KEY);
        self.mark_user_selected(Self::EXTERNAL_SKILLS_AUTO_EXPOSE_KEY);
    }

    pub fn set_acp_enabled(&mut self, enabled: bool) {
        self.protocols.acp_enabled = enabled;
        self.config.acp.enabled = enabled;
        self.mark_user_selected(Self::ACP_ENABLED_KEY);

        if !enabled {
            self.set_acp_backend(None);
            self.set_bootstrap_mcp_servers(Vec::new());
        }
    }

    pub fn set_acp_backend(&mut self, backend: Option<String>) {
        self.protocols.acp_backend = backend.clone();
        self.config.acp.backend = backend;
        self.mark_user_selected(Self::ACP_BACKEND_KEY);
    }

    pub fn set_bootstrap_mcp_servers(&mut self, bootstrap_mcp_servers: Vec<String>) {
        self.protocols.bootstrap_mcp_servers = bootstrap_mcp_servers.clone();
        self.config.acp.dispatch.bootstrap_mcp_servers = bootstrap_mcp_servers;
        self.mark_user_selected(Self::ACP_BOOTSTRAP_MCP_SERVERS_KEY);
    }

    fn seed_origin(&mut self, key: &'static str, origin: OnboardValueOrigin) {
        self.origins.insert(key, origin);
    }

    fn mark_user_selected(&mut self, key: &'static str) {
        self.seed_origin(key, OnboardValueOrigin::UserSelected);
    }

    fn set_web_search_credential_value(&mut self, provider: &str, value: Option<String>) -> bool {
        match provider {
            mvp::config::WEB_SEARCH_PROVIDER_BRAVE => {
                self.config.tools.web_search.brave_api_key = value;
                true
            }
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY => {
                self.config.tools.web_search.tavily_api_key = value;
                true
            }
            mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY => {
                self.config.tools.web_search.perplexity_api_key = value;
                true
            }
            mvp::config::WEB_SEARCH_PROVIDER_EXA => {
                self.config.tools.web_search.exa_api_key = value;
                true
            }
            mvp::config::WEB_SEARCH_PROVIDER_JINA => {
                self.config.tools.web_search.jina_api_key = value;
                true
            }
            _ => false,
        }
    }
}

fn normalize_selected_service_channel_ids<I>(channel_ids: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = String>,
{
    let supported_ids = mvp::config::service_channel_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.id.to_owned())
        .collect::<BTreeSet<_>>();
    let mut normalized_ids = BTreeSet::new();

    for raw_id in channel_ids {
        let trimmed_id = raw_id.trim();
        let channel_is_blank = trimmed_id.is_empty();
        if channel_is_blank {
            continue;
        }

        let normalized_id = trimmed_id.to_ascii_lowercase();
        let channel_is_supported = supported_ids.contains(normalized_id.as_str());
        if !channel_is_supported {
            continue;
        }

        normalized_ids.insert(normalized_id);
    }

    normalized_ids
}

fn set_selected_service_channels_in_config(
    config: &mut mvp::config::LoongClawConfig,
    selected_ids: &BTreeSet<String>,
) -> bool {
    let config_value_result = serde_json::to_value(&*config);
    let Ok(mut config_value) = config_value_result else {
        return false;
    };
    let Some(config_object) = config_value.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    let descriptors = mvp::config::service_channel_descriptors();

    for descriptor in descriptors {
        let field_name = descriptor.id.replace('-', "_");
        let should_enable = selected_ids.contains(descriptor.id);
        let field_changed =
            set_channel_enabled_flag_in_value(config_object, field_name.as_str(), should_enable);
        if field_changed {
            changed = true;
        }
    }

    if !changed {
        return false;
    }

    let next_config_result = serde_json::from_value(config_value);
    let Ok(next_config) = next_config_result else {
        return false;
    };
    *config = next_config;
    true
}

fn set_channel_enabled_flag_in_value(
    config_object: &mut Map<String, Value>,
    field_name: &str,
    enabled: bool,
) -> bool {
    let Some(channel_value) = config_object.get_mut(field_name) else {
        return false;
    };
    let Some(channel_object) = channel_value.as_object_mut() else {
        return false;
    };

    let current_enabled = channel_object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if current_enabled == enabled {
        return false;
    }

    let enabled_value = Value::Bool(enabled);
    channel_object.insert("enabled".to_owned(), enabled_value);
    true
}

fn set_optional_string_path_in_config(
    config: &mut mvp::config::LoongClawConfig,
    path: &str,
    value: Option<String>,
) -> bool {
    let config_value_result = serde_json::to_value(&*config);
    let Ok(mut config_value) = config_value_result else {
        return false;
    };
    let Some(config_object) = config_value.as_object_mut() else {
        return false;
    };

    let next_value = value.map(Value::String).unwrap_or(Value::Null);
    let changed = set_json_value_at_path(config_object, path, next_value);
    if !changed {
        return false;
    }

    let next_config_result = serde_json::from_value(config_value);
    let Ok(next_config) = next_config_result else {
        return false;
    };
    *config = next_config;
    true
}

fn set_json_value_at_path(
    config_object: &mut Map<String, Value>,
    path: &str,
    next_value: Value,
) -> bool {
    let mut path_segments = path
        .split('.')
        .filter(|segment| !segment.trim().is_empty())
        .peekable();
    let Some(first_segment) = path_segments.next() else {
        return false;
    };

    let current_value = config_object.get_mut(first_segment);
    let Some(current_value) = current_value else {
        return false;
    };

    let mut current_value = current_value;
    while let Some(segment) = path_segments.next() {
        let is_leaf_segment = path_segments.peek().is_none();
        if is_leaf_segment {
            let Some(current_object) = current_value.as_object_mut() else {
                return false;
            };
            let existing_value = current_object.get(segment);
            if existing_value.is_some_and(|existing_value| *existing_value == next_value) {
                return false;
            }
            current_object.insert(segment.to_owned(), next_value);
            return true;
        }

        let Some(current_object) = current_value.as_object_mut() else {
            return false;
        };
        let nested_value = current_object.get_mut(segment);
        let Some(nested_value) = nested_value else {
            return false;
        };
        current_value = nested_value;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use loongclaw_contracts::SecretRef;

    fn sample_config() -> mvp::config::LoongClawConfig {
        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = "/starting/memory.sqlite3".to_owned();
        config.tools.file_root = Some("/starting/workspace".to_owned());
        config.acp.enabled = false;
        config.acp.backend = Some("builtin".to_owned());
        config.acp.dispatch.bootstrap_mcp_servers = vec!["filesystem".to_owned()];
        config
    }

    #[test]
    fn draft_origin_tracking_distinguishes_current_detected_and_user_selected_values() {
        let current = OnboardDraft::from_config(
            sample_config(),
            PathBuf::from("/tmp/current.toml"),
            Some(OnboardValueOrigin::CurrentSetup),
        );
        assert_eq!(
            current.origin_for(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY),
            Some(OnboardValueOrigin::CurrentSetup)
        );
        assert_eq!(
            current.origin_for(OnboardDraft::WORKSPACE_FILE_ROOT_KEY),
            Some(OnboardValueOrigin::CurrentSetup)
        );

        let mut detected = OnboardDraft::from_config(
            sample_config(),
            PathBuf::from("/tmp/detected.toml"),
            Some(OnboardValueOrigin::DetectedStartingPoint),
        );
        detected.set_workspace_file_root(PathBuf::from("/user/workspace"));

        assert_eq!(
            detected.origin_for(OnboardDraft::WORKSPACE_SQLITE_PATH_KEY),
            Some(OnboardValueOrigin::DetectedStartingPoint)
        );
        assert_eq!(
            detected.origin_for(OnboardDraft::WORKSPACE_FILE_ROOT_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }

    #[test]
    fn native_prompt_pack_updates_effective_system_prompt_origin() {
        let mut draft = OnboardDraft::from_config(
            sample_config(),
            PathBuf::from("/tmp/native-prompt.toml"),
            Some(OnboardValueOrigin::CurrentSetup),
        );

        draft.use_native_prompt_pack(
            mvp::prompt::PromptPersonality::FriendlyCollab,
            Some("be concise".to_owned()),
        );

        assert_eq!(
            draft.origin_for(OnboardDraft::CLI_PROMPT_MODE_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::CLI_PERSONALITY_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::CLI_PROMPT_ADDENDUM_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::CLI_SYSTEM_PROMPT_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }

    #[test]
    fn unknown_web_search_provider_does_not_mark_credential_origin() {
        let mut draft =
            OnboardDraft::from_config(sample_config(), PathBuf::from("/tmp/current.toml"), None);

        draft.set_web_search_credential_env("unknown-provider", "IGNORED_KEY".to_owned());
        draft.clear_web_search_credential("unknown-provider");

        assert_eq!(
            draft.origin_for(OnboardDraft::WEB_SEARCH_CREDENTIAL_KEY),
            None
        );
    }

    #[test]
    fn service_channel_selection_updates_enabled_service_channel_ids_and_origin() {
        let mut draft =
            OnboardDraft::from_config(sample_config(), PathBuf::from("/tmp/current.toml"), None);

        draft.set_enabled_service_channels([
            "telegram".to_owned(),
            "wecom".to_owned(),
            "telegram".to_owned(),
        ]);

        assert_eq!(
            draft.config.enabled_service_channel_ids(),
            vec!["telegram".to_owned(), "wecom".to_owned()]
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::CHANNELS_ENABLED_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }

    #[test]
    fn service_channel_selection_clears_unselected_runtime_channels() {
        let mut config = sample_config();
        config.telegram.enabled = true;
        config.wecom.enabled = true;
        let mut draft = OnboardDraft::from_config(config, PathBuf::from("/tmp/current.toml"), None);

        draft.set_enabled_service_channels(["telegram".to_owned()]);

        assert_eq!(
            draft.config.enabled_service_channel_ids(),
            vec!["telegram".to_owned()]
        );
        assert!(draft.config.telegram.enabled);
        assert!(!draft.config.wecom.enabled);
    }

    #[test]
    fn cli_enabled_toggle_updates_config_and_origin() {
        let mut draft =
            OnboardDraft::from_config(sample_config(), PathBuf::from("/tmp/current.toml"), None);

        draft.set_cli_enabled(false);

        assert!(!draft.config.cli.enabled);
        assert_eq!(
            draft.origin_for(OnboardDraft::CLI_ENABLED_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }

    #[test]
    fn external_skills_runtime_toggle_applies_safe_policy_defaults_and_origin() {
        let mut config = sample_config();
        config.external_skills.enabled = false;
        config.external_skills.require_download_approval = false;
        config.external_skills.auto_expose_installed = true;
        let mut draft = OnboardDraft::from_config(config, PathBuf::from("/tmp/current.toml"), None);

        draft.set_external_skills_runtime_enabled(true);

        assert!(draft.config.external_skills.enabled);
        assert!(draft.config.external_skills.require_download_approval);
        assert!(!draft.config.external_skills.auto_expose_installed);
        assert_eq!(
            draft.origin_for(OnboardDraft::EXTERNAL_SKILLS_ENABLED_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::EXTERNAL_SKILLS_REQUIRE_APPROVAL_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::EXTERNAL_SKILLS_AUTO_EXPOSE_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }

    #[test]
    fn provider_oauth_access_token_updates_config_and_origin() {
        let mut draft =
            OnboardDraft::from_config(sample_config(), PathBuf::from("/tmp/current.toml"), None);

        draft.set_provider_oauth_access_token("oauth-inline-token".to_owned());

        assert_eq!(draft.config.provider.api_key, None);
        assert_eq!(draft.config.provider.api_key_env, None);
        assert_eq!(draft.config.provider.oauth_access_token_env, None);
        assert_eq!(
            draft.config.provider.oauth_access_token,
            Some(SecretRef::Inline("oauth-inline-token".to_owned()))
        );
        assert_eq!(
            draft.origin_for(OnboardDraft::PROVIDER_CREDENTIAL_KEY),
            Some(OnboardValueOrigin::UserSelected)
        );
    }
}
