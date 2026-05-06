use crossterm::event::{
    self, Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde::Deserialize;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use crate::CliResult;
use crate::channel::resolve_channel_onboarding_descriptor;
use crate::chat::CliChatOptions;
use crate::chat::CliTurnRuntime;
use crate::chat::control_plane::ChatControlPlaneStore;
use crate::config::{
    InitiativeLevel, LoongConfig, MemoryProfile, PersonalizationConfig, PersonalizationPromptState,
    ProviderAuthScheme, ProviderConfig, ProviderKind, ProviderProfileConfig, ReasoningEffort,
    ResponseDensity, normalize_web_search_provider, service_channel_descriptors,
    web_search_provider_api_key_env_names, web_search_provider_descriptor,
};
use crate::tools::bundled_preinstall_targets;
use crate::tui_surface::{TuiCalloutTone, TuiKeyValueSpec, TuiMessageSpec, TuiSectionSpec};

use super::command_palette::{
    CommandAction, CommandPalette, SettingsCommandAction, SettingsEntry, SettingsSurfaceFocus,
    SkillEntry, slash_command_specs,
};
use super::composer::Composer;
use super::i18n::{I18nService, Language, SurfaceCopy, resolve_default_language};
use super::message_list::{MessageList, StartupEyeAnimation, StartupEyeFocus};
use super::utils::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    Composer,
    CommandPalette,
    MessageList,
}

const FOOTER_BOTTOM_BREATHING_HEIGHT: u16 = 1;
const FOOTER_HORIZONTAL_INDENT: u16 = 2;
const PENDING_TOOL_ANIMATION_FRAME_MS: u64 = 90;
const PENDING_TOOL_LABEL_COLORS: [Color; 6] = [
    SURFACE_DIM_GRAY,
    SURFACE_GRAY,
    SURFACE_ACCENT,
    SURFACE_CYAN,
    Color::White,
    SURFACE_CYAN,
];
const PENDING_TOOL_BODY_COLORS: [Color; 6] = [
    SURFACE_GRAY,
    SURFACE_ACCENT,
    SURFACE_CYAN,
    Color::White,
    SURFACE_CYAN,
    SURFACE_ACCENT,
];

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
    pub i18n: I18nService,
}

impl App {
    pub fn new(
        runtime: &CliTurnRuntime,
        options: &CliChatOptions,
        render_width: usize,
    ) -> CliResult<Self> {
        let language = resolve_default_language();
        let detected_skills =
            detect_available_skills(runtime.effective_working_directory.as_deref());
        let startup_mcp_count = runtime.effective_bootstrap_mcp_servers.len();
        let mut app = Self {
            message_list: MessageList::new(),
            composer: Composer::new(),
            command_palette: CommandPalette::new(language, detected_skills.clone()),
            focus: Focus::Composer,
            pending_turn: false,
            turn_start: None,
            live_transcript: Arc::new(StdMutex::new(LiveTranscriptState::default())),
            pending_task: None,
            pending_steers: VecDeque::new(),
            pending_queue: VecDeque::new(),
            composer_follow_up_intent: false,
            pending_first_turn_bootstrap_addendum: None,
            awaiting_first_turn_bootstrap_reply: false,
            live_render_width: Arc::new(AtomicUsize::new(render_width.max(1))),
            live_rerender: None,
            spinner_seed: spinner_seed(),
            last_pending_signature: None,
            last_live_transcript_signature: None,
            pending_render_cache: None,
            inline_skill_popup_active: false,
            last_render_width: render_width as u16,
            last_render_height: 0,
            last_transcript_area: Rect::default(),
            last_composer_area: Rect::default(),
            last_palette_area: Rect::default(),
            startup_onboarding: StartupOnboardingState::new(runtime, language),
            startup_version: String::new(),
            startup_mcp_count,
            detected_skills,
            cwd: format_cwd(runtime),
            model: runtime.config.provider.model.clone(),
            title: None,
            i18n: I18nService::new(language),
        };

        let (version, tutorial, sections, tips) =
            build_chat_startup_content(runtime, options, render_width, &app.i18n);
        app.startup_version = version.clone();
        let startup_eye_animation =
            startup_eye_animation_for_state(app.startup_onboarding.as_ref());
        app.message_list.add_startup_header_with_tips_and_eye(
            version,
            tutorial,
            sections,
            tips,
            startup_eye_animation,
        );

        Ok(app)
    }

    pub fn render(&mut self, f: &mut Frame) {
        let size = f.area();
        self.last_render_width = size.width;
        self.last_render_height = size.height;
        let composer_height = self.composer.height_for_area(size.width, size.height);
        let palette_visible =
            matches!(self.focus, Focus::CommandPalette) || self.inline_skill_popup_active;
        let palette_height = if palette_visible {
            self.command_palette.desired_height() as u16
        } else {
            0
        };
        let interstitial_lines =
            self.interstitial_lines_for(size.width, size.height, composer_height, palette_height);
        let interstitial_height = interstitial_lines.len() as u16;
        let provisional_assistant_text = provisional_assistant_text(&self.live_transcript);
        let transcript_line_count = self
            .message_list
            .rendered_line_count_with_provisional_assistant(
                size.width,
                provisional_assistant_text.as_deref(),
            ) as u16;
        let bottom_band_height = interstitial_height
            + 1
            + composer_height
            + if palette_height > 0 {
                1 + palette_height
            } else {
                0
            }
            + 1
            + 1
            + FOOTER_BOTTOM_BREATHING_HEIGHT;
        let available_transcript_height = size.height.saturating_sub(bottom_band_height).max(1);
        let transcript_height = if self.message_list.messages.is_empty() {
            0
        } else {
            transcript_line_count.min(available_transcript_height)
        };
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(transcript_height),
                Constraint::Length(0),
                Constraint::Length(interstitial_height),
                Constraint::Length(1),
                Constraint::Length(composer_height),
                Constraint::Length(if palette_height > 0 { 1 } else { 0 }),
                Constraint::Length(palette_height),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(FOOTER_BOTTOM_BREATHING_HEIGHT),
            ])
            .split(size);

        let [
            transcript_area,
            _spacer_area,
            pending_area,
            composer_separator_area,
            composer_area,
            palette_separator_area,
            palette_area,
            footer_separator_area,
            footer_area,
            footer_bottom_spacing_area,
        ] = main_layout.as_ref()
        else {
            return;
        };

        self.last_transcript_area = *transcript_area;
        self.last_composer_area = *composer_area;
        self.last_palette_area = if palette_visible {
            *palette_area
        } else {
            Rect::default()
        };

        self.message_list.render_with_provisional_assistant(
            f,
            *transcript_area,
            provisional_assistant_text.as_deref(),
        );

        if interstitial_height > 0 {
            f.render_widget(Paragraph::new(interstitial_lines), *pending_area);
        }

        let line_color = SURFACE_COTTON_CANDY;
        let composer_separator_is_blank =
            interstitial_height == 0 && self.message_list.trailing_colored_block(size.width);
        if composer_separator_is_blank {
            f.render_widget(Paragraph::new(""), *composer_separator_area);
        } else {
            f.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(line_color)),
                *composer_separator_area,
            );
        }

        self.composer
            .render(f, *composer_area, matches!(self.focus, Focus::Composer));
        if matches!(self.focus, Focus::Composer) {
            let (x, y) = self.composer.cursor_position(*composer_area);
            f.set_cursor_position((x, y));
        }

        if palette_visible {
            f.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(line_color)),
                *palette_separator_area,
            );
            self.command_palette.render(f, *palette_area);
        }

        f.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(line_color)),
            *footer_separator_area,
        );

        let footer_content_area = footer_content_area(*footer_area);
        let footer_line = if self.pending_turn && !self.composer.is_empty() {
            build_queue_footer_line(
                &self.i18n,
                self.pending_queue.len(),
                footer_content_area.width,
            )
        } else if let Some(state) = self.startup_onboarding.as_ref() {
            build_startup_onboarding_footer_line(state, footer_content_area.width)
        } else if self.pending_turn && !self.pending_queue.is_empty() {
            build_restore_footer_line(
                &self.i18n,
                self.pending_queue.len(),
                footer_content_area.width,
            )
        } else if !self.message_list.is_following_tail() {
            build_follow_footer_line(&self.i18n, &self.model, footer_content_area.width)
        } else {
            build_status_footer_line(&self.cwd, &self.model, footer_content_area.width)
        };
        f.render_widget(Paragraph::new(footer_line), footer_content_area);
        f.render_widget(Paragraph::new(""), *footer_bottom_spacing_area);
    }

    fn refresh_startup_header(&mut self) {
        let tutorial = self.i18n.text(SurfaceCopy::Tutorial).to_owned();
        let sections = vec![
            (
                self.i18n.text(SurfaceCopy::StartupSectionSkills).to_owned(),
                vec![self.detected_skills.len().to_string()],
            ),
            (
                self.i18n.text(SurfaceCopy::StartupSectionMcp).to_owned(),
                vec![self.startup_mcp_count.to_string()],
            ),
        ];
        let tips = vec![
            tutorial.clone(),
            self.i18n.text(SurfaceCopy::StartupTipCommands).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipSkills).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipQueue).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipHistory).to_owned(),
        ];
        let eye_animation = startup_eye_animation_for_state(self.startup_onboarding.as_ref());
        self.message_list.replace_latest_startup_header_with_eye(
            self.startup_version.clone(),
            tutorial,
            sections,
            tips,
            eye_animation,
        );
    }

    fn apply_startup_onboarding_action(
        &mut self,
        action: StartupOnboardingAction,
        runtime: &mut CliTurnRuntime,
    ) -> CliResult<bool> {
        match action {
            StartupOnboardingAction::Ignored => Ok(false),
            StartupOnboardingAction::Handled => {
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::ApplyLanguage(language) => {
                self.i18n = I18nService::new(language);
                self.command_palette = CommandPalette::new(language, self.detected_skills.clone());
                self.inline_skill_popup_active = false;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.refresh_localized_runtime_content(runtime);
                }
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::PersistProviderSelection(option) => {
                let language = self
                    .startup_onboarding
                    .as_ref()
                    .map(StartupOnboardingState::current_language)
                    .unwrap_or(Language::En);
                let summary = persist_startup_provider_selection(runtime, option, language)?;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.refresh_localized_runtime_content(runtime);
                    state.feedback = Some(summary);
                    state.stage = StartupOnboardingStage::Skills;
                }
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::PersistPersonalization(preset) => {
                let language = self
                    .startup_onboarding
                    .as_ref()
                    .map(StartupOnboardingState::current_language)
                    .unwrap_or(Language::En);
                let summary = persist_startup_personalization(runtime, preset, language)?;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.selected_personalization = Some(preset);
                    state.feedback = Some(summary);
                    state.stage = StartupOnboardingStage::Finish;
                }
                self.pending_first_turn_bootstrap_addendum =
                    startup_first_turn_bootstrap_addendum(preset, language);
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::Complete => {
                self.startup_onboarding = None;
                self.refresh_startup_header();
                Ok(true)
            }
        }
    }

    fn interstitial_lines_for(
        &mut self,
        width: u16,
        height: u16,
        composer_height: u16,
        palette_height: u16,
    ) -> Vec<Line<'static>> {
        if self.pending_turn {
            return self.pending_lines_for(width, height, composer_height, palette_height);
        }

        self.startup_onboarding
            .as_ref()
            .map(|state| render_startup_onboarding_lines(state, width))
            .unwrap_or_default()
    }

    fn apply_palette_action(&mut self, action: CommandAction) -> Option<String> {
        match action {
            CommandAction::RunCommand(command) => {
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                Some(command.to_owned())
            }
            CommandAction::OpenSettings(_)
            | CommandAction::ApplySettings(_)
            | CommandAction::OpenModelReasoning(_)
            | CommandAction::ApplyModelSelection { .. } => None,
            CommandAction::Noop => None,
            CommandAction::InsertText(text) => {
                if let Some(range) = current_skill_token_range(&self.composer) {
                    let replacement =
                        inline_skill_replacement_text(self.composer.text(), &range, text.as_str());
                    self.composer.replace_range(range, replacement.as_str());
                } else {
                    self.composer.set_input(text);
                }
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                None
            }
            CommandAction::Close => {
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                None
            }
        }
    }

    fn handle_mouse_event(&mut self, mouse_event: MouseEvent) -> Option<String> {
        if rect_contains_point(self.last_palette_area, mouse_event.column, mouse_event.row)
            && (matches!(self.focus, Focus::CommandPalette) || self.inline_skill_popup_active)
        {
            return self
                .command_palette
                .handle_mouse(mouse_event, self.last_palette_area)
                .and_then(|action| self.apply_palette_action(action));
        }

        if rect_contains_point(self.last_composer_area, mouse_event.column, mouse_event.row) {
            if matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.focus = Focus::Composer;
                self.sync_inline_skill_popup();
            }
            return None;
        }

        if rect_contains_point(
            self.last_transcript_area,
            mouse_event.column,
            mouse_event.row,
        ) {
            if matches!(
                mouse_event.kind,
                MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::Down(MouseButton::Right)
                    | MouseEventKind::Down(MouseButton::Middle)
            ) {
                self.focus = Focus::MessageList;
                self.sync_inline_skill_popup();
            }
            self.message_list.handle_mouse(mouse_event);
        }

        None
    }

    fn sync_inline_skill_popup(&mut self) {
        if !matches!(self.focus, Focus::Composer) {
            self.inline_skill_popup_active = false;
            return;
        }

        if self.command_palette.has_skills()
            && let Some(query) = current_skill_token_query(&self.composer)
        {
            self.command_palette.show_skills(query.as_str());
            self.inline_skill_popup_active = true;
        } else {
            self.inline_skill_popup_active = false;
        }
    }

    fn confirm_inline_skill_popup(&mut self) {
        if let Some(action) = self
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))
        {
            let _ = self.apply_palette_action(action);
        } else {
            self.inline_skill_popup_active = false;
        }
    }

    fn handle_inline_skill_popup_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if !self.inline_skill_popup_active {
            return false;
        }

        if matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
        ) {
            let _ = self.command_palette.handle_key(key);
            return true;
        }

        if key.code == KeyCode::Esc {
            self.inline_skill_popup_active = false;
            return true;
        }

        if (key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT))
            || key.code == KeyCode::Tab
        {
            self.confirm_inline_skill_popup();
            return true;
        }

        false
    }

    fn pending_lines_for(
        &mut self,
        width: u16,
        height: u16,
        composer_height: u16,
        palette_height: u16,
    ) -> Vec<Line<'static>> {
        if !self.pending_turn {
            self.pending_render_cache = None;
            return Vec::new();
        }

        let max_pending_height = pending_band_max_height(height, composer_height, palette_height);
        let Some(signature) = pending_render_signature_for_geometry(
            self,
            width,
            height,
            composer_height,
            palette_height,
        ) else {
            self.pending_render_cache = None;
            return Vec::new();
        };

        if let Some(cache) = self.pending_render_cache.as_ref()
            && cache.signature == signature
            && cache.max_pending_height == max_pending_height
        {
            return cache.lines.clone();
        }

        let max_pending_preview_lines = max_pending_height.saturating_sub(2).max(1) as usize;
        let live_lines =
            pending_live_tool_activity_lines(&self.live_transcript, max_pending_preview_lines);
        let raw_pending_lines = build_pending_lines(
            self.turn_start,
            &live_lines,
            self.spinner_seed,
            &self.pending_steers,
            &self.pending_queue,
            width,
        );
        let lines = compact_pending_lines_for_height(raw_pending_lines, max_pending_height);
        self.pending_render_cache = Some(PendingRenderCache {
            signature,
            max_pending_height,
            lines: lines.clone(),
        });
        lines
    }
}

pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    runtime: CliTurnRuntime,
    options: CliChatOptions,
) -> CliResult<()> {
    let mut runtime = runtime;
    let mut last_known_size = terminal
        .size()
        .map_err(|e| format!("failed to query terminal size: {e}"))?;
    let render_width = last_known_size.width as usize;
    let mut app = App::new(&runtime, &options, render_width)?;
    let mut startup_release_task = Some(tokio::spawn(load_startup_release_lines(render_width)));
    let mut dirty = true;
    let mut last_resize_at: Option<std::time::Instant> = None;
    let mut pending_live_resize_rerender = false;

    loop {
        if let Some(task) = startup_release_task.as_ref()
            && task.is_finished()
            && let Some(task) = startup_release_task.take()
            && let Ok(Some(lines)) = task.await
        {
            app.message_list.add_rendered_lines(lines);
            dirty = true;
        }

        if maybe_finalize_pending_turn(terminal, &mut app, &mut runtime).await? {
            dirty = true;
        }

        if app.message_list.refresh_startup_animation() {
            dirty = true;
        }

        if app.pending_turn {
            let signature = pending_render_signature(&app);
            if signature != app.last_pending_signature {
                app.last_pending_signature = signature;
                dirty = true;
            }
            let live_transcript_signature = transcript_preview_signature(&app);
            if live_transcript_signature != app.last_live_transcript_signature {
                app.last_live_transcript_signature = live_transcript_signature;
                dirty = true;
            }
        } else {
            app.last_pending_signature = None;
            app.last_live_transcript_signature = None;
        }

        if resize_live_rerender_ready(
            pending_live_resize_rerender,
            last_resize_at.map(|instant| instant.elapsed()),
        ) {
            if let Some(rerender) = app.live_rerender.as_ref() {
                rerender();
            }
            pending_live_resize_rerender = false;
            last_resize_at = None;
            dirty = true;
        }

        if dirty {
            terminal
                .draw(|f| app.render(f))
                .map_err(|e| format!("draw error: {}", e))?;
            dirty = false;
            if !pending_live_resize_rerender {
                last_resize_at = None;
            }
        }

        let poll_timeout = if pending_live_resize_rerender {
            Duration::from_millis(16)
        } else if app.pending_turn {
            Duration::from_millis(40)
        } else if app.message_list.startup_animation_active() {
            Duration::from_millis(70)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(poll_timeout).map_err(|e| format!("poll error: {}", e))? {
            let event = event::read().map_err(|e| format!("read error: {}", e))?;

            match event {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        break;
                    }

                    if key.code == KeyCode::Char('o')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        if app.message_list.toggle_latest_compaction() {
                            app.focus = Focus::MessageList;
                        }
                        continue;
                    }

                    if app.pending_turn {
                        let mut pending_command = None;
                        let mut pending_submission = None;
                        if key.code == KeyCode::Up
                            && key.modifiers.contains(KeyModifiers::ALT)
                            && dequeue_pending_steer(&mut app)
                        {
                            continue;
                        }
                        match app.focus {
                            Focus::Composer => {
                                if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                    && app.composer.is_empty()
                                {
                                    let prefix = if key.code == KeyCode::Char(':') {
                                        ':'
                                    } else {
                                        '/'
                                    };
                                    open_slash_command_palette(&mut app, prefix, "");
                                } else if app.handle_inline_skill_popup_key(key) {
                                } else if should_route_composer_key_to_transcript(&app, key) {
                                    app.message_list.handle_key(key);
                                } else if key.code == KeyCode::Tab {
                                    if !app.composer.is_empty() {
                                        queue_pending_message(&mut app);
                                        app.inline_skill_popup_active = false;
                                    } else {
                                        app.focus = Focus::MessageList;
                                    }
                                } else if let Some(msg) = app.composer.handle_key(key) {
                                    pending_submission = Some(msg);
                                    app.sync_inline_skill_popup();
                                } else if !app.composer.is_empty() {
                                    app.composer_follow_up_intent = true;
                                    app.sync_inline_skill_popup();
                                } else {
                                    app.sync_inline_skill_popup();
                                }
                            }
                            Focus::MessageList => {
                                if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                    && app.composer.is_empty()
                                {
                                    let prefix = if key.code == KeyCode::Char(':') {
                                        ':'
                                    } else {
                                        '/'
                                    };
                                    open_slash_command_palette(&mut app, prefix, "");
                                } else if should_focus_composer_for_transcript_key(key) {
                                    pending_submission =
                                        route_transcript_key_to_composer(&mut app, key);
                                } else {
                                    app.message_list.handle_key(key);
                                    if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                                        app.focus = Focus::Composer;
                                    }
                                }
                            }
                            Focus::CommandPalette => {
                                if app.command_palette.is_commands_mode()
                                    && key.code == KeyCode::Backspace
                                    && app.command_palette.query_text().is_empty()
                                {
                                    clear_slash_palette_composer(&mut app);
                                    app.inline_skill_popup_active = false;
                                    app.focus = Focus::Composer;
                                    dirty = true;
                                    continue;
                                }
                                if let Some(action) = app.command_palette.handle_key(key)
                                    && let Some(command) = dispatch_palette_action(
                                        &mut app,
                                        &mut runtime,
                                        current_render_width(terminal)?,
                                        action,
                                    )?
                                {
                                    pending_command = Some(command);
                                } else if app.command_palette.is_commands_mode() {
                                    sync_slash_palette_composer(&mut app);
                                }
                            }
                        }
                        if let Some(msg) = pending_submission {
                            if msg == "/exit" {
                                break;
                            }
                            let trimmed_msg = msg.trim();
                            if matches!(trimmed_msg, "/" | ":") {
                                let prefix = if trimmed_msg.starts_with(':') {
                                    ':'
                                } else {
                                    '/'
                                };
                                open_slash_command_palette(&mut app, prefix, "");
                                dirty = true;
                                continue;
                            }
                            if let Some(command) = recognized_surface_command(trimmed_msg) {
                                pending_command = Some(command);
                            } else {
                                queue_pending_steer(&mut app, msg);
                            }
                        }
                        if let Some(command) = pending_command {
                            if command == "/exit" {
                                break;
                            }
                            run_surface_command(
                                terminal,
                                &mut app,
                                &mut runtime,
                                &options,
                                &command,
                            )
                            .await?;
                        }
                        dirty = true;
                        continue;
                    }

                    let mut command_to_run = None;
                    let mut submitted_message = None;

                    if app.startup_onboarding.is_some()
                        && app.composer.is_empty()
                        && matches!(app.focus, Focus::Composer)
                    {
                        let action = app
                            .startup_onboarding
                            .as_mut()
                            .map(|state| state.handle_key(key))
                            .unwrap_or(StartupOnboardingAction::Ignored);
                        if app.apply_startup_onboarding_action(action, &mut runtime)? {
                            dirty = true;
                            continue;
                        }
                    }

                    match app.focus {
                        Focus::Composer => {
                            if key.code == KeyCode::Esc {
                                if !app.composer.is_empty() {
                                    app.composer.clear();
                                    app.composer_follow_up_intent = false;
                                    app.inline_skill_popup_active = false;
                                }
                            } else if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                && app.composer.is_empty()
                            {
                                let prefix = if key.code == KeyCode::Char(':') {
                                    ':'
                                } else {
                                    '/'
                                };
                                open_slash_command_palette(&mut app, prefix, "");
                            } else if app.handle_inline_skill_popup_key(key) {
                            } else if should_route_composer_key_to_transcript(&app, key) {
                                app.message_list.handle_key(key);
                            } else if key.code == KeyCode::Tab {
                                app.focus = Focus::MessageList;
                            } else if let Some(msg) = app.composer.handle_key(key) {
                                submitted_message = Some(msg);
                                app.sync_inline_skill_popup();
                            } else {
                                app.sync_inline_skill_popup();
                            }
                        }
                        Focus::CommandPalette => {
                            if app.command_palette.is_commands_mode()
                                && key.code == KeyCode::Backspace
                                && app.command_palette.query_text().is_empty()
                            {
                                clear_slash_palette_composer(&mut app);
                                app.inline_skill_popup_active = false;
                                app.focus = Focus::Composer;
                                dirty = true;
                                continue;
                            }
                            if let Some(action) = app.command_palette.handle_key(key)
                                && let Some(command) = dispatch_palette_action(
                                    &mut app,
                                    &mut runtime,
                                    current_render_width(terminal)?,
                                    action,
                                )?
                            {
                                command_to_run = Some(command);
                            } else if app.command_palette.is_commands_mode() {
                                sync_slash_palette_composer(&mut app);
                            }
                        }
                        Focus::MessageList => {
                            if key.code == KeyCode::Tab {
                                app.focus = Focus::Composer;
                            } else if should_focus_composer_for_transcript_key(key) {
                                submitted_message = route_transcript_key_to_composer(&mut app, key);
                            } else {
                                app.message_list.handle_key(key);
                                if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                                    app.focus = Focus::Composer;
                                }
                            }
                        }
                    }

                    if let Some(msg) = submitted_message {
                        if msg == "/exit" {
                            break;
                        }

                        let trimmed_msg = msg.trim();
                        if matches!(trimmed_msg, "/" | ":") {
                            let prefix = if trimmed_msg.starts_with(':') {
                                ':'
                            } else {
                                '/'
                            };
                            open_slash_command_palette(&mut app, prefix, "");
                            continue;
                        }

                        if let Some(command) = recognized_surface_command(trimmed_msg) {
                            command_to_run = Some(command);
                        } else if submitted_message_is_follow_up(&app, &msg) {
                            start_turn(terminal, &mut app, &mut runtime, msg, false).await?;
                        } else {
                            submit_user_turn(terminal, &mut app, &mut runtime, msg).await?;
                        }
                    }

                    if let Some(command) = command_to_run {
                        if command == "/exit" {
                            break;
                        }

                        run_surface_command(terminal, &mut app, &mut runtime, &options, &command)
                            .await?;
                    }
                    dirty = true;
                }
                Event::Mouse(mouse_event) => {
                    if let Some(command) = app.handle_mouse_event(mouse_event) {
                        if command == "/exit" {
                            break;
                        }
                        run_surface_command(terminal, &mut app, &mut runtime, &options, &command)
                            .await?;
                    }
                    dirty = true;
                }
                Event::Resize(width, height) => {
                    let new_size = ratatui::layout::Size::new(width, height);
                    if new_size.width == last_known_size.width
                        && new_size.height == last_known_size.height
                    {
                        continue;
                    }
                    let width_changed = last_known_size.width != new_size.width;
                    let layout_changed = resize_reflow_required(
                        last_known_size.width,
                        last_known_size.height,
                        new_size.width,
                        new_size.height,
                    );
                    if layout_changed {
                        last_resize_at = Some(std::time::Instant::now());
                    }
                    last_known_size = new_size;
                    app.last_render_width = new_size.width;
                    app.last_render_height = new_size.height;
                    app.live_render_width
                        .store(new_size.width.max(1) as usize, Ordering::Relaxed);
                    if width_changed && app.live_rerender.is_some() {
                        pending_live_resize_rerender = true;
                    }
                    dirty = true;
                }
                Event::Paste(text) => {
                    paste_into_composer(&mut app, text.as_str());
                    dirty = true;
                }
                Event::FocusGained | Event::FocusLost => {}
            }
        }
    }
    Ok(())
}

fn startup_onboarding_enabled(runtime: &CliTurnRuntime) -> bool {
    startup_env_truthy("LOONG_TUI_ONBOARD")
        || (runtime.config.provider.api_key().is_none()
            && runtime.config.provider.oauth_access_token().is_none())
}

fn startup_env_truthy(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn build_startup_provider_options(
    runtime: &CliTurnRuntime,
    language: Language,
) -> Vec<StartupProviderOption> {
    let current_provider_kind = runtime.config.provider.kind;
    ProviderKind::all_sorted()
        .iter()
        .map(|kind| {
            let is_current = *kind == current_provider_kind;
            let provider = ProviderConfig {
                kind: *kind,
                ..ProviderConfig::default()
            };
            let auth_env_name = provider
                .auth_hint_env_names()
                .into_iter()
                .find(|env_name| std::env::var_os(env_name).is_some());
            let detail = if is_current {
                startup_current_provider_detail(runtime, language)
            } else if let Some(env_name) = auth_env_name.as_deref() {
                startup_provider_migration_detail(kind.display_name(), env_name, language)
            } else {
                startup_provider_kind_detail(kind.display_name(), language)
            };

            StartupProviderOption {
                kind: *kind,
                auth_env_name,
                is_current,
                label: kind.display_name().to_owned(),
                detail,
                recommended: is_current,
            }
        })
        .collect()
}

fn startup_current_provider_detail(runtime: &CliTurnRuntime, language: Language) -> String {
    if let Some(env_name) = runtime.config.provider.resolved_auth_env_name() {
        return match language {
            Language::ZhCn => {
                format!("沿用 config.toml 里的当前 Loong provider。凭证目前通过 {env_name} 解析。")
            }
            Language::ZhTw => {
                format!("沿用 config.toml 裡目前的 Loong provider。憑證目前透過 {env_name} 解析。")
            }
            Language::Ja => format!(
                "config.toml の現在の Loong provider をそのまま使います。認証情報は今 {env_name} から解決されています。"
            ),
            Language::Ru => format!(
                "использовать текущий provider из config.toml. Сейчас учётные данные берутся через {env_name}."
            ),
            Language::En => format!(
                "reuse the active Loong provider from config.toml. credentials currently resolve through {env_name}."
            ),
        };
    }

    if runtime.config.provider.api_key().is_some()
        || runtime.config.provider.oauth_access_token().is_some()
    {
        return match language {
            Language::ZhCn => {
                "沿用 config.toml 里的当前 Loong provider。当前 runtime 已经拿到了 provider 凭证。"
                    .to_owned()
            }
            Language::ZhTw => {
                "沿用 config.toml 裡目前的 Loong provider。當前 runtime 已經拿到 provider 憑證。"
                    .to_owned()
            }
            Language::Ja => {
                "config.toml の現在の Loong provider をそのまま使います。いまの runtime には認証情報が読み込まれています。"
                    .to_owned()
            }
            Language::Ru => {
                "использовать текущий provider из config.toml. В текущем runtime учётные данные уже загружены."
                    .to_owned()
            }
            Language::En => {
                "reuse the active Loong provider from config.toml. the current runtime already has provider credentials loaded."
                    .to_owned()
            }
        };
    }

    match language {
        Language::ZhCn => {
            "沿用 config.toml 里的当前 provider 形状。第一轮真实对话前仍需要把凭证接好。"
                .to_owned()
        }
        Language::ZhTw => {
            "沿用 config.toml 裡目前的 provider 形狀。第一輪真實對話前仍需要把憑證接好。"
                .to_owned()
        }
        Language::Ja => {
            "config.toml の現在の provider 形を引き継ぎます。最初の実際のターンの前に認証情報の配線はまだ必要です。"
                .to_owned()
        }
        Language::Ru => {
            "сохранить текущую форму provider из config.toml. Перед первым реальным ходом авторизацию ещё нужно подключить."
                .to_owned()
        }
        Language::En => {
            "reuse the current provider shape from config.toml. credentials still need to be wired before the first real turn."
                .to_owned()
        }
    }
}

fn startup_provider_migration_detail(
    provider_label: &str,
    env_name: &str,
    language: Language,
) -> String {
    match language {
        Language::ZhCn => format!(
            "Loong 在 {env_name} 里发现了可复用的 {provider_label} 凭证。这里先接上 provider，剩余细节后面再回到 config.toml。"
        ),
        Language::ZhTw => format!(
            "Loong 在 {env_name} 裡發現了可重用的 {provider_label} 憑證。這裡先接上 provider，剩餘細節之後再回到 config.toml。"
        ),
        Language::Ja => format!(
            "Loong は {env_name} に再利用できる {provider_label} 認証を見つけました。ここでは provider だけ先につなぎ、残りの細部は後で config.toml に戻って詰められます。"
        ),
        Language::Ru => format!(
            "Loong нашёл готовые учётные данные {provider_label} в {env_name}. Здесь можно сначала выбрать provider, а остальное позже довести в config.toml."
        ),
        Language::En => format!(
            "Loong found a ready local {provider_label} credential in {env_name}. You can keep moving here and wire the rest later in config.toml."
        ),
    }
}

fn startup_provider_kind_detail(provider_label: &str, language: Language) -> String {
    match language {
        Language::ZhCn => format!(
            "先切到 {provider_label}；后续 base_url、model 和鉴权还可以回到 config.toml 里细调。"
        ),
        Language::ZhTw => format!(
            "先切到 {provider_label}；後續 base_url、model 和驗證還可以回到 config.toml 裡微調。"
        ),
        Language::Ja => format!(
            "先に {provider_label} へ切り替え、base_url・model・認証の細部はあとで config.toml に戻って詰められます。"
        ),
        Language::Ru => format!(
            "Сначала переключитесь на {provider_label}; base_url, model и авторизацию потом можно дочистить в config.toml."
        ),
        Language::En => format!(
            "switch to {provider_label} now; you can still fine-tune base_url, model, and auth later in config.toml."
        ),
    }
}

fn persist_startup_provider_selection(
    runtime: &mut CliTurnRuntime,
    option: StartupProviderOption,
    language: Language,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let mut provider = if option.is_current {
        config.provider.clone()
    } else {
        ProviderConfig {
            kind: option.kind,
            ..ProviderConfig::default()
        }
        .selection_baseline()
    };

    if !option.is_current
        && let Some(env_name) = option.auth_env_name.as_deref()
    {
        match option.kind.auth_scheme() {
            ProviderAuthScheme::Bearer => {
                provider.set_oauth_access_token_env_binding(Some(env_name.to_owned()));
            }
            ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
                provider.set_api_key_env_binding(Some(env_name.to_owned()));
            }
        }
    }
    let profile = ProviderProfileConfig::from_provider(provider.clone());
    if config.active_provider_id().map(str::to_owned) != Some(provider.inferred_profile_id()) {
        config.last_provider = config.active_provider_id().map(str::to_owned);
    }
    config.set_active_provider_profile(provider.inferred_profile_id(), profile);

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;

    let summary = if let Some(env_name) = option.auth_env_name.as_deref() {
        match language {
            Language::ZhCn => format!("provider 已保存：{}（复用 {env_name}）。", option.label),
            Language::ZhTw => format!("provider 已保存：{}（重用 {env_name}）。", option.label),
            Language::Ja => format!(
                "provider を保存しました: {}（{env_name} を再利用）。",
                option.label
            ),
            Language::Ru => format!(
                "provider сохранён: {} (переиспользуется {env_name}).",
                option.label
            ),
            Language::En => format!("provider saved: {} (reusing {env_name}).", option.label),
        }
    } else {
        match language {
            Language::ZhCn => format!("provider 已保存：{}。", option.label),
            Language::ZhTw => format!("provider 已保存：{}。", option.label),
            Language::Ja => format!("provider を保存しました: {}。", option.label),
            Language::Ru => format!("provider сохранён: {}.", option.label),
            Language::En => format!("provider saved: {}.", option.label),
        }
    };

    Ok(summary)
}

fn startup_language_label(language: Language) -> &'static str {
    match language {
        Language::En => "English",
        Language::ZhCn => "简体中文",
        Language::ZhTw => "繁體中文",
        Language::Ja => "日本語",
        Language::Ru => "Русский",
    }
}

fn startup_onboarding_footer_text(stage: StartupOnboardingStage) -> &'static str {
    match stage {
        StartupOnboardingStage::Skills => "↑/↓ move · Space toggle · Enter continue · Esc back",
        StartupOnboardingStage::Language
        | StartupOnboardingStage::Provider
        | StartupOnboardingStage::SetupPath
        | StartupOnboardingStage::Personalization => "↑/↓ move · Enter continue · Esc back",
        StartupOnboardingStage::Finish => "Enter start chatting · Esc back",
    }
}

fn startup_onboarding_footer_text_for_language(
    stage: StartupOnboardingStage,
    language: Language,
) -> &'static str {
    match language {
        Language::ZhCn => match stage {
            StartupOnboardingStage::Skills => "↑/↓ 移动 · Space 勾选 · Enter 继续 · Esc 返回",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ 移动 · Enter 继续 · Esc 返回",
            StartupOnboardingStage::Finish => "Enter 开始聊天 · Esc 返回",
        },
        Language::ZhTw => match stage {
            StartupOnboardingStage::Skills => "↑/↓ 移動 · Space 勾選 · Enter 繼續 · Esc 返回",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ 移動 · Enter 繼續 · Esc 返回",
            StartupOnboardingStage::Finish => "Enter 開始聊天 · Esc 返回",
        },
        Language::Ja => match stage {
            StartupOnboardingStage::Skills => "↑/↓ move · Space 選択 · Enter 続行 · Esc 戻る",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ move · Enter 続行 · Esc 戻る",
            StartupOnboardingStage::Finish => "Enter で会話開始 · Esc 戻る",
        },
        Language::Ru => match stage {
            StartupOnboardingStage::Skills => "↑/↓ move · Space выбор · Enter дальше · Esc назад",
            StartupOnboardingStage::Language
            | StartupOnboardingStage::Provider
            | StartupOnboardingStage::SetupPath
            | StartupOnboardingStage::Personalization => "↑/↓ move · Enter дальше · Esc назад",
            StartupOnboardingStage::Finish => "Enter начать чат · Esc назад",
        },
        _ => startup_onboarding_footer_text(stage),
    }
}

