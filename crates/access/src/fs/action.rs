use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use loong_contracts::Capability;
use loong_core::policy::action::{ActionMeta, ActionMetadata};
use serde_json::{Value, json};

use super::path::{GrantedPath, ResolvedDeletionPath, ResolvedPath};

const FS_RESOLVE_REQUIRED_CAPABILITIES: [Capability; 0] = [];
const FS_READ_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];
const FS_WRITE_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];
const FS_COPY_FILE_REQUIRED_CAPABILITIES: [Capability; 2] =
    [Capability::FilesystemRead, Capability::FilesystemWrite];
const FS_CREATE_DIR_ALL_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];
const FS_REMOVE_FILE_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemWrite];
const FS_GLOB_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];
const FS_CONTENT_SEARCH_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];
const FS_INSPECT_PATH_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

/// Typed action for resolving a raw path into a governed fs path.
///
/// Path resolution observes the filesystem through canonicalization and symlink
/// handling, so it is modeled as an action instead of an ungoverned helper.
/// The action carries resolved path facts for policy; it does not decide
/// whether those facts are allowed.
/// It does not declare read/search/glob capabilities; concrete data-leaking fs
/// actions declare those after receiving the governed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsResolvePathAction {
    raw_path: PathBuf,
    resolved: ResolvedPath,
}

impl FsResolvePathAction {
    // Access prepares filesystem-observation facts before policy evaluation.
    // The constructor stays fs-private so callers cannot mint a resolve action
    // without using the context roots from `FsAccess`.
    pub(in crate::fs) fn resolve(
        path: impl AsRef<Path>,
        resolution_root: impl AsRef<Path>,
    ) -> Result<Self, super::error::FsActionError> {
        let raw_path = path.as_ref().to_path_buf();
        let resolved = ResolvedPath::resolve(&raw_path, resolution_root)?;
        Ok(Self { raw_path, resolved })
    }

    #[must_use]
    pub fn raw_path(&self) -> &Path {
        &self.raw_path
    }

    #[must_use]
    pub fn resolved_path(&self) -> &Path {
        self.resolved.path()
    }

    pub(in crate::fs) fn into_granted_path(self) -> GrantedPath {
        GrantedPath::new(self.resolved.into_path_buf())
    }
}

impl ActionMeta for FsResolvePathAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.resolve_path",
            operation: Cow::Borrowed("resolve_path"),
            required_capabilities: Cow::Borrowed(&FS_RESOLVE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.raw_path.display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.raw_path.display().to_string(),
            "resolved_path": self.resolved_path().display().to_string(),
        }))
    }
}

/// Typed action for reading one canonical filesystem path.
///
/// The action declares the required capability and audit resource. It does not
/// carry workspace roots; roots belong to the invocation context and path
/// resolver. Its constructor accepts `GrantedPath` so path policy cannot be
/// bypassed with a raw or merely canonical `PathBuf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadAction {
    path: GrantedPath,
}

impl FsReadAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsReadAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.read",
            operation: Cow::Borrowed("read_file"),
            required_capabilities: Cow::Borrowed(&FS_READ_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Policy-visible options for writing one governed filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsWriteOptions {
    pub create_dirs: bool,
    pub overwrite: bool,
}

/// Typed action for writing bytes to one governed filesystem path.
///
/// The action carries bytes because the access side-effect boundary needs them
/// to perform the write. Its audit payload records only byte count and flags,
/// not file content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsWriteAction {
    path: GrantedPath,
    bytes: Vec<u8>,
    options: FsWriteOptions,
}

impl FsWriteAction {
    #[must_use]
    pub fn new(path: GrantedPath, bytes: Vec<u8>, options: FsWriteOptions) -> Self {
        Self {
            path,
            bytes,
            options,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    #[must_use]
    pub fn options(&self) -> FsWriteOptions {
        self.options
    }
}

impl ActionMeta for FsWriteAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.write",
            operation: Cow::Borrowed("write_file"),
            required_capabilities: Cow::Borrowed(&FS_WRITE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
            "byte_count": self.bytes.len(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Typed action for copying bytes between two governed filesystem paths.
///
/// Copy is modeled as one action instead of `read_file` plus app-side
/// `write_file` so backup/restore flows do not move file bytes through tool or
/// migration orchestration code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCopyFileAction {
    source: GrantedPath,
    destination: GrantedPath,
    options: FsWriteOptions,
}

impl FsCopyFileAction {
    #[must_use]
    pub fn new(source: GrantedPath, destination: GrantedPath, options: FsWriteOptions) -> Self {
        Self {
            source,
            destination,
            options,
        }
    }

    #[must_use]
    pub fn source_path(&self) -> &Path {
        self.source.as_path()
    }

    #[must_use]
    pub fn destination_path(&self) -> &Path {
        self.destination.as_path()
    }

    #[must_use]
    pub fn options(&self) -> FsWriteOptions {
        self.options
    }
}

impl ActionMeta for FsCopyFileAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.copy_file",
            operation: Cow::Borrowed("copy_file"),
            required_capabilities: Cow::Borrowed(&FS_COPY_FILE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(
            format!(
                "{} -> {}",
                self.source.as_path().display(),
                self.destination.as_path().display()
            )
            .into(),
        )
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "source": self.source.as_path().display().to_string(),
            "destination": self.destination.as_path().display().to_string(),
            "create_dirs": self.options.create_dirs,
            "overwrite": self.options.overwrite,
        }))
    }
}

