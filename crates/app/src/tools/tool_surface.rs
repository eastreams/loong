use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::ToolView;

pub(crate) const DIRECT_READ_TOOL_NAME: &str = "read";
pub(crate) const DIRECT_EDIT_TOOL_NAME: &str = "edit";
pub(crate) const DIRECT_WRITE_TOOL_NAME: &str = "write";
pub(crate) const DIRECT_BASH_TOOL_NAME: &str = "bash";
pub(crate) const DIRECT_EXEC_TOOL_NAME: &str = "exec";
pub(crate) const DIRECT_WEB_TOOL_NAME: &str = "web";
pub(crate) const DIRECT_BROWSER_TOOL_NAME: &str = "browser";
pub(crate) const DIRECT_MEMORY_TOOL_NAME: &str = "memory";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectToolSurfaceMetadata {
    pub(crate) argument_hint: &'static str,
    pub(crate) search_hint: &'static str,
    pub(crate) parameter_types: &'static [(&'static str, &'static str)],
    pub(crate) required_fields: &'static [&'static str],
    pub(crate) tags: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectWebRuntimeModes {
    pub(crate) query_search_available: bool,
    pub(crate) fetch_available: bool,
    pub(crate) request_available: bool,
}

impl DirectWebRuntimeModes {
    pub(crate) fn from_view(view: &ToolView) -> Self {
        Self {
            query_search_available: view.contains("web.search"),
            fetch_available: view.contains("web.fetch"),
            request_available: view.contains("http.request"),
        }
    }

    pub(crate) fn ordinary_network_access_available(self) -> bool {
        self.fetch_available || self.request_available
    }