fn startup_onboarding_subtitle(stage: StartupOnboardingStage, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match stage {
            StartupOnboardingStage::Language => "先选 TUI 语言，之后仍可继续细调 config.toml。",
            StartupOnboardingStage::Provider => {
                "先选 Loong 优先准备的 provider，本地可复用的凭证会自动显示出来。"
            }
            StartupOnboardingStage::Skills => {
                "Loong 可以预装少量 bundled skills。Space 勾选，Enter 继续。"
            }
            StartupOnboardingStage::SetupPath => {
                "决定是保持 shell 极简，还是继续看 provider、web search、channels、MCP 这些后续路径。"
            }
            StartupOnboardingStage::Personalization => {
                "先存一个首轮对话风格，让第一条真正回复更贴近你想要的节奏。"
            }
            StartupOnboardingStage::Finish => {
                "现在可以直接开聊；MCP、web provider、channel、personalization 也可以按需要再继续。"
            }
        },
        Language::ZhTw => match stage {
            StartupOnboardingStage::Language => "先選 TUI 語言，之後仍可繼續微調 config.toml。",
            StartupOnboardingStage::Provider => {
                "先選 Loong 優先準備的 provider，本地可重用的憑證會自動顯示出來。"
            }
            StartupOnboardingStage::Skills => {
                "Loong 可以預裝少量 bundled skills。Space 勾選，Enter 繼續。"
            }
            StartupOnboardingStage::SetupPath => {
                "決定是保持 shell 極簡，還是繼續看 provider、web search、channels、MCP 這些後續路徑。"
            }
            StartupOnboardingStage::Personalization => {
                "先存一個首輪對話風格，讓第一條真正回覆更貼近你想要的節奏。"
            }
            StartupOnboardingStage::Finish => {
                "現在可以直接開聊；MCP、web provider、channel、personalization 也可以按需要再繼續。"
            }
        },
        Language::Ja => match stage {
            StartupOnboardingStage::Language => {
                "先に TUI の言語を選びます。あとで config.toml は引き続き細かく調整できます。"
            }
            StartupOnboardingStage::Provider => {
                "Loong が先に整える provider を選びます。再利用できるローカル認証は自動で見えます。"
            }
            StartupOnboardingStage::Skills => {
                "Loong は少数の bundled skills を先に入れられます。Space で選択し、Enter で進みます。"
            }
            StartupOnboardingStage::SetupPath => {
                "shell を最小限に保つか、provider、web search、channels、MCP の続き方まで今のうちに見るかを決めます。"
            }
            StartupOnboardingStage::Personalization => {
                "最初の実際の返答のテンポを合わせるため、会話スタイルを軽く保存します。"
            }
            StartupOnboardingStage::Finish => {
                "ここからすぐ会話できます。MCP、web provider、channels、personalization は必要になった時に続きを出せます。"
            }
        },
        Language::Ru => match stage {
            StartupOnboardingStage::Language => {
                "Сначала выберите язык TUI. Позже config.toml всё ещё можно будет тонко подстроить."
            }
            StartupOnboardingStage::Provider => {
                "Выберите provider, который Loong должен подготовить первым. Готовые локальные учётные данные будут показаны автоматически."
            }
            StartupOnboardingStage::Skills => {
                "Loong может заранее поставить несколько bundled skills. Space выбирает, Enter идёт дальше."
            }
            StartupOnboardingStage::SetupPath => {
                "Решите: оставить shell минимальным сейчас или сразу заглянуть в provider, web search, channels и MCP."
            }
            StartupOnboardingStage::Personalization => {
                "Сохраните лёгкий стиль первого разговора, чтобы первый реальный ответ попал в нужный ритм."
            }
            StartupOnboardingStage::Finish => {
                "Теперь можно сразу начать чат; MCP, web provider, channels и personalization можно продолжить по мере надобности."
            }
        },
        _ => match stage {
            StartupOnboardingStage::Language => {
                "choose the TUI language first. You can still fine-tune config.toml later."
            }
            StartupOnboardingStage::Provider => {
                "pick the provider Loong should prepare first. Ready local credentials are surfaced automatically."
            }
            StartupOnboardingStage::Skills => {
                "Loong can preinstall a few bundled skills. Space toggles selection; Enter moves on."
            }
            StartupOnboardingStage::SetupPath => {
                "keep the shell minimal or keep going into the current provider, web search, channels, MCP, and workspace setup details before the first real turn."
            }
            StartupOnboardingStage::Personalization => {
                "save a light first-conversation style so the first real answer lands with the right density and initiative."
            }
            StartupOnboardingStage::Finish => {
                "skip the rest for now. Loong will guide MCP, web-provider setup, and first-turn personalization when a conversation actually needs it."
            }
        },
    }
}

fn startup_onboarding_subtitle_for_state(state: &StartupOnboardingState) -> &'static str {
    if state.stage != StartupOnboardingStage::Finish {
        return startup_onboarding_subtitle(state.stage, state.current_language());
    }

    match (state.current_language(), state.selected_personalization) {
        (Language::ZhCn, Some(StartupPersonalizationPreset::Later)) => {
            "现在可以直接开聊；如果之后想补首轮偏好，再手动运行 `loong personalize`。"
        }
        (Language::ZhCn, Some(StartupPersonalizationPreset::TurnOff)) => {
            "现在可以直接开聊；Loong 不会再主动弹个性化提示，后面需要时仍可手动运行 `loong personalize`。"
        }
        (Language::ZhTw, Some(StartupPersonalizationPreset::Later)) => {
            "現在可以直接開聊；如果之後想補首輪偏好，再手動執行 `loong personalize`。"
        }
        (Language::ZhTw, Some(StartupPersonalizationPreset::TurnOff)) => {
            "現在可以直接開聊；Loong 不會再主動跳出個性化提示，之後需要時仍可手動執行 `loong personalize`。"
        }
        (Language::Ja, Some(StartupPersonalizationPreset::Later)) => {
            "ここからすぐ会話できます。最初の好みを後で足したい時だけ `loong personalize` を使います。"
        }
        (Language::Ja, Some(StartupPersonalizationPreset::TurnOff)) => {
            "ここからすぐ会話できます。Loong は今後個性化の案内を自動では出さず、必要なら手動で `loong personalize` を使えます。"
        }
        (Language::Ru, Some(StartupPersonalizationPreset::Later)) => {
            "Теперь можно сразу начать чат; если позже захотите добавить предпочтения первого разговора, вручную запустите `loong personalize`."
        }
        (Language::Ru, Some(StartupPersonalizationPreset::TurnOff)) => {
            "Теперь можно сразу начать чат; Loong больше не будет сам предлагать персонализацию, но при необходимости вы всё ещё можете вручную запустить `loong personalize`."
        }
        (Language::En, Some(StartupPersonalizationPreset::Later)) => {
            "start chatting now; run `loong personalize` later if you want to capture first-conversation preferences."
        }
        (Language::En, Some(StartupPersonalizationPreset::TurnOff)) => {
            "start chatting now; Loong will stop surfacing personalization prompts unless you later run `loong personalize` yourself."
        }
        _ => startup_onboarding_subtitle(state.stage, state.current_language()),
    }
}

fn startup_feedback_prefix(kind: &str, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match kind {
            "language_set" => "语言已设为",
            "provider_saved" => "provider 选择已保存：",
            "skills_none_selected" => "暂时还没有选任何 skill pack。",
            "skills_skipped" => "先跳过 skills；后面需要时 Loong 还能继续引导安装。",
            "setup_chat_now" => "先把更深的配置留到真正需要时再展开。",
            "setup_provider_web" => {
                "provider 与 web search 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            "setup_channels_delivery" => {
                "channels 与 delivery 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            "setup_mcp_skills" => {
                "MCP 与 workspace 的后续路径已经整理好了，下一步可以存一个首轮风格。"
            }
            _ => "",
        },
        Language::ZhTw => match kind {
            "language_set" => "語言已設為",
            "provider_saved" => "provider 選擇已保存：",
            "skills_none_selected" => "暫時還沒有選任何 skill pack。",
            "skills_skipped" => "先略過 skills；之後需要時 Loong 還能繼續引導安裝。",
            "setup_chat_now" => "先把更深的配置留到真正需要時再展開。",
            "setup_provider_web" => {
                "provider 與 web search 的後續路徑已整理好，下一步可以存一個首輪風格。"
            }
            "setup_channels_delivery" => {
                "channels 與 delivery 的後續路徑已整理好，下一步可以存一個首輪風格。"
            }
            "setup_mcp_skills" => "MCP 與 workspace 的後續路徑已整理好，下一步可以存一個首輪風格。",
            _ => "",
        },
        Language::Ja => match kind {
            "language_set" => "言語を設定:",
            "provider_saved" => "provider を保存:",
            "skills_none_selected" => "skill pack はまだ選択していません。",
            "skills_skipped" => {
                "skills はいったん保留です。必要になったら Loong があとで案内します。"
            }
            "setup_chat_now" => "深い設定は、最初の実際の作業で必要になるまで開きません。",
            "setup_provider_web" => {
                "provider と web search の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            "setup_channels_delivery" => {
                "channels と delivery の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            "setup_mcp_skills" => {
                "MCP と workspace の続き方は整理できました。次は最初の会話スタイルを保存できます。"
            }
            _ => "",
        },
        Language::Ru => match kind {
            "language_set" => "язык выбран:",
            "provider_saved" => "provider сохранён:",
            "skills_none_selected" => "пока не выбран ни один skill pack.",
            "skills_skipped" => {
                "skills пока пропущены. Когда понадобится, Loong подскажет установку позже."
            }
            "setup_chat_now" => {
                "глубокую настройку оставляем до момента, когда она понадобится в реальной работе."
            }
            "setup_provider_web" => {
                "маршрут для provider и web search уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            "setup_channels_delivery" => {
                "маршрут для channels и delivery уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            "setup_mcp_skills" => {
                "маршрут для MCP и workspace уже разложен. Дальше можно сохранить стиль первого разговора."
            }
            _ => "",
        },
        _ => match kind {
            "language_set" => "language set to",
            "provider_saved" => "provider choice saved:",
            "skills_none_selected" => "no skill packs selected yet.",
            "skills_skipped" => "skills skipped for now. Loong can guide installation later.",
            "setup_chat_now" => "keeping deeper setup deferred until the first real task needs it.",
            "setup_provider_web" => {
                "provider and web-search follow-up mapped. next step: save a first-chat style."
            }
            "setup_channels_delivery" => {
                "channel and delivery follow-up mapped. next step: save a first-chat style."
            }
            "setup_mcp_skills" => {
                "MCP and workspace follow-up mapped. next step: save a first-chat style."
            }
            _ => "",
        },
    }
}

fn startup_feedback_selected_skill_packs(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已选 {count} 个 skill pack。"),
        Language::ZhTw => format!("已選 {count} 個 skill pack。"),
        Language::Ja => format!("{count} 個の skill pack を選択しました。"),
        Language::Ru => format!("выбрано {count} skill pack."),
        _ => format!("selected {count} skill pack(s)."),
    }
}

fn startup_feedback_queued_skill_packs(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已加入 {count} 个 skill pack，后面仍可继续细调。"),
        Language::ZhTw => format!("已加入 {count} 個 skill pack，之後仍可繼續微調。"),
        Language::Ja => {
            format!("{count} 個の skill pack をキューに入れました。あとでまだ微調整できます。")
        }
        Language::Ru => {
            format!("{count} skill pack добавлены в очередь. Позже это можно ещё уточнить.")
        }
        _ => format!("{count} skill pack(s) queued. You can still refine this later."),
    }
}

fn startup_recommended_badge(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "推荐",
        Language::ZhTw => "推薦",
        Language::Ja => "おすすめ",
        Language::Ru => "рекомендуется",
        Language::En => "recommended",
    }
}

fn startup_personalization_footer_detail(language: Language) -> &'static str {
    match language {
        Language::ZhCn => {
            "Loong 会把这项写进 memory.personalization，只有当这套风格应该投射到 Session Profile 时，才会升级 memory.profile。"
        }
        Language::ZhTw => {
            "Loong 會把這項寫進 memory.personalization，只有當這套風格應該投射到 Session Profile 時，才會升級 memory.profile。"
        }
        Language::Ja => {
            "Loong はこれを memory.personalization に保存し、このスタイルを Session Profile に投影すべき時だけ memory.profile を上げます。"
        }
        Language::Ru => {
            "Loong сохраняет это в memory.personalization и повышает memory.profile только когда этот стиль должен проецироваться в Session Profile."
        }
        _ => {
            "Loong saves this into memory.personalization and only upgrades memory.profile when the saved style should project into Session Profile."
        }
    }
}

fn startup_no_preinstalled_skills_text(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "没有预装 skills",
        Language::ZhTw => "沒有預裝 skills",
        Language::Ja => "プリインストール skill なし",
        Language::Ru => "без предустановленных skills",
        Language::En => "no preinstalled skills",
    }
}

fn startup_selected_skill_count_text(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("已选 {count} 项"),
        Language::ZhTw => format!("已選 {count} 項"),
        Language::Ja => format!("{count} 件を選択"),
        Language::Ru => format!("выбрано {count}"),
        Language::En => format!("{count} selected"),
    }
}

fn startup_not_saved_text(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "未保存",
        Language::ZhTw => "未保存",
        Language::Ja => "未保存",
        Language::Ru => "не сохранено",
        Language::En => "not saved",
    }
}

fn startup_summary_label(kind: &str, language: Language) -> &'static str {
    match language {
        Language::ZhCn => match kind {
            "language" => "语言",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "后续路径",
            "personalization" => "个性化",
            _ => "",
        },
        Language::ZhTw => match kind {
            "language" => "語言",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "後續路徑",
            "personalization" => "個性化",
            _ => "",
        },
        Language::Ja => match kind {
            "language" => "言語",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "続きの設定",
            "personalization" => "会話スタイル",
            _ => "",
        },
        Language::Ru => match kind {
            "language" => "язык",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "дальше настроить",
            "personalization" => "стиль",
            _ => "",
        },
        _ => match kind {
            "language" => "language",
            "provider" => "provider",
            "skills" => "skills",
            "setup_path" => "setup path",
            "personalization" => "personalization",
            _ => "",
        },
    }
}

fn startup_finish_prompt(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "按 Enter 关闭引导并开始聊天。",
        Language::ZhTw => "按 Enter 關閉引導並開始聊天。",
        Language::Ja => "Enter でオンボードを閉じて会話を始めます。",
        Language::Ru => "Нажмите Enter, чтобы закрыть онбординг и начать чат.",
        Language::En => "press Enter to close onboarding and start chatting.",
    }
}

