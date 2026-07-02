use super::*;

pub(super) fn resolve_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<mvp::config::ProviderConfig> {
    if options.non_interactive {
        if let Some(provider_raw) = options.provider.as_deref() {
            return resolve_provider_config_from_selector(
                &config.provider,
                provider_selection,
                provider_raw,
            );
        }
        if provider_selection.requires_explicit_choice {
            let detected = provider_selection
                .imported_choices
                .iter()
                .map(|choice| choice.profile_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "multiple detected provider choices found ({detected}); rerun with --provider {} to choose the active provider",
                crate::migration::provider_selection::PROVIDER_SELECTOR_PLACEHOLDER,
            ));
        }
        if let Some(default_profile_id) = provider_selection.default_profile_id.as_deref() {
            return resolve_provider_config_from_selector(
                &config.provider,
                provider_selection,
                default_profile_id,
            );
        }
        return Ok(crate::migration::resolve_provider_config_from_selection(
            &config.provider,
            provider_selection,
            provider_selection
                .default_kind
                .unwrap_or(config.provider.kind),
        ));
    }

    if !provider_selection.imported_choices.is_empty() {
        let select_options: Vec<SelectOption> = provider_selection
            .imported_choices
            .iter()
            .map(|choice| SelectOption {
                label: provider_kind_display_name(choice.kind).to_owned(),
                slug: choice.profile_id.clone(),
                description: format!("source: {}, summary: {}", choice.source, choice.summary),
                recommended: Some(choice.profile_id.as_str())
                    == provider_selection.default_profile_id.as_deref(),
            })
            .collect();
        let default_idx = if provider_selection.requires_explicit_choice {
            None
        } else {
            provider_selection
                .default_profile_id
                .as_deref()
                .and_then(|default_id| {
                    provider_selection
                        .imported_choices
                        .iter()
                        .position(|choice| choice.profile_id == default_id)
                })
        };
        print_lines(
            ui,
            render_provider_selection_header_lines(
                provider_selection,
                guided_prompt_path,
                context.render_width,
            ),
        )?;
        let idx = ui.select_one(
            "Provider",
            &select_options,
            default_idx,
            SelectInteractionMode::List,
        )?;
        let choice = provider_selection
            .imported_choices
            .get(idx)
            .ok_or_else(|| format!("provider selection index {idx} out of range"))?;
        return Ok(choice.config.clone());
    }

    // No imported choices — still use the numbered chooser so the provider
    // step stays aligned with the rest of onboarding.
    let default_provider_kind = options
        .provider
        .as_deref()
        .and_then(parse_provider_kind)
        .or(provider_selection.default_kind)
        .or_else(|| {
            provider_selection
                .default_profile_id
                .as_deref()
                .and_then(parse_provider_kind)
        })
        .unwrap_or(config.provider.kind);
    let provider_kinds = mvp::config::ProviderKind::all_sorted()
        .iter()
        .copied()
        .filter(|kind| {
            *kind != mvp::config::ProviderKind::Kimi
                && *kind != mvp::config::ProviderKind::KimiCoding
                && *kind != mvp::config::ProviderKind::Stepfun
                && *kind != mvp::config::ProviderKind::StepPlan
        })
        .collect::<Vec<_>>();
    let mut select_options: Vec<SelectOption> = provider_kinds
        .iter()
        .map(|kind| SelectOption {
            label: provider_kind_display_name(*kind).to_owned(),
            slug: provider_kind_id(*kind).to_owned(),
            description: String::new(),
            recommended: *kind == default_provider_kind,
        })
        .collect();
    select_options.push(SelectOption {
        label: "Kimi".to_owned(),
        slug: "kimi".to_owned(),
        description: "Kimi API or Kimi Coding".to_owned(),
        recommended: default_provider_kind == mvp::config::ProviderKind::Kimi
            || default_provider_kind == mvp::config::ProviderKind::KimiCoding,
    });
    select_options.push(SelectOption {
        label: "Stepfun".to_owned(),
        slug: "stepfun".to_owned(),
        description: "Stepfun API or Step Plan".to_owned(),
        recommended: default_provider_kind == mvp::config::ProviderKind::Stepfun
            || default_provider_kind == mvp::config::ProviderKind::StepPlan,
    });
    select_options.sort_by(|a, b| a.label.cmp(&b.label));
    let default_provider_slug = if matches!(
        default_provider_kind,
        mvp::config::ProviderKind::Kimi | mvp::config::ProviderKind::KimiCoding
    ) {
        "kimi"
    } else if matches!(
        default_provider_kind,
        mvp::config::ProviderKind::Stepfun | mvp::config::ProviderKind::StepPlan
    ) {
        "stepfun"
    } else {
        provider_kind_id(default_provider_kind)
    };
    let default_idx = if provider_selection.requires_explicit_choice {
        None
    } else {
        select_options
            .iter()
            .position(|option| option.slug == default_provider_slug)
    };
    print_lines(
        ui,
        render_provider_selection_header_lines(
            provider_selection,
            guided_prompt_path,
            context.render_width,
        ),
    )?;
    let idx = ui.select_one(
        "Provider",
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let selected_slug = select_options
        .get(idx)
        .ok_or_else(|| format!("provider selection index {idx} out of range"))?
        .slug
        .clone();

    let kind: mvp::config::ProviderKind = if selected_slug == "kimi" {
        let kimi_options = vec![
            SelectOption {
                label: "Kimi API".to_owned(),
                slug: "kimi_api".to_owned(),
                description: "Standard Kimi chat completion API".to_owned(),
                recommended: true,
            },
            SelectOption {
                label: "Kimi Coding".to_owned(),
                slug: "kimi_coding".to_owned(),
                description: "Kimi for coding tasks".to_owned(),
                recommended: false,
            },
        ];
        print_lines(ui, vec!["Select the Kimi variant:".to_owned()])?;
        let kimi_default_idx = Some(usize::from(
            default_provider_kind == mvp::config::ProviderKind::KimiCoding,
        ));
        let sub_idx = ui.select_one(
            "Kimi variant",
            &kimi_options,
            kimi_default_idx,
            SelectInteractionMode::List,
        )?;
        let sub_slug = kimi_options
            .get(sub_idx)
            .ok_or_else(|| format!("kimi variant index {sub_idx} out of range"))?
            .slug
            .clone();
        if sub_slug == "kimi_coding" {
            mvp::config::ProviderKind::KimiCoding
        } else {
            mvp::config::ProviderKind::Kimi
        }
    } else if selected_slug == "stepfun" {
        let stepfun_options = vec![
            SelectOption {
                label: "Stepfun API".to_owned(),
                slug: "stepfun_api".to_owned(),
                description: "Standard Stepfun chat completion API".to_owned(),
                recommended: true,
            },
            SelectOption {
                label: "Step Plan".to_owned(),
                slug: "step_plan".to_owned(),
                description: "Step Plan for specialized tasks".to_owned(),
                recommended: false,
            },
        ];
        print_lines(ui, vec!["Select the Stepfun variant:".to_owned()])?;
        let stepfun_default_idx = Some(usize::from(
            default_provider_kind == mvp::config::ProviderKind::StepPlan,
        ));
        let sub_idx = ui.select_one(
            "Stepfun variant",
            &stepfun_options,
            stepfun_default_idx,
            SelectInteractionMode::List,
        )?;
        let sub_slug = stepfun_options
            .get(sub_idx)
            .ok_or_else(|| format!("stepfun variant index {sub_idx} out of range"))?
            .slug
            .clone();
        if sub_slug == "step_plan" {
            mvp::config::ProviderKind::StepPlan
        } else {
            mvp::config::ProviderKind::Stepfun
        }
    } else {
        provider_kinds
            .iter()
            .find(|kind| provider_kind_id(**kind) == selected_slug)
            .copied()
            .ok_or_else(|| format!("provider kind not found for slug {}", selected_slug))?
    };

    let mut provider_config =
        resolve_provider_config_from_selection(&config.provider, provider_selection, kind);

    if let Some(region_info) = kind.region_endpoint_info() {
        let configured_base_url = provider_config.base_url.as_str();
        let default_region_idx = region_info
            .variants
            .iter()
            .position(|variant| variant.base_url == configured_base_url)
            .unwrap_or(0);
        let region_options = region_info
            .variants
            .iter()
            .enumerate()
            .map(|(index, variant)| {
                let is_default_variant = index == 0;
                let label = if is_default_variant {
                    format!("{} (default)", variant.label)
                } else {
                    variant.label.to_owned()
                };
                let slug = variant.base_url.to_owned();
                let description = format!("endpoint: {}", variant.base_url);
                let recommended = index == default_region_idx;
                SelectOption {
                    label,
                    slug,
                    description,
                    recommended,
                }
            })
            .collect::<Vec<_>>();
        let region_prompt = format!("Select the {} region endpoint:", region_info.family_label);
        print_lines(ui, vec![region_prompt])?;
        let region_idx = ui.select_one(
            "Region",
            &region_options,
            Some(default_region_idx),
            SelectInteractionMode::List,
        )?;
        let selected_base_url = region_options
            .get(region_idx)
            .ok_or_else(|| format!("region selection index {region_idx} out of range"))?
            .slug
            .clone();
        provider_config.set_base_url(selected_base_url);
    }

    prompt_provider_base_url_if_needed(options, kind, &mut provider_config, ui)?;

    Ok(provider_config)
}