    pub(crate) fn provider_description(self) -> Option<&'static str> {
        match (
            self.query_search_available,
            self.ordinary_network_access_available(),
        ) {
            (true, true) => Some("Fetch a URL, send HTTP requests, or search the public web"),
            (true, false) => Some("Search the public web"),
            (false, true) => Some("Fetch a URL or send HTTP requests"),
            (false, false) => None,
        }
    }

    pub(crate) fn search_hint(self) -> Option<&'static str> {
        match (
            self.query_search_available,
            self.ordinary_network_access_available(),
        ) {
            (true, true) => Some(
                "fetch a url, search the web, or send a low-level http request through one direct tool; only query mode depends on web-search providers",
            ),
            (true, false) => Some(
                "search the public web through one direct tool; query mode uses web-search providers",
            ),
            (false, true) => Some(
                "fetch a url or send a low-level http request through one direct tool; query search mode is unavailable in this runtime",
            ),
            (false, false) => None,
        }
    }

    fn prompt_state(self, surface: ToolSurfaceDescriptor) -> (&'static str, &'static str) {
        match (
            self.query_search_available,
            self.ordinary_network_access_available(),
        ) {
            (true, true) => (surface.prompt_snippet, surface.prompt_guidance),
            (true, false) => (
                "search the public web for docs, APIs, and references.",
                "Use web for public-web search. Query mode uses web-search providers.",
            ),
            (false, true) => (
                "fetch a URL or send an HTTP request.",
                "Use web for direct network access to docs, APIs, scraping, and public references. Query search mode is unavailable in this runtime.",
            ),
            (false, false) => (surface.prompt_snippet, surface.prompt_guidance),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolSurfaceDescriptor {
    pub(crate) id: &'static str,
    pub(crate) prompt_snippet: &'static str,
    pub(crate) prompt_guidance: &'static str,
    pub(crate) prompt_guidelines: &'static [&'static str],
    pub(crate) direct_tool_name: Option<&'static str>,
    pub(crate) covered_tool_names: &'static [&'static str],
    pub(crate) direct_metadata: Option<DirectToolSurfaceMetadata>,
    pub(crate) hidden_search_summary: Option<&'static str>,
    pub(crate) hidden_search_argument_hint: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSurfaceState {
    pub surface_id: String,
    pub prompt_snippet: String,
    pub usage_guidance: String,
    #[serde(default)]
    pub tool_ids: Vec<String>,
}

impl ToolSurfaceState {
    pub fn tool_count(&self) -> usize {
        self.tool_ids.len()
    }

    pub(crate) fn render_prompt_line(&self) -> String {
        format!(
            "- {} (hidden surface): {} {}",
            self.surface_id, self.prompt_snippet, self.usage_guidance
        )
    }
}

impl ToolSurfaceDescriptor {
    fn into_state(self, tool_ids: Vec<String>) -> ToolSurfaceState {
        ToolSurfaceState {
            surface_id: self.id.to_owned(),
            prompt_snippet: self.prompt_snippet.to_owned(),
            usage_guidance: self.prompt_guidance.to_owned(),
            tool_ids,
        }
    }
}

const READ_GUIDELINES: &[&str] = &[
    "Use read for repo inspection before shelling out.",
    "Use `offset` and `limit` to page through large files instead of reading everything at once.",
];
const WRITE_GUIDELINES: &[&str] = &["Use write for new files or whole-file rewrites."];
const EDIT_GUIDELINES: &[&str] = &[
    "Use edit for precise changes to existing files.",
    "When one file needs multiple disjoint changes, prefer one edit call with multiple exact edit blocks.",
    "Prefer edit for surgical changes to existing files instead of whole-file rewrites.",
];
const EXEC_GUIDELINES: &[&str] = &[
    "Use exec for direct program execution with explicit argv-style arguments.",
    "Prefer bash when the task depends on shell syntax, pipelines, redirects, builtins, or multi-line shell scripts.",
    "If exec output is truncated, prefer `details.handoff.recommended_payload` with `read`; if needed, inspect `details.handoff.recipes.*` for alternate first-page / last-page / wider-byte windows.",
];
const BASH_GUIDELINES: &[&str] = &[
    "Use bash for shell commands, shell builtins, pipelines, redirects, and multi-line shell workflows.",
    "Prefer bash over exec when the task is naturally expressed as a shell command string.",
];
const WEB_GUIDELINES: &[&str] = &[
    "Use web for public docs, APIs, and references.",
    "`query` uses web-search providers; `url` and low-level request mode use normal network egress instead.",
    "Prefer plain fetch or search before dropping to low-level request fields.",
];
const BROWSER_GUIDELINES: &[&str] = &[
    "Use browser when page structure or interaction matters.",
    "Keep managed browser session work under `browser` instead of teaching a long tail of sub-tool names.",
    "Prefer `web` for simple URL fetches that do not need live page interaction.",
];
const MEMORY_GUIDELINES: &[&str] = &[
    "Use memory for persisted notes and cross-session recall.",
    "Prefer read for normal workspace files and memory only for durable note content.",
];
const AGENT_GUIDELINES: &[&str] = &[
    "Use agent only for Loong's own approvals, sessions, delegation, provider routing, or config work.",
    "Prefer a direct tool first; reach for agent when the task is about runtime control rather than user data.",
];
const SKILLS_GUIDELINES: &[&str] = &[
    "Use skills when the task is about discovering, installing, or running external skills.",
    "Keep capability-expansion work under skills instead of mixing it with normal repo editing or runtime control.",
];
const CHANNEL_GUIDELINES: &[&str] = &[
    "Keep channel-specific work on the channel lane instead of folding it into core runtime surfaces.",
    "Treat Feishu-style tools as add-ons that remain structurally separable from Loong core.",
];

const READ_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("path", "string"),
    ("offset", "integer"),
    ("limit", "integer"),
    ("max_bytes", "integer"),
    ("query", "string"),
    ("pattern", "string"),
    ("root", "string"),
    ("glob", "string"),
    ("max_results", "integer"),
    ("max_bytes_per_file", "integer"),
    ("case_sensitive", "boolean"),
    ("include_directories", "boolean"),
];
const EDIT_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("path", "string"),
    ("edits", "array"),
    ("old_string", "string"),
    ("new_string", "string"),
    ("replace_all", "boolean"),
];
const WRITE_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("path", "string"),
    ("content", "string"),
    ("create_dirs", "boolean"),
    ("overwrite", "boolean"),
];
const BASH_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("command", "string"),
    ("script", "string"),
    ("timeout_ms", "integer"),
    ("cwd", "string"),
];
const EXEC_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("command", "string"),
    ("args", "array"),
    ("timeout_ms", "integer"),
    ("cwd", "string"),
];
const WEB_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("url", "string"),
    ("mode", "string"),
    ("max_bytes", "integer"),
    ("query", "string"),
    ("provider", "string"),
    ("max_results", "integer"),
];
const BROWSER_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("url", "string"),
    ("max_bytes", "integer"),
    ("session_id", "string"),
    ("mode", "string"),
    ("selector", "string"),
    ("limit", "integer"),
    ("link_id", "integer"),
    ("text", "string"),
    ("condition", "string"),
    ("timeout_ms", "integer"),
];
const MEMORY_DIRECT_PARAMETER_TYPES: &[(&str, &str)] = &[
    ("query", "string"),
    ("max_results", "integer"),
    ("path", "string"),
    ("from", "integer"),
    ("lines", "integer"),
];

const READ_COVERED_TOOL_NAMES: &[&str] = &["file.read", "glob.search", "content.search"];
const EDIT_COVERED_TOOL_NAMES: &[&str] = &["file.edit"];
const WRITE_COVERED_TOOL_NAMES: &[&str] = &["file.write"];
const BASH_COVERED_TOOL_NAMES: &[&str] = &["bash.exec"];
const EXEC_COVERED_TOOL_NAMES: &[&str] = &["shell.exec"];
const WEB_COVERED_TOOL_NAMES: &[&str] = &["web.fetch", "web.search", "http.request"];
const BROWSER_COVERED_TOOL_NAMES: &[&str] = &["browser.open", "browser.extract", "browser.click"];
const MEMORY_COVERED_TOOL_NAMES: &[&str] = &["memory_search", "memory_get"];

