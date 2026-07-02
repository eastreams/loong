use super::*;

pub(super) fn render_preinstalled_skills_selection_screen_lines_with_style(
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = mvp::tools::bundled_preinstall_targets()
        .iter()
        .map(|target| OnboardScreenOption {
            key: target.install_id.to_owned(),
            label: target.display_name.to_owned(),
            detail_lines: vec![target.summary.to_owned()],
            recommended: target.recommended,
        })
        .collect();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "optional add-ons",
        "preinstalled skills",
        None,
        vec![
            "- choose zero or more bundled skills to install into the managed runtime".to_owned(),
            "- type comma-separated ids, for example: find-skills,agent-browser".to_owned(),
        ],
        options,
        vec!["- press Enter to skip".to_owned()],
        true,
        color_enabled,
    )
}

pub(super) fn parse_preinstalled_skill_selection(raw: &str) -> CliResult<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for token in trimmed
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let Some(choice) = mvp::tools::bundled_preinstall_targets()
            .iter()
            .find(|choice| choice.install_id.eq_ignore_ascii_case(token))
        else {
            let supported = mvp::tools::bundled_preinstall_targets()
                .iter()
                .map(|choice| choice.install_id)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "unsupported preinstalled skill selection `{token}`. choose from: {supported}"
            ));
        };
        for skill_id in choice.skill_ids {
            if seen.insert((*skill_id).to_owned()) {
                selected.push((*skill_id).to_owned());
            }
        }
    }
    Ok(selected)
}

pub(super) fn resolve_preinstalled_skill_selection(
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<Vec<String>> {
    if options.non_interactive {
        return Ok(Vec::new());
    }

    print_lines(
        ui,
        render_preinstalled_skills_selection_screen_lines_with_style(context.render_width, true),
    )?;
    let raw = ui.prompt_allow_empty(PREINSTALLED_SKILLS_PROMPT_LABEL)?;
    parse_preinstalled_skill_selection(raw.as_str())
}

pub(super) fn onboarding_default_skills_install_root(output_path: &Path) -> PathBuf {
    let base_dir = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base_dir.join(".loong/skills")
}

pub(super) fn apply_selected_preinstalled_skills_to_config(
    config: &mut mvp::config::LoongConfig,
    output_path: &Path,
    selected_skill_ids: &[String],
) {
    if selected_skill_ids.is_empty() {
        return;
    }
    config.skills.enabled = true;
    config.skills.auto_expose_installed = true;
    if config.skills.install_root.is_none() {
        config.skills.install_root = Some(
            onboarding_default_skills_install_root(output_path)
                .display()
                .to_string(),
        );
    }
}

pub(super) fn install_root_for_onboarded_skills(
    config: &mvp::config::LoongConfig,
    config_path: &Path,
) -> PathBuf {
    config
        .skills
        .resolved_install_root()
        .unwrap_or_else(|| onboarding_default_skills_install_root(config_path))
}

pub(super) fn install_selected_preinstalled_skills(
    config_path: &Path,
    config: &mvp::config::LoongConfig,
    selected_skill_ids: &[String],
) -> CliResult<()> {
    if selected_skill_ids.is_empty() {
        return Ok(());
    }

    let install_root = install_root_for_onboarded_skills(config, config_path);
    let tool_runtime_config =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, Some(config_path));
    let mut installed_now: Vec<String> = Vec::new();

    for skill_id in selected_skill_ids {
        if install_root.join(skill_id).join("SKILL.md").is_file() {
            continue;
        }
        if let Err(error) = mvp::tools::skills_install_with_config(
            None,
            Some(skill_id.as_str()),
            None,
            None,
            false,
            false,
            &tool_runtime_config,
        ) {
            for installed_skill_id in installed_now.iter().rev() {
                let _ =
                    mvp::tools::skills_remove_with_config(installed_skill_id, &tool_runtime_config);
            }
            return Err(format!(
                "failed to install selected bundled skill `{skill_id}`: {error}"
            ));
        }
        installed_now.push(skill_id.clone());
    }

    Ok(())
}
