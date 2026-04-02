use std::path::PathBuf;
use std::pin::Pin;

use loongclaw_app as mvp;
use loongclaw_app::chat::{TuiBootFlow, TuiBootScreen, TuiBootTransition};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstRunStep {
    Risk,
    Model,
    CredentialEnv,
    MemoryProfile,
    SqlitePath,
    FileRoot,
    AcpEnabled,
    AcpBackend,
    Preflight,
    Review,
    WriteConfirm,
    Success,
}

#[derive(Debug, Clone)]
pub(crate) struct FirstRunFullscreenBootFlow {
    step: FirstRunStep,
    draft: OnboardDraft,
    output_path: PathBuf,
    checks: Vec<OnboardCheck>,
    success_lines: Vec<String>,
}

impl FirstRunFullscreenBootFlow {
    pub(crate) fn new(output_path: PathBuf) -> Self {
        let context = OnboardRuntimeContext::capture();
        let config = mvp::config::LoongClawConfig::default();
        let mut draft = OnboardDraft::from_config(config, output_path.clone(), None);
        let workspace_values = onboard_workspace::derive_workspace_step_values(&draft, &context);
        onboard_workspace::apply_workspace_step_values(&mut draft, &workspace_values);

        Self {
            step: FirstRunStep::Risk,
            draft,
            output_path,
            checks: Vec::new(),
            success_lines: Vec::new(),
        }
    }

