use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::ToolView;

pub(crate) const DIRECT_READ_TOOL_NAME: &str = "read";
pub(crate) const DIRECT_WRITE_TOOL_NAME: &str = "write";
pub(crate) const DIRECT_EXEC_TOOL_NAME: &str = "exec";
pub(crate) const DIRECT_WEB_TOOL_NAME: &str = "web";
pub(crate) const DIRECT_BROWSER_TOOL_NAME: &str = "browser";
pub(crate) const DIRECT_MEMORY_TOOL_NAME: &str = "memory";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolSurfaceDescriptor {
    pub(crate) id: &'static str,
    pub(crate) prompt_snippet: &'static str,
    pub(crate) prompt_guidance: &'static str,
    pub(crate) direct_tool_name: Option<&'static str>,
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
        let tool_count = self.tool_count();
        let tool_count_label = if tool_count == 1 {
            "1 hidden tool"
        } else {
            "multiple hidden tools"
        };
        format!(
            "- {} ({}): {} {}",
            self.surface_id, tool_count_label, self.prompt_snippet, self.usage_guidance
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

const READ_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "read",
    prompt_snippet: "read files, search workspace content, and list matching paths.",
    prompt_guidance: "Use direct read first for normal repo inspection. Search only when the task needs a more specialized hidden file or config tool.",
    direct_tool_name: Some(DIRECT_READ_TOOL_NAME),
};

const WRITE_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "write",
    prompt_snippet: "create files or apply exact text edits in the workspace.",
    prompt_guidance: "Use direct write for normal file creation and patch work. It still stays inside policy, approval, and audit boundaries.",
    direct_tool_name: Some(DIRECT_WRITE_TOOL_NAME),
};

const EXEC_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "exec",
    prompt_snippet: "run guarded workspace commands.",
    prompt_guidance: "Use direct exec for normal command execution. Search only when you explicitly need the hidden bash-oriented surface.",
    direct_tool_name: Some(DIRECT_EXEC_TOOL_NAME),
};

const WEB_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "web",
    prompt_snippet: "fetch a URL or search the public web under runtime policy.",
    prompt_guidance: "Use direct web first for normal fetch and search tasks. Search only when you need lower-level hidden HTTP behavior.",
    direct_tool_name: Some(DIRECT_WEB_TOOL_NAME),
};

const BROWSER_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "browser",
    prompt_snippet: "open pages, extract structured content, and follow discovered links.",
    prompt_guidance: "Use direct browser for bounded page navigation. Search only when the task needs hidden managed browser companion operations.",
    direct_tool_name: Some(DIRECT_BROWSER_TOOL_NAME),
};

const MEMORY_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "memory",
    prompt_snippet: "search or read durable memory notes.",
    prompt_guidance: "Use memory for persisted notes and recall, not general repo browsing.",
    direct_tool_name: Some(DIRECT_MEMORY_TOOL_NAME),
};

const APPROVAL_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "approval",
    prompt_snippet: "inspect or resolve pending governed actions.",
    prompt_guidance: "Use this only when the user is acting as operator for an approval-gated action.",
    direct_tool_name: None,
};

const SESSION_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "session",
    prompt_snippet: "inspect prior runs, waits, events, and session policy state.",
    prompt_guidance: "Use this when the user asks about history or an in-flight session instead of guessing from memory.",
    direct_tool_name: None,
};

const DELEGATE_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "delegate",
    prompt_snippet: "spawn or queue bounded child work.",
    prompt_guidance: "Use this for longer or parallel follow-up tasks instead of promising to do them later.",
    direct_tool_name: None,
};

const PROVIDER_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "provider",
    prompt_snippet: "inspect or switch provider and model routing deliberately.",
    prompt_guidance: "Use this only when the user explicitly wants a different route or the task requires one.",
    direct_tool_name: None,
};

const EXTERNAL_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "external",
    prompt_snippet: "inspect, fetch, install, or govern external skills.",
    prompt_guidance: "Treat this as capability expansion or policy control rather than routine execution.",
    direct_tool_name: None,
};

const CONFIG_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "config",
    prompt_snippet: "import or migrate workspace-owned config state.",
    prompt_guidance: "Use this only for explicit migration or compatibility work, not routine repo inspection.",
    direct_tool_name: None,
};

