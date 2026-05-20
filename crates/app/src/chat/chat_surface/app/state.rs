#[derive(Clone)]
struct PendingRenderCache {
    signature: u64,
    max_pending_height: u16,
    lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LiveTranscriptState {
    draft_preview: Option<String>,
    tool_activity_lines: Vec<String>,
}

impl LiveTranscriptState {
    fn has_needs_approval(&self) -> bool {
        self.tool_activity_lines.iter().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("[needs_approval]") || trimmed.contains("[needs_approval]")
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupOnboardingStage {
    Language,
    Provider,
    Skills,
    SetupPath,
    Personalization,
    Finish,
}

impl StartupOnboardingStage {
    const ALL: [Self; 6] = [
        Self::Language,
        Self::Provider,
        Self::Skills,
        Self::SetupPath,
        Self::Personalization,
        Self::Finish,
    ];

    fn title(self, language: Language) -> &'static str {
        match self {
            Self::Language => match language {
                Language::ZhCn => "语言",
                Language::ZhTw => "語言",
                Language::Ja => "言語",
                Language::Ru => "язык",
                Language::En => "language",
            },
            Self::Provider => "provider",
            Self::Skills => match language {
                Language::ZhCn => "技能",
                Language::ZhTw => "技能",
                Language::Ja => "スキル",
                Language::Ru => "навыки",
                Language::En => "skills",
            },
            Self::SetupPath => match language {
                Language::ZhCn => "后续配置",
                Language::ZhTw => "後續配置",
                Language::Ja => "続きの設定",
                Language::Ru => "дальше настроить",
                Language::En => "continue setup",
            },
            Self::Personalization => match language {
                Language::ZhCn => "首轮风格",
                Language::ZhTw => "首輪風格",
                Language::Ja => "最初の会話スタイル",
                Language::Ru => "стиль первого хода",
                Language::En => "first chat style",
            },
            Self::Finish => match language {
                Language::ZhCn => "准备开聊",
                Language::ZhTw => "準備開聊",
                Language::Ja => "チャット開始",
                Language::Ru => "готово к чату",
                Language::En => "ready to chat",
            },
        }
    }

    fn step_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|stage| *stage == self)
            .unwrap_or(0)
            + 1
    }

    fn total_steps() -> usize {
        Self::ALL.len()
    }

