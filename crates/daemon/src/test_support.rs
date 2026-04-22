use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::mvp;
use crate::{Capability, OperationSpec};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;

pub use crate::SecurityScanProfile;

static DAEMON_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());
static DAEMON_GATEWAY_CONTROL_SURFACE_LOCK: Mutex<()> = Mutex::new(());

pub fn lock_daemon_test_environment() -> MutexGuard<'static, ()> {
    DAEMON_TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub fn lock_gateway_control_surface_tests() -> MutexGuard<'static, ()> {
    DAEMON_GATEWAY_CONTROL_SURFACE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn set_test_env_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so no concurrent env readers or writers
    // observe racy updates while these helpers are active.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_test_env_var(key: impl AsRef<OsStr>) {
    // SAFETY: daemon tests serialize process env mutations behind
    // `lock_daemon_test_environment`, so removals are coordinated with all
    // other daemon-side env mutation helpers.
    #[allow(unsafe_code, clippy::disallowed_methods)]
    unsafe {
        std::env::remove_var(key);
    }
}

pub struct ScopedEnv {
    saved: Vec<(String, Option<OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl ScopedEnv {
    // Daemon-side tests must share this guard so every env mutation in the
    // daemon test process is serialized behind one lock.
    pub fn new() -> Self {
        let lock = lock_daemon_test_environment();
        let saved = Vec::new();
        Self { saved, _lock: lock }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl AsRef<OsStr>) {
        let key = key.into();
        self.capture_original(&key);
        set_test_env_var(&key, value);
    }

    pub fn remove(&mut self, key: impl Into<String>) {
        let key = key.into();
        self.capture_original(&key);
        remove_test_env_var(&key);
    }

    fn capture_original(&mut self, key: &str) {
        let already_saved = self.saved.iter().any(|(saved_key, _)| saved_key == key);
        if already_saved {
            return;
        }

        let original_value = std::env::var_os(key);
        let saved_key = key.to_owned();
        self.saved.push((saved_key, original_value));
    }
}

impl Drop for ScopedEnv {
    fn drop(&mut self) {
        while let Some((key, original_value)) = self.saved.pop() {
            match original_value {
                Some(value) => set_test_env_var(&key, value),
                None => remove_test_env_var(&key),
            }
        }
    }
}

#[cfg(unix)]
pub fn write_executable_script_atomically(script_path: &Path, contents: &str) {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static STAGED_SCRIPT_NONCE: AtomicUsize = AtomicUsize::new(0);

    let parent = script_path
        .parent()
        .expect("script path should have parent directory");
    fs::create_dir_all(parent).expect("create script parent directory");
    let staged_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        script_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("script file name"),
        std::process::id(),
        STAGED_SCRIPT_NONCE.fetch_add(1, Ordering::Relaxed)
    ));

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_path)
        .expect("create staged executable script");
    file.write_all(contents.as_bytes())
        .expect("write staged executable script");
    file.sync_all().expect("sync staged executable script");
    drop(file);

    let mut permissions = fs::metadata(&staged_path)
        .expect("staged script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&staged_path, permissions).expect("chmod staged executable script");
    fs::rename(&staged_path, script_path).expect("rename staged executable script");
}

pub fn catalog_entry(raw: &str) -> mvp::channel::ChannelCatalogEntry {
    mvp::channel::resolve_channel_catalog_entry(raw).expect("channel catalog entry")
}

pub fn catalog_command_family(raw: &str) -> mvp::channel::ChannelCatalogCommandFamilyDescriptor {
    mvp::channel::resolve_channel_catalog_command_family_descriptor(raw)
        .expect("channel catalog command family")
}

pub fn channel_send_command(raw: &str) -> &'static str {
    catalog_command_family(raw).send.command
}

pub fn channel_serve_command(raw: &str) -> &'static str {
    catalog_command_family(raw).serve.command
}

pub fn channel_capability_ids(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .capabilities
        .into_iter()
        .map(|capability| capability.as_str())
        .collect()
}

