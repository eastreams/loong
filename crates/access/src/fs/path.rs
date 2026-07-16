use std::{
    borrow::Cow,
    ffi::OsString,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::{
    error::AuthorizationError,
    policy::{
        action::{Action, ActionMeta, ActionMetadata},
        context::ContextFactory,
        engine::PolicyEngine,
        grant::Granted,
        policy::Policy,
    },
};
use serde_json::{Value, json};

use super::{
    access::{FsAccess, FsAccessError},
    error::FsActionError,
};

const FS_PATH_REQUIRED_CAPABILITIES: [Capability; 0] = [];

mod sealed {
    pub trait Sealed {}
}

/// Marker for target-following path resolution.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPath;

/// Marker for final-component no-follow path resolution.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPath;

impl sealed::Sealed for TargetPath {}
impl sealed::Sealed for EntryPath {}

/// Sealed path-resolution semantics carried through the fs typestate chain.
///
/// This is public only because it bounds public generic action types. External
/// code cannot implement it, and normal callers use the concrete path aliases.
#[doc(hidden)]
pub trait FsPathMode: sealed::Sealed + Send + Sync + 'static {
    const NAME: &'static str;
    const RESOLVE_OPERATION: &'static str;
    const AUTHORIZE_OPERATION: &'static str;
    const FOLLOWS_FINAL_COMPONENT: bool;
}

impl FsPathMode for TargetPath {
    const NAME: &'static str = "target";
    const RESOLVE_OPERATION: &'static str = "resolve_target_path";
    const AUTHORIZE_OPERATION: &'static str = "authorize_target_path";
    const FOLLOWS_FINAL_COMPONENT: bool = true;
}

impl FsPathMode for EntryPath {
    const NAME: &'static str = "entry";
    const RESOLVE_OPERATION: &'static str = "resolve_entry_path";
    const AUTHORIZE_OPERATION: &'static str = "authorize_entry_path";
    const FOLLOWS_FINAL_COMPONENT: bool = false;
}

/// Filesystem root view required by path resolution.
///
/// Resolving a relative path is part of running `FsResolvePathAction`, not a
/// policy concern. Contexts therefore expose only the root needed to produce
/// resolved path facts at this stage.
pub trait FsResolutionContext {
    fn fs_resolution_root(&self) -> &Path;
}

/// Filesystem root view required by path containment policy.
///
/// Implementors must provide canonical or simplified roots in the same path
/// space as the facts produced by `FsResolvePathAction`.
pub trait FsPathPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf];
}

/// Typed action for resolving one requested path into filesystem facts.
///
/// Construction is pure. Canonicalization and symlink observation happen only
/// when a granted action runs. Its output is resolved but not yet authorized;
/// callers must pass it through `FsPathAction` before a concrete fs action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsResolvePathAction<M = TargetPath> {
    raw_path: PathBuf,
    resolution_root: PathBuf,
    _mode: PhantomData<fn() -> M>,
}

impl FsResolvePathAction<TargetPath> {
    #[must_use]
    pub(in crate::fs) fn target(path: impl AsRef<Path>, resolution_root: impl AsRef<Path>) -> Self {
        Self::new(path, resolution_root)
    }
}

impl FsResolvePathAction<EntryPath> {
    #[must_use]
    pub(in crate::fs) fn entry(path: impl AsRef<Path>, resolution_root: impl AsRef<Path>) -> Self {
        Self::new(path, resolution_root)
    }
}

impl<M> FsResolvePathAction<M>
where
    M: FsPathMode,
{
    fn new(path: impl AsRef<Path>, resolution_root: impl AsRef<Path>) -> Self {
        Self {
            raw_path: path.as_ref().to_path_buf(),
            resolution_root: resolution_root.as_ref().to_path_buf(),
            _mode: PhantomData,
        }
    }

    #[must_use]
    pub fn raw_path(&self) -> &Path {
        &self.raw_path
    }

    #[must_use]
    pub fn resolution_root(&self) -> &Path {
        &self.resolution_root
    }

    fn resolve(self) -> Result<ResolvedFsPath<M>, FsActionError> {
        let resolved = if M::FOLLOWS_FINAL_COMPONENT {
            resolve_target_path(&self.raw_path, &self.resolution_root)?
        } else {
            resolve_entry_path(&self.raw_path, &self.resolution_root)?
        };
        Ok(ResolvedFsPath::new(self.raw_path, resolved))
    }
}