    fn next(self) -> Self {
        match self {
            Self::Language => Self::Provider,
            Self::Provider => Self::Skills,
            Self::Skills => Self::SetupPath,
            Self::SetupPath => Self::Personalization,
            Self::Personalization => Self::Finish,
            Self::Finish => Self::Finish,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Language => Self::Language,
            Self::Provider => Self::Language,
            Self::Skills => Self::Provider,
            Self::SetupPath => Self::Skills,
            Self::Personalization => Self::SetupPath,
            Self::Finish => Self::Personalization,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupSetupPathChoice {
    ChatNow,
    ProviderAndWeb,
    ChannelsAndDelivery,
    McpAndSkills,
}

impl StartupSetupPathChoice {
    const ALL: [Self; 4] = [
        Self::ChatNow,
        Self::ProviderAndWeb,
        Self::ChannelsAndDelivery,
        Self::McpAndSkills,
    ];

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::ChatNow => match language {
                Language::ZhCn => "先聊天",
                Language::ZhTw => "先聊天",
                Language::Ja => "まず会話",
                Language::Ru => "сначала чат",
                Language::En => "chat now",
            },
            Self::ProviderAndWeb => match language {
                Language::ZhCn => "provider + web 配置",
                Language::ZhTw => "provider + web 配置",
                Language::Ja => "provider + web 設定",
                Language::Ru => "provider + web настройка",
                Language::En => "provider + web setup",
            },
            Self::ChannelsAndDelivery => match language {
                Language::ZhCn => "channels + delivery",
                Language::ZhTw => "channels + delivery",
                Language::Ja => "channels + delivery",
                Language::Ru => "channels + delivery",
                Language::En => "channels + delivery",
            },
            Self::McpAndSkills => match language {
                Language::ZhCn => "MCP + workspace 配置",
                Language::ZhTw => "MCP + workspace 配置",
                Language::Ja => "MCP + workspace 設定",
                Language::Ru => "MCP + workspace настройка",
                Language::En => "MCP + workspace setup",
            },
        }
    }

    fn detail(self, language: Language) -> &'static str {
        match self {
            Self::ChatNow => match language {
                Language::ZhCn => "先保持 shell 简洁，等真实任务需要时再展开更深配置。",
                Language::ZhTw => "先保持 shell 精簡，等真實任務需要時再展開更深配置。",
                Language::Ja => "今は shell を最小限に保ち、必要になった時に深い設定を開きます。",
                Language::Ru => {
                    "пока оставить shell минимальным и раскрывать глубокую настройку только когда она реально нужна."
                }
                Language::En => {
                    "keep the shell minimal now; surface deeper setup when a real task needs it"
                }
            },
            Self::ProviderAndWeb => match language {
                Language::ZhCn => {
                    "继续看 provider 鉴权、web search 默认项，以及完整 onboard 向导。"
                }
                Language::ZhTw => {
                    "繼續看 provider 驗證、web search 預設項，以及完整 onboard 嚮導。"
                }
                Language::Ja => {
                    "provider 認証、web search の既定値、完全な onboard wizard を確認します。"
                }
                Language::Ru => {
                    "посмотреть provider auth, web search defaults и полный onboard wizard."
                }
                Language::En => {
                    "review provider auth, web search defaults, and the full onboard wizard path"
                }
            },
            Self::ChannelsAndDelivery => match language {
                Language::ZhCn => "继续看已启用 channels、delivery 面以及下一条 serve/setup 命令。",
                Language::ZhTw => "繼續看已啟用 channels、delivery 面以及下一條 serve/setup 命令。",
                Language::Ja => {
                    "有効な channels、delivery 面、次の serve/setup コマンドを確認します。"
                }
                Language::Ru => {
                    "посмотреть включённые channels, delivery surfaces и следующие serve/setup команды."
                }
                Language::En => {
                    "review enabled channels, delivery surfaces, and the next serve/setup commands"
                }
            },
            Self::McpAndSkills => match language {
                Language::ZhCn => "继续看 MCP server、bundled skill 以及本地工具的下一步命令。",
                Language::ZhTw => "繼續看 MCP server、bundled skill 以及本地工具的下一步命令。",
                Language::Ja => {
                    "MCP server、bundled skill、ローカルツールの次コマンドを確認します。"
                }
                Language::Ru => {
                    "посмотреть MCP servers, bundled skills и следующие локальные команды."
                }
                Language::En => {
                    "review MCP servers, bundled skills, and the next commands for local tooling"
                }
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupPersonalizationPreset {
    Balanced,
    Concise,
    Thorough,
    Later,
    TurnOff,
}

impl StartupPersonalizationPreset {
    const ALL: [Self; 5] = [
        Self::Balanced,
        Self::Concise,
        Self::Thorough,
        Self::Later,
        Self::TurnOff,
    ];

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Balanced => match language {
                Language::ZhCn => "平衡模式",
                Language::ZhTw => "平衡模式",
                Language::Ja => "バランス型",
                Language::Ru => "сбалансированный режим",
                Language::En => "balanced operator",
            },
            Self::Concise => match language {
                Language::ZhCn => "简洁模式",
                Language::ZhTw => "精簡模式",
                Language::Ja => "簡潔型",
                Language::Ru => "краткий режим",
                Language::En => "concise reviewer",
            },
            Self::Thorough => match language {
                Language::ZhCn => "深入模式",
                Language::ZhTw => "深入模式",
                Language::Ja => "深掘り型",
                Language::Ru => "подробный режим",
                Language::En => "deep pairer",
            },
            Self::Later => match language {
                Language::ZhCn => "稍后决定",
                Language::ZhTw => "稍後決定",
                Language::Ja => "後で決める",
                Language::Ru => "решить позже",
                Language::En => "decide later",
            },
            Self::TurnOff => match language {
                Language::ZhCn => "关闭个性化",
                Language::ZhTw => "關閉個性化",
                Language::Ja => "個性化をオフ",
                Language::Ru => "выключить персонализацию",
                Language::En => "turn off personalization",
            },
        }
    }

    fn detail(self, language: Language) -> &'static str {
        match self {
            Self::Balanced => match language {
                Language::ZhCn => "默认保持平衡的密度与主动性。",
                Language::ZhTw => "預設保持平衡的密度與主動性。",
                Language::Ja => "密度と主体性のバランスを保ちます。",
                Language::Ru => "сбалансированная плотность ответа и инициативность.",
                Language::En => "balanced density and initiative for a normal first conversation",
            },
            Self::Concise => match language {
                Language::ZhCn => "回答更短，并更偏向先问再行动。",
                Language::ZhTw => "回答更短，並更偏向先問再行動。",
                Language::Ja => "短めに返し、先に確認する寄りです。",
                Language::Ru => "короче ответы и поведение сначала спросить.",
                Language::En => "short answers and ask-before-acting behavior",
            },
            Self::Thorough => match language {
                Language::ZhCn => "回答更深入，合适时更主动。",
                Language::ZhTw => "回答更深入，合適時更主動。",
                Language::Ja => "より深く返し、必要なら主体的に進みます。",
                Language::Ru => "более глубокие ответы и выше инициативность, когда уместно.",
                Language::En => "deeper responses with higher initiative when useful",
            },
            Self::Later => match language {
                Language::ZhCn => "先不保存这轮对话偏好。",
                Language::ZhTw => "先不保存這輪對話偏好。",
                Language::Ja => "今は会話プリセットを保存しません。",
                Language::Ru => "пока не сохранять этот разговорный пресет.",
                Language::En => "skip saved conversation preferences for now",
            },
            Self::TurnOff => match language {
                Language::ZhCn => "关闭后续个性化提示，先保持默认对话风格。",
                Language::ZhTw => "關閉後續個性化提示，先保持預設對話風格。",
                Language::Ja => "以後の個性化案内を止め、標準の会話スタイルを保ちます。",
                Language::Ru => {
                    "отключить дальнейшие подсказки по персонализации и оставить стиль по умолчанию."
                }
                Language::En => {
                    "suppress future personalization prompts and keep the default conversation style"
                }
            },
        }
    }

    fn response_density(self) -> Option<ResponseDensity> {
        match self {
            Self::Balanced => Some(ResponseDensity::Balanced),
            Self::Concise => Some(ResponseDensity::Concise),
            Self::Thorough => Some(ResponseDensity::Thorough),
            Self::Later | Self::TurnOff => None,
        }
    }

    fn initiative_level(self) -> Option<InitiativeLevel> {
        match self {
            Self::Balanced => Some(InitiativeLevel::Balanced),
            Self::Concise => Some(InitiativeLevel::AskBeforeActing),
            Self::Thorough => Some(InitiativeLevel::HighInitiative),
            Self::Later | Self::TurnOff => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupProviderOption {
    kind: ProviderKind,
    auth_env_name: Option<String>,
    is_current: bool,
    label: String,
    detail: String,
    recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupSkillOption {
    install_id: String,
    display_name: String,
    summary: String,
    recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupChannelFollowUpDescriptor {
    label: String,
    serve_command: Option<String>,
    status_command: String,
    repair_command: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StartupBootstrapCapture {
    preferred_address: Option<String>,
    pronouns: Option<String>,
    agent_name: Option<String>,
    creature: Option<String>,
    vibe: Option<String>,
    emoji: Option<String>,
    timezone: Option<String>,
    standing_boundaries: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupOnboardingState {
    stage: StartupOnboardingStage,
    language_options: Vec<Language>,
    language_index: usize,
    provider_options: Vec<StartupProviderOption>,
    provider_index: usize,
    skill_options: Vec<StartupSkillOption>,
    selected_skill_ids: BTreeSet<String>,
    skill_cursor: usize,
    setup_path_index: usize,
    personalization_index: usize,
    selected_personalization: Option<StartupPersonalizationPreset>,
    web_search_provider_label: String,
    web_search_provider_detail: String,
    provider_auth_env_name: Option<String>,
    provider_configuration_hint: Option<String>,
    enabled_channel_labels: Vec<String>,
    channel_follow_up_commands: Vec<String>,
    channel_status_commands: Vec<String>,
    channel_repair_commands: Vec<String>,
    startup_mcp_count: usize,
    detected_skill_count: usize,
    feedback: Option<String>,
    last_interaction_at: std::time::Instant,
    last_interaction_kind: StartupOnboardingInteractionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupOnboardingInteractionKind {
    Passive,
    Navigate,
    Confirm,
    Persist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupProviderAuthBindingKind {
    ApiKey,
    OauthAccessToken,
}

fn startup_provider_config_for_kind(kind: ProviderKind) -> ProviderConfig {
    let mut provider = ProviderConfig::fresh_for_kind(kind);
    if let Some((env_name, binding_kind)) = detected_startup_auth_binding(kind) {
        apply_startup_auth_binding(&mut provider, env_name.as_str(), binding_kind);
    }
    provider
}

fn detected_startup_auth_binding(
    kind: ProviderKind,
) -> Option<(String, StartupProviderAuthBindingKind)> {
    if let Some(env_name) = kind
        .default_oauth_access_token_env()
        .filter(|env_name| std::env::var_os(env_name).is_some())
    {
        return Some((
            env_name.to_owned(),
            StartupProviderAuthBindingKind::OauthAccessToken,
        ));
    }
    for env_name in kind.oauth_access_token_env_aliases() {
        if std::env::var_os(env_name).is_some() {
            return Some((
                (*env_name).to_owned(),
                StartupProviderAuthBindingKind::OauthAccessToken,
            ));
        }
    }
    if let Some(env_name) = kind
        .default_api_key_env()
        .filter(|env_name| std::env::var_os(env_name).is_some())
    {
        return Some((env_name.to_string(), StartupProviderAuthBindingKind::ApiKey));
    }
    for env_name in kind.api_key_env_aliases() {
        if std::env::var_os(env_name).is_some() {
            return Some((
                (*env_name).to_owned(),
                StartupProviderAuthBindingKind::ApiKey,
            ));
        }
    }
    None
}

fn apply_startup_auth_binding(
    provider: &mut ProviderConfig,
    env_name: &str,
    binding_kind: StartupProviderAuthBindingKind,
) {
    match binding_kind {
        StartupProviderAuthBindingKind::ApiKey => {
            provider.set_api_key_env_binding(Some(env_name.to_owned()));
        }
        StartupProviderAuthBindingKind::OauthAccessToken => {
            provider.set_oauth_access_token_env_binding(Some(env_name.to_owned()));
        }
    }
}

impl StartupOnboardingState {
    fn new(runtime: &CliTurnRuntime, preferred_language: Language) -> Option<Self> {
        if !startup_onboarding_enabled(runtime) {
            return None;
        }

        let language_options = vec![
            Language::En,
            Language::ZhCn,
            Language::ZhTw,
            Language::Ja,
            Language::Ru,
        ];
        let language_index = language_options
            .iter()
            .position(|language| *language == preferred_language)
            .unwrap_or(0);

        let skill_options = bundled_preinstall_targets()
            .iter()
            .map(|target| StartupSkillOption {
                install_id: target.install_id.to_owned(),
                display_name: target.display_name.to_owned(),
                summary: target.summary.to_owned(),
                recommended: target.recommended,
            })
            .collect::<Vec<_>>();
        let detected_skill_count = skill_options.len();

        let mut state = Self {
            stage: StartupOnboardingStage::Language,
            language_options,
            language_index,
            provider_options: Vec::new(),
            provider_index: 0,
            skill_options,
            selected_skill_ids: BTreeSet::new(),
            skill_cursor: 0,
            setup_path_index: 0,
            personalization_index: 0,
            selected_personalization: None,
            web_search_provider_label: String::new(),
            web_search_provider_detail: String::new(),
            provider_auth_env_name: None,
            provider_configuration_hint: None,
            enabled_channel_labels: Vec::new(),
            channel_follow_up_commands: Vec::new(),
            channel_status_commands: Vec::new(),
            channel_repair_commands: Vec::new(),
            startup_mcp_count: runtime.effective_bootstrap_mcp_servers.len(),
            detected_skill_count,
            feedback: Some(
                "choose language first, then confirm provider and optional skill packs.".to_owned(),
            ),
            last_interaction_at: std::time::Instant::now(),
            last_interaction_kind: StartupOnboardingInteractionKind::Passive,
        };
        state.refresh_localized_runtime_content(runtime);
        Some(state)
    }

    fn refresh_localized_runtime_content(&mut self, runtime: &CliTurnRuntime) {
        let language = self.current_language();
        let selected_provider_kind = self
            .provider_options
            .get(self.provider_index)
            .map(|option| option.kind)
            .unwrap_or(runtime.config.provider.kind);
        self.provider_options = build_startup_provider_options(runtime, language);
        self.provider_index = self
            .provider_options
            .iter()
            .position(|option| option.kind == selected_provider_kind)
            .or_else(|| {
                self.provider_options
                    .iter()
                    .position(|option| option.kind == runtime.config.provider.kind)
            })
            .unwrap_or(0);

        let normalized_web_search_provider = normalize_web_search_provider(
            runtime.config.tools.web_search.default_provider.as_str(),
        )
        .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
        self.web_search_provider_label =
            web_search_provider_descriptor(normalized_web_search_provider)
                .map(|descriptor| descriptor.display_name)
                .unwrap_or(normalized_web_search_provider)
                .to_owned();
        self.web_search_provider_detail =
            startup_web_search_detail(runtime, normalized_web_search_provider, language);
        self.provider_auth_env_name = runtime.config.provider.resolved_auth_env_name();
        self.provider_configuration_hint = runtime.config.provider.configuration_hint();

        let channel_follow_up = startup_channel_follow_up_descriptors(runtime, language);
        self.enabled_channel_labels = channel_follow_up
            .iter()
            .map(|descriptor| descriptor.label.clone())
            .collect();
        self.channel_follow_up_commands = channel_follow_up
            .iter()
            .filter_map(|descriptor| descriptor.serve_command.clone())
            .collect();
        self.channel_status_commands = channel_follow_up
            .iter()
            .map(|descriptor| descriptor.status_command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.channel_repair_commands = channel_follow_up
            .iter()
            .filter_map(|descriptor| descriptor.repair_command.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }

    fn mark_interaction(&mut self, kind: StartupOnboardingInteractionKind) {
        self.last_interaction_at = std::time::Instant::now();
        self.last_interaction_kind = kind;
    }

    fn current_language(&self) -> Language {
        self.language_options
            .get(self.language_index)
            .copied()
            .unwrap_or(Language::En)
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> StartupOnboardingAction {
        match self.stage {
            StartupOnboardingStage::Language => self.handle_language_key(key),
            StartupOnboardingStage::Provider => self.handle_provider_key(key),
            StartupOnboardingStage::Skills => self.handle_skills_key(key),
            StartupOnboardingStage::SetupPath => self.handle_setup_path_key(key),
            StartupOnboardingStage::Personalization => self.handle_personalization_key(key),
            StartupOnboardingStage::Finish => self.handle_finish_key(key),
        }
    }

    fn handle_language_key(&mut self, key: crossterm::event::KeyEvent) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Up {
            self.language_index = self.language_index.saturating_sub(1);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Down {
            let max_index = self.language_options.len().saturating_sub(1);
            self.language_index = (self.language_index + 1).min(max_index);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Enter {
            let language = self.current_language();
            self.feedback = Some(format!(
                "{} {}。",
                startup_feedback_prefix("language_set", language),
                startup_language_label(language)
            ));
            self.stage = self.stage.next();
            self.mark_interaction(StartupOnboardingInteractionKind::Confirm);
            StartupOnboardingAction::ApplyLanguage(language)
        } else if code == KeyCode::Esc {
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn handle_provider_key(&mut self, key: crossterm::event::KeyEvent) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Up {
            self.provider_index = self.provider_index.saturating_sub(1);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Down {
            let max_index = self.provider_options.len().saturating_sub(1);
            self.provider_index = (self.provider_index + 1).min(max_index);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Enter {
            self.mark_interaction(StartupOnboardingInteractionKind::Confirm);
            self.provider_options
                .get(self.provider_index)
                .cloned()
                .map_or(StartupOnboardingAction::Ignored, |option| {
                    StartupOnboardingAction::PersistProviderSelection(option)
                })
        } else if code == KeyCode::Esc {
            self.stage = self.stage.previous();
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn handle_skills_key(&mut self, key: crossterm::event::KeyEvent) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Up {
            self.skill_cursor = self.skill_cursor.saturating_sub(1);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Down {
            let max_index = self.skill_options.len().saturating_sub(1);
            self.skill_cursor = (self.skill_cursor + 1).min(max_index);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Char(' ') {
            let language = self.current_language();
            if let Some(option) = self.skill_options.get(self.skill_cursor) {
                if !self.selected_skill_ids.insert(option.install_id.clone()) {
                    self.selected_skill_ids.remove(option.install_id.as_str());
                }
                let selection_count = self.selected_skill_ids.len();
                self.feedback = Some(if selection_count == 0 {
                    startup_feedback_prefix("skills_none_selected", language).to_owned()
                } else {
                    startup_feedback_selected_skill_packs(language, selection_count)
                });
            }
            self.mark_interaction(StartupOnboardingInteractionKind::Confirm);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Enter {
            let language = self.current_language();
            let selection_count = self.selected_skill_ids.len();
            self.feedback = Some(if selection_count == 0 {
                startup_feedback_prefix("skills_skipped", language).to_owned()
            } else {
                startup_feedback_queued_skill_packs(language, selection_count)
            });
            self.stage = self.stage.next();
            self.mark_interaction(StartupOnboardingInteractionKind::Confirm);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Esc {
            self.stage = self.stage.previous();
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn handle_setup_path_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Up {
            self.setup_path_index = self.setup_path_index.saturating_sub(1);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Down {
            let max_index = StartupSetupPathChoice::ALL.len().saturating_sub(1);
            self.setup_path_index = (self.setup_path_index + 1).min(max_index);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Enter {
            let choice = self.current_setup_path_choice();
            let language = self.current_language();
            self.feedback = Some(match choice {
                StartupSetupPathChoice::ChatNow => {
                    startup_feedback_prefix("setup_chat_now", language).to_owned()
                }
                StartupSetupPathChoice::ProviderAndWeb => {
                    startup_feedback_prefix("setup_provider_web", language).to_owned()
                }
                StartupSetupPathChoice::ChannelsAndDelivery => {
                    startup_feedback_prefix("setup_channels_delivery", language).to_owned()
                }
                StartupSetupPathChoice::McpAndSkills => {
                    startup_feedback_prefix("setup_mcp_skills", language).to_owned()
                }
            });
            self.stage = self.stage.next();
            self.mark_interaction(StartupOnboardingInteractionKind::Confirm);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Esc {
            self.stage = self.stage.previous();
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn handle_personalization_key(
        &mut self,
        key: crossterm::event::KeyEvent,
    ) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Up {
            self.personalization_index = self.personalization_index.saturating_sub(1);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Down {
            let max_index = StartupPersonalizationPreset::ALL.len().saturating_sub(1);
            self.personalization_index = (self.personalization_index + 1).min(max_index);
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else if code == KeyCode::Enter {
            self.mark_interaction(StartupOnboardingInteractionKind::Persist);
            StartupOnboardingAction::PersistPersonalization(self.current_personalization_preset())
        } else if code == KeyCode::Esc {
            self.stage = self.stage.previous();
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn handle_finish_key(&mut self, key: crossterm::event::KeyEvent) -> StartupOnboardingAction {
        let code = key.code;
        if code == KeyCode::Enter {
            StartupOnboardingAction::Complete
        } else if code == KeyCode::Esc {
            self.stage = self.stage.previous();
            self.mark_interaction(StartupOnboardingInteractionKind::Navigate);
            StartupOnboardingAction::Handled
        } else {
            StartupOnboardingAction::Ignored
        }
    }

    fn current_setup_path_choice(&self) -> StartupSetupPathChoice {
        StartupSetupPathChoice::ALL
            .get(self.setup_path_index)
            .copied()
            .unwrap_or(StartupSetupPathChoice::ChatNow)
    }

    fn current_personalization_preset(&self) -> StartupPersonalizationPreset {
        StartupPersonalizationPreset::ALL
            .get(self.personalization_index)
            .copied()
            .unwrap_or(StartupPersonalizationPreset::Balanced)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupOnboardingAction {
    Ignored,
    Handled,
    ApplyLanguage(Language),
    PersistProviderSelection(StartupProviderOption),
    PersistPersonalization(StartupPersonalizationPreset),
    Complete,
}

pub struct App {
    pub message_list: MessageList,
    pub composer: Composer,
    pub command_palette: CommandPalette,
    pub focus: Focus,
    pub pending_turn: bool,
    pub turn_start: Option<std::time::Instant>,
    live_transcript: Arc<StdMutex<LiveTranscriptState>>,
    pub pending_task: Option<JoinHandle<CliResult<String>>>,
    pub pending_steers: VecDeque<String>,
    pub pending_queue: VecDeque<String>,
    pub composer_follow_up_intent: bool,
    pending_first_turn_bootstrap_addendum: Option<String>,
    awaiting_first_turn_bootstrap_reply: bool,
    pub live_render_width: Arc<AtomicUsize>,
    pub live_rerender: Option<super::super::CliChatLiveSurfaceRerender>,
    pub spinner_seed: u64,
    pub last_pending_signature: Option<u64>,
    last_live_transcript_signature: Option<u64>,
    pending_render_cache: Option<PendingRenderCache>,
    inline_skill_popup_active: bool,
    pub last_render_width: u16,
    pub last_render_height: u16,
    pub last_transcript_area: Rect,
    pub last_composer_area: Rect,
    pub last_palette_area: Rect,
    startup_onboarding: Option<StartupOnboardingState>,
    startup_version: String,
    startup_mcp_count: usize,
    detected_skills: Vec<SkillEntry>,
    pub cwd: String,
    pub model: String,
    pub title: Option<String>,
    last_terminal_title: Option<String>,
    title_attention_required: bool,
    title_pending_approval_count: usize,
    pub i18n: I18nService,
}

