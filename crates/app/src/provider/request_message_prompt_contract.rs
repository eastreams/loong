use crate::config::LoongConfig;
use crate::conversation::{ContextArtifactKind, PromptFragment, PromptLane};

use super::super::native_tool_surface::ProviderNativePromptSection;
use super::ProviderRuntimeBinding;

pub(super) fn build_prompt_fragments_from_prompt_sources(
    config: &LoongConfig,
    system_text: String,
    workspace_guidance_section: Option<String>,
    runtime_self_section: Option<String>,
    runtime_identity_section: Option<String>,
    extra_section: Option<String>,
    capability_snapshot: String,
    native_tool_sections: Vec<ProviderNativePromptSection>,
) -> Vec<PromptFragment> {
    let deferred_tool_text_workflow = render_deferred_tool_text_workflow_section_if_needed(config);
    let execution_discipline_section = render_execution_discipline_section();
    let mut prompt_fragments = Vec::new();

    if !system_text.is_empty() {
        let base_fragment = PromptFragment::new(
            "base-system",
            PromptLane::BaseSystem,
            "base-system",
            system_text,
            ContextArtifactKind::SystemPrompt,
        )
        .with_dedupe_key("base-system")
        .with_cacheable(true);

        prompt_fragments.push(base_fragment);
    }

    if let Some(section) = workspace_guidance_section {
        let workspace_guidance_fragment = PromptFragment::new(
            "workspace-guidance",
            PromptLane::WorkspaceGuidance,
            "workspace-guidance",
            section,
            ContextArtifactKind::RuntimeContract,
        )
        .with_cacheable(true);

        prompt_fragments.push(workspace_guidance_fragment);
    }

    if let Some(section) = runtime_self_section {
        let runtime_self_fragment = PromptFragment::new(
            "runtime-self",
            PromptLane::RuntimeSelf,
            "runtime-self",
            section,
            ContextArtifactKind::RuntimeContract,
        )
        .with_cacheable(true);

        prompt_fragments.push(runtime_self_fragment);
    }

    if let Some(section) = runtime_identity_section {
        let runtime_identity_fragment = PromptFragment::new(
            "runtime-identity",
            PromptLane::RuntimeIdentity,
            "runtime-identity",
            section,
            ContextArtifactKind::Profile,
        )
        .with_cacheable(true);

        prompt_fragments.push(runtime_identity_fragment);
    }

    let execution_discipline_fragment = PromptFragment::new(
        "execution-discipline",
        PromptLane::ExecutionDiscipline,
        "execution-discipline",
        execution_discipline_section,
        ContextArtifactKind::RuntimeContract,
    )
    .with_cacheable(true);

    prompt_fragments.push(execution_discipline_fragment);

    if let Some(section) = extra_section {
        let binding_fragment = PromptFragment::new(
            "governed-runtime-binding",
            PromptLane::CapabilitySnapshot,
            "governed-runtime-binding",
            section,
            ContextArtifactKind::RuntimeContract,
        )
        .with_cacheable(true);

        prompt_fragments.push(binding_fragment);
    }

    let capability_fragment = PromptFragment::new(
        "capability-snapshot",
        PromptLane::CapabilitySnapshot,
        "capability-snapshot",
        capability_snapshot,
        ContextArtifactKind::RuntimeContract,
    )
    .with_cacheable(true);

    prompt_fragments.push(capability_fragment);

    for section in native_tool_sections {
        let native_fragment = PromptFragment::new(
            section.id,
            PromptLane::CapabilitySnapshot,
            section.id,
            section.content,
            ContextArtifactKind::RuntimeContract,
        )
        .with_cacheable(true);

        prompt_fragments.push(native_fragment);
    }

    if let Some(section) = deferred_tool_text_workflow {
        let deferred_tool_text_fragment = PromptFragment::new(
            "deferred-tool-text-workflow",
            PromptLane::CapabilitySnapshot,
            "deferred-tool-text-workflow",
            section,
            ContextArtifactKind::RuntimeContract,
        )
        .with_cacheable(true);

        prompt_fragments.push(deferred_tool_text_fragment);
    }

    prompt_fragments
}

