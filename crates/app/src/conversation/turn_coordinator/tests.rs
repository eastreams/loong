use super::compact::persist_runtime_self_continuity_for_compaction;
use super::*;
use crate::config::ToolConfig;
use crate::context::bootstrap_test_kernel_context;
use crate::conversation::delegate_support::{
    finalize_async_delegate_spawn_failure, finalize_async_delegate_spawn_failure_with_recovery,
    finalize_delegate_child_terminal_with_recovery,
};
use crate::conversation::turn_coordinator::skill_activation::ExplicitSkillActivationInput;
use crate::conversation::turn_engine::ToolBatchExecutionIntentTrace;
use crate::conversation::{
    ConversationTurnObserver, ConversationTurnPhase, ConversationTurnToolState,
};
use crate::session::repository::FinalizeSessionTerminalResult;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;

fn unique_sqlite_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loong-turn-coordinator-{label}-{}.sqlite3",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

mod control;
use self::control::*;
#[path = "tests/approval.rs"]
mod approval;
#[path = "tests/compaction.rs"]
mod compaction;
#[path = "tests/delegate.rs"]
mod delegate;
#[path = "tests/followup.rs"]
mod followup;
#[path = "tests/plan_node.rs"]
mod plan_node;
#[path = "tests/runtime.rs"]
mod runtime;
#[path = "tests/tooling.rs"]
mod tooling;

mod provider_turn;
#[path = "tests/task_progress.rs"]
mod task_progress;

#[path = "tests/turn_coordinator_safe_lane_route_tests.rs"]
mod safe_lane_route_tests;