fn startup_eye_animation_for_state(state: Option<&StartupOnboardingState>) -> StartupEyeAnimation {
    let Some(state) = state else {
        return StartupEyeAnimation::Ambient;
    };

    let interaction_age = state.last_interaction_at.elapsed();
    let fresh_navigate = interaction_age < Duration::from_millis(380)
        && state.last_interaction_kind == StartupOnboardingInteractionKind::Navigate;
    let fresh_confirm = interaction_age < Duration::from_millis(640)
        && matches!(
            state.last_interaction_kind,
            StartupOnboardingInteractionKind::Confirm | StartupOnboardingInteractionKind::Persist
        );

    match state.stage {
        StartupOnboardingStage::Language => {
            let focus = if state.language_index == 0 {
                StartupEyeFocus::DownLeft
            } else {
                StartupEyeFocus::DownRight
            };
            if fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::Provider => {
            let focus = startup_list_focus(state.provider_index, state.provider_options.len());
            if fresh_confirm {
                StartupEyeAnimation::Confirm(focus)
            } else if fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::Skills => {
            let focus = startup_list_focus(state.skill_cursor, state.skill_options.len());
            if fresh_confirm {
                StartupEyeAnimation::Confirm(focus)
            } else if !state.selected_skill_ids.is_empty() || fresh_navigate {
                StartupEyeAnimation::Thinking(focus)
            } else {
                StartupEyeAnimation::Focus(focus)
            }
        }
        StartupOnboardingStage::SetupPath => match state.current_setup_path_choice() {
            StartupSetupPathChoice::ChatNow => {
                let focus = StartupEyeFocus::DownCenter;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupSetupPathChoice::ProviderAndWeb => {
                let focus = StartupEyeFocus::Right;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
            StartupSetupPathChoice::ChannelsAndDelivery => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
            StartupSetupPathChoice::McpAndSkills => {
                let focus = StartupEyeFocus::Left;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else {
                    StartupEyeAnimation::Thinking(focus)
                }
            }
        },
        StartupOnboardingStage::Personalization => match state.current_personalization_preset() {
            StartupPersonalizationPreset::Balanced => {
                let focus = StartupEyeFocus::DownCenter;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Concise => {
                let focus = StartupEyeFocus::Left;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Thorough => {
                let focus = StartupEyeFocus::Right;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::Later => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
            StartupPersonalizationPreset::TurnOff => {
                let focus = StartupEyeFocus::Up;
                if fresh_confirm {
                    StartupEyeAnimation::Confirm(focus)
                } else if fresh_navigate {
                    StartupEyeAnimation::Thinking(focus)
                } else {
                    StartupEyeAnimation::Focus(focus)
                }
            }
        },
        StartupOnboardingStage::Finish => StartupEyeAnimation::Celebrate,
    }
}

fn startup_list_focus(index: usize, total: usize) -> StartupEyeFocus {
    if total <= 1 {
        return StartupEyeFocus::DownCenter;
    }

    if index == 0 {
        StartupEyeFocus::DownLeft
    } else if index + 1 >= total {
        StartupEyeFocus::DownRight
    } else {
        StartupEyeFocus::DownCenter
    }
}

fn build_startup_onboarding_footer_line(
    state: &StartupOnboardingState,
    width: u16,
) -> Line<'static> {
    let text = startup_onboarding_footer_text_for_language(state.stage, state.current_language());
    Line::from(Span::styled(
        truncate_right_for_width(text, width as usize),
        Style::default().fg(SURFACE_GRAY),
    ))
}

fn render_startup_onboarding_lines(
    state: &StartupOnboardingState,
    width: u16,
) -> Vec<Line<'static>> {
    let content_width = width.max(24) as usize;
    let mut lines = Vec::new();
    let title = format!(
        "onboarding · {}/{} · {}",
        state.stage.step_index(),
        StartupOnboardingStage::total_steps(),
        state.stage.title(state.current_language())
    );
    lines.push(Line::from(Span::styled(
        truncate_right_for_width(title.as_str(), content_width),
        Style::default()
            .fg(SURFACE_ACCENT)
            .add_modifier(Modifier::BOLD),
    )));

    let subtitle = startup_onboarding_subtitle_for_state(state);
    lines.extend(render_onboarding_wrapped_line(
        "  ",
        subtitle,
        Style::default().fg(SURFACE_GRAY),
        Style::default().fg(SURFACE_GRAY),
        content_width,
    ));

    if let Some(feedback) = state.feedback.as_deref() {
        lines.push(Line::from(""));
        lines.extend(render_onboarding_wrapped_line(
            "✓ ",
            feedback,
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_GREEN),
            content_width,
        ));
    }

    lines.push(Line::from(""));
    match state.stage {
        StartupOnboardingStage::Language => {
            for (index, language) in state.language_options.iter().enumerate() {
                let selected = index == state.language_index;
                let label = startup_language_label(*language);
                lines.extend(render_onboarding_option_line(
                    selected,
                    label,
                    if *language == Language::En {
                        Some(startup_recommended_badge(state.current_language()))
                    } else {
                        None
                    },
                    content_width,
                ));
            }
        }
        StartupOnboardingStage::Provider => {
            for (index, option) in state.provider_options.iter().enumerate() {
                let selected = index == state.provider_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    option.label.as_str(),
                    option
                        .recommended
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        option.detail.as_str(),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }
        }
        StartupOnboardingStage::Skills => {
            for (index, option) in state.skill_options.iter().enumerate() {
                let selected = index == state.skill_cursor;
                let checked = state
                    .selected_skill_ids
                    .contains(option.install_id.as_str());
                let cursor = if selected { "›" } else { " " };
                let mark = if checked { "[x]" } else { "[ ]" };
                let badge = option
                    .recommended
                    .then_some(startup_recommended_badge(state.current_language()));
                let label = match badge {
                    Some(badge) => format!("{cursor} {mark} {} · {badge}", option.display_name),
                    None => format!("{cursor} {mark} {}", option.display_name),
                };
                lines.push(Line::from(Span::styled(
                    truncate_right_for_width(label.as_str(), content_width),
                    if selected {
                        Style::default()
                            .fg(SURFACE_CYAN)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                )));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        option.summary.as_str(),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }
        }
        StartupOnboardingStage::SetupPath => {
            for (index, choice) in StartupSetupPathChoice::ALL.iter().enumerate() {
                let selected = index == state.setup_path_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    choice.label(state.current_language()),
                    matches!(choice, StartupSetupPathChoice::ChatNow)
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        choice.detail(state.current_language()),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }

            lines.push(Line::from(""));
            for detail in startup_setup_path_detail_lines(state) {
                lines.extend(render_onboarding_wrapped_line(
                    "  • ",
                    detail.as_str(),
                    Style::default().fg(SURFACE_ACCENT),
                    Style::default().fg(Color::White),
                    content_width,
                ));
            }
        }
        StartupOnboardingStage::Personalization => {
            for (index, preset) in StartupPersonalizationPreset::ALL.iter().enumerate() {
                let selected = index == state.personalization_index;
                lines.extend(render_onboarding_option_line(
                    selected,
                    preset.label(state.current_language()),
                    matches!(preset, StartupPersonalizationPreset::Balanced)
                        .then_some(startup_recommended_badge(state.current_language())),
                    content_width,
                ));
                if selected {
                    lines.extend(render_onboarding_wrapped_line(
                        "    ",
                        preset.detail(state.current_language()),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        Style::default().fg(SURFACE_DIM_GRAY),
                        content_width,
                    ));
                }
            }

            lines.push(Line::from(""));
            lines.extend(render_onboarding_wrapped_line(
                "  ",
                startup_personalization_footer_detail(state.current_language()),
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_GRAY),
                content_width,
            ));
        }
        StartupOnboardingStage::Finish => {
            let language = startup_language_label(state.current_language());
            let provider = state
                .provider_options
                .get(state.provider_index)
                .map(|option| option.label.as_str())
                .unwrap_or("start fresh");
            let skills = if state.selected_skill_ids.is_empty() {
                startup_no_preinstalled_skills_text(state.current_language()).to_owned()
            } else {
                startup_selected_skill_count_text(
                    state.current_language(),
                    state.selected_skill_ids.len(),
                )
            };
            let setup_path = state
                .current_setup_path_choice()
                .label(state.current_language());
            let personalization = state
                .selected_personalization
                .map(|preset| preset.label(state.current_language()))
                .unwrap_or(startup_not_saved_text(state.current_language()));
            for summary in [
                format!(
                    "{} · {language}",
                    startup_summary_label("language", state.current_language())
                ),
                format!(
                    "{} · {provider}",
                    startup_summary_label("provider", state.current_language())
                ),
                format!(
                    "{} · {skills}",
                    startup_summary_label("skills", state.current_language())
                ),
                format!(
                    "{} · {setup_path}",
                    startup_summary_label("setup_path", state.current_language())
                ),
                format!(
                    "{} · {personalization}",
                    startup_summary_label("personalization", state.current_language())
                ),
            ] {
                lines.extend(render_onboarding_wrapped_line(
                    "  • ",
                    summary.as_str(),
                    Style::default().fg(SURFACE_ACCENT),
                    Style::default().fg(Color::White),
                    content_width,
                ));
            }
            lines.push(Line::from(""));
            lines.extend(render_onboarding_wrapped_line(
                "  ",
                startup_finish_prompt(state.current_language()),
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_GRAY),
                content_width,
            ));
        }
    }

    lines
}

fn startup_setup_path_detail_lines(state: &StartupOnboardingState) -> Vec<String> {
    let language = state.current_language();
    match state.current_setup_path_choice() {
        StartupSetupPathChoice::ChatNow => match language {
            Language::ZhCn => vec![
                "当前 splash/chat shell 会保持不变；更深的配置随时都能按需展开。".to_owned(),
                "如果你之后想走完整的 provider、web、channel、daemon 向导，可以再用 `loong onboard`。".to_owned(),
                "如果你想先看当前 runtime snapshot，也可以随时在空白输入框里用 /mcp 或 /skills。".to_owned(),
            ],
            Language::ZhTw => vec![
                "目前 splash/chat shell 會保持不變；更深的配置隨時都能按需展開。".to_owned(),
                "如果你之後想走完整的 provider、web、channel、daemon 嚮導，可以再用 `loong onboard`。".to_owned(),
                "如果你想先看目前 runtime snapshot，也可以隨時在空白輸入框裡用 /mcp 或 /skills。".to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                "The current splash/chat shell stays intact; deeper setup remains available on demand."
                    .to_owned(),
                "Use `loong onboard` later when you want the full provider, web, channel, and daemon wizard."
                    .to_owned(),
                "Use /mcp or /skills from the empty composer whenever you want the live runtime snapshot."
                    .to_owned(),
            ],
        },
        StartupSetupPathChoice::ProviderAndWeb => match language {
            Language::ZhCn => vec![
                format!(
                    "当前 provider 路径：{}。",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("当前 provider 凭证环境变量：{}。", env_name),
                    None => "当前 provider 还没有解析到可用的凭证环境变量。".to_owned(),
                },
                format!("当前 web 默认项：{}。", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "如果你要继续补 provider 的 base_url、model 或鉴权，下一步优先看 `loong doctor`。"
                        .to_owned()
                }),
                "完整的 provider / auth continuation 仍然在 `loong onboard`；这里会保持 shell 轻一点，只把真正的下一条命令露出来。"
                    .to_owned(),
            ],
            Language::ZhTw => vec![
                format!(
                    "目前 provider 路徑：{}。",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("目前 provider 憑證環境變數：{}。", env_name),
                    None => "目前 provider 還沒有解析到可用的憑證環境變數。".to_owned(),
                },
                format!("目前 web 預設項：{}。", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "如果你要繼續補 provider 的 base_url、model 或驗證，下一步優先看 `loong doctor`。"
                        .to_owned()
                }),
                "完整的 provider / auth continuation 仍然在 `loong onboard`；這裡會保持 shell 輕一點，只把真正的下一條命令露出來。"
                    .to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                format!(
                    "Provider lane now: {}.",
                    state
                        .provider_options
                        .get(state.provider_index)
                        .map(|option| option.label.as_str())
                        .unwrap_or("provider")
                ),
                match state.provider_auth_env_name.as_deref() {
                    Some(env_name) => format!("Provider auth env now: {}.", env_name),
                    None => "No provider credential env is resolved yet.".to_owned(),
                },
                format!("Web setup default: {}.", state.web_search_provider_label),
                state.web_search_provider_detail.clone(),
                state.provider_configuration_hint.clone().unwrap_or_else(|| {
                    "If you need to keep tuning provider base_url, model, or auth, `loong doctor` is the next check to run."
                        .to_owned()
                }),
                "Full provider/auth continuation lives in `loong onboard`; the TUI keeps the shell minimal and surfaces the real next command instead of opening a second startup UI."
                    .to_owned(),
            ],
        },
        StartupSetupPathChoice::ChannelsAndDelivery => {
            let has_enabled_channels = !state.enabled_channel_labels.is_empty();
            let suggested_channels = startup_suggested_channel_follow_up_descriptors(language);
            let mut lines = vec![match language {
                Language::ZhCn if state.enabled_channel_labels.is_empty() => {
                    "当前还没有启用任何外部 channel。".to_owned()
                }
                Language::ZhTw if state.enabled_channel_labels.is_empty() => {
                    "目前還沒有啟用任何外部 channel。".to_owned()
                }
                Language::ZhCn => format!(
                    "当前已启用的 channel：{}。",
                    state.enabled_channel_labels.join(", ")
                ),
                Language::ZhTw => format!(
                    "目前已啟用的 channel：{}。",
                    state.enabled_channel_labels.join(", ")
                ),
                Language::En | Language::Ja | Language::Ru
                    if state.enabled_channel_labels.is_empty() =>
                {
                    "No external channels are enabled yet.".to_owned()
                }
                Language::En | Language::Ja | Language::Ru => {
                    format!("Enabled channels now: {}.", state.enabled_channel_labels.join(", "))
                }
            }];
            if !has_enabled_channels && !suggested_channels.is_empty() {
                let suggested_labels = suggested_channels
                    .iter()
                    .map(|descriptor| descriptor.label.clone())
                    .collect::<Vec<_>>();
                lines.push(match language {
                    Language::ZhCn => {
                        format!("建议优先接的 channels：{}。", suggested_labels.join(", "))
                    }
                    Language::ZhTw => {
                        format!("建議優先接的 channels：{}。", suggested_labels.join(", "))
                    }
                    Language::Ja => {
                        format!("まず候補になる channels: {}。", suggested_labels.join(", "))
                    }
                    Language::Ru => {
                        format!("С чего обычно начинают: {}.", suggested_labels.join(", "))
                    }
                    Language::En => {
                        format!("Good first channels to wire: {}.", suggested_labels.join(", "))
                    }
                });

                let suggested_commands = suggested_channels
                    .iter()
                    .filter_map(|descriptor| descriptor.repair_command.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if !suggested_commands.is_empty() {
                    lines.push(match language {
                        Language::ZhCn => {
                            format!("建议先跑的 onboarding 命令：{}。", suggested_commands.join(", "))
                        }
                        Language::ZhTw => {
                            format!("建議先跑的 onboarding 命令：{}。", suggested_commands.join(", "))
                        }
                        Language::Ja => {
                            format!("先に試す onboarding コマンド: {}。", suggested_commands.join(", "))
                        }
                        Language::Ru => {
                            format!("Сначала стоит запустить: {}.", suggested_commands.join(", "))
                        }
                        Language::En => {
                            format!("Suggested onboarding commands: {}.", suggested_commands.join(", "))
                        }
                    });
                }
            }
            if state.channel_follow_up_commands.is_empty() {
                lines.push(match language {
                    Language::ZhCn => {
                        "当前还没有可直接运行的 channel runtime command；如果你要继续接 delivery surface，可以走 `loong onboard`。".to_owned()
                    }
                    Language::ZhTw => {
                        "目前還沒有可直接執行的 channel runtime command；如果你要繼續接 delivery surface，可以走 `loong onboard`。".to_owned()
                    }
                    Language::En | Language::Ja | Language::Ru => {
                        "No channel runtime command is ready yet; continue setup through `loong onboard` when you want to wire delivery surfaces.".to_owned()
                    }
                });
            } else if let Some(next_command) = state.channel_follow_up_commands.first() {
                lines.push(match language {
                    Language::ZhCn => format!("下一条 runtime command：{}。", next_command),
                    Language::ZhTw => format!("下一條 runtime command：{}。", next_command),
                    Language::En | Language::Ja | Language::Ru => {
                        format!("Next runtime command: {}.", next_command)
                    }
                });
                if let Some(other_commands) = state.channel_follow_up_commands.get(1..)
                    && !other_commands.is_empty()
                {
                    lines.push(match language {
                        Language::ZhCn => format!(
                            "其他 channel command：{}。",
                            other_commands.join(", ")
                        ),
                        Language::ZhTw => format!(
                            "其他 channel command：{}。",
                            other_commands.join(", ")
                        ),
                        Language::En | Language::Ja | Language::Ru => format!(
                            "Other channel commands: {}.",
                            other_commands.join(", ")
                        ),
                    });
                }
            }
            if let Some(status_command) = state.channel_status_commands.first() {
                lines.push(match language {
                    Language::ZhCn => format!("健康检查命令：{}。", status_command),
                    Language::ZhTw => format!("健康檢查命令：{}。", status_command),
                    Language::Ja => format!("ヘルス確認コマンド: {}。", status_command),
                    Language::Ru => format!("Команда проверки состояния: {}.", status_command),
                    Language::En => format!("Health command: {}.", status_command),
                });
            }
            if !state.channel_repair_commands.is_empty() {
                lines.push(match language {
                    Language::ZhCn => format!(
                        "修复路径：{}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::ZhTw => format!(
                        "修復路徑：{}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::Ja => format!(
                        "修復パス: {}。",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::Ru => format!(
                        "Путь восстановления: {}.",
                        state.channel_repair_commands.join(", ")
                    ),
                    Language::En => format!(
                        "Repair path: {}.",
                        state.channel_repair_commands.join(", ")
                    ),
                });
            }
            lines.push(match language {
                Language::ZhCn => {
                    "这条路径会先保持当前聊天面简洁，但把你下一步真正要继续的 channel / delivery 命令露出来。".to_owned()
                }
                Language::ZhTw => {
                    "這條路徑會先保持目前聊天面精簡，但把你下一步真正要繼續的 channel / delivery 命令露出來。".to_owned()
                }
                Language::En | Language::Ja | Language::Ru => {
                    "This path keeps chat focused now while surfacing the real channel and delivery commands you can continue with next.".to_owned()
                }
            });
            lines
        }
        StartupSetupPathChoice::McpAndSkills => match language {
            Language::ZhCn => vec![
                format!("当前可用的 bootstrap MCP server：{}。", state.startup_mcp_count),
                format!(
                    "当前可见的 bundled skill pack：{}（这轮已选 {} 个）。",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "如果你想看 live server list 用 /mcp；想看 pack inventory 用 /skills；想走更完整的 workspace/setup wizard 还是用 `loong onboard`。".to_owned(),
            ],
            Language::ZhTw => vec![
                format!("目前可用的 bootstrap MCP server：{}。", state.startup_mcp_count),
                format!(
                    "目前可見的 bundled skill pack：{}（這輪已選 {} 個）。",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "如果你想看 live server list 用 /mcp；想看 pack inventory 用 /skills；想走更完整的 workspace/setup wizard 還是用 `loong onboard`。".to_owned(),
            ],
            Language::En | Language::Ja | Language::Ru => vec![
                format!("Bootstrap MCP servers available now: {}.", state.startup_mcp_count),
                format!(
                    "Bundled skill packs visible now: {} ({} selected in this startup pass).",
                    state.detected_skill_count,
                    state.selected_skill_ids.len()
                ),
                "Use /mcp for the live server list, /skills for the pack inventory, or `loong onboard` if you want the broader workspace/setup wizard."
                    .to_owned(),
            ],
        },
    }
}

fn startup_web_search_detail(
    runtime: &CliTurnRuntime,
    provider: &str,
    language: Language,
) -> String {
    if runtime
        .config
        .tools
        .web_search
        .configured_api_key_for_provider(provider)
        .is_some()
    {
        let provider_label = web_search_provider_descriptor(provider)
            .map(|descriptor| descriptor.display_name)
            .unwrap_or(provider);
        return match language {
            Language::ZhCn => {
                format!(
                    "web provider 已就绪：{} 已在 tools.web_search 里配置好。",
                    provider_label
                )
            }
            Language::ZhTw => {
                format!(
                    "web provider 已就緒：{} 已在 tools.web_search 裡配置好。",
                    provider_label
                )
            }
            Language::Ja => format!(
                "web provider は準備できています: {} はすでに tools.web_search に設定されています。",
                provider_label
            ),
            Language::Ru => format!(
                "web provider готов: {} уже настроен в tools.web_search.",
                provider_label
            ),
            Language::En => format!(
                "Web provider ready: {} is already configured inside tools.web_search.",
                provider_label
            ),
        };
    }

    if let Some(env_name) = web_search_provider_api_key_env_names(provider)
        .iter()
        .find(|env_name| std::env::var_os(env_name).is_some())
    {
        let provider_label = web_search_provider_descriptor(provider)
            .map(|descriptor| descriptor.display_name)
            .unwrap_or(provider);
        return match language {
            Language::ZhCn => {
                format!("web provider 后续可以复用 {env_name}：{}。", provider_label)
            }
            Language::ZhTw => {
                format!("web provider 後續可以重用 {env_name}：{}。", provider_label)
            }
            Language::Ja => format!(
                "後で setup を続けるなら、web provider {} は {env_name} を再利用できます。",
                provider_label
            ),
            Language::Ru => format!(
                "Если потом продолжить setup, web provider {} сможет переиспользовать {env_name}.",
                provider_label
            ),
            Language::En => format!(
                "Web provider follow-up: {} can reuse {env_name} if you continue setup later.",
                provider_label
            ),
        };
    }

    let provider_label = web_search_provider_descriptor(provider)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(provider);
    match language {
        Language::ZhCn => format!(
            "web provider 后续默认会走 {}，但要继续 web 相关配置，鉴权还需要补上。",
            provider_label
        ),
        Language::ZhTw => format!(
            "web provider 後續預設會走 {}，但要繼續 web 相關配置，驗證還需要補上。",
            provider_label
        ),
        Language::Ja => format!(
            "web provider の既定は {} ですが、web 連携を先へ進めるには認証の配線がまだ必要です。",
            provider_label
        ),
        Language::Ru => format!(
            "По умолчанию web provider сейчас {}, но чтобы продолжить web-настройку, авторизацию ещё нужно подключить.",
            provider_label
        ),
        Language::En => format!(
            "Web provider follow-up: {} is the current default, but auth still needs to be wired before web-backed setup can go further.",
            provider_label
        ),
    }
}

fn startup_channel_follow_up_descriptors(
    runtime: &CliTurnRuntime,
    language: Language,
) -> Vec<StartupChannelFollowUpDescriptor> {
    service_channel_descriptors()
        .into_iter()
        .filter(|descriptor| channel_enabled_in_config(&runtime.config, descriptor.id))
        .filter_map(|descriptor| {
            let onboarding = resolve_channel_onboarding_descriptor(descriptor.id)?;
            Some(StartupChannelFollowUpDescriptor {
                label: startup_channel_label(descriptor.id, descriptor.label, language),
                serve_command: descriptor.serve_subcommand.map(str::to_owned),
                status_command: onboarding.status_command.to_owned(),
                repair_command: onboarding.repair_command.map(str::to_owned),
            })
        })
        .collect()
}

fn startup_suggested_channel_follow_up_descriptors(
    language: Language,
) -> Vec<StartupChannelFollowUpDescriptor> {
    preferred_startup_channel_ids(language)
        .iter()
        .filter_map(|channel_id| {
            let descriptor = service_channel_descriptors()
                .into_iter()
                .find(|descriptor| descriptor.id == *channel_id)?;
            let onboarding = resolve_channel_onboarding_descriptor(descriptor.id)?;
            Some(StartupChannelFollowUpDescriptor {
                label: startup_channel_label(descriptor.id, descriptor.label, language),
                serve_command: descriptor.serve_subcommand.map(str::to_owned),
                status_command: onboarding.status_command.to_owned(),
                repair_command: onboarding.repair_command.map(str::to_owned),
            })
        })
        .collect()
}

fn preferred_startup_channel_ids(language: Language) -> &'static [&'static str] {
    match language {
        Language::ZhCn | Language::ZhTw => &["feishu", "wecom", "dingtalk", "weixin"],
        Language::Ja => &["line", "telegram", "discord", "slack"],
        Language::Ru => &["telegram", "matrix", "discord", "slack"],
        Language::En => &["telegram", "matrix", "slack", "discord"],
    }
}

fn channel_enabled_in_config(config: &LoongConfig, channel_id: &str) -> bool {
    match channel_id {
        "cli" => config.cli.enabled,
        "telegram" => config.telegram.enabled,
        "feishu" => config.feishu.enabled,
        "matrix" => config.matrix.enabled,
        "wecom" => config.wecom.enabled,
        "weixin" => config.weixin.enabled,
        "qqbot" => config.qqbot.enabled,
        "onebot" => config.onebot.enabled,
        "discord" => config.discord.enabled,
        "line" => config.line.enabled,
        "dingtalk" => config.dingtalk.enabled,
        "webhook" => config.webhook.enabled,
        "slack" => config.slack.enabled,
        "google-chat" => config.google_chat.enabled,
        "mattermost" => config.mattermost.enabled,
        "nextcloud-talk" => config.nextcloud_talk.enabled,
        "synology-chat" => config.synology_chat.enabled,
        "irc" => config.irc.enabled,
        "imessage" => config.imessage.enabled,
        "signal" => config.signal.enabled,
        "whatsapp" => config.whatsapp.enabled,
        "teams" => config.teams.enabled,
        "twitch" => config.twitch.enabled,
        "nostr" => config.nostr.enabled,
        "tlon" => config.tlon.enabled,
        _ => false,
    }
}

fn startup_channel_label(channel_id: &str, fallback_label: &str, language: Language) -> String {
    match language {
        Language::ZhCn => match channel_id {
            "feishu" => "飞书".to_owned(),
            "dingtalk" => "钉钉".to_owned(),
            "wecom" => "企业微信".to_owned(),
            "weixin" => "微信".to_owned(),
            _ => fallback_label.to_owned(),
        },
        Language::ZhTw => match channel_id {
            "feishu" => "飛書".to_owned(),
            "dingtalk" => "釘釘".to_owned(),
            "wecom" => "企業微信".to_owned(),
            "weixin" => "微信".to_owned(),
            _ => fallback_label.to_owned(),
        },
        Language::En | Language::Ja | Language::Ru => fallback_label.to_owned(),
    }
}

fn startup_personalization_locale(language: Language) -> &'static str {
    match language {
        Language::En => "en-US",
        Language::ZhCn => "zh-CN",
        Language::ZhTw => "zh-TW",
        Language::Ja => "ja-JP",
        Language::Ru => "ru-RU",
    }
}

fn startup_first_turn_bootstrap_addendum(
    preset: StartupPersonalizationPreset,
    language: Language,
) -> Option<String> {
    if matches!(
        preset,
        StartupPersonalizationPreset::Later | StartupPersonalizationPreset::TurnOff
    ) {
        return None;
    }

    let instruction = match (preset, language) {
        (StartupPersonalizationPreset::Concise, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条非常简短、自然、友善的 assistant 消息确认该怎么称呼用户；",
            "如果语气允许，再顺带问一句 Loong 该叫什么。不要列清单，不要提到系统提示，问完就停下来等用户回答。"
        ),
        (StartupPersonalizationPreset::Concise, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條非常簡短、自然、友善的 assistant 訊息確認該怎麼稱呼使用者；",
            "如果語氣允許，再順帶問一句 Loong 該叫什麼。不要列清單，不要提到系統提示，問完就停下來等使用者回答。"
        ),
        (StartupPersonalizationPreset::Concise, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "ユーザーをどう呼べばよいかを確認してください。流れが自然なら、Loong の呼び名も一言だけ聞いてください。箇条書きや system prompt への言及は避け、聞いたら止まって返答を待ちます。"
        ),
        (StartupPersonalizationPreset::Concise, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь очень короткое, естественное и дружелюбное сообщение assistant и уточни, как обращаться к пользователю.",
            " Если это уместно, одной фразой спроси и как лучше звать Loong. Без чеклиста и без упоминания system prompt; после вопроса остановись и жди ответа."
        ),
        (StartupPersonalizationPreset::Concise, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one very short, natural, friendly assistant message that confirms how to address the user.",
            " If it feels natural, ask in one clause what Loong should be called too. No checklist, no system-prompt talk, and stop after asking."
        ),
        (StartupPersonalizationPreset::Thorough, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条自然、简短、友善的 assistant 消息确认：",
            "该怎么称呼用户；如果用户愿意，Loong 应该叫什么、整体气质想偏什么感觉、要不要带一个 emoji；如果和协作有关，也可以顺带说一下时区、长期边界，或者你想让 Loong 记住的一条备注。",
            "不要写成清单，不要提到系统提示或个性化流程，问完后停下来等待用户回答。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條自然、簡短、友善的 assistant 訊息確認：",
            "該怎麼稱呼使用者；如果使用者願意，Loong 應該叫什麼、整體氣質想偏什麼感覺、要不要帶一個 emoji；如果和協作有關，也可以順帶說一下時區、長期邊界，或是一條希望 Loong 記住的備註。",
            "不要寫成清單，不要提到系統提示或個性化流程，問完後停下來等待使用者回答。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "どう呼べばよいかを確認してください。望むなら Loong の呼び名、雰囲気、emoji の好みも一緒に聞いて構いません。協力に関係するなら、タイムゾーンや長く保ちたい境界、覚えておいてほしい短いメモも軽く聞いて構いません。",
            "箇条書きや system prompt への言及は避け、聞いたら止まって返答を待ちます。"
        ),
        (StartupPersonalizationPreset::Thorough, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь одно короткое, естественное и дружелюбное сообщение assistant и мягко уточни,",
            " как обращаться к пользователю; если ему хочется, как звать Loong, какой vibe ближе и нужен ли emoji. Если это помогает совместной работе, можно заодно спросить про timezone, постоянные boundaries или короткую заметку, которую Loong стоит помнить.",
            " Не оформляй это как чеклист и не упоминай system prompt или personalization workflow; после вопроса остановись и жди ответа."
        ),
        (StartupPersonalizationPreset::Thorough, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one short, natural, friendly assistant message that asks how to address the user and, if they want, what name, vibe, or emoji Loong should use. If it helps future collaboration, you may also invite a timezone, a long-lived boundary, or one short note Loong should remember.",
            " Do not present a checklist, do not mention system prompts or personalization workflow, and stop after asking so the user can answer."
        ),
        (_, Language::ZhCn) => concat!(
            "仅在下一次真正回复里生效：先不要直接解决任务。先用一条自然、简短、友善的 assistant 消息问清两件事：",
            "1）该怎么称呼用户；2）如果用户愿意，Loong 应该叫什么、整体气质/emoji 想要什么感觉。",
            "不要把问题写成清单，不要提到“系统提示”或“个性化流程”，也不要一次问太多。问完后停下来等待用户回答。"
        ),
        (_, Language::ZhTw) => concat!(
            "僅在下一次真正回覆裡生效：先不要直接解決任務。先用一條自然、簡短、友善的 assistant 訊息問清兩件事：",
            "1）該怎麼稱呼使用者；2）如果使用者願意，Loong 應該叫什麼、整體氣質/emoji 想要什麼感覺。",
            "不要把問題寫成清單，不要提到「系統提示」或「個性化流程」，也不要一次問太多。問完後停下來等待使用者回答。"
        ),
        (_, Language::Ja) => concat!(
            "次の実際の返信で一度だけ使います。いきなり作業を進めず、まず短く自然でやわらかい assistant メッセージを一つ送り、",
            "1) ユーザーをどう呼べばよいか、2) 望むなら Loong の呼び名や雰囲気、emoji の好みも教えてほしい、と聞いてください。",
            "箇条書きにはせず、system prompt や personalization workflow には触れず、聞き終えたら返答を待って止まってください。"
        ),
        (_, Language::Ru) => concat!(
            "Это действует только для следующего реального ответа. Не переходи сразу к задаче.",
            " Сначала отправь одно короткое, естественное и дружелюбное сообщение assistant, где мягко уточнишь:",
            " 1) как обращаться к пользователю; 2) если пользователю хочется, как звать Loong и какой vibe или emoji ему ближе.",
            " Не оформляй это как чеклист, не упоминай system prompt или personalization workflow, и после вопроса остановись в ожидании ответа."
        ),
        (_, Language::En) => concat!(
            "For the next real reply only, do not jump straight into the task.",
            " First send one short, natural, friendly assistant message that asks two things:",
            " how to address the user, and, if they want, what name, vibe, or emoji Loong should use.",
            " Do not present a checklist, do not mention system prompts or personalization workflow, and stop after asking so the user can answer."
        ),
    };

    Some(instruction.to_owned())
}

fn apply_first_turn_bootstrap_addendum(runtime: &mut CliTurnRuntime, addendum: String) {
    if addendum.trim().is_empty() {
        return;
    }

    runtime.config.cli.system_prompt_addendum = Some(addendum.clone());
    if !runtime.config.cli.uses_native_prompt_pack() {
        let base = runtime.config.cli.system_prompt.trim().to_owned();
        runtime.config.cli.system_prompt = if base.is_empty() {
            addendum
        } else {
            format!("{base}\n\n{addendum}")
        };
    }
}

fn maybe_capture_and_persist_first_turn_bootstrap_reply(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: &str,
) -> CliResult<()> {
    if !app.awaiting_first_turn_bootstrap_reply {
        return Ok(());
    }

    if detect_startup_bootstrap_reply_opt_out(input) {
        app.awaiting_first_turn_bootstrap_reply = false;
        return Ok(());
    }

    let Some(capture) = infer_startup_bootstrap_capture(input) else {
        return Ok(());
    };
    app.awaiting_first_turn_bootstrap_reply = false;
    persist_startup_bootstrap_capture(runtime, &capture)
}

fn detect_startup_bootstrap_reply_opt_out(input: &str) -> bool {
    let lowered = input.to_ascii_lowercase();
    [
        "skip for now",
        "skip this",
        "no preference",
        "up to you",
        "either is fine",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
        || [
            "跳过",
            "不用了",
            "随便",
            "隨便",
            "都可以",
            "先这样",
            "先這樣",
        ]
        .iter()
        .any(|pattern| input.contains(pattern))
}

fn infer_startup_bootstrap_capture(input: &str) -> Option<StartupBootstrapCapture> {
    let capture = StartupBootstrapCapture {
        preferred_address: extract_bootstrap_field_value(
            input,
            &[
                "you can call me ",
                "call me ",
                "address me as ",
                "叫我",
                "稱呼我",
                "称呼我",
            ],
        ),
        pronouns: extract_bootstrap_field_value(
            input,
            &[
                "my pronouns are ",
                "pronouns are ",
                "pronouns=",
                "代词是",
                "代詞是",
            ],
        ),
        agent_name: extract_bootstrap_field_value(
            input,
            &[
                "your name is ",
                "call yourself ",
                "you can be ",
                "你叫",
                "你可以叫",
                "loong叫",
                "loong 叫",
            ],
        ),
        creature: extract_bootstrap_field_value(
            input,
            &[
                "creature=",
                "creature is ",
                "species is ",
                "物种是",
                "物種是",
                "设定是",
                "設定是",
            ],
        ),
        vibe: extract_bootstrap_field_value(
            input,
            &[
                "vibe=",
                "vibe is ",
                "tone is ",
                "气质是",
                "氣質是",
                "风格是",
                "風格是",
            ],
        ),
        emoji: extract_bootstrap_field_value(
            input,
            &["emoji=", "emoji is ", "emoji 用", "emoji用", "表情是"],
        ),
        timezone: extract_bootstrap_field_value(
            input,
            &[
                "timezone=",
                "timezone is ",
                "my timezone is ",
                "time zone is ",
                "时区是",
                "時區是",
            ],
        ),
        standing_boundaries: extract_bootstrap_field_value(
            input,
            &[
                "boundaries are ",
                "boundary is ",
                "boundary=",
                "standing boundaries are ",
                "边界是",
                "界限是",
                "原則是",
                "原则是",
            ],
        ),
        notes: extract_bootstrap_field_value(
            input,
            &[
                "notes are ",
                "note is ",
                "note: ",
                "notes: ",
                "keep in mind ",
                "备注是",
                "備註是",
                "备注：",
                "備註：",
            ],
        ),
    };

    if capture == StartupBootstrapCapture::default() {
        None
    } else {
        Some(capture)
    }
}

fn extract_bootstrap_field_value(input: &str, patterns: &[&str]) -> Option<String> {
    for pattern in patterns {
        let value = if pattern.is_ascii() {
            extract_ascii_pattern_value(input, pattern)
        } else {
            extract_direct_pattern_value(input, pattern)
        };
        if value.is_some() {
            return value;
        }
    }
    None
}

fn extract_ascii_pattern_value(input: &str, pattern: &str) -> Option<String> {
    let haystack = input.to_ascii_lowercase();
    let index = haystack.find(pattern)?;
    let start = index + pattern.len();
    normalize_bootstrap_value(&input[start..])
}

fn extract_direct_pattern_value(input: &str, pattern: &str) -> Option<String> {
    let index = input.find(pattern)?;
    let start = index + pattern.len();
    normalize_bootstrap_value(&input[start..])
}

fn normalize_bootstrap_value(value: &str) -> Option<String> {
    let trimmed = value
        .trim_start_matches([' ', ':', '：', '=', '-', '—'])
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let end_index = trimmed
        .char_indices()
        .find_map(|(index, ch)| {
            if matches!(
                ch,
                ';' | '\n' | '。' | '，' | ',' | '.' | '!' | '?' | '！' | '？'
            ) {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(trimmed.len());
    let normalized = trimmed[..end_index]
        .trim()
        .trim_matches([
            '"', '\'', '“', '”', '「', '」', '『', '』', '。', '，', ',', '.',
        ])
        .trim();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.to_owned())
}

fn persist_startup_bootstrap_capture(
    runtime: &mut CliTurnRuntime,
    capture: &StartupBootstrapCapture,
) -> CliResult<()> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let personalization = config
        .memory
        .personalization
        .get_or_insert_with(PersonalizationConfig::default);
    if let Some(preferred_address) = capture.preferred_address.as_deref() {
        personalization.preferred_name = Some(preferred_address.to_owned());
    }
    if let Some(pronouns) = capture.pronouns.as_deref() {
        personalization.pronouns = Some(pronouns.to_owned());
    }
    if let Some(timezone) = capture.timezone.as_deref() {
        personalization.timezone = Some(timezone.to_owned());
    }
    if let Some(standing_boundaries) = capture.standing_boundaries.as_deref() {
        personalization.standing_boundaries = Some(standing_boundaries.to_owned());
    }
    if let Some(notes) = capture.notes.as_deref() {
        personalization.notes = Some(notes.to_owned());
    }
    if personalization.prompt_state == PersonalizationPromptState::Pending
        && (capture.preferred_address.is_some()
            || capture.pronouns.is_some()
            || capture.timezone.is_some()
            || capture.standing_boundaries.is_some()
            || capture.notes.is_some())
    {
        personalization.prompt_state = PersonalizationPromptState::Configured;
    }

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;

    let workspace_root = current_working_directory(runtime);
    persist_startup_bootstrap_runtime_self_files(workspace_root.as_path(), capture)
}

fn persist_startup_bootstrap_runtime_self_files(
    workspace_root: &Path,
    capture: &StartupBootstrapCapture,
) -> CliResult<()> {
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("USER.md").as_path(),
        "# User",
        render_bootstrap_user_block(capture),
    )?;
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("IDENTITY.md").as_path(),
        "# Identity",
        render_bootstrap_identity_block(capture),
    )?;
    upsert_bootstrap_runtime_self_file(
        workspace_root.join("SOUL.md").as_path(),
        "# Soul",
        render_bootstrap_soul_block(capture),
    )?;
    Ok(())
}

fn render_bootstrap_user_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(preferred_address) = capture.preferred_address.as_deref() {
        lines.push(format!("- Preferred address: {preferred_address}"));
    }
    if let Some(pronouns) = capture.pronouns.as_deref() {
        lines.push(format!("- Pronouns: {pronouns}"));
    }
    if let Some(timezone) = capture.timezone.as_deref() {
        lines.push(format!("- Timezone: {timezone}"));
    }
    if let Some(standing_boundaries) = capture.standing_boundaries.as_deref() {
        lines.push(format!("- Standing boundaries: {standing_boundaries}"));
    }
    if let Some(notes) = capture.notes.as_deref() {
        lines.push(format!("- Notes: {notes}"));
    }

    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

fn render_bootstrap_identity_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(agent_name) = capture.agent_name.as_deref() {
        lines.push(format!("- Name: {agent_name}"));
    }
    if let Some(creature) = capture.creature.as_deref() {
        lines.push(format!("- Creature: {creature}"));
    }
    if let Some(vibe) = capture.vibe.as_deref() {
        lines.push(format!("- Vibe: {vibe}"));
    }
    if let Some(emoji) = capture.emoji.as_deref() {
        lines.push(format!("- Emoji: {emoji}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn render_bootstrap_soul_block(capture: &StartupBootstrapCapture) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(vibe) = capture.vibe.as_deref() {
        lines.push(format!("- Preferred vibe: {vibe}"));
    }
    if let Some(emoji) = capture.emoji.as_deref() {
        lines.push(format!("- Signature emoji: {emoji}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn upsert_bootstrap_runtime_self_file(
    path: &Path,
    heading: &str,
    block_body: Option<String>,
) -> CliResult<()> {
    let Some(block_body) = block_body else {
        return Ok(());
    };
    let start_marker = "<!-- loong-bootstrap:start -->";
    let end_marker = "<!-- loong-bootstrap:end -->";
    let managed_block = format!("{start_marker}\n{block_body}\n{end_marker}");
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = if let Some(start) = existing.find(start_marker) {
        if let Some(end_relative) = existing[start..].find(end_marker) {
            let end = start + end_relative + end_marker.len();
            format!(
                "{}{}{}",
                &existing[..start],
                managed_block,
                &existing[end..]
            )
        } else {
            format!("{}\n\n{}", existing.trim_end(), managed_block)
        }
    } else if existing.trim().is_empty() {
        format!("{heading}\n\n{managed_block}\n")
    } else {
        format!("{}\n\n{}\n", existing.trim_end(), managed_block)
    };

    fs::write(path, updated).map_err(|error| {
        format!(
            "failed to write bootstrap runtime self file {}: {error}",
            path.display()
        )
    })
}

fn persist_startup_personalization(
    runtime: &mut CliTurnRuntime,
    preset: StartupPersonalizationPreset,
    language: Language,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let path = runtime.resolved_path.display().to_string();
    let now = OffsetDateTime::now_utc();
    let updated_at_epoch_seconds = u64::try_from(now.unix_timestamp()).ok();

    let message = if preset == StartupPersonalizationPreset::Later {
        config.memory.personalization = Some(PersonalizationConfig {
            prompt_state: PersonalizationPromptState::Deferred,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        match language {
            Language::ZhCn => "个性化暂时先不保存；Loong 会先保持首轮对话中性。".to_owned(),
            Language::ZhTw => "個性化暫時先不保存；Loong 會先保持首輪對話中性。".to_owned(),
            Language::En | Language::Ja | Language::Ru => {
                "personalization deferred; Loong will keep the first conversation neutral for now."
                    .to_owned()
            }
        }
    } else if preset == StartupPersonalizationPreset::TurnOff {
        config.memory.personalization = Some(PersonalizationConfig {
            prompt_state: PersonalizationPromptState::Suppressed,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        match language {
            Language::ZhCn => "已关闭后续个性化提示；Loong 会保持默认对话风格。".to_owned(),
            Language::ZhTw => "已關閉後續個性化提示；Loong 會保持預設對話風格。".to_owned(),
            Language::Ja => {
                "今後の個性化案内をオフにしました。Loong は標準の会話スタイルを保ちます。"
                    .to_owned()
            }
            Language::Ru => {
                "Дальнейшие подсказки по персонализации отключены; Loong сохранит стиль по умолчанию."
                    .to_owned()
            }
            Language::En => {
                "future personalization prompts turned off; Loong will keep the default conversation style."
                    .to_owned()
            }
        }
    } else {
        let mut upgraded_memory_profile = false;
        if config.memory.profile != MemoryProfile::ProfilePlusWindow {
            config.memory.profile = MemoryProfile::ProfilePlusWindow;
            upgraded_memory_profile = true;
        }
        config.memory.personalization = Some(PersonalizationConfig {
            response_density: preset.response_density(),
            initiative_level: preset.initiative_level(),
            locale: Some(startup_personalization_locale(language).to_owned()),
            prompt_state: PersonalizationPromptState::Configured,
            updated_at_epoch_seconds,
            ..PersonalizationConfig::default()
        });
        if upgraded_memory_profile {
            match language {
                Language::ZhCn => format!(
                    "已保存 {}，并把 memory.profile 升级为 profile_plus_window。",
                    preset.label(language)
                ),
                Language::ZhTw => format!(
                    "已保存 {}，並把 memory.profile 升級為 profile_plus_window。",
                    preset.label(language)
                ),
                Language::En | Language::Ja | Language::Ru => format!(
                    "saved {} and upgraded memory.profile to profile_plus_window.",
                    preset.label(language)
                ),
            }
        } else {
            match language {
                Language::ZhCn => {
                    format!("已把 {} 保存为首轮对话风格。", preset.label(language))
                }
                Language::ZhTw => {
                    format!("已把 {} 保存為首輪對話風格。", preset.label(language))
                }
                Language::En | Language::Ja | Language::Ru => format!(
                    "saved {} for the first real conversation.",
                    preset.label(language)
                ),
            }
        }
    };

    crate::config::write(Some(path.as_str()), &config, true)?;
    runtime.config = config;
    Ok(message)
}

fn render_onboarding_option_line(
    selected: bool,
    label: &str,
    badge: Option<&str>,
    content_width: usize,
) -> Vec<Line<'static>> {
    let prefix = if selected { "› " } else { "  " };
    let text = match badge {
        Some(badge) => format!("{label} · {badge}"),
        None => label.to_owned(),
    };
    vec![Line::from(Span::styled(
        truncate_right_for_width(format!("{prefix}{text}").as_str(), content_width),
        if selected {
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        },
    ))]
}

fn render_onboarding_wrapped_line(
    prefix: &str,
    text: &str,
    prefix_style: Style,
    body_style: Style,
    content_width: usize,
) -> Vec<Line<'static>> {
    let prefix_width = crate::presentation::display_width(prefix);
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped = crate::presentation::render_wrapped_plain_display_line(text, body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(prefix.to_owned(), prefix_style),
                    Span::styled(line, body_style),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" ".repeat(prefix_width)),
                    Span::styled(line, body_style),
                ])
            }
        })
        .collect()
}

fn paste_into_composer(app: &mut App, text: &str) {
    if text.is_empty() {
        return;
    }
    app.composer.insert_paste(text);
    app.focus = Focus::Composer;
    if app.pending_turn && !app.composer.is_empty() {
        app.composer_follow_up_intent = true;
    }
    app.sync_inline_skill_popup();
}

fn open_slash_command_palette(app: &mut App, prefix: char, query: &str) {
    let normalized_prefix = if prefix == ':' { ':' } else { '/' };
    app.command_palette.show_commands(query);
    app.composer
        .set_input(format!("{normalized_prefix}{}", query.trim()));
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
}

fn sync_slash_palette_composer(app: &mut App) {
    if !app.command_palette.is_commands_mode() {
        return;
    }
    let prefix = app
        .composer
        .text()
        .chars()
        .next()
        .filter(|ch| matches!(ch, '/' | ':'))
        .unwrap_or('/');
    app.composer
        .set_input(format!("{prefix}{}", app.command_palette.query_text()));
}

fn clear_slash_palette_composer(app: &mut App) {
    if app.command_palette.is_commands_mode()
        && app
            .composer
            .text()
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '/' | ':'))
    {
        app.composer.clear();
        app.composer_follow_up_intent = false;
    }
}

fn push_unique_model_candidate(out: &mut Vec<String>, model: &str) {
    let trimmed = model.trim();
    if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
        return;
    }
    out.push(trimmed.to_owned());
}

fn local_model_candidates(provider: &ProviderConfig) -> Vec<String> {
    let mut models = Vec::new();
    push_unique_model_candidate(&mut models, provider.model.as_str());
    for preferred in &provider.preferred_models {
        push_unique_model_candidate(&mut models, preferred.as_str());
    }
    if let Some(default_model) = provider.kind.default_model() {
        push_unique_model_candidate(&mut models, default_model);
    }
    if let Some(recommended_model) = provider.kind.recommended_onboarding_model() {
        push_unique_model_candidate(&mut models, recommended_model);
    }
    models
}

fn merged_model_catalog_entries(
    provider: &ProviderConfig,
    catalog: &[crate::provider::ProviderModelCatalogEntry],
    include_hidden_and_deprecated: bool,
) -> Vec<crate::provider::ProviderModelCatalogEntry> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for model in local_model_candidates(provider) {
        if seen.insert(model.clone()) {
            if let Some(entry) = catalog.iter().find(|entry| entry.model == model) {
                merged.push(entry.clone());
            } else {
                merged.push(crate::provider::ProviderModelCatalogEntry {
                    model,
                    display_name: None,
                    description: None,
                    is_default: false,
                    hidden: false,
                    deprecated: false,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: Vec::new(),
                    supported_reasoning_effort_descriptions: Vec::new(),
                });
            }
        }
    }

    for entry in catalog {
        if !include_hidden_and_deprecated && (entry.hidden || entry.deprecated) {
            continue;
        }
        if seen.insert(entry.model.clone()) {
            merged.push(entry.clone());
        }
    }

    merged
}

fn find_exact_model_catalog_entry<'a>(
    catalog: &'a [crate::provider::ProviderModelCatalogEntry],
    query: &str,
) -> Option<&'a crate::provider::ProviderModelCatalogEntry> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    catalog.iter().find(|entry| {
        entry.model.eq_ignore_ascii_case(query)
            || entry
                .display_name
                .as_deref()
                .is_some_and(|display_name| display_name.eq_ignore_ascii_case(query))
    })
}

fn model_entry_label(entry: &crate::provider::ProviderModelCatalogEntry) -> String {
    entry
        .display_name
        .clone()
        .unwrap_or_else(|| entry.model.clone())
}

fn model_entry_description(
    provider: &ProviderConfig,
    entry: &crate::provider::ProviderModelCatalogEntry,
    reasoning_efforts: &[ReasoningEffort],
) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = entry.display_name.as_deref()
        && !display_name.eq_ignore_ascii_case(entry.model.as_str())
    {
        parts.push(entry.model.clone());
    }
    if let Some(description) = entry.description.as_deref()
        && !description.is_empty()
    {
        parts.push(description.to_owned());
    }
    if entry.is_default {
        parts.push("catalog default".to_owned());
    }
    if entry.hidden {
        parts.push("hidden from default picker".to_owned());
    }
    if entry.deprecated {
        parts.push("deprecated".to_owned());
    }
    if let Some(default_effort) =
        crate::provider::effective_default_reasoning_effort_for_entry(provider, entry)
    {
        parts.push(format!("default {}", default_effort.as_str()));
    }

    match reasoning_efforts {
        [] => parts.push("apply immediately".to_owned()),
        [only_effort] => parts.push(format!("apply {} immediately", only_effort.as_str())),
        _ => parts.push("choose reasoning next".to_owned()),
    }

    parts.join(" · ")
}

fn current_reasoning_label(runtime: &CliTurnRuntime) -> String {
    runtime
        .config
        .provider
        .reasoning_effort
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_else(|| "default".to_owned())
}

fn reasoning_option_description(reasoning_effort: Option<ReasoningEffort>) -> String {
    match reasoning_effort {
        None => "use the provider or model default reasoning behavior".to_owned(),
        Some(ReasoningEffort::None) => {
            "disable explicit reasoning effort for this model".to_owned()
        }
        Some(ReasoningEffort::Minimal) => "keep reasoning as light as possible".to_owned(),
        Some(ReasoningEffort::Low) => "favor quick turns with light reasoning".to_owned(),
        Some(ReasoningEffort::Medium) => "balance speed and deeper reasoning".to_owned(),
        Some(ReasoningEffort::High) => "prefer deeper reasoning for harder turns".to_owned(),
        Some(ReasoningEffort::Xhigh) => {
            "maximize reasoning depth when the provider supports it".to_owned()
        }
    }
}

fn reasoning_option_description_for_entry(
    entry: &crate::provider::ProviderModelCatalogEntry,
    reasoning_effort: ReasoningEffort,
) -> String {
    crate::provider::reasoning_effort_description_for_entry(entry, reasoning_effort)
        .map(str::to_owned)
        .unwrap_or_else(|| reasoning_option_description(Some(reasoning_effort)))
}

fn default_reasoning_option_description(
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) -> String {
    crate::provider::effective_default_reasoning_effort_for_entry(&runtime.config.provider, entry)
        .map(|effort| {
            let detail = reasoning_option_description_for_entry(entry, effort);
            format!(
                "use the model default reasoning behavior ({} · {})",
                effort.as_str(),
                detail
            )
        })
        .unwrap_or_else(|| "use the provider or model default reasoning behavior".to_owned())
}

fn build_model_palette_entries(
    runtime: &CliTurnRuntime,
    catalog: &[crate::provider::ProviderModelCatalogEntry],
) -> Vec<SettingsEntry> {
    let provider = &runtime.config.provider;
    let current_model = provider.model.trim();
    let default_model = provider.kind.default_model();
    let configured_auto_models = provider.configured_auto_model_candidates();

    let mut ordered = catalog.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        let left_model = left.model.trim();
        let right_model = right.model.trim();
        let left_rank = (
            usize::from(left_model != current_model),
            usize::from(
                !configured_auto_models
                    .iter()
                    .any(|candidate| candidate == left_model),
            ),
            usize::from(Some(left_model) != default_model && !left.is_default),
            usize::from(left.hidden),
            usize::from(left.deprecated),
        );
        let right_rank = (
            usize::from(right_model != current_model),
            usize::from(
                !configured_auto_models
                    .iter()
                    .any(|candidate| candidate == right_model),
            ),
            usize::from(Some(right_model) != default_model && !right.is_default),
            usize::from(right.hidden),
            usize::from(right.deprecated),
        );
        left_rank
            .cmp(&right_rank)
            .then_with(|| model_entry_label(left).cmp(&model_entry_label(right)))
            .then_with(|| left.model.cmp(&right.model))
    });

    ordered
        .into_iter()
        .map(|entry| {
            let trimmed = entry.model.trim();
            let is_current = trimmed == current_model;
            let status_tag = if is_current {
                Some("current".to_owned())
            } else if entry.is_default {
                Some("default".to_owned())
            } else if entry.deprecated {
                Some("deprecated".to_owned())
            } else if entry.hidden {
                Some("hidden".to_owned())
            } else if Some(trimmed) == default_model {
                Some("default".to_owned())
            } else if configured_auto_models
                .iter()
                .any(|candidate| candidate == trimmed)
            {
                Some("preferred".to_owned())
            } else {
                None
            };
            let reasoning_efforts =
                crate::provider::effective_supported_reasoning_efforts_for_entry(provider, entry);
            let description =
                model_entry_description(provider, entry, reasoning_efforts.as_slice());
            let action = if reasoning_efforts.is_empty() {
                CommandAction::ApplyModelSelection {
                    model: trimmed.to_owned(),
                    reasoning_effort: None,
                }
            } else if reasoning_efforts.len() == 1 {
                CommandAction::ApplyModelSelection {
                    model: trimmed.to_owned(),
                    reasoning_effort: reasoning_efforts.first().copied(),
                }
            } else {
                CommandAction::OpenModelReasoning(entry.clone())
            };
            SettingsEntry {
                label: model_entry_label(entry),
                category_tag: "[Model]".to_owned(),
                status_tag,
                description,
                action,
                selectable: true,
            }
        })
        .collect()
}

fn build_reasoning_palette_entries(
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) -> (Vec<SettingsEntry>, String) {
    let supported = crate::provider::effective_supported_reasoning_efforts_for_entry(
        &runtime.config.provider,
        entry,
    );
    let selected_label = runtime
        .config
        .provider
        .reasoning_effort
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_else(|| "default".to_owned());

    let mut entries = vec![SettingsEntry {
        label: "default".to_owned(),
        category_tag: "[Reasoning]".to_owned(),
        status_tag: (runtime.config.provider.model == entry.model
            && runtime.config.provider.reasoning_effort.is_none())
        .then(|| "current".to_owned()),
        description: default_reasoning_option_description(runtime, entry),
        action: CommandAction::ApplyModelSelection {
            model: entry.model.clone(),
            reasoning_effort: None,
        },
        selectable: true,
    }];

    for effort in supported {
        entries.push(SettingsEntry {
            label: effort.as_str().to_owned(),
            category_tag: "[Reasoning]".to_owned(),
            status_tag: (runtime.config.provider.model == entry.model
                && runtime.config.provider.reasoning_effort == Some(effort))
            .then(|| "current".to_owned()),
            description: reasoning_option_description_for_entry(entry, effort),
            action: CommandAction::ApplyModelSelection {
                model: entry.model.clone(),
                reasoning_effort: Some(effort),
            },
            selectable: true,
        });
    }

    (entries, selected_label)
}

async fn open_model_palette(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    query: &str,
) -> CliResult<()> {
    let (catalog, status) = match crate::provider::fetch_model_catalog(&runtime.config).await {
        Ok(catalog) => {
            let count = catalog.len();
            (
                catalog,
                Some(format!(
                    "{count} models available for {}",
                    runtime.config.provider.kind.display_name()
                )),
            )
        }
        Err(error) => (
            merged_model_catalog_entries(&runtime.config.provider, &[], false),
            Some(format!(
                "model catalog unavailable; showing local candidates ({error})"
            )),
        ),
    };
    let exact_catalog =
        merged_model_catalog_entries(&runtime.config.provider, catalog.as_slice(), true);
    if let Some(entry) = find_exact_model_catalog_entry(exact_catalog.as_slice(), query) {
        let reasoning_efforts = crate::provider::effective_supported_reasoning_efforts_for_entry(
            &runtime.config.provider,
            entry,
        );
        if reasoning_efforts.is_empty() {
            apply_model_selection(app, runtime, entry.model.clone(), None)?;
            return Ok(());
        }
        if reasoning_efforts.len() == 1 {
            apply_model_selection(
                app,
                runtime,
                entry.model.clone(),
                reasoning_efforts.first().copied(),
            )?;
            return Ok(());
        }
        open_reasoning_palette(app, runtime, entry);
        return Ok(());
    }
    let merged = merged_model_catalog_entries(&runtime.config.provider, catalog.as_slice(), false);
    let entries = build_model_palette_entries(runtime, merged.as_slice());
    app.command_palette.show_model_selector(
        entries,
        status,
        Some(runtime.config.provider.model.as_str()),
        query,
    );
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
    app.composer.clear();
    Ok(())
}

fn open_reasoning_palette(
    app: &mut App,
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) {
    let (entries, selected_label) = build_reasoning_palette_entries(runtime, entry);
    app.command_palette.show_reasoning_selector(
        entry.model.as_str(),
        entries,
        Some(format!(
            "Current reasoning: {} · Enter apply · Esc back",
            current_reasoning_label(runtime)
        )),
        Some(selected_label.as_str()),
    );
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
}

fn apply_model_selection(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    model: String,
    reasoning_effort: Option<ReasoningEffort>,
) -> CliResult<()> {
    let _ = persist_runtime_settings(runtime, app, |config| {
        config.provider.model = model.clone();
        config.provider.reasoning_effort = reasoning_effort;
        Ok(format!(
            "model switched to {} · reasoning {}",
            model,
            reasoning_effort
                .map(|effort| effort.as_str().to_owned())
                .unwrap_or_else(|| "default".to_owned())
        ))
    })?;
    app.inline_skill_popup_active = false;
    app.focus = Focus::Composer;
    app.composer.clear();
    Ok(())
}

