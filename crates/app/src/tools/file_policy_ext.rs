use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loongclaw_contracts::{Capability, PolicyError};
use loongclaw_kernel::{PolicyExtension, PolicyExtensionContext};

pub struct FilePolicyExtension {
    file_root: Option<PathBuf>,
    canon_root: Option<PathBuf>,
}

impl FilePolicyExtension {
    pub fn new(file_root: Option<PathBuf>) -> Self {
        let canon_root = file_root.as_ref().and_then(|r| r.canonicalize().ok());
        if file_root.is_some() && canon_root.is_none() {
            #[cfg(feature = "tool-file")]
            #[allow(clippy::print_stderr)]
            {
                eprintln!(
                    "warning: file_root {:?} could not be canonicalized; \
                     symlink-aware path checks will use raw path comparison",
                    file_root.as_deref().unwrap_or(Path::new("")),
                );
            }
        }
        Self {
            file_root,
            canon_root,
        }
    }

    fn required_capabilities(
        tool_name: &str,
        payload: &serde_json::Map<String, serde_json::Value>,
    ) -> BTreeSet<Capability> {
        let mut required_capabilities = BTreeSet::new();

        match tool_name {
            "file.read" | "memory_search" | "memory_get" => {
                required_capabilities.insert(Capability::FilesystemRead);
            }
            "file.write" | "file.edit" => {
                required_capabilities.insert(Capability::FilesystemWrite);
            }
            "config.import" => {
                required_capabilities.insert(Capability::FilesystemRead);

                let mode_requires_write =
                    super::config_import::config_import_mode_requires_write_object(payload);
                if mode_requires_write {
                    required_capabilities.insert(Capability::FilesystemWrite);
                }
            }
            _ => {}
        }

        required_capabilities
    }

    /// Check whether `raw_path` escapes the configured file root.
    ///
    /// Uses a layered strategy with filesystem-aware canonicalization so that
    /// symbolic links pointing outside the sandbox are caught at the policy
    /// layer (defense in depth — the execution layer still performs its own
    /// `canonicalize()`).
    ///
    /// Layers:
    /// 1. Full `canonicalize` — works when the entire path already exists.
    /// 2. Symlink detection — if the path is a symlink whose target cannot be
    ///    canonicalized (dangling), read the link target and check it directly.
    /// 3. Deepest existing ancestor `canonicalize` + missing suffix re-attach —
    ///    handles `file.write "nested/new.txt"` where one or more trailing
    ///    components do not exist yet.
    /// 4. Pure path normalization — no existing ancestor can be resolved.
    ///
    /// # Known limitations
    ///
    /// - **TOCTOU**: A race exists between this check and the actual file
    ///   operation. The execution layer's own `canonicalize()` serves as the
    ///   final defense.
    /// - **Deep intermediate symlinks**: If an intermediate directory component
    ///   is a dangling symlink (e.g. `root/a/b/file` where `a` is a symlink
    ///   and `b` does not exist under the target), layer 4 cannot detect the
    ///   escape. The execution layer catches this at open time.
    fn path_escapes_root(&self, raw_path: &str) -> bool {
        let root = match self.file_root.as_deref() {
            Some(r) => r,
            None => return false,
        };

        let effective_root = self.canon_root.as_deref().unwrap_or(root);
        let normalized_effective_root = super::normalize_without_fs(effective_root);

        let candidate = Path::new(raw_path);
        let combined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            effective_root.join(candidate)
        };

        // 1. Try full canonicalize (works when the path already exists).
        if let Ok(canon) = combined.canonicalize() {
            return !canon.starts_with(&normalized_effective_root);
        }

        // 1.5. Path is a symlink but canonicalize failed (dangling target) —
        //      read the link target and check whether it escapes.
        if combined.is_symlink() {
            if let Ok(target) = std::fs::read_link(&combined) {
                let resolved = if target.is_absolute() {
                    target
                } else {
                    let raw_parent = combined.parent().unwrap_or(effective_root);
                    let symlink_parent = match raw_parent.canonicalize() {
                        Ok(parent) => parent,
                        Err(_) => return true,
                    };
                    symlink_parent.join(&target)
                };
                let normalized_target = super::normalize_without_fs(&resolved);
                return !normalized_target.starts_with(&normalized_effective_root);
            }
            // Cannot read the link — conservatively deny.
            return true;
        }

