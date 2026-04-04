use std::collections::BTreeSet;

use loongclaw_contracts::PolicyError;
use loongclaw_kernel::{PolicyExtension, PolicyExtensionContext};

pub(crate) const SHELL_EXEC_APPROVAL_RULE_ID: &str = "shell_exec_requires_approval";
const SHELL_INTERNAL_APPROVAL_CONTEXT_KEY: &str = "shell_approval";
const SHELL_INTERNAL_APPROVAL_KEY_FIELD: &str = "approval_key";
const REPAIRABLE_TOOL_INPUT_PREFIX: &str = "repairable_tool_input: ";

fn repairable_tool_input_reason(reason: String) -> String {
    format!("{REPAIRABLE_TOOL_INPUT_PREFIX}{reason}")
}

pub(crate) fn is_repairable_tool_input_reason(reason: &str) -> bool {
    reason.starts_with(REPAIRABLE_TOOL_INPUT_PREFIX)
}

pub(crate) fn strip_repairable_tool_input_prefix(reason: &str) -> &str {
    reason
        .strip_prefix(REPAIRABLE_TOOL_INPUT_PREFIX)
        .unwrap_or(reason)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPolicyDefault {
    Deny,
    Allow,
}

impl ShellPolicyDefault {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "allow" => Self::Allow,
            _ => Self::Deny,
        }
    }
}

pub struct ToolPolicyExtension {
    hard_deny: BTreeSet<String>,
    allow: BTreeSet<String>,
    default_mode: ShellPolicyDefault,
}

impl ToolPolicyExtension {
    pub fn new(
        hard_deny: BTreeSet<String>,
        allow: BTreeSet<String>,
        default_mode: ShellPolicyDefault,
    ) -> Self {
        Self {
            hard_deny,
            allow,
            default_mode,
        }
    }

    /// Build from runtime config. All lists come exclusively from the config;
    /// there are no hidden hardcoded entries that cannot be removed by the user.
    pub fn from_config(rt: &super::runtime_config::ToolRuntimeConfig) -> Self {
        Self {
            hard_deny: rt.shell_deny.clone(),
            allow: rt.shell_allow.clone(),
            default_mode: rt.shell_default_mode,
        }
    }

    fn authorize_shell_payload(
        &self,
        tool_name: &str,
        payload: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), PolicyError> {
        let command = payload.get("command").and_then(serde_json::Value::as_str);
        let Some(command) = command else {
            return Ok(());
        };
        let trimmed_command = command.trim();
        if trimmed_command.is_empty() {
            return Ok(());
        }

        let basename = validate_shell_command_name(trimmed_command).map_err(|reason| {
            PolicyError::ToolCallDenied {
                tool_name: tool_name.to_owned(),
                reason,
            }
        })?;

        if self.hard_deny.contains(basename.as_str()) {
            return Err(PolicyError::ToolCallDenied {
                tool_name: tool_name.to_owned(),
                reason: format!("command `{basename}` is blocked by shell policy"),
            });
        }

        if self.allow.contains(basename.as_str()) {
            return Ok(());
        }

        let approval_key = shell_exec_approval_key_for_normalized_command(basename.as_str());
        let approved_by_internal_context =
            shell_exec_matches_trusted_internal_approval(payload, approval_key.as_str());
        if approved_by_internal_context {
            return Ok(());
        }

        match self.default_mode {
            ShellPolicyDefault::Allow => Ok(()),
            ShellPolicyDefault::Deny => Err(PolicyError::ToolCallDenied {
                tool_name: tool_name.to_owned(),
                reason: format!(
                    "command `{basename}` is not in the allow list (default-deny policy)"
                ),
            }),
        }
    }
}

pub(crate) fn authorize_direct_shell_payload(
    payload: &serde_json::Map<String, serde_json::Value>,
    rt: &super::runtime_config::ToolRuntimeConfig,
) -> Result<(), String> {
    let extension = ToolPolicyExtension::from_config(rt);
    extension
        .authorize_shell_payload("shell.exec", payload)
        .map_err(|error| format!("policy_denied: {error}"))
}

pub(crate) fn validate_shell_command_name(command: &str) -> Result<String, String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        let reason = repairable_tool_input_reason(
            "shell.exec requires payload.command. Provide a bare executable in payload.command and move arguments into payload.args.".to_owned(),
        );
        return Err(reason);
    }

    let contains_newline = trimmed.chars().any(|ch| matches!(ch, '\n' | '\r'));
    if contains_newline {
        let reason = repairable_tool_input_reason(
            "shell.exec requires payload.command. Provide a bare executable in payload.command and move arguments into payload.args.".to_owned(),
        );
        return Err(reason);
    }

    if trimmed.contains(char::is_whitespace) {
        let reason = repairable_tool_input_reason(
            "shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
        );
        return Err(reason);
    }

    if trimmed.contains('/') || trimmed.contains('\\') {
        let reason = format!(
            "shell.exec payload.command `{trimmed}` must be a bare lowercase executable name without path separators."
        );
        let reason = repairable_tool_input_reason(reason);
        return Err(reason);
    }

    let normalized = trimmed.to_ascii_lowercase();
    if trimmed != normalized {
        return Err(format!(
            "policy_denied: shell command `{trimmed}` must use a lowercase bare command name to match shell policy"
        ));
    }

    Ok(normalized)
}

