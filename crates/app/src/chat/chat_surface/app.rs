use crossterm::terminal::SetTitle;
use opentelemetry::context::FutureExt;
use opentelemetry::trace::{SpanKind, TraceContextExt, Tracer, TracerProvider};
use opentelemetry::{Context, KeyValue, global};
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
use std::io::{IsTerminal, Write};
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
    CommandAction, CommandPalette, ResumePaletteEntry, SettingsCommandAction, SettingsEntry,
    SettingsSurfaceFocus, SkillEntry, slash_command_specs,
};
use super::composer::Composer;
use super::i18n::{I18nService, Language, SurfaceCopy, resolve_default_language};
use super::input::{
    ChatInputEvent, ChatKeyCode as KeyCode, ChatKeyEvent, ChatKeyModifiers as KeyModifiers,
    ChatMouseButton as MouseButton, ChatMouseEvent, ChatMouseEventKind, InputAdapter,
};
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
const MAX_TERMINAL_TITLE_CHARS: usize = 180;
const TERMINAL_TITLE_BRAILLE_FRAMES: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TERMINAL_TITLE_BRAILLE_INTERVAL_MS: u64 = 100;
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

mod commands;
mod input_palette;
mod pending;
mod runtime;
mod session_router;
mod startup;
mod startup_catalog;
mod state;
mod surface;

use self::commands::*;
use self::input_palette::*;
use self::pending::*;
use self::runtime::refresh_app_cwd_dependent_state;
pub use self::runtime::run_app;
use self::session_router::*;
use self::startup::*;
use self::startup_catalog::*;
pub use self::state::App;
use self::state::{
    LiveTranscriptState, PendingRenderCache, StartupBootstrapCapture,
    StartupChannelFollowUpDescriptor, StartupOnboardingAction, StartupOnboardingInteractionKind,
    StartupOnboardingStage, StartupOnboardingState, StartupPersonalizationPreset,
    StartupProviderAuthBindingKind, StartupProviderOption, StartupSetupPathChoice,
    detected_startup_auth_binding, startup_provider_config_for_kind,
};

mod resume_candidates;
#[cfg(test)]
#[path = "app/app_tests.rs"]
mod tests;