        // 2. Path doesn't exist — canonicalize the deepest existing ancestor,
        //    then re-attach the missing suffix. Handles nested new paths such
        //    as `file.write "nested/new.txt"` and keeps `/var` vs
        //    `/private/var` aliases aligned on macOS.
        if let Some(reconstructed_path) = reconstruct_from_existing_ancestor(&combined) {
            let normalized_reconstructed = super::normalize_without_fs(&reconstructed_path);
            return !normalized_reconstructed.starts_with(&normalized_effective_root);
        }

        // 3. Neither path nor parent exists — fall back to pure normalization.
        let normalized = super::normalize_without_fs(&combined);
        !normalized.starts_with(&normalized_effective_root)
    }

    fn raw_paths_for_request<'a>(
        tool_name: &str,
        payload: &'a serde_json::Map<String, serde_json::Value>,
    ) -> Vec<&'a str> {
        let tool_name = super::canonical_tool_name(tool_name);
        let mut raw_paths = Vec::new();

        if tool_name == "config.import" {
            let input_path = trimmed_non_empty_path(payload.get("input_path"));
            if let Some(input_path) = input_path {
                raw_paths.push(input_path);
            }

            let output_path = trimmed_non_empty_path(payload.get("output_path"));
            if let Some(output_path) = output_path {
                raw_paths.push(output_path);
            }

            return raw_paths;
        }

        let raw_path = trimmed_non_empty_path(payload.get("path"));
        if let Some(raw_path) = raw_path {
            raw_paths.push(raw_path);
        }

        raw_paths
    }

    fn authorize_file_payload(
        &self,
        tool_name: &str,
        payload: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), PolicyError> {
        let Some(root) = self.file_root.as_deref() else {
            return Ok(());
        };

        let raw_paths = Self::raw_paths_for_request(tool_name, payload);
        if raw_paths.is_empty() {
            return Ok(());
        }

        for raw_path in raw_paths {
            let escapes_root = self.path_escapes_root(raw_path);
            if !escapes_root {
                continue;
            }

            let reason = format!("path `{raw_path}` escapes file root `{}`", root.display());
            return Err(PolicyError::ExtensionDenied {
                extension: self.name().to_owned(),
                reason,
            });
        }

        Ok(())
    }
}

fn reconstruct_from_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut suffix_components = Vec::new();
    let mut cursor = path;

    while !cursor.exists() {
        let component = cursor.file_name()?;
        suffix_components.push(component.to_os_string());
        cursor = cursor.parent()?;
    }

    let mut reconstructed = cursor.canonicalize().ok()?;
    for component in suffix_components.iter().rev() {
        reconstructed.push(component);
    }

    Some(reconstructed)
}

fn trimmed_non_empty_path(value: Option<&serde_json::Value>) -> Option<&str> {
    let raw_value = value.and_then(serde_json::Value::as_str);
    let trimmed_value = raw_value.map(str::trim);

    trimmed_value.filter(|value| !value.is_empty())
}