const READ_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "path?:string,offset?:integer,limit?:integer,max_bytes?:integer,query?:string,pattern?:string,root?:string,glob?:string,max_results?:integer,max_bytes_per_file?:integer,case_sensitive?:boolean,include_directories?:boolean",
    search_hint: "read one file, page through a large file, search workspace content, or list matching paths through one direct tool",
    parameter_types: READ_DIRECT_PARAMETER_TYPES,
    required_fields: &[],
    tags: &["surface", "read", "file", "search"],
};
const EDIT_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "path:string,edits?:array,old_string?:string,new_string?:string,replace_all?:boolean",
    search_hint: "edit an existing workspace file with exact replacement blocks through one direct tool",
    parameter_types: EDIT_DIRECT_PARAMETER_TYPES,
    required_fields: &["path"],
    tags: &["surface", "edit", "file", "replace"],
};
const WRITE_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "path:string,content:string,create_dirs?:boolean,overwrite?:boolean",
    search_hint: "create a file or rewrite one file through one direct tool",
    parameter_types: WRITE_DIRECT_PARAMETER_TYPES,
    required_fields: &["path", "content"],
    tags: &["surface", "write", "file", "create"],
};
const BASH_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "command?:string,script?:string,timeout_ms?:integer,cwd?:string",
    search_hint: "run shell commands, pipelines, redirects, or shell scripts through one direct tool",
    parameter_types: BASH_DIRECT_PARAMETER_TYPES,
    required_fields: &[],
    tags: &["surface", "bash", "shell", "script"],
};
const EXEC_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "command:string,args?:string[],timeout_ms?:integer,cwd?:string",
    search_hint: "execute a program directly with optional argv arguments through one direct tool",
    parameter_types: EXEC_DIRECT_PARAMETER_TYPES,
    required_fields: &["command"],
    tags: &["surface", "exec", "process", "argv"],
};
const WEB_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "url?:string,mode?:string,max_bytes?:integer,query?:string,provider?:string,max_results?:integer",
    search_hint: "fetch a url, search the web, or send a low-level http request through one direct tool; only query mode depends on web-search providers",
    parameter_types: WEB_DIRECT_PARAMETER_TYPES,
    required_fields: &[],
    tags: &["surface", "web", "fetch", "search"],
};
const BROWSER_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "url?:string,max_bytes?:integer,session_id?:string,mode?:string,selector?:string,limit?:integer,link_id?:integer,text?:string,condition?:string,timeout_ms?:integer",
    search_hint: "open pages, inspect structure, follow links, wait on page state, or interact with browser sessions through one direct tool",
    parameter_types: BROWSER_DIRECT_PARAMETER_TYPES,
    required_fields: &[],
    tags: &["surface", "browser", "navigation", "extract"],
};
const MEMORY_DIRECT_METADATA: DirectToolSurfaceMetadata = DirectToolSurfaceMetadata {
    argument_hint: "query?:string,max_results?:integer,path?:string,from?:integer,lines?:integer",
    search_hint: "search durable memory or read one durable memory note file",
    parameter_types: MEMORY_DIRECT_PARAMETER_TYPES,
    required_fields: &[],
    tags: &["surface", "memory", "recall", "read"],
};

