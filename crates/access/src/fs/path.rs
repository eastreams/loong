use std::{
    borrow::Cow,
    ffi::OsString,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::{
    error::PolicyGrantError,
    policy::{
        action::{Action, ActionMeta, ActionMetadata},
        context::ContextFactory,
        engine::PolicyEngine,
        grant::Granted,
        policy::Policy,
    },
};
use serde_json::{Value, json};
use thiserror::Error;

use super::access::FsAccess;

#[cfg(test)]
mod tests;

const FS_PATH_REQUIRED_CAPABILITIES: [Capability; 0] = [];

mod sealed {
    pub trait Sealed {}
}

/// Marker for target-following path resolution.
/// Follows the final symlink and authorizes the object reached by the path.
/// Use this when an operation acts on that target rather than its directory entry.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPath;

/// Marker for final-component no-follow path resolution.
/// Preserves the final component as a directory entry instead of following it.
/// Use this for unlink and rename operations that act on the entry itself.
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
/// Implementors provide absolute, lexically normalized configuration roots.
/// `FsResolvePathAction` observes and resolves them only after grant so Session
/// construction and policy evaluation never touch the filesystem directly.
pub trait FsPathPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf];

    /// Parent/root authority that the effective roots must remain beneath.
    ///
    /// Root contexts use their own allowed roots. Derived Session contexts
    /// override this with the parent's effective roots so a lexical child path
    /// cannot gain authority by traversing a symlink outside its parent.
    fn fs_authority_ceiling_roots(&self) -> &[PathBuf] {
        self.fs_allowed_roots()
    }
}

/// Failure while preparing or authorizing a filesystem path.
///
/// Concrete operation errors wrap this type as their path prerequisite. Keeping
/// it in the path module avoids either duplicating resolution failures or
/// rebuilding a domain-wide filesystem error enum.
#[derive(Debug, Error)]
pub enum FsPathError {
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error("filesystem path must not be empty")]
    EmptyPath,
    #[error("filesystem path {path} must include a file name", path = .path.display())]
    MissingFileName { path: PathBuf },
    #[error("failed to canonicalize filesystem path {path}: {source}", path = .path.display())]
    CanonicalizePath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot resolve existing ancestor for filesystem path {path}", path = .path.display())]
    MissingExistingAncestor { path: PathBuf },
}

/// Filesystem object kind shared by governed path-inspection operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsPathKind {
    File,
    Directory,
}

impl FsPathKind {
    pub(in crate::fs) fn from_file_type(file_type: std::fs::FileType) -> Option<Self> {
        if file_type.is_dir() {
            return Some(Self::Directory);
        }
        if file_type.is_file() || file_type.is_symlink() {
            return Some(Self::File);
        }
        None
    }
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
    allowed_roots: Vec<PathBuf>,
    authority_ceiling_roots: Vec<PathBuf>,
    _mode: PhantomData<fn() -> M>,
}

impl FsResolvePathAction<TargetPath> {
    #[must_use]
    pub(in crate::fs) fn target<Cx>(path: impl AsRef<Path>, ctx: &Cx) -> Self
    where
        Cx: FsResolutionContext + FsPathPolicyContext,
    {
        Self::new(path, ctx)
    }
}

impl FsResolvePathAction<EntryPath> {
    #[must_use]
    pub(in crate::fs) fn entry<Cx>(path: impl AsRef<Path>, ctx: &Cx) -> Self
    where
        Cx: FsResolutionContext + FsPathPolicyContext,
    {
        Self::new(path, ctx)
    }
}

