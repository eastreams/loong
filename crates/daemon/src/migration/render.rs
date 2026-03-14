use std::collections::BTreeSet;

use super::{ImportCandidate, ImportSourceKind, ProviderSelectionPlan};

fn normalized_source_label(source: &str) -> Option<String> {
    crate::source_presentation::rollup_source_label(source)
}

fn push_source_label(labels: &mut Vec<String>, seen: &mut BTreeSet<String>, label: Option<String>) {
    if let Some(label) = label {
        if seen.insert(label.clone()) {
            labels.push(label);
        }
    }
}

pub(crate) fn candidate_source_rollup_labels(candidate: &ImportCandidate) -> Vec<String> {
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
    let mut lines = vec![format!("- {} [{}]", channel.label, channel.status.label())];
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
        "- {:18} {:14} {:28} {}",
        channel.label,
        channel.status.label(),
        channel.source,
        channel.summary
    )
}

fn render_stacked_provider_choice_lines(
    choice: &super::ImportedProviderChoice,
    default_kind: Option<loongclaw_app::config::ProviderKind>,
    width: usize,
) -> Vec<String> {
    let suffix = if Some(choice.kind) == default_kind {
        " (recommended)"
    } else {
        ""
    };
    let mut lines = vec![format!(
        "- {}{}",
        crate::provider_presentation::provider_choice_label(choice.kind),
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
    default_kind: Option<loongclaw_app::config::ProviderKind>,
) -> String {
    let suffix = if Some(choice.kind) == default_kind {
        " (recommended)"
    } else {
        ""
    };
    format!(
        "- {:24} {:28} {}{}",
        crate::provider_presentation::provider_choice_label(choice.kind),
        choice.source,
        choice.summary,
        suffix
    )
}

pub(crate) fn render_candidate_preview_lines(
    candidate: &ImportCandidate,
    width: usize,
) -> Vec<String> {
    let mut lines =
        loongclaw_app::presentation::render_wrapped_text_line("source: ", &candidate.source, width);
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
        for domain in &candidate.domains {
            lines.extend(render_stacked_domain_lines(domain, width));
        }
    } else {
        lines.push("domains:".to_owned());
        for domain in &candidate.domains {
            lines.push(render_wide_domain_line(domain));
        }
    }

    if !candidate.channel_candidates.is_empty() {
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

pub(crate) fn render_provider_selection_lines(
    plan: &ProviderSelectionPlan,
    width: usize,
) -> Vec<String> {
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
                || render_wide_provider_choice_line(choice, plan.default_kind).len() > width
        });
    if use_stacked_choices {
        for choice in &plan.imported_choices {
            lines.extend(render_stacked_provider_choice_lines(
                choice,
                plan.default_kind,
                width,
            ));
        }
    } else {
        for choice in &plan.imported_choices {
            lines.push(render_wide_provider_choice_line(choice, plan.default_kind));
        }
    }
    if plan.requires_explicit_choice {
        lines.extend(loongclaw_app::presentation::render_wrapped_segments(
            if use_stacked_choices {
                "  note: "
            } else {
                "note: "
            },
            "  ",
            &[
                "other detected settings stay merged",
                "use --provider <id> to choose the active provider",
            ],
            "; ",
            width,
        ));
    }
    lines
}