async fn run_surface_command<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    options: &CliChatOptions,
    input: &str,
) -> CliResult<()> {
    let trimmed = input.trim();
    let (command, args) = split_surface_command(trimmed);
    let width = current_render_width(terminal)?;

    match command {
        "/clear" => {
            app.message_list.clear_transcript();
            app.focus = Focus::Composer;
            Ok(())
        }
        "/new" => {
            app.message_list.clear_transcript();
            app.message_list
                .add_rendered_lines(render_new_conversation_lines_with_width(width));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/copy" => {
            let copy_result = copy_command_text(app, args)
                .and_then(|text| copy_to_system_clipboard(text.as_str()).map(|()| text));
            app.message_list
                .add_rendered_lines(render_copy_command_lines_with_width(copy_result, width));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/diff" => {
            let cwd = current_working_directory(runtime);
            let lines = render_git_diff_command_lines_with_width(cwd.as_path(), width);
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
        "/export" | "/share" => {
            let cwd = current_working_directory(runtime);
            let markdown = app.message_list.export_markdown();
            let result = write_transcript_export(
                cwd.as_path(),
                runtime.session_id.as_str(),
                command.trim_start_matches('/'),
                markdown.as_str(),
            );
            app.message_list
                .add_rendered_lines(render_export_command_lines_with_width(
                    command, result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/import" => {
            if args.trim().is_empty() {
                let lines = build_command_lines(runtime, options, input, width).await?;
                app.message_list.add_rendered_lines(lines);
            } else {
                let cwd = current_working_directory(runtime);
                let result = import_context_into_composer(app, cwd.as_path(), args);
                app.message_list
                    .add_rendered_lines(render_import_command_lines_with_width(result, width));
            }
            app.focus = Focus::Composer;
            Ok(())
        }
        "/simplify" => {
            let result = stage_simplify_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "simplify", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/plan" => {
            let result = stage_plan_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "plan", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/title" | "/rename" => {
            if !args.trim().is_empty() {
                app.title = Some(args.trim().to_owned());
            }
            let lines = render_title_command_lines_with_width(command, args, width);
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
        "/feedback" => {
            let result = stage_feedback_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "feedback", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/model" => open_model_palette(app, runtime, args).await,
        "/settings" if args.trim().is_empty() => {
            open_settings_palette(
                app,
                runtime,
                SettingsSurfaceFocus::Overview,
                width,
                None,
                None,
            );
            Ok(())
        }
        "/settings" if !args.trim().is_empty() => {
            let action = parse_settings_command_action(args)?;
            let _ = dispatch_palette_action(app, runtime, width, action)?;
            Ok(())
        }
        _ => {
            let lines = build_command_lines(runtime, options, input, width).await?;
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
    }
}

fn split_surface_command(input: &str) -> (&str, &str) {
    let trimmed = input.trim();
    if let Some((command, rest)) = trimmed.split_once(char::is_whitespace) {
        (command, rest.trim())
    } else {
        (trimmed, "")
    }
}

fn is_known_surface_command(command: &str) -> bool {
    match command {
        super::super::CLI_CHAT_HELP_COMMAND
        | super::super::CLI_CHAT_STATUS_COMMAND
        | super::super::CLI_CHAT_HISTORY_COMMAND
        | super::super::CLI_CHAT_COMPACT_COMMAND
        | "/model"
        | "/settings"
        | "/permissions"
        | "/experimental"
        | "/themes"
        | "/cwd"
        | "/language"
        | "/mcp"
        | "/skills"
        | "/usage"
        | "/sessions"
        | "/subagents"
        | "/missions"
        | "/mission"
        | "/clear"
        | "/new"
        | "/copy"
        | "/diff"
        | "/export"
        | "/share"
        | "/import"
        | "/simplify"
        | "/plan"
        | "/title"
        | "/rename"
        | "/feedback"
        | "/exit" => true,
        _ => slash_command_specs()
            .iter()
            .any(|spec| spec.command == command),
    }
}

fn recognized_surface_command(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('/') || trimmed.starts_with(':')) {
        return None;
    }
    let normalized = if trimmed.starts_with(':') {
        format!("/{}", trimmed.trim_start_matches(':'))
    } else {
        trimmed.to_owned()
    };
    let (command, _) = split_surface_command(normalized.as_str());
    is_known_surface_command(command).then_some(normalized)
}

fn parse_settings_command_action(args: &str) -> Result<CommandAction, String> {
    let tokens = args.split_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        [] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Overview)),
        ["provider"] | ["web"] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Provider)),
        ["workspace"] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Workspace)),
        ["provider", raw_kind] => crate::config::parse_provider_kind_id(raw_kind)
            .map(|kind| CommandAction::ApplySettings(SettingsCommandAction::SetProvider(kind)))
            .ok_or_else(|| format!("unknown provider `{raw_kind}`; use `/settings` to inspect the current setup")),
        ["web", raw_provider] => normalize_web_search_provider(raw_provider)
            .map(|provider| {
                CommandAction::ApplySettings(SettingsCommandAction::SetWebProvider(
                    provider.to_owned(),
                ))
            })
            .ok_or_else(|| format!("unknown web.search provider `{raw_provider}`")),
        ["skills", "install", target_id] => {
            Ok(CommandAction::ApplySettings(
                SettingsCommandAction::InstallSkillPack((*target_id).to_owned()),
            ))
        }
        ["skills", "remove", target_id] | ["skills", "uninstall", target_id] => {
            Ok(CommandAction::ApplySettings(
                SettingsCommandAction::RemoveSkillPack((*target_id).to_owned()),
            ))
        }
        _ => Err(
            "usage: /settings [provider [id] | web [provider] | skills [install|remove <target>] | workspace]"
                .to_owned(),
        ),
    }
}

fn apply_settings_command(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    action: SettingsCommandAction,
) -> CliResult<(SettingsSurfaceFocus, String, String)> {
    match action {
        SettingsCommandAction::SetProvider(kind) => {
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.provider = startup_provider_config_for_kind(kind);
                Ok(format!("provider switched to {}", kind.display_name()))
            })?;
            Ok((
                SettingsSurfaceFocus::Provider,
                summary,
                kind.display_name().to_owned(),
            ))
        }
        SettingsCommandAction::SetWebProvider(provider) => {
            let provider_for_summary = provider.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.tools.web_search.enabled = true;
                config.tools.web_search.default_provider = provider.clone();
                if let Some(env_name) = web_search_provider_api_key_env_names(provider.as_str())
                    .iter()
                    .find(|env_name| std::env::var_os(env_name).is_some())
                {
                    let _ = config.tools.web_search.set_configured_api_key_for_provider(
                        provider.as_str(),
                        Some(format!("${{{}}}", env_name)),
                    );
                    Ok(format!(
                        "web-search provider switched to {} using {}",
                        provider_for_summary, env_name
                    ))
                } else {
                    Ok(format!(
                        "web-search provider switched to {}; credentials still need wiring",
                        provider_for_summary
                    ))
                }
            })?;
            let label = web_search_provider_descriptor(provider.as_str())
                .map(|descriptor| descriptor.display_name.to_owned())
                .unwrap_or(provider);
            Ok((SettingsSurfaceFocus::Provider, summary, label))
        }
        SettingsCommandAction::InstallSkillPack(target_id) => {
            let resolved_path = runtime.resolved_path.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.external_skills.enabled = true;
                config.external_skills.auto_expose_installed = true;
                if config.external_skills.install_root.is_none() {
                    let install_root = resolved_path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .map(|parent| parent.join("external-skills-installed"))
                        .unwrap_or_else(|| PathBuf::from("external-skills-installed"));
                    config.external_skills.install_root = Some(install_root.display().to_string());
                }
                let runtime_config =
                    crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
                        config,
                        Some(resolved_path.as_path()),
                    );
                let mut selected = BTreeSet::new();
                selected.insert(target_id.clone());
                let installed = crate::tools::install_bundled_preinstall_targets_for_bootstrap(
                    &runtime_config,
                    &selected,
                )?;
                if installed.is_empty() {
                    Ok(format!(
                        "skill pack `{target_id}` was already present in the managed runtime"
                    ))
                } else {
                    Ok(format!(
                        "installed managed skill pack `{target_id}`: {}",
                        installed.join(", ")
                    ))
                }
            })?;
            let label = bundled_preinstall_targets()
                .iter()
                .find(|target| target.install_id == target_id.as_str())
                .map(|target| target.display_name.to_owned())
                .unwrap_or(target_id);
            Ok((SettingsSurfaceFocus::Workspace, summary, label))
        }
        SettingsCommandAction::RemoveSkillPack(target_id) => {
            let resolved_path = runtime.resolved_path.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                let runtime_config =
                    crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
                        config,
                        Some(resolved_path.as_path()),
                    );
                let mut selected = BTreeSet::new();
                selected.insert(target_id.clone());
                let removed = crate::tools::remove_bundled_preinstall_targets_for_bootstrap(
                    &runtime_config,
                    &selected,
                )?;
                if removed.is_empty() {
                    Ok(format!(
                        "skill pack `{target_id}` was already absent from the managed runtime"
                    ))
                } else {
                    Ok(format!(
                        "removed managed skill pack `{target_id}`: {}",
                        removed.join(", ")
                    ))
                }
            })?;
            let label = bundled_preinstall_targets()
                .iter()
                .find(|target| target.install_id == target_id.as_str())
                .map(|target| target.display_name.to_owned())
                .unwrap_or(target_id);
            Ok((SettingsSurfaceFocus::Workspace, summary, label))
        }
    }
}

fn dispatch_palette_action(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    width: usize,
    action: CommandAction,
) -> CliResult<Option<String>> {
    let should_clear_slash_buffer = app.command_palette.is_commands_mode();
    match action {
        CommandAction::RunCommand(command) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(Some(command.to_owned()))
        }
        CommandAction::OpenSettings(focus) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            open_settings_palette(app, runtime, focus, width, None, None);
            Ok(None)
        }
        CommandAction::ApplySettings(action) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            let (focus, summary, selected_label) = apply_settings_command(app, runtime, action)?;
            open_settings_palette(
                app,
                runtime,
                focus,
                width,
                Some(summary),
                Some(selected_label.as_str()),
            );
            Ok(None)
        }
        CommandAction::OpenModelReasoning(entry) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            open_reasoning_palette(app, runtime, &entry);
            Ok(None)
        }
        CommandAction::ApplyModelSelection {
            model,
            reasoning_effort,
        } => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            apply_model_selection(app, runtime, model, reasoning_effort)?;
            Ok(None)
        }
        CommandAction::Noop => Ok(None),
        CommandAction::InsertText(text) => {
            if let Some(range) = current_skill_token_range(&app.composer) {
                let replacement =
                    inline_skill_replacement_text(app.composer.text(), &range, text.as_str());
                app.composer.replace_range(range, replacement.as_str());
            } else {
                app.composer.set_input(text);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(None)
        }
        CommandAction::Close => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(None)
        }
    }
}

fn open_settings_palette(
    app: &mut App,
    runtime: &CliTurnRuntime,
    focus: SettingsSurfaceFocus,
    width: usize,
    status: Option<String>,
    selected_label: Option<&str>,
) {
    let entries = build_settings_palette_entries(runtime, focus, width);
    app.command_palette
        .show_settings(focus, entries, status, selected_label);
    app.focus = Focus::CommandPalette;
    app.inline_skill_popup_active = false;
}

fn build_settings_palette_entries(
    runtime: &CliTurnRuntime,
    focus: SettingsSurfaceFocus,
    width: usize,
) -> Vec<SettingsEntry> {
    let runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &runtime.config,
        Some(runtime.resolved_path.as_path()),
    );
    let installed_skill_ids =
        crate::tools::installed_managed_skill_ids_for_bootstrap(&runtime_config)
            .unwrap_or_default();
    if focus == SettingsSurfaceFocus::Overview {
        return build_settings_overview_entries(runtime, width, &installed_skill_ids);
    }

    let provider_focus = focus == SettingsSurfaceFocus::Provider;
    let workspace_focus = focus == SettingsSurfaceFocus::Workspace;
    let mut entries = Vec::new();

    if provider_focus {
        let current_auth = runtime
            .config
            .provider
            .resolved_auth_env_name()
            .unwrap_or_else(|| "still needs credentials".to_owned());
        entries.push(SettingsEntry {
            label: "Current provider".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: format!(
                "{} · model {} · auth {}",
                runtime.config.provider.kind.display_name(),
                runtime.config.provider.model,
                current_auth
            ),
            action: CommandAction::Noop,
            selectable: false,
        });
        entries.push(SettingsEntry {
            label: "Back to settings".to_owned(),
            category_tag: "[Navigation]".to_owned(),
            status_tag: None,
            description: "return to the top-level settings overview".to_owned(),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Overview),
            selectable: true,
        });
        let current_provider = runtime.config.provider.kind;
        let mut provider_kinds = ProviderKind::all_sorted().to_vec();
        provider_kinds.sort_by_key(|kind| {
            let is_current = *kind == current_provider;
            let is_ready = detected_startup_auth_binding(*kind).is_some();
            (
                usize::from(!is_current),
                usize::from(!is_ready && !is_current),
                kind.display_name(),
            )
        });
        for kind in provider_kinds {
            let is_current = runtime.config.provider.kind == kind;
            let (status_tag, description) =
                render_provider_settings_entry(runtime, kind, is_current);
            entries.push(SettingsEntry {
                label: kind.display_name().to_owned(),
                category_tag: "[Provider]".to_owned(),
                status_tag,
                description,
                action: CommandAction::ApplySettings(SettingsCommandAction::SetProvider(kind)),
                selectable: true,
            });
        }
        let current_web_provider = normalize_web_search_provider(
            runtime.config.tools.web_search.default_provider.as_str(),
        )
        .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
        let mut web_descriptors = crate::config::web_search_provider_descriptors().to_vec();
        web_descriptors.sort_by_key(|descriptor| {
            let is_current = descriptor.id == current_web_provider;
            let is_ready = web_search_provider_env_api_key_name(descriptor.id).is_some()
                || runtime
                    .config
                    .tools
                    .web_search
                    .configured_api_key_for_provider(descriptor.id)
                    .is_some();
            (
                usize::from(!is_current),
                usize::from(!is_ready && !is_current),
                descriptor.display_name,
            )
        });
        entries.push(SettingsEntry {
            label: "Current web search".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: render_current_web_search_summary(runtime),
            action: CommandAction::Noop,
            selectable: false,
        });
        for descriptor in web_descriptors {
            let is_current = runtime.config.tools.web_search.default_provider == descriptor.id;
            let (status_tag, description) = render_web_provider_settings_entry(
                runtime,
                descriptor.id,
                descriptor.display_name,
                is_current,
            );
            entries.push(SettingsEntry {
                label: descriptor.display_name.to_owned(),
                category_tag: "[Web]".to_owned(),
                status_tag,
                description,
                action: CommandAction::ApplySettings(SettingsCommandAction::SetWebProvider(
                    descriptor.id.to_owned(),
                )),
                selectable: true,
            });
        }
    }

    if workspace_focus {
        let installed_pack_count = bundled_preinstall_targets()
            .iter()
            .filter(|target| {
                target
                    .skill_ids
                    .iter()
                    .all(|skill_id| installed_skill_ids.contains(*skill_id))
            })
            .count();
        entries.push(SettingsEntry {
            label: "Current workspace".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: format!(
                "{} bootstrap MCP · {} installed packs",
                runtime.effective_bootstrap_mcp_servers.len(),
                installed_pack_count
            ),
            action: CommandAction::Noop,
            selectable: false,
        });
        entries.push(SettingsEntry {
            label: "Back to settings".to_owned(),
            category_tag: "[Navigation]".to_owned(),
            status_tag: None,
            description: "return to the top-level settings overview".to_owned(),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Overview),
            selectable: true,
        });
        let mut targets = bundled_preinstall_targets().to_vec();
        targets.sort_by_key(|target| (usize::from(!target.recommended), target.display_name));
        for target in targets {
            let is_installed = target
                .skill_ids
                .iter()
                .all(|skill_id| installed_skill_ids.contains(*skill_id));
            entries.push(SettingsEntry {
                label: target.display_name.to_owned(),
                category_tag: "[Skill Pack]".to_owned(),
                status_tag: is_installed.then_some("installed".to_owned()),
                description: if is_installed {
                    format!("remove from the managed runtime · {}", target.summary)
                } else {
                    format!("install into the managed runtime · {}", target.summary)
                },
                action: if is_installed {
                    CommandAction::ApplySettings(SettingsCommandAction::RemoveSkillPack(
                        target.install_id.to_owned(),
                    ))
                } else {
                    CommandAction::ApplySettings(SettingsCommandAction::InstallSkillPack(
                        target.install_id.to_owned(),
                    ))
                },
                selectable: true,
            });
        }
    }

    if entries.is_empty() {
        entries.push(SettingsEntry {
            label: "settings".to_owned(),
            category_tag: String::new(),
            status_tag: None,
            description: "no adjustable settings available in this view".to_owned(),
            action: CommandAction::Close,
            selectable: false,
        });
    }

    let max_desc_width = width.saturating_sub(24).max(24);
    for entry in &mut entries {
        entry.description = truncate_right_for_width(entry.description.as_str(), max_desc_width);
    }

    entries
}

fn build_settings_overview_entries(
    runtime: &CliTurnRuntime,
    width: usize,
    installed_skill_ids: &BTreeSet<String>,
) -> Vec<SettingsEntry> {
    let provider_label = runtime.config.provider.kind.display_name();
    let web_provider =
        normalize_web_search_provider(runtime.config.tools.web_search.default_provider.as_str())
            .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
    let web_label = web_search_provider_descriptor(web_provider)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(web_provider);
    let mcp_count = runtime.effective_bootstrap_mcp_servers.len();
    let installed_pack_count = bundled_preinstall_targets()
        .iter()
        .filter(|target| {
            target
                .skill_ids
                .iter()
                .all(|skill_id| installed_skill_ids.contains(*skill_id))
        })
        .count();
    let skills_state = if runtime.config.external_skills.enabled {
        if installed_pack_count == 0 {
            "managed skills enabled"
        } else {
            "managed skills active"
        }
    } else {
        "managed skills disabled"
    };

    let mut entries = vec![
        SettingsEntry {
            label: "Provider & web".to_owned(),
            category_tag: "[Setup]".to_owned(),
            status_tag: None,
            description: format!("{provider_label} · {web_label}"),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Provider),
            selectable: true,
        },
        SettingsEntry {
            label: "Workspace setup".to_owned(),
            category_tag: "[Setup]".to_owned(),
            status_tag: None,
            description: if installed_pack_count == 0 {
                format!("{mcp_count} bootstrap MCP · {skills_state}")
            } else {
                format!("{mcp_count} bootstrap MCP · {installed_pack_count} packs installed")
            },
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Workspace),
            selectable: true,
        },
        SettingsEntry {
            label: "Permissions".to_owned(),
            category_tag: "[Review]".to_owned(),
            status_tag: None,
            description: "inspect the current tool-permission posture".to_owned(),
            action: CommandAction::RunCommand("/permissions"),
            selectable: true,
        },
    ];

    let max_desc_width = width.saturating_sub(24).max(24);
    for entry in &mut entries {
        entry.description = truncate_right_for_width(entry.description.as_str(), max_desc_width);
    }
    entries
}

fn render_current_web_search_summary(runtime: &CliTurnRuntime) -> String {
    let provider_id =
        normalize_web_search_provider(runtime.config.tools.web_search.default_provider.as_str())
            .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
    let provider_label = web_search_provider_descriptor(provider_id)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(provider_id);
    let credential_state = runtime
        .config
        .tools
        .web_search
        .configured_api_key_for_provider(provider_id)
        .map(str::to_owned)
        .or_else(|| {
            let env_names = web_search_provider_api_key_env_names(provider_id);
            if env_names.is_empty() {
                None
            } else {
                Some(format!("expects {}", env_names.join(" or ")))
            }
        })
        .unwrap_or_else(|| "not required".to_owned());
    format!("{provider_label} · {credential_state}")
}

fn render_provider_settings_entry(
    runtime: &CliTurnRuntime,
    kind: ProviderKind,
    is_current: bool,
) -> (Option<String>, String) {
    if is_current {
        let auth_state = runtime
            .config
            .provider
            .resolved_auth_env_name()
            .map(|env_name| format!("auth {env_name}"))
            .or_else(|| {
                if runtime.config.provider.api_key().is_some()
                    || runtime.config.provider.oauth_access_token().is_some()
                {
                    Some("runtime credentials already loaded".to_owned())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "credentials still need wiring".to_owned());
        return (
            Some("current".to_owned()),
            format!(
                "current active provider · model {} · {auth_state}",
                runtime.config.provider.model
            ),
        );
    }

    if let Some((env_name, binding_kind)) = detected_startup_auth_binding(kind) {
        let binding_label = match binding_kind {
            StartupProviderAuthBindingKind::ApiKey => "api key",
            StartupProviderAuthBindingKind::OauthAccessToken => "oauth",
        };
        return (
            Some("ready".to_owned()),
            format!(
                "switch provider to {} · {binding_label} {env_name} available",
                kind.display_name()
            ),
        );
    }

    (
        Some("unconfigured".to_owned()),
        format!(
            "switch provider to {} · credentials still need wiring",
            kind.display_name()
        ),
    )
}

fn web_search_provider_env_api_key_name(provider_id: &str) -> Option<String> {
    web_search_provider_api_key_env_names(provider_id)
        .iter()
        .find(|env_name| std::env::var_os(env_name).is_some())
        .map(|env_name| (*env_name).to_owned())
}

fn render_web_provider_settings_entry(
    runtime: &CliTurnRuntime,
    provider_id: &str,
    provider_label: &str,
    is_current: bool,
) -> (Option<String>, String) {
    if is_current {
        let credential_state = runtime
            .config
            .tools
            .web_search
            .configured_api_key_for_provider(provider_id)
            .map(|value| format!("configured in tools.web_search as {value}"))
            .or_else(|| {
                web_search_provider_env_api_key_name(provider_id)
                    .map(|env_name| format!("env {env_name} available"))
            })
            .unwrap_or_else(|| "credentials still need wiring".to_owned());
        return (
            Some("current".to_owned()),
            format!("current default web-search provider · {credential_state}"),
        );
    }

    if let Some(env_name) = web_search_provider_env_api_key_name(provider_id) {
        return (
            Some("ready".to_owned()),
            format!("switch default web-search to {provider_label} · env {env_name} available"),
        );
    }

    (
        Some("unconfigured".to_owned()),
        format!("switch default web-search to {provider_label} · credentials still need wiring"),
    )
}

fn persist_runtime_settings(
    runtime: &mut CliTurnRuntime,
    app: &mut App,
    mutate: impl FnOnce(&mut LoongConfig) -> Result<String, String>,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let summary = mutate(&mut config)?;
    crate::config::write(
        Some(runtime.resolved_path.to_string_lossy().as_ref()),
        &config,
        true,
    )?;
    #[cfg(not(test))]
    crate::runtime_env::initialize_runtime_environment(
        &config,
        Some(runtime.resolved_path.as_path()),
    );
    runtime.config = config;
    runtime.config_present = true;
    app.model = runtime.config.provider.model.clone();
    Ok(summary)
}
fn current_working_directory(runtime: &CliTurnRuntime) -> PathBuf {
    runtime
        .effective_working_directory
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn render_new_conversation_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "new".to_owned(),
        caption: Some("fresh conversation".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("ready".to_owned()),
            lines: vec![
                "The visible transcript has been cleared and the composer is ready for the next turn."
                    .to_owned(),
            ],
        }],
        footer_lines: vec!["Type immediately; no extra focus step is needed.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn copy_command_text(app: &App, args: &str) -> Result<String, String> {
    if !args.trim().is_empty() {
        return Ok(args.trim().to_owned());
    }
    app.message_list
        .latest_copy_text()
        .ok_or_else(|| "nothing copyable yet".to_owned())
}

fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };

    let mut last_error = "no clipboard command attempted".to_owned();
    for (program, args) in candidates {
        let spawn_result = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut child) = spawn_result else {
            last_error = format!("{program} unavailable");
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut()
            && let Err(error) = stdin.write_all(text.as_bytes())
        {
            last_error = format!("{program} write failed: {error}");
            let _ = child.kill();
            continue;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("{program} wait failed: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        last_error = if stderr.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            format!("{program}: {stderr}")
        };
    }
    Err(last_error)
}

fn render_copy_command_lines_with_width(
    result: Result<String, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(text) => {
            let char_count = text.chars().count();
            (
                TuiCalloutTone::Info,
                "copied".to_owned(),
                vec![format!(
                    "Copied {char_count} character(s) to the system clipboard."
                )],
            )
        }
        Err(error) => (
            TuiCalloutTone::Warning,
            "copy unavailable".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: "copy".to_owned(),
        caption: Some("clipboard".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "/copy copies the latest reply, or /copy <text> copies explicit text.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|error| format!("git failed to start: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.status.success() {
        Ok(stdout)
    } else if stderr.is_empty() {
        Err(format!("git exited with {}", output.status))
    } else {
        Err(stderr)
    }
}

fn render_git_diff_command_lines_with_width(cwd: &Path, width: usize) -> Vec<String> {
    let status = run_git_capture(cwd, &["status", "--short"]);
    let stat = run_git_capture(cwd, &["diff", "--stat"]);
    let shortstat = run_git_capture(cwd, &["diff", "--shortstat"]);

    let mut sections = Vec::new();
    match (status, stat, shortstat) {
        (Ok(status), Ok(stat), Ok(shortstat)) => {
            let status_lines = if status.trim().is_empty() {
                vec!["working tree clean".to_owned()]
            } else {
                status.lines().map(ToOwned::to_owned).collect()
            };
            sections.push(TuiSectionSpec::Preformatted {
                title: Some("status".to_owned()),
                language: None,
                lines: status_lines,
            });
            if !stat.trim().is_empty() {
                sections.push(TuiSectionSpec::Preformatted {
                    title: Some("diff stat".to_owned()),
                    language: None,
                    lines: stat.lines().map(ToOwned::to_owned).collect(),
                });
            }
            if !shortstat.trim().is_empty() {
                sections.push(TuiSectionSpec::Narrative {
                    title: Some("summary".to_owned()),
                    lines: vec![shortstat],
                });
            }
        }
        (status, stat, shortstat) => {
            let errors = [status.err(), stat.err(), shortstat.err()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            sections.push(TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some("git diff unavailable".to_owned()),
                lines: if errors.is_empty() {
                    vec!["git did not return diff information".to_owned()]
                } else {
                    errors
                },
            });
        }
    }

    let message_spec = TuiMessageSpec {
        role: "diff".to_owned(),
        caption: Some("working tree".to_owned()),
        sections,
        footer_lines: vec![format!("cwd: {}", cwd.display())],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(64)
        .collect::<String>()
}

fn write_transcript_export(
    cwd: &Path,
    session_id: &str,
    label: &str,
    markdown: &str,
) -> Result<PathBuf, String> {
    if markdown.trim().is_empty() {
        return Err("transcript is empty".to_owned());
    }
    let export_dir = cwd.join(".loong").join("exports");
    fs::create_dir_all(export_dir.as_path())
        .map_err(|error| format!("failed to create export directory: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock error: {error}"))?
        .as_secs();
    let session = safe_file_component(session_id);
    let label = safe_file_component(label);
    let file_name = format!("{label}-{session}-{timestamp}.md");
    let path = export_dir.join(file_name);
    fs::write(path.as_path(), markdown)
        .map_err(|error| format!("failed to write export: {error}"))?;
    Ok(path)
}

fn render_export_command_lines_with_width(
    command: &str,
    result: Result<PathBuf, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(path) => (
            TuiCalloutTone::Info,
            "written".to_owned(),
            vec![format!("{} wrote {}", command, path.display())],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "not written".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: command.trim_start_matches('/').to_owned(),
        caption: Some("transcript artifact".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "Artifacts stay local until you explicitly move or publish them.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn resolve_import_path(cwd: &Path, input: &str) -> PathBuf {
    let trimmed = input.trim().trim_matches('"').trim_matches('\'');
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn import_context_into_composer(app: &mut App, cwd: &Path, args: &str) -> Result<PathBuf, String> {
    let path = resolve_import_path(cwd, args);
    let content = fs::read_to_string(path.as_path())
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let clipped = if content.chars().count() > 20_000 {
        let prefix = content.chars().take(20_000).collect::<String>();
        format!("{prefix}\n\n[import truncated to first 20000 characters]")
    } else {
        content
    };
    app.composer.set_input(format!(
        "Use this imported context from {}:\n\n{}",
        path.display(),
        clipped
    ));
    Ok(path)
}

fn render_import_command_lines_with_width(
    result: Result<PathBuf, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(path) => (
            TuiCalloutTone::Info,
            "staged".to_owned(),
            vec![format!(
                "Imported {} into the composer draft.",
                path.display()
            )],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "import failed".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: "import".to_owned(),
        caption: Some("composer context".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "Review the staged draft before sending if the file is large.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn latest_text_or_args(app: &App, args: &str) -> Result<String, String> {
    if !args.trim().is_empty() {
        return Ok(args.trim().to_owned());
    }
    app.message_list
        .latest_copy_text()
        .ok_or_else(|| "no previous content to use".to_owned())
}

fn stage_simplify_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let source = latest_text_or_args(app, args)?;
    app.composer.set_input(format!(
        "Please simplify and clarify the following content without losing important details:\n\n{source}"
    ));
    Ok(())
}

fn stage_plan_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let subject = if args.trim().is_empty() {
        "the current task".to_owned()
    } else {
        args.trim().to_owned()
    };
    app.composer.set_input(format!(
        "Create a concise implementation plan for {subject}. Include risks, verification, and the smallest safe sequence."
    ));
    Ok(())
}

fn stage_feedback_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let body = if args.trim().is_empty() {
        "Feedback: ".to_owned()
    } else {
        format!("Feedback: {}", args.trim())
    };
    app.composer.set_input(body);
    Ok(())
}

fn render_prompt_staging_lines_with_width(
    role: &str,
    result: Result<(), String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(()) => (
            TuiCalloutTone::Info,
            "draft staged".to_owned(),
            vec!["The composer has been populated; edit or press Enter to send.".to_owned()],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "not staged".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: role.to_owned(),
        caption: Some("composer draft".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec!["Typing continues in the composer immediately.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_title_command_lines_with_width(command: &str, args: &str, width: usize) -> Vec<String> {
    let lines = if args.trim().is_empty() {
        vec![format!("Usage: {command} <title>")]
    } else {
        vec![format!(
            "Title noted for this local chat surface: {}",
            args.trim()
        )]
    };
    let message_spec = TuiMessageSpec {
        role: command.trim_start_matches('/').to_owned(),
        caption: Some("local title".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("title".to_owned()),
            lines,
        }],
        footer_lines: vec!["The title is reflected in the footer for this TUI session.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

async fn submit_user_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: String,
) -> CliResult<()> {
    start_turn(terminal, app, runtime, input, true).await
}

async fn start_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: String,
    echo_user_message: bool,
) -> CliResult<()> {
    maybe_capture_and_persist_first_turn_bootstrap_reply(app, runtime, input.as_str())?;
    let width = current_render_width(terminal)?;
    app.live_render_width.store(width.max(1), Ordering::Relaxed);
    if echo_user_message {
        app.message_list.add_user_message(input.clone());
    }
    app.composer_follow_up_intent = false;
    app.spinner_seed = spinner_seed();
    app.last_pending_signature = None;
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.focus = Focus::Composer;
    clear_live_transcript(&app.live_transcript);

    terminal
        .draw(|f| app.render(f))
        .map_err(|e| format!("draw error: {}", e))?;

    let sink = {
        let live_transcript = Arc::clone(&app.live_transcript);
        Arc::new(
            move |payload: super::super::CliChatLiveSurfaceRenderPayload| {
                if let Ok(mut state) = live_transcript.lock() {
                    state.draft_preview = payload.draft_preview;
                    state.tool_activity_lines = payload.tool_activity_lines;
                }
            },
        )
    };
    let (observer, rerender) = super::super::build_cli_chat_live_compact_observer_controller(
        Arc::clone(&app.live_render_width),
        sink,
    );
    app.live_rerender = Some(rerender);
    let mut turn_runtime = runtime.clone();
    if let Some(addendum) = app.pending_first_turn_bootstrap_addendum.take() {
        apply_first_turn_bootstrap_addendum(&mut turn_runtime, addendum);
        app.awaiting_first_turn_bootstrap_reply = true;
    }
    app.pending_task = Some(spawn_pending_turn(turn_runtime, input, observer));
    Ok(())
}

fn queue_pending_steer(app: &mut App, input: String) {
    if input.trim().is_empty() {
        return;
    }
    app.pending_steers.push_back(input);
    app.focus = Focus::Composer;
}

fn queue_pending_message(app: &mut App) {
    let input = app.composer.take_input();
    if input.trim().is_empty() {
        return;
    }
    app.composer_follow_up_intent = false;
    app.pending_queue.push_back(input);
    app.focus = Focus::Composer;
}

fn dequeue_pending_steer(app: &mut App) -> bool {
    if let Some(input) = app.pending_queue.pop_back() {
        app.composer.set_input(input);
        app.focus = Focus::Composer;
        return true;
    }
    let Some(input) = app.pending_steers.pop_back() else {
        return false;
    };
    app.composer.set_input(input);
    app.focus = Focus::Composer;
    true
}

fn is_transcript_navigation_key(key: crossterm::event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
    )
}

fn should_focus_composer_for_transcript_key(key: crossterm::event::KeyEvent) -> bool {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return false;
    }

    matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Enter
            | KeyCode::Left
            | KeyCode::Right
    )
}

fn route_transcript_key_to_composer(
    app: &mut App,
    key: crossterm::event::KeyEvent,
) -> Option<String> {
    app.focus = Focus::Composer;
    let submitted = app.composer.handle_key(key);
    app.sync_inline_skill_popup();
    submitted
}

fn should_route_composer_key_to_transcript(app: &App, key: crossterm::event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
    ) || (app.composer.is_empty() && is_transcript_navigation_key(key))
}

fn submitted_message_is_follow_up(app: &App, msg: &str) -> bool {
    app.pending_turn
        && app.composer_follow_up_intent
        && !msg.starts_with('/')
        && !msg.starts_with(':')
}

fn display_columns(text: &str) -> usize {
    crate::presentation::display_width(text)
}

fn truncate_right_for_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_columns(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if used + ch_width > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
}

fn truncate_middle_for_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_columns(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }

    let target_prefix_width = width.saturating_sub(1).div_ceil(2);
    let target_suffix_width = width.saturating_sub(1).saturating_sub(target_prefix_width);

    let mut prefix = String::new();
    let mut prefix_used = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if prefix_used + ch_width > target_prefix_width {
            break;
        }
        prefix.push(ch);
        prefix_used += ch_width;
    }

    let mut suffix_chars = Vec::new();
    let mut suffix_used = 0usize;
    for ch in text.chars().rev() {
        let ch_width = crate::presentation::char_display_width(ch);
        if suffix_used + ch_width > target_suffix_width {
            break;
        }
        suffix_chars.push(ch);
        suffix_used += ch_width;
    }
    suffix_chars.reverse();
    let suffix = suffix_chars.into_iter().collect::<String>();

    format!("{prefix}…{suffix}")
}

fn rect_contains_point(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn current_skill_token_query(composer: &Composer) -> Option<String> {
    let range = current_skill_token_range(composer)?;
    composer.text()[range]
        .strip_prefix('$')
        .map(|query| query.to_owned())
}

fn current_skill_token_range(composer: &Composer) -> Option<std::ops::Range<usize>> {
    let text = composer.text();
    let cursor = composer.cursor().min(text.len());
    if text.is_empty() {
        return None;
    }

    let before_cursor = &text[..cursor];
    let token_start = before_cursor
        .char_indices()
        .rfind(|(_, ch)| ch.is_whitespace())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    let after_cursor = &text[cursor..];
    let token_end = after_cursor
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, _)| cursor + idx)
        .unwrap_or(text.len());
    if token_start >= token_end {
        return None;
    }

    let token = &text[token_start..token_end];
    token.starts_with('$').then_some(token_start..token_end)
}

fn inline_skill_replacement_text(
    text: &str,
    range: &std::ops::Range<usize>,
    replacement: &str,
) -> String {
    let should_trim_trailing_space = replacement.ends_with(' ')
        && text
            .get(range.end..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(char::is_whitespace);

    if should_trim_trailing_space {
        replacement.trim_end_matches(' ').to_owned()
    } else {
        replacement.to_owned()
    }
}

fn build_status_footer_line(cwd: &str, model: &str, width: u16) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::from(String::new());
    }

    if width <= 24 {
        return single_footer_span(model, width, Style::default().fg(SURFACE_GRAY));
    }

    let mut model_text = model.to_owned();
    let mut cwd_text = cwd.to_owned();
    let mut model_width = display_columns(&model_text);
    let mut cwd_width = display_columns(&cwd_text);

    if model_width >= width {
        model_text = truncate_right_for_width(&model_text, width.saturating_sub(1).max(1));
        model_width = display_columns(&model_text);
    }

    let available_for_cwd = width.saturating_sub(model_width + 1);
    if cwd_width > available_for_cwd {
        cwd_text = truncate_middle_for_width(&cwd_text, available_for_cwd);
        cwd_width = display_columns(&cwd_text);
    }

    let mut spacer_width = width.saturating_sub(cwd_width + model_width);
    if !cwd_text.is_empty() && !model_text.is_empty() && spacer_width == 0 {
        if cwd_width > model_width {
            cwd_text = truncate_middle_for_width(&cwd_text, cwd_width.saturating_sub(1));
            cwd_width = display_columns(&cwd_text);
        } else {
            model_text = truncate_right_for_width(&model_text, model_width.saturating_sub(1));
            model_width = display_columns(&model_text);
        }
        spacer_width = width.saturating_sub(cwd_width + model_width);
    }

    Line::from(vec![
        Span::styled(cwd_text, Style::default().fg(SURFACE_GRAY)),
        Span::raw(" ".repeat(spacer_width)),
        Span::styled(model_text, Style::default().fg(SURFACE_GRAY)),
    ])
}

fn single_footer_span(text: &str, width: usize, style: Style) -> Line<'static> {
    let mut rendered = truncate_right_for_width(text, width);
    let rendered_width = display_columns(&rendered);
    if rendered_width < width {
        rendered.push_str(&" ".repeat(width - rendered_width));
    }
    Line::from(vec![Span::styled(rendered, style)])
}