const CHANNEL_SURFACE: ToolSurfaceDescriptor = ToolSurfaceDescriptor {
    id: "channel",
    prompt_snippet: "operate explicit channel-specific workflows such as Feishu tooling.",
    prompt_guidance: "Use this only when the task explicitly targets that channel runtime.",
    direct_tool_name: None,
};

const ALL_TOOL_SURFACES: &[ToolSurfaceDescriptor] = &[
    READ_SURFACE,
    WRITE_SURFACE,
    EXEC_SURFACE,
    WEB_SURFACE,
    BROWSER_SURFACE,
    MEMORY_SURFACE,
    APPROVAL_SURFACE,
    SESSION_SURFACE,
    DELEGATE_SURFACE,
    PROVIDER_SURFACE,
    EXTERNAL_SURFACE,
    CONFIG_SURFACE,
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

pub(crate) fn discovery_tool_name_for_tool_name(tool_name: &str) -> String {
    let discovery_name = tool_name.replace('.', "-");
    discovery_name.replace('_', "-")
}

pub(crate) fn tool_surface_for_name(tool_name: &str) -> Option<&'static ToolSurfaceDescriptor> {
    let surface = if tool_name == DIRECT_READ_TOOL_NAME
        || matches_surface_name(tool_name, "file.read")
        || matches_surface_name(tool_name, "glob.search")
        || matches_surface_name(tool_name, "content.search")
    {
        &READ_SURFACE
    } else if tool_name == DIRECT_WRITE_TOOL_NAME
        || matches_surface_name(tool_name, "file.write")
        || matches_surface_name(tool_name, "file.edit")
    {
        &WRITE_SURFACE
    } else if tool_name == DIRECT_EXEC_TOOL_NAME
        || matches_surface_name(tool_name, "shell.exec")
        || matches_surface_name(tool_name, "bash.exec")
    {
        &EXEC_SURFACE
    } else if tool_name == DIRECT_WEB_TOOL_NAME
        || matches_surface_name(tool_name, "web.fetch")
        || matches_surface_name(tool_name, "web.search")
        || matches_surface_name(tool_name, "http.request")
    {
        &WEB_SURFACE
    } else if tool_name == DIRECT_BROWSER_TOOL_NAME
        || matches_surface_name(tool_name, "browser.open")
        || matches_surface_name(tool_name, "browser.extract")
        || matches_surface_name(tool_name, "browser.click")
        || matches_surface_name(tool_name, "browser.companion.session.start")
        || matches_surface_name(tool_name, "browser.companion.navigate")
        || matches_surface_name(tool_name, "browser.companion.snapshot")
        || matches_surface_name(tool_name, "browser.companion.wait")
        || matches_surface_name(tool_name, "browser.companion.session.stop")
        || matches_surface_name(tool_name, "browser.companion.click")
        || matches_surface_name(tool_name, "browser.companion.type")
    {
        &BROWSER_SURFACE
    } else if tool_name == DIRECT_MEMORY_TOOL_NAME
        || matches_surface_name(tool_name, "memory_search")
        || matches_surface_name(tool_name, "memory_get")
    {
        &MEMORY_SURFACE
    } else if matches_surface_name(tool_name, "approval_requests_list")
        || matches_surface_name(tool_name, "approval_request_status")
        || matches_surface_name(tool_name, "approval_request_resolve")
    {
        &APPROVAL_SURFACE
    } else if tool_name.starts_with("session_")
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
    {
        &SESSION_SURFACE
    } else if tool_name == "delegate" || matches_surface_name(tool_name, "delegate_async") {
        &DELEGATE_SURFACE
    } else if matches_surface_name(tool_name, "provider.switch") {
        &PROVIDER_SURFACE
    } else if tool_name.starts_with("external_skills.")
        || matches_surface_name(tool_name, "external_skills.fetch")
        || matches_surface_name(tool_name, "external_skills.resolve")
        || matches_surface_name(tool_name, "external_skills.search")
        || matches_surface_name(tool_name, "external_skills.recommend")
        || matches_surface_name(tool_name, "external_skills.source_search")
        || matches_surface_name(tool_name, "external_skills.inspect")
        || matches_surface_name(tool_name, "external_skills.install")
        || matches_surface_name(tool_name, "external_skills.invoke")
        || matches_surface_name(tool_name, "external_skills.list")
        || matches_surface_name(tool_name, "external_skills.policy")
        || matches_surface_name(tool_name, "external_skills.remove")
    {
        &EXTERNAL_SURFACE
    } else if matches_surface_name(tool_name, "config.import") {
        &CONFIG_SURFACE
    } else if tool_name.starts_with("feishu.") || matches_surface_name(tool_name, "feishu.whoami") {
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

pub(crate) fn tool_surface_usage_guidance(tool_name: &str) -> Option<&'static str> {
    let surface = tool_surface_for_name(tool_name)?;
    Some(surface.prompt_guidance)
}

pub(crate) fn direct_tool_name_for_hidden_tool(tool_name: &str) -> Option<&'static str> {
    if matches_surface_name(tool_name, "file.read")
        || matches_surface_name(tool_name, "glob.search")
        || matches_surface_name(tool_name, "content.search")
    {
        return Some(DIRECT_READ_TOOL_NAME);
    }

    if matches_surface_name(tool_name, "file.write") || matches_surface_name(tool_name, "file.edit")
    {
        return Some(DIRECT_WRITE_TOOL_NAME);
    }

    if matches_surface_name(tool_name, "shell.exec") {
        return Some(DIRECT_EXEC_TOOL_NAME);
    }

    if matches_surface_name(tool_name, "web.fetch") || matches_surface_name(tool_name, "web.search")
    {
        return Some(DIRECT_WEB_TOOL_NAME);
    }

    if matches_surface_name(tool_name, "browser.open")
        || matches_surface_name(tool_name, "browser.extract")
        || matches_surface_name(tool_name, "browser.click")
    {
        return Some(DIRECT_BROWSER_TOOL_NAME);
    }

    if matches_surface_name(tool_name, "memory_search")
        || matches_surface_name(tool_name, "memory_get")
    {
        return Some(DIRECT_MEMORY_TOOL_NAME);
    }

    None
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
        let state = surface.into_state(Vec::new());
        states.push(state);
    }

    states
}