#[cfg(test)]
pub(crate) fn shell_exec_approval_key(command: &str) -> Result<String, String> {
    let normalized_command = validate_shell_command_name(command)?;
    Ok(shell_exec_approval_key_for_normalized_command(
        normalized_command.as_str(),
    ))
}

pub(crate) fn shell_exec_approval_key_for_normalized_command(normalized_command: &str) -> String {
    format!("tool:shell.exec:{normalized_command}")
}

pub(crate) fn shell_exec_internal_approval_context(approval_key: &str) -> serde_json::Value {
    serde_json::json!({
        SHELL_INTERNAL_APPROVAL_CONTEXT_KEY: {
            SHELL_INTERNAL_APPROVAL_KEY_FIELD: approval_key,
        }
    })
}

pub(crate) fn shell_exec_matches_trusted_internal_approval(
    payload: &serde_json::Map<String, serde_json::Value>,
    approval_key: &str,
) -> bool {
    let trusted_context = payload.get(super::LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY);
    let trusted_context = trusted_context.and_then(serde_json::Value::as_object);
    let trusted_context =
        trusted_context.and_then(|value| value.get(SHELL_INTERNAL_APPROVAL_CONTEXT_KEY));
    let trusted_context = trusted_context.and_then(serde_json::Value::as_object);
    let trusted_approval_key =
        trusted_context.and_then(|value| value.get(SHELL_INTERNAL_APPROVAL_KEY_FIELD));
    let trusted_approval_key = trusted_approval_key.and_then(serde_json::Value::as_str);
    trusted_approval_key == Some(approval_key)
}

impl PolicyExtension for ToolPolicyExtension {
    fn name(&self) -> &str {
        "tool-policy"
    }

    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        let Some(params) = context.request_parameters else {
            return Ok(());
        };

        let (tool_name, payload) = effective_shell_request(params);
        if tool_name != "shell.exec" {
            return Ok(());
        }

        let Some(payload) = payload.and_then(serde_json::Value::as_object) else {
            return Ok(());
        };

        self.authorize_shell_payload(tool_name, payload)
    }
}