fn footer_content_area(area: Rect) -> Rect {
    if area.width <= FOOTER_HORIZONTAL_INDENT {
        return area;
    }

    Rect {
        x: area.x.saturating_add(FOOTER_HORIZONTAL_INDENT),
        y: area.y,
        width: area.width.saturating_sub(FOOTER_HORIZONTAL_INDENT),
        height: area.height,
    }
}

fn build_queue_footer_line(i18n: &I18nService, queued: usize, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 18 {
        let text = if queued > 0 {
            format!("queued ×{queued}")
        } else {
            i18n.text(SurfaceCopy::FooterQueueShort).to_owned()
        };
        return single_footer_span(
            text.as_str(),
            max_width,
            Style::default().fg(SURFACE_ACCENT),
        );
    }

    let hint = i18n.text(SurfaceCopy::FooterQueueHint).to_owned();
    let short_hint = i18n.text(SurfaceCopy::FooterQueueShort).to_owned();
    let suffix = if queued > 0 {
        format!(" · queued ×{queued}")
    } else {
        String::new()
    };
    let total_width = display_columns(&hint) + display_columns(&suffix);
    if total_width <= max_width {
        let mut spans = vec![Span::styled(hint, Style::default().fg(SURFACE_ACCENT))];
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, Style::default().fg(SURFACE_GRAY)));
        }
        return Line::from(spans);
    }

    let short_total_width = display_columns(&short_hint) + display_columns(&suffix);
    if short_total_width <= max_width {
        let mut spans = vec![Span::styled(
            short_hint,
            Style::default().fg(SURFACE_ACCENT),
        )];
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, Style::default().fg(SURFACE_GRAY)));
        }
        return Line::from(spans);
    }

    if display_columns(&short_hint) >= max_width {
        return Line::from(vec![Span::styled(
            truncate_right_for_width(&short_hint, max_width),
            Style::default().fg(SURFACE_ACCENT),
        )]);
    }

    let remaining = max_width.saturating_sub(display_columns(&short_hint));
    Line::from(vec![
        Span::styled(short_hint, Style::default().fg(SURFACE_ACCENT)),
        Span::styled(
            truncate_right_for_width(&suffix, remaining),
            Style::default().fg(SURFACE_GRAY),
        ),
    ])
}

fn build_restore_footer_line(i18n: &I18nService, queued: usize, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 18 {
        return single_footer_span(
            format!("restore ×{queued}").as_str(),
            max_width,
            Style::default().fg(SURFACE_GRAY),
        );
    }

    let full_text = format!(
        "{} {} · queued ×{}",
        queue_restore_shortcut_label(),
        i18n.text(SurfaceCopy::FooterRestoreQueued),
        queued
    );
    let short_text = format!(
        "{} {} · ×{}",
        queue_restore_shortcut_label(),
        i18n.text(SurfaceCopy::FooterRestoreShort),
        queued
    );
    let selected = if display_columns(&full_text) <= width as usize {
        full_text
    } else {
        short_text
    };
    Line::from(vec![Span::styled(
        truncate_right_for_width(&selected, max_width),
        Style::default().fg(SURFACE_GRAY),
    )])
}

fn build_follow_footer_line(i18n: &I18nService, model: &str, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 24 {
        return single_footer_span(
            i18n.text(SurfaceCopy::FooterFollowShort),
            max_width,
            Style::default().fg(SURFACE_ACCENT),
        );
    }

    let full_hint = i18n.text(SurfaceCopy::FooterFollowHint).to_owned();
    let short_hint = i18n.text(SurfaceCopy::FooterFollowShort).to_owned();
    let hint = if display_columns(&full_hint) <= max_width {
        full_hint
    } else {
        short_hint
    };

    if display_columns(&hint) >= max_width {
        return Line::from(vec![Span::styled(
            truncate_right_for_width(&hint, max_width),
            Style::default().fg(SURFACE_ACCENT),
        )]);
    }

    let available_for_model = max_width.saturating_sub(display_columns(&hint) + 1);
    let model_text = truncate_right_for_width(model, available_for_model);
    let spacer_width =
        max_width.saturating_sub(display_columns(&hint) + display_columns(&model_text));

    Line::from(vec![
        Span::styled(hint, Style::default().fg(SURFACE_ACCENT)),
        Span::raw(" ".repeat(spacer_width)),
        Span::styled(model_text, Style::default().fg(SURFACE_GRAY)),
    ])
}

fn queue_restore_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Option + Up"
    } else {
        "Alt + Up"
    }
}

async fn build_command_lines(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
    input: &str,
    width: usize,
) -> CliResult<Vec<String>> {
    let trimmed = input.trim();

    match trimmed {
        super::super::CLI_CHAT_HELP_COMMAND => Ok(render_chat_surface_help_lines_with_width(width)),
        super::super::CLI_CHAT_STATUS_COMMAND => {
            let summary = super::super::ops::build_cli_chat_startup_summary(runtime, options)?;
            Ok(super::super::ops::render_cli_chat_status_lines_with_width(
                &summary, width,
            ))
        }
        super::super::CLI_CHAT_HISTORY_COMMAND => {
            #[cfg(feature = "memory-sqlite")]
            {
                let history_lines = super::super::ops::load_history_lines(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::ops::render_cli_chat_history_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &history_lines,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "history",
                        "history unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        super::super::CLI_CHAT_COMPACT_COMMAND => {
            #[cfg(feature = "memory-sqlite")]
            {
                let result = super::super::ops::load_manual_compaction_result(
                    &runtime.config,
                    &runtime.session_id,
                    &runtime.turn_coordinator,
                    runtime.conversation_binding(),
                )
                .await?;
                Ok(
                    super::super::ops::render_manual_compaction_lines_with_width(
                        &runtime.session_id,
                        &result,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "compact",
                        "manual compaction unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/model" => Ok(render_model_command_lines_with_width(runtime, width)),
        "/permissions" => Ok(render_permissions_command_lines_with_width(width)),
        "/experimental" => Ok(render_experimental_command_lines_with_width(width)),
        "/themes" => Ok(render_themes_command_lines_with_width(width)),
        "/cwd" => Ok(render_cwd_command_lines_with_width(runtime, width)),
        "/language" => Ok(render_language_command_lines_with_width(width)),
        "/mcp" => Ok(render_mcp_command_lines_with_width(runtime, width)),
        "/skills" => Ok(render_skills_command_lines_with_width(runtime, width)),
        "/usage" => Ok(render_slash_command_usage_lines_with_width(width)),
        "/fast_lane_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let summary = crate::conversation::load_fast_lane_tool_batch_event_summary(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::render_fast_lane_summary_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &summary,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "fast_lane_summary",
                        "fast lane summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/safe_lane_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let summary = crate::conversation::load_safe_lane_event_summary(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::render_safe_lane_summary_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &runtime.config.conversation,
                    &summary,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "safe_lane_summary",
                        "safe lane summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/turn_checkpoint_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let diagnostics = runtime
                    .turn_coordinator
                    .load_production_turn_checkpoint_diagnostics_with_limit(
                        &runtime.config,
                        &runtime.session_id,
                        runtime.config.memory.sliding_window,
                        runtime.conversation_binding(),
                    )
                    .await?;
                Ok(
                    super::super::render_turn_checkpoint_summary_lines_with_width(
                        &runtime.session_id,
                        runtime.config.memory.sliding_window,
                        &diagnostics,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "turn_checkpoint_summary",
                        "turn checkpoint summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/turn_checkpoint_repair" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let outcome = runtime
                    .turn_coordinator
                    .repair_production_turn_checkpoint_tail(
                        &runtime.config,
                        &runtime.session_id,
                        runtime.conversation_binding(),
                    )
                    .await?;
                Ok(
                    super::super::render_turn_checkpoint_repair_lines_with_width(
                        &runtime.session_id,
                        &outcome,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "turn_checkpoint_repair",
                        "turn checkpoint repair unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/sessions" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_sessions_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "sessions",
                        "session queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/subagents" | "/workers" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_workers_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "workers",
                        "worker queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/review" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_review_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "review",
                        "review queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/missions" | "/mission" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_mission_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "mission",
                        "mission control unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        _ => {
            if let Some(spec) = slash_command_specs()
                .iter()
                .find(|spec| spec.command == trimmed)
            {
                Ok(render_slash_command_detail_lines_with_width(spec, width))
            } else {
                Ok(render_slash_command_usage_lines_with_width(width))
            }
        }
    }
}

fn render_slash_command_usage_lines_with_width(width: usize) -> Vec<String> {
    let command_items = slash_command_specs()
        .iter()
        .map(|spec| TuiKeyValueSpec::Plain {
            key: spec.command.to_owned(),
            value: slash_command_help_value(spec),
        })
        .collect::<Vec<_>>();

    let message_spec = TuiMessageSpec {
        role: "usage".to_owned(),
        caption: Some("slash commands".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("commands".to_owned()),
                items: command_items,
            },
            TuiSectionSpec::Narrative {
                title: Some("navigation".to_owned()),
                lines: vec![
                    "Open this deck with / or : from an empty composer.".to_owned(),
                    "Every command stays visible in the same product order so muscle memory keeps working across releases."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec![
            "Enter runs the command or opens its detail card without permission ceremony.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_slash_command_detail_lines_with_width(
    spec: &super::command_palette::SlashCommandSpec,
    width: usize,
) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "command".to_owned(),
        caption: Some(spec.command.trim_start_matches('/').to_owned()),
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("enabled".to_owned()),
                lines: vec![format!(
                    "{} is available in the command deck and keeps a stable slot in the local TUI.",
                    spec.command
                )],
            },
            TuiSectionSpec::Narrative {
                title: Some("intent".to_owned()),
                lines: vec![spec.description.to_owned()],
            },
        ],
        footer_lines: vec!["Use /usage to see the complete command deck.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn slash_command_help_value(spec: &super::command_palette::SlashCommandSpec) -> String {
    spec.description.to_owned()
}

fn render_model_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let provider = &runtime.config.provider;
    let active_profile = runtime
        .config
        .active_provider_id()
        .unwrap_or("legacy provider");
    let reasoning_effort = provider
        .reasoning_effort
        .map(|effort| format!("{effort:?}").to_ascii_lowercase())
        .unwrap_or_else(|| "default".to_owned());

    let message_spec = TuiMessageSpec {
        role: "model".to_owned(),
        caption: Some("active model".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("provider".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "profile".to_owned(),
                    value: active_profile.to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "provider".to_owned(),
                    value: provider.kind.display_name().to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "model".to_owned(),
                    value: provider.model.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "wire api".to_owned(),
                    value: format!("{:?}", provider.wire_api).to_ascii_lowercase(),
                },
                TuiKeyValueSpec::Plain {
                    key: "reasoning".to_owned(),
                    value: reasoning_effort,
                },
            ],
        }],
        footer_lines: vec![
            "Use /model <selector> to switch when you want a different model.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_permissions_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "permissions".to_owned(),
        caption: Some("YOLO".to_owned()),
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("YOLO by default".to_owned()),
                lines: vec![
                    "Hey yo, you only live once, take care.".to_owned(),
                ],
            },
            TuiSectionSpec::KeyValues {
                title: Some("default posture".to_owned()),
                items: vec![
                    TuiKeyValueSpec::Plain {
                        key: "mode".to_owned(),
                        value: "YOLO".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "commands".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "tools".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "slash deck".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "permission prompts".to_owned(),
                        value: "not part of the happy path".to_owned(),
                    },
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("behavior".to_owned()),
                lines: vec![
                    "This screen stays intentionally simple; it does not show allow/deny tables or ask the user to negotiate routine actions."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec!["The default local TUI stays open; stricter deployments can still configure policy explicitly."
            .to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_experimental_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "experimental".to_owned(),
        caption: Some("experimental features".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("enabled surface work".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "streaming renderer".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "startup animation".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "markdown/diff/table preview".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "resize smoothing".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "slash command deck".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "tool activity compaction".to_owned(),
                    value: "enabled".to_owned(),
                },
            ],
        }],
        footer_lines: vec!["No toggle ceremony in the default TUI path.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_themes_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "themes".to_owned(),
        caption: Some("theme".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("current surface".to_owned()),
                items: vec![
                    TuiKeyValueSpec::Plain {
                        key: "palette".to_owned(),
                        value: "terminal-adaptive dark surface".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "accent".to_owned(),
                        value: "startup blue with semantic red/green/yellow states".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "resize".to_owned(),
                        value: "layout recalculates from viewport on every draw".to_owned(),
                    },
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("behavior".to_owned()),
                lines: vec![
                    "The default theme path is already active: dark, terminal-adaptive, and readable without extra setup."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec!["The terminal-adaptive theme is active for this session.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_cwd_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let cwd = runtime
        .effective_working_directory
        .as_deref()
        .unwrap_or(runtime.resolved_path.as_path())
        .display()
        .to_string();
    let message_spec = TuiMessageSpec {
        role: "cwd".to_owned(),
        caption: Some("working directory".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("current scope".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "cwd".to_owned(),
                    value: cwd,
                },
                TuiKeyValueSpec::Plain {
                    key: "session".to_owned(),
                    value: runtime.session_id.clone(),
                },
            ],
        }],
        footer_lines: vec!["Use /cwd <path> to move the chat working directory.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_language_command_lines_with_width(width: usize) -> Vec<String> {
    let language = resolve_default_language();
    let message_spec = TuiMessageSpec {
        role: "language".to_owned(),
        caption: Some("language".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("current language".to_owned()),
            items: vec![TuiKeyValueSpec::Plain {
                key: "detected".to_owned(),
                value: language_label(language).to_owned(),
            }],
        }],
        footer_lines: vec!["Use /language <locale> to switch the UI language.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn language_label(language: super::i18n::Language) -> &'static str {
    match language {
        super::i18n::Language::En => "English",
        super::i18n::Language::ZhCn => "简体中文",
        super::i18n::Language::ZhTw => "繁體中文",
        super::i18n::Language::Ja => "日本語",
        super::i18n::Language::Ru => "Русский",
    }
}

fn render_mcp_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let mut items = runtime
        .effective_bootstrap_mcp_servers
        .iter()
        .map(|server| TuiKeyValueSpec::Plain {
            key: server.clone(),
            value: "enabled for this chat".to_owned(),
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "configured".to_owned(),
            value: "0".to_owned(),
        });
    }

    let message_spec = TuiMessageSpec {
        role: "mcp".to_owned(),
        caption: Some("MCP".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some(format!(
                "servers ({})",
                runtime.effective_bootstrap_mcp_servers.len()
            )),
            items,
        }],
        footer_lines: vec![
            "Startup keeps this compact; /mcp shows the details on demand.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_skills_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let skills = detect_available_skills(runtime.effective_working_directory.as_deref());
    let mut items = skills
        .iter()
        .take(14)
        .map(|skill| {
            let key = if let Some(alias) = skill.source_alias.as_deref() {
                format!("${} ({alias})", skill.name)
            } else {
                format!("${}", skill.name)
            };
            TuiKeyValueSpec::Plain {
                key,
                value: skill.description.clone(),
            }
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "available".to_owned(),
            value: "0".to_owned(),
        });
    }

    let hidden_count = skills.len().saturating_sub(items.len());
    let mut footer_lines =
        vec!["Type $skill-name directly in the composer to invoke a skill.".to_owned()];
    if hidden_count > 0 {
        footer_lines.push(format!(
            "Showing 14 of {}; keep typing to filter.",
            skills.len()
        ));
    }

    let message_spec = TuiMessageSpec {
        role: "skills".to_owned(),
        caption: Some("skills".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some(format!("available ({})", skills.len())),
            items,
        }],
        footer_lines,
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(feature = "memory-sqlite")]
fn render_sessions_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let sessions = store.visible_sessions(&runtime.session_id, 24)?;
    let mut items = Vec::new();
    for session in sessions.iter().take(12) {
        items.push(TuiKeyValueSpec::Plain {
            key: session.session_id.clone(),
            value: format!(
                "{} · {} · turns={}{}",
                session.label,
                session.state,
                session.turn_count,
                session
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No visible sessions rooted at the current scope.".to_owned(),
        });
    }
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("visible lineage".to_owned()),
        items,
    }];
    if let Some(primary) = sessions.first()
        && let Some(details) = store.session_details(&primary.session_id, false)?
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("selected session detail".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "label".to_owned(),
                    value: primary.label.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage root".to_owned(),
                    value: details
                        .lineage_root_session_id
                        .unwrap_or_else(|| "-".to_owned()),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage depth".to_owned(),
                    value: details.lineage_depth.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "trajectory turns".to_owned(),
                    value: details.trajectory_turn_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "events".to_owned(),
                    value: details.event_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "approvals".to_owned(),
                    value: details.approval_count.to_string(),
                },
            ],
        });
        if !details.recent_events.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("recent events".to_owned()),
                lines: details.recent_events,
            });
        }
    }
    let message_spec = TuiMessageSpec {
        role: "sessions".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Use /subagents for delegate lanes and /review for approvals.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_workers_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let workers = store.visible_worker_sessions(&runtime.session_id, 24)?;
    let mut items = Vec::new();
    for worker in workers.iter().take(12) {
        items.push(TuiKeyValueSpec::Plain {
            key: worker.session_id.clone(),
            value: format!(
                "{} · {} · turns={}{}",
                worker.label,
                worker.state,
                worker.turn_count,
                worker
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No visible delegate workers in the current scope.".to_owned(),
        });
    }
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("delegate lanes".to_owned()),
        items,
    }];
    if let Some(primary) = workers.first()
        && let Some(details) = store.session_details(&primary.session_id, true)?
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("selected worker detail".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "label".to_owned(),
                    value: primary.label.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "state".to_owned(),
                    value: primary.state.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "turns".to_owned(),
                    value: primary.turn_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage depth".to_owned(),
                    value: details.lineage_depth.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "delegate events".to_owned(),
                    value: details.delegate_events.len().to_string(),
                },
            ],
        });
        if !details.delegate_events.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("delegate lifecycle".to_owned()),
                lines: details.delegate_events,
            });
        }
    }
    let message_spec = TuiMessageSpec {
        role: "workers".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Use /sessions for the full lineage and /mission for lane rollups.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_review_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let approvals = store.approval_queue(&runtime.session_id, 16)?;
    let mut sections = Vec::new();
    let mut queue_items = Vec::new();
    for approval in approvals.iter().take(8) {
        queue_items.push(TuiKeyValueSpec::Plain {
            key: approval.approval_request_id.clone(),
            value: format!(
                "{} · {}{}{}",
                approval.tool_name,
                approval.status,
                approval
                    .reason
                    .as_deref()
                    .map(|reason| format!(" · {reason}"))
                    .unwrap_or_default(),
                approval
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if queue_items.is_empty() {
        queue_items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No approval requests are currently recorded for this session.".to_owned(),
        });
    }
    sections.push(TuiSectionSpec::KeyValues {
        title: Some("review queue".to_owned()),
        items: queue_items,
    });
    if let Some(latest) = approvals.first() {
        let mut detail_lines = vec![
            format!("tool={}", latest.tool_name),
            format!("status={}", latest.status),
            format!("turn_id={}", latest.turn_id),
            format!("requested_at={}", latest.requested_at),
        ];
        if let Some(reason) = latest.reason.as_deref() {
            detail_lines.push(format!("reason={reason}"));
        }
        if let Some(rule_id) = latest.rule_id.as_deref() {
            detail_lines.push(format!("rule_id={rule_id}"));
        }
        if let Some(error) = latest.last_error.as_deref() {
            detail_lines.push(format!("last_error={error}"));
        }
        sections.push(TuiSectionSpec::Narrative {
            title: Some("latest approval".to_owned()),
            lines: detail_lines,
        });
    }
    let message_spec = TuiMessageSpec {
        role: "review".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Governed actions will surface approval screens here when needed.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_mission_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let sessions = store.visible_sessions(&runtime.session_id, 32)?;
    let workers = store.visible_worker_sessions(&runtime.session_id, 32)?;
    let approvals = store.approval_queue(&runtime.session_id, 32)?;
    let state_mix = summarize_state_mix(sessions.iter().map(|session| session.state.as_str()));
    let worker_mix = summarize_state_mix(workers.iter().map(|worker| worker.state.as_str()));
    let summary_items = vec![
        TuiKeyValueSpec::Plain {
            key: "scope".to_owned(),
            value: runtime.session_id.clone(),
        },
        TuiKeyValueSpec::Plain {
            key: "provider".to_owned(),
            value: runtime
                .config
                .active_provider_id()
                .unwrap_or("-")
                .to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "visible sessions".to_owned(),
            value: sessions.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "delegate lanes".to_owned(),
            value: workers.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "review queue".to_owned(),
            value: approvals.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "session mix".to_owned(),
            value: state_mix.unwrap_or_else(|| "-".to_owned()),
        },
        TuiKeyValueSpec::Plain {
            key: "worker mix".to_owned(),
            value: worker_mix.unwrap_or_else(|| "-".to_owned()),
        },
    ];
    let recent_session_values = sessions
        .iter()
        .take(6)
        .map(|session| format!("{} ({})", session.label, session.state))
        .collect::<Vec<_>>();
    let recent_worker_values = workers
        .iter()
        .take(6)
        .map(|worker| format!("{} ({})", worker.label, worker.state))
        .collect::<Vec<_>>();
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("mission control".to_owned()),
        items: summary_items,
    }];
    if !recent_session_values.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("recent sessions".to_owned()),
            items: vec![TuiKeyValueSpec::Csv {
                key: "sessions".to_owned(),
                values: recent_session_values,
            }],
        });
    }
    if !recent_worker_values.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("recent workers".to_owned()),
            items: vec![TuiKeyValueSpec::Csv {
                key: "workers".to_owned(),
                values: recent_worker_values,
            }],
        });
    }
    let message_spec = TuiMessageSpec {
        role: "mission".to_owned(),
        caption: Some("control plane".to_owned()),
        sections,
        footer_lines: vec![
            "Use /sessions, /subagents, and /review to drill into each lane.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn summarize_state_mix<'a>(states: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts = std::collections::BTreeMap::new();
    for state in states {
        *counts.entry(state.to_owned()).or_insert(0usize) += 1;
    }
    if counts.is_empty() {
        return None;
    }
    Some(
        counts
            .into_iter()
            .map(|(state, count)| format!("{state}={count}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

async fn maybe_finalize_pending_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
) -> CliResult<bool> {
    let Some(handle) = app.pending_task.as_ref() else {
        return Ok(false);
    };
    if !handle.is_finished() {
        return Ok(false);
    }

    let handle = app
        .pending_task
        .take()
        .ok_or_else(|| "pending turn handle disappeared".to_owned())?;
    let assistant_text = handle
        .await
        .map_err(|error| format!("pending turn task failed to join: {error}"))??;
    let width = current_render_width(terminal)?;
    app.pending_turn = false;
    app.turn_start = None;
    app.live_rerender = None;
    app.composer_follow_up_intent = false;
    clear_live_transcript(&app.live_transcript);
    app.focus = Focus::Composer;
    if super::super::build_cli_chat_approval_screen_spec(&assistant_text).is_some() {
        app.message_list.add_rendered_lines(
            super::super::render_cli_chat_assistant_lines_with_width(&assistant_text, width),
        );
    } else {
        app.message_list.add_assistant_message(assistant_text);
    }
    if let Some(next_input) = app.pending_steers.pop_front() {
        start_turn(terminal, app, runtime, next_input, true).await?;
    } else if let Some(next_input) = app.pending_queue.pop_front() {
        start_turn(terminal, app, runtime, next_input, true).await?;
    }
    Ok(true)
}

fn current_render_width<B: Backend>(terminal: &Terminal<B>) -> CliResult<usize> {
    terminal
        .size()
        .map(|size| size.width as usize)
        .map_err(|e| format!("failed to query terminal size: {e}"))
}

fn spawn_pending_turn(
    runtime: CliTurnRuntime,
    input: String,
    observer: crate::conversation::ConversationTurnObserverHandle,
) -> JoinHandle<CliResult<String>> {
    tokio::spawn(async move {
        let result = crate::agent_runtime::AgentRuntime::new()
            .run_turn_with_runtime_and_observer(
                &runtime,
                &crate::agent_runtime::AgentTurnRequest {
                    message: input,
                    turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
                    channel_id: runtime.session_address.channel_id.clone(),
                    account_id: runtime.session_address.account_id.clone(),
                    conversation_id: runtime.session_address.conversation_id.clone(),
                    participant_id: runtime.session_address.participant_id.clone(),
                    thread_id: runtime.session_address.thread_id.clone(),
                    metadata: std::collections::BTreeMap::new(),
                    acp: runtime.explicit_acp_request,
                    acp_event_stream: false,
                    acp_bootstrap_mcp_servers: runtime.effective_bootstrap_mcp_servers.clone(),
                    acp_cwd: runtime
                        .effective_working_directory
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    live_surface_enabled: true,
                },
                None,
                Some(observer),
            )
            .await?;
        Ok(result.output_text)
    })
}

fn clear_live_transcript(live_transcript: &Arc<StdMutex<LiveTranscriptState>>) {
    if let Ok(mut state) = live_transcript.lock() {
        *state = LiveTranscriptState::default();
    }
}

fn pending_live_lines(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
    max_lines: usize,
) -> Vec<String> {
    let max_lines = max_lines.max(1);
    live_transcript
        .lock()
        .map(|state| {
            let state = &state.tool_activity_lines;
            let normalize = |mut lines: Vec<String>| {
                while lines.first().is_some_and(|line| line.trim().is_empty()) {
                    lines.remove(0);
                }
                while lines.last().is_some_and(|line| line.trim().is_empty()) {
                    lines.pop();
                }

                let mut normalized = Vec::new();
                let mut last_was_blank = false;
                for line in lines {
                    let is_blank = line.trim().is_empty();
                    if is_blank && last_was_blank {
                        continue;
                    }
                    last_was_blank = is_blank;
                    normalized.push(line);
                }
                normalized
            };

            if state.len() <= max_lines {
                return normalize(state.clone());
            }

            if let Some(blank_idx) = state.iter().position(|line| line.trim().is_empty()) {
                let (reasoning_lines, trailing_lines) = state.split_at(blank_idx);
                let visible_lines = trailing_lines.get(1..).unwrap_or(&[]);
                let reasoning = reasoning_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .take((max_lines / 2).max(1))
                    .cloned()
                    .collect::<Vec<_>>();
                let visible = visible_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .take(max_lines.saturating_sub(reasoning.len() + 1))
                    .cloned()
                    .collect::<Vec<_>>();
                if !reasoning.is_empty() && !visible.is_empty() {
                    let mut lines = reasoning;
                    lines.push(String::new());
                    lines.extend(visible);
                    return normalize(lines);
                }
            }

            normalize(state.iter().take(max_lines).cloned().collect())
        })
        .unwrap_or_default()
}

fn pending_live_tool_activity_lines(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
    max_lines: usize,
) -> Vec<String> {
    pending_live_lines(live_transcript, max_lines)
        .into_iter()
        .filter(|line| pending_line_is_tool_activity(line))
        .collect()
}

fn provisional_assistant_text(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
) -> Option<String> {
    live_transcript
        .lock()
        .ok()
        .and_then(|state| state.draft_preview.clone())
        .filter(|text| !text.trim().is_empty())
}

fn pending_line_is_tool_activity(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('•')
        || trimmed.starts_with("[running]")
        || trimmed.starts_with("[pending]")
        || trimmed.starts_with("[completed]")
        || trimmed.starts_with("[failed]")
        || trimmed.starts_with("[interrupted]")
        || trimmed.starts_with("[needs_approval]")
        || trimmed.starts_with("[denied]")
        || trimmed.starts_with("request:")
        || trimmed.starts_with("args:")
        || trimmed.starts_with("stdout:")
        || trimmed.starts_with("stderr:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("metrics:")
        || trimmed.starts_with("↳ ")
}

fn pending_render_signature(app: &App) -> Option<u64> {
    if app.last_render_width == 0 || app.last_render_height == 0 {
        if !app.pending_turn {
            return None;
        }
        let start = app.turn_start?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        focus_ring_frame(start).hash(&mut hasher);
        get_spinner_verb_with_seed(start, app.spinner_seed).hash(&mut hasher);
        app.pending_steers
            .iter()
            .for_each(|message| message.hash(&mut hasher));
        app.pending_queue
            .iter()
            .for_each(|message| message.hash(&mut hasher));
        for line in pending_live_tool_activity_lines(&app.live_transcript, 6) {
            line.hash(&mut hasher);
        }
        return Some(hasher.finish());
    }

    let composer_height = app.composer.height_for_width(app.last_render_width);
    let palette_height = if matches!(app.focus, Focus::CommandPalette) {
        app.command_palette.desired_height() as u16
    } else {
        0
    };
    pending_render_signature_for_geometry(
        app,
        app.last_render_width,
        app.last_render_height,
        composer_height,
        palette_height,
    )
}

fn transcript_preview_signature(app: &App) -> Option<u64> {
    let preview = provisional_assistant_text(&app.live_transcript)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.last_render_width.hash(&mut hasher);
    preview.hash(&mut hasher);
    Some(hasher.finish())
}

#[cfg_attr(not(test), allow(dead_code))]
fn pending_signature_preview_budget(app: &App) -> usize {
    if app.last_render_width == 0 || app.last_render_height == 0 {
        return 6;
    }

    let composer_height = app.composer.height_for_width(app.last_render_width);
    let palette_height = if matches!(app.focus, Focus::CommandPalette) {
        app.command_palette.desired_height() as u16
    } else {
        0
    };
    pending_signature_preview_budget_for_geometry(
        app.last_render_height,
        composer_height,
        palette_height,
    )
}

fn pending_signature_preview_budget_for_geometry(
    height: u16,
    composer_height: u16,
    palette_height: u16,
) -> usize {
    let max_pending_height = pending_band_max_height(height, composer_height, palette_height);
    max_pending_height.saturating_sub(2).max(1) as usize
}

fn pending_band_max_height(height: u16, composer_height: u16, palette_height: u16) -> u16 {
    let reserved_without_pending = 1
        + composer_height
        + if palette_height > 0 {
            1 + palette_height
        } else {
            0
        }
        + 1
        + 1
        + 1;
    height.saturating_sub(reserved_without_pending).max(3)
}

fn pending_render_signature_for_geometry(
    app: &App,
    width: u16,
    height: u16,
    composer_height: u16,
    palette_height: u16,
) -> Option<u64> {
    if !app.pending_turn {
        return None;
    }
    let start = app.turn_start?;
    let max_pending_preview_lines =
        pending_signature_preview_budget_for_geometry(height, composer_height, palette_height);
    let visible_lines =
        pending_live_tool_activity_lines(&app.live_transcript, max_pending_preview_lines);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    focus_ring_frame(start).hash(&mut hasher);
    get_spinner_verb_with_seed(start, app.spinner_seed).hash(&mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    visible_lines.hash(&mut hasher);
    app.pending_steers
        .iter()
        .for_each(|message| message.hash(&mut hasher));
    app.pending_queue
        .iter()
        .for_each(|message| message.hash(&mut hasher));
    Some(hasher.finish())
}

fn build_pending_lines(
    turn_start: Option<std::time::Instant>,
    live_lines: &[String],
    spinner_seed: u64,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
) -> Vec<Line<'static>> {
    let start = turn_start.unwrap_or_else(std::time::Instant::now);
    let spinner_spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} ", focus_ring_frame(start)),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}...", get_spinner_verb_with_seed(start, spinner_seed)),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let content_width = width.saturating_sub(2).max(1) as usize;
    let mut lines = Vec::new();
    let has_visible_reply_after_blank = live_lines
        .iter()
        .position(|line| line.trim().is_empty())
        .is_some_and(|blank_idx| {
            live_lines
                .iter()
                .skip(blank_idx + 1)
                .any(|line| !line.trim().is_empty())
        });
    let mut in_reasoning_block = has_visible_reply_after_blank;

    for line in live_lines {
        if line.trim().is_empty() {
            lines.push(Line::from(""));
            if has_visible_reply_after_blank {
                in_reasoning_block = false;
            }
            continue;
        }

        let style = if in_reasoning_block {
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(ratatui::style::Color::White)
        };
        lines.extend(render_pending_live_line(
            line.as_str(),
            content_width,
            style,
            start,
        ));
    }
    append_pending_input_preview_lines(
        &mut lines,
        pending_steers,
        pending_queue,
        width,
        !live_lines.is_empty(),
    );
    lines.push(Line::from(""));
    lines.push(Line::from(spinner_spans));
    lines
}

fn render_pending_live_line(
    line: &str,
    content_width: usize,
    default_style: Style,
    start: std::time::Instant,
) -> Vec<Line<'static>> {
    if let Some(lines) = render_pending_tool_headline_line(line, content_width, start) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_child_line(line, content_width) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_sample_line(line, content_width) {
        return lines;
    }

    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::raw("  "), Span::styled(wrapped, default_style)]))
        .collect()
}

fn render_pending_tool_headline_line(
    line: &str,
    content_width: usize,
    start: std::time::Instant,
) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("• ").unwrap_or(trimmed);
    let (label, rest, label_style, body_style) = pending_tool_headline_parts(trimmed, start)?;
    let label_text = format!("{label} ");
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(label_text.clone(), label_style),
                        Span::styled(wrapped_line, body_style),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_headline_parts(
    trimmed: &str,
    start: std::time::Instant,
) -> Option<(&'static str, &str, Style, Style)> {
    if let Some(rest) = trimmed.strip_prefix("Called ") {
        return Some((
            "Called",
            rest,
            Style::default()
                .fg(pending_tool_label_color(start))
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(pending_tool_body_color(start))
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Closed ") {
        return Some((
            "Closed",
            rest,
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_GRAY),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Approval ") {
        return Some((
            "Approval",
            rest,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::White),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Denied ") {
        return Some((
            "Denied",
            rest,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED),
        ));
    }

    None
}

fn pending_tool_animation_frame(start: std::time::Instant) -> usize {
    if reduced_motion_enabled() {
        return PENDING_TOOL_LABEL_COLORS.len().saturating_sub(2);
    }
    pending_tool_animation_frame_for_elapsed(start.elapsed())
}

fn pending_tool_animation_frame_for_elapsed(elapsed: Duration) -> usize {
    let frame_count = PENDING_TOOL_LABEL_COLORS.len().max(1) as u64;
    ((elapsed.as_millis() as u64 / PENDING_TOOL_ANIMATION_FRAME_MS.max(1)) % frame_count) as usize
}

fn pending_tool_label_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_LABEL_COLORS
        .get(frame)
        .unwrap_or(&SURFACE_CYAN)
}

fn pending_tool_body_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_BODY_COLORS.get(frame).unwrap_or(&Color::White)
}

fn render_pending_tool_child_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("↳ ")?;
    let (label, rest) = body.split_once(' ').unwrap_or((body, ""));
    let label_text = if rest.is_empty() {
        String::new()
    } else {
        format!("{label} ")
    };
    let (label_style, body_style) = pending_tool_child_styles(label);
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled("↳ ", Style::default().fg(SURFACE_ACCENT)),
                    ];
                    if !label_text.is_empty() {
                        spans.push(Span::styled(label_text.clone(), label_style));
                    }
                    spans.push(Span::styled(wrapped_line, body_style));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_child_styles(label: &str) -> (Style, Style) {
    match label {
        "stdout" => (
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "stderr" => (
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
        ),
        "file" => (
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "metrics" => (
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "request" | "args" => (
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        _ => (
            Style::default().fg(SURFACE_ACCENT),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
    }
}

fn render_pending_tool_sample_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    if !line.starts_with("    ") {
        return None;
    }

    let sample = line.trim_start();
    if sample.is_empty() {
        return None;
    }

    let sample_style = if sample.starts_with('+') {
        Style::default().fg(SURFACE_GREEN)
    } else if sample.starts_with('-') {
        Style::default().fg(SURFACE_RED)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };
    let sample_width = content_width.saturating_sub(4).max(1);

    Some(
        crate::presentation::render_wrapped_literal_display_line(sample, sample_width)
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let guide = if index == 0 { "    " } else { "      " };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(guide, Style::default().fg(SURFACE_DARK_GRAY)),
                    Span::styled(wrapped_line, sample_style),
                ])
            })
            .collect(),
    )
}

