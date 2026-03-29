use std::collections::BTreeSet;
use std::path::Path;

use loongclaw_app as mvp;

use crate::provider_credential_policy;

use super::channels;
use super::discovery::{build_import_candidate, resolve_channel_import_readiness_from_config};
use super::provider_transport;
use super::types::{
    ChannelCandidate, DomainPreview, ImportCandidate, ImportSourceKind, PreviewDecision,
    PreviewStatus, SetupDomainKind, WorkspaceGuidanceCandidate,
};

pub fn compose_recommended_import_candidate(
    candidates: &[ImportCandidate],
) -> Option<ImportCandidate> {
    if candidates.len() < 2 {
        return None;
    }

    let mut merged_config = current_candidate(candidates)
        .map(|candidate| candidate.config.clone())
        .unwrap_or_default();
    let mut domains = Vec::new();

    if let Some(provider_domain) = compose_provider_domain(&mut merged_config, candidates) {
        domains.push(provider_domain);
    }

    let (channel_candidates, channels_domain) =
        compose_channels_domain(&mut merged_config, candidates);
    if let Some(channels_domain) = channels_domain {
        domains.push(channels_domain);
    }
    if let Some(domain) = compose_cli_domain(&mut merged_config, candidates) {
        domains.push(domain);
    }
    if let Some(domain) = compose_memory_domain(&mut merged_config, candidates) {
        domains.push(domain);
    }
    if let Some(domain) = compose_tools_domain(&mut merged_config, candidates) {
        domains.push(domain);
    }

    let workspace_guidance = merge_workspace_guidance(candidates);
    if !workspace_guidance.is_empty() {
        domains.push(DomainPreview {
            kind: SetupDomainKind::WorkspaceGuidance,
            status: PreviewStatus::Ready,
            decision: Some(PreviewDecision::UseDetected),
            source: crate::source_presentation::workspace_source_label().to_owned(),
            summary: workspace_guidance_summary(&workspace_guidance),
        });
    }

    domains.sort_by_key(|domain| domain.kind);
    domains.dedup_by_key(|domain| domain.kind);

    if domains.is_empty() && workspace_guidance.is_empty() && channel_candidates.is_empty() {
        return None;
    }

    let mut candidate = build_import_candidate(
        ImportSourceKind::RecommendedPlan,
        crate::source_presentation::recommended_plan_source_label().to_owned(),
        merged_config.clone(),
        resolve_channel_import_readiness_from_config,
        workspace_guidance.clone(),
    )
    .unwrap_or(ImportCandidate {
        source_kind: ImportSourceKind::RecommendedPlan,
        source: crate::source_presentation::recommended_plan_source_label().to_owned(),
        config: merged_config.clone(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: workspace_guidance.clone(),
    });
    candidate.config = merged_config;
    candidate.domains = domains;
    candidate.channel_candidates = channel_candidates;
    candidate.workspace_guidance = workspace_guidance;
    Some(candidate)
}

pub fn prepend_recommended_import_candidate(
    mut candidates: Vec<ImportCandidate>,
) -> Vec<ImportCandidate> {
    if let Some(candidate) = compose_recommended_import_candidate(&candidates) {
        candidates.insert(0, candidate);
    }
    candidates
}

fn current_candidate(candidates: &[ImportCandidate]) -> Option<&ImportCandidate> {
    candidates
        .iter()
        .find(|candidate| candidate.source_kind == ImportSourceKind::ExistingLoongClawConfig)
}

fn domain_for_kind(candidate: &ImportCandidate, kind: SetupDomainKind) -> Option<&DomainPreview> {
    candidate.domains.iter().find(|domain| domain.kind == kind)
}

fn choose_passive_domain_candidate(
    kind: SetupDomainKind,
    candidates: &[ImportCandidate],
) -> Option<&ImportCandidate> {
    if let Some(candidate) =
        current_candidate(candidates).filter(|candidate| domain_for_kind(candidate, kind).is_some())
    {
        return Some(candidate);
    }

    candidates
        .iter()
        .filter(|candidate| candidate.source_kind != ImportSourceKind::ExistingLoongClawConfig)
        .find(|candidate| domain_for_kind(candidate, kind).is_some())
}

fn compose_provider_domain(
    merged_config: &mut mvp::config::LoongClawConfig,
    candidates: &[ImportCandidate],
) -> Option<DomainPreview> {
    let current = current_candidate(candidates);
    if current.is_none() {
        let ready_provider_kinds = candidates
            .iter()
            .filter(|candidate| {
                domain_for_kind(candidate, SetupDomainKind::Provider)
                    .is_some_and(|domain| domain.status == PreviewStatus::Ready)
            })
            .map(|candidate| candidate.config.provider.kind.profile().id)
            .collect::<BTreeSet<_>>();
        if ready_provider_kinds.len() > 1 {
            return None;
        }
    }
    let current_domain =
        current.and_then(|candidate| domain_for_kind(candidate, SetupDomainKind::Provider));

    let chosen = if let Some(candidate) = current {
        match current_domain.map(|domain| domain.status) {
            Some(PreviewStatus::Ready) => candidate,
            _ => select_provider_upgrade_candidate(candidate, candidates).unwrap_or(candidate),
        }
    } else {
        select_provider_upgrade_candidate_from_any(candidates)?
    };
    let base_source = current
        .filter(|candidate| candidate.config.provider.kind == chosen.config.provider.kind)
        .map(|candidate| candidate.source.clone());
    merged_config.provider = if let Some(candidate) =
        current.filter(|candidate| candidate.config.provider.kind == chosen.config.provider.kind)
    {
        candidate.config.provider.clone()
    } else {
        chosen.config.provider.clone()
    };
    merged_config
        .provider
        .canonicalize_configured_auth_env_bindings();
    let mut supplemented_from = Vec::new();
    for candidate in candidates {
        if Some(candidate.source.as_str()) == base_source.as_deref() {
            continue;
        }
        let Some(_) = domain_for_kind(candidate, SetupDomainKind::Provider) else {
            continue;
        };
        if candidate.config.provider.kind == merged_config.provider.kind
            && supplement_provider_config(&mut merged_config.provider, &candidate.config.provider)
        {
            supplemented_from.push(candidate.source.clone());
        }
    }
    let conflicting_ready_sources = candidates
        .iter()
        .filter(|candidate| candidate.source != chosen.source)
        .filter(|candidate| candidate.config.provider.kind != merged_config.provider.kind)
        .filter(|candidate| {
            domain_for_kind(candidate, SetupDomainKind::Provider)
                .is_some_and(|domain| domain.status == PreviewStatus::Ready)
        })
        .map(|candidate| candidate.source.clone())
        .collect::<Vec<_>>();

    Some(DomainPreview {
        kind: SetupDomainKind::Provider,
        status: if provider_credential_policy::provider_has_locally_available_credentials(
            &merged_config.provider,
        ) {
            PreviewStatus::Ready
        } else {
            PreviewStatus::NeedsReview
        },
        decision: if !conflicting_ready_sources.is_empty()
            && current
                .and_then(|candidate| domain_for_kind(candidate, SetupDomainKind::Provider))
                .is_some_and(|domain| domain.status != PreviewStatus::Ready)
            && base_source.as_deref() == current.map(|candidate| candidate.source.as_str())
        {
            Some(PreviewDecision::ReviewConflict)
        } else if !supplemented_from.is_empty() {
            Some(PreviewDecision::Supplement)
        } else if base_source.as_deref() == current.map(|candidate| candidate.source.as_str()) {
            Some(PreviewDecision::KeepCurrent)
        } else {
            Some(PreviewDecision::UseDetected)
        },
        source: if base_source.as_deref() == Some(chosen.source.as_str())
            || supplemented_from
                .iter()
                .any(|source| source == &chosen.source)
        {
            chosen.source.clone()
        } else {
            base_source.unwrap_or_else(|| chosen.source.clone())
        },
        summary: provider_summary(
            &merged_config.provider,
            &supplemented_from,
            &conflicting_ready_sources,
        ),
    })
}

fn select_provider_upgrade_candidate<'a>(
    current: &'a ImportCandidate,
    candidates: &'a [ImportCandidate],
) -> Option<&'a ImportCandidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.source != current.source)
        .filter(|candidate| domain_for_kind(candidate, SetupDomainKind::Provider).is_some())
        .find(|candidate| {
            candidate.config.provider.kind == current.config.provider.kind
                && domain_for_kind(candidate, SetupDomainKind::Provider)
                    .is_some_and(|domain| domain.status == PreviewStatus::Ready)
        })
}

