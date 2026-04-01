use std::collections::BTreeSet;

use serde_json::Value;
use serde_json::json;

use super::context_engine::AssembledConversationContext;
use super::context_engine::ContextArtifactDescriptor;
use super::context_engine::ContextArtifactKind;
use super::context_engine::ToolOutputStreamingPolicy;
use super::prompt_fragments::PromptFragment;
use super::prompt_fragments::PromptLane;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptCompilation {
    pub fragments: Vec<PromptFragment>,
    pub system_text: String,
    pub artifacts: Vec<ContextArtifactDescriptor>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PromptCompiler;

impl PromptCompiler {
    pub fn compile(self, fragments: Vec<PromptFragment>) -> PromptCompilation {
        let normalized_fragments = normalize_fragments(fragments);
        let ordered_fragments = order_fragments(normalized_fragments);
        let system_text = render_system_text(&ordered_fragments);
        let artifacts = build_artifacts(&ordered_fragments);

        PromptCompilation {
            fragments: ordered_fragments,
            system_text,
            artifacts,
        }
    }
}

fn normalize_fragments(fragments: Vec<PromptFragment>) -> Vec<PromptFragment> {
    let mut normalized_fragments = Vec::new();
    let mut seen_dedupe_keys = BTreeSet::new();

    for mut fragment in fragments {
        let trimmed_content = fragment.content.trim().to_owned();

        if trimmed_content.is_empty() {
            continue;
        }

        fragment.content = trimmed_content;

        let dedupe_key = fragment.dedupe_key.clone();

        if let Some(dedupe_key) = dedupe_key {
            let inserted = seen_dedupe_keys.insert(dedupe_key);

            if !inserted {
                continue;
            }
        }

        normalized_fragments.push(fragment);
    }

    normalized_fragments
}

fn order_fragments(fragments: Vec<PromptFragment>) -> Vec<PromptFragment> {
    let mut ordered_fragments = Vec::new();

    for lane in PromptLane::ordered() {
        for fragment in &fragments {
            let fragment_lane = fragment.lane;

            if fragment_lane != *lane {
                continue;
            }

            ordered_fragments.push(fragment.clone());
        }
    }

    ordered_fragments
}

fn render_system_text(fragments: &[PromptFragment]) -> String {
    let mut sections = Vec::new();

    for fragment in fragments {
        let section = fragment.content.clone();

        sections.push(section);
    }

    sections.join("\n\n")
}

fn build_artifacts(fragments: &[PromptFragment]) -> Vec<ContextArtifactDescriptor> {
    let mut artifacts = Vec::new();

    if fragments.is_empty() {
        return artifacts;
    }

    let system_prompt_artifact = ContextArtifactDescriptor {
        message_index: 0,
        artifact_kind: ContextArtifactKind::SystemPrompt,
        maskable: false,
        streaming_policy: ToolOutputStreamingPolicy::BufferFull,
    };

    artifacts.push(system_prompt_artifact);

    for fragment in fragments {
        if fragment.artifact_kind == ContextArtifactKind::SystemPrompt {
            continue;
        }

        let artifact = ContextArtifactDescriptor {
            message_index: 0,
            artifact_kind: fragment.artifact_kind,
            maskable: fragment.maskable,
            streaming_policy: ToolOutputStreamingPolicy::BufferFull,
        };

        artifacts.push(artifact);
    }

    artifacts
}

pub(crate) fn seed_prompt_fragments_from_context(assembled: &mut AssembledConversationContext) {
    let has_prompt_fragments = !assembled.prompt_fragments.is_empty();

    if has_prompt_fragments {
        return;
    }

    let system_index = system_prompt_message_index(&assembled.messages, &assembled.artifacts);
    let Some(system_index) = system_index else {
        return;
    };

    let system_message = assembled.messages.get(system_index);
    let system_content = system_message
        .and_then(Value::as_object)
        .and_then(|object| object.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty());
    let Some(system_content) = system_content else {
        return;
    };

    let snapshot_start = system_content.find("[available_tools]");

    if let Some(snapshot_start) = snapshot_start {
        let base_content = system_content[..snapshot_start].trim();
        let capability_content = system_content[snapshot_start..].trim();

        if !base_content.is_empty() {
            let base_fragment = PromptFragment::new(
                "legacy-system-prompt",
                PromptLane::BaseSystem,
                "legacy-system-prompt",
                base_content,
                ContextArtifactKind::SystemPrompt,
            )
            .with_dedupe_key("legacy-system-prompt")
            .with_cacheable(true);

            assembled.prompt_fragments.push(base_fragment);
        }

        if capability_content.is_empty() {
            return;
        }

        let capability_fragment = PromptFragment::new(
            "legacy-capability-snapshot",
            PromptLane::CapabilitySnapshot,
            "capability-snapshot",
            capability_content,
            ContextArtifactKind::RuntimeContract,
        )
        .with_dedupe_key("capability-snapshot")
        .with_cacheable(true);

        assembled.prompt_fragments.push(capability_fragment);
        return;
    }

    let base_fragment = PromptFragment::new(
        "legacy-system-prompt",
        PromptLane::BaseSystem,
        "legacy-system-prompt",
        system_content,
        ContextArtifactKind::SystemPrompt,
    )
    .with_dedupe_key("legacy-system-prompt")
    .with_cacheable(true);

    assembled.prompt_fragments.push(base_fragment);
}

pub(crate) fn sync_prompt_fragments_into_context(assembled: &mut AssembledConversationContext) {
    if assembled.prompt_fragments.is_empty() {
        return;
    }

    let compiler = PromptCompiler;
    let compilation = compiler.compile(assembled.prompt_fragments.clone());
    let system_text = compilation.system_text.clone();

    if system_text.is_empty() {
        return;
    }

    let system_index = replace_or_insert_system_message(&mut assembled.messages, system_text);

    assembled.prompt_fragments = compilation.fragments;
    replace_system_prompt_artifacts(assembled, system_index, compilation.artifacts);
}

fn system_prompt_message_index(
    messages: &[Value],
    artifacts: &[ContextArtifactDescriptor],
) -> Option<usize> {
    let artifact_index = artifacts
        .iter()
        .find(|artifact| artifact.artifact_kind == ContextArtifactKind::SystemPrompt)
        .map(|artifact| artifact.message_index);

    if artifact_index.is_some() {
        return artifact_index;
    }

    messages
        .iter()
        .position(|message| message.get("role").and_then(Value::as_str) == Some("system"))
}

fn replace_or_insert_system_message(messages: &mut Vec<Value>, system_text: String) -> usize {
    let system_index = messages
        .iter()
        .position(|message| message.get("role").and_then(Value::as_str) == Some("system"));

    if let Some(system_index) = system_index
        && let Some(system_message) = messages.get_mut(system_index)
    {
        *system_message = json!({
            "role": "system",
            "content": system_text,
        });

        return system_index;
    }

    messages.insert(
        0,
        json!({
            "role": "system",
            "content": system_text,
        }),
    );

    0
}

fn replace_system_prompt_artifacts(
    assembled: &mut AssembledConversationContext,
    system_index: usize,
    compiled_artifacts: Vec<ContextArtifactDescriptor>,
) {
    let inserted_new_system_message = system_index == 0
        && assembled
            .artifacts
            .iter()
            .all(|artifact| artifact.message_index != 0);

    if inserted_new_system_message {
        for artifact in &mut assembled.artifacts {
            artifact.message_index += 1;
        }
    }

    let mut retained_artifacts = Vec::new();

    for artifact in assembled.artifacts.drain(..) {
        if artifact.message_index == system_index {
            continue;
        }

        retained_artifacts.push(artifact);
    }

    for mut artifact in compiled_artifacts {
        artifact.message_index = system_index;
        retained_artifacts.push(artifact);
    }

    assembled.artifacts = retained_artifacts;
}
