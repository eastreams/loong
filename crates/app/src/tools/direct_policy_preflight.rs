use loongclaw_contracts::ToolCoreRequest;

use super::file_policy_ext;
use super::runtime_config;
use super::shell_policy_ext;

pub(super) fn run(
    request: &ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<(), String> {
    let payload = request.payload.as_object();
    let Some(payload) = payload else {
        return Ok(());
    };

    let tool_name = request.tool_name.as_str();

    if tool_name == "shell.exec" {
        return shell_policy_ext::authorize_direct_shell_payload(payload, config);
    }

    let is_file_tool = matches!(tool_name, "file.read" | "file.write" | "file.edit");
    if is_file_tool {
        return file_policy_ext::authorize_direct_file_payload(tool_name, payload, config);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use loongclaw_contracts::ToolCoreRequest;
    use serde_json::json;

    use super::run;
    use super::runtime_config;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir();
        let dirname = format!("loongclaw-{prefix}-{sequence}");
        let path = temp_root.join(dirname);
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn run_reuses_shared_shell_policy_default_deny() {
        let config = runtime_config::ToolRuntimeConfig {
            shell_allow: BTreeSet::new(),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let request = ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": "git"}),
        };

        let error = run(&request, &config).expect_err("unknown shell command should be denied");

        assert_eq!(
            error,
            "policy_denied: tool call denied by policy for `shell.exec`: command `git` is not in the allow list (default-deny policy)"
        );
    }

    #[test]
    fn run_reuses_shared_file_policy_escape_guard() {
        let root = unique_temp_dir("direct-policy-preflight");
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "../outside.txt",
                "content": "blocked"
            }),
        };

        let error = run(&request, &config).expect_err("escaped file path should be denied");

        assert!(
            error.starts_with("policy_denied: "),
            "expected policy_denied prefix, got: {error}"
        );
        assert!(
            error.contains("policy extension file-policy denied request"),
            "expected shared file policy prefix, got: {error}"
        );
        assert!(
            error.contains("escapes file root"),
            "expected shared file policy denial, got: {error}"
        );
    }
}