impl<M> ActionMeta for FsResolvePathAction<M>
where
    M: FsPathMode,
{
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.resolve_path",
            operation: Cow::Borrowed(M::RESOLVE_OPERATION),
            required_capabilities: Cow::Borrowed(&FS_PATH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.raw_path.display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.raw_path.display().to_string(),
            "resolution_root": self.resolution_root.display().to_string(),
            "semantics": M::NAME,
        }))
    }
}

/// Explicitly permits access-internal path resolution.
///
/// Resolution produces opaque facts, not filesystem authority. The following
/// `FsPathAction` still must pass allowed-roots policy before any concrete fs
/// action can receive a granted path.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsResolvePathAllowPolicy<M = TargetPath>(PhantomData<fn() -> M>);

impl FsResolvePathAllowPolicy<TargetPath> {
    #[must_use]
    pub const fn target() -> Self {
        Self(PhantomData)
    }
}

impl FsResolvePathAllowPolicy<EntryPath> {
    #[must_use]
    pub const fn entry() -> Self {
        Self(PhantomData)
    }
}

#[async_trait]
impl<C, M> Policy<C, FsResolvePathAction<M>> for FsResolvePathAllowPolicy<M>
where
    C: ContextFactory + Send + Sync,
    M: FsPathMode,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-resolve-path-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsResolvePathAction<M>) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("path resolution does not grant filesystem authority".into()),
            reason: "filesystem path resolution allowed before path authorization".into(),
        }
    }
}

/// Typed action for authorizing one already-resolved filesystem path.
///
/// This action performs no filesystem observation. It presents resolved facts
/// to path policy and mints a `GrantedFsPath` only after policy accepts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPathAction<M = TargetPath> {
    resolved: ResolvedFsPath<M>,
}

impl<M> FsPathAction<M>
where
    M: FsPathMode,
{
    #[must_use]
    pub(in crate::fs) fn new(resolved: ResolvedFsPath<M>) -> Self {
        Self { resolved }
    }

    #[must_use]
    pub fn requested_path(&self) -> &Path {
        self.resolved.requested_path()
    }

    #[must_use]
    pub fn resolved_path(&self) -> &Path {
        self.resolved.path()
    }

    fn into_granted_path(self) -> GrantedFsPath<M> {
        GrantedFsPath::new(self.resolved.into_path_buf())
    }
}

impl<M> ActionMeta for FsPathAction<M>
where
    M: FsPathMode,
{
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.path",
            operation: Cow::Borrowed(M::AUTHORIZE_OPERATION),
            required_capabilities: Cow::Borrowed(&FS_PATH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.requested_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.requested_path().display().to_string(),
            "resolved_path": self.resolved_path().display().to_string(),
            "semantics": M::NAME,
        }))
    }
}

/// Default fs path containment policy.
///
/// A granted resolve action prepares path facts, but containment remains a
/// policy decision so denials produce `PolicyReport` evidence instead of domain
/// action errors. Target-following and entry/no-follow registrations share this
/// implementation while retaining distinct grant types for concrete actions.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsPathAllowedRootsPolicy<M = TargetPath>(PhantomData<fn() -> M>);

impl FsPathAllowedRootsPolicy<TargetPath> {
    #[must_use]
    pub const fn target() -> Self {
        Self(PhantomData)
    }
}

impl FsPathAllowedRootsPolicy<EntryPath> {
    #[must_use]
    pub const fn entry() -> Self {
        Self(PhantomData)
    }
}

