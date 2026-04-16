use std::collections::BTreeSet;

use super::{ImportCandidate, ImportSourceKind, ProviderSelectionPlan};

fn normalized_source_label(source: &str) -> Option<String> {
    crate::source_presentation::rollup_source_label(source)
}

fn push_source_label(labels: &mut Vec<String>, seen: &mut BTreeSet<String>, label: Option<String>) {
    if let Some(label) = label
        && seen.insert(label.clone())
    {
        labels.push(label);
    }
}

fn display_line(prefix: &str, value: &str) -> String {
    format!("{prefix}{value}")
}

pub fn candidate_source_rollup_labels(candidate: &ImportCandidate) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();

    if candidate.source_kind != ImportSourceKind::RecommendedPlan {
        push_source_label(
            &mut labels,
            &mut seen,
            normalized_source_label(&candidate.source),
        );
    }

    for domain in &candidate.domains {
        push_source_label(
            &mut labels,
            &mut seen,
            normalized_source_label(&domain.source),
        );
    }

    for channel in &candidate.channel_candidates {
        push_source_label(
            &mut labels,
            &mut seen,
            normalized_source_label(&channel.source),
        );
    }

    if !candidate.workspace_guidance.is_empty() {
        push_source_label(
            &mut labels,
            &mut seen,
            Some(crate::source_presentation::workspace_guidance_rollup_label().to_owned()),
        );
    }

    labels
}

fn render_stacked_domain_lines(domain: &super::DomainPreview, width: usize) -> Vec<String> {
    let mut lines = vec![format!(
        "- {} [{}]",
        domain.kind.label(),
        domain.status.label()
    )];
    if let Some(decision) = domain.decision {
        lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
            "  action: ",
            decision.label(),
            width,
        ));
    }
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  source: ",
        &domain.source,
        width,
    ));
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  summary: ",
        &domain.summary,
        width,
    ));
    lines
}

fn render_wide_domain_line(domain: &super::DomainPreview) -> String {
    let mut parts = vec![
        format!("- {:18} {:14}", domain.kind.label(), domain.status.label()),
        domain
            .decision
            .map(|decision| format!("{:32}", decision.label()))
            .unwrap_or_else(|| format!("{:32}", "")),
        format!("{:28}", domain.source),
        domain.summary.clone(),
    ];
    while parts.last().is_some_and(|part| part.trim().is_empty()) {
        parts.pop();
    }
    parts.join(" ")
}

fn render_stacked_channel_lines(channel: &super::ChannelCandidate, width: usize) -> Vec<String> {
    let mut lines = vec![format!(
        "- {} [{} · {}]",
        channel.label,
        channel.status.label(),
        channel_maturity_label(channel.id)
    )];
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  source: ",
        &channel.source,
        width,
    ));
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  summary: ",
        &channel.summary,
        width,
    ));
    lines
}

fn render_wide_channel_line(channel: &super::ChannelCandidate) -> String {
    format!(
        "- {:18} {:30} {:28} {}",
        channel.label,
        format!(
            "{} · {}",
            channel.status.label(),
            channel_maturity_label(channel.id)
        ),
        channel.source,
        channel.summary
    )
}

fn render_stacked_provider_choice_lines(
    plan: &ProviderSelectionPlan,
    choice: &super::ImportedProviderChoice,
    default_profile_id: Option<&str>,
    width: usize,
) -> Vec<String> {
    let suffix = if Some(choice.profile_id.as_str()) == default_profile_id {
        " (recommended)"
    } else {
        ""
    };
    let mut lines = vec![format!(
        "- {}{}",
        crate::provider_presentation::provider_choice_label(&choice.profile_id, choice.kind),
        suffix
    )];
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  source: ",
        &choice.source,
        width,
    ));
    lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
        "  summary: ",
        &choice.summary,
        width,
    ));
    if let Some(selector_detail) =
        super::provider_selection::selector_detail_line(plan, &choice.profile_id, width)
    {
        lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
            "  ",
            &selector_detail,
            width,
        ));
    }
    if let Some(transport_summary) = choice.config.preview_transport_summary() {
        lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
            "  transport: ",
            &transport_summary,
            width,
        ));
    }
    lines
}