pub(crate) fn direct_tool_visible_in_view(tool_name: &str, view: &ToolView) -> bool {
    let covered_tool_names = match tool_name {
        DIRECT_READ_TOOL_NAME => &["file.read", "glob.search", "content.search"][..],
        DIRECT_WRITE_TOOL_NAME => &["file.write", "file.edit"][..],
        DIRECT_EXEC_TOOL_NAME => &["shell.exec"][..],
        DIRECT_WEB_TOOL_NAME => &["web.fetch", "web.search"][..],
        DIRECT_BROWSER_TOOL_NAME => &["browser.open", "browser.extract", "browser.click"][..],
        DIRECT_MEMORY_TOOL_NAME => &["memory_search", "memory_get"][..],
        _ => &[][..],
    };

    for covered_tool_name in covered_tool_names {
        let tool_visible = view.contains(covered_tool_name);
        if tool_visible {
            return true;
        }
    }

    false
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
            "shell.exec",
            "web.fetch",
            "memory_search",
        ]);

        let states = visible_direct_tool_states_for_view(&view);
        let state_ids: Vec<&str> = states
            .iter()
            .map(|state| state.surface_id.as_str())
            .collect();

        assert_eq!(state_ids, vec!["read", "write", "exec", "web", "memory"]);
    }

    #[test]
    fn hidden_surface_states_group_tools_deterministically() {
        let states = active_discoverable_tool_surface_states([
            "bash.exec",
            "provider.switch",
            "delegate",
            "delegate_async",
        ]);

        assert_eq!(states.len(), 3);
        assert_eq!(states[0].surface_id, "exec");
        assert_eq!(states[0].tool_ids, vec!["bash-exec"]);
        assert_eq!(states[1].surface_id, "delegate");
        assert_eq!(states[1].tool_ids, vec!["delegate", "delegate-async"]);
        assert_eq!(states[2].surface_id, "provider");
        assert_eq!(states[2].tool_ids, vec!["provider-switch"]);
    }

    #[test]
    fn direct_surface_coverage_only_applies_to_common_hidden_tools() {
        let view = ToolView::from_tool_names(["shell.exec", "browser.open", "http.request"]);

        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "shell.exec",
            &view
        ));
        assert!(hidden_tool_is_covered_by_visible_direct_tool(
            "browser.open",
            &view
        ));
        assert!(!hidden_tool_is_covered_by_visible_direct_tool(
            "bash.exec",
            &view
        ));
        assert!(!hidden_tool_is_covered_by_visible_direct_tool(
            "http.request",
            &view
        ));
    }
}
