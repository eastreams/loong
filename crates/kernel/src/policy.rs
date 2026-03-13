use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

// Re-export data types from contracts
pub use loongclaw_contracts::{PolicyContext, PolicyDecision, PolicyRequest};

use crate::{contracts::CapabilityToken, errors::PolicyError, pack::VerticalPackManifest};

const SHELL_HARD_DENY_COMMANDS: &[&str] = &[
    "rm", "dd", "mkfs", "shutdown", "reboot", "poweroff", "halt", "init",
];
const SHELL_APPROVAL_REQUIRED_COMMANDS: &[&str] = &[
    "bash",
    "sh",
    "zsh",
    "fish",
    "sudo",
    "su",
    "curl",
    "wget",
    "ssh",
    "scp",
    "sftp",
    "nc",
    "ncat",
    "netcat",
    "python",
    "python3",
    "node",
    "perl",
    "ruby",
    "php",
    "cat",
    "ls",
    "pwsh",
    "powershell",
];

pub trait PolicyEngine: Send + Sync {
    fn issue_token(
        &self,
        pack: &VerticalPackManifest,
        agent_id: &str,
        now_epoch_s: u64,
        ttl_s: u64,
    ) -> Result<CapabilityToken, PolicyError>;

    fn authorize(
        &self,
        token: &CapabilityToken,
        runtime_pack_id: &str,
        now_epoch_s: u64,
        required: &std::collections::BTreeSet<crate::contracts::Capability>,
    ) -> Result<(), PolicyError>;

    fn revoke_token(&self, token_id: &str) -> Result<(), PolicyError>;

    fn revoke_generation(&self, _below: u64) {
        // Default no-op
    }

    fn check_tool_call(&self, _request: &PolicyRequest) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[derive(Debug, Default)]
pub struct StaticPolicyEngine {
    token_seq: AtomicU64,
    revoked_tokens: Mutex<BTreeSet<String>>,
    generation: AtomicU64,
    revoked_below_generation: AtomicU64,
}

impl StaticPolicyEngine {
    fn next_token_id(&self) -> String {
        let seq = self.token_seq.fetch_add(1, Ordering::Relaxed) + 1;
        format!("tok-{seq:016x}")
    }

    /// Revoke all tokens with generation <= `below`.
    ///
    /// Note: tokens issued concurrently during this call may land in the
    /// revoked range. This is acceptable for StaticPolicyEngine (test/dev).
    /// A production engine should use a lock or AcqRel ordering.
    pub fn revoke_generation(&self, below: u64) {
        self.revoked_below_generation
            .fetch_max(below, Ordering::Relaxed);
        // Fast-forward generation so newly issued tokens won't be immediately revoked.
        self.generation.fetch_max(below, Ordering::Relaxed);
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}

impl PolicyEngine for StaticPolicyEngine {
    fn issue_token(
        &self,
        pack: &VerticalPackManifest,
        agent_id: &str,
        now_epoch_s: u64,
        ttl_s: u64,
    ) -> Result<CapabilityToken, PolicyError> {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(CapabilityToken {
            token_id: self.next_token_id(),
            pack_id: pack.pack_id.clone(),
            agent_id: agent_id.to_owned(),
            allowed_capabilities: pack.granted_capabilities.clone(),
            issued_at_epoch_s: now_epoch_s,
            expires_at_epoch_s: now_epoch_s.saturating_add(ttl_s),
            generation,
            membrane: None,
        })
    }

    fn authorize(
        &self,
        token: &CapabilityToken,
        runtime_pack_id: &str,
        now_epoch_s: u64,
        required: &std::collections::BTreeSet<crate::contracts::Capability>,
    ) -> Result<(), PolicyError> {
        if self
            .revoked_tokens
            .lock()
            .map_err(|_err| PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            })?
            .contains(&token.token_id)
        {
            return Err(PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            });
        }

        let threshold = self.revoked_below_generation.load(Ordering::Relaxed);
        if token.generation > 0 && token.generation <= threshold {
            return Err(PolicyError::RevokedToken {
                token_id: token.token_id.clone(),
            });
        }

        if token.pack_id != runtime_pack_id {
            return Err(PolicyError::PackMismatch {
                token_pack_id: token.pack_id.clone(),
                runtime_pack_id: runtime_pack_id.to_owned(),
            });
        }

        if now_epoch_s > token.expires_at_epoch_s {
            return Err(PolicyError::ExpiredToken {
                token_id: token.token_id.clone(),
                expires_at_epoch_s: token.expires_at_epoch_s,
            });
        }