const READ_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "read",
    prompt_snippet: "read files, page through large files, search repo text, or list matching paths.",
    prompt_guidance: "Use read for normal repo inspection and file pagination.",
    prompt_guidelines: READ_GUIDELINES,
    direct_tool_name: Some(DIRECT_READ_TOOL_NAME),
    covered_tool_names: READ_COVERED_TOOL_NAMES,
    direct_metadata: Some(READ_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const EDIT_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "edit",
    prompt_snippet: "apply exact text edits to existing files.",
    prompt_guidance: "Use edit for surgical changes to files you have already inspected.",
    prompt_guidelines: EDIT_GUIDELINES,
    direct_tool_name: Some(DIRECT_EDIT_TOOL_NAME),
    covered_tool_names: EDIT_COVERED_TOOL_NAMES,
    direct_metadata: Some(EDIT_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const WRITE_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "write",
    prompt_snippet: "create files or replace whole-file contents.",
    prompt_guidance: "Use write for new files and full rewrites.",
    prompt_guidelines: WRITE_GUIDELINES,
    direct_tool_name: Some(DIRECT_WRITE_TOOL_NAME),
    covered_tool_names: WRITE_COVERED_TOOL_NAMES,
    direct_metadata: Some(WRITE_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const BASH_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "bash",
    prompt_snippet: "run shell commands or shell scripts in the workspace.",
    prompt_guidance: "Use bash for shell-native command strings and shell workflows.",
    prompt_guidelines: BASH_GUIDELINES,
    direct_tool_name: Some(DIRECT_BASH_TOOL_NAME),
    covered_tool_names: BASH_COVERED_TOOL_NAMES,
    direct_metadata: Some(BASH_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const EXEC_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "exec",
    prompt_snippet: "execute a program directly with argv-style arguments.",
    prompt_guidance: "Use exec for direct program execution rather than shell syntax.",
    prompt_guidelines: EXEC_GUIDELINES,
    direct_tool_name: Some(DIRECT_EXEC_TOOL_NAME),
    covered_tool_names: EXEC_COVERED_TOOL_NAMES,
    direct_metadata: Some(EXEC_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const WEB_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "web",
    prompt_snippet: "fetch a URL, search the web, or send an HTTP request.",
    prompt_guidance: "Use web for docs, APIs, scraping, and public references. `query` is search-provider mode; `url` and request fields are normal network mode.",
    prompt_guidelines: WEB_GUIDELINES,
    direct_tool_name: Some(DIRECT_WEB_TOOL_NAME),
    covered_tool_names: WEB_COVERED_TOOL_NAMES,
    direct_metadata: Some(WEB_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const BROWSER_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "browser",
    prompt_snippet: "open pages, inspect page structure, and drive browser sessions.",
    prompt_guidance: "Use browser when the task depends on page structure or interaction.",
    prompt_guidelines: BROWSER_GUIDELINES,
    direct_tool_name: Some(DIRECT_BROWSER_TOOL_NAME),
    covered_tool_names: BROWSER_COVERED_TOOL_NAMES,
    direct_metadata: Some(BROWSER_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

const MEMORY_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "memory",
    prompt_snippet: "search or read durable memory notes.",
    prompt_guidance: "Use memory for persisted notes and recall.",
    prompt_guidelines: MEMORY_GUIDELINES,
    direct_tool_name: Some(DIRECT_MEMORY_TOOL_NAME),
    covered_tool_names: MEMORY_COVERED_TOOL_NAMES,
    direct_metadata: Some(MEMORY_DIRECT_METADATA),
    hidden_search_summary: None,
    hidden_search_argument_hint: None,
};

// `agent` and `skills` are grouped hidden facades for Loong core behavior.
// `channel` is also grouped, but it remains an addon lane instead of being
// folded into the core runtime-control vocabulary.
const AGENT_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "agent",
    prompt_snippet: "inspect approvals, sessions, delegation, model routing, or config state.",
    prompt_guidance: "Use this when the task is about Loong's own runtime, setup, or control flow.",
    prompt_guidelines: AGENT_GUIDELINES,
    direct_tool_name: None,
    covered_tool_names: &[],
    direct_metadata: None,
    hidden_search_summary: Some(
        "Inspect approvals, sessions, delegation, provider routing, or config migration through one hidden control tool.",
    ),
    hidden_search_argument_hint: Some(
        "operation?:string,session_id?:string,approval_request_id?:string,decision?:string,task?:string,selector?:string,query?:string,text?:string,input?:string,input_path?:string,output_path?:string",
    ),
};

const SKILLS_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "skills",
    prompt_snippet: "search, inspect, install, run, or manage external skills.",
    prompt_guidance: "Use this when the task is about capability expansion.",
    prompt_guidelines: SKILLS_GUIDELINES,
    direct_tool_name: None,
    covered_tool_names: &[],
    direct_metadata: None,
    hidden_search_summary: Some(
        "Search, inspect, install, or run external skills through one hidden capability tool.",
    ),
    hidden_search_argument_hint: Some(
        "operation?:string,query?:string,skill_id?:string,path?:string,bundled_skill_id?:string,source_skill_id?:string,limit?:integer",
    ),
};

// Keep channel-specific tools out of the core hidden facades. For example,
// Feishu belongs to the addon/channel lane instead of `agent` or `skills`.
const CHANNEL_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "channel",
    prompt_snippet: "operate channel-specific tools such as Feishu.",
    prompt_guidance: "Use this only when the task explicitly targets that channel.",
    prompt_guidelines: CHANNEL_GUIDELINES,
    direct_tool_name: None,
    covered_tool_names: &[],
    direct_metadata: None,
    hidden_search_summary: Some(
        "Operate channel-specific capabilities such as Feishu through one separate addon tool.",
    ),
    hidden_search_argument_hint: Some(
        "operation:string,account_id?:string,open_id?:string,receive_id?:string,message_id?:string,url?:string,query?:string",
    ),
};

const ALL_TOOL_SURFACES: &[ToolSurfaceDescriptor] = &[
    READ_SURFACE,
    EDIT_SURFACE,
    WRITE_SURFACE,
    BASH_SURFACE,
    EXEC_SURFACE,
    WEB_SURFACE,
    BROWSER_SURFACE,
    MEMORY_SURFACE,
    AGENT_SURFACE,
    SKILLS_SURFACE,
    CHANNEL_SURFACE,
];

fn dotted_variant(raw: &str) -> String {
    raw.replace('-', ".")
}

fn underscored_variant(raw: &str) -> String {
    raw.replace('-', "_")
}

fn matches_surface_name(raw: &str, expected: &str) -> bool {
    if raw == expected {
        return true;
    }

    let dotted = dotted_variant(raw);
    if dotted == expected {
        return true;
    }

    let underscored = underscored_variant(raw);
    underscored == expected
}

fn generic_discovery_tool_name_for_tool_name(tool_name: &str) -> String {
    let discovery_name = tool_name.replace('.', "-");
    discovery_name.replace('_', "-")
}

fn browser_surface_matches_tool_name(tool_name: &str) -> bool {
    matches_surface_name(tool_name, "browser.open")
        || matches_surface_name(tool_name, "browser.extract")
        || matches_surface_name(tool_name, "browser.click")
        || tool_name.starts_with("browser.companion.")
}

fn surface_covers_tool_name(surface: &ToolSurfaceDescriptor, tool_name: &str) -> bool {
    if surface.id == DIRECT_BROWSER_TOOL_NAME {
        return browser_surface_matches_tool_name(tool_name);
    }

    surface
        .covered_tool_names
        .iter()
        .any(|covered_tool_name| matches_surface_name(tool_name, covered_tool_name))
}

fn surface_has_visible_covered_tool(surface: &ToolSurfaceDescriptor, view: &ToolView) -> bool {
    if surface.id == DIRECT_BROWSER_TOOL_NAME {
        return view.tool_names().any(browser_surface_matches_tool_name);
    }

    surface
        .covered_tool_names
        .iter()
        .any(|covered_tool_name| view.contains(covered_tool_name))
}

pub(crate) fn discovery_tool_name_for_tool_name(tool_name: &str) -> String {
    if let Some(direct_tool_name) = direct_tool_name_for_hidden_tool(tool_name) {
        return direct_tool_name.to_owned();
    }

    if let Some(surface) = tool_surface_for_name(tool_name) {
        let direct_tool_name = surface.direct_tool_name;
        let direct_name_matches = direct_tool_name == Some(tool_name);
        if direct_name_matches {
            return surface.id.to_owned();
        }
    }

    if let Some(hidden_facade_tool_name) = hidden_facade_tool_name_for_hidden_tool(tool_name) {
        return hidden_facade_tool_name.to_owned();
    }

    generic_discovery_tool_name_for_tool_name(tool_name)
}

pub(crate) fn legacy_discovery_tool_names_for_tool_name(tool_name: &str) -> Vec<String> {
    let current_name = discovery_tool_name_for_tool_name(tool_name);
    let legacy_name = generic_discovery_tool_name_for_tool_name(tool_name);
    if current_name == legacy_name {
        Vec::new()
    } else {
        vec![legacy_name]
    }
}

pub(crate) fn tool_surface_for_name(tool_name: &str) -> Option<&'static ToolSurfaceDescriptor> {
    if let Some(surface) = direct_surface_descriptor_for_tool_name(tool_name) {
        return Some(surface);
    }

    let surface = if tool_name == "agent"
        || matches_surface_name(tool_name, "approval_requests_list")
        || matches_surface_name(tool_name, "approval_request_status")
        || matches_surface_name(tool_name, "approval_request_resolve")
        || tool_name.starts_with("session_")
        || tool_name.starts_with("sessions_")
        || matches_surface_name(tool_name, "session_events")
        || matches_surface_name(tool_name, "session_search")
        || matches_surface_name(tool_name, "session_status")
        || matches_surface_name(tool_name, "session_wait")
        || matches_surface_name(tool_name, "session_archive")
        || matches_surface_name(tool_name, "session_cancel")
        || matches_surface_name(tool_name, "session_continue")
        || matches_surface_name(tool_name, "session_recover")
        || matches_surface_name(tool_name, "session_tool_policy_status")
        || matches_surface_name(tool_name, "session_tool_policy_set")
        || matches_surface_name(tool_name, "session_tool_policy_clear")
        || matches_surface_name(tool_name, "sessions_history")
        || matches_surface_name(tool_name, "sessions_list")
        || matches_surface_name(tool_name, "sessions_send")
        || tool_name == "delegate"
        || matches_surface_name(tool_name, "delegate_async")
        || matches_surface_name(tool_name, "provider.switch")
        || matches_surface_name(tool_name, "config.import")
    {
        &AGENT_SURFACE
    } else if tool_name == "skills"
        || tool_name.starts_with("external_skills.")
        || matches_surface_name(tool_name, "external_skills.search")
        || matches_surface_name(tool_name, "external_skills.policy")
        || matches_surface_name(tool_name, "external_skills.inspect")
        || matches_surface_name(tool_name, "external_skills.install")
        || matches_surface_name(tool_name, "external_skills.invoke")
        || matches_surface_name(tool_name, "external_skills.list")
    {
        &SKILLS_SURFACE
    } else if tool_name == "channel"
        || tool_name.starts_with("feishu.")
        || matches_surface_name(tool_name, "feishu.whoami")
    {
        &CHANNEL_SURFACE
    } else {
        return None;
    };

    Some(surface)
}

pub(crate) fn tool_surface_id_for_name(tool_name: &str) -> Option<&'static str> {
    let surface = tool_surface_for_name(tool_name)?;
    Some(surface.id)
}

fn tool_surface_descriptor_for_id(surface_id: &str) -> Option<&'static ToolSurfaceDescriptor> {
    ALL_TOOL_SURFACES
        .iter()
        .find(|descriptor| descriptor.id == surface_id)
}