pub(super) fn prompt_provider_base_url_if_needed(
    options: &OnboardCommandOptions,
    kind: mvp::config::ProviderKind,
    provider_config: &mut mvp::config::ProviderConfig,
    ui: &mut impl OnboardUi,
) -> CliResult<()> {
    let requires_custom_base_url = kind.requires_custom_base_url();
    if !requires_custom_base_url || options.non_interactive {
        return Ok(());
    }

    let configured_base_url = provider_config.base_url.trim().to_owned();
    let has_configured_base_url = !configured_base_url.is_empty();
    let has_unresolved_custom_base_url = provider_config.has_unresolved_custom_base_url();
    let prompt_lines = build_provider_base_url_prompt_lines(
        kind,
        configured_base_url.as_str(),
        has_unresolved_custom_base_url,
    );
    print_lines(ui, prompt_lines)?;

    let selected_base_url = if has_unresolved_custom_base_url || !has_configured_base_url {
        ui.prompt_required("Provider base URL")?
    } else {
        ui.prompt_with_default("Provider base URL", configured_base_url.as_str())?
    };
    let validated_base_url = validate_onboard_provider_base_url(selected_base_url.as_str())?;
    provider_config.set_base_url(validated_base_url);

    Ok(())
}

pub(super) fn build_provider_base_url_prompt_lines(
    kind: mvp::config::ProviderKind,
    configured_base_url: &str,
    has_unresolved_custom_base_url: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    let provider_label = provider_kind_display_name(kind);
    let intro_line = format!("Set the {} API base URL:", provider_label);
    lines.push(intro_line);

    if let Some(configuration_hint) = kind.configuration_hint() {
        lines.push(configuration_hint.to_owned());
    }

    if has_unresolved_custom_base_url {
        let template_line = format!("Current template: {}", configured_base_url.trim());
        lines.push(template_line);
    }

    lines
}

