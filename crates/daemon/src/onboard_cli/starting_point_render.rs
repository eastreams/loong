use super::*;

pub fn render_single_detected_setup_preview_screen_lines(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_single_detected_setup_preview_screen_lines_with_style(
        candidate,
        all_candidates,
        width,
        false,
    )
}

pub(super) fn render_single_detected_setup_preview_screen_lines_with_style(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    let provider_selection = crate::migration::build_provider_selection_plan_for_candidate(
        &migration_candidate,
        &migration_candidates,
    );
    let mut intro_lines = Vec::new();
    if let Some(reason_line) =
        format_starting_point_reason(&collect_starting_point_fit_hints(candidate))
    {
        intro_lines.push(reason_line);
    }
    let preview_candidate = migration_candidate_for_onboard_display(candidate);
    let preview_lines =
        crate::migration::render::candidate_preview_display_lines(&preview_candidate);
    intro_lines.extend(preview_lines);

    let provider_selection_lines =
        crate::migration::render::provider_selection_display_lines(&provider_selection);
    intro_lines.extend(provider_selection_lines);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard::presentation::single_detected_starting_point_preview_subtitle(),
        crate::onboard::presentation::single_detected_starting_point_preview_title(),
        None,
        intro_lines,
        Vec::new(),
        vec![
            crate::onboard::presentation::single_detected_starting_point_preview_footer()
                .to_owned(),
        ],
        false,
        color_enabled,
    )
}

fn push_starting_point_fit_hint(
    hints: &mut Vec<StartingPointFitHint>,
    seen: &mut std::collections::BTreeSet<&'static str>,
    key: &'static str,
    detail: impl Into<String>,
    domain: Option<crate::migration::SetupDomainKind>,
) {
    if seen.insert(key) {
        hints.push(StartingPointFitHint {
            key,
            detail: detail.into(),
            domain,
        });
    }
}

fn summarize_direct_starting_point_source_reason(
    candidate: &ImportCandidate,
) -> Option<&'static str> {
    candidate.source_kind.direct_starting_point_reason()
}

fn collect_starting_point_fit_hints(candidate: &ImportCandidate) -> Vec<StartingPointFitHint> {
    let mut hints = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(reason) = summarize_direct_starting_point_source_reason(candidate) {
        push_starting_point_fit_hint(&mut hints, &mut seen, "direct_source", reason, None);
    } else if let Some(provider_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Provider)
        && let Some(decision) = provider_domain.decision
        && let Some(reason) = provider_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::KeepCurrent => "provider_keep",
            crate::migration::types::PreviewDecision::UseDetected => "provider_detected",
            crate::migration::types::PreviewDecision::Supplement
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "provider",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Provider),
        );
    }

    if let Some(channels_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Channels)
        && let Some(decision) = channels_domain.decision
        && let Some(reason) = channels_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::Supplement => "channels_add",
            crate::migration::types::PreviewDecision::UseDetected => "channels_detected",
            crate::migration::types::PreviewDecision::KeepCurrent
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "channels",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    } else if !candidate.channel_candidates.is_empty()
        && let Some(reason) = crate::migration::SetupDomainKind::Channels
            .starting_point_reason(crate::migration::types::PreviewDecision::Supplement)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "channels_add",
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    }

    if (!candidate.workspace_guidance.is_empty()
        || candidate.domains.iter().any(|domain| {
            domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }))
        && let Some(reason) = crate::migration::SetupDomainKind::WorkspaceGuidance
            .starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "workspace_guidance",
            reason,
            Some(crate::migration::SetupDomainKind::WorkspaceGuidance),
        );
    }

    for (kind, key) in [
        (crate::migration::SetupDomainKind::Cli, "cli"),
        (crate::migration::SetupDomainKind::Memory, "memory"),
        (crate::migration::SetupDomainKind::Tools, "tools"),
    ] {
        if hints.len() >= 3 {
            break;
        }
        if candidate.domains.iter().any(|domain| {
            domain.kind == kind
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }) && let Some(reason) =
            kind.starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
        {
            push_starting_point_fit_hint(&mut hints, &mut seen, key, reason, Some(kind));
        }
    }

    if hints.is_empty() {
        let source_count = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(candidate),
        )
        .len();
        if source_count > 1 {
            push_starting_point_fit_hint(
                &mut hints,
                &mut seen,
                "combined_sources",
                format!("combine {source_count} reusable sources"),
                None,
            );
        }
    }

    hints
}

