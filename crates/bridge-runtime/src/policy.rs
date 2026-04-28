use std::collections::BTreeSet;
use std::path::Path;

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
            return parsed_args
                .into_iter()
                .map(|arg| resolve_relative_process_path_arg(provider, arg))
                .collect();
        }
    }

    let args = provider.metadata.get("args");
    let Some(args) = args else {
        return Vec::new();
    };

    args.split_whitespace()
        .map(str::to_owned)
        .map(|arg| resolve_relative_process_path_arg(provider, arg))
        .collect()
}

fn resolve_relative_process_path_arg(provider: &kernel::ProviderConfig, arg: String) -> String {
    let package_root = provider
        .metadata
        .get("plugin_package_root")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(package_root) = package_root else {
        return arg;
    };

    let candidate = Path::new(arg.as_str());
    if candidate.is_absolute() {
        return arg;
    }
    if arg.starts_with('-') {
        return arg;
    }

    let resolved = Path::new(package_root).join(candidate);
    if !resolved.exists() {
        return arg;
    }

    resolved.display().to_string()
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
    use std::collections::BTreeSet;

    use super::is_process_command_allowed;

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
}
