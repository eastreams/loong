use std::collections::BTreeMap;
use std::path::PathBuf;

use loongclaw_app as mvp;

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
        }
        draft
    }

    pub fn origin_for(&self, key: &'static str) -> Option<OnboardValueOrigin> {
        self.origins.get(key).copied()
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
}