fn direct_surface_descriptor_for_tool_name(
    tool_name: &str,
) -> Option<&'static ToolSurfaceDescriptor> {
    ALL_TOOL_SURFACES.iter().find(|surface| {
        let Some(direct_tool_name) = surface.direct_tool_name else {
            return false;
        };

        matches_surface_name(tool_name, direct_tool_name)
            || surface_covers_tool_name(surface, tool_name)
    })
}

fn direct_surface_descriptor_for_direct_tool_name(
    tool_name: &str,
) -> Option<&'static ToolSurfaceDescriptor> {
    ALL_TOOL_SURFACES.iter().find(|surface| {
        surface
            .direct_tool_name
            .is_some_and(|direct_tool_name| direct_tool_name == tool_name)
    })
}

pub(crate) fn is_tool_surface_id(surface_id: &str) -> bool {
    tool_surface_descriptor_for_id(surface_id).is_some()
}

pub(crate) fn tool_surface_usage_guidance(tool_name: &str) -> Option<&'static str> {
    let surface = tool_surface_for_name(tool_name)?;
    Some(surface.prompt_guidance)
}

pub(crate) fn tool_surface_prompt_guidelines_for_id(
    surface_id: &str,
) -> Option<&'static [&'static str]> {
    let surface = tool_surface_descriptor_for_id(surface_id)?;
    Some(surface.prompt_guidelines)
}