fn select_provider_upgrade_candidate_from_any(
    candidates: &[ImportCandidate],
) -> Option<&ImportCandidate> {
    candidates
        .iter()
        .filter(|candidate| domain_for_kind(candidate, SetupDomainKind::Provider).is_some())
        .find(|candidate| {
            domain_for_kind(candidate, SetupDomainKind::Provider)
                .is_some_and(|domain| domain.status == PreviewStatus::Ready)
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|candidate| domain_for_kind(candidate, SetupDomainKind::Provider).is_some())
        })
}

fn supplement_provider_config(
    target: &mut mvp::config::ProviderConfig,
    source: &mvp::config::ProviderConfig,
) -> bool {
    if target.kind != source.kind {
        return false;
    }
    target.canonicalize_configured_auth_env_bindings();
    let mut source = source.clone();
    source.canonicalize_configured_auth_env_bindings();
    let default_provider = mvp::config::ProviderConfig::default();
    let target_has_auth =
        provider_credential_policy::provider_has_locally_available_credentials(target);
    let source_has_auth =
        provider_credential_policy::provider_has_locally_available_credentials(&source);
    let mut changed = false;
    if (target.model.trim().is_empty() || target.model.eq_ignore_ascii_case("auto"))
        && !source.model.trim().is_empty()
    {
        target.model = source.model.clone();
        changed = true;
    }
    changed |= provider_transport::supplement_provider_transport(target, &source);
    if (target.api_key.is_none() || (!target_has_auth && source_has_auth))
        && source.api_key.is_some()
        && target.api_key != source.api_key
    {
        target.api_key = source.api_key.clone();
        changed = true;
    }
    if (target.oauth_access_token.is_none() || (!target_has_auth && source_has_auth))
        && source.oauth_access_token.is_some()
        && target.oauth_access_token != source.oauth_access_token
    {
        target.oauth_access_token = source.oauth_access_token.clone();
        changed = true;
    }
    let target_missing_auth_config =
        target.api_key.is_none() && target.oauth_access_token.is_none();
    let should_materialize_source_env_binding =
        source_has_auth && (target_missing_auth_config || !target_has_auth);
    if should_materialize_source_env_binding {
        let source_binding =
            provider_credential_policy::provider_available_credential_env_binding(&source);
        let target_binding =
            provider_credential_policy::configured_provider_credential_env_binding(target);
        let binding_changed = source_binding != target_binding;
        let source_binding = if binding_changed {
            source_binding
        } else {
            None
        };
        if let Some(source_binding) = source_binding {
            provider_credential_policy::apply_provider_credential_env_binding(
                target,
                &source_binding,
            );
            changed = true;
        }
    }
    if target.endpoint.is_none() && source.endpoint.is_some() {
        target.endpoint = source.endpoint.clone();
        changed = true;
    }
    if target.models_endpoint.is_none() && source.models_endpoint.is_some() {
        target.models_endpoint = source.models_endpoint.clone();
        changed = true;
    }
    if target.reasoning_effort.is_none() && source.reasoning_effort.is_some() {
        target.reasoning_effort = source.reasoning_effort;
        changed = true;
    }
    if target.max_tokens.is_none() && source.max_tokens.is_some() {
        target.max_tokens = source.max_tokens;
        changed = true;
    }
    if target.temperature == default_provider.temperature
        && source.temperature != default_provider.temperature
    {
        target.temperature = source.temperature;
        changed = true;
    }
    if target.request_timeout_ms == default_provider.request_timeout_ms
        && source.request_timeout_ms != default_provider.request_timeout_ms
    {
        target.request_timeout_ms = source.request_timeout_ms;
        changed = true;
    }
    if target.retry_max_attempts == default_provider.retry_max_attempts
        && source.retry_max_attempts != default_provider.retry_max_attempts
    {
        target.retry_max_attempts = source.retry_max_attempts;
        changed = true;
    }
    if target.retry_initial_backoff_ms == default_provider.retry_initial_backoff_ms
        && source.retry_initial_backoff_ms != default_provider.retry_initial_backoff_ms
    {
        target.retry_initial_backoff_ms = source.retry_initial_backoff_ms;
        changed = true;
    }
    if target.retry_max_backoff_ms == default_provider.retry_max_backoff_ms
        && source.retry_max_backoff_ms != default_provider.retry_max_backoff_ms
    {
        target.retry_max_backoff_ms = source.retry_max_backoff_ms;
        changed = true;
    }
    for model in &source.preferred_models {
        if !target.preferred_models.contains(model) {
            target.preferred_models.push(model.clone());
            changed = true;
        }
    }
    for (key, value) in &source.headers {
        if !target
            .headers
            .keys()
            .any(|existing| existing.eq_ignore_ascii_case(key))
        {
            target.headers.insert(key.clone(), value.clone());
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplement_provider_config_canonicalizes_legacy_api_key_env_binding() {
        let mut target =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        target.api_key = None;
        target.set_api_key_env(None);

        let mut source =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        source.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));

        let changed = supplement_provider_config(&mut target, &source);

        assert!(changed);
        assert_eq!(
            target.api_key,
            Some(loongclaw_contracts::SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(target.api_key_env, None);
    }

    #[test]
    fn compose_provider_domain_canonicalizes_selected_current_provider() {
        let mut merged_config = mvp::config::LoongClawConfig::default();
        let mut current = ImportCandidate {
            source_kind: ImportSourceKind::ExistingLoongClawConfig,
            source: "current config".to_owned(),
            config: mvp::config::LoongClawConfig::default(),
            surfaces: Vec::new(),
            domains: Vec::new(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        };
        current.config.provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        current
            .config
            .provider
            .set_api_key_env(Some("OPENAI_API_KEY".to_owned()));
        current.domains.push(DomainPreview {
            kind: SetupDomainKind::Provider,
            status: PreviewStatus::Ready,
            decision: Some(PreviewDecision::KeepCurrent),
            source: current.source.clone(),
            summary: String::new(),
        });

        let domain = compose_provider_domain(&mut merged_config, &[current]);

        assert!(domain.is_some());
        assert_eq!(
            merged_config.provider.api_key,
            Some(loongclaw_contracts::SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(merged_config.provider.api_key_env, None);
    }

    #[test]
    fn supplement_provider_config_accepts_x_api_key_source_credentials() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
        let mut target =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Anthropic);
        target.api_key = None;
        target.set_api_key_env(None);

        let source =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Anthropic);

        let changed = supplement_provider_config(&mut target, &source);

        assert!(changed);
        assert_eq!(
            target.api_key,
            Some(loongclaw_contracts::SecretRef::Env {
                env: "ANTHROPIC_API_KEY".to_owned(),
            })
        );
    }
}

fn compose_cli_domain(
    merged_config: &mut mvp::config::LoongClawConfig,
    candidates: &[ImportCandidate],
) -> Option<DomainPreview> {
    let primary_candidate = choose_passive_domain_candidate(SetupDomainKind::Cli, candidates)?;
    let mut changed_sources = Vec::new();
    for candidate in candidates {
        let Some(_) = domain_for_kind(candidate, SetupDomainKind::Cli) else {
            continue;
        };
        if supplement_cli_config(&mut merged_config.cli, &candidate.config.cli) {
            changed_sources.push(candidate.source.clone());
        }
    }
    let supplemented_from = changed_sources
        .into_iter()
        .filter(|source| source != &primary_candidate.source)
        .collect::<Vec<_>>();
    Some(DomainPreview {
        kind: SetupDomainKind::Cli,
        status: PreviewStatus::Ready,
        decision: Some(passive_domain_decision(
            primary_candidate.source_kind,
            &supplemented_from,
        )),
        source: if supplemented_from.is_empty() {
            primary_candidate.source.clone()
        } else {
            "multiple sources".to_owned()
        },
        summary: cli_summary(&merged_config.cli, &supplemented_from),
    })
}

fn compose_memory_domain(
    merged_config: &mut mvp::config::LoongClawConfig,
    candidates: &[ImportCandidate],
) -> Option<DomainPreview> {
    let primary_candidate = choose_passive_domain_candidate(SetupDomainKind::Memory, candidates)?;
    let mut changed_sources = Vec::new();
    for candidate in candidates {
        let Some(_) = domain_for_kind(candidate, SetupDomainKind::Memory) else {
            continue;
        };
        if supplement_memory_config(&mut merged_config.memory, &candidate.config.memory) {
            changed_sources.push(candidate.source.clone());
        }
    }
    let supplemented_from = changed_sources
        .into_iter()
        .filter(|source| source != &primary_candidate.source)
        .collect::<Vec<_>>();
    Some(DomainPreview {
        kind: SetupDomainKind::Memory,
        status: PreviewStatus::Ready,
        decision: Some(passive_domain_decision(
            primary_candidate.source_kind,
            &supplemented_from,
        )),
        source: if supplemented_from.is_empty() {
            primary_candidate.source.clone()
        } else {
            "multiple sources".to_owned()
        },
        summary: memory_summary(&merged_config.memory, &supplemented_from),
    })
}

fn compose_tools_domain(
    merged_config: &mut mvp::config::LoongClawConfig,
    candidates: &[ImportCandidate],
) -> Option<DomainPreview> {
    let primary_candidate = choose_passive_domain_candidate(SetupDomainKind::Tools, candidates)?;
    let mut changed_sources = Vec::new();
    for candidate in candidates {
        let Some(_) = domain_for_kind(candidate, SetupDomainKind::Tools) else {
            continue;
        };
        if supplement_tool_config(&mut merged_config.tools, &candidate.config.tools) {
            changed_sources.push(candidate.source.clone());
        }
    }
    let supplemented_from = changed_sources
        .into_iter()
        .filter(|source| source != &primary_candidate.source)
        .collect::<Vec<_>>();
    Some(DomainPreview {
        kind: SetupDomainKind::Tools,
        status: PreviewStatus::NeedsReview,
        decision: Some(passive_domain_decision(
            primary_candidate.source_kind,
            &supplemented_from,
        )),
        source: if supplemented_from.is_empty() {
            primary_candidate.source.clone()
        } else {
            "multiple sources".to_owned()
        },
        summary: tool_summary(&merged_config.tools, &supplemented_from),
    })
}

fn supplement_cli_config(
    target: &mut mvp::config::CliChannelConfig,
    source: &mvp::config::CliChannelConfig,
) -> bool {
    let default = mvp::config::CliChannelConfig::default();
    let mut changed = false;
    if !target.enabled && source.enabled {
        target.enabled = true;
        changed = true;
    }
    let target_prompt_defaults = target.prompt_pack_id == default.prompt_pack_id
        && target.personality == default.personality
        && target.system_prompt_addendum == default.system_prompt_addendum
        && target.system_prompt == default.system_prompt;
    let source_prompt_changed = source.prompt_pack_id != default.prompt_pack_id
        || source.personality != default.personality
        || source.system_prompt_addendum != default.system_prompt_addendum
        || source.system_prompt != default.system_prompt;
    if target_prompt_defaults && source_prompt_changed {
        target.prompt_pack_id = source.prompt_pack_id.clone();
        target.personality = source.personality;
        target.system_prompt_addendum = source.system_prompt_addendum.clone();
        target.system_prompt = source.system_prompt.clone();
        if target.uses_native_prompt_pack() {
            target.refresh_native_system_prompt();
        }
        changed = true;
    }
    for command in &source.exit_commands {
        if !target.exit_commands.contains(command) {
            target.exit_commands.push(command.clone());
            changed = true;
        }
    }
    changed
}

fn supplement_memory_config(
    target: &mut mvp::config::MemoryConfig,
    source: &mvp::config::MemoryConfig,
) -> bool {
    let default = mvp::config::MemoryConfig::default();
    let mut changed = false;
    if target.profile == default.profile && source.profile != default.profile {
        target.profile = source.profile;
        changed = true;
    }
    if target.sqlite_path == default.sqlite_path && source.sqlite_path != default.sqlite_path {
        target.sqlite_path = source.sqlite_path.clone();
        changed = true;
    }
    if target.sliding_window == default.sliding_window
        && source.sliding_window != default.sliding_window
    {
        target.sliding_window = source.sliding_window;
        changed = true;
    }
    changed
}

fn supplement_tool_config(
    target: &mut mvp::config::ToolConfig,
    source: &mvp::config::ToolConfig,
) -> bool {
    let mut changed = false;
    if target.file_root.is_none() && source.file_root.is_some() {
        target.file_root = source.file_root.clone();
        changed = true;
    }
    for command in &source.shell_allow {
        if !target.shell_allow.contains(command) {
            target.shell_allow.push(command.clone());
            changed = true;
        }
    }
    changed
}

fn cli_summary(config: &mvp::config::CliChannelConfig, supplemented_from: &[String]) -> String {
    let mut parts = Vec::new();
    if config.uses_native_prompt_pack() {
        parts.push("native prompt pack".to_owned());
        parts.push(format!(
            "personality {}",
            crate::onboard_cli::prompt_personality_id(config.resolved_personality())
        ));
        if config
            .system_prompt_addendum
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            parts.push("prompt addendum configured".to_owned());
        }
    } else if !config.system_prompt.trim().is_empty() {
        parts.push("inline system prompt override".to_owned());
    }
    if config.exit_commands != mvp::config::CliChannelConfig::default().exit_commands {
        parts.push(format!("exit commands {}", config.exit_commands.join(", ")));
    }
    let mut summary = if parts.is_empty() {
        "custom CLI behavior detected".to_owned()
    } else {
        parts.join(" · ")
    };
    if !supplemented_from.is_empty() {
        summary.push_str(" · supplemented from ");
        summary.push_str(&supplemented_from.join(", "));
    }
    summary
}

fn memory_summary(config: &mvp::config::MemoryConfig, supplemented_from: &[String]) -> String {
    let mut summary = format!(
        "profile {} · {} · window {}",
        config.profile.as_str(),
        config.resolved_sqlite_path().display(),
        config.sliding_window
    );
    if !supplemented_from.is_empty() {
        summary.push_str(" · supplemented from ");
        summary.push_str(&supplemented_from.join(", "));
    }
    summary
}

fn tool_summary(config: &mvp::config::ToolConfig, supplemented_from: &[String]) -> String {
    let default = mvp::config::ToolConfig::default();
    let mut parts = Vec::new();
    if config.file_root != default.file_root {
        parts.push(format!(
            "workspace root {}",
            config.resolved_file_root().display()
        ));
    }
    if config.shell_allow != default.shell_allow {
        parts.push(format!(
            "shell permissions {}",
            config.shell_allow.join(", ")
        ));
    }
    let mut summary = parts.join(" · ");
    if !supplemented_from.is_empty() {
        summary.push_str(" · supplemented from ");
        summary.push_str(&supplemented_from.join(", "));
    }
    summary
}

fn provider_summary(
    config: &mvp::config::ProviderConfig,
    supplemented_from: &[String],
    conflicting_ready_sources: &[String],
) -> String {
    let mut summary = crate::provider_presentation::provider_identity_summary(config);
    if !supplemented_from.is_empty() {
        summary.push_str(" · supplemented from ");
        summary.push_str(&supplemented_from.join(", "));
    }
    if !conflicting_ready_sources.is_empty() {
        summary.push_str(" · conflicting ready provider also detected from ");
        summary.push_str(&conflicting_ready_sources.join(", "));
    }
    summary
}

fn compose_channels_domain(
    merged_config: &mut mvp::config::LoongClawConfig,
    candidates: &[ImportCandidate],
) -> (Vec<ChannelCandidate>, Option<DomainPreview>) {
    let mut chosen_channels = Vec::new();
    let mut any_channel_supplemented = false;
    let mut all_channels_from_current = true;
    let mut distinct_channel_sources = BTreeSet::new();
    for channel_id in channels::registered_channel_ids() {
        let Some(channel) = select_channel_candidate(channel_id, candidates) else {
            continue;
        };
        if channel.source_kind != ImportSourceKind::ExistingLoongClawConfig {
            all_channels_from_current = false;
        }
        distinct_channel_sources.insert(channel.source.clone());
        channels::apply_selected_channels(merged_config, &channel.config, &[channel.id]);
        let mut supplemented_from = Vec::new();
        for candidate in candidates {
            if candidate.source == channel.source {
                continue;
            }
            if candidate
                .channel_candidates
                .iter()
                .any(|candidate_channel| candidate_channel.id == channel.id)
                && channels::apply_selected_channels(
                    merged_config,
                    &candidate.config,
                    &[channel.id],
                )
            {
                supplemented_from.push(candidate.source.clone());
            }
        }
        any_channel_supplemented |= !supplemented_from.is_empty();

        let effective_source = if supplemented_from.is_empty() {
            channel.source.clone()
        } else {
            "multiple sources".to_owned()
        };
        let mut final_channel = channels::collect_channel_previews(
            merged_config,
            &resolve_channel_import_readiness_from_config(merged_config),
            &effective_source,
        )
        .into_iter()
        .find(|preview| preview.candidate.id == channel.id)
        .map(|preview| preview.candidate)
        .unwrap_or(ChannelCandidate {
            id: channel.id,
            label: channel.label,
            status: channel.status,
            source: effective_source,
            summary: channel.summary.clone(),
        });
        if !supplemented_from.is_empty() {
            final_channel.summary.push_str(" · supplemented from ");
            final_channel
                .summary
                .push_str(&supplemented_from.join(", "));
        }
        chosen_channels.push(final_channel);
    }

    if chosen_channels.is_empty() {
        return (Vec::new(), None);
    }

    let status = if chosen_channels
        .iter()
        .all(|channel| channel.status == PreviewStatus::Ready)
    {
        PreviewStatus::Ready
    } else if chosen_channels
        .iter()
        .any(|channel| channel.status == PreviewStatus::NeedsReview)
    {
        PreviewStatus::NeedsReview
    } else {
        PreviewStatus::Unavailable
    };
    let unique_sources = chosen_channels
        .iter()
        .map(|channel| channel.source.clone())
        .collect::<BTreeSet<_>>();
    let summary = chosen_channels
        .iter()
        .map(|channel| {
            format!(
                "{} {} from {}",
                channel.label,
                channel.status.label(),
                channel.source
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let source = if unique_sources.len() == 1 {
        unique_sources.iter().next().cloned().unwrap_or_default()
    } else {
        "multiple sources".to_owned()
    };
    (
        chosen_channels
            .iter()
            .map(|channel| ChannelCandidate {
                id: channel.id,
                label: channel.label,
                status: channel.status,
                source: channel.source.clone(),
                summary: channel.summary.clone(),
            })
            .collect(),
        Some(DomainPreview {
            kind: SetupDomainKind::Channels,
            status,
            decision: Some(
                if any_channel_supplemented || distinct_channel_sources.len() > 1 {
                    PreviewDecision::Supplement
                } else if all_channels_from_current {
                    PreviewDecision::KeepCurrent
                } else {
                    PreviewDecision::UseDetected
                },
            ),
            source,
            summary,
        }),
    )
}

fn passive_domain_decision(
    source_kind: ImportSourceKind,
    supplemented_from: &[String],
) -> PreviewDecision {
    if !supplemented_from.is_empty() {
        PreviewDecision::Supplement
    } else if source_kind == ImportSourceKind::ExistingLoongClawConfig {
        PreviewDecision::KeepCurrent
    } else {
        PreviewDecision::UseDetected
    }
}

fn select_channel_candidate(
    channel_id: &str,
    candidates: &[ImportCandidate],
) -> Option<SelectedChannel> {
    let mut best: Option<SelectedChannel> = None;
    for candidate in candidates {
        if let Some(channel) = candidate
            .channel_candidates
            .iter()
            .find(|channel| channel.id == channel_id)
        {
            let selected = SelectedChannel {
                id: channel.id,
                status: channel.status,
                label: channel.label,
                source_kind: candidate.source_kind,
                source: channel.source.clone(),
                summary: channel.summary.clone(),
                config: candidate.config.clone(),
            };
            if best.as_ref().is_none_or(|current| {
                selected_channel_score(&selected) > selected_channel_score(current)
            }) {
                best = Some(selected);
            }
        }
    }
    best
}

#[derive(Clone)]
struct SelectedChannel {
    id: &'static str,
    label: &'static str,
    status: PreviewStatus,
    source_kind: ImportSourceKind,
    source: String,
    summary: String,
    config: mvp::config::LoongClawConfig,
}

fn selected_channel_score(channel: &SelectedChannel) -> u8 {
    match channel.status {
        PreviewStatus::Ready => 2,
        PreviewStatus::NeedsReview => 1,
        PreviewStatus::Unavailable => 0,
    }
}

fn merge_workspace_guidance(candidates: &[ImportCandidate]) -> Vec<WorkspaceGuidanceCandidate> {
    let mut seen = BTreeSet::new();
    let mut merged = Vec::new();
    for candidate in candidates {
        for item in &candidate.workspace_guidance {
            if seen.insert(item.path.clone()) {
                merged.push(item.clone());
            }
        }
    }
    merged
}

fn workspace_guidance_summary(guidance: &[WorkspaceGuidanceCandidate]) -> String {
    guidance
        .iter()
        .map(|item| {
            Path::new(&item.path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| item.path.clone())
        })
        .collect::<Vec<_>>()
        .join(", ")
}