/// Typed action for creating one governed directory tree.
///
/// Directory creation is a write side effect, so the action declares
/// `FilesystemWrite` and can only run after path resolution has produced a
/// `GrantedPath`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsCreateDirAllAction {
    path: GrantedPath,
}

impl FsCreateDirAllAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsCreateDirAllAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.create_dir_all",
            operation: Cow::Borrowed("create_dir_all"),
            required_capabilities: Cow::Borrowed(&FS_CREATE_DIR_ALL_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Typed action for removing one governed file or symlink.
///
/// This action does not consume `GrantedPath`: deletion needs final-component
/// no-follow semantics, while `GrantedPath` represents a canonical target.
/// Access prepares the deletion path facts and kernel policy decides whether
/// they are allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsRemoveFileAction {
    raw_path: PathBuf,
    deletion: ResolvedDeletionPath,
}

impl FsRemoveFileAction {
    pub(in crate::fs) fn resolve(
        path: impl AsRef<Path>,
        resolution_root: impl AsRef<Path>,
    ) -> Result<Self, super::error::FsActionError> {
        let raw_path = path.as_ref().to_path_buf();
        let deletion = ResolvedDeletionPath::resolve(&raw_path, resolution_root)?;
        Ok(Self { raw_path, deletion })
    }

    #[must_use]
    pub fn raw_path(&self) -> &Path {
        &self.raw_path
    }

    #[must_use]
    pub fn deletion_path(&self) -> &Path {
        self.deletion.path()
    }
}

impl ActionMeta for FsRemoveFileAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.remove_file",
            operation: Cow::Borrowed("remove_file"),
            required_capabilities: Cow::Borrowed(&FS_REMOVE_FILE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.deletion_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.raw_path.display().to_string(),
            "deletion_path": self.deletion_path().display().to_string(),
        }))
    }
}

/// Typed action for observing one governed filesystem path.
///
/// Existence and file-kind metadata leak filesystem state, so inspect is a
/// read-family action even though it does not read file contents. Write actions
/// keep their own write-authorized checks for overwrite safety.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInspectPathAction {
    path: GrantedPath,
}

impl FsInspectPathAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsInspectPathAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.inspect_path",
            operation: Cow::Borrowed("inspect_path"),
            required_capabilities: Cow::Borrowed(&FS_INSPECT_PATH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Typed action for listing filesystem paths by glob pattern.
///
/// Like `FsReadAction`, this is a data-leaking filesystem operation and
/// therefore consumes a `GrantedPath` root. Pattern matching happens inside the
/// access side-effect boundary so tools do not receive a broader directory
/// listing than the action payload describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsGlobAction {
    root: GrantedPath,
    pattern: String,
    include_directories: bool,
    max_results: usize,
}

impl FsGlobAction {
    #[must_use]
    pub fn new(
        root: GrantedPath,
        pattern: impl Into<String>,
        include_directories: bool,
        max_results: usize,
    ) -> Self {
        Self {
            root,
            pattern: pattern.into(),
            include_directories,
            max_results,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    #[must_use]
    pub fn include_directories(&self) -> bool {
        self.include_directories
    }

    #[must_use]
    pub fn max_results(&self) -> usize {
        self.max_results
    }
}

impl ActionMeta for FsGlobAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.glob",
            operation: Cow::Borrowed("glob_paths"),
            required_capabilities: Cow::Borrowed(&FS_GLOB_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.root.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "root": self.root.as_path().display().to_string(),
            "pattern": self.pattern,
            "include_directories": self.include_directories,
            "max_results": self.max_results,
        }))
    }
}

/// Policy-visible options for one governed content search.
///
/// Defaults and bounds belong to the caller/tool parser; access receives the
/// already-selected values and records them in the action payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchOptions {
    pub glob: Option<String>,
    pub max_results: usize,
    pub max_bytes_per_file: usize,
    pub case_sensitive: bool,
}

