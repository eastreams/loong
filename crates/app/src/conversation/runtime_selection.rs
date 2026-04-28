use crate::CliResult;

use super::super::super::config::LoongConfig;
use super::super::context_engine::ContextEngineMetadata;
use super::super::context_engine_registry::{
    DEFAULT_CONTEXT_ENGINE_ID, context_engine_id_from_env, describe_context_engine,
    list_context_engine_metadata,
};
use super::super::turn_middleware::TurnMiddlewareMetadata;
use super::super::turn_middleware_registry::{
    default_turn_middleware_ids, describe_turn_middlewares, list_turn_middleware_metadata,
    turn_middleware_ids_from_env,
};
use super::runtime_prompt::normalize_turn_middleware_ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEngineSelectionSource {
    Env,
    Config,
    Default,
}

impl ContextEngineSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextEngineSelectionSource::Env => "env",
            ContextEngineSelectionSource::Config => "config",
            ContextEngineSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMiddlewareSelectionSource {
    Env,
    Config,
    Default,
}

impl TurnMiddlewareSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnMiddlewareSelectionSource::Env => "env",
            TurnMiddlewareSelectionSource::Config => "config",
            TurnMiddlewareSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineSelection {
    pub id: String,
    pub source: ContextEngineSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareSelection {
    pub ids: Vec<String>,
    pub source: TurnMiddlewareSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionPolicySnapshot {
    pub enabled: bool,
    pub min_messages: Option<usize>,
    pub trigger_estimated_tokens: Option<usize>,
    pub fail_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineRuntimeSnapshot {
    pub selected: ContextEngineSelection,
    pub selected_metadata: ContextEngineMetadata,
    pub available: Vec<ContextEngineMetadata>,
    pub turn_middlewares: TurnMiddlewareRuntimeSnapshot,
    pub compaction: ContextCompactionPolicySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareRuntimeSnapshot {
    pub selected: TurnMiddlewareSelection,
    pub selected_metadata: Vec<TurnMiddlewareMetadata>,
    pub available: Vec<TurnMiddlewareMetadata>,
}

pub fn resolve_context_engine_selection(config: &LoongConfig) -> ContextEngineSelection {
    if let Some(id) = context_engine_id_from_env() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Env,
        };
    }

    if let Some(id) = config.conversation.context_engine_id() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Config,
        };
    }

    ContextEngineSelection {
        id: DEFAULT_CONTEXT_ENGINE_ID.to_owned(),
        source: ContextEngineSelectionSource::Default,
    }
}

pub fn resolve_turn_middleware_selection(
    config: &LoongConfig,
) -> CliResult<TurnMiddlewareSelection> {
    let mut ids = default_turn_middleware_ids()?;
    if let Some(env_ids) = turn_middleware_ids_from_env() {
        ids.extend(env_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Env,
        });
    }

    let configured_ids = config.conversation.turn_middleware_ids();
    if !configured_ids.is_empty() {
        ids.extend(configured_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Config,
        });
    }

    Ok(TurnMiddlewareSelection {
        ids: normalize_turn_middleware_ids(ids),
        source: TurnMiddlewareSelectionSource::Default,
    })
}

pub fn collect_context_engine_runtime_snapshot(
    config: &LoongConfig,
) -> CliResult<ContextEngineRuntimeSnapshot> {
    let selected = resolve_context_engine_selection(config);
    let selected_metadata = describe_context_engine(Some(selected.id.as_str()))?;
    let available = list_context_engine_metadata()?;
    let turn_middleware_selection = resolve_turn_middleware_selection(config)?;
    let turn_middlewares = TurnMiddlewareRuntimeSnapshot {
        selected_metadata: describe_turn_middlewares(turn_middleware_selection.ids.as_slice())?,
        available: list_turn_middleware_metadata()?,
        selected: turn_middleware_selection,
    };
    let compaction = ContextCompactionPolicySnapshot {
        enabled: config.conversation.compact_enabled,
        min_messages: config.conversation.compact_min_messages(),
        trigger_estimated_tokens: config.conversation.compact_trigger_estimated_tokens(),
        fail_open: config.conversation.compaction_fail_open(),
    };

    Ok(ContextEngineRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        turn_middlewares,
        compaction,
    })
}
