use std::collections::BTreeMap;

use hex::encode as hex_encode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::compaction::is_compacted_summary_content;
use super::context_engine::AssembledConversationContext;
use super::context_engine::ContextArtifactDescriptor;
use super::context_engine::ContextArtifactKind;
use super::prompt_fragments::PromptFragment;
use super::prompt_fragments::PromptFrameAuthority;
use super::prompt_fragments::PromptFrameLayer;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrameLayerStats {
    pub fragment_count: u32,
    pub message_count: u32,
    pub total_chars: u32,
    pub estimated_tokens: u32,
    pub cacheable_fragment_count: u32,
}

impl PromptFrameLayerStats {
    fn record_fragment(&mut self, content: &str, cacheable: bool) {
        let char_count = count_chars_as_u32(content);
        let estimated_tokens = estimate_tokens_from_chars(char_count);

        self.fragment_count = self.fragment_count.saturating_add(1);
        self.total_chars = self.total_chars.saturating_add(char_count);
        self.estimated_tokens = self.estimated_tokens.saturating_add(estimated_tokens);

        if cacheable {
            self.cacheable_fragment_count = self.cacheable_fragment_count.saturating_add(1);
        }
    }

    fn record_message(&mut self, content: &str) {
        let char_count = count_chars_as_u32(content);
        let estimated_tokens = estimate_tokens_from_chars(char_count);

        self.message_count = self.message_count.saturating_add(1);
        self.total_chars = self.total_chars.saturating_add(char_count);
        self.estimated_tokens = self.estimated_tokens.saturating_add(estimated_tokens);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrameFragmentSummary {
    pub fragment_id: String,
    pub source_id: String,
    pub lane: super::prompt_fragments::PromptLane,
    pub frame_layer: PromptFrameLayer,
    pub frame_authority: PromptFrameAuthority,
    pub cacheable: bool,
    pub content_chars: usize,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrameMessageSummary {
    pub message_index: usize,
    pub role: Option<String>,
    pub frame_layer: PromptFrameLayer,
    pub frame_authority: PromptFrameAuthority,
    pub content_chars: usize,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrameBucketSummary {
    pub frame_layer: PromptFrameLayer,
    pub fragment_count: usize,
    pub message_count: usize,
    pub content_chars: usize,
    pub content_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptFrameSummary {
    pub fragments: Vec<PromptFrameFragmentSummary>,
    pub messages: Vec<PromptFrameMessageSummary>,
    pub buckets: Vec<PromptFrameBucketSummary>,
    pub stable_runtime_guidance: PromptFrameLayerStats,
    pub session_latched_context: PromptFrameLayerStats,
    pub advisory_profile: PromptFrameLayerStats,
    pub session_local_recall: PromptFrameLayerStats,
    pub recent_window: PromptFrameLayerStats,
    pub turn_ephemeral_tail: PromptFrameLayerStats,
    pub total_estimated_tokens: Option<usize>,
    pub stable_runtime_segment_count: usize,
    pub stable_runtime_estimated_tokens: usize,
    pub session_latched_segment_count: usize,
    pub session_latched_estimated_tokens: usize,
    pub advisory_profile_segment_count: usize,
    pub advisory_profile_estimated_tokens: usize,
    pub session_local_recall_segment_count: usize,
    pub session_local_recall_estimated_tokens: usize,
    pub recent_window_segment_count: usize,
    pub recent_window_estimated_tokens: usize,
    pub turn_ephemeral_segment_count: usize,
    pub turn_ephemeral_estimated_tokens: usize,
    pub stable_runtime_hash: Option<String>,
    pub session_latched_hash: Option<String>,
    pub advisory_profile_hash: Option<String>,
    pub session_local_recall_hash: Option<String>,
    pub recent_window_hash: Option<String>,
    pub turn_ephemeral_hash: Option<String>,
    pub stable_runtime_guidance_hash_sha256: Option<String>,
    pub session_latched_context_hash_sha256: Option<String>,
    pub stable_prefix_hash_sha256: Option<String>,
    pub cached_prefix_sha256: Option<String>,
    pub advisory_profile_hash_sha256: Option<String>,
    pub local_recall_hash_sha256: Option<String>,
    pub recent_window_hash_sha256: Option<String>,
    pub turn_ephemeral_hash_sha256: Option<String>,
}

impl PromptFrameSummary {
    fn refresh_derived_fields(&mut self) {
        self.stable_runtime_segment_count = layer_segment_count(&self.stable_runtime_guidance);
        self.stable_runtime_estimated_tokens =
            usize::try_from(self.stable_runtime_guidance.estimated_tokens).unwrap_or(usize::MAX);
        self.session_latched_segment_count = layer_segment_count(&self.session_latched_context);
        self.session_latched_estimated_tokens =
            usize::try_from(self.session_latched_context.estimated_tokens).unwrap_or(usize::MAX);
        self.advisory_profile_segment_count = layer_segment_count(&self.advisory_profile);
        self.advisory_profile_estimated_tokens =
            usize::try_from(self.advisory_profile.estimated_tokens).unwrap_or(usize::MAX);
        self.session_local_recall_segment_count = layer_segment_count(&self.session_local_recall);
        self.session_local_recall_estimated_tokens =
            usize::try_from(self.session_local_recall.estimated_tokens).unwrap_or(usize::MAX);
        self.recent_window_segment_count = layer_segment_count(&self.recent_window);
        self.recent_window_estimated_tokens =
            usize::try_from(self.recent_window.estimated_tokens).unwrap_or(usize::MAX);
        self.turn_ephemeral_segment_count = layer_segment_count(&self.turn_ephemeral_tail);
        self.turn_ephemeral_estimated_tokens =
            usize::try_from(self.turn_ephemeral_tail.estimated_tokens).unwrap_or(usize::MAX);
        self.stable_runtime_hash = self.stable_runtime_guidance_hash_sha256.clone();
        self.session_latched_hash = self.session_latched_context_hash_sha256.clone();
        self.advisory_profile_hash = self.advisory_profile_hash_sha256.clone();
        self.session_local_recall_hash = self.local_recall_hash_sha256.clone();
        self.recent_window_hash = self.recent_window_hash_sha256.clone();
        self.turn_ephemeral_hash = self.turn_ephemeral_hash_sha256.clone();
    }

    fn estimated_total_from_layers(&self) -> usize {
        self.stable_runtime_estimated_tokens
            .saturating_add(self.session_latched_estimated_tokens)
            .saturating_add(
                usize::try_from(self.advisory_profile.estimated_tokens).unwrap_or(usize::MAX),
            )
            .saturating_add(self.session_local_recall_estimated_tokens)
            .saturating_add(self.recent_window_estimated_tokens)
            .saturating_add(self.turn_ephemeral_estimated_tokens)
    }

    pub fn from_fragments(fragments: &[PromptFragment]) -> Self {
        let empty_messages: &[Value] = &[];
        let empty_artifacts: &[ContextArtifactDescriptor] = &[];
        let ephemeral_tail_start = None;

        Self::from_context_with_ephemeral_tail_start(
            empty_messages,
            empty_artifacts,
            fragments,
            ephemeral_tail_start,
            None,
        )
    }

    pub fn from_context(
        messages: &[Value],
        artifacts: &[ContextArtifactDescriptor],
        fragments: &[PromptFragment],
    ) -> Self {
        let ephemeral_tail_start = None;

        Self::from_context_with_ephemeral_tail_start(
            messages,
            artifacts,
            fragments,
            ephemeral_tail_start,
            None,
        )
    }

    pub fn from_context_with_ephemeral_tail_start(
        messages: &[Value],
        artifacts: &[ContextArtifactDescriptor],
        fragments: &[PromptFragment],
        ephemeral_tail_start: Option<usize>,
        estimated_tokens: Option<usize>,
    ) -> Self {
        summarize_prompt_frame_components(
            messages,
            artifacts,
            fragments,
            ephemeral_tail_start,
            estimated_tokens,
        )
    }

    pub fn bucket(&self, frame_layer: PromptFrameLayer) -> Option<&PromptFrameBucketSummary> {
        self.buckets
            .iter()
            .find(|bucket| bucket.frame_layer == frame_layer)
    }

    pub fn to_event_payload(&self) -> Value {
        json!({
            "schema_version": 1,
            "total_estimated_tokens": self.total_estimated_tokens,
            "stable_runtime_segment_count": self.stable_runtime_segment_count,
            "stable_runtime_estimated_tokens": self.stable_runtime_estimated_tokens,
            "session_latched_segment_count": self.session_latched_segment_count,
            "session_latched_estimated_tokens": self.session_latched_estimated_tokens,
            "advisory_profile_segment_count": self.advisory_profile_segment_count,
            "advisory_profile_estimated_tokens": self.advisory_profile_estimated_tokens,
            "session_local_recall_segment_count": self.session_local_recall_segment_count,
            "session_local_recall_estimated_tokens": self.session_local_recall_estimated_tokens,
            "recent_window_segment_count": self.recent_window_segment_count,
            "recent_window_estimated_tokens": self.recent_window_estimated_tokens,
            "turn_ephemeral_segment_count": self.turn_ephemeral_segment_count,
            "turn_ephemeral_estimated_tokens": self.turn_ephemeral_estimated_tokens,
            "stable_runtime_hash": self.stable_runtime_hash,
            "session_latched_hash": self.session_latched_hash,
            "advisory_profile_hash": self.advisory_profile_hash,
            "session_local_recall_hash": self.session_local_recall_hash,
            "recent_window_hash": self.recent_window_hash,
            "turn_ephemeral_hash": self.turn_ephemeral_hash,
            "stable_runtime_guidance_hash_sha256": self.stable_runtime_guidance_hash_sha256,
            "session_latched_context_hash_sha256": self.session_latched_context_hash_sha256,
            "stable_prefix_hash_sha256": self.stable_prefix_hash_sha256,
            "cached_prefix_sha256": self.cached_prefix_sha256,
            "advisory_profile_hash_sha256": self.advisory_profile_hash_sha256,
            "local_recall_hash_sha256": self.local_recall_hash_sha256,
            "recent_window_hash_sha256": self.recent_window_hash_sha256,
            "turn_ephemeral_hash_sha256": self.turn_ephemeral_hash_sha256,
            "layers": {
                "stable_runtime_guidance": self.stable_runtime_guidance,
                "session_latched_context": self.session_latched_context,
                "advisory_profile": self.advisory_profile,
                "session_local_recall": self.session_local_recall,
                "recent_window": self.recent_window,
                "turn_ephemeral_tail": self.turn_ephemeral_tail,
            }
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrame {
    pub summary: PromptFrameSummary,
}

impl PromptFrame {
    pub fn from_context_parts(
        prompt_fragments: &[PromptFragment],
        messages: &[Value],
        artifacts: &[ContextArtifactDescriptor],
        estimated_tokens: Option<usize>,
        turn_ephemeral_start_index: Option<usize>,
    ) -> Self {
        let summary = PromptFrameSummary::from_context_with_ephemeral_tail_start(
            messages,
            artifacts,
            prompt_fragments,
            turn_ephemeral_start_index,
            estimated_tokens,
        );

        Self { summary }
    }

    pub fn with_turn_ephemeral_messages(
        &self,
        messages: &[Value],
        estimated_tokens: Option<usize>,
    ) -> Self {
        let tail_summary = summarize_followup_prompt_frame(messages);
        let mut summary = self.summary.clone();

        summary.turn_ephemeral_tail = tail_summary.turn_ephemeral_tail;
        summary.turn_ephemeral_segment_count = tail_summary.turn_ephemeral_segment_count;
        summary.turn_ephemeral_estimated_tokens = tail_summary.turn_ephemeral_estimated_tokens;
        summary.turn_ephemeral_hash = tail_summary.turn_ephemeral_hash;
        summary.turn_ephemeral_hash_sha256 = tail_summary.turn_ephemeral_hash_sha256;
        let total_estimated_tokens =
            estimated_tokens.unwrap_or_else(|| summary.estimated_total_from_layers());

        summary.total_estimated_tokens = Some(total_estimated_tokens);

        Self { summary }
    }
}

pub fn summarize_assembled_prompt_frame(
    assembled: &AssembledConversationContext,
) -> PromptFrameSummary {
    let prompt_fragments = assembled.prompt_fragments.as_slice();
    let messages = assembled.messages.as_slice();
    let artifacts = assembled.artifacts.as_slice();
    let estimated_tokens = assembled.estimated_tokens;

    PromptFrameSummary::from_context_with_ephemeral_tail_start(
        messages,
        artifacts,
        prompt_fragments,
        None,
        estimated_tokens,
    )
}

pub fn summarize_followup_prompt_frame(messages: &[Value]) -> PromptFrameSummary {
    let prompt_fragments = &[];
    let artifacts = &[];
    let ephemeral_tail_start = (!messages.is_empty()).then_some(0);
    let estimated_tokens = None;

    PromptFrameSummary::from_context_with_ephemeral_tail_start(
        messages,
        artifacts,
        prompt_fragments,
        ephemeral_tail_start,
        estimated_tokens,
    )
}

fn summarize_prompt_frame_components(
    messages: &[Value],
    artifacts: &[ContextArtifactDescriptor],
    prompt_fragments: &[PromptFragment],
    ephemeral_tail_start: Option<usize>,
    estimated_tokens: Option<usize>,
) -> PromptFrameSummary {
    let mut bucket_builders = build_bucket_builders();
    let mut summary_builder = PromptFrameSummaryBuilder::default();
    let fragment_summaries =
        summarize_fragments(prompt_fragments, &mut bucket_builders, &mut summary_builder);
    let message_summaries = summarize_messages(
        messages,
        artifacts,
        &mut bucket_builders,
        &mut summary_builder,
        ephemeral_tail_start,
    );
    let buckets = finalize_bucket_summaries(bucket_builders);
    let mut summary = summary_builder.finish();

    summary.fragments = fragment_summaries;
    summary.messages = message_summaries;
    summary.buckets = buckets;
    summary.refresh_derived_fields();
    let total_estimated_tokens =
        estimated_tokens.unwrap_or_else(|| summary.estimated_total_from_layers());
    summary.total_estimated_tokens = Some(total_estimated_tokens);
    summary
}

#[derive(Debug, Default)]
struct PromptFrameBucketBuilder {
    fragment_count: usize,
    message_count: usize,
    content_chars: usize,
    serialized_parts: Vec<String>,
}

impl PromptFrameBucketBuilder {
    fn push_fragment(&mut self, content: &str) {
        let content_chars = content.chars().count();
        let serialized_content = content.to_owned();

        self.fragment_count += 1;
        self.content_chars += content_chars;
        self.serialized_parts.push(serialized_content);
    }

    fn push_message(&mut self, serialized_message: String) {
        let content_chars = serialized_message.chars().count();

        self.message_count += 1;
        self.content_chars += content_chars;
        self.serialized_parts.push(serialized_message);
    }

    fn content_sha256(&self) -> Option<String> {
        hash_serialized_parts(self.serialized_parts.as_slice())
    }
}

#[derive(Debug, Default)]
struct PromptFrameSummaryBuilder {
    summary: PromptFrameSummary,
    stable_runtime_guidance_blocks: Vec<String>,
    session_latched_context_blocks: Vec<String>,
    stable_prefix_blocks: Vec<String>,
    cached_prefix_blocks: Vec<String>,
    advisory_profile_blocks: Vec<String>,
    local_recall_blocks: Vec<String>,
    recent_window_blocks: Vec<String>,
    turn_ephemeral_blocks: Vec<String>,
}

impl PromptFrameSummaryBuilder {
    fn finish(mut self) -> PromptFrameSummary {
        self.summary.stable_runtime_guidance_hash_sha256 =
            hash_blocks(self.stable_runtime_guidance_blocks.as_slice());
        self.summary.session_latched_context_hash_sha256 =
            hash_blocks(self.session_latched_context_blocks.as_slice());
        self.summary.stable_prefix_hash_sha256 = hash_blocks(self.stable_prefix_blocks.as_slice());
        self.summary.cached_prefix_sha256 = hash_blocks(self.cached_prefix_blocks.as_slice());
        self.summary.advisory_profile_hash_sha256 =
            hash_blocks(self.advisory_profile_blocks.as_slice());
        self.summary.local_recall_hash_sha256 = hash_blocks(self.local_recall_blocks.as_slice());
        self.summary.recent_window_hash_sha256 = hash_blocks(self.recent_window_blocks.as_slice());
        self.summary.turn_ephemeral_hash_sha256 =
            hash_blocks(self.turn_ephemeral_blocks.as_slice());
        self.summary.refresh_derived_fields();

        self.summary
    }

    fn record_fragment(&mut self, fragment: &PromptFragment) {
        let frame_layer = fragment.frame_layer;
        let content = fragment.content.as_str();
        let cacheable = fragment.cacheable;

        let layer_stats = self.layer_stats_mut(frame_layer);
        layer_stats.record_fragment(content, cacheable);

        let serialized_block = format!(
            "fragment:{}:{}:{}",
            fragment.fragment_id, fragment.source_id, content
        );

        self.record_hashed_block(frame_layer, cacheable, serialized_block);
    }

    fn record_message(&mut self, frame_layer: PromptFrameLayer, content: &str) {
        let layer_stats = self.layer_stats_mut(frame_layer);
        layer_stats.record_message(content);

        let serialized_block = format!("message:{frame_layer}:{content}");

        self.record_hashed_block(frame_layer, false, serialized_block);
    }

    fn layer_stats_mut(&mut self, frame_layer: PromptFrameLayer) -> &mut PromptFrameLayerStats {
        match frame_layer {
            PromptFrameLayer::StableRuntimeGuidance => &mut self.summary.stable_runtime_guidance,
            PromptFrameLayer::SessionLatchedContext => &mut self.summary.session_latched_context,
            PromptFrameLayer::AdvisoryProfile => &mut self.summary.advisory_profile,
            PromptFrameLayer::SessionLocalRecall => &mut self.summary.session_local_recall,
            PromptFrameLayer::RecentWindow => &mut self.summary.recent_window,
            PromptFrameLayer::TurnEphemeralTail => &mut self.summary.turn_ephemeral_tail,
        }
    }

    fn record_hashed_block(
        &mut self,
        frame_layer: PromptFrameLayer,
        cacheable: bool,
        serialized_block: String,
    ) {
        match frame_layer {
            PromptFrameLayer::StableRuntimeGuidance => {
                self.stable_runtime_guidance_blocks
                    .push(serialized_block.clone());
                self.stable_prefix_blocks.push(serialized_block.clone());
                if cacheable {
                    self.cached_prefix_blocks.push(serialized_block);
                }
            }
            PromptFrameLayer::SessionLatchedContext => {
                self.session_latched_context_blocks
                    .push(serialized_block.clone());
                self.stable_prefix_blocks.push(serialized_block.clone());
                if cacheable {
                    self.cached_prefix_blocks.push(serialized_block);
                }
            }
            PromptFrameLayer::AdvisoryProfile => {
                self.advisory_profile_blocks.push(serialized_block);
            }
            PromptFrameLayer::SessionLocalRecall => {
                self.local_recall_blocks.push(serialized_block);
            }
            PromptFrameLayer::RecentWindow => {
                self.recent_window_blocks.push(serialized_block);
            }
            PromptFrameLayer::TurnEphemeralTail => {
                self.turn_ephemeral_blocks.push(serialized_block);
            }
        }
    }
}

fn build_bucket_builders() -> BTreeMap<PromptFrameLayer, PromptFrameBucketBuilder> {
    let mut bucket_builders = BTreeMap::new();

    for frame_layer in prompt_frame_layers_in_order() {
        let bucket_builder = PromptFrameBucketBuilder::default();

        bucket_builders.insert(*frame_layer, bucket_builder);
    }

    bucket_builders
}

fn summarize_fragments(
    fragments: &[PromptFragment],
    bucket_builders: &mut BTreeMap<PromptFrameLayer, PromptFrameBucketBuilder>,
    summary_builder: &mut PromptFrameSummaryBuilder,
) -> Vec<PromptFrameFragmentSummary> {
    let mut fragment_summaries = Vec::new();

    for fragment in fragments {
        let frame_layer = fragment.frame_layer;
        let frame_authority = fragment.frame_authority;
        let content = fragment.content.as_str();
        let content_chars = content.chars().count();
        let content_sha256 = hash_text(content);

        if let Some(bucket_builder) = bucket_builders.get_mut(&frame_layer) {
            bucket_builder.push_fragment(content);
        }

        summary_builder.record_fragment(fragment);

        let fragment_summary = PromptFrameFragmentSummary {
            fragment_id: fragment.fragment_id.clone(),
            source_id: fragment.source_id.to_owned(),
            lane: fragment.lane,
            frame_layer,
            frame_authority,
            cacheable: fragment.cacheable,
            content_chars,
            content_sha256,
        };

        fragment_summaries.push(fragment_summary);
    }

    fragment_summaries
}

fn summarize_messages(
    messages: &[Value],
    artifacts: &[ContextArtifactDescriptor],
    bucket_builders: &mut BTreeMap<PromptFrameLayer, PromptFrameBucketBuilder>,
    summary_builder: &mut PromptFrameSummaryBuilder,
    ephemeral_tail_start: Option<usize>,
) -> Vec<PromptFrameMessageSummary> {
    let mut message_summaries = Vec::new();
    let system_prompt_message_index = find_system_prompt_message_index(artifacts);

    for (message_index, message) in messages.iter().enumerate() {
        let is_system_prompt_message = system_prompt_message_index == Some(message_index);

        if is_system_prompt_message {
            continue;
        }

        let frame_layer =
            classify_message_frame(message_index, message, artifacts, ephemeral_tail_start);
        let Some(frame_layer) = frame_layer else {
            continue;
        };
        let frame_authority =
            classify_message_authority(frame_layer, artifacts, message_index, message);
        let serialized_message = serialize_message_for_frame(message);
        let content_chars = serialized_message.chars().count();
        let content_sha256 = hash_text(serialized_message.as_str());

        if let Some(bucket_builder) = bucket_builders.get_mut(&frame_layer) {
            bucket_builder.push_message(serialized_message.clone());
        }

        summary_builder.record_message(frame_layer, serialized_message.as_str());

        let role = message
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let message_summary = PromptFrameMessageSummary {
            message_index,
            role,
            frame_layer,
            frame_authority,
            content_chars,
            content_sha256,
        };

        message_summaries.push(message_summary);
    }

    message_summaries
}

fn finalize_bucket_summaries(
    bucket_builders: BTreeMap<PromptFrameLayer, PromptFrameBucketBuilder>,
) -> Vec<PromptFrameBucketSummary> {
    let mut bucket_summaries = Vec::new();

    for frame_layer in prompt_frame_layers_in_order() {
        let bucket_builder = bucket_builders.get(frame_layer);
        let Some(bucket_builder) = bucket_builder else {
            continue;
        };
        let content_sha256 = bucket_builder.content_sha256();
        let bucket_summary = PromptFrameBucketSummary {
            frame_layer: *frame_layer,
            fragment_count: bucket_builder.fragment_count,
            message_count: bucket_builder.message_count,
            content_chars: bucket_builder.content_chars,
            content_sha256,
        };

        bucket_summaries.push(bucket_summary);
    }

    bucket_summaries
}

fn prompt_frame_layers_in_order() -> &'static [PromptFrameLayer] {
    &[
        PromptFrameLayer::StableRuntimeGuidance,
        PromptFrameLayer::SessionLatchedContext,
        PromptFrameLayer::AdvisoryProfile,
        PromptFrameLayer::SessionLocalRecall,
        PromptFrameLayer::RecentWindow,
        PromptFrameLayer::TurnEphemeralTail,
    ]
}

fn find_system_prompt_message_index(artifacts: &[ContextArtifactDescriptor]) -> Option<usize> {
    artifacts
        .iter()
        .find(|artifact| artifact.artifact_kind == ContextArtifactKind::SystemPrompt)
        .map(|artifact| artifact.message_index)
}

fn classify_message_frame(
    message_index: usize,
    message: &Value,
    artifacts: &[ContextArtifactDescriptor],
    ephemeral_tail_start: Option<usize>,
) -> Option<PromptFrameLayer> {
    if ephemeral_tail_start.is_some_and(|tail_start| message_index >= tail_start) {
        return Some(PromptFrameLayer::TurnEphemeralTail);
    }

    let artifact_kinds = artifacts
        .iter()
        .filter(|artifact| artifact.message_index == message_index)
        .map(|artifact| artifact.artifact_kind)
        .collect::<Vec<_>>();

    if artifact_kinds.contains(&ContextArtifactKind::Profile) {
        return Some(PromptFrameLayer::AdvisoryProfile);
    }

    if artifact_kinds.contains(&ContextArtifactKind::RetrievedMemory) {
        return Some(PromptFrameLayer::AdvisoryProfile);
    }

    if artifact_kinds.contains(&ContextArtifactKind::Summary) {
        return Some(PromptFrameLayer::SessionLocalRecall);
    }

    if artifact_kinds.contains(&ContextArtifactKind::ToolHint) {
        return Some(PromptFrameLayer::SessionLocalRecall);
    }

    if artifact_kinds.contains(&ContextArtifactKind::RuntimeContract) {
        return Some(PromptFrameLayer::SessionLatchedContext);
    }

    if artifact_kinds.contains(&ContextArtifactKind::ConversationTurn) {
        let message_content = message.get("content").and_then(Value::as_str);
        let compacted_summary = message_content.is_some_and(is_compacted_summary_content);

        if compacted_summary {
            return Some(PromptFrameLayer::SessionLocalRecall);
        }

        return Some(PromptFrameLayer::RecentWindow);
    }

    if artifact_kinds.contains(&ContextArtifactKind::ToolResult) {
        return Some(PromptFrameLayer::RecentWindow);
    }

    let message_content = message.get("content").and_then(Value::as_str);
    let compacted_summary = message_content.is_some_and(is_compacted_summary_content);

    if compacted_summary {
        return Some(PromptFrameLayer::SessionLocalRecall);
    }

    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match role {
        "system" => Some(PromptFrameLayer::SessionLatchedContext),
        "user" | "assistant" | "tool" => Some(PromptFrameLayer::RecentWindow),
        _ => Some(PromptFrameLayer::RecentWindow),
    }
}

fn classify_message_authority(
    frame_layer: PromptFrameLayer,
    artifacts: &[ContextArtifactDescriptor],
    message_index: usize,
    _message: &Value,
) -> PromptFrameAuthority {
    let artifact_kinds = artifacts
        .iter()
        .filter(|artifact| artifact.message_index == message_index)
        .map(|artifact| artifact.artifact_kind)
        .collect::<Vec<_>>();

    if artifact_kinds.contains(&ContextArtifactKind::Profile) {
        return PromptFrameAuthority::AdvisoryProfile;
    }

    if artifact_kinds.contains(&ContextArtifactKind::RetrievedMemory) {
        return PromptFrameAuthority::AdvisoryProfile;
    }

    if artifact_kinds.contains(&ContextArtifactKind::Summary) {
        return PromptFrameAuthority::SessionLocalRecall;
    }

    if artifact_kinds.contains(&ContextArtifactKind::ToolHint) {
        return PromptFrameAuthority::SessionLocalRecall;
    }

    if artifact_kinds.contains(&ContextArtifactKind::RuntimeContract) {
        return PromptFrameAuthority::CapabilityContract;
    }

    match frame_layer {
        PromptFrameLayer::StableRuntimeGuidance => PromptFrameAuthority::CoreSystem,
        PromptFrameLayer::SessionLatchedContext => PromptFrameAuthority::CapabilityContract,
        PromptFrameLayer::AdvisoryProfile => PromptFrameAuthority::AdvisoryProfile,
        PromptFrameLayer::SessionLocalRecall => PromptFrameAuthority::SessionLocalRecall,
        PromptFrameLayer::RecentWindow => PromptFrameAuthority::LiveTurn,
        PromptFrameLayer::TurnEphemeralTail => PromptFrameAuthority::LiveTurn,
    }
}

fn serialize_message_for_frame(message: &Value) -> String {
    let serialized_message = serde_json::to_string(message);

    match serialized_message {
        Ok(serialized_message) => serialized_message,
        Err(_error) => message.to_string(),
    }
}

fn hash_text(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());

    hex_encode(digest)
}

fn hash_blocks(blocks: &[String]) -> Option<String> {
    if blocks.is_empty() {
        return None;
    }

    let joined_blocks = blocks.join("\n\n");
    let digest = Sha256::digest(joined_blocks.as_bytes());
    let digest_hex = hex_encode(digest);

    Some(digest_hex)
}

fn hash_serialized_parts(serialized_parts: &[String]) -> Option<String> {
    if serialized_parts.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();

    for serialized_part in serialized_parts {
        let part_bytes = serialized_part.as_bytes();

        hasher.update(part_bytes);
        hasher.update(b"\n--prompt-frame-boundary--\n");
    }

    let digest = hasher.finalize();
    let encoded = hex_encode(digest);

    Some(encoded)
}

fn count_chars_as_u32(content: &str) -> u32 {
    let char_count = content.chars().count();

    u32::try_from(char_count).unwrap_or(u32::MAX)
}

fn estimate_tokens_from_chars(char_count: u32) -> u32 {
    let adjusted_chars = char_count.saturating_add(3);

    adjusted_chars / 4
}

fn layer_segment_count(layer_stats: &PromptFrameLayerStats) -> usize {
    let fragment_count = usize::try_from(layer_stats.fragment_count).unwrap_or(usize::MAX);
    let message_count = usize::try_from(layer_stats.message_count).unwrap_or(usize::MAX);

    fragment_count.saturating_add(message_count)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::conversation::ContextArtifactDescriptor;
    use crate::conversation::ContextArtifactKind;
    use crate::conversation::PromptFragment;
    use crate::conversation::PromptFrameAuthority;
    use crate::conversation::PromptLane;
    use crate::conversation::ToolOutputStreamingPolicy;

    use super::*;
    use crate::conversation::turn_shared::build_tool_result_followup_tail;
    use crate::conversation::turn_shared::reduce_followup_payload_for_model;

    #[test]
    fn summarize_assembled_prompt_frame_tracks_stable_prefix_and_recall_layers() {
        let prompt_fragments = vec![
            PromptFragment::new(
                "base-system",
                PromptLane::BaseSystem,
                "base-system",
                "base system",
                ContextArtifactKind::SystemPrompt,
            )
            .with_cacheable(true),
            PromptFragment::new(
                "runtime-identity",
                PromptLane::RuntimeIdentity,
                "runtime-identity",
                "resolved identity",
                ContextArtifactKind::Profile,
            )
            .with_cacheable(true)
            .with_frame_authority(PromptFrameAuthority::RuntimeIdentity),
            PromptFragment::new(
                "tool-discovery-delta",
                PromptLane::ToolDiscoveryDelta,
                "tool-discovery-delta",
                "delta state",
                ContextArtifactKind::ToolHint,
            ),
        ];
        let messages = vec![
            json!({
                "role": "system",
                "content": "compiled system"
            }),
            json!({
                "role": "system",
                "content": "profile note"
            }),
            json!({
                "role": "assistant",
                "content": "recent assistant turn"
            }),
        ];
        let artifacts = vec![
            ContextArtifactDescriptor {
                message_index: 0,
                artifact_kind: ContextArtifactKind::SystemPrompt,
                maskable: false,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
            ContextArtifactDescriptor {
                message_index: 1,
                artifact_kind: ContextArtifactKind::Profile,
                maskable: false,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
            ContextArtifactDescriptor {
                message_index: 2,
                artifact_kind: ContextArtifactKind::ConversationTurn,
                maskable: false,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
        ];
        let assembled = AssembledConversationContext {
            messages,
            artifacts,
            estimated_tokens: None,
            prompt_fragments,
            system_prompt_addition: None,
        };

        let summary = summarize_assembled_prompt_frame(&assembled);

        assert_eq!(summary.stable_runtime_guidance.fragment_count, 1);
        assert_eq!(summary.session_latched_context.fragment_count, 1);
        assert_eq!(summary.session_local_recall.fragment_count, 1);
        assert_eq!(summary.advisory_profile.message_count, 1);
        assert_eq!(summary.recent_window.message_count, 1);
        assert!(summary.stable_runtime_guidance_hash_sha256.is_some());
        assert!(summary.session_latched_context_hash_sha256.is_some());
        assert!(summary.stable_prefix_hash_sha256.is_some());
        assert!(summary.advisory_profile_hash_sha256.is_some());
        assert!(summary.local_recall_hash_sha256.is_some());
        assert!(summary.recent_window_hash_sha256.is_some());
        assert!(summary.turn_ephemeral_hash_sha256.is_none());
    }

    #[test]
    fn summarize_followup_prompt_frame_classifies_live_tail_messages() {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "[tool_result]\n{\"status\":\"ok\"}"
            }),
            json!({
                "role": "user",
                "content": "Use the tool result above."
            }),
        ];

        let summary = summarize_followup_prompt_frame(messages.as_slice());

        assert_eq!(summary.turn_ephemeral_tail.message_count, 2);
        assert!(summary.turn_ephemeral_hash_sha256.is_some());
        assert!(summary.stable_prefix_hash_sha256.is_none());
    }

    #[test]
    fn summarize_assembled_prompt_frame_keeps_profile_memory_out_of_local_recall_hash() {
        let messages = vec![
            json!({
                "role": "system",
                "content": "compiled system"
            }),
            json!({
                "role": "user",
                "content": "## Session Profile\n- prefers concise replies"
            }),
            json!({
                "role": "user",
                "content": "## Memory Summary\n- earlier recall"
            }),
        ];
        let artifacts = vec![
            ContextArtifactDescriptor {
                message_index: 0,
                artifact_kind: ContextArtifactKind::SystemPrompt,
                maskable: false,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
            ContextArtifactDescriptor {
                message_index: 1,
                artifact_kind: ContextArtifactKind::Profile,
                maskable: true,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
            ContextArtifactDescriptor {
                message_index: 2,
                artifact_kind: ContextArtifactKind::Summary,
                maskable: true,
                streaming_policy: ToolOutputStreamingPolicy::BufferFull,
            },
        ];
        let assembled = AssembledConversationContext {
            messages,
            artifacts,
            estimated_tokens: None,
            prompt_fragments: Vec::new(),
            system_prompt_addition: None,
        };

        let summary = summarize_assembled_prompt_frame(&assembled);

        assert_eq!(summary.advisory_profile.message_count, 1);
        assert_eq!(summary.session_local_recall.message_count, 1);
        assert!(summary.advisory_profile_hash_sha256.is_some());
        assert!(summary.local_recall_hash_sha256.is_some());
    }

    #[test]
    fn prompt_frame_defaults_unartifected_turns_to_recent_window() {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "earlier assistant turn"
            }),
            json!({
                "role": "user",
                "content": "later user turn"
            }),
        ];
        let summary = PromptFrameSummary::from_context(messages.as_slice(), &[], &[]);

        assert_eq!(summary.recent_window.message_count, 2);
        assert_eq!(summary.turn_ephemeral_tail.message_count, 0);
        assert!(summary.recent_window_hash_sha256.is_some());
        assert!(summary.turn_ephemeral_hash_sha256.is_none());
    }

    #[test]
    fn compacted_followup_tail_keeps_stable_prefix_hash_constant() {
        let stable_fragments = vec![
            PromptFragment::new(
                "base-system",
                PromptLane::BaseSystem,
                "base-system",
                "base system",
                ContextArtifactKind::SystemPrompt,
            )
            .with_cacheable(true),
            PromptFragment::new(
                "runtime-identity",
                PromptLane::RuntimeIdentity,
                "runtime-identity",
                "resolved identity",
                ContextArtifactKind::Profile,
            )
            .with_cacheable(true),
        ];
        let raw_tool_result = "[ok] {\"payload_summary\":{\"stdout\":\"line 1\\nline 2\\nline 3\\nline 4\\nline 5\\nline 6\"}}";
        let raw_messages = build_tool_result_followup_tail(
            "assistant preface",
            raw_tool_result,
            "summarize the command output",
            None,
            |_label, text| text.to_owned(),
        );
        let compacted_messages = build_tool_result_followup_tail(
            "assistant preface",
            raw_tool_result,
            "summarize the command output",
            None,
            |label, text| {
                let reduced = reduce_followup_payload_for_model(label, text);
                reduced.into_owned()
            },
        );
        let raw_summary = PromptFrameSummary::from_context_with_ephemeral_tail_start(
            raw_messages.as_slice(),
            &[],
            stable_fragments.as_slice(),
            Some(0),
            None,
        );
        let compacted_summary = PromptFrameSummary::from_context_with_ephemeral_tail_start(
            compacted_messages.as_slice(),
            &[],
            stable_fragments.as_slice(),
            Some(0),
            None,
        );

        assert_eq!(
            raw_summary.stable_prefix_hash_sha256,
            compacted_summary.stable_prefix_hash_sha256,
        );
        assert!(
            raw_summary.turn_ephemeral_tail.total_chars
                >= compacted_summary.turn_ephemeral_tail.total_chars,
        );
    }

    #[test]
    fn compacted_summary_turns_classify_as_session_local_recall() {
        let messages = vec![
            json!({
                "role": "user",
                "content": "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 3 earlier turns\nuser: older ask",
            }),
            json!({
                "role": "assistant",
                "content": "recent reply",
            }),
        ];
        let summary = PromptFrameSummary::from_context(messages.as_slice(), &[], &[]);
        let recall_bucket = summary
            .bucket(PromptFrameLayer::SessionLocalRecall)
            .expect("session local recall bucket");
        let recent_bucket = summary
            .bucket(PromptFrameLayer::RecentWindow)
            .expect("recent window bucket");

        assert_eq!(recall_bucket.message_count, 1);
        assert_eq!(recent_bucket.message_count, 1);
    }
}