fn render_wide_provider_choice_line(
    choice: &super::ImportedProviderChoice,
    default_profile_id: Option<&str>,
) -> String {
    let suffix = if Some(choice.profile_id.as_str()) == default_profile_id {
        " (recommended)"
    } else {
        ""
    };
    format!(
        "- {:24} {:28} {}{}",
        crate::provider_presentation::provider_choice_label(&choice.profile_id, choice.kind),
        choice.source,
        choice.summary,
        suffix
    )
}

pub fn render_candidate_preview_lines(candidate: &ImportCandidate, width: usize) -> Vec<String> {
    let mut lines =
        loongclaw_app::presentation::render_wrapped_text_line("source: ", &candidate.source, width);
    lines.insert(0, "candidate snapshot".to_owned());
    let source_labels = candidate_source_rollup_labels(candidate);
    let should_render_source_rollup = if candidate.source_kind == ImportSourceKind::RecommendedPlan
    {
        !source_labels.is_empty()
    } else {
        source_labels.len() > 1
    };
    if should_render_source_rollup {
        lines.extend(loongclaw_app::presentation::render_wrapped_segments(
            "derived from: ",
            "  ",
            &source_labels.iter().map(String::as_str).collect::<Vec<_>>(),
            " + ",
            width,
        ));
    }
    if let Some(transport_summary) = candidate.config.provider.preview_transport_summary() {
        lines.extend(loongclaw_app::presentation::render_wrapped_text_line(
            "provider transport: ",
            &transport_summary,
            width,
        ));
    }
    if candidate.domains.is_empty() && candidate.channel_candidates.is_empty() {
        return lines;
    }

    let use_stacked_domains = width < 68
        || candidate
            .domains
            .iter()
            .any(|domain| render_wide_domain_line(domain).len() > width);
    if use_stacked_domains {
        lines.push("domain signals:".to_owned());
        for domain in &candidate.domains {
            lines.extend(render_stacked_domain_lines(domain, width));
        }
    } else {
        lines.push("domain signals:".to_owned());
        for domain in &candidate.domains {
            lines.push(render_wide_domain_line(domain));
        }
    }

    if !candidate.channel_candidates.is_empty() {
        lines.push("channel handoff".to_owned());
        lines.push("channels:".to_owned());
        let use_stacked_channels = width < 68
            || candidate
                .channel_candidates
                .iter()
                .any(|channel| render_wide_channel_line(channel).len() > width);
        if use_stacked_channels {
            for channel in &candidate.channel_candidates {
                lines.extend(render_stacked_channel_lines(channel, width));
            }
        } else {
            lines.extend(
                candidate
                    .channel_candidates
                    .iter()
                    .map(render_wide_channel_line),
            );
        }
    }

    lines
}

pub fn candidate_preview_display_lines(candidate: &ImportCandidate) -> Vec<String> {
    let mut lines = vec![display_line("source: ", &candidate.source)];
    let source_labels = candidate_source_rollup_labels(candidate);
    let should_render_source_rollup = if candidate.source_kind == ImportSourceKind::RecommendedPlan
    {
        !source_labels.is_empty()
    } else {
        source_labels.len() > 1
    };

    if should_render_source_rollup {
        lines.push(display_line("derived from: ", &source_labels.join(" + ")));
    }

    if let Some(transport_summary) = candidate.config.provider.preview_transport_summary() {
        lines.push(display_line("provider transport: ", &transport_summary));
    }

    if candidate.domains.is_empty() && candidate.channel_candidates.is_empty() {
        return lines;
    }

    for domain in &candidate.domains {
        lines.push(format!(
            "- {} [{}]",
            domain.kind.label(),
            domain.status.label()
        ));
        if let Some(decision) = domain.decision {
            lines.push(display_line("  action: ", decision.label()));
        }
        lines.push(display_line("  source: ", &domain.source));
        lines.push(display_line("  summary: ", &domain.summary));
    }

    if !candidate.channel_candidates.is_empty() {
        lines.push("channels:".to_owned());
        for channel in &candidate.channel_candidates {
            lines.push(format!(
                "- {} [{} · {}]",
                channel.label,
                channel.status.label(),
                channel_maturity_label(channel.id)
            ));
            lines.push(display_line("  source: ", &channel.source));
            lines.push(display_line("  summary: ", &channel.summary));
        }
    }

    lines
}

