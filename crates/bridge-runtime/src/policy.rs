use std::collections::BTreeSet;
use std::path::Path;

use loongclaw_contracts::ExecutionSecurityTier;
use loongclaw_kernel as kernel;

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
            return parsed_args;
        }
    }

    let args = provider.metadata.get("args");
    let Some(args) = args else {
        return Vec::new();
    };

    args.split_whitespace().map(str::to_owned).collect()
}

pub fn is_process_command_allowed(program: &str, allowed: &BTreeSet<String>) -> bool {
    let allowlist_is_empty = allowed.is_empty();
    if allowlist_is_empty {
        return false;
    }

    let normalized_program = program.trim().to_ascii_lowercase();
    let direct_match = allowed.contains(&normalized_program);
    if direct_match {
        return true;
    }

    let program_path = Path::new(program);
    let file_name = program_path.file_name();
    let file_name = file_name.and_then(|name| name.to_str());
    let Some(file_name) = file_name else {
        return false;
    };

    let normalized_file_name = file_name.to_ascii_lowercase();
    allowed.contains(&normalized_file_name)
}
