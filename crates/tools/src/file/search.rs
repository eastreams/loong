use std::{collections::BTreeSet, path::PathBuf};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolInputError, ToolSchedulingClass, ToolSpec};
use loong_core::{
    PolicyGrantError,
    policy::context::ContextFactory,
    tool::{ToolFailureKind, ToolImpl},
};
use loong_kernel::{
    KernelAccess,
    access::fs::{
        FsContentSearchError, FsContentSearchOptions, FsContentSearchOutput, FsGlobError,
        FsGlobOutput, FsPathError, FsPathKind, FsPathPolicyContext, FsResolutionContext,
    },
};
use serde_json::{Value, json};

use super::{
    optional_bounded_usize_field, optional_trimmed_string_field, required_trimmed_string_field,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobReadRequest {
    pub(super) root: String,
    pub(super) pattern: String,
    pub(super) max_results: usize,
    pub(super) include_directories: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobReadOutput {
    root: PathBuf,
    pattern: String,
    max_results: usize,
    truncated: bool,
    matches: Vec<GlobReadMatch>,
}

impl GlobReadOutput {
    pub(super) fn from_access_output(request: GlobReadRequest, output: FsGlobOutput) -> Self {
        let matches = output
            .matches
            .into_iter()
            .map(|entry| GlobReadMatch {
                relative_path: entry.relative_path,
                kind: match entry.kind {
                    FsPathKind::File => GlobReadMatchKind::File,
                    FsPathKind::Directory => GlobReadMatchKind::Directory,
                },
            })
            .collect();

        Self {
            root: output.root,
            pattern: request.pattern,
            max_results: request.max_results,
            truncated: output.truncated,
            matches,
        }
    }
}

impl From<GlobReadOutput> for Value {
    fn from(output: GlobReadOutput) -> Self {
        let matches = output
            .matches
            .iter()
            .map(|entry| {
                json!({
                    "path": entry.relative_path,
                    "kind": entry.kind.as_str(),
                })
            })
            .collect::<Vec<_>>();
        let continuation = glob_search_continuation_payload(matches.as_slice());
        let mut payload = json!({
            "root": output.root.display().to_string(),
            "query": output.pattern,
            "max_results": output.max_results,
            "truncated": output.truncated,
            "match_count": matches.len(),
            "matches": matches,
        });
        if let Some(continuation) = continuation
            && let Some(payload_object) = payload.as_object_mut()
        {
            payload_object.insert("continuation".to_owned(), continuation);
        }
        payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GlobReadMatch {
    relative_path: String,
    kind: GlobReadMatchKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobReadMatchKind {
    File,
    Directory,
}

impl GlobReadMatchKind {
    #[must_use]
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSearchReadRequest {
    pub(super) root: String,
    pub(super) query: String,
    pub(super) glob: Option<String>,
    pub(super) max_results: usize,
    pub(super) max_bytes_per_file: usize,
    pub(super) case_sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSearchReadOutput {
    root: PathBuf,
    query: String,
    max_results: usize,
    truncated: bool,
    matches: Vec<ContentSearchReadMatch>,
}

impl ContentSearchReadOutput {
    pub(super) fn from_access_output(
        request: ContentSearchReadRequest,
        output: FsContentSearchOutput,
    ) -> Self {
        let matches = output
            .matches
            .into_iter()
            .map(|entry| ContentSearchReadMatch {
                relative_path: entry.relative_path,
                line: entry.line,
                column: entry.column,
                match_text: entry.match_text,
                snippet: entry.snippet,
                truncated_file: entry.truncated_file,
            })
            .collect();

        Self {
            root: output.root,
            query: request.query,
            max_results: request.max_results,
            truncated: output.truncated,
            matches,
        }
    }
}

impl From<ContentSearchReadOutput> for Value {
    fn from(output: ContentSearchReadOutput) -> Self {
        let matches = output
            .matches
            .iter()
            .map(|entry| {
                json!({
                    "path": entry.relative_path,
                    "line": entry.line,
                    "column": entry.column,
                    "match_text": entry.match_text,
                    "snippet": entry.snippet,
                    "truncated_file": entry.truncated_file,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "root": output.root.display().to_string(),
            "query": output.query,
            "max_results": output.max_results,
            "truncated": output.truncated,
            "match_count": matches.len(),
            "matches": matches,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentSearchReadMatch {
    relative_path: String,
    line: usize,
    column: usize,
    match_text: String,
    snippet: String,
    truncated_file: bool,
}

pub struct GlobSearchTool;

impl GlobSearchTool {
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Workspace glob pattern to match."
                },
                "root": {
                    "type": "string",
                    "description": "Optional search root path. Defaults to the workspace root."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "description": "Optional maximum result count."
                },
                "include_directories": {
                    "type": "boolean",
                    "description": "Include matching directories. Defaults to false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }
}

pub struct ContentSearchTool;

impl ContentSearchTool {
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to search for in workspace files."
                },
                "root": {
                    "type": "string",
                    "description": "Optional search root path. Defaults to the workspace root."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional file glob filter."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Optional maximum result count."
                },
                "max_bytes_per_file": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1_048_576,
                    "description": "Optional per-file scan budget."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Use case-sensitive matching. Defaults to false."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }
}

/// Typed implementation for glob search.
///
/// The tool path is app-owned; this concrete type only parses the selected
/// payload, calls fs access, and returns domain output.
#[async_trait]
impl<C> ToolImpl<C> for GlobSearchTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsResolutionContext + FsPathPolicyContext + Sync,
{
    type Input = GlobReadRequest;
    type Output = GlobReadOutput;
    type Error = FsGlobError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Search the workspace for files matching a glob pattern.".to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
            scheduling: ToolSchedulingClass::ParallelSafe,
            argument_hint: Some(
                "pattern:string,root?:string,max_results?:integer,include_directories?:boolean"
                    .to_owned(),
            ),
            search_hint: Some("list workspace paths matching a glob pattern".to_owned()),
            tags: ["surface", "read", "file", "glob"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        let payload_object = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;
        GlobReadRequest::parse_payload(payload_object)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        match error {
            FsGlobError::Authorization(source)
            | FsGlobError::Path(FsPathError::Authorization(source))
                if matches!(
                    source,
                    PolicyGrantError::MissingCapability { .. }
                        | PolicyGrantError::Denied { .. }
                        | PolicyGrantError::PermissionDenied { .. }
                ) =>
            {
                ToolFailureKind::Denied
            }
            FsGlobError::Path(_)
            | FsGlobError::Authorization(_)
            | FsGlobError::InvalidGlobPattern { .. }
            | FsGlobError::ReadDirectory { .. }
            | FsGlobError::InspectPath { .. }
            | FsGlobError::RenderRelativePath { .. } => ToolFailureKind::Execution,
        }
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let output = ctx
            .access()
            .fs()
            .glob_paths(
                input.root.as_str(),
                input.pattern.clone(),
                input.include_directories,
                input.max_results,
            )
            .await?;

        Ok(GlobReadOutput::from_access_output(input, output))
    }
}

/// Typed implementation for content search.
///
/// Content scanning can read many files, so this surface must go through
/// access-granted fs search rather than app-local `std::fs::read`.
#[async_trait]
impl<C> ToolImpl<C> for ContentSearchTool
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: KernelAccess<C> + FsResolutionContext + FsPathPolicyContext + Sync,
{
    type Input = ContentSearchReadRequest;
    type Output = ContentSearchReadOutput;
    type Error = FsContentSearchError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Search workspace file contents for a text match with bounded results."
                .to_owned(),
            input_schema: Self::input_schema(),
            required_capabilities: BTreeSet::from([Capability::FilesystemRead]),
            scheduling: ToolSchedulingClass::ParallelSafe,
            argument_hint: Some(
                "query:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean"
                    .to_owned(),
            ),
            search_hint: Some("search workspace file contents with bounded results".to_owned()),
            tags: ["surface", "read", "file", "search"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        let payload_object = payload
            .as_object()
            .ok_or(ToolInputError::PayloadMustBeObject)?;
        ContentSearchReadRequest::parse_payload(payload_object)
    }

    fn failure_kind(&self, error: &Self::Error) -> ToolFailureKind {
        match error {
            FsContentSearchError::Authorization(source)
            | FsContentSearchError::Path(FsPathError::Authorization(source))
                if matches!(
                    source,
                    PolicyGrantError::MissingCapability { .. }
                        | PolicyGrantError::Denied { .. }
                        | PolicyGrantError::PermissionDenied { .. }
                ) =>
            {
                ToolFailureKind::Denied
            }
            FsContentSearchError::Path(_)
            | FsContentSearchError::Authorization(_)
            | FsContentSearchError::InvalidGlobPattern { .. }
            | FsContentSearchError::BuildContentSearchRegex { .. }
            | FsContentSearchError::ReadDirectory { .. }
            | FsContentSearchError::InspectPath { .. }
            | FsContentSearchError::RenderRelativePath { .. }
            | FsContentSearchError::ReadFile { .. }
            | FsContentSearchError::InvalidContentMatchRange { .. } => ToolFailureKind::Execution,
        }
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        let options = FsContentSearchOptions {
            glob: input.glob.clone(),
            max_results: input.max_results,
            max_bytes_per_file: input.max_bytes_per_file,
            case_sensitive: input.case_sensitive,
        };
        let output = ctx
            .access()
            .fs()
            .search_content(input.root.as_str(), input.query.clone(), options)
            .await?;

        Ok(ContentSearchReadOutput::from_access_output(input, output))
    }
}

impl GlobReadRequest {
    pub(super) fn parse_payload(
        payload: &serde_json::Map<String, Value>,
    ) -> Result<Self, ToolInputError> {
        let pattern = if let Some(pattern) = optional_trimmed_string_field(payload.get("pattern")) {
            pattern.to_owned()
        } else if let Some(glob) = optional_trimmed_string_field(payload.get("glob")) {
            let parts = glob
                .split('|')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            if glob.contains('|') && !glob.contains('{') && !glob.contains('}') && parts.len() > 1 {
                format!("{{{}}}", parts.join(","))
            } else {
                glob.to_owned()
            }
        } else {
            for field_name in ["pattern", "glob"] {
                if payload.contains_key(field_name) {
                    required_trimmed_string_field(payload, field_name)?;
                }
            }
            return Err(ToolInputError::missing_field("pattern"));
        };
        let root = optional_trimmed_string_field(payload.get("root"))
            .unwrap_or(".")
            .to_owned();
        let max_results = optional_bounded_usize_field(payload, "max_results", 50, 1, 200)?;
        let include_directories = payload
            .get("include_directories")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(Self {
            root,
            pattern,
            max_results,
            include_directories,
        })
    }
}

impl ContentSearchReadRequest {
    pub(super) fn parse_payload(
        payload: &serde_json::Map<String, Value>,
    ) -> Result<Self, ToolInputError> {
        let query = required_trimmed_string_field(payload, "query")?.to_owned();
        let root = optional_trimmed_string_field(payload.get("root"))
            .unwrap_or(".")
            .to_owned();
        let glob = optional_trimmed_string_field(payload.get("glob")).map(ToOwned::to_owned);
        let max_results = optional_bounded_usize_field(payload, "max_results", 20, 1, 100)?;
        let max_bytes_per_file =
            optional_bounded_usize_field(payload, "max_bytes_per_file", 262_144, 1, 1_048_576)?;
        let case_sensitive = payload
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(Self {
            root,
            query,
            glob,
            max_results,
            max_bytes_per_file,
            case_sensitive,
        })
    }
}

fn glob_search_continuation_payload(matches: &[Value]) -> Option<Value> {
    let first_path = matches
        .iter()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str))
        .find(|path| !path.trim().is_empty())?;

    Some(json!({
        "state": "path_listing",
        "is_terminal": false,
        "recommended_tool": "read",
        "recommended_payload": {
            "path": first_path,
        },
        "note": "The last read result only listed candidate paths. If the user still needs grounded file contents or a repository summary, continue with direct `read` calls before answering."
    }))
}