pub(crate) fn hidden_surface_search_summary(surface_id: &str) -> Option<&'static str> {
    let surface = tool_surface_descriptor_for_id(surface_id)?;
    surface.hidden_search_summary
}

pub(crate) fn hidden_surface_search_argument_hint(surface_id: &str) -> Option<&'static str> {
    let surface = tool_surface_descriptor_for_id(surface_id)?;
    surface.hidden_search_argument_hint
}

fn direct_tool_surface_metadata(tool_name: &str) -> Option<DirectToolSurfaceMetadata> {
    let surface = tool_surface_for_name(tool_name)?;
    let direct_tool_name = surface.direct_tool_name?;
    if direct_tool_name != tool_name {
        return None;
    }
    surface.direct_metadata
}

pub(crate) fn direct_tool_argument_hint(tool_name: &str) -> Option<&'static str> {
    let metadata = direct_tool_surface_metadata(tool_name)?;
    Some(metadata.argument_hint)
}

pub(crate) fn direct_tool_search_hint(tool_name: &str) -> Option<&'static str> {
    let metadata = direct_tool_surface_metadata(tool_name)?;
    Some(metadata.search_hint)
}

pub(crate) fn direct_tool_parameter_types(
    tool_name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    let metadata = direct_tool_surface_metadata(tool_name)?;
    Some(metadata.parameter_types)
}

pub(crate) fn direct_tool_required_fields(tool_name: &str) -> Option<&'static [&'static str]> {
    let metadata = direct_tool_surface_metadata(tool_name)?;
    Some(metadata.required_fields)
}

pub(crate) fn direct_tool_tags(tool_name: &str) -> Option<&'static [&'static str]> {
    let metadata = direct_tool_surface_metadata(tool_name)?;
    Some(metadata.tags)
}

pub(crate) fn hidden_facade_tool_name_for_hidden_tool(tool_name: &str) -> Option<&'static str> {
    // Keep the addon boundary explicit, but still collapse channel tools into
    // their own grouped facade so the model does not learn a long tail of ids.
    if matches_surface_name(tool_name, "approval_requests_list")
        || matches_surface_name(tool_name, "approval_request_status")
        || matches_surface_name(tool_name, "approval_request_resolve")
        || tool_name.starts_with("session_")
        || tool_name.starts_with("sessions_")
        || matches_surface_name(tool_name, "delegate")
        || matches_surface_name(tool_name, "delegate_async")
        || matches_surface_name(tool_name, "provider.switch")
        || matches_surface_name(tool_name, "config.import")
    {
        return Some("agent");
    }

    if matches!(
        tool_name,
        "external_skills.search"
            | "external_skills.policy"
            | "external_skills.inspect"
            | "external_skills.install"
            | "external_skills.invoke"
            | "external_skills.list"
    ) {
        return Some("skills");
    }

    if tool_name.starts_with("feishu.") {
        return Some("channel");
    }

    None
}

pub(crate) fn direct_tool_name_for_hidden_tool(tool_name: &str) -> Option<&'static str> {
    let surface = direct_surface_descriptor_for_tool_name(tool_name)?;
    let direct_tool_name = surface.direct_tool_name?;
    if matches_surface_name(tool_name, direct_tool_name) {
        return None;
    }

    Some(direct_tool_name)
}

pub(crate) fn visible_direct_tool_states_for_view(view: &ToolView) -> Vec<ToolSurfaceState> {
    let mut states = Vec::new();

    for surface in ALL_TOOL_SURFACES {
        let Some(direct_tool_name) = surface.direct_tool_name else {
            continue;
        };
        let direct_tool_visible = direct_tool_visible_in_view(direct_tool_name, view);
        if !direct_tool_visible {
            continue;
        }
        let state = direct_surface_state_for_view(*surface, view);
        states.push(state);
    }

    states
}

