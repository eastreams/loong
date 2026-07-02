use super::*;

#[derive(Debug, Clone)]
pub struct OnboardCommandOptions {
    pub output: Option<String>,
    pub force: bool,
    pub non_interactive: bool,
    pub accept_risk: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub web_search_provider: Option<String>,
    pub web_search_api_key_env: Option<String>,
    pub personality: Option<String>,
    pub memory_profile: Option<String>,
    pub system_prompt: Option<String>,
    pub skip_model_probe: bool,
}

#[derive(Debug, Clone)]
pub struct SelectOption {
    pub label: String,
    pub slug: String,
    pub description: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectInteractionMode {
    List,
    Search,
}

pub trait OnboardUi {
    fn print_line(&mut self, line: &str) -> CliResult<()>;
    fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String>;
    fn prompt_required(&mut self, label: &str) -> CliResult<String>;
    fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
        self.prompt_required(label)
    }
    fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool>;
    fn select_one(
        &mut self,
        label: &str,
        options: &[SelectOption],
        default: Option<usize>,
        interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize>;
}

#[derive(Debug, Clone)]
pub struct OnboardRuntimeContext {
    pub(super) render_width: usize,
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) codex_config_paths: Vec<PathBuf>,
}

impl OnboardRuntimeContext {
    fn capture() -> Self {
        Self {
            render_width: detect_render_width(),
            workspace_root: env::current_dir().ok(),
            codex_config_paths: default_codex_config_paths(),
        }
    }

    pub fn new_for_tests(
        render_width: usize,
        workspace_root: Option<PathBuf>,
        codex_config_paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            render_width,
            workspace_root,
            codex_config_paths: codex_config_paths.into_iter().collect(),
        }
    }
}

#[cfg(test)]
pub(super) fn provider_model_probe_failure_check(
    config: &mvp::config::LoongConfig,
    error: String,
) -> OnboardCheck {
    crate::onboard::preflight::provider_model_probe_failure_check(config, error)
}

pub async fn run_onboard_cli(options: OnboardCommandOptions) -> CliResult<()> {
    let context = OnboardRuntimeContext::capture();
    let mut ui = StdioOnboardUi::default();
    run_onboard_cli_with_ui(options, &mut ui, &context).await
}

pub async fn run_onboard_cli_with_ui(
    options: OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    let preparation = prepare_onboard_session(&options, ui, context)?;
    let OnboardSessionPreparation {
        output_path,
        starting_selection,
        mut config,
        skip_detailed_setup,
        review_flow_style,
    } = preparation;
    let reuse_existing_non_interactive_config = options.non_interactive
        && starting_selection.entry_choice == OnboardEntryChoice::ContinueCurrentSetup
        && starting_selection.current_setup_state == crate::migration::CurrentSetupState::Healthy
        && !onboard_has_explicit_overrides(&options);

    if !skip_detailed_setup && !reuse_existing_non_interactive_config {
        apply_guided_onboard_configuration(
            &options,
            &output_path,
            &starting_selection.provider_selection,
            &mut config,
            ui,
            context,
        )
        .await?;
    }
    let selected_preinstalled_skill_ids =
        resolve_preinstalled_skill_selection(&options, ui, context)?;
    apply_selected_preinstalled_skills_to_config(
        &mut config,
        &output_path,
        &selected_preinstalled_skill_ids,
    );

    let review =
        build_onboard_review_context(&config, &starting_selection, review_flow_style, context);
    let preflight = complete_onboard_preflight(
        &options,
        &output_path,
        &config,
        reuse_existing_non_interactive_config,
        review.review_flow_style,
        ui,
        context,
    )
    .await?;
    finalize_onboard_closeout(
        &options,
        &output_path,
        &config,
        &selected_preinstalled_skill_ids,
        &starting_selection,
        &review,
        preflight,
        ui,
        context,
    )
    .await
}