pub(super) fn validate_onboard_provider_base_url(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("provider base URL cannot be empty".to_owned());
    }

    let parsed_url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("provider base URL is invalid: {error}"))?;
    let scheme = parsed_url.scheme();
    let valid_scheme = scheme == "http" || scheme == "https";
    if !valid_scheme {
        return Err("provider base URL must use http or https".to_owned());
    }

    let has_host = parsed_url.host_str().is_some();
    if !has_host {
        return Err("provider base URL must include a host".to_owned());
    }

    Ok(trimmed.to_owned())
}

pub fn resolve_provider_config_from_selector(
    current_provider: &mvp::config::ProviderConfig,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    selector: &str,
) -> CliResult<mvp::config::ProviderConfig> {
    match crate::migration::resolve_choice_by_selector_resolution(provider_selection, selector) {
        crate::migration::ImportedChoiceSelectorResolution::Match(profile_id) => {
            let Some(choice) = provider_selection
                .imported_choices
                .iter()
                .find(|choice| choice.profile_id == profile_id)
            else {
                return Err(format!(
                    "provider selection plan is inconsistent: resolved profile `{profile_id}` is missing"
                ));
            };
            return Ok(choice.config.clone());
        }
        crate::migration::ImportedChoiceSelectorResolution::Ambiguous(profile_ids) => {
            return Err(crate::migration::format_ambiguous_selector_error(
                provider_selection,
                selector,
                &profile_ids,
            ));
        }
        crate::migration::ImportedChoiceSelectorResolution::NoMatch => {}
    }

    let kind = parse_provider_kind(selector).ok_or_else(|| {
        if provider_selection.imported_choices.is_empty() {
            return format!(
                "unsupported provider value \"{selector}\". accepted selectors: {}. {}",
                supported_provider_list(),
                crate::migration::provider_selection::PROVIDER_SELECTOR_NOTE,
            );
        }
        crate::migration::format_unknown_selector_error(
            provider_selection,
            format!("unsupported provider value \"{selector}\"").as_str(),
        )
    })?;
    let matching_choices = provider_selection
        .imported_choices
        .iter()
        .filter(|choice| choice.kind == kind)
        .collect::<Vec<_>>();
    if matching_choices.len() > 1 {
        let profile_ids = matching_choices
            .iter()
            .map(|choice| choice.profile_id.clone())
            .collect::<Vec<_>>();
        return Err(crate::migration::format_ambiguous_selector_error(
            provider_selection,
            selector,
            &profile_ids,
        ));
    }
    if let Some(choice) = matching_choices.first() {
        return Ok(choice.config.clone());
    }
    Ok(crate::migration::resolve_provider_config_from_selection(
        current_provider,
        provider_selection,
        kind,
    ))
}

pub fn build_provider_selection_plan_for_candidate(
    selected_candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
) -> crate::migration::ProviderSelectionPlan {
    let migration_selected = migration_candidate_from_onboard(selected_candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    crate::migration::build_provider_selection_plan_for_candidate(
        &migration_selected,
        &migration_candidates,
    )
}

pub fn resolve_provider_config_from_selection(
    current_provider: &mvp::config::ProviderConfig,
    plan: &crate::migration::ProviderSelectionPlan,
    selected_kind: mvp::config::ProviderKind,
) -> mvp::config::ProviderConfig {
    crate::migration::resolve_provider_config_from_selection(current_provider, plan, selected_kind)
}