fn direct_surface_state_for_view(
    surface: ToolSurfaceDescriptor,
    view: &ToolView,
) -> ToolSurfaceState {
    if surface.id == WEB_SURFACE.id {
        return web_surface_state_for_view(surface, view);
    }

    surface.into_state(Vec::new())
}

pub(crate) fn direct_web_runtime_modes_for_view(view: &ToolView) -> DirectWebRuntimeModes {
    DirectWebRuntimeModes::from_view(view)
}

fn web_surface_state_for_view(surface: ToolSurfaceDescriptor, view: &ToolView) -> ToolSurfaceState {
    let web_runtime_modes = direct_web_runtime_modes_for_view(view);
    let (prompt_snippet, usage_guidance) = web_runtime_modes.prompt_state(surface);

    ToolSurfaceState {
        surface_id: surface.id.to_owned(),
        prompt_snippet: prompt_snippet.to_owned(),
        usage_guidance: usage_guidance.to_owned(),
        tool_ids: Vec::new(),
    }
}

pub(crate) fn direct_tool_visible_in_view(tool_name: &str, view: &ToolView) -> bool {
    let Some(surface) = direct_surface_descriptor_for_direct_tool_name(tool_name) else {
        return false;
    };

    surface_has_visible_covered_tool(surface, view)
}

pub(crate) fn hidden_tool_is_covered_by_visible_direct_tool(
    tool_name: &str,
    view: &ToolView,
) -> bool {
    let Some(direct_tool_name) = direct_tool_name_for_hidden_tool(tool_name) else {
        return false;
    };

    direct_tool_visible_in_view(direct_tool_name, view)
}

pub(crate) fn tool_surface_visible_in_view(surface_id: &str, view: &ToolView) -> bool {
    let Some(surface) = tool_surface_descriptor_for_id(surface_id) else {
        return false;
    };

    if let Some(direct_tool_name) = surface.direct_tool_name {
        return direct_tool_visible_in_view(direct_tool_name, view);
    }

    view.tool_names().any(|tool_name| {
        tool_surface_id_for_name(tool_name) == Some(surface_id)
            && !hidden_tool_is_covered_by_visible_direct_tool(tool_name, view)
    })
}