fn append_pending_input_preview_lines(
    lines: &mut Vec<Line<'static>>,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
    has_live_preview: bool,
) {
    const MAX_PENDING_PREVIEW_MESSAGES: usize = 3;

    if pending_steers.is_empty() && pending_queue.is_empty() {
        return;
    }

    if has_live_preview || lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::from(""));
    }

    let content_width = width.saturating_sub(6).max(1) as usize;
    let mut remaining_preview_budget = MAX_PENDING_PREVIEW_MESSAGES;
    if !pending_steers.is_empty() {
        push_pending_input_header(
            lines,
            content_width,
            "Messages to be submitted after next tool call",
            Some("Esc"),
            "to interrupt and send immediately",
        );
        let preview_items = pending_steers
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_CYAN)
                        .add_modifier(Modifier::DIM),
                )
            })
            .collect::<Vec<_>>();
        let displayed = push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
        remaining_preview_budget = remaining_preview_budget.saturating_sub(displayed);
    }

    if !pending_queue.is_empty() {
        if !pending_steers.is_empty() {
            lines.push(Line::from(""));
        }
        push_pending_input_header(lines, content_width, "Queued follow-up messages", None, "");
        let preview_items = pending_queue
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_GRAY)
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                )
            })
            .collect::<Vec<_>>();
        push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
    }
}

fn push_pending_input_header(
    lines: &mut Vec<Line<'static>>,
    content_width: usize,
    title: &str,
    key_hint: Option<&str>,
    suffix: &str,
) {
    let mut spans = vec![
        Span::styled(
            "• ",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(title.to_owned(), Style::default().fg(SURFACE_GRAY)),
    ];
    if let Some(key_hint) = key_hint {
        spans.push(Span::styled(
            " (press ".to_owned(),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
        spans.push(Span::styled(
            key_hint.to_owned(),
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {suffix})"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
    }
    for (line_index, wrapped) in crate::presentation::render_wrapped_text_line(
        "",
        &spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        content_width + 2,
    )
    .into_iter()
    .enumerate()
    {
        let prefix = if line_index == 0 { "" } else { "  " };
        lines.push(Line::from(vec![Span::styled(
            format!("{prefix}{wrapped}"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        )]));
    }
}

fn push_pending_input_lines(
    lines: &mut Vec<Line<'static>>,
    messages: &[(&str, Style)],
    content_width: usize,
    first_prefix: &str,
    max_preview_messages: usize,
) -> usize {
    let displayed_messages = messages.len().min(max_preview_messages);
    for (message, message_style) in messages.iter().take(max_preview_messages) {
        let wrapped_lines =
            crate::presentation::render_wrapped_literal_display_line(message, content_width);
        let wrapped_count = wrapped_lines.len();
        for (line_index, wrapped) in wrapped_lines.into_iter().take(3).enumerate() {
            let prefix = if line_index == 0 {
                first_prefix.to_owned()
            } else {
                "      ".to_owned()
            };
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(wrapped, *message_style),
            ]));
        }

        if wrapped_count > 3 {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled("…".to_owned(), *message_style),
            ]));
        }
    }

    let remaining_messages = messages.len().saturating_sub(displayed_messages);
    if remaining_messages > 0 {
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("… +{remaining_messages} more"),
                Style::default()
                    .fg(SURFACE_GRAY)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    }

    displayed_messages
}

fn compact_pending_lines_for_height(
    mut lines: Vec<Line<'static>>,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_height = max_height.max(1) as usize;
    if lines.len() <= max_height {
        return lines;
    }

    let removable_blank_indices = [0usize, lines.len().saturating_sub(1), 2usize];
    for index in removable_blank_indices {
        if lines.len() <= max_height {
            break;
        }
        if lines
            .get(index)
            .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
        {
            lines.remove(index);
        }
    }

    while lines.len() > max_height {
        if let Some(index) = lines.iter().enumerate().skip(2).find_map(|(idx, line)| {
            line.spans
                .iter()
                .all(|span| span.content.trim().is_empty())
                .then_some(idx)
        }) {
            lines.remove(index);
        } else {
            break;
        }
    }

    lines.truncate(max_height);
    lines
}

fn format_cwd(runtime: &CliTurnRuntime) -> String {
    if let Some(path) = runtime.effective_working_directory.as_ref() {
        return path.display().to_string();
    }

    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "~".to_owned())
}

fn build_chat_startup_content(
    runtime: &CliTurnRuntime,
    _options: &CliChatOptions,
    _render_width: usize,
    i18n: &I18nService,
) -> (String, String, Vec<(String, Vec<String>)>, Vec<String>) {
    let version = startup_version_line();
    let mcp_count = runtime.effective_bootstrap_mcp_servers.len();
    let skills = detect_available_skills(runtime.effective_working_directory.as_deref());
    let skill_count = skills.len();

    let tutorial = i18n.text(SurfaceCopy::Tutorial).to_owned();
    let sections = vec![
        (
            i18n.text(SurfaceCopy::StartupSectionSkills).to_owned(),
            vec![skill_count.to_string()],
        ),
        (
            i18n.text(SurfaceCopy::StartupSectionMcp).to_owned(),
            vec![mcp_count.to_string()],
        ),
    ];

    let tips = vec![
        tutorial.clone(),
        i18n.text(SurfaceCopy::StartupTipCommands).to_owned(),
        i18n.text(SurfaceCopy::StartupTipSkills).to_owned(),
        i18n.text(SurfaceCopy::StartupTipQueue).to_owned(),
        i18n.text(SurfaceCopy::StartupTipHistory).to_owned(),
    ];

    (version, tutorial, sections, tips)
}

fn startup_version_line() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn detect_available_skills(root: Option<&Path>) -> Vec<SkillEntry> {
    let mut seen_dirs = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut skills = Vec::new();

    for source in skill_search_roots(root) {
        let normalized_dir = source
            .directory
            .canonicalize()
            .unwrap_or_else(|_| source.directory.clone());
        if !seen_dirs.insert(normalized_dir) {
            continue;
        }

        for skill_dir in skill_dirs_in(source.directory.as_path()) {
            let folder_name = skill_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "skill".to_owned());
            let skill = read_skill_metadata(
                folder_name,
                skill_dir.join("SKILL.md"),
                source.category_tag,
                source.search_label,
            );
            let name_key = skill.name.to_ascii_lowercase();
            if seen_names.insert(name_key) {
                skills.push(skill);
            }
        }
    }

    skills.sort_by(|left, right| {
        skill_source_priority(left.category_tag.as_str())
            .cmp(&skill_source_priority(right.category_tag.as_str()))
            .then_with(|| left.name.cmp(&right.name))
    });
    skills
}

struct SkillSearchRoot {
    directory: std::path::PathBuf,
    category_tag: &'static str,
    search_label: &'static str,
}

fn skill_search_roots(root: Option<&Path>) -> Vec<SkillSearchRoot> {
    let mut roots = Vec::new();
    let repo_skills_dir = root
        .map(|path| path.join("skills"))
        .unwrap_or_else(|| Path::new("skills").to_path_buf());
    roots.push(SkillSearchRoot {
        directory: repo_skills_dir,
        category_tag: "[Repo]",
        search_label: "repo",
    });

    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        roots.push(SkillSearchRoot {
            directory: std::path::PathBuf::from(codex_home).join("skills"),
            category_tag: "[Skill]",
            search_label: "global",
        });
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        roots.push(SkillSearchRoot {
            directory: home.join(".codex").join("skills"),
            category_tag: "[Skill]",
            search_label: "global",
        });
        roots.push(SkillSearchRoot {
            directory: home.join(".agents").join("skills"),
            category_tag: "[Skill]",
            search_label: "agent",
        });
    }

    roots
}

fn skill_dirs_in(skills_dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return Vec::new();
    };

    let mut skill_dirs = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        if path.join("SKILL.md").is_file() {
            skill_dirs.push(path);
            continue;
        }
        let Ok(children) = std::fs::read_dir(path) else {
            continue;
        };
        skill_dirs.extend(
            children
                .filter_map(|child| child.ok())
                .filter(|child| child.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
                .map(|child| child.path())
                .filter(|child| child.join("SKILL.md").is_file()),
        );
    }
    skill_dirs
}

fn skill_source_priority(category_tag: &str) -> u8 {
    match category_tag {
        "[Repo]" => 0,
        "[Skill]" => 1,
        _ => 2,
    }
}

fn read_skill_metadata(
    folder_name: String,
    skill_doc_path: std::path::PathBuf,
    category_tag: &'static str,
    search_label: &'static str,
) -> SkillEntry {
    let Ok(contents) = std::fs::read_to_string(skill_doc_path) else {
        return SkillEntry {
            name: folder_name.clone(),
            description: "available skill".to_owned(),
            search_terms: build_skill_search_terms(
                folder_name.as_str(),
                folder_name.as_str(),
                search_label,
            ),
            category_tag: category_tag.to_owned(),
            source_alias: None,
        };
    };

    let name = parse_skill_frontmatter_value(contents.as_str(), "name")
        .filter(|value| !value.is_empty())
        .unwrap_or(folder_name.clone());
    let description = parse_skill_frontmatter_value(contents.as_str(), "description")
        .filter(|value| !value.is_empty())
        .or_else(|| fallback_skill_description(contents.as_str()))
        .unwrap_or_else(|| "available skill".to_owned());
    let search_terms = build_skill_search_terms(folder_name.as_str(), name.as_str(), search_label);
    let source_alias = (folder_name != name).then_some(folder_name);

    SkillEntry {
        name,
        description,
        search_terms,
        category_tag: category_tag.to_owned(),
        source_alias,
    }
}

fn build_skill_search_terms(folder_name: &str, name: &str, source_label: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for value in [folder_name, name, source_label] {
        if !terms.iter().any(|term| term == value) {
            terms.push(value.to_owned());
        }
        for segment in value.split(|ch: char| ch == '-' || ch == '_' || ch.is_whitespace()) {
            let trimmed = segment.trim();
            if trimmed.len() >= 2 && !terms.iter().any(|term| term == trimmed) {
                terms.push(trimmed.to_owned());
            }
        }
    }
    terms
}

fn parse_skill_frontmatter_value(contents: &str, key: &str) -> Option<String> {
    let lines = contents.lines().collect::<Vec<_>>();
    let mut inside_frontmatter = false;
    let mut frontmatter_consumed = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            if !frontmatter_consumed {
                inside_frontmatter = !inside_frontmatter;
                if !inside_frontmatter {
                    frontmatter_consumed = true;
                }
            }
            continue;
        }

        if inside_frontmatter && let Some(value) = trimmed.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }

    None
}

fn fallback_skill_description(contents: &str) -> Option<String> {
    contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && *line != "---")
        .map(ToOwned::to_owned)
}

