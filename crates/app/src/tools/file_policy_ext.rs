use std::path::{Path, PathBuf};

use loongclaw_contracts::{Capability, PolicyError};
use loongclaw_kernel::{PolicyExtension, PolicyExtensionContext};

pub struct FilePolicyExtension {
    file_root: Option<PathBuf>,
}

impl FilePolicyExtension {
    pub fn new(file_root: Option<PathBuf>) -> Self {
        Self { file_root }
    }

    fn normalize_tool_name(raw: &str) -> &str {
        match raw {
            "file_read" => "file.read",
            "file_write" => "file.write",
            other => other,
        }
    }

    fn required_capability(tool_name: &str) -> Option<Capability> {
        match tool_name {
            "file.read" | "claw.import" => Some(Capability::FilesystemRead),
            "file.write" => Some(Capability::FilesystemWrite),
            _ => None,
        }
    }

    /// Pure path-based escape check without filesystem access.
    fn path_escapes_root(root: &Path, raw_path: &str) -> bool {
        let candidate = Path::new(raw_path);
        let combined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        let normalized = normalize_without_fs(&combined);
        !normalized.starts_with(root)
    }
}

/// Normalize a path by resolving `.` and `..` components without touching the filesystem.
fn normalize_without_fs(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

impl PolicyExtension for FilePolicyExtension {
    fn name(&self) -> &str {
        "file-policy"
    }

    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        let Some(params) = context.request_parameters else {
            return Ok(());
        };

        let raw_tool_name = params
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let tool_name = Self::normalize_tool_name(raw_tool_name);

        let Some(required_cap) = Self::required_capability(tool_name) else {
            return Ok(());
        };

        if !context.token.allowed_capabilities.contains(&required_cap) {
            return Err(PolicyError::ExtensionDenied {
                extension: self.name().to_owned(),
                reason: format!(
                    "tool `{tool_name}` requires capability `{required_cap:?}` not granted to token"
                ),
            });
        }

        if let Some(ref root) = self.file_root {
            let raw_path = params
                .get("payload")
                .and_then(|p| p.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if !raw_path.is_empty() && Self::path_escapes_root(root, raw_path) {
                return Err(PolicyError::ExtensionDenied {
                    extension: self.name().to_owned(),
                    reason: format!("path `{raw_path}` escapes file root `{}`", root.display()),
                });
            }
        }

        Ok(())
    }
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
            granted_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
            metadata: BTreeMap::new(),
        }
    }

    fn token_with_caps(caps: BTreeSet<Capability>) -> CapabilityToken {
        CapabilityToken {
            token_id: "tok-1".into(),
            agent_id: "agent-1".into(),
            pack_id: "test-pack".into(),
            issued_at_epoch_s: 1000,
            expires_at_epoch_s: 2000,
            allowed_capabilities: caps,
            generation: 1,
            membrane: None,
        }
    }

    fn make_context<'a>(
        pack: &'a VerticalPackManifest,
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
    fn denies_file_write_without_capability() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params =
            json!({"tool_name": "file.write", "payload": {"path": "foo.txt", "content": "x"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn denies_file_read_without_capability() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "foo.txt"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn allows_file_read_with_capability() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "src/main.rs"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn denies_path_escape() {
        let ext = FilePolicyExtension::new(Some(PathBuf::from("/home/user/project")));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "../../etc/passwd"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn allows_path_within_root() {
        let ext = FilePolicyExtension::new(Some(PathBuf::from("/home/user/project")));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "src/main.rs"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }
}