pub(crate) fn active_discoverable_tool_surface_states<'a>(
    tool_names: impl IntoIterator<Item = &'a str>,
) -> Vec<ToolSurfaceState> {
    let mut tool_ids_by_surface = BTreeMap::<&'static str, BTreeSet<String>>::new();

    for tool_name in tool_names {
        let Some(surface) = tool_surface_for_name(tool_name) else {
            continue;
        };
        let entry = tool_ids_by_surface.entry(surface.id).or_default();
        let discovery_tool_name = discovery_tool_name_for_tool_name(tool_name);
        entry.insert(discovery_tool_name);
    }

    let mut states = Vec::new();

    for surface in ALL_TOOL_SURFACES {
        let Some(tool_ids) = tool_ids_by_surface.remove(surface.id) else {
            continue;
        };
        let tool_ids = tool_ids.into_iter().collect();
        let state = surface.into_state(tool_ids);
        states.push(state);
    }

    states
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_direct_tool_states_follow_runtime_view() {
        let view = ToolView::from_tool_names([
            "file.read",
            "file.write",
            "file.edit",
            "bash.exec",
            "shell.exec",
            "web.fetch",
            "memory_search",
        ]);

        let states = visible_direct_tool_states_for_view(&view);
        let state_ids: Vec<&str> = states
            .iter()
            .map(|state| state.surface_id.as_str())
            .collect();

        assert_eq!(
            state_ids,
            vec!["read", "edit", "write", "bash", "exec", "web", "memory"]
        );
    }

    #[test]
    fn hidden_surface_states_group_tools_deterministically() {
        let states = active_discoverable_tool_surface_states([
            "bash.exec",
            "provider.switch",
            "delegate",
            "delegate_async",
        ]);

        assert_eq!(states.len(), 2);
        assert_eq!(states[0].surface_id, "bash");
        assert_eq!(states[0].tool_ids, vec!["bash"]);
        assert_eq!(states[1].surface_id, "agent");
        assert_eq!(states[1].tool_ids, vec!["agent"]);
    }

    #[test]
    fn direct_surface_coverage_only_applies_to_common_hidden_tools() {
        let view = ToolView::from_tool_names([
            "bash.exec",
            "shell.exec",
            "browser.open",
            "browser.companion.snapshot",
            "http.request",
        ]);

        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "shell.exec",
            &view
        ));
        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "bash.exec",
            &view
        ));
        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "browser.open",
            &view
        ));
        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "browser.companion.snapshot",
            &view
        ));
        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "http.request",
            &view
        ));
    }

    #[test]
    fn direct_surface_metadata_stays_definition_first() {
        let exec_parameter_types =
            direct_tool_parameter_types(DIRECT_EXEC_TOOL_NAME).expect("exec parameter types");
        assert!(exec_parameter_types.contains(&("args", "array")));
        assert!(!exec_parameter_types.contains(&("script", "string")));
        let bash_parameter_types =
            direct_tool_parameter_types(DIRECT_BASH_TOOL_NAME).expect("bash parameter types");
        assert!(bash_parameter_types.contains(&("script", "string")));
        assert!(
            direct_tool_required_fields(DIRECT_BASH_TOOL_NAME)
                .is_some_and(|fields| fields.is_empty())
        );
        assert_eq!(
            direct_tool_required_fields(DIRECT_EDIT_TOOL_NAME),
            Some(["path"].as_slice())
        );
        assert_eq!(
            direct_tool_required_fields(DIRECT_WRITE_TOOL_NAME),
            Some(["path", "content"].as_slice())
        );
        assert_eq!(
            direct_tool_tags(DIRECT_WEB_TOOL_NAME),
            Some(["surface", "web", "fetch", "search"].as_slice())
        );
        assert!(
            direct_tool_search_hint(DIRECT_WEB_TOOL_NAME)
                .expect("web search hint")
                .contains("web-search providers")
        );
        assert!(
            direct_tool_argument_hint(DIRECT_READ_TOOL_NAME)
                .expect("read argument hint")
                .contains("offset?:integer")
        );
        assert!(
            direct_tool_search_hint(DIRECT_BROWSER_TOOL_NAME)
                .expect("browser search hint")
                .contains("browser sessions")
        );
        assert!(
            direct_tool_search_hint(DIRECT_BASH_TOOL_NAME)
                .expect("bash search hint")
                .contains("shell commands")
        );
        assert!(
            direct_tool_parameter_types(DIRECT_BROWSER_TOOL_NAME)
                .expect("browser parameter types")
                .contains(&("text", "string"))
        );
        assert_eq!(direct_tool_argument_hint("shell.exec"), None);
    }

    #[test]
    fn discovery_ids_prefer_surfaces_over_curated_long_tail_aliases() {
        assert_eq!(
            discovery_tool_name_for_tool_name("file.read"),
            DIRECT_READ_TOOL_NAME
        );
        assert_eq!(
            discovery_tool_name_for_tool_name("bash.exec"),
            DIRECT_BASH_TOOL_NAME
        );
        assert_eq!(
            discovery_tool_name_for_tool_name("browser.companion.session.start"),
            DIRECT_BROWSER_TOOL_NAME
        );
        assert_eq!(
            discovery_tool_name_for_tool_name("external_skills.install"),
            "skills"
        );
        assert_eq!(
            discovery_tool_name_for_tool_name("provider.switch"),
            "agent"
        );
        assert_eq!(
            discovery_tool_name_for_tool_name("feishu.messages.send"),
            "channel"
        );
    }

    #[test]
    fn surface_visibility_checks_support_grouped_hidden_and_direct_paths() {
        let view = ToolView::from_tool_names([
            "bash.exec",
            "file.read",
            "shell.exec",
            "browser.companion.snapshot",
            "provider.switch",
            "feishu.messages.send",
        ]);

        assert!(tool_surface_visible_in_view("read", &view));
        assert!(tool_surface_visible_in_view("bash", &view));
        assert!(tool_surface_visible_in_view("exec", &view));
        assert!(tool_surface_visible_in_view("browser", &view));
        assert!(tool_surface_visible_in_view("agent", &view));
        assert!(tool_surface_visible_in_view("channel", &view));
        assert!(!tool_surface_visible_in_view("skills", &view));
    }

    #[test]
    fn visible_web_surface_state_distinguishes_search_from_network_modes() {
        let search_only_view = ToolView::from_tool_names(["web.search"]);
        let search_only_state = visible_direct_tool_states_for_view(&search_only_view)
            .into_iter()
            .find(|state| state.surface_id == "web")
            .expect("web surface for search-only view");
        assert!(
            search_only_state
                .prompt_snippet
                .contains("search the public web")
        );
        assert!(
            search_only_state
                .usage_guidance
                .contains("Query mode uses web-search providers")
        );

        let network_only_view = ToolView::from_tool_names(["web.fetch"]);
        let network_only_state = visible_direct_tool_states_for_view(&network_only_view)
            .into_iter()
            .find(|state| state.surface_id == "web")
            .expect("web surface for network-only view");
        assert!(network_only_state.prompt_snippet.contains("fetch a URL"));
        assert!(
            network_only_state
                .usage_guidance
                .contains("Query search mode is unavailable")
        );
    }

    #[test]
    fn channel_surface_stays_separate_from_core_hidden_facades() {
        assert_eq!(
            tool_surface_id_for_name("feishu.messages.send"),
            Some("channel")
        );
        assert_eq!(
            hidden_facade_tool_name_for_hidden_tool("feishu.messages.send"),
            Some("channel")
        );
    }
}