pub fn channel_supported_target_kinds(raw: &str) -> Vec<&'static str> {
    catalog_entry(raw)
        .supported_target_kinds
        .into_iter()
        .map(|kind| kind.as_str())
        .collect()
}

pub fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
    OperationSpec::ToolCore {
        tool_name: tool_name.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload,
        core: None,
    }
}

pub fn write_temp_risk_profile(path: &Path, body: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("temp risk profile path should have parent directory"),
    )
    .expect("create temp risk profile directory");
    fs::write(path, body).expect("write temp risk profile");
}

pub fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
    use crate::security_scan_profile_message;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&security_scan_profile_message(profile));
    let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
    (public_key_base64, signature_base64)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::ScopedEnv;
    use syn::visit::{self, Visit};

    fn daemon_source_uses_forbidden_env_guard(source: &str) -> bool {
        let syntax = syn::parse_file(source).expect("parse daemon source for env guard");
        let mut inspector = DaemonSourceGuardInspector::new();
        inspector.visit_file(&syntax);
        inspector.has_forbidden_reference
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ForbiddenImportKind {
        Module,
        ScopedEnv,
    }

    #[derive(Debug, Default)]
    struct ForbiddenAliasScope {
        imported_bindings: BTreeMap<String, Vec<String>>,
        imported_paths: Vec<Vec<String>>,
        glob_import_paths: Vec<Vec<String>>,
        scoped_env_visible_from_glob_import: bool,
    }

    #[derive(Debug, Default)]
    struct DaemonSourceGuardInspector {
        alias_scopes: Vec<ForbiddenAliasScope>,
        has_forbidden_reference: bool,
    }

    impl DaemonSourceGuardInspector {
        fn new() -> Self {
            let root_scope = ForbiddenAliasScope::default();
            let alias_scopes = vec![root_scope];
            Self {
                alias_scopes,
                has_forbidden_reference: false,
            }
        }

        fn push_scope(&mut self) {
            let scope = ForbiddenAliasScope::default();
            self.alias_scopes.push(scope);
        }

        fn pop_scope(&mut self) {
            let can_pop = self.alias_scopes.len() > 1;

            debug_assert!(can_pop, "daemon source guard should retain the root scope");

            if can_pop {
                let _ = self.alias_scopes.pop();
            }
        }

        fn mark_forbidden_reference(&mut self) {
            self.has_forbidden_reference = true;
        }

        fn current_scope_mut(&mut self) -> &mut ForbiddenAliasScope {
            self.alias_scopes
                .last_mut()
                .expect("daemon source guard scope should exist")
        }

        fn record_alias(&mut self, alias: String, imported_path: Vec<String>) {
            let current_scope = self.current_scope_mut();
            current_scope.imported_bindings.insert(alias, imported_path);
        }

        fn lookup_alias(&self, alias: &str) -> Option<Vec<String>> {
            for scope in self.alias_scopes.iter().rev() {
                let imported_path = scope.imported_bindings.get(alias);

                if let Some(imported_path) = imported_path {
                    return Some(imported_path.clone());
                }
            }

            None
        }

        fn resolve_path_aliases(&self, path_segments: &[String]) -> Vec<String> {
            let mut resolved_segments = path_segments.to_vec();
            let mut seen_alias_segments = BTreeSet::new();

            loop {
                let mut alias_index = 0;

                while let Some(segment) = resolved_segments.get(alias_index) {
                    let is_relative_qualifier = is_relative_path_qualifier(segment);

                    if !is_relative_qualifier {
                        break;
                    }

                    alias_index += 1;
                }

                let alias_segment = match resolved_segments.get(alias_index) {
                    Some(alias_segment) => alias_segment,
                    None => break,
                };
                let imported_path = match self.lookup_alias(alias_segment) {
                    Some(imported_path) => imported_path,
                    None => break,
                };
                let inserted_new_alias_segment = seen_alias_segments.insert(alias_segment.clone());
                let already_saw_alias_segment = !inserted_new_alias_segment;

                if already_saw_alias_segment {
                    break;
                }

                let trailing_segments = &resolved_segments[(alias_index + 1)..];
                // Replace the alias with its imported path directly.
                // Carrying the original relative qualifier chain forward can
                // create unbounded growth for imports like `use super::ScopedEnv;`.
                let mut expanded_segments = imported_path;

                for trailing_segment in trailing_segments {
                    expanded_segments.push(trailing_segment.clone());
                }

                let expansion_changed = expanded_segments != resolved_segments;

                if !expansion_changed {
                    break;
                }

                resolved_segments = expanded_segments;
            }

            resolved_segments
        }

        fn record_glob_import_path(&mut self, imported_path: Vec<String>) {
            let current_scope = self.current_scope_mut();
            current_scope.glob_import_paths.push(imported_path);
        }

        fn path_uses_glob_visible_scoped_env(path_segments: &[String]) -> bool {
            for qualifier_segment in path_segments {
                let is_relative_qualifier = is_relative_path_qualifier(qualifier_segment);

                if is_relative_qualifier {
                    continue;
                }

                return qualifier_segment == "ScopedEnv";
            }

            false
        }

        fn path_uses_forbidden_scoped_env(&self, path: &syn::Path) -> bool {
            let path_segments = path_segment_names(path);
            let uses_forbidden_path = path_contains_forbidden_scoped_env_path(&path_segments);

            if uses_forbidden_path {
                return true;
            }

            let resolved_path_segments = self.resolve_path_aliases(&path_segments);
            let uses_resolved_path =
                path_contains_forbidden_scoped_env_path(&resolved_path_segments);

            if uses_resolved_path {
                return true;
            }

            let uses_glob_visible_scoped_env =
                Self::path_uses_glob_visible_scoped_env(&path_segments);

            if uses_glob_visible_scoped_env {
                for scope in self.alias_scopes.iter().rev() {
                    let scoped_env_visible_from_glob_import =
                        scope.scoped_env_visible_from_glob_import;

                    if scoped_env_visible_from_glob_import {
                        return true;
                    }
                }
            }

            false
        }

        fn inspect_use_tree(&mut self, use_tree: &syn::UseTree, prefix: &mut Vec<String>) {
            match use_tree {
                syn::UseTree::Path(use_path) => {
                    let segment_name = use_path.ident.to_string();
                    prefix.push(segment_name);
                    self.inspect_use_tree(&use_path.tree, prefix);
                    let _ = prefix.pop();
                }
                syn::UseTree::Name(use_name) => {
                    let imported_name = use_name.ident.to_string();
                    let imported_path = build_imported_path(prefix, &imported_name);
                    self.record_import_path(imported_path, None);
                }
                syn::UseTree::Rename(use_rename) => {
                    let imported_name = use_rename.ident.to_string();
                    let imported_path = build_imported_path(prefix, &imported_name);
                    let alias = use_rename.rename.to_string();
                    self.record_import_path(imported_path, Some(alias));
                }
                syn::UseTree::Glob(_) => {
                    let imported_path = prefix.clone();
                    self.record_glob_import_path(imported_path);
                }
                syn::UseTree::Group(use_group) => {
                    for child_tree in &use_group.items {
                        self.inspect_use_tree(child_tree, prefix);
                    }
                }
            }
        }

        fn record_import_path(&mut self, imported_path: Vec<String>, alias: Option<String>) {
            let alias_name = match alias {
                Some(alias_name) => alias_name,
                None => imported_binding_name(&imported_path),
            };
            {
                let current_scope = self.current_scope_mut();
                current_scope.imported_paths.push(imported_path.clone());
            }
            self.record_alias(alias_name, imported_path);
        }

        fn collect_item_uses(&mut self, items: &[syn::Item]) {
            for item in items {
                let syn::Item::Use(item_use) = item else {
                    continue;
                };
                self.collect_single_item_use(item_use);
            }
        }

        fn collect_single_item_use(&mut self, item_use: &syn::ItemUse) {
            let mut prefix = Vec::new();
            self.inspect_use_tree(&item_use.tree, &mut prefix);
        }

        fn current_scope_use_effect_counts(&self) -> (usize, usize) {
            let current_scope = self
                .alias_scopes
                .last()
                .expect("daemon source guard scope should exist");
            let imported_path_count = current_scope.imported_paths.len();
            let glob_import_path_count = current_scope.glob_import_paths.len();

            (imported_path_count, glob_import_path_count)
        }

        fn refresh_current_scope_use_effects_since(
            &mut self,
            imported_path_start: usize,
            glob_import_path_start: usize,
        ) {
            let (imported_paths, glob_import_paths, scoped_env_visible_from_glob_import) = {
                let current_scope = self
                    .alias_scopes
                    .last()
                    .expect("daemon source guard scope should exist");
                let imported_paths = current_scope.imported_paths[imported_path_start..].to_vec();
                let glob_import_paths =
                    current_scope.glob_import_paths[glob_import_path_start..].to_vec();
                let scoped_env_visible_from_glob_import =
                    current_scope.scoped_env_visible_from_glob_import;

                (
                    imported_paths,
                    glob_import_paths,
                    scoped_env_visible_from_glob_import,
                )
            };
            let mut scoped_env_visible_from_glob_import = scoped_env_visible_from_glob_import;

            for imported_path in &imported_paths {
                let resolved_imported_path = self.resolve_path_aliases(imported_path);
                let import_kind = forbidden_import_kind_for_use_path(&resolved_imported_path);
                let imports_scoped_env =
                    matches!(import_kind, Some(ForbiddenImportKind::ScopedEnv));

                if imports_scoped_env {
                    self.mark_forbidden_reference();
                }
            }

            for glob_import_path in &glob_import_paths {
                let resolved_imported_path = self.resolve_path_aliases(glob_import_path);
                let import_kind = forbidden_import_kind_for_use_path(&resolved_imported_path);

                match import_kind {
                    Some(ForbiddenImportKind::Module) => {
                        scoped_env_visible_from_glob_import = true;
                    }
                    Some(ForbiddenImportKind::ScopedEnv) => {
                        self.mark_forbidden_reference();
                    }
                    None => {}
                }
            }

            let current_scope = self.current_scope_mut();
            current_scope.scoped_env_visible_from_glob_import = scoped_env_visible_from_glob_import;
        }
    }

    impl<'ast> Visit<'ast> for DaemonSourceGuardInspector {
        fn visit_file(&mut self, file: &'ast syn::File) {
            self.collect_item_uses(&file.items);
            self.refresh_current_scope_use_effects_since(0, 0);

            for item in &file.items {
                self.visit_item(item);
            }
        }

        fn visit_item_mod(&mut self, item_mod: &'ast syn::ItemMod) {
            let module_content = match &item_mod.content {
                Some(module_content) => module_content,
                None => return,
            };
            let (_, items) = module_content;

            self.push_scope();
            self.collect_item_uses(items);
            self.refresh_current_scope_use_effects_since(0, 0);

            for item in items {
                self.visit_item(item);
            }

            self.pop_scope();
        }

        fn visit_block(&mut self, block: &'ast syn::Block) {
            self.push_scope();
            let mut pending_use_items = Vec::new();

            for statement in &block.stmts {
                if let syn::Stmt::Item(syn::Item::Use(item_use)) = statement {
                    pending_use_items.push(item_use);
                    continue;
                }

                let has_pending_use_items = !pending_use_items.is_empty();

                if has_pending_use_items {
                    let (imported_path_start, glob_import_path_start) =
                        self.current_scope_use_effect_counts();

                    for pending_use_item in pending_use_items.drain(..) {
                        self.collect_single_item_use(pending_use_item);
                    }

                    self.refresh_current_scope_use_effects_since(
                        imported_path_start,
                        glob_import_path_start,
                    );
                }

                self.visit_stmt(statement);
            }

            let has_pending_use_items = !pending_use_items.is_empty();

            if has_pending_use_items {
                let (imported_path_start, glob_import_path_start) =
                    self.current_scope_use_effect_counts();

                for pending_use_item in pending_use_items.drain(..) {
                    self.collect_single_item_use(pending_use_item);
                }

                self.refresh_current_scope_use_effects_since(
                    imported_path_start,
                    glob_import_path_start,
                );
            }

            self.pop_scope();
        }

        fn visit_item_use(&mut self, _item_use: &'ast syn::ItemUse) {}

        fn visit_path(&mut self, path: &'ast syn::Path) {
            let uses_forbidden_env_guard = self.path_uses_forbidden_scoped_env(path);

            if uses_forbidden_env_guard {
                self.mark_forbidden_reference();
            }

            visit::visit_path(self, path);
        }
    }

    fn build_imported_path(prefix: &[String], imported_name: &str) -> Vec<String> {
        if imported_name == "self" {
            return prefix.to_vec();
        }

        let mut imported_path = prefix.to_vec();
        let imported_name = imported_name.to_owned();
        imported_path.push(imported_name);
        imported_path
    }

    fn imported_binding_name(imported_path: &[String]) -> String {
        imported_path
            .last()
            .cloned()
            .expect("forbidden import path should have a binding name")
    }

    fn is_relative_path_qualifier(segment: &str) -> bool {
        matches!(segment, "self" | "super" | "crate")
    }

    fn forbidden_import_kind_for_use_path(imported_path: &[String]) -> Option<ForbiddenImportKind> {
        let imports_scoped_env = path_ends_with_forbidden_scoped_env_path(imported_path);

        if imports_scoped_env {
            return Some(ForbiddenImportKind::ScopedEnv);
        }

        let imports_test_support_module =
            path_ends_with_forbidden_test_support_module(imported_path);

        if imports_test_support_module {
            return Some(ForbiddenImportKind::Module);
        }

        None
    }

    fn path_segment_names(path: &syn::Path) -> Vec<String> {
        let mut segments = Vec::new();

        for segment in &path.segments {
            let segment_name = segment.ident.to_string();
            segments.push(segment_name);
        }

        segments
    }

    fn path_contains_forbidden_scoped_env_path(path_segments: &[String]) -> bool {
        let uses_mvp_scoped_env =
            path_segments_contain_sequence(path_segments, &["mvp", "test_support", "ScopedEnv"]);

        if uses_mvp_scoped_env {
            return true;
        }

        path_segments_contain_sequence(path_segments, &["loong_app", "test_support", "ScopedEnv"])
    }

    fn path_ends_with_forbidden_scoped_env_path(path_segments: &[String]) -> bool {
        let imports_mvp_scoped_env =
            path_segments_end_with_sequence(path_segments, &["mvp", "test_support", "ScopedEnv"]);

        if imports_mvp_scoped_env {
            return true;
        }

        path_segments_end_with_sequence(path_segments, &["loong_app", "test_support", "ScopedEnv"])
    }

    fn path_ends_with_forbidden_test_support_module(path_segments: &[String]) -> bool {
        let imports_mvp_test_support =
            path_segments_end_with_sequence(path_segments, &["mvp", "test_support"]);

        if imports_mvp_test_support {
            return true;
        }

        path_segments_end_with_sequence(path_segments, &["loong_app", "test_support"])
    }

    fn path_segments_contain_sequence(path_segments: &[String], sequence: &[&str]) -> bool {
        if path_segments.len() < sequence.len() {
            return false;
        }

        let last_start = path_segments.len() - sequence.len();

        for start in 0..=last_start {
            let mut matches_sequence = true;

            for (offset, expected_segment) in sequence.iter().enumerate() {
                let actual_segment = &path_segments[start + offset];
                let matches_segment = actual_segment == expected_segment;

                if !matches_segment {
                    matches_sequence = false;
                    break;
                }
            }

            if matches_sequence {
                return true;
            }
        }

        false
    }

    fn path_segments_end_with_sequence(path_segments: &[String], sequence: &[&str]) -> bool {
        if path_segments.len() < sequence.len() {
            return false;
        }

        let start = path_segments.len() - sequence.len();
        let tail_segments = &path_segments[start..];

        for (actual_segment, expected_segment) in tail_segments.iter().zip(sequence.iter()) {
            let matches_segment = actual_segment == expected_segment;

            if !matches_segment {
                return false;
            }
        }

        true
    }

    fn daemon_guard_scan_roots(manifest_dir: &Path) -> Vec<PathBuf> {
        let source_root = manifest_dir.join("src");
        let tests_root = manifest_dir.join("tests");
        let candidate_roots = [source_root, tests_root];
        let mut scan_roots = Vec::new();

        for candidate_root in candidate_roots {
            let root_exists = candidate_root.is_dir();

            if root_exists {
                scan_roots.push(candidate_root);
            }
        }

        scan_roots.sort();
        scan_roots
    }

    fn collect_rust_source_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
        let mut pending_paths = roots.to_vec();
        let mut rust_source_paths = Vec::new();

        while let Some(current_path) = pending_paths.pop() {
            let read_dir = fs::read_dir(&current_path).expect("read daemon source directory");
            let mut child_paths = Vec::new();

            for entry in read_dir {
                let entry = entry.expect("daemon source directory entry");
                let child_path = entry.path();
                child_paths.push(child_path);
            }

            child_paths.sort();

            for child_path in child_paths {
                if child_path.is_dir() {
                    pending_paths.push(child_path);
                    continue;
                }

                let extension = child_path.extension();
                let rust_extension = Some(std::ffi::OsStr::new("rs"));

                if extension == rust_extension {
                    rust_source_paths.push(child_path);
                }
            }
        }

        rust_source_paths.sort();
        rust_source_paths
    }

    #[test]
    fn scoped_env_remove_restores_original_value() {
        let key = "LOONG_SCOPED_ENV_REMOVE_TEST_KEY";
        let sentinel_value = "scoped-env-sentinel";
        let mut env = ScopedEnv::new();
        let original_value = std::env::var_os(key);

        env.set(key, sentinel_value);
        env.remove(key);

        assert!(
            std::env::var_os(key).is_none(),
            "ScopedEnv::remove should clear the environment variable while the guard is alive"
        );

        drop(env);

        assert_eq!(
            std::env::var_os(key),
            original_value,
            "ScopedEnv should restore the original environment value when dropped"
        );
    }

    #[test]
    fn daemon_source_guard_flags_mvp_scoped_env_reference() {
        let base_path = ["mvp", "::test_support"].concat();
        let scoped_env_name = ["Scoped", "Env"].concat();
        let sample_source = format!(
            "fn build_guard() {{
                let mut env = {base_path}::{scoped_env_name}::new();
                drop(env);
            }}"
        );

        assert!(
            daemon_source_uses_forbidden_env_guard(&sample_source),
            "daemon source guard should flag direct mvp scoped env references"
        );
    }

    #[test]
    fn daemon_source_guard_flags_loong_app_scoped_env_reference() {
        let base_path = ["loong_app", "::test_support"].concat();
        let scoped_env_name = ["Scoped", "Env"].concat();
        let sample_source = format!(
            "fn build_guard() {{
                let mut env = {base_path}::{scoped_env_name}::new();
                drop(env);
            }}"
        );

        assert!(
            daemon_source_uses_forbidden_env_guard(&sample_source),
            "daemon source guard should flag direct loong_app scoped env references"
        );
    }

    #[test]
    fn daemon_source_guard_accepts_daemon_scoped_env_reference() {
        let sample_source = r#"
            fn build_guard() {
                let mut env = crate::test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should allow daemon scoped env references"
        );
    }

    #[test]
    fn daemon_source_guard_flags_mvp_scoped_env_brace_import() {
        let prefix = ["crate::", "mvp", "::test_support"].concat();
        let scoped_env_name = ["Scoped", "Env"].concat();
        let sample_source = format!("use {prefix}::{{self,{scoped_env_name}}};");

        assert!(
            daemon_source_uses_forbidden_env_guard(&sample_source),
            "daemon source guard should flag mvp scoped env brace imports"
        );
    }

    #[test]
    fn daemon_source_guard_flags_multiline_mvp_scoped_env_brace_import() {
        let prefix = ["crate::", "mvp", "::test_support"].concat();
        let scoped_env_name = ["Scoped", "Env"].concat();
        let sample_source = format!(
            "use {prefix}::{{
                self,
                {scoped_env_name},
            }};"
        );

        assert!(
            daemon_source_uses_forbidden_env_guard(&sample_source),
            "daemon source guard should flag multiline mvp scoped env brace imports"
        );
    }

    #[test]
    fn daemon_source_guard_flags_alias_to_forbidden_test_support_module() {
        let sample_source = r#"
            use crate::mvp::test_support as app_test_support;

            fn build_guard() {
                let mut env = app_test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag aliases to forbidden test support modules"
        );
    }

    #[test]
    fn daemon_source_guard_flags_alias_to_forbidden_scoped_env_item() {
        let sample_source = r#"
            use loong_app::test_support::ScopedEnv as AppScopedEnv;

            fn build_guard() {
                let mut env = AppScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag aliases to forbidden scoped env items"
        );
    }

    #[test]
    fn daemon_source_guard_flags_alias_to_forbidden_mvp_root_module() {
        let sample_source = r#"
            use crate::mvp as app_side;

            fn build_guard() {
                let mut env = app_side::test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag aliases to the forbidden mvp root module"
        );
    }

    #[test]
    fn daemon_source_guard_flags_alias_to_forbidden_app_root_module() {
        let sample_source = r#"
            use loong_app as app_side;

            fn build_guard() {
                let mut env = app_side::test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag aliases to the forbidden app root module"
        );
    }

    #[test]
    fn daemon_source_guard_flags_long_alias_chain_to_forbidden_root_module() {
        let sample_source = r#"
            use crate::mvp as app_side_0;
            use app_side_0 as app_side_1;
            use app_side_1 as app_side_2;
            use app_side_2 as app_side_3;
            use app_side_3 as app_side_4;
            use app_side_4 as app_side_5;
            use app_side_5 as app_side_6;
            use app_side_6 as app_side_7;
            use app_side_7 as app_side_8;
            use app_side_8 as app_side_9;

            fn build_guard() {
                let mut env = app_side_9::test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag long alias chains that still resolve to the forbidden root module"
        );
    }

    #[test]
    fn daemon_source_guard_flags_glob_import_from_later_root_alias_before_function_use() {
        let sample_source = r#"
            fn build_guard() {
                let mut env = ScopedEnv::new();
                drop(env);
            }

            use app_side::test_support::*;
            use crate::mvp as app_side;
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag glob imports that resolve through a later root alias in the same scope"
        );
    }

    #[test]
    fn daemon_source_guard_flags_import_only_scoped_env_from_later_root_alias() {
        let sample_source = r#"
            use app_side::test_support::ScopedEnv;
            use crate::mvp as app_side;
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag import-only forbidden scoped env paths that resolve through a later root alias"
        );
    }

    #[test]
    fn daemon_source_guard_flags_glob_import_from_later_root_alias_inside_nested_module() {
        let sample_source = r#"
            mod outer {
                fn build_guard() {
                    let mut env = ScopedEnv::new();
                    drop(env);
                }

                use self::app_side::test_support::*;
                use crate::mvp as app_side;
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag nested-module glob imports that resolve through a later root alias"
        );
    }

    #[test]
    fn daemon_source_guard_flags_same_name_root_alias_without_unbounded_growth() {
        let sample_source = r#"
            use crate::mvp as mvp;

            fn build_guard() {
                let mut env = mvp::test_support::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should resolve same-name root aliases without growing the expanded path without bound"
        );
    }

    #[test]
    fn daemon_source_guard_allows_super_scoped_env_import_without_unbounded_growth() {
        let sample_source = r#"
            mod outer {
                pub struct ScopedEnv;

                mod inner {
                    use super::ScopedEnv;

                    fn build_guard() {
                        let _env = ScopedEnv;
                    }
                }
            }
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should allow daemon-local super imports without growing alias resolution without bound"
        );
    }

    #[test]
    fn daemon_source_guard_flags_self_qualified_path_from_later_root_alias_inside_nested_module() {
        let sample_source = r#"
            mod outer {
                fn build_guard() {
                    let mut env = self::app_side::test_support::ScopedEnv::new();
                    drop(env);
                }

                use crate::mvp as app_side;
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag self-qualified nested-module paths that resolve through a later root alias"
        );
    }

    #[test]
    fn daemon_source_guard_allows_non_scoped_env_helper_via_forbidden_module_alias() {
        let sample_source = r#"
            use crate::mvp::test_support as app_test_support;

            fn build_helper() {
                let path = app_test_support::unique_temp_dir("daemon-helper");
                assert!(path.to_string_lossy().contains("daemon-helper"));
            }
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should not flag non-ScopedEnv helper usage through a forbidden test support alias"
        );
    }

    #[test]
    fn daemon_source_guard_allows_non_scoped_env_helper_via_forbidden_glob_import() {
        let sample_source = r#"
            use crate::mvp::test_support::*;

            fn build_helper() {
                let path = unique_temp_dir("daemon-helper");
                assert!(path.to_string_lossy().contains("daemon-helper"));
            }
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should not flag non-ScopedEnv helper usage through a forbidden glob import"
        );
    }

    #[test]
    fn daemon_source_guard_flags_scoped_env_usage_via_forbidden_glob_import() {
        let sample_source = r#"
            use crate::mvp::test_support::*;

            fn build_guard() {
                let mut env = ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag ScopedEnv usage introduced through a forbidden glob import"
        );
    }

    #[test]
    fn daemon_source_guard_flags_self_qualified_scoped_env_via_forbidden_glob_import() {
        let sample_source = r#"
            use crate::mvp::test_support::*;

            fn build_guard() {
                let mut env = self::ScopedEnv::new();
                drop(env);
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag self-qualified ScopedEnv usage introduced through a forbidden glob import"
        );
    }

    #[test]
    fn daemon_source_guard_flags_parent_qualified_scoped_env_via_forbidden_glob_import() {
        let sample_source = r#"
            mod outer {
                use crate::mvp::test_support::*;

                mod inner {
                    fn build_guard() {
                        let mut env = super::ScopedEnv::new();
                        drop(env);
                    }
                }
            }
        "#;

        assert!(
            daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should flag parent-qualified ScopedEnv usage introduced through a forbidden glob import"
        );
    }

    #[test]
    fn daemon_source_guard_ignores_comment_mentions() {
        let sample_source = r#"
            // mvp::test_support::ScopedEnv should stay out of daemon tests.
            fn ok() {}
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should ignore comment-only mentions"
        );
    }

    #[test]
    fn daemon_source_guard_ignores_string_literal_mentions() {
        let sample_source = r#"
            fn note() {
                let message = "mvp::test_support::ScopedEnv is not allowed in daemon tests";
                assert!(!message.is_empty());
            }
        "#;

        assert!(
            !daemon_source_uses_forbidden_env_guard(sample_source),
            "daemon source guard should ignore string literal mentions"
        );
    }

    #[test]
    fn daemon_guard_scan_roots_include_daemon_tests_directory() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let scan_roots = daemon_guard_scan_roots(manifest_dir);
        let tests_root = manifest_dir.join("tests");

        assert!(
            scan_roots.contains(&tests_root),
            "daemon source guard should scan crates/daemon/tests for forbidden env guard usage"
        );
    }

    #[test]
    fn daemon_test_env_source_files_do_not_use_app_scoped_env_guard() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let scan_roots = daemon_guard_scan_roots(manifest_dir);
        let rust_source_paths = collect_rust_source_paths(&scan_roots);
        let mut violating_paths = Vec::new();

        for rust_source_path in rust_source_paths {
            let source = fs::read_to_string(&rust_source_path).expect("read daemon source file");
            let has_forbidden_env_guard = daemon_source_uses_forbidden_env_guard(&source);

            if has_forbidden_env_guard {
                violating_paths.push(rust_source_path);
            }
        }

        assert!(
            violating_paths.is_empty(),
            "daemon source files must not use the app scoped env guard: {violating_paths:?}"
        );
    }
}
