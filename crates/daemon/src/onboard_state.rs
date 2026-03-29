use std::collections::BTreeMap;
use std::path::PathBuf;

use loongclaw_app as mvp;

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
    pub const MEMORY_PROFILE_KEY: &'static str = "memory.profile";
    pub const WEB_SEARCH_PROVIDER_KEY: &'static str = "tools.web_search.default_provider";
    pub const WEB_SEARCH_CREDENTIAL_KEY: &'static str = "tools.web_search.credential";
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
            draft.seed_origin(Self::MEMORY_PROFILE_KEY, origin);
            draft.seed_origin(Self::WEB_SEARCH_PROVIDER_KEY, origin);
            draft.seed_origin(Self::WEB_SEARCH_CREDENTIAL_KEY, origin);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