        for capability in required {
            if !token.allowed_capabilities.contains(capability) {
                return Err(PolicyError::MissingCapability {
                    token_id: token.token_id.clone(),
                    capability: *capability,
                });
            }
        }

        Ok(())
    }

    fn revoke_token(&self, token_id: &str) -> Result<(), PolicyError> {
        let mut revoked = self
            .revoked_tokens
            .lock()
            .map_err(|_err| PolicyError::RevokedToken {
                token_id: token_id.to_owned(),
            })?;
        revoked.insert(token_id.to_owned());
        Ok(())
    }

    fn revoke_generation(&self, below: u64) {
        self.revoke_generation(below);
    }

    fn check_tool_call(&self, request: &PolicyRequest) -> PolicyDecision {
        default_tool_policy(request)
    }
}

fn default_tool_policy(request: &PolicyRequest) -> PolicyDecision {
    match normalize_tool_name(request.tool_name.as_str()) {
        "shell.exec" => default_shell_policy(&request.parameters),
        _ => PolicyDecision::Allow,
    }
}

fn normalize_tool_name(raw: &str) -> &str {
    match raw {
        "shell_exec" | "shell" => "shell.exec",
        "file_read" => "file.read",
        "file_write" => "file.write",
        other => other,
    }
}

fn default_shell_policy(parameters: &serde_json::Value) -> PolicyDecision {
    let command = parameters
        .as_object()
        .and_then(|map| map.get("command"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    let Some(command) = command else {
        // Keep malformed-payload handling in tool adapter for backward compatibility.
        return PolicyDecision::Allow;
    };

    if SHELL_HARD_DENY_COMMANDS
        .iter()
        .any(|blocked| command == *blocked)
    {
        return PolicyDecision::Deny(format!(
            "command `{command}` is blocked by default shell policy"
        ));
    }

    if SHELL_APPROVAL_REQUIRED_COMMANDS
        .iter()
        .any(|gated| command == *gated)
    {
        return PolicyDecision::RequireApproval(format!(
            "command `{command}` requires approval by default shell policy"
        ));
    }

    PolicyDecision::Allow
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;

    fn policy_request(tool_name: &str, parameters: serde_json::Value) -> PolicyRequest {
        PolicyRequest {
            tool_name: tool_name.to_owned(),
            parameters,
            pack_id: "test-pack".to_owned(),
            agent_id: "test-agent".to_owned(),
            capabilities_used: BTreeSet::new(),
            context: PolicyContext::default(),
        }
    }

    #[test]
    fn static_policy_denies_destructive_shell_commands() {
        let request = policy_request("shell.exec", json!({"command": "rm", "args": ["-rf", "/"]}));
        let decision = default_tool_policy(&request);
        assert!(matches!(decision, PolicyDecision::Deny(_)));
    }

    #[test]
    fn static_policy_requires_approval_for_high_risk_shell_commands() {
        let request = policy_request(
            "shell.exec",
            json!({"command": "curl", "args": ["https://example.com"]}),
        );
        let decision = default_tool_policy(&request);
        assert!(matches!(decision, PolicyDecision::RequireApproval(_)));
    }

    #[test]
    fn static_policy_requires_approval_for_file_listing_commands() {
        let cat_request = policy_request(
            "shell.exec",
            json!({"command": "cat", "args": ["/etc/hosts"]}),
        );
        let ls_request = policy_request("shell.exec", json!({"command": "ls", "args": ["/"]}));

        assert!(matches!(
            default_tool_policy(&cat_request),
            PolicyDecision::RequireApproval(_)
        ));
        assert!(matches!(
            default_tool_policy(&ls_request),
            PolicyDecision::RequireApproval(_)
        ));
    }

    #[test]
    fn static_policy_allows_safe_shell_commands() {
        let request = policy_request("shell.exec", json!({"command": "echo", "args": ["hello"]}));
        let decision = default_tool_policy(&request);
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn static_policy_normalizes_underscore_shell_alias() {
        let request = policy_request("shell_exec", json!({"command": "curl"}));
        let decision = default_tool_policy(&request);
        assert!(matches!(decision, PolicyDecision::RequireApproval(_)));
    }

    #[test]
    fn static_policy_keeps_non_shell_tools_allowed() {
        let request = policy_request("file.read", json!({"path": "README.md"}));
        let decision = default_tool_policy(&request);
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn static_policy_allows_malformed_shell_payloads_to_adapter_layer() {
        let request = policy_request("shell.exec", json!("not an object"));
        let decision = default_tool_policy(&request);
        assert_eq!(decision, PolicyDecision::Allow);
    }
}