fn effective_shell_request(params: &serde_json::Value) -> (&str, Option<&serde_json::Value>) {
    let raw_tool_name = params
        .get("tool_name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let tool_name = super::canonical_tool_name(raw_tool_name);
    if tool_name != "tool.invoke" {
        return (tool_name, params.get("payload"));
    }

    let payload = params.get("payload");
    let invoked_tool_name = payload
        .and_then(|payload| payload.get("tool_id"))
        .and_then(|value| value.as_str())
        .map(super::canonical_tool_name)
        .unwrap_or(tool_name);
    let invoked_payload = payload.and_then(|payload| payload.get("arguments"));
    (invoked_tool_name, invoked_payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loongclaw_contracts::{Capability, CapabilityToken, ExecutionRoute, HarnessKind};
    use loongclaw_kernel::{PolicyExtensionContext, VerticalPackManifest};
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn test_pack() -> VerticalPackManifest {
        VerticalPackManifest {
            pack_id: "test-pack".into(),
            domain: "test".into(),
            version: "0.1.0".into(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        }
    }

    fn test_token() -> CapabilityToken {
        CapabilityToken {
            token_id: "tok-1".into(),
            agent_id: "agent-1".into(),
            pack_id: "test-pack".into(),
            issued_at_epoch_s: 1000,
            expires_at_epoch_s: 2000,
            allowed_capabilities: BTreeSet::from([Capability::InvokeTool]),
            generation: 1,
        }
    }

    fn make_context<'a>(
        pack: &'a loongclaw_kernel::VerticalPackManifest,
        token: &'a CapabilityToken,
        caps: &'a BTreeSet<Capability>,
        params: Option<&'a serde_json::Value>,
    ) -> PolicyExtensionContext<'a> {
        PolicyExtensionContext {
            pack,
            token,
            now_epoch_s: 1500,
            required_capabilities: caps,
            request_parameters: params,
        }
    }

    #[test]
    fn denies_destructive_shell_commands() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "rm"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn allows_safe_shell_commands() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::new(),
            BTreeSet::from(["echo".to_owned()]),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "echo"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn normalizes_underscore_shell_alias() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["curl".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell_exec", "payload": {"command": "curl"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn unwraps_tool_invoke_shell_payloads() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::from(["ls".to_owned()]),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);

        let denied = json!({
            "tool_name": "tool.invoke",
            "payload": {
                "tool_id": "shell.exec",
                "arguments": {"command": "rm"}
            }
        });
        let denied_ctx = make_context(&pack, &token, &caps, Some(&denied));
        assert!(matches!(
            ext.authorize_extension(&denied_ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));

        let allowed = json!({
            "tool_name": "tool.invoke",
            "payload": {
                "tool_id": "shell_exec",
                "arguments": {"command": "ls"}
            }
        });
        let allowed_ctx = make_context(&pack, &token, &caps, Some(&allowed));
        assert!(ext.authorize_extension(&allowed_ctx).is_ok());
    }

    #[test]
    fn allows_trusted_shell_approval_context_when_default_policy_denies() {
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Deny);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let approval_key = shell_exec_approval_key("cargo").expect("approval key");
        let params = json!({
            "tool_name": "shell.exec",
            "payload": {
                "command": "cargo",
                "_loongclaw": shell_exec_internal_approval_context(approval_key.as_str()),
            }
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));

        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn hard_deny_overrides_trusted_shell_approval_context() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["cargo".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let approval_key = shell_exec_approval_key("cargo").expect("approval key");
        let params = json!({
            "tool_name": "shell.exec",
            "payload": {
                "command": "cargo",
                "_loongclaw": shell_exec_internal_approval_context(approval_key.as_str()),
            }
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));

        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn keeps_non_shell_tools_allowed() {
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Deny);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "/etc/passwd"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn allows_when_no_request_parameters() {
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Deny);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let ctx = make_context(&pack, &token, &caps, None);
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn allows_malformed_shell_payload_to_adapter_layer() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);

        // payload is a string instead of an object — no command can be extracted,
        // so the extension defers to the tool adapter for error handling.
        let params = json!({"tool_name": "shell.exec", "payload": "not an object"});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());

        // payload.command is missing entirely
        let params = json!({"tool_name": "shell.exec", "payload": {"args": ["hello"]}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());

        // payload.command is empty after trimming
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "  "}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn denies_absolute_path_command() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "/usr/bin/rm"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn denies_command_with_trailing_separator() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        // Trailing slash should still extract "rm" as basename, not "".
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "/usr/bin/rm/"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn denies_windows_absolute_path_command() {
        // "rm.exe" is not in any list; default-deny mode blocks it.
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Deny);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "C:\\Windows\\System32\\rm.exe"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn normalizes_bare_shell_alias() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell", "payload": {"command": "rm"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn case_insensitive_command_matching() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::from(["rm".to_owned()]),
            BTreeSet::new(),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "RM"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ToolCallDenied { .. }
        ));
    }

    #[test]
    fn default_deny_blocks_unknown_command() {
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Deny);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params =
            json!({"tool_name": "shell.exec", "payload": {"command": "some_unknown_tool"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let err = ext.authorize_extension(&ctx).unwrap_err();
        assert!(matches!(err, PolicyError::ToolCallDenied { .. }));
    }

    #[test]
    fn allow_listed_command_passes() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::new(),
            BTreeSet::from(["git".to_owned()]),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "git"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn mixed_case_command_is_denied_before_allowlist_lookup() {
        let ext = ToolPolicyExtension::new(
            BTreeSet::new(),
            BTreeSet::from(["mixedcmd".to_owned()]),
            ShellPolicyDefault::Deny,
        );
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "MiXeDCmd"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let err = ext
            .authorize_extension(&ctx)
            .expect_err("mixed-case commands should be denied before allowlist lookup");
        let PolicyError::ToolCallDenied { reason, .. } = err else {
            panic!("expected ToolCallDenied for mixed-case command");
        };
        assert!(
            reason.contains("lowercase"),
            "expected lowercase rejection, got: {reason}"
        );
    }

    #[test]
    fn empty_shell_command_reason_is_marked_repairable() {
        let error =
            validate_shell_command_name("   ").expect_err("blank shell command should be denied");

        assert!(
            is_repairable_tool_input_reason(error.as_str()),
            "expected repairable prefix, got: {error}"
        );
        assert!(
            strip_repairable_tool_input_prefix(error.as_str()).contains("payload.command"),
            "expected payload.command guidance, got: {error}"
        );
    }

    #[test]
    fn allow_mode_passes_unknown_command() {
        let ext =
            ToolPolicyExtension::new(BTreeSet::new(), BTreeSet::new(), ShellPolicyDefault::Allow);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "shell.exec", "payload": {"command": "anything"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    // ── from_config tests ────────────────────────────────────────────────────
    // These tests verify that from_config uses the runtime config values
    // exclusively, with no hidden hardcoded entries.

    fn make_rt(
        allow: &[&str],
        deny: &[&str],
        mode: ShellPolicyDefault,
    ) -> super::super::runtime_config::ToolRuntimeConfig {
        super::super::runtime_config::ToolRuntimeConfig {
            file_root: None,
            shell_allow: allow.iter().map(|s| s.to_string()).collect(),
            shell_deny: deny.iter().map(|s| s.to_string()).collect(),
            shell_default_mode: mode,
            ..super::super::runtime_config::ToolRuntimeConfig::default()
        }
    }

    fn shell_params(command: &str) -> serde_json::Value {
        json!({"tool_name": "shell.exec", "payload": {"command": command}})
    }

    /// `from_config` with an empty deny list does NOT explicitly deny `rm`;
    /// `rm` is only blocked by the default-deny fallback, not a deny entry.
    #[test]
    fn from_config_empty_deny_rm_hits_default_deny_not_explicit_deny() {
        let rt = make_rt(&[], &[], ShellPolicyDefault::Deny);
        let ext = ToolPolicyExtension::from_config(&rt);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = shell_params("rm");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        // Blocked — but because of default-deny, not an explicit deny entry.
        let err = ext.authorize_extension(&ctx).unwrap_err();
        match err {
            PolicyError::ToolCallDenied { reason, .. } => {
                assert!(
                    reason.contains("not in the allow list"),
                    "expected default-deny reason, got: {reason}"
                );
            }
            other @ (PolicyError::ExpiredToken { .. }
            | PolicyError::MissingCapability { .. }
            | PolicyError::PackMismatch { .. }
            | PolicyError::RevokedToken { .. }
            | PolicyError::ExtensionDenied { .. }
            | _) => {
                panic!("expected ToolCallDenied, got {other:?}")
            }
        }
    }

    /// With an empty allow list, `echo` (a default-allow command in config
    /// defaults) must be denied when the user explicitly sets allow = [].
    #[test]
    fn from_config_empty_allow_echo_is_denied() {
        let rt = make_rt(&[], &[], ShellPolicyDefault::Deny);
        let ext = ToolPolicyExtension::from_config(&rt);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = shell_params("echo");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(
            ext.authorize_extension(&ctx).is_err(),
            "echo must be denied when allow list is empty"
        );
    }

    /// User-supplied deny list is used verbatim: a command listed in deny is
    /// explicitly denied, and a command NOT in the list is not.
    #[test]
    fn from_config_user_deny_list_is_exact() {
        let rt = make_rt(&["ls"], &["custom_danger"], ShellPolicyDefault::Deny);
        let ext = ToolPolicyExtension::from_config(&rt);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);

        // custom_danger → explicit deny: reason must say "blocked by shell policy",
        // not "not in the allow list" (which would indicate default-deny, not explicit deny).
        let params = shell_params("custom_danger");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let err = ext.authorize_extension(&ctx).unwrap_err();
        match err {
            PolicyError::ToolCallDenied { reason, .. } => {
                assert!(
                    reason.contains("blocked by shell policy"),
                    "expected explicit-deny reason, got: {reason}"
                );
            }
            other @ (PolicyError::ExpiredToken { .. }
            | PolicyError::MissingCapability { .. }
            | PolicyError::PackMismatch { .. }
            | PolicyError::RevokedToken { .. }
            | PolicyError::ExtensionDenied { .. }
            | _) => {
                panic!("expected ToolCallDenied, got {other:?}")
            }
        }

        // ls → allowed
        let params = shell_params("ls");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());

        // unknown_cmd → not in any list, hits default-deny (NOT explicit deny)
        let params = shell_params("unknown_cmd");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let err = ext.authorize_extension(&ctx).unwrap_err();
        match err {
            PolicyError::ToolCallDenied { reason, .. } => {
                assert!(
                    reason.contains("not in the allow list"),
                    "expected default-deny reason, got: {reason}"
                );
            }
            other @ (PolicyError::ExpiredToken { .. }
            | PolicyError::MissingCapability { .. }
            | PolicyError::PackMismatch { .. }
            | PolicyError::RevokedToken { .. }
            | PolicyError::ExtensionDenied { .. }
            | _) => {
                panic!("expected ToolCallDenied, got {other:?}")
            }
        }
    }

    /// With `shell_default_mode = allow`, unknown commands pass even with
    /// empty allow/deny lists.
    #[test]
    fn from_config_allow_mode_passes_unknown() {
        let rt = make_rt(&[], &[], ShellPolicyDefault::Allow);
        let ext = ToolPolicyExtension::from_config(&rt);
        let pack = test_pack();
        let token = test_token();
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = shell_params("anything_unknown");
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }
}
