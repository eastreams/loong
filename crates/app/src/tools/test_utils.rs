use super::*;
use crate::test_utils::*;
use std::ops::{Deref, DerefMut};

#[cfg(feature = "tool-shell")]
pub fn ready_bash_exec_runtime_policy() -> runtime_config::BashExecRuntimePolicy {
    let resolved_bash =
        which::which("bash").unwrap_or_else(|_| std::path::PathBuf::from("/bin/bash"));
    runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(resolved_bash),
        ..runtime_config::BashExecRuntimePolicy::default()
    }
}

#[cfg(all(feature = "tool-shell", unix))]
pub fn configured_test_bash_runtime_with_rules(
    root: &std::path::Path,
) -> (runtime_config::BashExecRuntimePolicy, std::path::PathBuf) {
    let log_path = root.join("bash-args.log");
    let runtime_path = write_fake_bash_runtime(root, "fake-bash", &log_path);
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    let rules = super::bash::rules::load_rules_from_dir(&rules_dir).expect("load rules");

    (
        runtime_config::BashExecRuntimePolicy {
            available: true,
            command: Some(runtime_path),
            governance: runtime_config::BashGovernanceRuntimePolicy {
                rules_dir,
                rules,
                load_error: None,
            },
            ..runtime_config::BashExecRuntimePolicy::default()
        },
        log_path,
    )
}

#[cfg(all(feature = "tool-shell", unix))]
pub fn write_fake_bash_runtime(
    root: &std::path::Path,
    name: &str,
    log_path: &std::path::Path,
) -> std::path::PathBuf {
    let path = root.join(name);
    let script = format!(
        "#!/bin/sh\nLOG_PATH=\"{}\"\n: > \"$LOG_PATH\"\nfor arg in \"$@\"; do\n  printf '%s\\n' \"$arg\" >> \"$LOG_PATH\"\ndone\nMODE=\"${{1:-}}\"\nCOMMAND=\"${{2:-}}\"\ncase \"$MODE\" in\n  -c|-lc)\n    exec /bin/sh -c \"$COMMAND\"\n    ;;\n  *)\n    printf 'unexpected bash args: %s' \"$*\" >&2\n    exit 97\n    ;;\nesac\n",
        log_path.display()
    );
    crate::test_utils::write_executable_script_atomically(&path, &script)
        .expect("write fake bash runtime");
    path
}

pub fn execute_tool_core_with_test_context(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if payload_uses_reserved_internal_tool_context(&request.payload) {
        with_trusted_internal_tool_payload(|| super::execute_tool_core_with_config(request, config))
    } else {
        super::execute_tool_core_with_config(request, config)
    }
}

pub struct ToolTestRuntimeConfig {
    config: runtime_config::ToolRuntimeConfig,
    _runtime_home: ScopedLoongHome,
}

impl Deref for ToolTestRuntimeConfig {
    type Target = runtime_config::ToolRuntimeConfig;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl DerefMut for ToolTestRuntimeConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

impl ToolTestRuntimeConfig {
    pub fn into_inner(self) -> runtime_config::ToolRuntimeConfig {
        self.config
    }

    pub fn runtime_home(&self) -> &ScopedLoongHome {
        &self._runtime_home
    }
}

pub fn test_tool_runtime_config(root: impl AsRef<std::path::Path>) -> ToolTestRuntimeConfig {
    let runtime_home = ScopedLoongHome::new("loong-tool-runtime-home");
    let config = runtime_config::ToolRuntimeConfig {
        #[cfg(feature = "tool-shell")]
        shell_allow: BTreeSet::from(["echo".to_owned(), "cat".to_owned(), "ls".to_owned()]),
        file_root: Some(root.as_ref().to_path_buf()),
        messages_enabled: true,
        skills: runtime_config::SkillsRuntimePolicy {
            enabled: true,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: false,
        },
        ..Default::default()
    };
    ToolTestRuntimeConfig {
        config,
        _runtime_home: runtime_home,
    }
}