fn channel_maturity_label(channel_id: &'static str) -> &'static str {
    let descriptor = crate::mvp::config::channel_descriptor(channel_id);

    match descriptor.map(|descriptor| descriptor.runtime_kind) {
        Some(crate::mvp::config::ChannelRuntimeKind::RuntimeBacked) => "runtime-backed",
        Some(crate::mvp::config::ChannelRuntimeKind::PluginBacked) => "plugin-backed",
        Some(crate::mvp::config::ChannelRuntimeKind::OutboundOnly) => "outbound-only",
        Some(crate::mvp::config::ChannelRuntimeKind::CatalogOnly) => "catalog-only",
        Some(crate::mvp::config::ChannelRuntimeKind::Interactive) => "interactive",
        None => "channel",
    }
}

pub fn render_provider_selection_lines(plan: &ProviderSelectionPlan, width: usize) -> Vec<String> {
    if plan.imported_choices.is_empty()
        || (!plan.requires_explicit_choice && plan.imported_choices.len() <= 1)
    {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let heading = if plan.requires_explicit_choice {
        "provider choice required"
    } else {
        "provider choices"
    };
    lines.push(format!("{heading}:"));
    let use_stacked_choices = width < 68
        || plan.imported_choices.iter().any(|choice| {
            choice.config.preview_transport_summary().is_some()
                || render_wide_provider_choice_line(choice, plan.default_profile_id.as_deref())
                    .len()
                    > width
        });
    if use_stacked_choices {
        for choice in &plan.imported_choices {
            lines.extend(render_stacked_provider_choice_lines(
                plan,
                choice,
                plan.default_profile_id.as_deref(),
                width,
            ));
        }
    } else {
        for choice in &plan.imported_choices {
            lines.push(render_wide_provider_choice_line(
                choice,
                plan.default_profile_id.as_deref(),
            ));
        }
    }
    if plan.requires_explicit_choice {
        let note_segments = super::unresolved_choice_note_segments(plan);
        lines.extend(loongclaw_app::presentation::render_wrapped_segments(
            if use_stacked_choices {
                "  note: "
            } else {
                "note: "
            },
            "  ",
            &note_segments.iter().map(String::as_str).collect::<Vec<_>>(),
            "; ",
            width,
        ));
    }
    lines
}

pub fn provider_selection_display_lines(plan: &ProviderSelectionPlan) -> Vec<String> {
    if plan.imported_choices.is_empty()
        || (!plan.requires_explicit_choice && plan.imported_choices.len() <= 1)
    {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let heading = if plan.requires_explicit_choice {
        "provider choice required"
    } else {
        "provider choices"
    };
    lines.push(format!("{heading}:"));

    for choice in &plan.imported_choices {
        let suffix = if Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref() {
            " (recommended)"
        } else {
            ""
        };
        let label =
            crate::provider_presentation::provider_choice_label(&choice.profile_id, choice.kind);
        lines.push(format!("- {label}{suffix}"));
        lines.push(display_line("  source: ", &choice.source));
        lines.push(display_line("  summary: ", &choice.summary));
        if let Some(selector_detail) =
            super::provider_selection::selector_detail_line(plan, &choice.profile_id, usize::MAX)
        {
            lines.push(display_line("  ", &selector_detail));
        }
        if let Some(transport_summary) = choice.config.preview_transport_summary() {
            lines.push(display_line("  transport: ", &transport_summary));
        }
    }

    if plan.requires_explicit_choice {
        let note_segments = super::unresolved_choice_note_segments(plan);
        lines.push(display_line("note: ", &note_segments.join("; ")));
    }

    lines
}