#[async_trait]
impl<C, M> Policy<C, FsPathAction<M>> for FsPathAllowedRootsPolicy<M>
where
    C: ContextFactory + Send + Sync,
    M: FsPathMode,
    for<'a> C::Cx<'a>: FsPathPolicyContext,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-path-allowed-roots")
    }

    async fn grant(&self, ctx: &C::Cx<'_>, action: &FsPathAction<M>) -> PolicyGrant {
        let allowed_roots = ctx.fs_allowed_roots();
        if allowed_roots
            .iter()
            .any(|allowed_root| action.resolved_path().starts_with(allowed_root))
        {
            return PolicyGrant {
                decision: PolicyDecision::Allow,
                predicate: Some("resolved fs path starts with an allowed root".into()),
                reason: "resolved fs path is within allowed roots".into(),
            };
        }

        PolicyGrant {
            decision: PolicyDecision::Deny,
            predicate: Some("resolved fs path must start with an allowed root".into()),
            reason: format!(
                "filesystem path {} escapes allowed filesystem roots [{}]",
                action.resolved_path().display(),
                allowed_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into(),
        }
    }
}

/// Resolved path facts produced by running a granted resolve action.
///
/// Resolution does not imply authorization. `FsPathAction` consumes this
/// unforgeable value so policy can decide whether the resolved path is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFsPath<M> {
    requested: PathBuf,
    path: PathBuf,
    _mode: std::marker::PhantomData<fn() -> M>,
}