fn format_starting_point_reason(hints: &[StartingPointFitHint]) -> Option<String> {
    if hints.is_empty() {
        return None;
    }

    Some(format!(
        "good fit: {}",
        hints
            .iter()
            .take(3)
            .map(|hint| hint.detail.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    ))
}

fn should_include_starting_point_domain_decision(candidate: &ImportCandidate) -> bool {
    candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
}

fn format_starting_point_domain_detail(
    candidate: &ImportCandidate,
    domain: &crate::migration::DomainPreview,
) -> String {
    let mut detail = format!("{}: ", domain.kind.label());
    if should_include_starting_point_domain_decision(candidate)
        && let Some(decision) = domain.decision
    {
        detail.push_str(decision.label());
        detail.push_str(" · ");
    }
    detail.push_str(&domain.summary);
    detail
}

pub(super) fn summarize_starting_point_detail_lines(
    candidate: &ImportCandidate,
    width: usize,
) -> Vec<String> {
    let mut details = Vec::new();
    let max_lines = if width < 68 { 4 } else { 5 };
    let mut detail_lines_used = 0usize;
    let has_channel_details = !candidate.channel_candidates.is_empty();
    let has_workspace_guidance_details = !candidate.workspace_guidance.is_empty();
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let fit_hints = collect_starting_point_fit_hints(candidate);
    let emphasized_domains = if width < 68 {
        fit_hints
            .iter()
            .filter_map(|hint| hint.domain)
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };

    if let Some(reason_line) = format_starting_point_reason(&fit_hints) {
        details.push(reason_line);
    }

    let mut source_labels =
        crate::migration::render::candidate_source_rollup_labels(&migration_candidate);
    if has_workspace_guidance_details {
        source_labels.retain(|label| label != "workspace guidance");
    }
    let should_render_source_summary =
        if candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan {
            !source_labels.is_empty()
        } else {
            source_labels.len() > 1
        };
    if should_render_source_summary {
        details.push(format!("sources: {}", source_labels.join(" + ")));
        detail_lines_used += 1;
    }

    for domain in &candidate.domains {
        if has_channel_details && domain.kind == crate::migration::SetupDomainKind::Channels {
            continue;
        }
        if has_workspace_guidance_details
            && domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
        {
            continue;
        }
        if emphasized_domains.contains(&domain.kind) {
            continue;
        }
        details.push(format_starting_point_domain_detail(candidate, domain));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    for channel in &candidate.channel_candidates {
        details.push(format!(
            "{}: {}",
            channel.label.to_ascii_lowercase(),
            channel.summary
        ));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    if details.len() < max_lines && !candidate.workspace_guidance.is_empty() {
        let files = candidate
            .workspace_guidance
            .iter()
            .filter_map(|guidance| Path::new(&guidance.path).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if !files.is_empty() {
            details.push(format!("workspace guidance: {}", files.join(", ")));
        }
    }

    if details.is_empty() {
        details.push("ready to use as a starting point".to_owned());
    }

    details
}

pub(super) fn start_fresh_starting_point_detail_lines() -> Vec<String> {
    vec![
        crate::onboard::presentation::start_fresh_starting_point_fit_line().to_owned(),
        crate::onboard::presentation::start_fresh_starting_point_detail_line().to_owned(),
    ]
}

fn render_starting_point_selection_footer_lines(
    sorted_candidates: &[ImportCandidate],
) -> Vec<String> {
    let Some(first_candidate) = sorted_candidates.first() else {
        return Vec::new();
    };

    let first_hint = render_default_choice_footer_line(
        "1",
        crate::onboard::presentation::starting_point_footer_description(
            first_candidate.source_kind,
        ),
    );

    vec![first_hint]
}

pub fn render_starting_point_selection_screen_lines(
    candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_starting_point_selection_screen_lines_with_style(candidates, width, false)
}

fn render_starting_point_selection_screen_lines_with_style(
    candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let sorted_candidates = sort_starting_point_candidates(candidates.to_vec());
    let mut options = sorted_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: onboard_starting_point_label(Some(candidate.source_kind), &candidate.source),
            detail_lines: summarize_starting_point_detail_lines(candidate, width),
            recommended: matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::RecommendedPlan
            ),
        })
        .collect::<Vec<_>>();
    options.push(OnboardScreenOption {
        key: "0".to_owned(),
        label: crate::onboard::presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    let footer_lines = render_starting_point_selection_footer_lines(&sorted_candidates);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard::presentation::starting_point_selection_subtitle(),
        crate::onboard::presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard::presentation::starting_point_selection_hint().to_owned()],
        options,
        footer_lines,
        true,
        color_enabled,
    )
}

pub(super) fn render_starting_point_selection_header_lines_with_style(
    _candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard::presentation::starting_point_selection_subtitle(),
        crate::onboard::presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard::presentation::starting_point_selection_hint().to_owned()],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}