impl<M> FsResolvePathAction<M>
where
    M: FsPathMode,
{
    fn new<Cx>(path: impl AsRef<Path>, ctx: &Cx) -> Self
    where
        Cx: FsResolutionContext + FsPathPolicyContext,
    {
        Self {
            raw_path: path.as_ref().to_path_buf(),
            resolution_root: ctx.fs_resolution_root().to_path_buf(),
            allowed_roots: ctx.fs_allowed_roots().to_vec(),
            authority_ceiling_roots: ctx.fs_authority_ceiling_roots().to_vec(),
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

    fn resolve(self) -> Result<ResolvedFsPath<M>, FsPathError> {
        let resolved = if M::FOLLOWS_FINAL_COMPONENT {
            resolve_target_path(&self.raw_path, &self.resolution_root)?
        } else {
            resolve_entry_path(&self.raw_path, &self.resolution_root)?
        };
        let allowed_roots = self
            .allowed_roots
            .iter()
            .map(|root| resolve_target_path(root, &self.resolution_root))
            .collect::<Result<Vec<_>, _>>()?;
        let authority_ceiling_roots = self
            .authority_ceiling_roots
            .iter()
            .map(|root| resolve_target_path(root, &self.resolution_root))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ResolvedFsPath::new(
            self.raw_path,
            resolved,
            allowed_roots,
            authority_ceiling_roots,
        ))
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
            "allowed_roots": self.allowed_roots.iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>(),
            "authority_ceiling_roots": self.authority_ceiling_roots.iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>(),
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

    #[must_use]
    pub fn resolved_allowed_roots(&self) -> &[PathBuf] {
        self.resolved.allowed_roots()
    }

    #[must_use]
    pub fn resolved_authority_ceiling_roots(&self) -> &[PathBuf] {
        self.resolved.authority_ceiling_roots()
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
            "resolved_allowed_roots": self.resolved_allowed_roots().iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>(),
            "resolved_authority_ceiling_roots": self.resolved_authority_ceiling_roots().iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>(),
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
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-path-allowed-roots")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, action: &FsPathAction<M>) -> PolicyGrant {
        let allowed_roots = action.resolved_allowed_roots();
        let authority_ceiling_roots = action.resolved_authority_ceiling_roots();
        if allowed_roots.iter().any(|allowed_root| {
            !authority_ceiling_roots
                .iter()
                .any(|ceiling_root| allowed_root.starts_with(ceiling_root))
        }) {
            return PolicyGrant {
                decision: PolicyDecision::Deny,
                predicate: Some(
                    "resolved allowed roots must remain beneath authority ceiling roots".into(),
                ),
                reason: "filesystem root narrowing escapes its parent authority after resolution"
                    .into(),
            };
        }
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
    allowed_roots: Vec<PathBuf>,
    authority_ceiling_roots: Vec<PathBuf>,
    _mode: std::marker::PhantomData<fn() -> M>,
}

impl<M> ResolvedFsPath<M> {
    pub(in crate::fs) fn new(
        requested: PathBuf,
        path: PathBuf,
        allowed_roots: Vec<PathBuf>,
        authority_ceiling_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            requested,
            path,
            allowed_roots,
            authority_ceiling_roots,
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

    #[must_use]
    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    #[must_use]
    pub fn authority_ceiling_roots(&self) -> &[PathBuf] {
        &self.authority_ceiling_roots
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
    type Error = FsPathError;

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
    type Error = FsPathError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        Ok(granted.into_action().into_granted_path())
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Run the mandatory resolve and path-policy stages for a target path.
    ///
    /// Keeping this chain inside access prevents individual operations from
    /// accidentally skipping either grant while preserving both stages as
    /// distinct typed actions for policy and audit.
    pub(in crate::fs) async fn grant_target_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<GrantedPath, FsPathError> {
        self.grant_path(FsResolvePathAction::target(path, self.ctx))
            .await
    }

    /// Resolve and authorize a final-component no-follow entry path.
    pub(in crate::fs) async fn grant_entry_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<GrantedEntryPath, FsPathError> {
        self.grant_path(FsResolvePathAction::entry(path, self.ctx))
            .await
    }

    // Both public-to-fs entry points use this exact two-stage order. Keeping the
    // generic chain private prevents operation modules from authorizing resolved
    // facts through a different sequence.
    async fn grant_path<M>(
        &self,
        resolve: FsResolvePathAction<M>,
    ) -> Result<GrantedFsPath<M>, FsPathError>
    where
        M: FsPathMode,
    {
        let resolved = self
            .policy_engine
            .grant(self.ctx, resolve)
            .await?
            .into_granted()
            .run(self.ctx)
            .await?;
        let path_action = FsPathAction::new(resolved);
        let path = self
            .policy_engine
            .grant(self.ctx, path_action)
            .await?
            .into_granted()
            .run(self.ctx)
            .await?;
        Ok(path)
    }
}

pub(in crate::fs) fn resolve_target_path(
    path: &Path,
    resolution_root: &Path,
) -> Result<PathBuf, FsPathError> {
    if path.as_os_str().is_empty() {
        return Err(FsPathError::EmptyPath);
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
) -> Result<PathBuf, FsPathError> {
    if path.as_os_str().is_empty() {
        return Err(FsPathError::EmptyPath);
    }

    let resolution_root = resolve_existing_or_missing_path(resolution_root)?;
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolution_root.join(path)
    };
    let normalized = normalize_path_lexically(&combined);
    let file_name = normalized
        .file_name()
        .map(std::ffi::OsStr::to_owned)
        .ok_or_else(|| FsPathError::MissingFileName {
            path: normalized.clone(),
        })?;
    let parent = normalized
        .parent()
        .ok_or_else(|| FsPathError::MissingExistingAncestor {
            path: normalized.clone(),
        })?;
    let mut resolved = resolve_existing_or_missing_path(parent)?;
    resolved.push(file_name);

    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn resolve_existing_or_missing_path(path: &Path) -> Result<PathBuf, FsPathError> {
    let normalized = normalize_path_lexically(path);
    if !normalized.exists() {
        // Missing suffixes still inherit symlinks from existing ancestors.
        // Resolve that ancestor now so the returned path is the policy-visible
        // filesystem location, not a lexical path that std::fs would later
        // reinterpret at read time.
        return resolve_from_existing_ancestor(&normalized);
    }
    canonicalize_existing_path(&normalized)
}

fn resolve_from_existing_ancestor(path: &Path) -> Result<PathBuf, FsPathError> {
    let (ancestor, suffix) = split_existing_ancestor(path)?;
    let mut resolved = canonicalize_existing_path(&ancestor)?;
    for component in suffix {
        resolved.push(component);
    }
    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, FsPathError> {
    let canonical = dunce::canonicalize(path).map_err(|source| FsPathError::CanonicalizePath {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), FsPathError> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(std::ffi::OsStr::to_owned) else {
            return Err(FsPathError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        suffix.push(name);

        let Some(parent) = cursor.parent() else {
            return Err(FsPathError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        cursor = parent.to_path_buf();
    }
}

/// Normalize `.` and `..` components without touching the filesystem.
///
/// This operation is deliberately pure: it neither canonicalizes symlinks nor
/// authorizes the resulting path. Keeping the one implementation in fs Access
/// prevents configured roots and action resolution from applying different
/// lexical rules before policy evaluates canonical filesystem facts.
pub fn normalize_path_lexically(path: &Path) -> PathBuf {
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
