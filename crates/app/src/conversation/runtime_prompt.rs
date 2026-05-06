use std::collections::BTreeSet;

use crate::tools::runtime_config::ToolRuntimeConfig;

use super::super::super::config::LoongConfig;
use super::super::context_engine::ContextArtifactKind;
use super::super::runtime_binding::ConversationRuntimeBinding;
use super::super::subagent::DelegateBuiltinProfile;
#[cfg(feature = "memory-sqlite")]
use super::active_skills;
use super::session_runtime::open_session_repository;
use super::{
    AssembledConversationContext, PromptFragment, PromptFrameAuthority, PromptLane, SessionContext,
    provider,
};

pub(super) fn provider_runtime_binding(
    binding: ConversationRuntimeBinding<'_>,
) -> provider::ProviderRuntimeBinding<'_> {
    match binding {
        ConversationRuntimeBinding::Kernel(kernel_ctx) => {
            provider::ProviderRuntimeBinding::kernel(kernel_ctx)
        }
        ConversationRuntimeBinding::Direct => provider::ProviderRuntimeBinding::advisory_only(),
    }
}

pub(super) fn delegate_child_runtime_contract_prompt_summary(
    config: &LoongConfig,
    session_context: &SessionContext,
) -> Option<String> {
    session_context.parent_session_id.as_ref()?;
    session_context.subagent_runtime_narrowing()?;
    let subagent_contract = session_context.resolved_subagent_contract();
    ToolRuntimeConfig::from_loong_config(config, None)
        .delegate_child_prompt_summary(subagent_contract.as_ref())
}

pub(super) fn delegate_child_profile_prompt_summary(
    session_context: &SessionContext,
) -> Option<String> {
    let _parent_session_id = session_context.parent_session_id.as_ref()?;
    let profile = session_context.profile?;
    let summary = match profile {
        DelegateBuiltinProfile::Research => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `research` delegate profile.\n",
            "- Gather evidence before conclusions.\n",
            "- Prefer reading files, web sources, and browser extraction over proposing edits.\n",
            "- Return concise findings, concrete references, and unresolved risks."
        ),
        DelegateBuiltinProfile::Plan => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `plan` delegate profile.\n",
            "- Turn findings into an execution plan.\n",
            "- Prefer ordered steps, explicit assumptions, and acceptance criteria.\n",
            "- Do not claim implementation is complete when you only have a proposal."
        ),
        DelegateBuiltinProfile::Verify => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `verify` delegate profile.\n",
            "- Try to falsify success claims before accepting them.\n",
            "- Prefer concrete checks, observed failures, and residual risk notes.\n",
            "- Report a clear verdict with evidence."
        ),
    };
    Some(summary.to_owned())
}

pub(super) fn runtime_self_continuity_prompt_summary(
    config: &LoongConfig,
    session_context: &SessionContext,
) -> Option<String> {
    let stored_continuity = session_context.runtime_self_continuity.as_ref()?;
    let live_continuity =
        crate::runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let missing_continuity = crate::runtime_self_continuity::missing_runtime_self_continuity(
        stored_continuity,
        live_continuity.as_ref(),
    )?;
    let inherited = session_context.parent_session_id.is_some();
    crate::runtime_self_continuity::render_runtime_self_continuity_section(
        &missing_continuity,
        inherited,
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn active_skills_prompt_summary(
    config: &LoongConfig,
    session_id: &str,
) -> Option<String> {
    let repo = open_session_repository(config).ok()?;
    let active_skills = active_skills::load_persisted_active_skills(&repo, session_id)
        .ok()
        .flatten()?;
    active_skills::render_active_skills_section(&active_skills)
}

pub(super) fn append_runtime_prompt_fragment(
    assembled: &mut AssembledConversationContext,
    source_id: &'static str,
    content: Option<String>,
    frame_authority: PromptFrameAuthority,
) {
    let Some(content) = content else {
        return;
    };

    let fragment = PromptFragment::new(
        source_id,
        PromptLane::Continuity,
        source_id,
        content,
        ContextArtifactKind::RuntimeContract,
    )
    .with_dedupe_key(source_id)
    .with_cacheable(true)
    .with_frame_authority(frame_authority);

    assembled.prompt_fragments.push(fragment);
}

pub(super) fn normalize_turn_middleware_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            normalized.push(id);
        }
    }
    normalized
}