pub(crate) fn authorize_direct_file_payload(
    tool_name: &str,
    payload: &serde_json::Map<String, serde_json::Value>,
    rt: &super::runtime_config::ToolRuntimeConfig,
) -> Result<(), String> {
    let extension = FilePolicyExtension::new(rt.file_root.clone());
    extension
        .authorize_file_payload(tool_name, payload)
        .map_err(|error| format!("policy_denied: {error}"))
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

        let tool_name = super::canonical_tool_name(raw_tool_name);

        let payload = params.get("payload").and_then(serde_json::Value::as_object);
        let Some(payload) = payload else {
            return Ok(());
        };

        let required_capabilities = Self::required_capabilities(tool_name, payload);
        if required_capabilities.is_empty() {
            return Ok(());
        }

        for required_capability in required_capabilities {
            let capability_is_allowed = context
                .token
                .allowed_capabilities
                .contains(&required_capability);
            if capability_is_allowed {
                continue;
            }

            let extension = self.name().to_owned();
            let reason = format!(
                "tool `{tool_name}` requires capability `{required_capability:?}` not granted to token"
            );
            return Err(PolicyError::ExtensionDenied { extension, reason });
        }

        self.authorize_file_payload(tool_name, payload)?;

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
    fn denies_file_edit_without_capability() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({
            "tool_name": "file.edit",
            "payload": {"path": "foo.txt", "old_string": "a", "new_string": "b"}
        });
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
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
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
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
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
    fn config_import_requires_filesystem_read() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params =
            json!({"tool_name": "config.import", "payload": {"input_path": "config.toml"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn config_import_allowed_with_filesystem_read() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params =
            json!({"tool_name": "config.import", "payload": {"input_path": "config.toml"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn memory_search_requires_filesystem_read() {
        let payload = serde_json::Map::new();
        let required = FilePolicyExtension::required_capabilities("memory_search", &payload);
        assert_eq!(required, BTreeSet::from([Capability::FilesystemRead]));

        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "memory_search", "payload": {"query": "deploy"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn memory_search_allowed_with_filesystem_read() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "memory_search", "payload": {"query": "deploy"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn memory_get_requires_filesystem_read() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "memory_get", "payload": {"path": "MEMORY.md"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn memory_get_allowed_with_filesystem_read() {
        let payload = serde_json::Map::new();
        let required = FilePolicyExtension::required_capabilities("memory_get", &payload);
        assert_eq!(required, BTreeSet::from([Capability::FilesystemRead]));

        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "memory_get", "payload": {"path": "MEMORY.md"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn normalizes_file_read_underscore_alias() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([Capability::InvokeTool]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file_read", "payload": {"path": "foo.txt"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn no_path_check_when_file_root_is_none() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({"tool_name": "file.read", "payload": {"path": "../../etc/passwd"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        // No file_root means no escape check — allowed
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn denies_absolute_path_outside_root() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let outside_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let escape_path = outside_dir.path().join("outside.txt");
        let params = json!({
            "tool_name": "file.read",
            "payload": {"path": escape_path.display().to_string()}
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn config_import_sandbox_uses_input_path_key() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        // input_path escapes the root — must be denied
        let params =
            json!({"tool_name": "config.import", "payload": {"input_path": "../../etc/passwd"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn config_import_within_root_allowed() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params =
            json!({"tool_name": "config.import", "payload": {"input_path": "subdir/config.toml"}});
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(ext.authorize_extension(&ctx).is_ok());
    }

    #[test]
    fn config_import_apply_checks_output_path() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({
            "tool_name": "config.import",
            "payload": {
                "mode": "apply",
                "input_path": "subdir/config.toml",
                "output_path": "../../etc/passwd"
            }
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn config_import_plan_checks_trimmed_output_preview_path() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({
            "tool_name": "config.import",
            "payload": {
                "mode": "plan",
                "input_path": "subdir/config.toml",
                "output_path": " ../../etc/passwd "
            }
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        let result = ext.authorize_extension(&ctx);
        assert!(matches!(
            result.unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn config_import_apply_requires_filesystem_write() {
        let ext = FilePolicyExtension::new(None);
        let pack = test_pack();
        let token = token_with_caps(BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
        ]));
        let caps = BTreeSet::from([Capability::InvokeTool]);
        let params = json!({
            "tool_name": "config.import",
            "payload": {
                "mode": "apply",
                "input_path": "config.toml",
                "output_path": "loongclaw.toml"
            }
        });
        let ctx = make_context(&pack, &token, &caps, Some(&params));
        assert!(matches!(
            ext.authorize_extension(&ctx).unwrap_err(),
            PolicyError::ExtensionDenied { .. }
        ));
    }

    #[test]
    fn nested_new_path_within_root_is_allowed() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));

        assert!(
            !ext.path_escapes_root("nested/generated/loongclaw.toml"),
            "nested new path under the file root should stay allowed"
        );
    }

    #[test]
    fn lexical_parent_segments_after_missing_path_still_escape_root() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let nested_dir = root_dir.path().join("nested");
        std::fs::create_dir_all(&nested_dir).expect("create nested dir");

        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));

        assert!(
            ext.path_escapes_root("nested/missing/../../../outside.txt"),
            "lexical parent segments should not bypass the file root after ancestor reconstruction"
        );
    }

    #[test]
    fn allows_path_within_lexically_normalized_missing_root() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let raw_root = root_dir.path().join("missing").join("..");
        let ext = FilePolicyExtension::new(Some(raw_root));

        assert!(
            !ext.path_escapes_root("inside.txt"),
            "normalized missing-root paths should still allow in-root files"
        );
    }

    #[test]
    fn direct_file_payload_authorization_accepts_config_import_aliases() {
        let root_dir = tempfile::tempdir().expect("tempdir");
        let runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
            file_root: Some(root_dir.path().to_path_buf()),
            ..crate::tools::runtime_config::ToolRuntimeConfig::default()
        };
        let payload_value = json!({
            "mode": "apply",
            "input_path": "legacy-config.toml",
            "output_path": "../outside.toml"
        });
        let payload = payload_value
            .as_object()
            .cloned()
            .expect("payload should be an object");

        let error = authorize_direct_file_payload("claw.migrate", &payload, &runtime_config)
            .expect_err("alias should still reuse config.import direct file policy checks");

        assert!(error.starts_with("policy_denied: "));
        assert!(error.contains("outside.toml"));
    }

    // ── Symlink-aware filesystem tests ──────────────────────────────────

    /// Try to create a symlink, returning `false` if the OS does not support
    /// it without elevated privileges (Windows without Developer Mode).
    fn try_symlink(original: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(original, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (original, link);
            false
        }
    }

    /// Try to create a directory symlink (needed on Windows for dir targets).
    fn try_symlink_dir(original: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(original, link).is_ok()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (original, link);
            false
        }
    }

    #[test]
    fn denies_symlink_pointing_outside_root() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "sensitive").unwrap();

        let root_dir = tempfile::tempdir().unwrap();
        let link = root_dir.path().join("escape_link");
        if !try_symlink(&secret, &link) {
            return; // symlinks not supported — skip
        }

        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        assert!(ext.path_escapes_root(link.to_str().unwrap()));
    }

    #[test]
    fn denies_dangling_symlink_pointing_outside_root() {
        let outside = tempfile::tempdir().unwrap();
        let root_dir = tempfile::tempdir().unwrap();
        let link = root_dir.path().join("dangling_link");
        // Target does not exist and is outside root.
        let nonexistent_outside = outside.path().join("nonexistent_target");
        if !try_symlink(&nonexistent_outside, &link) {
            return;
        }

        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        assert!(ext.path_escapes_root(link.to_str().unwrap()));
    }

    #[test]
    fn allows_symlink_within_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let real_file = root_dir.path().join("real.txt");
        std::fs::write(&real_file, "ok").unwrap();

        let link = root_dir.path().join("internal_link");
        if !try_symlink(&real_file, &link) {
            return;
        }

        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        assert!(!ext.path_escapes_root(link.to_str().unwrap()));
    }

    #[test]
    fn denies_dangling_relative_symlink_via_symlinked_parent_directory() {
        let outside = tempfile::tempdir().unwrap();
        let outside_parent = outside.path().join("real-parent");
        std::fs::create_dir_all(&outside_parent).unwrap();

        let nested_link = outside_parent.join("dangling_relative_link");
        let relative_target = Path::new("..").join("secret.txt");
        if !try_symlink(&relative_target, &nested_link) {
            return;
        }

        let root_dir = tempfile::tempdir().unwrap();
        let linked_parent = root_dir.path().join("linked-parent");
        if !try_symlink_dir(&outside_parent, &linked_parent) {
            return;
        }

        let escaped_path = linked_parent.join("dangling_relative_link");
        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));

        assert!(
            ext.path_escapes_root(escaped_path.to_str().unwrap()),
            "relative dangling targets must resolve against the canonical symlink parent"
        );
    }

    #[test]
    fn denies_symlink_in_parent_directory() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("sub")).unwrap();
        std::fs::write(outside.path().join("sub/target.txt"), "sensitive").unwrap();

        let root_dir = tempfile::tempdir().unwrap();
        let link_dir = root_dir.path().join("linked_dir");
        if !try_symlink_dir(outside.path(), &link_dir) {
            return;
        }

        let ext = FilePolicyExtension::new(Some(root_dir.path().to_path_buf()));
        let escape_path = link_dir.join("sub").join("target.txt");
        assert!(ext.path_escapes_root(escape_path.to_str().unwrap()));
    }
}
