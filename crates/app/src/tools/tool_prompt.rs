use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiscoverableToolPromptFamily {
    pub(crate) id: &'static str,
    pub(crate) prompt_snippet: &'static str,
    pub(crate) prompt_guidance: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverableCapabilityFamilyState {
    pub family_id: String,
    pub prompt_snippet: String,
    pub usage_guidance: String,
    #[serde(default)]
    pub tool_ids: Vec<String>,
}

impl DiscoverableCapabilityFamilyState {
    pub fn tool_count(&self) -> usize {
        self.tool_ids.len()
    }

    fn render_prompt_line(&self) -> String {
        format!(
            "- {}: {} {}",
            self.family_id, self.prompt_snippet, self.usage_guidance
        )
    }
}

impl DiscoverableToolPromptFamily {
    fn into_state(self, tool_ids: Vec<String>) -> DiscoverableCapabilityFamilyState {
        DiscoverableCapabilityFamilyState {
            family_id: self.id.to_owned(),
            prompt_snippet: self.prompt_snippet.to_owned(),
            usage_guidance: self.prompt_guidance.to_owned(),
            tool_ids,
        }
    }
}

const APPROVAL_WORKFLOWS: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "approval_workflows",
    prompt_snippet: "inspect or resolve pending governed actions.",
    prompt_guidance: "Use this only when the user is acting as operator for an approval-gated action.",
};

const LOCAL_FILES: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "local_files",
    prompt_snippet: "inspect, search, and edit workspace files deterministically.",
    prompt_guidance: "Prefer this family before shell for source, config, and patch-oriented work.",
};

const SHELL_RUNTIME: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "shell_runtime",
    prompt_snippet: "run guarded shell commands inside the configured workspace.",
    prompt_guidance: "Use this for build/test/CLI workflows or when deterministic file tools are insufficient.",
};

const WEB_RESEARCH: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "web_research",
    prompt_snippet: "fetch known URLs or search the open web under runtime policy.",
    prompt_guidance: "Prefer direct fetch for a specific URL and search for open-ended discovery.",
};

const BROWSER_AUTOMATION: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "browser_automation",
    prompt_snippet: "open pages, follow links, and extract DOM-aware content.",
    prompt_guidance: "Escalate here only when navigation, rendered structure, or interactive page state matters.",
};

const SESSION_STATE: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "session_state",
    prompt_snippet: "inspect prior runs, waits, events, and session policy state.",
    prompt_guidance: "Use this when the user asks about history or an in-flight session instead of guessing from memory.",
};

const DELEGATION: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "delegation",
    prompt_snippet: "spawn or queue bounded child work.",
    prompt_guidance: "Use this for longer or parallel follow-up tasks instead of promising to do them later.",
};

const PROVIDER_ROUTING: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "provider_routing",
    prompt_snippet: "switch provider or model routing deliberately.",
    prompt_guidance: "Use this only when the user explicitly wants a different route or the task requires one.",
};

const EXTERNAL_CAPABILITIES: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "external_capabilities",
    prompt_snippet: "inspect or govern installable external skills.",
    prompt_guidance: "Treat this as capability expansion or policy control rather than routine execution.",
};

const MEMORY_RECALL: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "memory_recall",
    prompt_snippet: "search or read durable memory notes.",
    prompt_guidance: "Use memory for persisted notes and recall, not general repo browsing.",
};

const CHANNEL_OPERATIONS: DiscoverableToolPromptFamily = DiscoverableToolPromptFamily {
    id: "channel_operations",
    prompt_snippet: "operate explicit channel-specific surfaces such as Feishu workflows.",
    prompt_guidance: "Use this only when the task explicitly targets that channel runtime.",
};

const ALL_DISCOVERABLE_TOOL_PROMPT_FAMILIES: &[DiscoverableToolPromptFamily] = &[
    APPROVAL_WORKFLOWS,
    LOCAL_FILES,
    SHELL_RUNTIME,
    WEB_RESEARCH,
    BROWSER_AUTOMATION,
    SESSION_STATE,
    DELEGATION,
    PROVIDER_ROUTING,
    EXTERNAL_CAPABILITIES,
    MEMORY_RECALL,
    CHANNEL_OPERATIONS,
];

pub(crate) fn discoverable_tool_prompt_family_for_name(
    tool_name: &str,
) -> Option<&'static DiscoverableToolPromptFamily> {
    let family = if matches!(
        tool_name,
        "approval_requests_list" | "approval_request_status" | "approval_request_resolve"
    ) {
        &APPROVAL_WORKFLOWS
    } else if matches!(
        tool_name,
        "content.search"
            | "config.import"
            | "file.read"
            | "file.write"
            | "file.edit"
            | "glob.search"
    ) {
        &LOCAL_FILES
    } else if matches!(tool_name, "shell.exec" | "bash.exec") {
        &SHELL_RUNTIME
    } else if matches!(tool_name, "web.fetch" | "web.search" | "http.request") {
        &WEB_RESEARCH
    } else if tool_name.starts_with("browser.") {
        &BROWSER_AUTOMATION
    } else if tool_name == "delegate" || tool_name == "delegate_async" {
        &DELEGATION
    } else if tool_name == "provider.switch" {
        &PROVIDER_ROUTING
    } else if tool_name.starts_with("external_skills.") {
        &EXTERNAL_CAPABILITIES
    } else if tool_name.starts_with("memory_") {
        &MEMORY_RECALL
    } else if tool_name.starts_with("feishu.") {
        &CHANNEL_OPERATIONS
    } else if tool_name.starts_with("session_") || tool_name.starts_with("sessions_") {
        &SESSION_STATE
    } else {
        return None;
    };

    Some(family)
}

pub(crate) fn active_discoverable_capability_family_states<'a>(
    tool_names: impl IntoIterator<Item = &'a str>,
) -> Vec<DiscoverableCapabilityFamilyState> {
    let mut tool_ids_by_family = BTreeMap::<&'static str, BTreeSet<String>>::new();

    for tool_name in tool_names {
        let Some(family) = discoverable_tool_prompt_family_for_name(tool_name) else {
            continue;
        };
        tool_ids_by_family
            .entry(family.id)
            .or_default()
            .insert(tool_name.to_owned());
    }

    ALL_DISCOVERABLE_TOOL_PROMPT_FAMILIES
        .iter()
        .filter_map(|family| {
            let tool_ids = tool_ids_by_family.remove(family.id)?;
            Some(family.into_state(tool_ids.into_iter().collect()))
        })
        .collect()
}

pub(crate) fn render_active_discoverable_tool_family_lines<'a>(
    tool_names: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let states = active_discoverable_capability_family_states(tool_names);
    if states.is_empty() {
        return Vec::new();
    }

    let mut lines = vec!["Discoverable capability families currently active:".to_owned()];
    lines.extend(
        states
            .iter()
            .map(DiscoverableCapabilityFamilyState::render_prompt_line),
    );
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_capability_family_states_group_tools_deterministically() {
        let states = active_discoverable_capability_family_states([
            "shell.exec",
            "file.write",
            "file.read",
            "delegate",
            "delegate_async",
        ]);

        assert_eq!(states.len(), 3);
        assert_eq!(states[0].family_id, "local_files");
        assert_eq!(states[0].tool_ids, vec!["file.read", "file.write"]);
        assert_eq!(states[1].family_id, "shell_runtime");
        assert_eq!(states[1].tool_ids, vec!["shell.exec"]);
        assert_eq!(states[2].family_id, "delegation");
        assert_eq!(states[2].tool_ids, vec!["delegate", "delegate_async"]);
    }
}