/// Typed action for searching text inside files under one governed root.
///
/// Content search reads many candidate files, so the query and optional glob
/// filter belong to the action payload. Keeping matching inside access avoids
/// returning broad file contents to a tool just so it can filter them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsContentSearchAction {
    root: GrantedPath,
    query: String,
    options: FsContentSearchOptions,
}

impl FsContentSearchAction {
    #[must_use]
    pub fn new(
        root: GrantedPath,
        query: impl Into<String>,
        options: FsContentSearchOptions,
    ) -> Self {
        Self {
            root,
            query: query.into(),
            options,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    #[must_use]
    pub fn query(&self) -> &str {
        self.query.as_str()
    }

    #[must_use]
    pub fn options(&self) -> &FsContentSearchOptions {
        &self.options
    }
}

impl ActionMeta for FsContentSearchAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.content_search",
            operation: Cow::Borrowed("search_content"),
            required_capabilities: Cow::Borrowed(&FS_CONTENT_SEARCH_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.root.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "root": self.root.as_path().display().to_string(),
            "query": self.query,
            "glob": self.options.glob.as_deref(),
            "max_results": self.options.max_results,
            "max_bytes_per_file": self.options.max_bytes_per_file,
            "case_sensitive": self.options.case_sensitive,
        }))
    }
}

/// Filesystem action family.
///
/// Keep variants here thin wrappers around typed actions so policies can
/// register either for a concrete action or for the family as needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsAction {
    Read(FsReadAction),
    Write(FsWriteAction),
    CopyFile(FsCopyFileAction),
    CreateDirAll(FsCreateDirAllAction),
    RemoveFile(FsRemoveFileAction),
    InspectPath(FsInspectPathAction),
    Glob(FsGlobAction),
    ContentSearch(FsContentSearchAction),
}

impl FsAction {
    #[must_use]
    pub fn read_file(path: GrantedPath) -> Self {
        Self::Read(FsReadAction::new(path))
    }

    #[must_use]
    pub fn write_file(path: GrantedPath, bytes: Vec<u8>, options: FsWriteOptions) -> Self {
        Self::Write(FsWriteAction::new(path, bytes, options))
    }

    #[must_use]
    pub fn copy_file(
        source: GrantedPath,
        destination: GrantedPath,
        options: FsWriteOptions,
    ) -> Self {
        Self::CopyFile(FsCopyFileAction::new(source, destination, options))
    }

    #[must_use]
    pub fn create_dir_all(path: GrantedPath) -> Self {
        Self::CreateDirAll(FsCreateDirAllAction::new(path))
    }

    #[must_use]
    pub fn remove_file(action: FsRemoveFileAction) -> Self {
        Self::RemoveFile(action)
    }

    #[must_use]
    pub fn inspect_path(path: GrantedPath) -> Self {
        Self::InspectPath(FsInspectPathAction::new(path))
    }

    #[must_use]
    pub fn glob_paths(
        root: GrantedPath,
        pattern: impl Into<String>,
        include_directories: bool,
        max_results: usize,
    ) -> Self {
        Self::Glob(FsGlobAction::new(
            root,
            pattern,
            include_directories,
            max_results,
        ))
    }

    #[must_use]
    pub fn search_content(
        root: GrantedPath,
        query: impl Into<String>,
        options: FsContentSearchOptions,
    ) -> Self {
        Self::ContentSearch(FsContentSearchAction::new(root, query, options))
    }
}

impl ActionMeta for FsAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        match self {
            Self::Read(action) => action.metadata(),
            Self::Write(action) => action.metadata(),
            Self::CopyFile(action) => action.metadata(),
            Self::CreateDirAll(action) => action.metadata(),
            Self::RemoveFile(action) => action.metadata(),
            Self::InspectPath(action) => action.metadata(),
            Self::Glob(action) => action.metadata(),
            Self::ContentSearch(action) => action.metadata(),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Read(action) => action.audit_resource(),
            Self::Write(action) => action.audit_resource(),
            Self::CopyFile(action) => action.audit_resource(),
            Self::CreateDirAll(action) => action.audit_resource(),
            Self::RemoveFile(action) => action.audit_resource(),
            Self::InspectPath(action) => action.audit_resource(),
            Self::Glob(action) => action.audit_resource(),
            Self::ContentSearch(action) => action.audit_resource(),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        match self {
            Self::Read(action) => action.payload(),
            Self::Write(action) => action.payload(),
            Self::CopyFile(action) => action.payload(),
            Self::CreateDirAll(action) => action.payload(),
            Self::RemoveFile(action) => action.payload(),
            Self::InspectPath(action) => action.payload(),
            Self::Glob(action) => action.payload(),
            Self::ContentSearch(action) => action.payload(),
        }
    }
}