impl<M> ResolvedFsPath<M> {
    pub(in crate::fs) fn new(requested: PathBuf, path: PathBuf) -> Self {
        Self {
            requested,
            path,
            _mode: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn requested_path(&self) -> &Path {
        &self.requested
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::fs) fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

/// Target-following resolved path facts.
pub type ResolvedPath = ResolvedFsPath<TargetPath>;

/// Final-component no-follow resolved path facts.
pub type ResolvedEntryPath = ResolvedFsPath<EntryPath>;

/// Filesystem path produced by governed path authorization.
///
/// Downstream fs actions accept this value instead of raw paths so their
/// constructors prove that resolve and path policy have already run. Only the
/// fs module can mint one.
///
/// This authorizes the path observed during resolution; it does not pin the
/// underlying inode. A descriptor-relative backend is still required to close
/// races where another actor replaces a path between authorization and use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedFsPath<M> {
    path: PathBuf,
    _mode: std::marker::PhantomData<fn() -> M>,
}

impl<M> GrantedFsPath<M> {
    pub(in crate::fs) fn new(path: PathBuf) -> Self {
        Self {
            path,
            _mode: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl<M> AsRef<Path> for GrantedFsPath<M> {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

/// Governed target-following path used by content and directory operations.
pub type GrantedPath = GrantedFsPath<TargetPath>;

/// Governed entry path used by unlink and rename operations.
pub type GrantedEntryPath = GrantedFsPath<EntryPath>;

/// Resolve one path only after policy grants the observation action.
#[async_trait]
impl<Cx, M> Action<Cx> for FsResolvePathAction<M>
where
    Cx: Sync,
    M: FsPathMode,
{
    type Output = ResolvedFsPath<M>;
    type Error = FsActionError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        granted.into_action().resolve()
    }
}

/// Mint a governed path from facts accepted by path policy.
///
/// This stage performs no filesystem observation. It only consumes the path
/// authorization grant and preserves target/entry typestate for the concrete
/// operation action.
#[async_trait]
impl<Cx, M> Action<Cx> for FsPathAction<M>
where
    Cx: Sync,
    M: FsPathMode,
{
    type Output = GrantedFsPath<M>;
    type Error = FsActionError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        Ok(granted.into_action().into_granted_path())
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Run the mandatory resolve and path-policy stages for a target path.
    ///
    /// Keeping this chain inside access prevents individual operations from
    /// accidentally skipping either grant while preserving both stages as
    /// distinct typed actions for policy and audit.
    pub(in crate::fs) async fn grant_target_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<GrantedPath, FsAccessError> {
        self.grant_path(FsResolvePathAction::target(
            path,
            self.ctx.fs_resolution_root(),
        ))
        .await
    }

    /// Resolve and authorize a final-component no-follow entry path.
    pub(in crate::fs) async fn grant_entry_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<GrantedEntryPath, FsAccessError> {
        self.grant_path(FsResolvePathAction::entry(
            path,
            self.ctx.fs_resolution_root(),
        ))
        .await
    }

    // Both public-to-fs entry points use this exact two-stage order. Keeping the
    // generic chain private prevents operation modules from authorizing resolved
    // facts through a different sequence.
    async fn grant_path<M>(
        &self,
        resolve: FsResolvePathAction<M>,
    ) -> Result<GrantedFsPath<M>, FsAccessError>
    where
        M: FsPathMode,
    {
        let resolved = self
            .policy_engine
            .grant(self.ctx, resolve)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?
            .into_granted()
            .run(self.ctx)
            .await?;
        let path_action = FsPathAction::new(resolved);
        let path = self
            .policy_engine
            .grant(self.ctx, path_action)
            .await
            .map_err(AuthorizationError::from)
            .map_err(FsAccessError::Authorization)?
            .into_granted()
            .run(self.ctx)
            .await?;
        Ok(path)
    }
}

pub(in crate::fs) fn resolve_target_path(
    path: &Path,
    resolution_root: &Path,
) -> Result<PathBuf, FsActionError> {
    if path.as_os_str().is_empty() {
        return Err(FsActionError::EmptyPath);
    }

    let resolution_root = resolve_existing_or_missing_path(resolution_root)?;
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolution_root.join(path)
    };
    resolve_existing_or_missing_path(&combined)
}

pub(in crate::fs) fn resolve_entry_path(
    path: &Path,
    resolution_root: &Path,
) -> Result<PathBuf, FsActionError> {
    if path.as_os_str().is_empty() {
        return Err(FsActionError::EmptyPath);
    }

    let resolution_root = resolve_existing_or_missing_path(resolution_root)?;
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolution_root.join(path)
    };
    let normalized = normalize_without_fs(&combined);
    let file_name = normalized
        .file_name()
        .map(std::ffi::OsStr::to_owned)
        .ok_or_else(|| FsActionError::MissingFileName {
            path: normalized.clone(),
        })?;
    let parent = normalized
        .parent()
        .ok_or_else(|| FsActionError::MissingExistingAncestor {
            path: normalized.clone(),
        })?;
    let mut resolved = resolve_existing_or_missing_path(parent)?;
    resolved.push(file_name);

    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn resolve_existing_or_missing_path(path: &Path) -> Result<PathBuf, FsActionError> {
    let normalized = normalize_without_fs(path);
    if !normalized.exists() {
        // Missing suffixes still inherit symlinks from existing ancestors.
        // Resolve that ancestor now so the returned path is the policy-visible
        // filesystem location, not a lexical path that std::fs would later
        // reinterpret at read time.
        return resolve_from_existing_ancestor(&normalized);
    }
    canonicalize_existing_path(&normalized)
}

fn resolve_from_existing_ancestor(path: &Path) -> Result<PathBuf, FsActionError> {
    let (ancestor, suffix) = split_existing_ancestor(path)?;
    let mut resolved = canonicalize_existing_path(&ancestor)?;
    for component in suffix {
        resolved.push(component);
    }
    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, FsActionError> {
    let canonical =
        dunce::canonicalize(path).map_err(|source| FsActionError::CanonicalizePath {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), FsActionError> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(std::ffi::OsStr::to_owned) else {
            return Err(FsActionError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        suffix.push(name);

        let Some(parent) = cursor.parent() else {
            return Err(FsActionError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        cursor = parent.to_path_buf();
    }
}

// Keep local normalization here until path normalization moves into a lower
// shared crate than `loong-access`.
fn normalize_without_fs(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = parts.last() {
                    if last != ".." {
                        let _ = parts.pop();
                    } else if !has_root {
                        parts.push(OsString::from(".."));
                    }
                } else if !has_root {
                    parts.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => parts.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }

    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}