    fn render_screen(&self, width: usize) -> TuiBootScreen {
        match self.step {
            FirstRunStep::Risk => TuiBootScreen {
                lines: screens::render_onboarding_risk_screen_lines(width),
                prompt_hint: " Type y to continue | Enter keeps the safe default n ".to_owned(),
                initial_value: String::new(),
                escape_submit: Some("n".to_owned()),
            },
            FirstRunStep::Model => TuiBootScreen {
                lines: screens::render_model_selection_screen_lines(&self.draft.config, width),
                prompt_hint: " Enter keeps the current model | Esc goes back ".to_owned(),
                initial_value: self.draft.config.provider.model.clone(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::CredentialEnv => {
                let default_env = preferred_api_key_env_default(&self.draft.config);

                TuiBootScreen {
                    lines: screens::render_api_key_env_selection_screen_lines(
                        &self.draft.config,
                        default_env.as_str(),
                        width,
                    ),
                    prompt_hint: " Enter keeps the suggested credential source | Esc goes back "
                        .to_owned(),
                    initial_value: default_env,
                    escape_submit: Some("back".to_owned()),
                }
            }
            FirstRunStep::MemoryProfile => TuiBootScreen {
                lines: screens::render_memory_profile_selection_screen_lines(
                    &self.draft.config,
                    width,
                ),
                prompt_hint:
                    " Type 1/2/3 or a profile id | Enter keeps the default | Esc goes back "
                        .to_owned(),
                initial_value: String::new(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::SqlitePath => TuiBootScreen {
                lines: render_path_input_lines(
                    width,
                    "choose sqlite memory path",
                    &self.draft.workspace.sqlite_path,
                    "where LoongClaw should store local conversation state",
                ),
                prompt_hint: " Enter keeps the current path | Esc goes back ".to_owned(),
                initial_value: self.draft.workspace.sqlite_path.display().to_string(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::FileRoot => TuiBootScreen {
                lines: render_path_input_lines(
                    width,
                    "choose workspace root",
                    &self.draft.workspace.file_root,
                    "which files local tools should be allowed to inspect",
                ),
                prompt_hint: " Enter keeps the current root | Esc goes back ".to_owned(),
                initial_value: self.draft.workspace.file_root.display().to_string(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::AcpEnabled => TuiBootScreen {
                lines: render_choice_lines(
                    width,
                    "choose protocol support",
                    vec![
                        SelectOption {
                            label: "Enable ACP".to_owned(),
                            slug: "enabled".to_owned(),
                            description: "allow protocol-driven tool dispatch and integrations"
                                .to_owned(),
                            recommended: self.draft.protocols.acp_enabled,
                        },
                        SelectOption {
                            label: "Disable ACP".to_owned(),
                            slug: "disabled".to_owned(),
                            description: "keep the first-run setup narrower and simpler".to_owned(),
                            recommended: !self.draft.protocols.acp_enabled,
                        },
                    ],
                    Some(if self.draft.protocols.acp_enabled {
                        0
                    } else {
                        1
                    }),
                    vec![format!(
                        "- current ACP state: {}",
                        if self.draft.protocols.acp_enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    )],
                ),
                prompt_hint:
                    " Type 1/2 or enabled/disabled | Enter keeps the default | Esc goes back "
                        .to_owned(),
                initial_value: String::new(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::AcpBackend => {
                let backend_options = self.backend_options();
                let default_index = self.default_backend_index(backend_options.as_slice());
                let current_backend = self
                    .draft
                    .protocols
                    .acp_backend
                    .clone()
                    .unwrap_or_else(|| "not configured".to_owned());

                TuiBootScreen {
                    lines: render_choice_lines(
                        width,
                        "choose ACP backend",
                        backend_options,
                        default_index,
                        vec![format!("- current backend: {current_backend}")],
                    ),
                    prompt_hint:
                        " Type a number or backend id | Enter keeps the default | Esc goes back "
                            .to_owned(),
                    initial_value: String::new(),
                    escape_submit: Some("back".to_owned()),
                }
            }
            FirstRunStep::Preflight => {
                let has_failures = self
                    .checks
                    .iter()
                    .any(|check| check.level == OnboardCheckLevel::Fail);
                let prompt_hint = if has_failures {
                    " Type back to revise the failing settings | Enter also goes back "
                } else {
                    " Enter continues to review | Type back to revise settings "
                };

                TuiBootScreen {
                    lines: render_preflight_summary_screen_lines(&self.checks, width),
                    prompt_hint: prompt_hint.to_owned(),
                    initial_value: String::new(),
                    escape_submit: Some("back".to_owned()),
                }
            }
            FirstRunStep::Review => TuiBootScreen {
                lines: render_review_lines(width, &self.draft),
                prompt_hint: " Enter continues to write the config | Type back to revise "
                    .to_owned(),
                initial_value: String::new(),
                escape_submit: Some("back".to_owned()),
            },
            FirstRunStep::WriteConfirm => TuiBootScreen {
                lines: render_write_confirmation_screen_lines(
                    self.output_path.display().to_string().as_str(),
                    false,
                    width,
                ),
                prompt_hint: " Type y to write now | Type n to go back ".to_owned(),
                initial_value: String::new(),
                escape_submit: Some("n".to_owned()),
            },
            FirstRunStep::Success => TuiBootScreen {
                lines: self.success_lines.clone(),
                prompt_hint: " Enter starts chat now | Type exit to leave setup ".to_owned(),
                initial_value: String::new(),
                escape_submit: Some("exit".to_owned()),
            },
        }
    }

    fn backend_options(&self) -> Vec<SelectOption> {
        let backends = onboard_protocols::list_available_acp_backends().unwrap_or_default();

        backends
            .into_iter()
            .map(|backend| SelectOption {
                label: backend.id.clone(),
                slug: backend.id,
                description: backend.summary,
                recommended: false,
            })
            .collect()
    }

    fn default_backend_index(&self, options: &[SelectOption]) -> Option<usize> {
        let current_backend = self.draft.protocols.acp_backend.clone();

        current_backend.and_then(|backend| options.iter().position(|option| option.slug == backend))
    }

    fn preflight_recovery_step(&self) -> FirstRunStep {
        if self.checks.iter().any(|check| {
            check.name == "provider credentials" && check.level == OnboardCheckLevel::Fail
        }) {
            return FirstRunStep::CredentialEnv;
        }

        if self
            .checks
            .iter()
            .any(|check| check.name == "memory path" || check.name == "tool file root")
        {
            return FirstRunStep::SqlitePath;
        }

        FirstRunStep::CredentialEnv
    }

    async fn enter_preflight_step(&mut self) {
        self.step = FirstRunStep::Preflight;
        self.checks = run_preflight_checks(&self.draft.config, true).await;
    }

    async fn submit_inner(&mut self, input: String, width: usize) -> CliResult<TuiBootTransition> {
        let trimmed = input.trim();

        match self.step {
            FirstRunStep::Risk => {
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("n") {
                    return Ok(TuiBootTransition::Exit);
                }

                if !trimmed.eq_ignore_ascii_case("y") {
                    return self.invalid_input(width, "enter y to continue or n to cancel");
                }

                self.step = FirstRunStep::Model;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::Model => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::Risk;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let resolved_model = if trimmed.is_empty() {
                    self.draft.config.provider.model.clone()
                } else {
                    trimmed.to_owned()
                };
                self.draft.set_provider_model(resolved_model);
                self.step = FirstRunStep::CredentialEnv;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::CredentialEnv => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::Model;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let default_env = preferred_api_key_env_default(&self.draft.config);
                let resolved_env = if trimmed.is_empty() {
                    default_env
                } else {
                    trimmed.to_owned()
                };
                self.draft.set_provider_credential_env(resolved_env);
                self.step = FirstRunStep::MemoryProfile;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::MemoryProfile => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::CredentialEnv;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let options = memory_profile_options();
                let selected_index = if trimmed.is_empty() {
                    MEMORY_PROFILE_CHOICES
                        .iter()
                        .position(|entry| entry.0 == self.draft.config.memory.profile)
                        .unwrap_or(0)
                } else {
                    match parse_select_one_input(trimmed, options.as_slice()) {
                        Some(index) => index,
                        None => {
                            let message =
                                render_select_one_invalid_input_message(options.as_slice());
                            return self.invalid_input(width, message.as_str());
                        }
                    }
                };

                let selected_profile = MEMORY_PROFILE_CHOICES
                    .get(selected_index)
                    .map(|entry| entry.0)
                    .ok_or_else(|| "invalid memory profile selection".to_owned())?;
                self.draft.set_memory_profile(selected_profile);
                self.step = FirstRunStep::SqlitePath;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::SqlitePath => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::MemoryProfile;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let resolved_path = if trimmed.is_empty() {
                    self.draft.workspace.sqlite_path.clone()
                } else {
                    PathBuf::from(trimmed)
                };
                self.draft.set_workspace_sqlite_path(resolved_path);
                self.step = FirstRunStep::FileRoot;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::FileRoot => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::SqlitePath;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let resolved_root = if trimmed.is_empty() {
                    self.draft.workspace.file_root.clone()
                } else {
                    PathBuf::from(trimmed)
                };
                self.draft.set_workspace_file_root(resolved_root);
                self.step = FirstRunStep::AcpEnabled;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::AcpEnabled => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::FileRoot;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let enabled = if trimmed.is_empty() {
                    self.draft.protocols.acp_enabled
                } else if trimmed.eq_ignore_ascii_case("1")
                    || trimmed.eq_ignore_ascii_case("enabled")
                {
                    true
                } else if trimmed.eq_ignore_ascii_case("2")
                    || trimmed.eq_ignore_ascii_case("disabled")
                {
                    false
                } else {
                    return self.invalid_input(width, "enter 1/enabled or 2/disabled");
                };

                self.draft.set_acp_enabled(enabled);
                if enabled {
                    self.step = FirstRunStep::AcpBackend;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                self.enter_preflight_step().await;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::AcpBackend => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::AcpEnabled;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                let options = self.backend_options();
                let selected_index = if trimmed.is_empty() {
                    self.default_backend_index(options.as_slice()).unwrap_or(0)
                } else {
                    match parse_select_one_input(trimmed, options.as_slice()) {
                        Some(index) => index,
                        None => {
                            let message =
                                render_select_one_invalid_input_message(options.as_slice());
                            return self.invalid_input(width, message.as_str());
                        }
                    }
                };
                let selected_backend = options
                    .get(selected_index)
                    .map(|option| option.slug.clone())
                    .ok_or_else(|| "invalid ACP backend selection".to_owned())?;
                self.draft.set_acp_backend(Some(selected_backend));
                self.enter_preflight_step().await;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::Preflight => {
                let has_failures = self
                    .checks
                    .iter()
                    .any(|check| check.level == OnboardCheckLevel::Fail);

                if trimmed.eq_ignore_ascii_case("back") || (trimmed.is_empty() && has_failures) {
                    self.step = self.preflight_recovery_step();
                    self.checks.clear();
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                if has_failures {
                    return self.invalid_input(
                        width,
                        "preflight found blocking issues; type back to revise the setup",
                    );
                }

                self.step = FirstRunStep::Review;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::Review => {
                if trimmed.eq_ignore_ascii_case("back") {
                    self.step = FirstRunStep::CredentialEnv;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                self.step = FirstRunStep::WriteConfirm;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::WriteConfirm => {
                if trimmed.eq_ignore_ascii_case("back") || trimmed.eq_ignore_ascii_case("n") {
                    self.step = FirstRunStep::Review;
                    return Ok(TuiBootTransition::Screen(self.render_screen(width)));
                }

                if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("y") {
                    return self
                        .invalid_input(width, "enter y to write the config or n to go back");
                }

                let output_path_string = self.output_path.to_string_lossy().into_owned();
                mvp::config::write(Some(output_path_string.as_str()), &self.draft.config, false)?;

                let summary = build_onboarding_success_summary(
                    self.output_path.as_path(),
                    &self.draft.config,
                    None,
                );
                self.success_lines =
                    render_onboarding_success_summary_lines(&summary, width, false);
                self.step = FirstRunStep::Success;
                Ok(TuiBootTransition::Screen(self.render_screen(width)))
            }
            FirstRunStep::Success => {
                if trimmed.eq_ignore_ascii_case("exit") {
                    return Ok(TuiBootTransition::Exit);
                }

                Ok(TuiBootTransition::StartChat {
                    system_message: Some("Setup complete. Entering chat.".to_owned()),
                })
            }
        }
    }

    fn invalid_input(&self, width: usize, message: &str) -> CliResult<TuiBootTransition> {
        let mut screen = self.render_screen(width);
        screen.lines.push(String::new());
        screen.lines.push(format!("invalid input: {message}"));
        Ok(TuiBootTransition::Screen(screen))
    }
}

pub(crate) fn build_first_run_fullscreen_boot_flow(output: Option<String>) -> Box<dyn TuiBootFlow> {
    let output_path = output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let flow = FirstRunFullscreenBootFlow::new(output_path);
    Box::new(flow)
}

impl TuiBootFlow for FirstRunFullscreenBootFlow {
    fn begin(&mut self, width: usize) -> CliResult<TuiBootScreen> {
        Ok(self.render_screen(width))
    }

    fn submit<'a>(
        &'a mut self,
        input: String,
        width: usize,
    ) -> Pin<Box<dyn std::future::Future<Output = CliResult<TuiBootTransition>> + Send + 'a>> {
        let future = self.submit_inner(input, width);
        Box::pin(future)
    }
}

fn memory_profile_options() -> Vec<SelectOption> {
    MEMORY_PROFILE_CHOICES
        .into_iter()
        .map(|(profile, label, detail)| SelectOption {
            label: label.to_owned(),
            slug: profile.as_str().to_owned(),
            description: detail.to_owned(),
            recommended: false,
        })
        .collect()
}

fn render_path_input_lines(
    width: usize,
    title: &str,
    current_path: &std::path::Path,
    description: &str,
) -> Vec<String> {
    let spec = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("first-run setup".to_owned()),
        title: Some(title.to_owned()),
        progress_line: None,
        intro_lines: vec![
            format!("- current value: {}", current_path.display()),
            format!("- purpose: {description}"),
        ],
        sections: Vec::new(),
        choices: Vec::new(),
        footer_lines: vec![
            "- edit the value in the composer below, then press Enter".to_owned(),
            "- press Esc to go back".to_owned(),
        ],
    };

    mvp::tui_surface::render_tui_screen_spec(&spec, width, false)
}

fn render_choice_lines(
    width: usize,
    title: &str,
    options: Vec<SelectOption>,
    default_index: Option<usize>,
    intro_lines: Vec<String>,
) -> Vec<String> {
    let screen_options = options
        .iter()
        .enumerate()
        .map(|(index, option)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: option.label.clone(),
            detail_lines: vec![option.description.clone()],
            recommended: default_index == Some(index),
        })
        .collect::<Vec<_>>();

    screens::render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "first-run setup",
        title,
        None,
        intro_lines,
        screen_options,
        Vec::new(),
        true,
        false,
    )
}

fn render_review_lines(width: usize, draft: &OnboardDraft) -> Vec<String> {
    let review_lines = screens::build_onboard_review_digest_display_lines_for_draft(draft);
    let spec = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("first-run setup".to_owned()),
        title: Some("review setup".to_owned()),
        progress_line: None,
        intro_lines: vec!["review the values that will be written before first chat".to_owned()],
        sections: vec![mvp::tui_surface::TuiSectionSpec::Narrative {
            title: Some("draft".to_owned()),
            lines: review_lines,
        }],
        choices: Vec::new(),
        footer_lines: vec![
            "- press Enter to continue to write confirmation".to_owned(),
            "- type back or press Esc to revise settings".to_owned(),
        ],
    };

    mvp::tui_surface::render_tui_screen_spec(&spec, width, false)
}