pub(super) fn render_execution_discipline_section() -> String {
    let lines = [
        "## Execution Discipline".to_owned(),
        "<yolo_by_default>".to_owned(),
        "- Default to the best bounded action already allowed by the current runtime authority."
            .to_owned(),
        "- Do not ask for confirmation for ordinary allowed work.".to_owned(),
        "- Continue from tool results and retrieved evidence until no useful bounded action remains."
            .to_owned(),
        "- Only stop for a verified completion condition, a concrete blocker, or a real approval boundary."
            .to_owned(),
        "</yolo_by_default>".to_owned(),
        "<tool_persistence>".to_owned(),
        "- Use tools whenever they materially improve correctness, completeness, or grounding."
            .to_owned(),
        "- Do not stop early when another bounded tool call would likely close an evidence gap."
            .to_owned(),
        "- If one retrieval path returns partial or empty results, retry with a different bounded strategy before asking the user."
            .to_owned(),
        "- Prefer finishing the work over narrating each intermediate step.".to_owned(),
        "</tool_persistence>".to_owned(),
        "<mandatory_tool_use>".to_owned(),
        "- Do not answer live system, file, git, or current-fact questions from memory when runtime retrieval is available."
            .to_owned(),
        "- Prefer runtime evidence over recalled assumptions about the current environment."
            .to_owned(),
        "</mandatory_tool_use>".to_owned(),
        "<act_dont_ask>".to_owned(),
        "- If ambiguity does not change the next tool or side effect, act on the obvious local interpretation."
            .to_owned(),
        "- Ask only when the missing detail changes the tool, target, or side effect."
            .to_owned(),
        "- Do not emit incremental progress chatter after each tool result.".to_owned(),
        "</act_dont_ask>".to_owned(),
        "<prerequisite_checks>".to_owned(),
        "- Before a mutating step or high-confidence claim, check whether discovery, inspection, or preflight is still needed."
            .to_owned(),
        "- Treat prerequisite discovery as part of the task.".to_owned(),
        "</prerequisite_checks>".to_owned(),
        "<verification>".to_owned(),
        "- Before finalizing, check correctness, grounding, output shape, and stop state."
            .to_owned(),
        "- A reply alone is not proof that a long-running task is complete.".to_owned(),
        "- Queued async work, waiting task handles, blocked task states, and pending approvals are intermediate states, not final completion."
            .to_owned(),
        "</verification>".to_owned(),
        "<missing_context>".to_owned(),
        "- If required information is retrievable, retrieve it instead of asking.".to_owned(),
        "- Ask only when the missing information is not locally or remotely retrievable."
            .to_owned(),
        "- If you must proceed under uncertainty, label assumptions explicitly.".to_owned(),
        "</missing_context>".to_owned(),
    ];

    lines.join("\n")
}

fn render_deferred_tool_text_workflow_section_if_needed(config: &LoongConfig) -> Option<String> {
    let tool_schema_mode = config.provider.resolved_tool_schema_mode_config();
    let tool_schema_disabled =
        tool_schema_mode == crate::config::ProviderToolSchemaModeConfig::Disabled;
    if !tool_schema_disabled {
        return None;
    }

    Some(render_deferred_tool_text_workflow_section())
}

fn render_deferred_tool_text_workflow_section() -> String {
    let direct_call_example_lines = [
        "{",
        "  \"name\": \"read\",",
        "  \"arguments\": {",
        "    \"path\": \"README.md\"",
        "  }",
        "}",
    ];
    let direct_call_example = direct_call_example_lines.join("\n");

    let discovery_call_example_lines = [
        "{",
        "  \"name\": \"tool_search\",",
        "  \"arguments\": {",
        "    \"query\": \"approval session status\",",
        "    \"limit\": 5",
        "  }",
        "}",
    ];
    let discovery_call_example = discovery_call_example_lines.join("\n");

    let invoke_call_example_lines = [
        "{",
        "  \"name\": \"tool_invoke\",",
        "  \"arguments\": {",
        "    \"tool_id\": \"agent\",",
        "    \"lease\": \"<lease from tool_search>\",",
        "    \"arguments\": {",
        "      \"operation\": \"session-status\",",
        "      \"session_id\": \"<session id>\"",
        "    }",
        "  }",
        "}",
    ];
    let invoke_call_example = invoke_call_example_lines.join("\n");

    let lines = [
        "## Tool Access".to_owned(),
        "Structured provider tool schemas are disabled for this profile.".to_owned(),
        "Use the smallest tool that fits: `read`, `write`, `exec`, `web`, `browser`, or `memory`. These direct tools are the normal path.".to_owned(),
        "For `web`, distinguish search-provider mode from ordinary network mode: `web { query }` uses web-search providers, while `web { url }` or low-level request fields are still normal network access.".to_owned(),
        "Use `tool_search` only when the task needs a hidden surface such as `agent`, `skills`, or `channel`, and keep the query short and capability-focused.".to_owned(),
        "Use `tool_invoke` only with a fresh lease returned by `tool_search`; do not route normal direct-tool work through leases.".to_owned(),
        "Grouped hidden surfaces such as `agent`, `skills`, and `channel` are not direct tool calls. If `tool_search` returns one of them, pass it back through `tool_invoke` with the returned lease instead of emitting that grouped name directly.".to_owned(),
        "When you need a tool, emit the raw JSON call instead of only describing the missing capability.".to_owned(),
        "Direct tool example:".to_owned(),
        direct_call_example,
        "In raw JSON tool calls, use the provider tool names `tool_search` and `tool_invoke`.".to_owned(),
        "tool_invoke leases are short-lived; after any invalid_tool_lease response, refresh with tool_search before retrying.".to_owned(),
        "If you already know the tool id, refresh directly with exact_tool_id to fetch a fresh lease card.".to_owned(),
        "Hidden-tool discovery example:".to_owned(),
        discovery_call_example,
        "Hidden-tool invocation example:".to_owned(),
        invoke_call_example,
    ];

    lines.join("\n")
}

pub(super) fn render_governed_runtime_binding_section(
    binding: ProviderRuntimeBinding<'_>,
) -> String {
    let kernel_binding = if binding.is_kernel_bound() {
        "present"
    } else {
        "absent"
    };
    format!(
        "## Governed Runtime Binding\n- session_mode: {}\n- kernel_binding: {kernel_binding}",
        binding.session_mode().as_str()
    )
}
