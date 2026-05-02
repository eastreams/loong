use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loong_contracts::ExecutionSecurityTier;
use loong_kernel as kernel;

#[derive(Debug, Clone, Default)]
pub struct BridgeExecutionPolicy {
    pub execute_process_stdio: bool,
    pub execute_http_json: bool,
    pub allowed_process_commands: BTreeSet<String>,
}

impl BridgeExecutionPolicy {
    #[must_use]
    pub fn process_stdio_execution_security_tier(&self) -> ExecutionSecurityTier {
        let process_stdio_enabled = self.execute_process_stdio;
        let commands_configured = !self.allowed_process_commands.is_empty();
        let balanced_execution = process_stdio_enabled && commands_configured;
        if balanced_execution {
            return ExecutionSecurityTier::Balanced;
        }

        ExecutionSecurityTier::Restricted
    }
}

pub fn parse_process_args(provider: &kernel::ProviderConfig) -> Vec<String> {
    let args_json = provider.metadata.get("args_json");
    if let Some(args_json) = args_json {
        let parsed_args = serde_json::from_str::<Vec<String>>(args_json);
        if let Ok(parsed_args) = parsed_args {
            return resolve_process_args_against_plugin_root(provider, parsed_args);
        }
    }

    let args = provider.metadata.get("args");
    let Some(args) = args else {
        return Vec::new();
    };

    let parsed_args = args.split_whitespace().map(str::to_owned).collect();
    resolve_process_args_against_plugin_root(provider, parsed_args)
}

fn resolve_process_args_against_plugin_root(
    provider: &kernel::ProviderConfig,
    args: Vec<String>,
) -> Vec<String> {
    let Some(plugin_package_root) = provider.metadata.get("plugin_package_root") else {
        return args;
    };
    let plugin_package_root = PathBuf::from(plugin_package_root);
    if !plugin_package_root.is_dir() {
        return args;
    }

    args.into_iter()
        .map(|arg| {
            if arg.trim().is_empty() || arg.starts_with('-') {
                return arg;
            }
            let arg_path = Path::new(arg.as_str());
            if arg_path.is_absolute() {
                return arg;
            }

            let candidate = plugin_package_root.join(arg_path);
            if candidate.exists() {
                return candidate.display().to_string();
            }

            arg
        })
        .collect()
}

pub fn is_process_command_allowed(program: &str, allowed: &BTreeSet<String>) -> bool {
    let allowlist_is_empty = allowed.is_empty();
    if allowlist_is_empty {
        return false;
    }

    let trimmed_program = program.trim();
    let normalized_program = trimmed_program.to_ascii_lowercase();
    let direct_match = allowed.contains(&normalized_program);
    if direct_match {
        return true;
    }

    let program_path = Path::new(trimmed_program);
    let has_path_component = program_path.is_absolute()
        || program_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
    if has_path_component {
        return false;
    }

    let file_name = program_path.file_name();
    let file_name = file_name.and_then(|name| name.to_str());
    let Some(file_name) = file_name else {
        return false;
    };

    let normalized_file_name = file_name.to_ascii_lowercase();
    allowed.contains(&normalized_file_name)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;

    use super::{is_process_command_allowed, parse_process_args};
    use loong_kernel::ProviderConfig;

    #[test]
    fn process_command_allowlist_rejects_path_spoofing() {
        let allowed_commands = BTreeSet::from(["python3".to_owned()]);

        assert!(is_process_command_allowed("python3", &allowed_commands));
        assert!(!is_process_command_allowed(
            "/tmp/python3",
            &allowed_commands,
        ));
        assert!(!is_process_command_allowed("./python3", &allowed_commands,));
    }

    #[test]
    fn parse_process_args_resolves_relative_paths_against_plugin_package_root() {
        let root = std::env::temp_dir().join(format!(
            "loong-bridge-process-args-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).expect("create root");
        fs::write(root.join("index.js"), "console.log('ok');\n").expect("write js");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write cargo");

        let provider = ProviderConfig {
            provider_id: "demo".to_owned(),
            connector_name: "demo".to_owned(),
            version: "0.1.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_package_root".to_owned(), root.display().to_string()),
                (
                    "args_json".to_owned(),
                    "[\"run\",\"--manifest-path\",\"Cargo.toml\",\"index.js\"]".to_owned(),
                ),
            ]),
        };

        let args = parse_process_args(&provider);

        assert_eq!(args[0], "run");
        assert_eq!(args[1], "--manifest-path");
        assert_eq!(args[2], root.join("Cargo.toml").display().to_string());
        assert_eq!(args[3], root.join("index.js").display().to_string());
    }
}