fn render_chat_surface_help_lines_with_width(width: usize) -> Vec<String> {
    let queue_restore_shortcut = queue_restore_shortcut_label();
    let mut slash_command_items = slash_command_specs()
        .iter()
        .map(|spec| TuiKeyValueSpec::Plain {
            key: spec.command.to_owned(),
            value: slash_command_help_value(spec),
        })
        .collect::<Vec<_>>();
    slash_command_items.push(TuiKeyValueSpec::Plain {
        key: "$skill-name <request>".to_owned(),
        value: "type an available skill invocation directly in the composer".to_owned(),
    });

    let message_spec = TuiMessageSpec {
        role: "help".to_owned(),
        caption: Some("chat surface".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("slash commands".to_owned()),
                items: slash_command_items,
            },
            TuiSectionSpec::Narrative {
                title: Some("surface controls".to_owned()),
                lines: vec![
                    "Use / or : from an empty composer to open the command palette.".to_owned(),
                    "Type $skill-name directly in the composer, then continue writing the rest of the request."
                        .to_owned(),
                    "When the inline $ suggestion popup is visible, Enter or Tab confirms the current skill."
                        .to_owned(),
                    "Use Ctrl+O to expand or collapse the latest compaction summary.".to_owned(),
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("keyboard".to_owned()),
                lines: vec![
                    "Enter sends the current draft. Shift+Enter inserts a new line."
                        .to_owned(),
                    format!(
                        "Tab moves between composer and transcript. While a turn is running, Tab queues the current draft and {queue_restore_shortcut} restores the latest queued message."
                    ),
                    "PgUp / PgDn and Home / End scroll the transcript; printable keys return to the composer immediately."
                        .to_owned(),
                ],
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("mouse".to_owned()),
                lines: vec![
                    "Mouse wheel scrolls the transcript where terminal alternate-scroll is supported."
                        .to_owned(),
                    "Native terminal drag-selection remains available by default.".to_owned(),
                ],
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("usage notes".to_owned()),
                lines: vec![
                    "Type any non-command text to send a normal assistant turn.".to_owned(),
                    "Available skill names can be invoked directly with $skill-name."
                        .to_owned(),
                    "Use Ctrl+C to leave chat.".to_owned(),
                ],
            },
        ],
        footer_lines: vec![
            "Send normal text to continue the transcript.".to_owned(),
            "Use /usage, /review, or /compact when you need to inspect or stabilize the current session."
                .to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    published_at: Option<String>,
    html_url: Option<String>,
    body: Option<String>,
}

async fn load_startup_release_lines(width: usize) -> Option<Vec<String>> {
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent("loongclaw-chat-surface")
        .build()
        .ok()?;
    let response = tokio::time::timeout(
        Duration::from_millis(1500),
        client
            .get("https://api.github.com/repos/eastreams/loong/releases/latest")
            .send(),
    )
    .await
    .ok()?
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let release: GithubRelease = response.json().await.ok()?;
    format_startup_release_lines(&release, &current, width)
}

fn format_startup_release_lines(
    release: &GithubRelease,
    current: &str,
    width: usize,
) -> Option<Vec<String>> {
    if normalize_tag(&release.tag_name) == normalize_tag(current) {
        return None;
    }

    let rule = "─".repeat(width.max(12));
    let mut lines = vec![
        rule.clone(),
        " What's New".to_owned(),
        String::new(),
        format!(
            " [{}]{}",
            release.tag_name,
            release
                .published_at
                .as_deref()
                .and_then(|value| value.get(..10))
                .map(|date| format!(" - {date}"))
                .unwrap_or_default()
        ),
        String::new(),
    ];

    let mut added = 0usize;
    for line in release.body.as_deref().unwrap_or_default().lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        lines.push(trimmed.to_owned());
        added += 1;
        if added >= 28 {
            break;
        }
    }

    if let Some(url) = release.html_url.as_deref() {
        lines.push(String::new());
        lines.push(format!(" Release: {url}"));
    }
    lines.push(rule);
    Some(lines)
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('v').to_ascii_lowercase()
}

fn resize_reflow_required(
    previous_width: u16,
    previous_height: u16,
    next_width: u16,
    next_height: u16,
) -> bool {
    previous_width != next_width || previous_height != next_height
}

fn resize_live_rerender_ready(
    pending_live_resize_rerender: bool,
    since_last_resize: Option<Duration>,
) -> bool {
    pending_live_resize_rerender
        && since_last_resize
            .map(|elapsed| elapsed >= Duration::from_millis(70))
            .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::{
        App, Focus, LiveTranscriptState, StartupBootstrapCapture, StartupOnboardingAction,
        StartupOnboardingInteractionKind, StartupOnboardingStage, StartupOnboardingState,
        StartupPersonalizationPreset, StartupProviderOption, StartupSetupPathChoice,
        StartupSkillOption, persist_startup_personalization, startup_eye_animation_for_state,
    };
    use crate::chat::chat_surface::command_palette::{
        CommandAction, CommandPalette, SkillEntry, slash_command_specs,
    };
    use crate::chat::chat_surface::composer::Composer;
    use crate::chat::chat_surface::i18n::{I18nService, Language};
    use crate::chat::chat_surface::message_list::{
        MessageList, StartupEyeAnimation, StartupEyeFocus,
    };
    use crate::chat::chat_surface::utils::SURFACE_USER_MSG_BG;
    use crate::chat::{
        CliChatOptions, CliSessionRequirement, initialize_cli_turn_runtime_with_loaded_config,
    };
    use crate::config::{LoongConfig, ProviderConfig, ProviderKind, ReasoningEffort};
    use crate::test_support::{ScopedEnv, unique_temp_dir};
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Style};
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    fn blank_app() -> App {
        App {
            message_list: MessageList::new(),
            composer: Composer::new(),
            command_palette: CommandPalette::new(Language::En, Vec::new()),
            focus: Focus::Composer,
            pending_turn: false,
            turn_start: None,
            live_transcript: Arc::new(StdMutex::new(LiveTranscriptState::default())),
            pending_task: None,
            pending_steers: Default::default(),
            pending_queue: Default::default(),
            composer_follow_up_intent: false,
            pending_first_turn_bootstrap_addendum: None,
            awaiting_first_turn_bootstrap_reply: false,
            live_render_width: Arc::new(AtomicUsize::new(1)),
            live_rerender: None,
            spinner_seed: 1,
            last_pending_signature: None,
            last_live_transcript_signature: None,
            pending_render_cache: None,
            inline_skill_popup_active: false,
            last_render_width: 0,
            last_render_height: 0,
            last_transcript_area: Rect::default(),
            last_composer_area: Rect::default(),
            last_palette_area: Rect::default(),
            startup_onboarding: None,
            startup_version: "v0.1.0".to_owned(),
            startup_mcp_count: 0,
            detected_skills: Vec::new(),
            cwd: "/tmp/example".to_owned(),
            model: "gpt-test".to_owned(),
            title: None,
            i18n: I18nService::new(Language::En),
        }
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn skill(name: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: format!("{name} description"),
            search_terms: vec![name.to_owned()],
            category_tag: "[Skill]".to_owned(),
            source_alias: None,
        }
    }

    fn onboarding_state() -> StartupOnboardingState {
        StartupOnboardingState {
            stage: StartupOnboardingStage::Language,
            language_options: vec![
                Language::En,
                Language::ZhCn,
                Language::ZhTw,
                Language::Ja,
                Language::Ru,
            ],
            language_index: 0,
            provider_options: vec![StartupProviderOption {
                kind: ProviderKind::Openai,
                auth_env_name: Some("OPENAI_API_KEY".to_owned()),
                is_current: true,
                label: "OpenAI".to_owned(),
                detail: "reuse the current config".to_owned(),
                recommended: true,
            }],
            provider_index: 0,
            skill_options: vec![StartupSkillOption {
                install_id: "agent-browser".to_owned(),
                display_name: "Agent Browser".to_owned(),
                summary: "browser automation".to_owned(),
                recommended: true,
            }],
            selected_skill_ids: BTreeSet::new(),
            skill_cursor: 0,
            setup_path_index: 0,
            personalization_index: 0,
            selected_personalization: None,
            web_search_provider_label: "DuckDuckGo".to_owned(),
            web_search_provider_detail: "web search still needs auth".to_owned(),
            provider_auth_env_name: None,
            provider_configuration_hint: None,
            enabled_channel_labels: Vec::new(),
            channel_follow_up_commands: Vec::new(),
            channel_status_commands: Vec::new(),
            channel_repair_commands: Vec::new(),
            startup_mcp_count: 0,
            detected_skill_count: 1,
            feedback: Some("demo feedback".to_owned()),
            last_interaction_at: std::time::Instant::now() - Duration::from_secs(5),
            last_interaction_kind: StartupOnboardingInteractionKind::Passive,
        }
    }

    #[test]
    fn startup_eye_animation_tracks_active_onboarding_focus() {
        let mut state = onboarding_state();

        assert_eq!(
            startup_eye_animation_for_state(Some(&state)),
            StartupEyeAnimation::Focus(StartupEyeFocus::DownLeft)
        );

        state.stage = StartupOnboardingStage::Provider;
        state.provider_options = vec![
            StartupProviderOption {
                kind: ProviderKind::Openai,
                auth_env_name: None,
                is_current: true,
                label: "first".to_owned(),
                detail: "first".to_owned(),
                recommended: true,
            },
            StartupProviderOption {
                kind: ProviderKind::Anthropic,
                auth_env_name: None,
                is_current: false,
                label: "middle".to_owned(),
                detail: "middle".to_owned(),
                recommended: false,
            },
            StartupProviderOption {
                kind: ProviderKind::Gemini,
                auth_env_name: None,
                is_current: false,
                label: "last".to_owned(),
                detail: "last".to_owned(),
                recommended: false,
            },
        ];
        state.provider_index = 1;
        assert_eq!(
            startup_eye_animation_for_state(Some(&state)),
            StartupEyeAnimation::Focus(StartupEyeFocus::DownCenter)
        );

        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = StartupSetupPathChoice::ProviderAndWeb as usize;
        assert_eq!(
            startup_eye_animation_for_state(Some(&state)),
            StartupEyeAnimation::Thinking(StartupEyeFocus::Right)
        );

        state.stage = StartupOnboardingStage::Skills;
        state.selected_skill_ids.insert("agent-browser".to_owned());
        assert_eq!(
            startup_eye_animation_for_state(Some(&state)),
            StartupEyeAnimation::Thinking(StartupEyeFocus::DownCenter)
        );

        state.stage = StartupOnboardingStage::Finish;
        assert_eq!(
            startup_eye_animation_for_state(Some(&state)),
            StartupEyeAnimation::Celebrate
        );
    }

    fn test_runtime_with_path(path: PathBuf) -> crate::chat::CliTurnRuntime {
        let mut config = LoongConfig::default();
        config.audit.path = unique_temp_dir("loong-chat-surface-audit")
            .join("audit")
            .join("events.jsonl")
            .display()
            .to_string();

        initialize_cli_turn_runtime_with_loaded_config(
            path,
            config,
            Some("chat-surface-test"),
            &CliChatOptions::default(),
            "chat-surface-test",
            CliSessionRequirement::RequireExplicit,
            false,
        )
        .expect("chat surface runtime")
    }

    #[test]
    fn resize_reflow_tracks_width_and_height_changes() {
        assert!(super::resize_reflow_required(80, 24, 72, 24));
        assert!(super::resize_reflow_required(80, 24, 80, 32));
        assert!(!super::resize_reflow_required(80, 24, 80, 24));
    }

    #[test]
    fn resize_live_rerender_waits_for_quiet_window() {
        assert!(!super::resize_live_rerender_ready(false, None));
        assert!(super::resize_live_rerender_ready(true, None));
        assert!(!super::resize_live_rerender_ready(
            true,
            Some(Duration::from_millis(32))
        ));
        assert!(super::resize_live_rerender_ready(
            true,
            Some(Duration::from_millis(70))
        ));
    }

    #[test]
    fn pending_tool_animation_frames_cycle_between_dim_and_bright_states() {
        let early = super::pending_tool_animation_frame_for_elapsed(Duration::from_millis(0));
        let bright = super::pending_tool_animation_frame_for_elapsed(Duration::from_millis(360));

        assert_ne!(early, bright);
        assert_eq!(
            super::PENDING_TOOL_LABEL_COLORS[early],
            super::SURFACE_DIM_GRAY
        );
        assert_eq!(
            super::PENDING_TOOL_LABEL_COLORS[bright],
            super::Color::White
        );
    }

    fn sample_release() -> super::GithubRelease {
        super::GithubRelease {
            tag_name: "v9.9.9".to_owned(),
            published_at: Some("2026-04-20T00:00:00Z".to_owned()),
            html_url: Some("https://github.com/eastreams/loong/releases/tag/v9.9.9".to_owned()),
            body: Some(
                "- Added a very long changelog line that should wrap cleanly inside narrow startup surfaces without overflowing the transcript width.".to_owned(),
            ),
        }
    }

    fn buffer_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn find_row(terminal: &Terminal<TestBackend>, needle: &str) -> Option<u16> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        for y in 0..area.height {
            let line = (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>();
            if line.contains(needle) {
                return Some(y);
            }
        }
        None
    }

    fn row_has_background(
        terminal: &Terminal<TestBackend>,
        row: u16,
        bg: ratatui::style::Color,
    ) -> bool {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        (0..area.width).all(|x| buf[(x, row)].bg == bg)
    }

    #[test]
    fn status_footer_truncates_long_cwd_from_the_left() {
        let cwd = std::env::current_dir()
            .expect("current dir")
            .join("nested")
            .join("session-tail-for-footer-test");
        let cwd = cwd.to_string_lossy();
        let line = super::build_status_footer_line(cwd.as_ref(), "gpt-5.4", 32);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(crate::presentation::display_width(&rendered), 32);
        assert!(rendered.contains("gpt-5.4"));
        assert!(rendered.contains("…"));
        assert!(rendered.contains("footer-test"));
        assert_eq!(rendered.chars().next(), cwd.chars().next());
    }

    #[test]
    fn status_footer_truncates_model_when_width_is_extremely_narrow() {
        let line =
            super::build_status_footer_line("/tmp/project", "gpt-5.4-super-long-model-name", 12);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(crate::presentation::display_width(&rendered), 12);
        assert!(rendered.contains("…"));
    }

    #[test]
    fn status_footer_respects_display_width_for_cjk_paths() {
        let line = super::build_status_footer_line("/tmp/项目/聊天记录", "gpt-5.4", 16);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(crate::presentation::display_width(&rendered), 16);
        assert!(rendered.contains("gpt-5.4"));
    }

    #[test]
    fn middle_truncation_preserves_both_path_ends() {
        let path = std::env::current_dir()
            .expect("current dir")
            .join("worktrees")
            .join("project-name")
            .join("session");
        let path = path.to_string_lossy();
        let truncated = super::truncate_middle_for_width(path.as_ref(), 20);

        assert_eq!(truncated.chars().next(), path.chars().next());
        assert!(truncated.ends_with("session"));
        assert_eq!(crate::presentation::display_width(&truncated), 20);
    }

    #[test]
    fn startup_release_lines_wrap_to_requested_width() {
        let release = sample_release();
        let lines =
            super::format_startup_release_lines(&release, "v0.1.0", 80).expect("release lines");
        let mut list = MessageList::new();
        list.add_rendered_lines(lines);

        let rendered = list
            .get_rendered_lines(24)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| line.is_empty() || crate::presentation::display_width(line) <= 24)
        );
        assert!(rendered.iter().any(|line| line.contains("What's New")));
        assert!(rendered.iter().any(|line| line.contains("Release:")));
    }

    #[test]
    fn startup_release_lines_skip_current_version() {
        let release = sample_release();

        assert!(super::format_startup_release_lines(&release, "v9.9.9", 24).is_none());
    }

    #[test]
    fn startup_version_line_is_product_only() {
        let version = super::startup_version_line();

        assert_eq!(version, format!("v{}", env!("CARGO_PKG_VERSION")));
        assert!(!version.contains(" · "));
    }

    #[test]
    fn queue_footer_truncates_to_available_width() {
        let line = super::build_queue_footer_line(&I18nService::new(Language::En), 12, 14);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(crate::presentation::display_width(&rendered), 14);
        assert!(rendered.contains("queued ×12"));
    }

    #[test]
    fn queue_footer_prefers_short_hint_before_truncating() {
        let line = super::build_queue_footer_line(&I18nService::new(Language::En), 2, 20);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("Tab to queue"));
        assert!(!rendered.contains("Tab to queue message"));
    }

    #[test]
    fn restore_footer_truncates_to_available_width() {
        let line = super::build_restore_footer_line(&I18nService::new(Language::En), 12, 14);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(crate::presentation::display_width(&rendered), 14);
        assert!(rendered.contains("restore ×12"));
    }

    #[test]
    fn restore_footer_prefers_short_hint_before_truncating() {
        let line = super::build_restore_footer_line(&I18nService::new(Language::En), 2, 32);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("restore queued"));
        assert!(!rendered.contains("to restore queued message"));
    }

    #[test]
    fn footer_tracks_content_when_transcript_is_short() {
        let backend = TestBackend::new(50, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.message_list.add_assistant_message("hello".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let footer_row = lines
            .iter()
            .position(|line| line.contains("/tmp/example"))
            .expect("footer row");

        assert!(footer_row < lines.len().saturating_sub(1));
    }

    #[test]
    fn wrapped_composer_expands_before_footer() {
        let backend = TestBackend::new(16, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.composer.set_input("abcdefg".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let footer_row = lines
            .iter()
            .position(|line| line.contains("gpt-test"))
            .expect("footer row");
        let wrapped_row = lines
            .iter()
            .enumerate()
            .find_map(|(idx, line)| line.contains("defg").then_some(idx))
            .expect("wrapped composer row");

        assert!(footer_row > wrapped_row);
    }

    #[test]
    fn footer_keeps_one_breathing_row_when_transcript_fills_available_height() {
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..8 {
            app.message_list.add_user_message(format!("msg-{idx}"));
            app.message_list
                .add_assistant_message(format!("reply-{idx}"));
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let footer_row = lines
            .iter()
            .position(|line| line.contains("/tmp/example"))
            .expect("footer row");

        assert_eq!(
            footer_row,
            lines
                .len()
                .saturating_sub(super::FOOTER_BOTTOM_BREATHING_HEIGHT as usize + 1)
        );
        assert!(lines.last().is_some_and(|line| line.trim().is_empty()));
    }

    #[test]
    fn footer_content_uses_left_indent_when_space_allows() {
        let backend = TestBackend::new(50, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.message_list.add_assistant_message("hello".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let footer_line = lines
            .iter()
            .find(|line| line.contains("/tmp/example"))
            .expect("footer line");

        assert!(footer_line.starts_with("  /tmp/example"));
    }

    #[test]
    fn pending_band_hides_plain_live_reply_lines() {
        let backend = TestBackend::new(50, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("streamed reply line".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let transcript_row = lines
            .iter()
            .position(|line| line.contains("streamed reply line"))
            .expect("provisional transcript row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        assert!(transcript_row < composer_row);
    }

    #[test]
    fn composer_and_footer_only_reclaim_pending_preview_rows_after_turn_finishes() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("streamed reply line".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw pending");
        let pending_lines = buffer_lines(&terminal);
        let pending_composer_row = pending_lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("pending composer row");
        let pending_footer_row = pending_lines
            .iter()
            .position(|line| line.contains("/tmp/example"))
            .expect("pending footer row");

        app.pending_turn = false;
        app.turn_start = None;
        if let Ok(mut live) = app.live_transcript.lock() {
            *live = LiveTranscriptState::default();
        }
        app.message_list
            .add_assistant_message("streamed reply line".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw complete");
        let settled_lines = buffer_lines(&terminal);
        let settled_composer_row = settled_lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("settled composer row");
        let settled_footer_row = settled_lines
            .iter()
            .position(|line| line.contains("/tmp/example"))
            .expect("settled footer row");

        let composer_reclaimed_rows = pending_composer_row.saturating_sub(settled_composer_row);
        let footer_reclaimed_rows = pending_footer_row.saturating_sub(settled_footer_row);

        assert!(
            composer_reclaimed_rows <= 2,
            "composer should only reclaim the pending preview rows, got pending={pending_composer_row} settled={settled_composer_row}"
        );
        assert!(
            footer_reclaimed_rows <= 2,
            "footer should only reclaim the pending preview rows, got pending={pending_footer_row} settled={settled_footer_row}"
        );
    }

    #[test]
    fn spinner_stays_adjacent_to_composer_when_plain_live_reply_is_hidden() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("streamed reply line".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let spinner_row = lines
            .iter()
            .position(|line| line.contains("..."))
            .expect("spinner row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        let preview_row = lines
            .iter()
            .position(|line| line.contains("streamed reply line"))
            .expect("preview row");

        assert!(composer_row > spinner_row);
        assert!(preview_row < composer_row);
    }

    #[test]
    fn split_surface_command_preserves_arguments() {
        assert_eq!(
            super::split_surface_command("/copy explicit text"),
            ("/copy", "explicit text")
        );
        assert_eq!(super::split_surface_command("  /diff  "), ("/diff", ""));
    }

    #[test]
    fn recognized_surface_command_only_accepts_known_builtins() {
        assert_eq!(
            super::recognized_surface_command("/model gpt-5"),
            Some("/model gpt-5".to_owned())
        );
        assert_eq!(
            super::recognized_surface_command(":settings provider"),
            Some("/settings provider".to_owned())
        );
        assert_eq!(super::recognized_surface_command("/workers"), None);
        assert_eq!(
            super::recognized_surface_command("/fast_lane_summary"),
            None
        );
        assert_eq!(
            super::recognized_surface_command("/safe_lane_summary"),
            None
        );
        assert_eq!(
            super::recognized_surface_command("/turn_checkpoint_summary"),
            None
        );
        assert_eq!(
            super::recognized_surface_command("/turn_checkpoint_repair"),
            None
        );
        assert_eq!(super::recognized_surface_command("/unknown note"), None);
        assert_eq!(super::recognized_surface_command(":unknown note"), None);
        assert_eq!(super::recognized_surface_command("plain text"), None);
    }

    #[test]
    fn staging_commands_populate_composer_drafts() {
        let mut app = blank_app();
        app.message_list
            .add_assistant_message("existing answer".to_owned());

        super::stage_simplify_prompt(&mut app, "").expect("simplify stage");
        assert!(app.composer.text().contains("existing answer"));
        assert!(app.composer.text().contains("simplify"));

        super::stage_plan_prompt(&mut app, "the rollout").expect("plan stage");
        assert!(app.composer.text().contains("the rollout"));
    }

    #[test]
    fn export_filename_components_are_safe() {
        assert_eq!(super::safe_file_component("abc-DEF_123"), "abc-DEF_123");
        assert_eq!(super::safe_file_component("a/b:c"), "a-b-c");
    }

    #[test]
    fn help_lines_match_chat_surface_controls() {
        let rendered = super::render_chat_surface_help_lines_with_width(80).join("\n");

        assert!(rendered.contains("Shift+Enter inserts a new line"));
        assert!(rendered.contains("Use / or : from an empty composer"));
        assert!(rendered.contains("Type $skill-name directly in the composer"));
        assert!(rendered.contains("printable keys return"));
        assert!(rendered.contains("Native terminal drag-selection remains available"));
        assert!(!rendered.contains("coming soon"));
        assert!(!rendered.contains("A trailing \\\\ keeps composing"));
        assert!(!rendered.contains("control deck"));
        assert!(!rendered.contains("Esc from an empty composer"));
    }

    #[test]
    fn slash_usage_and_detail_cards_are_enabled_without_placeholder_copy() {
        let usage = super::render_slash_command_usage_lines_with_width(90).join("\n");
        assert!(usage.contains("Every command stays visible"));
        assert!(!usage.contains("coming soon"));
        assert!(!usage.contains("placeholder"));
        assert!(!usage.contains("not wired"));

        let share_spec = slash_command_specs()
            .iter()
            .find(|spec| spec.command == "/share")
            .expect("/share spec");
        let detail = super::render_slash_command_detail_lines_with_width(share_spec, 90).join("\n");
        assert!(detail.contains("enabled"));
        assert!(detail.contains("/share is available"));
        assert!(detail.contains("write a local transcript artifact"));
        assert!(!detail.contains("coming soon"));
        assert!(!detail.contains("placeholder"));
        assert!(!detail.contains("not wired"));
    }

    #[test]
    fn permissions_command_keeps_yolo_default_copy_simple() {
        let rendered = super::render_permissions_command_lines_with_width(80).join("\n");

        assert!(rendered.contains("YOLO by default"));
        assert!(rendered.contains("Hey yo, you only live once, take care."));
        assert!(rendered.contains("commands"));
        assert!(rendered.contains("enabled"));
        assert!(rendered.contains("not part of the happy path"));
        assert!(!rendered.contains("current policy"));
        assert!(!rendered.contains("shell allow"));
        assert!(!rendered.contains("shell deny"));
        assert!(!rendered.contains("file root"));
    }

    #[test]
    fn experimental_command_reports_enabled_surface_features() {
        let rendered = super::render_experimental_command_lines_with_width(80).join("\n");

        assert!(rendered.contains("streaming renderer"));
        assert!(rendered.contains("startup animation"));
        assert!(rendered.contains("resize smoothing"));
        assert!(rendered.contains("enabled"));
        assert!(!rendered.contains("disabled"));
        assert!(!rendered.contains("toggles remain config-driven"));
    }

    #[test]
    fn typing_dollar_keeps_focus_in_composer_while_inline_skill_popup_filters() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(
            Language::En,
            vec![skill("demo-skill"), skill("other-skill")],
        );

        assert!(
            app.composer
                .handle_key(crossterm::event::KeyEvent::new(
                    KeyCode::Char('$'),
                    KeyModifiers::NONE,
                ))
                .is_none()
        );
        app.sync_inline_skill_popup();
        assert_eq!(app.focus, Focus::Composer);
        assert!(app.inline_skill_popup_active);
        assert_eq!(app.composer.text(), "$");

        assert!(
            app.composer
                .handle_key(crossterm::event::KeyEvent::new(
                    KeyCode::Char('d'),
                    KeyModifiers::NONE,
                ))
                .is_none()
        );
        app.sync_inline_skill_popup();

        if let Some(action) = app
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))
        {
            let _ = app.apply_palette_action(action);
        }
        assert_eq!(app.composer.text(), "$demo-skill ");
        assert_eq!(app.focus, Focus::Composer);
    }

    #[test]
    fn typing_dollar_without_available_skills_keeps_plain_text_without_popup() {
        let mut app = blank_app();

        assert!(
            app.composer
                .handle_key(crossterm::event::KeyEvent::new(
                    KeyCode::Char('$'),
                    KeyModifiers::NONE,
                ))
                .is_none()
        );
        app.sync_inline_skill_popup();

        assert_eq!(app.focus, Focus::Composer);
        assert!(!app.inline_skill_popup_active);
        assert_eq!(app.composer.text(), "$");
    }

    #[test]
    fn confirming_inline_skill_popup_with_no_matches_closes_popup_and_keeps_text() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$zzz".to_owned());
        app.sync_inline_skill_popup();

        assert!(app.inline_skill_popup_active);

        app.confirm_inline_skill_popup();

        assert_eq!(app.composer.text(), "$zzz");
        assert_eq!(app.focus, Focus::Composer);
        assert!(!app.inline_skill_popup_active);
    }

    #[test]
    fn read_skill_metadata_prefers_frontmatter_name_and_description() {
        let skill = super::read_skill_metadata(
            "folder-fallback".to_owned(),
            std::path::PathBuf::from("/tmp/nonexistent")
                .with_file_name("skill.md")
                .with_extension("tmp"),
            "[Repo]",
            "repo",
        );
        assert_eq!(skill.name, "folder-fallback");
        assert_eq!(skill.description, "available skill");
        assert_eq!(skill.category_tag, "[Repo]");

        let contents = r#"---
name: actual-skill
description: "actual description"
---

# Skill
"#;
        let dir = std::env::temp_dir().join(format!(
            "loong-chat-skill-meta-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("SKILL.md");
        std::fs::write(&path, contents).expect("write");

        let skill = super::read_skill_metadata(
            "folder-fallback".to_owned(),
            path.clone(),
            "[Repo]",
            "repo",
        );
        assert_eq!(skill.name, "actual-skill");
        assert_eq!(skill.description, "actual description");
        assert!(skill.search_terms.iter().any(|term| term == "repo"));

        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn detect_available_skills_reads_skill_metadata_from_workspace() {
        let root = std::env::temp_dir().join(format!(
            "loong-chat-skills-root-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let skills_dir = root.join("skills");
        std::fs::create_dir_all(&skills_dir).expect("mkdir skills");

        let alpha_dir = skills_dir.join("alpha");
        std::fs::create_dir_all(&alpha_dir).expect("mkdir alpha");
        std::fs::write(
            alpha_dir.join("SKILL.md"),
            "---\nname: alpha-skill\ndescription: alpha description\n---\n",
        )
        .expect("write alpha");

        let beta_dir = skills_dir.join("beta");
        std::fs::create_dir_all(&beta_dir).expect("mkdir beta");
        std::fs::write(
            beta_dir.join("SKILL.md"),
            "# Beta\nbeta fallback description\n",
        )
        .expect("write beta");

        let skills = super::detect_available_skills(Some(root.as_path()));

        let alpha = skills
            .iter()
            .find(|skill| skill.name == "alpha-skill")
            .expect("alpha skill");
        assert_eq!(alpha.description, "alpha description");
        assert_eq!(alpha.category_tag, "[Repo]");
        assert!(alpha.search_terms.iter().any(|term| term == "alpha"));

        let beta = skills
            .iter()
            .find(|skill| skill.name == "beta")
            .expect("beta skill");
        assert_eq!(beta.description, "beta fallback description");
        assert_eq!(beta.category_tag, "[Repo]");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn build_skill_search_terms_includes_folder_name_and_source_segments() {
        let terms = super::build_skill_search_terms("babysit-pr", "PR Babysitter", "repo");

        assert!(terms.iter().any(|term| term == "babysit-pr"));
        assert!(terms.iter().any(|term| term == "babysit"));
        assert!(terms.iter().any(|term| term == "pr"));
        assert!(terms.iter().any(|term| term == "PR Babysitter"));
        assert!(terms.iter().any(|term| term == "Babysitter"));
        assert!(terms.iter().any(|term| term == "repo"));
    }

    #[test]
    fn confirming_inline_skill_popup_keeps_focus_in_composer() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$dem".to_owned());
        app.sync_inline_skill_popup();

        assert!(app.inline_skill_popup_active);

        app.confirm_inline_skill_popup();

        assert_eq!(app.composer.text(), "$demo-skill ");
        assert_eq!(app.focus, Focus::Composer);
        assert!(!app.inline_skill_popup_active);
    }

    #[test]
    fn tab_confirms_inline_skill_popup_through_shared_key_handler() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$dem".to_owned());
        app.sync_inline_skill_popup();

        assert!(
            app.handle_inline_skill_popup_key(crossterm::event::KeyEvent::new(
                KeyCode::Tab,
                KeyModifiers::NONE,
            ))
        );

        assert_eq!(app.composer.text(), "$demo-skill ");
        assert_eq!(app.focus, Focus::Composer);
        assert!(!app.inline_skill_popup_active);
    }

    #[test]
    fn confirming_inline_skill_keeps_surrounding_text_stable() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("please $dem now".to_owned());
        for _ in 0..4 {
            let _ = app.composer.handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Left,
                KeyModifiers::NONE,
            ));
        }
        app.sync_inline_skill_popup();

        app.confirm_inline_skill_popup();

        assert_eq!(app.composer.text(), "please $demo-skill now");
    }

    #[test]
    fn confirming_inline_skill_works_with_cursor_inside_token_middle() {
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$demo now".to_owned());
        for _ in 0..4 {
            let _ = app.composer.handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Left,
                KeyModifiers::NONE,
            ));
        }
        app.sync_inline_skill_popup();

        app.confirm_inline_skill_popup();

        assert_eq!(app.composer.text(), "$demo-skill now");
    }

    #[test]
    fn inline_skill_popup_mouse_click_works_while_composer_keeps_focus() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$dem".to_owned());
        app.sync_inline_skill_popup();

        terminal.draw(|f| app.render(f)).expect("draw");
        let palette_row = app.last_palette_area.y;
        let palette_col = app.last_palette_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            palette_col,
            palette_row,
        ));

        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.text(), "$demo-skill ");
    }

    #[test]
    fn inline_skill_popup_mouse_scroll_updates_selection_while_composer_stays_focused() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(
            Language::En,
            vec![skill("demo-skill"), skill("other-skill")],
        );
        app.composer.set_input("$".to_owned());
        app.sync_inline_skill_popup();

        terminal.draw(|f| app.render(f)).expect("draw");
        let palette_row = app.last_palette_area.y;
        let palette_col = app.last_palette_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, palette_col, palette_row));
        app.confirm_inline_skill_popup();

        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.text(), "$other-skill ");
    }

    #[test]
    fn mouse_scroll_routes_to_transcript_even_with_a_draft() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..14 {
            app.message_list
                .add_assistant_message(format!("line-{idx}"));
        }
        app.composer.set_input("draft".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let before = buffer_lines(&terminal).join("\n");

        let transcript_row = app.last_transcript_area.y.saturating_add(1);
        let transcript_col = app.last_transcript_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::ScrollUp,
            transcript_col,
            transcript_row,
        ));

        terminal.draw(|f| app.render(f)).expect("draw after scroll");
        let after = buffer_lines(&terminal).join("\n");

        assert!(app.message_list.scroll_offset_for_test() > 0);
        assert_ne!(before, after);
        assert_eq!(app.focus, Focus::Composer);
    }

    #[test]
    fn footer_shows_follow_hint_when_transcript_is_off_tail() {
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..14 {
            app.message_list
                .add_assistant_message(format!("line-{idx}"));
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        app.message_list.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        ));
        terminal.draw(|f| app.render(f)).expect("draw off tail");
        let lines = buffer_lines(&terminal).join("\n");

        assert!(lines.contains("PgDn / End"));
        assert!(!lines.contains("/tmp/example"));
    }

    #[test]
    fn footer_returns_to_status_line_when_tail_is_restored() {
        let backend = TestBackend::new(50, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..14 {
            app.message_list
                .add_assistant_message(format!("line-{idx}"));
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        app.message_list.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        ));
        terminal.draw(|f| app.render(f)).expect("draw off tail");
        app.message_list.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        ));
        terminal
            .draw(|f| app.render(f))
            .expect("draw tail restored");
        let lines = buffer_lines(&terminal).join("\n");

        assert!(lines.contains("/tmp/example"));
        assert!(!lines.contains("PgDn / End"));
    }

    #[test]
    fn mouse_scroll_over_palette_changes_selection_without_scrolling_transcript() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..10 {
            app.message_list
                .add_assistant_message(format!("line-{idx}"));
        }
        app.message_list.set_scroll_offset_for_test(4);
        app.command_palette.show_commands(":");
        app.focus = Focus::CommandPalette;

        terminal.draw(|f| app.render(f)).expect("draw");
        let palette_row = app.last_palette_area.y.saturating_add(1);
        let palette_col = app.last_palette_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, palette_col, palette_row));

        assert_eq!(app.message_list.scroll_offset_for_test(), 4);
        match app
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )) {
            Some(CommandAction::RunCommand("/permissions")) => {}
            other => {
                panic!("expected palette mouse scroll to land on /permissions, got {other:?}")
            }
        }
    }

    #[test]
    fn slash_palette_open_and_sync_mirror_query_into_composer() {
        let mut app = blank_app();

        super::open_slash_command_palette(&mut app, '/', "");
        assert_eq!(app.focus, Focus::CommandPalette);
        assert_eq!(app.composer.text(), "/");

        let _ = app
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char('m'),
                KeyModifiers::NONE,
            ));
        let _ = app
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char('o'),
                KeyModifiers::NONE,
            ));
        super::sync_slash_palette_composer(&mut app);

        assert_eq!(app.composer.text(), "/mo");
    }

    #[test]
    fn clearing_slash_palette_buffer_resets_composer() {
        let mut app = blank_app();
        super::open_slash_command_palette(&mut app, '/', "model");

        super::clear_slash_palette_composer(&mut app);

        assert!(app.composer.is_empty());
    }

    #[test]
    fn model_palette_entries_open_reasoning_for_reasoning_capable_models() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-model-palette-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let runtime = test_runtime_with_path(config_path);
        let current_model = runtime.config.provider.model.clone();

        let entries = super::build_model_palette_entries(
            &runtime,
            &[crate::provider::ProviderModelCatalogEntry {
                model: current_model.clone(),
                display_name: None,
                description: None,
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                supported_reasoning_effort_descriptions: Vec::new(),
            }],
        );

        let entry = entries
            .iter()
            .find(|entry| entry.label == current_model)
            .expect("current model entry");
        assert_eq!(entry.status_tag.as_deref(), Some("current"));
        assert!(matches!(
            entry.action,
            CommandAction::OpenModelReasoning(ref entry) if entry.model == current_model
        ));
    }

    #[test]
    fn reasoning_palette_entries_include_default_and_current_effort() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-reasoning-palette-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);
        runtime.config.provider.reasoning_effort = Some(ReasoningEffort::High);
        let current_model = runtime.config.provider.model.clone();

        let (entries, selected_label) = super::build_reasoning_palette_entries(
            &runtime,
            &crate::provider::ProviderModelCatalogEntry {
                model: current_model.clone(),
                display_name: None,
                description: None,
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        );

        assert_eq!(
            entries.first().map(|entry| entry.label.as_str()),
            Some("default")
        );
        assert_eq!(selected_label, "high");
        let high_entry = entries
            .iter()
            .find(|entry| entry.label == "high")
            .expect("high entry");
        assert_eq!(high_entry.status_tag.as_deref(), Some("current"));
        assert!(matches!(
            high_entry.action,
            CommandAction::ApplyModelSelection {
                ref model,
                reasoning_effort: Some(ReasoningEffort::High)
            } if model == &current_model
        ));
    }

    #[test]
    fn reasoning_palette_default_row_surfaces_known_model_default_effort() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-reasoning-default-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);
        runtime.config.provider.model = "gpt-5.4".to_owned();

        let (entries, selected_label) = super::build_reasoning_palette_entries(
            &runtime,
            &crate::provider::ProviderModelCatalogEntry {
                model: "gpt-5.4".to_owned(),
                display_name: Some("GPT-5.4".to_owned()),
                description: Some("Strong model for everyday coding.".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Xhigh),
                supported_reasoning_efforts: vec![
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::Xhigh,
                ],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        );

        assert_eq!(selected_label, "default");
        let default_entry = entries.first().expect("default entry");
        assert_eq!(default_entry.label, "default");
        assert!(default_entry.description.contains("xhigh"));
    }

    #[test]
    fn reasoning_palette_default_row_prefers_catalog_default_effort_over_fallback() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-reasoning-catalog-default-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let runtime = test_runtime_with_path(config_path);

        let (entries, selected_label) = super::build_reasoning_palette_entries(
            &runtime,
            &crate::provider::ProviderModelCatalogEntry {
                model: "custom-model".to_owned(),
                display_name: Some("Custom Model".to_owned()),
                description: Some("Custom provider test model".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::High),
                supported_reasoning_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        );

        assert_eq!(selected_label, "default");
        let default_entry = entries.first().expect("default entry");
        assert!(default_entry.description.contains("high"));
    }

    #[test]
    fn reasoning_palette_uses_catalog_reasoning_option_descriptions_when_present() {
        let runtime = test_runtime_with_path(PathBuf::from(
            "/tmp/loong-reasoning-option-description.toml",
        ));

        let (entries, _) = super::build_reasoning_palette_entries(
            &runtime,
            &crate::provider::ProviderModelCatalogEntry {
                model: "gpt-5.5".to_owned(),
                display_name: Some("GPT-5.5".to_owned()),
                description: Some("Frontier model".to_owned()),
                is_default: true,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Medium),
                supported_reasoning_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
                supported_reasoning_effort_descriptions: vec![
                    (
                        ReasoningEffort::Low,
                        "Fast responses with lighter reasoning".to_owned(),
                    ),
                    (
                        ReasoningEffort::High,
                        "Greater reasoning depth for complex problems".to_owned(),
                    ),
                ],
            },
        );

        let low = entries
            .iter()
            .find(|entry| entry.label == "low")
            .expect("low entry");
        assert_eq!(low.description, "Fast responses with lighter reasoning");
        let high = entries
            .iter()
            .find(|entry| entry.label == "high")
            .expect("high entry");
        assert_eq!(
            high.description,
            "Greater reasoning depth for complex problems"
        );
    }

    #[test]
    fn apply_model_selection_updates_runtime_and_footer_model() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-model-apply-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);
        let mut app = blank_app();

        super::apply_model_selection(
            &mut app,
            &mut runtime,
            "gpt-5.4".to_owned(),
            Some(ReasoningEffort::Xhigh),
        )
        .expect("apply model selection");

        assert_eq!(runtime.config.provider.model, "gpt-5.4");
        assert_eq!(
            runtime.config.provider.reasoning_effort,
            Some(ReasoningEffort::Xhigh)
        );
        assert_eq!(app.model, "gpt-5.4");
        assert_eq!(app.focus, Focus::Composer);
    }

    #[test]
    fn model_command_opens_selector_surface_instead_of_static_card() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-model-command-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);
        runtime
            .config
            .provider
            .preferred_models
            .push("gpt-5.4".to_owned());
        runtime.config.provider.models_endpoint = Some("http://127.0.0.1:9/models".to_owned());
        runtime.config.provider.models_endpoint_explicit = true;
        let mut app = blank_app();
        let backend = TestBackend::new(72, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");

        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(super::run_surface_command(
                &mut terminal,
                &mut app,
                &mut runtime,
                &CliChatOptions::default(),
                "/model",
            ))
            .expect("run model command");

        assert_eq!(app.focus, Focus::CommandPalette);
        match app
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )) {
            Some(CommandAction::OpenModelReasoning(entry))
                if entry.model == runtime.config.provider.model => {}
            other => panic!("expected /model to open model selector flow, got {other:?}"),
        }
    }

    #[test]
    fn exact_model_catalog_match_finds_model_and_display_name() {
        let catalog = vec![
            crate::provider::ProviderModelCatalogEntry {
                model: "gpt-5.4".to_owned(),
                display_name: Some("GPT-5.4".to_owned()),
                description: Some("Strong model for everyday coding.".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Xhigh),
                supported_reasoning_efforts: vec![
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::Xhigh,
                ],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
            crate::provider::ProviderModelCatalogEntry {
                model: "command-r".to_owned(),
                display_name: Some("Command R".to_owned()),
                description: Some("Cohere model".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::High),
                supported_reasoning_efforts: vec![ReasoningEffort::High],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
            crate::provider::ProviderModelCatalogEntry {
                model: "hidden-model".to_owned(),
                display_name: Some("Hidden Model".to_owned()),
                description: Some("Not shown by default".to_owned()),
                is_default: false,
                hidden: true,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Low),
                supported_reasoning_efforts: vec![ReasoningEffort::Low],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        ];

        assert_eq!(
            super::find_exact_model_catalog_entry(catalog.as_slice(), "gpt-5.4")
                .map(|entry| entry.model.as_str()),
            Some("gpt-5.4")
        );
        assert_eq!(
            super::find_exact_model_catalog_entry(catalog.as_slice(), "Command R")
                .map(|entry| entry.model.as_str()),
            Some("command-r")
        );
        assert_eq!(
            super::find_exact_model_catalog_entry(catalog.as_slice(), "hidden-model")
                .map(|entry| entry.model.as_str()),
            Some("hidden-model")
        );
    }

    #[test]
    fn model_palette_entries_use_direct_apply_for_single_reasoning_option() {
        let provider = ProviderConfig {
            kind: ProviderKind::Cohere,
            model: "command-r".to_owned(),
            ..ProviderConfig::fresh_for_kind(ProviderKind::Cohere)
        };
        let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-single-effort.toml"));
        let runtime = crate::chat::CliTurnRuntime {
            config: LoongConfig {
                provider,
                ..runtime.config
            },
            ..runtime
        };

        let entries = super::build_model_palette_entries(
            &runtime,
            &[crate::provider::ProviderModelCatalogEntry {
                model: "command-r".to_owned(),
                display_name: Some("Command R".to_owned()),
                description: Some("Cohere model".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::High),
                supported_reasoning_efforts: vec![ReasoningEffort::High],
                supported_reasoning_effort_descriptions: Vec::new(),
            }],
        );

        let entry = entries.first().expect("single model entry");
        assert!(matches!(
            entry.action,
            CommandAction::ApplyModelSelection {
                ref model,
                reasoning_effort: Some(ReasoningEffort::High)
            } if model == "command-r"
        ));
    }

    #[test]
    fn model_palette_prefers_display_name_label_and_keeps_raw_id_in_description() {
        let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-display-name.toml"));

        let entries = super::build_model_palette_entries(
            &runtime,
            &[crate::provider::ProviderModelCatalogEntry {
                model: "gpt-5.4".to_owned(),
                display_name: Some("GPT-5.4 Frontier".to_owned()),
                description: Some("Strong model for everyday coding.".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Xhigh),
                supported_reasoning_efforts: vec![
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::Xhigh,
                ],
                supported_reasoning_effort_descriptions: Vec::new(),
            }],
        );

        let entry = entries.first().expect("display-name entry");
        assert_eq!(entry.label, "GPT-5.4 Frontier");
        assert!(entry.description.contains("gpt-5.4"));
        assert!(
            entry
                .description
                .contains("Strong model for everyday coding.")
        );
    }

    #[test]
    fn model_palette_sorts_current_before_other_entries() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            model: "current-model".to_owned(),
            ..ProviderConfig::fresh_for_kind(ProviderKind::Openai)
        };
        let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-sort.toml"));
        let runtime = crate::chat::CliTurnRuntime {
            config: LoongConfig {
                provider,
                ..runtime.config
            },
            ..runtime
        };

        let entries = super::build_model_palette_entries(
            &runtime,
            &[
                crate::provider::ProviderModelCatalogEntry {
                    model: "zeta-model".to_owned(),
                    display_name: Some("Zeta Model".to_owned()),
                    description: None,
                    is_default: false,
                    hidden: false,
                    deprecated: false,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                    supported_reasoning_effort_descriptions: Vec::new(),
                },
                crate::provider::ProviderModelCatalogEntry {
                    model: "alpha-model".to_owned(),
                    display_name: Some("Alpha Model".to_owned()),
                    description: None,
                    is_default: false,
                    hidden: false,
                    deprecated: false,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                    supported_reasoning_effort_descriptions: Vec::new(),
                },
                crate::provider::ProviderModelCatalogEntry {
                    model: "current-model".to_owned(),
                    display_name: Some("Current Model".to_owned()),
                    description: None,
                    is_default: false,
                    hidden: false,
                    deprecated: false,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                    supported_reasoning_effort_descriptions: Vec::new(),
                },
            ],
        );

        assert_eq!(entries[0].status_tag.as_deref(), Some("current"));
        assert_eq!(entries[0].label, "Current Model");
        assert_eq!(entries[1].label, "Alpha Model");
        assert_eq!(entries[2].label, "Zeta Model");
    }

    #[test]
    fn merged_model_catalog_entries_hide_remote_hidden_and_deprecated_models_by_default() {
        let provider = ProviderConfig::fresh_for_kind(ProviderKind::Openai);

        let merged = super::merged_model_catalog_entries(
            &provider,
            &[
                crate::provider::ProviderModelCatalogEntry {
                    model: "hidden-remote".to_owned(),
                    display_name: Some("Hidden Remote".to_owned()),
                    description: Some("hidden".to_owned()),
                    is_default: false,
                    hidden: true,
                    deprecated: false,
                    default_reasoning_effort: Some(ReasoningEffort::Medium),
                    supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                    supported_reasoning_effort_descriptions: Vec::new(),
                },
                crate::provider::ProviderModelCatalogEntry {
                    model: "deprecated-remote".to_owned(),
                    display_name: Some("Deprecated Remote".to_owned()),
                    description: Some("deprecated".to_owned()),
                    is_default: false,
                    hidden: false,
                    deprecated: true,
                    default_reasoning_effort: Some(ReasoningEffort::Low),
                    supported_reasoning_efforts: vec![ReasoningEffort::Low],
                    supported_reasoning_effort_descriptions: Vec::new(),
                },
            ],
            false,
        );

        assert!(!merged.iter().any(|entry| entry.model == "hidden-remote"));
        assert!(
            !merged
                .iter()
                .any(|entry| entry.model == "deprecated-remote")
        );
    }

    #[test]
    fn merged_model_catalog_entries_keep_current_local_candidate_even_if_hidden() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            model: "hidden-current".to_owned(),
            ..ProviderConfig::fresh_for_kind(ProviderKind::Openai)
        };

        let merged = super::merged_model_catalog_entries(
            &provider,
            &[crate::provider::ProviderModelCatalogEntry {
                model: "hidden-current".to_owned(),
                display_name: Some("Hidden Current".to_owned()),
                description: Some("still current".to_owned()),
                is_default: false,
                hidden: true,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Medium),
                supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                supported_reasoning_effort_descriptions: Vec::new(),
            }],
            false,
        );

        let current = merged
            .iter()
            .find(|entry| entry.model == "hidden-current")
            .expect("current hidden entry");
        assert!(current.hidden);
    }

    #[test]
    fn mouse_clicking_skill_palette_inserts_into_composer() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.command_palette.show_skills("$demo");
        app.focus = Focus::CommandPalette;

        terminal.draw(|f| app.render(f)).expect("draw");
        let palette_row = app.last_palette_area.y;
        let palette_col = app.last_palette_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            palette_col,
            palette_row,
        ));

        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.take_input(), "$demo-skill ");
    }

    #[test]
    fn mouse_clicking_composer_restores_focus() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.focus = Focus::MessageList;

        terminal.draw(|f| app.render(f)).expect("draw");
        let composer_row = app.last_composer_area.y;
        let composer_col = app.last_composer_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            composer_col,
            composer_row,
        ));

        assert_eq!(app.focus, Focus::Composer);
    }

    #[test]
    fn transcript_click_closes_inline_skill_popup_after_focus_change() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$dem".to_owned());
        app.sync_inline_skill_popup();
        app.message_list.add_assistant_message("line-0".to_owned());
        app.message_list.add_assistant_message("line-1".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let transcript_row = app.last_transcript_area.y.saturating_add(1);
        let transcript_col = app.last_transcript_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            transcript_col,
            transcript_row,
        ));

        assert_eq!(app.focus, Focus::MessageList);
        assert!(!app.inline_skill_popup_active);
    }

    #[test]
    fn composer_click_reopens_inline_skill_popup_after_transcript_focus() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
        app.composer.set_input("$dem".to_owned());
        app.focus = Focus::MessageList;
        app.message_list.add_assistant_message("line-0".to_owned());
        app.sync_inline_skill_popup();

        terminal.draw(|f| app.render(f)).expect("draw");
        let composer_row = app.last_composer_area.y;
        let composer_col = app.last_composer_area.x.saturating_add(1);
        app.handle_mouse_event(mouse(
            MouseEventKind::Down(MouseButton::Left),
            composer_col,
            composer_row,
        ));

        assert_eq!(app.focus, Focus::Composer);
        assert!(app.inline_skill_popup_active);
    }

    #[test]
    fn startup_tip_leaves_blank_row_before_composer_separator() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_startup_header_with_tips(
            "0.1.0".to_owned(),
            "fallback".to_owned(),
            vec![
                ("Skills".to_owned(), vec!["0".to_owned()]),
                ("MCP".to_owned(), vec!["1".to_owned()]),
            ],
            vec!["rotating tip".to_owned()],
        );

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let composer_separator_row = app.last_composer_area.y.saturating_sub(1) as usize;
        let blank_row_before_separator = composer_separator_row.saturating_sub(1);

        assert!(lines.iter().any(|line| line.contains("rotating tip")));
        assert!(
            lines
                .get(blank_row_before_separator)
                .is_some_and(|line| line.trim().is_empty())
        );
    }

    #[test]
    fn startup_header_remains_visible_after_first_message() {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_startup_header(
            "0.1.0".to_owned(),
            "tutorial".to_owned(),
            vec![("MCP".to_owned(), vec!["0".to_owned()])],
        );
        app.message_list.add_user_message("hi".to_owned());
        app.message_list.add_assistant_message("hello".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("0.1.0"));
        assert!(lines.contains("MCP (0)"));
        assert!(lines.contains("hi"));
        assert!(lines.contains("hello"));
    }

    #[test]
    fn pending_band_keeps_blank_padding_rows() {
        let backend = TestBackend::new(50, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let spinner_row = lines
            .iter()
            .position(|line| line.contains("..."))
            .expect("spinner row");
        assert!(spinner_row > 0);
        assert!(lines[spinner_row - 1].trim().is_empty());
    }

    #[test]
    fn compact_pending_lines_drops_padding_before_content_on_tiny_height() {
        let lines = super::build_pending_lines(
            Some(std::time::Instant::now()),
            &["visible reply".to_owned()],
            1,
            &std::collections::VecDeque::new(),
            &std::collections::VecDeque::new(),
            40,
        );

        let compacted = super::compact_pending_lines_for_height(lines, 3);
        let rendered = compacted
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 3);
        assert!(rendered.iter().any(|line| line.contains("visible reply")));
    }

    #[test]
    fn pending_band_hides_plain_streaming_preview_text() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview =
                Some("first streamed sentence\nsecond streamed sentence".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let first_row = lines
            .iter()
            .position(|line| line.contains("first streamed sentence"))
            .expect("first preview row");
        let second_row = lines
            .iter()
            .position(|line| line.contains("second streamed sentence"))
            .expect("second preview row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        assert!(first_row < composer_row);
        assert!(second_row < composer_row);
        assert!(!lines.iter().any(|line| line.contains("╭─")));
        assert!(!lines.iter().any(|line| line.contains("turn pipeline")));
    }

    #[test]
    fn pending_preview_styles_tool_activity_without_flattening_it_into_plain_text() {
        let lines = super::build_pending_lines(
            Some(std::time::Instant::now()),
            &[
                "• Called read_file · working".to_owned(),
                "  ↳ stderr 1 lines · 42 bytes".to_owned(),
                "    - denied".to_owned(),
            ],
            1,
            &std::collections::VecDeque::new(),
            &std::collections::VecDeque::new(),
            72,
        );

        let called_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.as_ref() == "Called ")
            })
            .expect("called line");
        let called_label = called_line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "Called ")
            .expect("called label");
        assert!(
            super::PENDING_TOOL_LABEL_COLORS.contains(
                &called_label
                    .style
                    .fg
                    .expect("called label should have an animated foreground"),
            )
        );
        assert!(
            called_label
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );

        let stderr_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.as_ref() == "stderr ")
            })
            .expect("stderr line");
        let stderr_label = stderr_line
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "stderr ")
            .expect("stderr label");
        assert_eq!(stderr_label.style.fg, Some(super::SURFACE_RED));

        let sample_line = lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.as_ref().contains("- denied"))
            })
            .expect("sample line");
        let sample_span = sample_line
            .spans
            .iter()
            .find(|span| span.content.as_ref().contains("- denied"))
            .expect("sample span");
        assert_eq!(sample_span.style.fg, Some(super::SURFACE_RED));
    }

    #[test]
    fn pending_live_generic_line_preserves_plain_label_like_text() {
        let rendered = super::render_pending_live_line(
            "source: imported config at ~/.loong/config.toml",
            24,
            Style::default(),
            std::time::Instant::now(),
        )
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line == "  source: imported config")
        );
        assert!(
            rendered
                .iter()
                .any(|line| line == "  at ~/.loong/config.toml")
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line == "    at ~/.loong/config.toml")
        );
    }

    #[test]
    fn pending_tool_activity_preserves_literal_plus_prefix() {
        let rendered = super::build_pending_lines(
            Some(std::time::Instant::now()),
            &[
                "• Called + added ~/.loong/config.toml".to_owned(),
                "  ↳ stderr + added ~/.loong/config.toml".to_owned(),
                "    + added ~/.loong/config.toml".to_owned(),
            ],
            1,
            &std::collections::VecDeque::new(),
            &std::collections::VecDeque::new(),
            48,
        )
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Called + added"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ stderr + added"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("+ added ~/.loong/config.toml"))
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("• Called - added"))
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("↳ stderr - added"))
        );
    }

    #[test]
    fn pending_queue_preview_preserves_literal_plus_prefix() {
        let mut pending_queue = std::collections::VecDeque::new();
        pending_queue.push_back("+ added ~/.loong/config.toml".to_owned());

        let rendered = super::build_pending_lines(
            Some(std::time::Instant::now()),
            &[],
            1,
            &std::collections::VecDeque::new(),
            &pending_queue,
            42,
        )
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ + added")));
        assert!(!rendered.iter().any(|line| line.contains("↳ - added")));
    }

    #[test]
    fn pending_preview_hides_plain_streaming_reply_between_transcript_and_composer() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("streamed reply line".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let user_row = lines
            .iter()
            .position(|line| line.contains("hi"))
            .expect("user row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");

        assert!(composer_row > user_row);
        let preview_row = lines
            .iter()
            .position(|line| line.contains("streamed reply line"))
            .expect("preview row");
        assert!(preview_row > user_row);
        assert!(preview_row < composer_row);
    }

    #[test]
    fn pending_preview_hides_reasoning_and_visible_reply_text() {
        let backend = TestBackend::new(70, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("quiet reasoning\nvisible reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("quiet reasoning"));
        assert!(lines.contains("visible reply"));
    }

    #[test]
    fn pending_preview_keeps_plain_reply_with_tool_like_prefix_out_of_pending_band() {
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("• not a tool call\nrequest: still plain prose".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        let bullet_row = lines
            .iter()
            .position(|line| line.contains("• not a tool call"))
            .expect("bullet reply row");
        let request_row = lines
            .iter()
            .position(|line| line.contains("request: still plain prose"))
            .expect("request reply row");

        assert!(bullet_row < composer_row);
        assert!(request_row < composer_row);
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("Called not a tool call")),
            "plain transcript preview should not be restyled as pending tool activity"
        );
    }

    #[test]
    fn pending_preview_no_longer_reserves_blank_row_for_plain_reply_preview() {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("visible reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let spinner_row = lines
            .iter()
            .position(|line| line.contains("..."))
            .expect("spinner row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        let preview_row = lines
            .iter()
            .position(|line| line.contains("visible reply"))
            .expect("preview row");

        assert!(composer_row > spinner_row);
        assert!(preview_row < composer_row);
    }

    #[test]
    fn pending_preview_does_not_render_plain_reply_lines() {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("visible reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("visible reply"));
    }
    #[test]
    fn pending_preview_does_not_wrap_hidden_plain_reply_lines() {
        let backend = TestBackend::new(28, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("visible reply wraps across the pending band".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("visible reply"));
        assert!(lines.contains("pending band"));
    }

    #[test]
    fn pending_preview_no_longer_expands_plain_reply_text_with_extra_height() {
        let backend = TestBackend::new(18, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some(
                "a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11 a12 a13 a14 a15 a16 a17 a18 a19 a20 omega"
                    .to_owned(),
            );
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let rendered = buffer_lines(&terminal).join("\n");

        assert!(rendered.contains("omega"));
    }

    #[test]
    fn pending_preview_hides_reasoning_reply_separator_when_plain_text_is_hidden() {
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("quiet reasoning\n\nvisible reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("quiet reasoning"));
        assert!(lines.contains("visible reply"));
    }

    #[test]
    fn pending_preview_does_not_style_hidden_reasoning_lines() {
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("quiet reasoning\n\nvisible reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");
        assert!(lines.contains("quiet reasoning"));
        assert!(lines.contains("visible reply"));
    }

    #[test]
    fn pending_preview_truncation_ignores_hidden_plain_reply_segments() {
        let backend = TestBackend::new(70, 12);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some(
                "reason-1\nreason-2\nreason-3\nreason-4\n\nreply-1\nreply-2\nreply-3".to_owned(),
            );
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let rendered = buffer_lines(&terminal).join("\n");

        assert!(rendered.contains("reason-1"));
        assert!(rendered.contains("reason-2"));
        assert!(rendered.contains("reply-1"));
        assert!(rendered.contains("reply-2"));
    }

    #[test]
    fn pending_live_lines_trim_outer_blank_lines_and_collapse_repeats() {
        let lines = Arc::new(StdMutex::new(LiveTranscriptState {
            tool_activity_lines: vec![
                String::new(),
                String::new(),
                "reasoning".to_owned(),
                String::new(),
                String::new(),
                "reply".to_owned(),
                String::new(),
                String::new(),
            ],
            draft_preview: None,
        }));

        let normalized = super::pending_live_lines(&lines, 6);
        assert_eq!(
            normalized,
            vec!["reasoning".to_owned(), String::new(), "reply".to_owned(),]
        );
    }

    #[test]
    fn pending_live_lines_expand_with_larger_preview_budget() {
        let lines = Arc::new(StdMutex::new(LiveTranscriptState {
            tool_activity_lines: vec![
                "reason-1".to_owned(),
                "reason-2".to_owned(),
                "reason-3".to_owned(),
                String::new(),
                "reply-1".to_owned(),
                "reply-2".to_owned(),
                "reply-3".to_owned(),
                "reply-4".to_owned(),
            ],
            draft_preview: None,
        }));

        let compact = super::pending_live_lines(&lines, 4);
        let expanded = super::pending_live_lines(&lines, 7);

        assert!(compact.len() < expanded.len());
        assert!(expanded.iter().any(|line| line.contains("reply-3")));
    }

    #[test]
    fn pending_signature_preview_budget_tracks_last_render_geometry() {
        let mut app = blank_app();
        app.last_render_width = 40;
        app.last_render_height = 20;

        assert!(super::pending_signature_preview_budget(&app) > 1);

        app.last_render_height = 8;
        assert_eq!(super::pending_signature_preview_budget(&app), 1);
    }

    #[test]
    fn transcript_navigation_key_helper_keeps_printable_keys_for_composer() {
        assert!(super::is_transcript_navigation_key(
            crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
        ));
        assert!(super::is_transcript_navigation_key(
            crossterm::event::KeyEvent::new(KeyCode::Home, KeyModifiers::NONE,)
        ));
        assert!(!super::is_transcript_navigation_key(
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)
        ));
        assert!(!super::is_transcript_navigation_key(
            crossterm::event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE,)
        ));
    }

    #[test]
    fn transcript_focus_text_keys_enter_composer_immediately() {
        let mut app = blank_app();
        app.focus = Focus::MessageList;

        let submitted = super::route_transcript_key_to_composer(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE),
        );

        assert!(submitted.is_none());
        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.text(), "你");
    }

    #[test]
    fn paste_event_always_restores_composer_focus_and_inserts_text() {
        let mut app = blank_app();
        app.focus = Focus::MessageList;

        super::paste_into_composer(&mut app, "alpha\r\nbeta");

        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.text(), "alpha\nbeta");
        assert!(!app.composer_follow_up_intent);
    }

    #[test]
    fn paste_event_marks_pending_draft_as_follow_up() {
        let mut app = blank_app();
        app.focus = Focus::CommandPalette;
        app.pending_turn = true;

        super::paste_into_composer(&mut app, "queued follow-up");

        assert_eq!(app.focus, Focus::Composer);
        assert_eq!(app.composer.text(), "queued follow-up");
        assert!(app.composer_follow_up_intent);
    }

    #[test]
    fn transcript_focus_enter_submits_existing_draft() {
        let mut app = blank_app();
        app.focus = Focus::MessageList;
        app.composer.set_input("send me".to_owned());

        let submitted = super::route_transcript_key_to_composer(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(submitted.as_deref(), Some("send me"));
        assert_eq!(app.focus, Focus::Composer);
        assert!(app.composer.is_empty());
    }

    #[test]
    fn transcript_focus_capture_helper_rejects_navigation_and_modified_keys() {
        assert!(super::should_focus_composer_for_transcript_key(
            crossterm::event::KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE,)
        ));
        assert!(super::should_focus_composer_for_transcript_key(
            crossterm::event::KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE,)
        ));
        assert!(super::should_focus_composer_for_transcript_key(
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)
        ));
        assert!(!super::should_focus_composer_for_transcript_key(
            crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
        ));
        assert!(!super::should_focus_composer_for_transcript_key(
            crossterm::event::KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)
        ));
    }

    #[test]
    fn composer_routes_arrow_and_page_scroll_even_with_a_draft() {
        let mut app = blank_app();
        app.composer.set_input("draft".to_owned());

        assert!(super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE,)
        ));
        assert!(super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE,)
        ));
        assert!(super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE,)
        ));
        assert!(super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
        ));
        assert!(!super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE,)
        ));
        assert!(!super::should_route_composer_key_to_transcript(
            &app,
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)
        ));
    }

    #[test]
    fn submitted_message_is_not_treated_as_follow_up_after_pending_turn_finishes() {
        let mut app = blank_app();
        app.composer_follow_up_intent = true;

        assert!(!super::submitted_message_is_follow_up(&app, "follow up"));

        app.pending_turn = true;
        assert!(super::submitted_message_is_follow_up(&app, "follow up"));
        assert!(!super::submitted_message_is_follow_up(&app, "/status"));
        assert!(!super::submitted_message_is_follow_up(&app, ":status"));
    }

    #[test]
    fn pending_footer_yields_to_queue_hint_when_draft_exists() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.composer.set_input("queued draft".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");

        assert!(lines.contains("Tab to queue message"));
        assert!(!lines.contains("/tmp/example"));
    }

    #[test]
    fn pending_footer_shows_restore_hint_when_queue_exists() {
        let backend = TestBackend::new(60, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_queue.push_back("queued draft".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal).join("\n");

        assert!(lines.contains("queued ×1"));
        assert!(lines.contains("Option + Up") || lines.contains("Alt + Up"));
        assert!(!lines.contains("/tmp/example"));
    }

    #[test]
    fn width_resize_keeps_provider_error_and_footer_visible() {
        let backend = TestBackend::new(72, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}".to_owned(),
        );

        terminal.draw(|f| app.render(f)).expect("draw");
        terminal.backend_mut().resize(28, 18);
        terminal.draw(|f| app.render(f)).expect("draw");

        let lines = buffer_lines(&terminal);
        let provider_row = lines
            .iter()
            .position(|line| line.contains("provider error"))
            .expect("provider error row");
        let detail_row = lines
            .iter()
            .position(|line| line.contains("INVALID_API_KEY"))
            .expect("provider error detail row");
        let footer_row = lines
            .iter()
            .position(|line| line.contains("gpt-test"))
            .expect("footer row");

        assert!(provider_row < detail_row);
        assert!(detail_row < footer_row);
        assert!(footer_row > detail_row);
        assert!(lines.iter().any(|line| line.contains("401")));
    }

    #[test]
    fn width_resize_does_not_surface_internal_tool_result_or_transport_tail_in_plain_reply() {
        let backend = TestBackend::new(72, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_assistant_message(
            concat!(
                "我明白你的意思。\n\n",
                "我已经核到一件关键事实：当前配置里确实存在一个更宽的 file_root。\n\n",
                "[ok] {\"status\":\"ok\",\"tool\":\"read\",\"tool_call_id\":\"call-1\",\"payload_summary\":\"{\\\"path\\\":\\\"/workspace/demo/crates/daemon/src/lib.rs\\\",\\\"line_start\\\":1,\\\"line_end\\\":50}\",\"payload_chars\":2121,\"payload_truncated\":true}\n",
                "candidate_index=1 candidate_count=1 profile_index=1 profile_count=1 exhausted=true error=provider request failed for model `gpt-5.4` on attempt 3/3: error sending request for url (https://api.tonsof.blue/v1/chat/completions)"
            )
            .to_owned(),
        );

        terminal.draw(|f| app.render(f)).expect("draw");
        terminal.backend_mut().resize(28, 18);
        terminal.draw(|f| app.render(f)).expect("draw");

        let lines = buffer_lines(&terminal).join("\n");
        assert!(
            !lines.trim().is_empty(),
            "sanitized plain reply should still leave visible assistant content after resize: {lines}"
        );
        assert!(!lines.contains("[ok] {\"status\":\"ok\""));
        assert!(!lines.contains("provider request failed for model"));
        assert!(!lines.contains("candidate_index=1"));
    }

    #[test]
    fn width_resize_keeps_pending_restore_footer_and_previews_visible() {
        let backend = TestBackend::new(72, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_steers
            .push_back("nudge the current answer toward the root cause".to_owned());
        app.pending_queue
            .push_back("after that, summarize the diff and keep the footer visible".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        terminal.backend_mut().resize(34, 18);
        terminal.draw(|f| app.render(f)).expect("draw");

        let lines = buffer_lines(&terminal);
        let steer_row = lines
            .iter()
            .position(|line| line.contains("root cause"))
            .expect("steer preview row");
        let queue_header_row = lines
            .iter()
            .position(|line| line.contains("Queued follow-up messages"))
            .expect("queued header row");
        let queued_row = lines
            .iter()
            .enumerate()
            .skip(queue_header_row + 1)
            .find_map(|(idx, line)| line.contains("↳").then_some(idx))
            .expect("queued preview row");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");
        let footer_row = lines
            .iter()
            .position(|line| line.contains("Option + Up") || line.contains("Alt + Up"))
            .expect("restore footer row");

        assert!(steer_row < queue_header_row);
        assert!(queue_header_row < queued_row);
        assert!(queued_row < composer_row);
        assert!(composer_row < footer_row);
        assert!(lines[queued_row].contains("↳"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Option + Up") || line.contains("Alt + Up"))
        );
    }

    #[test]
    fn off_tail_pending_resize_and_end_restore_tail_without_losing_state() {
        let backend = TestBackend::new(48, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        for idx in 0..18 {
            app.message_list.add_assistant_message(format!(
                "line-{idx} keeps transcript stable while pending preview and resize interact"
            ));
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        app.message_list.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        ));
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("streamed preview line".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw off tail");
        let off_tail_lines = buffer_lines(&terminal).join("\n");
        assert!(off_tail_lines.contains("PgDn / End"));

        app.message_list
            .add_assistant_message("new-tail-line after scroll".to_owned());
        terminal.backend_mut().resize(34, 18);
        terminal.draw(|f| app.render(f)).expect("draw resized");
        let resized_lines = buffer_lines(&terminal).join("\n");
        assert!(resized_lines.contains("PgDn / End"));

        app.message_list.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        ));
        terminal.draw(|f| app.render(f)).expect("draw restored");
        let restored_lines = buffer_lines(&terminal).join("\n");

        assert!(restored_lines.contains("new-tail-line after scroll"));
        assert!(!restored_lines.contains("PgDn / End"));
        assert_eq!(app.message_list.scroll_offset_for_test(), 0);
    }

    #[test]
    fn pending_preview_shows_queued_steer_and_follow_up_above_composer() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_steers
            .push_back("nudge the current answer toward the root cause".to_owned());
        app.pending_queue
            .push_back("after that, summarize the diff".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);
        let steer_header_row = lines
            .iter()
            .position(|line| line.contains("Messages to be submitted after next tool call"))
            .expect("steer header");
        let steer_row = lines
            .iter()
            .position(|line| line.contains("nudge the current answer"))
            .expect("steer preview");
        let queue_header_row = lines
            .iter()
            .position(|line| line.contains("Queued follow-up messages"))
            .expect("queue header");
        let queued_row = lines
            .iter()
            .position(|line| line.contains("after that, summarize"))
            .expect("queued preview");
        let composer_row = lines
            .iter()
            .position(|line| line.contains("›"))
            .expect("composer row");

        assert!(steer_header_row < steer_row);
        assert!(lines[steer_row].contains("↳"));
        assert!(queue_header_row < queued_row);
        assert!(lines[queued_row].contains("↳"));
        assert!(steer_row < queued_row);
        assert!(queued_row < composer_row);
    }

    #[test]
    fn pending_preview_collapses_extra_messages_into_overflow_count() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_steers.push_back("first steer".to_owned());
        app.pending_steers.push_back("second steer".to_owned());
        app.pending_steers.push_back("third steer".to_owned());
        app.pending_steers.push_back("fourth steer".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);

        assert!(lines.iter().any(|line| line.contains("first steer")));
        assert!(lines.iter().any(|line| line.contains("third steer")));
        assert!(!lines.iter().any(|line| line.contains("fourth steer")));
        assert!(lines.iter().any(|line| line.contains("… +1 more")));
    }

    #[test]
    fn pending_preview_caps_total_items_across_steer_and_follow_up_queues() {
        let backend = TestBackend::new(72, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_steers.push_back("first steer".to_owned());
        app.pending_steers.push_back("second steer".to_owned());
        app.pending_queue.push_back("first follow-up".to_owned());
        app.pending_queue.push_back("second follow-up".to_owned());

        terminal.draw(|f| app.render(f)).expect("draw");
        let lines = buffer_lines(&terminal);

        assert!(lines.iter().any(|line| line.contains("first steer")));
        assert!(lines.iter().any(|line| line.contains("second steer")));
        assert!(lines.iter().any(|line| line.contains("first follow-up")));
        assert!(!lines.iter().any(|line| line.contains("second follow-up")));
        assert!(lines.iter().any(|line| line.contains("… +1 more")));
    }

    #[test]
    fn queue_pending_message_moves_draft_into_follow_up_queue() {
        let mut app = blank_app();
        app.composer.set_input("queued draft".to_owned());
        app.composer_follow_up_intent = true;

        super::queue_pending_message(&mut app);

        assert_eq!(app.pending_queue.len(), 1);
        assert_eq!(
            app.pending_queue.front().map(String::as_str),
            Some("queued draft")
        );
        assert!(app.composer.is_empty());
        assert!(!app.composer_follow_up_intent);
    }

    #[test]
    fn dequeue_pending_steer_prefers_follow_up_queue_before_steer_stack() {
        let mut app = blank_app();
        app.pending_steers.push_back("steer text".to_owned());
        app.pending_queue.push_back("queued follow-up".to_owned());

        assert!(super::dequeue_pending_steer(&mut app));
        assert_eq!(app.composer.take_input(), "queued follow-up");
        assert_eq!(app.pending_steers.len(), 1);
    }

    #[test]
    fn pending_signature_ignores_hidden_tail_lines() {
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.tool_activity_lines = vec![
                "reason-1".to_owned(),
                "reason-2".to_owned(),
                "reason-3".to_owned(),
                String::new(),
                "reply-1".to_owned(),
                "reply-2".to_owned(),
                "hidden-tail".to_owned(),
            ];
        }
        let before = super::pending_render_signature(&app);
        if let Ok(mut live) = app.live_transcript.lock() {
            live.tool_activity_lines = vec![
                "reason-1".to_owned(),
                "reason-2".to_owned(),
                "reason-3".to_owned(),
                String::new(),
                "reply-1".to_owned(),
                "reply-2".to_owned(),
                "different-hidden-tail".to_owned(),
            ];
        }
        let after = super::pending_render_signature(&app);

        assert_eq!(before, after);
    }

    #[test]
    fn pending_signature_changes_when_follow_up_preview_changes() {
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        app.pending_steers.push_back("first steer".to_owned());
        let before = super::pending_render_signature(&app);
        app.pending_steers.clear();
        app.pending_queue
            .push_back("first queued follow-up".to_owned());
        let after = super::pending_render_signature(&app);

        assert_ne!(before, after);
    }

    #[test]
    fn pending_signature_ignores_plain_reply_preview_changes() {
        let mut app = blank_app();
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.tool_activity_lines =
                vec!["reason-1".to_owned(), String::new(), "reply-1".to_owned()];
        }
        let before = super::pending_render_signature(&app);
        if let Ok(mut live) = app.live_transcript.lock() {
            live.tool_activity_lines =
                vec!["reason-1".to_owned(), String::new(), "reply-2".to_owned()];
        }
        let after = super::pending_render_signature(&app);

        assert_eq!(before, after);
    }

    #[test]
    fn transcript_preview_signature_changes_when_plain_preview_changes() {
        let mut app = blank_app();
        app.pending_turn = true;
        app.last_render_width = 72;
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("first preview".to_owned());
        }
        let before = super::transcript_preview_signature(&app);
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("second preview".to_owned());
        }
        let after = super::transcript_preview_signature(&app);

        assert_ne!(before, after);
    }

    #[test]
    fn startup_overflow_still_keeps_user_block_top_padding_visible() {
        let backend = TestBackend::new(50, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_startup_header(
            "0.1.0".to_owned(),
            "tutorial".to_owned(),
            vec![
                (
                    "MCP".to_owned(),
                    vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
                ),
                (
                    "Skills".to_owned(),
                    vec![
                        "alpha".to_owned(),
                        "beta".to_owned(),
                        "gamma".to_owned(),
                        "delta".to_owned(),
                    ],
                ),
            ],
        );
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());

        terminal.draw(|f| app.render(f)).expect("draw");
        let user_row = find_row(&terminal, "hi").expect("user row");
        assert!(user_row > 0);
        assert!(
            row_has_background(&terminal, user_row - 1, SURFACE_USER_MSG_BG),
            "expected the row above the visible user text to be the user block top padding"
        );
    }

    #[test]
    fn pending_transcript_keeps_user_block_bottom_padding_visible() {
        let backend = TestBackend::new(50, 16);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_startup_header(
            "0.1.0".to_owned(),
            "tutorial".to_owned(),
            vec![
                (
                    "MCP".to_owned(),
                    vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
                ),
                (
                    "Skills".to_owned(),
                    vec![
                        "alpha".to_owned(),
                        "beta".to_owned(),
                        "gamma".to_owned(),
                        "delta".to_owned(),
                    ],
                ),
            ],
        );
        app.message_list.add_user_message("nihao".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());

        terminal.draw(|f| app.render(f)).expect("draw");
        let user_row = find_row(&terminal, "nihao").expect("user row");
        let pending_row = find_row(&terminal, "...")
            .or_else(|| find_row(&terminal, "中"))
            .unwrap_or(0);

        assert!(row_has_background(
            &terminal,
            user_row + 1,
            SURFACE_USER_MSG_BG
        ));
        assert!(pending_row > user_row);
    }

    #[test]
    fn startup_overflow_with_pending_preview_keeps_user_block_visible_with_transcript_preview() {
        let backend = TestBackend::new(50, 16);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list.add_startup_header(
            "0.1.0".to_owned(),
            "tutorial".to_owned(),
            vec![
                (
                    "MCP".to_owned(),
                    vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
                ),
                (
                    "Skills".to_owned(),
                    vec![
                        "alpha".to_owned(),
                        "beta".to_owned(),
                        "gamma".to_owned(),
                        "delta".to_owned(),
                    ],
                ),
            ],
        );
        app.message_list.add_user_message("hi".to_owned());
        app.pending_turn = true;
        app.turn_start = Some(std::time::Instant::now());
        if let Ok(mut live) = app.live_transcript.lock() {
            live.draft_preview = Some("pending reply".to_owned());
        }

        terminal.draw(|f| app.render(f)).expect("draw");
        let user_row = find_row(&terminal, "hi").expect("user row");
        let preview_row = find_row(&terminal, "pending reply").expect("preview row");
        let composer_row = find_row(&terminal, "›").expect("composer row");

        assert!(row_has_background(
            &terminal,
            user_row - 1,
            SURFACE_USER_MSG_BG
        ));
        assert!(preview_row > user_row);
        assert!(preview_row < composer_row);
        assert!(composer_row > user_row);
    }

    #[test]
    fn startup_onboarding_renders_between_startup_header_and_composer() {
        let backend = TestBackend::new(72, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut app = blank_app();
        app.message_list
            .add_startup_header("0.1.0".to_owned(), "tutorial".to_owned(), Vec::new());
        app.startup_onboarding = Some(onboarding_state());

        terminal.draw(|f| app.render(f)).expect("draw");

        let version_row = find_row(&terminal, "0.1.0").expect("version row");
        let onboarding_row =
            find_row(&terminal, "onboarding · 1/6 · language").expect("onboarding row");
        let composer_row = find_row(&terminal, "›").expect("composer row");

        assert!(version_row < onboarding_row);
        assert!(onboarding_row < composer_row);
    }

    #[test]
    fn startup_onboarding_language_confirmation_refreshes_header_copy() {
        let mut app = blank_app();
        app.detected_skills = vec![skill("demo-skill")];
        app.startup_mcp_count = 2;
        app.startup_version = "v0.1.0".to_owned();
        app.message_list.add_startup_header_with_tips(
            "v0.1.0".to_owned(),
            "ctrl+c exit".to_owned(),
            vec![
                ("Skills".to_owned(), vec!["1".to_owned()]),
                ("MCP".to_owned(), vec!["2".to_owned()]),
            ],
            vec!["type $skill".to_owned()],
        );
        let mut state = onboarding_state();
        state.language_index = 1;
        app.startup_onboarding = Some(state);

        let action = app
            .startup_onboarding
            .as_mut()
            .expect("onboarding state")
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            action,
            StartupOnboardingAction::ApplyLanguage(Language::ZhCn)
        );
        let mut runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-language-test.toml"));
        assert!(
            app.apply_startup_onboarding_action(action, &mut runtime)
                .expect("apply onboarding action")
        );

        let rendered = app
            .message_list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("技能 (1)"));
        assert!(rendered.contains("ctrl+c 退出"));
    }

    #[test]
    fn startup_onboarding_supports_all_shell_languages() {
        let mut env = ScopedEnv::new();
        env.set("LOONG_TUI_ONBOARD", "1");
        let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-language-list.toml"));
        let state =
            StartupOnboardingState::new(&runtime, Language::Ru).expect("startup onboarding state");

        assert_eq!(
            state.language_options,
            vec![
                Language::En,
                Language::ZhCn,
                Language::ZhTw,
                Language::Ja,
                Language::Ru,
            ]
        );
        assert_eq!(state.current_language(), Language::Ru);
    }

    #[test]
    fn startup_onboarding_provider_stage_lists_all_sorted_provider_kinds() {
        let mut env = ScopedEnv::new();
        env.set("LOONG_TUI_ONBOARD", "1");
        let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-provider-list.toml"));

        let options = super::build_startup_provider_options(&runtime, Language::En);
        let labels = options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>();
        let expected = ProviderKind::all_sorted()
            .iter()
            .map(|kind| kind.display_name())
            .collect::<Vec<_>>();

        assert_eq!(labels, expected);
        let current_index = options
            .iter()
            .position(|option| option.kind == runtime.config.provider.kind)
            .expect("current provider should be present");
        assert!(options[current_index].recommended);
        assert!(options[current_index].is_current);
    }

    #[test]
    fn startup_onboarding_escape_moves_back_without_dismissing() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::Provider;

        let action = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(action, StartupOnboardingAction::Handled);
        assert_eq!(state.stage, StartupOnboardingStage::Language);

        let action = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, StartupOnboardingAction::Handled);
        assert_eq!(state.stage, StartupOnboardingStage::Language);
    }

    #[test]
    fn apply_language_refreshes_localized_onboarding_runtime_copy() {
        let path = format!(
            "/tmp/loong-startup-language-refresh-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        );
        let mut env = ScopedEnv::new();
        env.set("LOONG_TUI_ONBOARD", "1");
        let mut runtime = test_runtime_with_path(PathBuf::from(path));
        runtime.config.feishu.enabled = true;

        let mut app = blank_app();
        app.startup_onboarding = StartupOnboardingState::new(&runtime, Language::En);
        let state = app
            .startup_onboarding
            .as_mut()
            .expect("startup onboarding state");
        state.language_index = state
            .language_options
            .iter()
            .position(|language| *language == Language::ZhCn)
            .expect("zh-CN option");

        app.apply_startup_onboarding_action(
            StartupOnboardingAction::ApplyLanguage(Language::ZhCn),
            &mut runtime,
        )
        .expect("apply language");

        let state = app
            .startup_onboarding
            .as_ref()
            .expect("refreshed onboarding state");
        assert_eq!(state.current_language(), Language::ZhCn);
        assert_eq!(state.enabled_channel_labels, vec!["飞书".to_owned()]);
        let current_provider = state
            .provider_options
            .iter()
            .find(|option| option.is_current)
            .expect("current provider option");
        assert!(
            current_provider
                .detail
                .contains("沿用 config.toml 里的当前")
                && current_provider.detail.contains("凭证"),
            "provider detail should be localized after language apply: {}",
            current_provider.detail
        );
    }

    #[test]
    fn persist_startup_provider_selection_updates_runtime_config_and_env_binding() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-startup-provider-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path.clone());
        let mut env = ScopedEnv::new();
        env.set("ANTHROPIC_API_KEY", "test-key");

        let summary = super::persist_startup_provider_selection(
            &mut runtime,
            StartupProviderOption {
                kind: ProviderKind::Anthropic,
                auth_env_name: Some("ANTHROPIC_API_KEY".to_owned()),
                is_current: false,
                label: "Anthropic".to_owned(),
                detail: "detail".to_owned(),
                recommended: false,
            },
            Language::En,
        )
        .expect("persist provider selection");

        assert!(summary.contains("Anthropic"));
        assert!(summary.contains("ANTHROPIC_API_KEY"));
        assert_eq!(runtime.config.provider.kind, ProviderKind::Anthropic);
        assert_eq!(runtime.config.active_provider_id(), Some("anthropic"));
        assert!(runtime.config.providers.contains_key("anthropic"));
        assert_eq!(
            runtime.config.provider.resolved_auth_env_name().as_deref(),
            Some("ANTHROPIC_API_KEY")
        );
        let loaded = crate::config::load(Some(config_path.to_string_lossy().as_ref()))
            .expect("reload config");
        assert_eq!(loaded.1.provider.kind, ProviderKind::Anthropic);
        assert_eq!(loaded.1.active_provider_id(), Some("anthropic"));
    }

    #[test]
    fn startup_onboarding_skills_stage_toggles_selection_with_space() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::Skills;
        state.feedback = None;

        let action = state.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

        assert_eq!(action, StartupOnboardingAction::Handled);
        assert!(state.selected_skill_ids.contains("agent-browser"));
        assert_eq!(state.feedback.as_deref(), Some("selected 1 skill pack(s)."));
    }

    #[test]
    fn startup_onboarding_setup_path_stage_surfaces_deeper_follow_up_details() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = 1;
        state.startup_mcp_count = 2;
        state.detected_skill_count = 5;
        state.provider_auth_env_name = Some("OPENAI_API_KEY".to_owned());
        state.provider_configuration_hint =
            Some("If you need to keep tuning provider base_url, model, or auth, `loong doctor` is the next check to run.".to_owned());

        let rendered = super::render_startup_onboarding_lines(&state, 90)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("provider + web setup"));
        assert!(rendered.contains("Provider auth env now: OPENAI_API_KEY."));
        assert!(rendered.contains("Web setup default: DuckDuckGo."));
        assert!(rendered.contains("loong doctor"));
        assert!(rendered.contains("loong onboard"));
    }

    #[test]
    fn startup_onboarding_only_expands_the_selected_provider_detail() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::Provider;
        state.provider_options = vec![
            StartupProviderOption {
                kind: ProviderKind::Openai,
                auth_env_name: None,
                is_current: true,
                label: "first provider".to_owned(),
                detail: "first provider detail".to_owned(),
                recommended: true,
            },
            StartupProviderOption {
                kind: ProviderKind::Anthropic,
                auth_env_name: None,
                is_current: false,
                label: "second provider".to_owned(),
                detail: "second provider detail".to_owned(),
                recommended: false,
            },
        ];
        state.provider_index = 1;

        let rendered = super::render_startup_onboarding_lines(&state, 90)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("first provider"));
        assert!(rendered.contains("second provider"));
        assert!(!rendered.contains("first provider detail"));
        assert!(rendered.contains("second provider detail"));
    }

    #[test]
    fn startup_onboarding_setup_path_stage_surfaces_channel_follow_up_details() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
        state.enabled_channel_labels = vec!["飞书".to_owned(), "企业微信".to_owned()];
        state.channel_follow_up_commands =
            vec!["feishu serve".to_owned(), "channels serve wecom".to_owned()];
        state.channel_status_commands = vec!["loong doctor".to_owned()];
        state.channel_repair_commands = vec!["loong feishu onboard".to_owned()];

        let rendered = super::render_startup_onboarding_lines(&state, 90)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("channels + delivery"));
        assert!(rendered.contains("Enabled channels now: 飞书, 企业微信."));
        assert!(rendered.contains("Next runtime command: feishu serve."));
        assert!(rendered.contains("channels serve wecom"));
        assert!(rendered.contains("Health command: loong doctor."));
        assert!(rendered.contains("Repair path: loong feishu onboard."));
    }

    #[test]
    fn startup_onboarding_channels_path_suggests_next_surfaces_when_none_are_enabled() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
        state.enabled_channel_labels.clear();
        state.channel_follow_up_commands.clear();
        state.channel_status_commands.clear();
        state.channel_repair_commands.clear();

        let rendered = super::render_startup_onboarding_lines(&state, 100)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Good first channels to wire"));
        assert!(rendered.contains("Telegram"));
        assert!(rendered.contains("Suggested onboarding commands"));
    }

    #[test]
    fn startup_onboarding_uses_localized_setup_path_copy_in_chinese() {
        let mut state = onboarding_state();
        state.language_options = vec![Language::ZhCn];
        state.language_index = 0;
        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
        state.enabled_channel_labels = vec!["飞书".to_owned()];
        state.channel_follow_up_commands = vec!["feishu serve".to_owned()];

        let rendered = super::render_startup_onboarding_lines(&state, 100)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("onboarding · 4/6 · 后续配置"));
        assert!(rendered.contains("当前已启用的 channel：飞书。"));
        assert!(rendered.contains("下一条 runtime command：feishu serve。"));
    }

    #[test]
    fn startup_onboarding_channels_path_localizes_suggested_surfaces_in_chinese() {
        let mut state = onboarding_state();
        state.language_options = vec![Language::ZhCn];
        state.language_index = 0;
        state.stage = StartupOnboardingStage::SetupPath;
        state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
        state.enabled_channel_labels.clear();
        state.channel_follow_up_commands.clear();
        state.channel_status_commands.clear();
        state.channel_repair_commands.clear();

        let rendered = super::render_startup_onboarding_lines(&state, 100)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("建议优先接的 channels"));
        assert!(rendered.contains("飞书"));
        assert!(rendered.contains("建议先跑的 onboarding 命令"));
    }

    #[test]
    fn persist_startup_personalization_localizes_summary_in_chinese() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-startup-personalization-zh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);

        let summary = persist_startup_personalization(
            &mut runtime,
            StartupPersonalizationPreset::Concise,
            Language::ZhCn,
        )
        .expect("persist personalization");

        assert!(summary.contains("已保存"));
        assert!(summary.contains("简洁模式"));
    }

    #[test]
    fn persist_startup_personalization_upgrades_memory_profile_and_saves_choice() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-startup-personalization-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);

        let summary = persist_startup_personalization(
            &mut runtime,
            StartupPersonalizationPreset::Thorough,
            Language::ZhCn,
        )
        .expect("persist personalization");

        assert!(summary.contains("profile_plus_window"));
        assert_eq!(
            runtime.config.memory.profile,
            crate::config::MemoryProfile::ProfilePlusWindow
        );
        let personalization = runtime
            .config
            .memory
            .personalization
            .as_ref()
            .expect("saved personalization");
        assert_eq!(
            personalization.response_density,
            Some(crate::config::ResponseDensity::Thorough)
        );
        assert_eq!(
            personalization.initiative_level,
            Some(crate::config::InitiativeLevel::HighInitiative)
        );
        assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));
    }

    #[test]
    fn persist_startup_personalization_turn_off_suppresses_future_prompts() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-startup-personalization-off-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);

        let summary = persist_startup_personalization(
            &mut runtime,
            StartupPersonalizationPreset::TurnOff,
            Language::En,
        )
        .expect("persist turn-off personalization");

        assert!(summary.contains("turned off"));
        let personalization = runtime
            .config
            .memory
            .personalization
            .as_ref()
            .expect("saved personalization");
        assert_eq!(
            personalization.prompt_state,
            crate::config::PersonalizationPromptState::Suppressed
        );
    }

    #[test]
    fn configured_personalization_stages_first_turn_bootstrap_addendum() {
        let mut app = blank_app();
        app.startup_onboarding = Some(onboarding_state());
        let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap.toml");
        let mut runtime = test_runtime_with_path(config_path);

        app.apply_startup_onboarding_action(
            StartupOnboardingAction::PersistPersonalization(StartupPersonalizationPreset::Balanced),
            &mut runtime,
        )
        .expect("apply startup personalization");

        let addendum = app
            .pending_first_turn_bootstrap_addendum
            .as_deref()
            .expect("bootstrap addendum");
        assert!(addendum.contains("next real reply") || addendum.contains("下一次真正回复"));
    }

    #[test]
    fn turn_off_personalization_does_not_stage_first_turn_bootstrap_addendum() {
        let mut app = blank_app();
        app.startup_onboarding = Some(onboarding_state());
        let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap-off.toml");
        let mut runtime = test_runtime_with_path(config_path);

        app.apply_startup_onboarding_action(
            StartupOnboardingAction::PersistPersonalization(StartupPersonalizationPreset::TurnOff),
            &mut runtime,
        )
        .expect("apply startup personalization");

        assert!(app.pending_first_turn_bootstrap_addendum.is_none());
    }

    #[test]
    fn apply_first_turn_bootstrap_addendum_mutates_runtime_transiently() {
        let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap-runtime.toml");
        let mut runtime = test_runtime_with_path(config_path);
        runtime.config.cli.prompt_pack_id = Some("default".to_owned());

        super::apply_first_turn_bootstrap_addendum(
            &mut runtime,
            "bootstrap question here".to_owned(),
        );

        assert_eq!(
            runtime.config.cli.system_prompt_addendum.as_deref(),
            Some("bootstrap question here")
        );
    }

    #[test]
    fn infer_startup_bootstrap_capture_supports_natural_language_english() {
        let capture = super::infer_startup_bootstrap_capture(
            "You can call me Chum. My pronouns are he/they. Your name is Loongy. Creature is dragon. Vibe is calm. Emoji is 🐉. My timezone is Asia/Shanghai. Boundaries are ask before destructive actions. Note: I work mostly late at night.",
        )
        .expect("capture");

        assert_eq!(capture.preferred_address.as_deref(), Some("Chum"));
        assert_eq!(capture.pronouns.as_deref(), Some("he/they"));
        assert_eq!(capture.agent_name.as_deref(), Some("Loongy"));
        assert_eq!(capture.creature.as_deref(), Some("dragon"));
        assert_eq!(capture.vibe.as_deref(), Some("calm"));
        assert_eq!(capture.emoji.as_deref(), Some("🐉"));
        assert_eq!(capture.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(
            capture.standing_boundaries.as_deref(),
            Some("ask before destructive actions")
        );
        assert_eq!(
            capture.notes.as_deref(),
            Some("I work mostly late at night")
        );
    }

    #[test]
    fn infer_startup_bootstrap_capture_supports_natural_language_chinese() {
        let capture = super::infer_startup_bootstrap_capture(
            "叫我伙伴。代词是 ta。你可以叫星龙。物种是龙，气质是沉稳，emoji 用✨。时区是 Asia/Shanghai。边界是先问再做破坏性操作。备注是我通常夜里工作。",
        )
        .expect("capture");

        assert_eq!(capture.preferred_address.as_deref(), Some("伙伴"));
        assert_eq!(capture.pronouns.as_deref(), Some("ta"));
        assert_eq!(capture.agent_name.as_deref(), Some("星龙"));
        assert_eq!(capture.creature.as_deref(), Some("龙"));
        assert_eq!(capture.vibe.as_deref(), Some("沉稳"));
        assert_eq!(capture.emoji.as_deref(), Some("✨"));
        assert_eq!(capture.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(
            capture.standing_boundaries.as_deref(),
            Some("先问再做破坏性操作")
        );
        assert_eq!(capture.notes.as_deref(), Some("我通常夜里工作"));
    }

    #[test]
    fn persist_startup_bootstrap_capture_updates_config_and_runtime_self_files() {
        let temp_root = std::env::temp_dir().join(format!(
            "loong-startup-bootstrap-capture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("unix time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        let config_path = temp_root.join("config.toml");
        crate::config::write(
            Some(config_path.to_string_lossy().as_ref()),
            &LoongConfig::default(),
            true,
        )
        .expect("seed config");
        let mut runtime = test_runtime_with_path(config_path);
        runtime.effective_working_directory = Some(temp_root.clone());

        let capture = StartupBootstrapCapture {
            preferred_address: Some("Chum".to_owned()),
            pronouns: Some("he/they".to_owned()),
            agent_name: Some("Loongy".to_owned()),
            creature: Some("dragon".to_owned()),
            vibe: Some("calm".to_owned()),
            emoji: Some("🐉".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            notes: Some("Works mostly late at night.".to_owned()),
        };

        super::persist_startup_bootstrap_capture(&mut runtime, &capture)
            .expect("persist bootstrap capture");

        let personalization = runtime
            .config
            .memory
            .personalization
            .as_ref()
            .expect("personalization");
        assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
        assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(
            personalization.standing_boundaries.as_deref(),
            Some("Ask before destructive actions.")
        );

        let user = std::fs::read_to_string(temp_root.join("USER.md")).expect("USER.md");
        let identity = std::fs::read_to_string(temp_root.join("IDENTITY.md")).expect("IDENTITY.md");
        let soul = std::fs::read_to_string(temp_root.join("SOUL.md")).expect("SOUL.md");
        assert!(user.contains("Preferred address: Chum"));
        assert!(user.contains("Pronouns: he/they"));
        assert!(user.contains("Timezone: Asia/Shanghai"));
        assert!(user.contains("Standing boundaries: Ask before destructive actions."));
        assert!(user.contains("Notes: Works mostly late at night."));
        assert!(identity.contains("Name: Loongy"));
        assert!(identity.contains("Creature: dragon"));
        assert!(identity.contains("Vibe: calm"));
        assert!(identity.contains("Emoji: 🐉"));
        assert!(soul.contains("Preferred vibe: calm"));
        assert!(soul.contains("Signature emoji: 🐉"));
    }

    #[test]
    fn unparsable_bootstrap_reply_keeps_waiting_for_capture() {
        let mut app = blank_app();
        app.awaiting_first_turn_bootstrap_reply = true;
        let config_path = PathBuf::from("/tmp/loong-bootstrap-waiting.toml");
        let mut runtime = test_runtime_with_path(config_path);

        super::maybe_capture_and_persist_first_turn_bootstrap_reply(
            &mut app,
            &mut runtime,
            "let's just continue for now",
        )
        .expect("capture reply");

        assert!(app.awaiting_first_turn_bootstrap_reply);
    }

    #[test]
    fn bootstrap_reply_opt_out_clears_waiting_without_persisting() {
        let mut app = blank_app();
        app.awaiting_first_turn_bootstrap_reply = true;
        let config_path = PathBuf::from("/tmp/loong-bootstrap-opt-out.toml");
        let mut runtime = test_runtime_with_path(config_path);

        super::maybe_capture_and_persist_first_turn_bootstrap_reply(
            &mut app,
            &mut runtime,
            "skip for now",
        )
        .expect("capture reply");

        assert!(!app.awaiting_first_turn_bootstrap_reply);
        assert!(runtime.config.memory.personalization.is_none());
    }

    #[test]
    fn finish_stage_summarizes_setup_path_and_personalization_choice() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::Finish;
        state.setup_path_index = StartupSetupPathChoice::ProviderAndWeb as usize;
        state.selected_personalization = Some(StartupPersonalizationPreset::Balanced);

        let rendered = super::render_startup_onboarding_lines(&state, 90)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("setup path · provider + web setup"));
        assert!(rendered.contains("personalization · balanced operator"));
    }

    #[test]
    fn finish_stage_turn_off_personalization_updates_finish_subtitle() {
        let mut state = onboarding_state();
        state.stage = StartupOnboardingStage::Finish;
        state.language_options = vec![Language::ZhCn];
        state.language_index = 0;
        state.selected_personalization = Some(StartupPersonalizationPreset::TurnOff);

        let rendered = super::render_startup_onboarding_lines(&state, 100)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Loong 不会再主动弹个性化提示"));
        assert!(rendered.contains("loong personalize"));
    }
}
