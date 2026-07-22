use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use loong_core::policy::grant::Granted;
use loong_kernel::InMemoryAuditSink;
use loong_kernel::access::memory::{
    MemoryAppendTurnAction, MemoryBackend, MemoryBackendError, MemoryCompactAction,
    MemoryReadStageEnvelopeAction, MemoryReplaceTurnsAction, MemoryReplaceTurnsOutcome,
    MemorySnapshot, MemoryTranscriptAction, MemoryTurn, MemoryWindowAction,
};

use super::{
    DefaultLegacyToolDispatcher, GovernedSessionMode, Session, TestRuntimeSession, test_config,
};

/// Typed observations emitted by the test backend.
///
/// Tests assert the concrete Action contract instead of reconstructing the
/// removed `MemoryCoreRequest` envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MemoryBackendCall {
    AppendTurn {
        session_id: String,
        role: String,
        content: String,
    },
    Window {
        session_id: String,
        limit: usize,
        allow_extended_limit: bool,
    },
    Transcript {
        session_id: String,
        page_size: usize,
    },
    ReplaceTurns {
        session_id: String,
        turns: Vec<MemoryTurn>,
        expected_turn_count: Option<usize>,
    },
    ReadStageEnvelope {
        session_id: String,
        workspace_root: Option<PathBuf>,
    },
    Compact {
        session_id: String,
        workspace_root: Option<PathBuf>,
    },
}

pub(super) struct TypedMemoryFixture {
    pub(super) owner: TestRuntimeSession,
    pub(super) calls: Arc<Mutex<Vec<MemoryBackendCall>>>,
}

impl TypedMemoryFixture {
    pub(super) fn with_turns(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
        turns: Vec<MemoryTurn>,
    ) -> Self {
        Self::new(
            audit,
            session_id,
            ScriptedMemoryBehavior::Static(MemorySnapshot {
                turn_count: turns.len(),
                turns,
            }),
        )
    }

    pub(super) fn with_window_sequence(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
        windows: Vec<Vec<MemoryTurn>>,
    ) -> Self {
        let snapshots = windows
            .into_iter()
            .map(|turns| MemorySnapshot {
                turn_count: turns.len(),
                turns,
            })
            .collect();
        Self::new(
            audit,
            session_id,
            ScriptedMemoryBehavior::Sequence(Mutex::new(snapshots)),
        )
    }

    pub(super) fn with_window_error(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
        reason: impl Into<String>,
    ) -> Self {
        Self::new(
            audit,
            session_id,
            ScriptedMemoryBehavior::FailingWindow(reason.into()),
        )
    }

    pub(super) fn with_compaction_conflict(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
    ) -> Self {
        Self::new(
            audit,
            session_id,
            ScriptedMemoryBehavior::Conflict(Mutex::new(ConflictState::default())),
        )
    }

    pub(super) fn with_incomplete_snapshot(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
        turns: Vec<MemoryTurn>,
        turn_count: usize,
    ) -> Self {
        Self::new(
            audit,
            session_id,
            ScriptedMemoryBehavior::Incomplete(MemorySnapshot { turns, turn_count }),
        )
    }

    pub(super) fn replace_owner_with_turns(
        owner: TestRuntimeSession,
        turns: Vec<MemoryTurn>,
    ) -> Self {
        Self::replace_owner(
            owner,
            ScriptedMemoryBehavior::Static(MemorySnapshot {
                turn_count: turns.len(),
                turns,
            }),
        )
    }

    pub(super) fn replace_owner_with_window_error(
        owner: TestRuntimeSession,
        reason: impl Into<String>,
    ) -> Self {
        Self::replace_owner(owner, ScriptedMemoryBehavior::FailingWindow(reason.into()))
    }

    /// Assemble the real Runtime/Session owners around one injected backend.
    ///
    /// This is deliberately the fixture's only construction path so custom
    /// memory behavior cannot accidentally replace the production policy
    /// pipeline with a legacy-only Kernel.
    fn new(
        audit: Arc<InMemoryAuditSink>,
        session_id: &str,
        behavior: ScriptedMemoryBehavior,
    ) -> Self {
        let config = test_config();
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);
        let runtime =
            crate::runtime::bootstrap_runtime_with_audit_sink(audit, &config, &tool_runtime_config)
                .expect("bootstrap typed memory test runtime");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(ScriptedMemoryBackend {
            calls: Arc::clone(&calls),
            behavior,
        });
        let session = Session::from_config(
            runtime.as_ref(),
            &config,
            session_id,
            "test-agent",
            GovernedSessionMode::MutatingCapable,
        )
        .expect("build typed memory test session")
        .with_memory_backend_for_test(backend);
        let legacy_tools = DefaultLegacyToolDispatcher::with_config(
            Arc::clone(&runtime),
            &session,
            crate::session::store::SessionStoreConfig::from_memory_config(&config.memory),
            config,
        )
        .expect("build explicit legacy fallback for conversation test");

        Self {
            owner: TestRuntimeSession {
                runtime,
                session,
                legacy_tools,
            },
            calls,
        }
    }

    fn replace_owner(mut owner: TestRuntimeSession, behavior: ScriptedMemoryBehavior) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(ScriptedMemoryBackend {
            calls: Arc::clone(&calls),
            behavior,
        });
        owner.session = owner.session.with_memory_backend_for_test(backend);
        Self { owner, calls }
    }
}

struct ScriptedMemoryBackend {
    calls: Arc<Mutex<Vec<MemoryBackendCall>>>,
    behavior: ScriptedMemoryBehavior,
}

enum ScriptedMemoryBehavior {
    Static(MemorySnapshot),
    Sequence(Mutex<VecDeque<MemorySnapshot>>),
    FailingWindow(String),
    Conflict(Mutex<ConflictState>),
    Incomplete(MemorySnapshot),
}

impl ScriptedMemoryBehavior {
    fn stage_snapshot(&self) -> Result<MemorySnapshot, MemoryBackendError> {
        match self {
            Self::Static(snapshot) | Self::Incomplete(snapshot) => Ok(snapshot.clone()),
            Self::Sequence(snapshots) => Ok(snapshots
                .lock()
                .expect("typed memory sequence")
                .front()
                .cloned()
                .unwrap_or(MemorySnapshot {
                    turns: Vec::new(),
                    turn_count: 0,
                })),
            Self::FailingWindow(reason) => Err(MemoryBackendError::Execution {
                operation: "read_stage_envelope",
                source: reason.clone().into(),
            }),
            Self::Conflict(state) => {
                let state = state.lock().expect("typed memory conflict state");
                Ok(MemorySnapshot {
                    turns: state.turns.clone(),
                    turn_count: state.turn_count,
                })
            }
        }
    }

    fn next_window(&self) -> Result<MemorySnapshot, MemoryBackendError> {
        match self {
            Self::Static(snapshot) | Self::Incomplete(snapshot) => Ok(snapshot.clone()),
            Self::Sequence(snapshots) => Ok(snapshots
                .lock()
                .expect("typed memory sequence")
                .pop_front()
                .unwrap_or(MemorySnapshot {
                    turns: Vec::new(),
                    turn_count: 0,
                })),
            Self::FailingWindow(reason) => Err(MemoryBackendError::Execution {
                operation: crate::memory::MEMORY_OP_WINDOW,
                source: reason.clone().into(),
            }),
            Self::Conflict(state) => {
                let state = state.lock().expect("typed memory conflict state");
                Ok(MemorySnapshot {
                    turns: state.turns.clone(),
                    turn_count: state.turn_count,
                })
            }
        }
    }
}

#[async_trait]
impl MemoryBackend for ScriptedMemoryBackend {
    type StageEnvelope = crate::memory::StageEnvelope;
    type CompactOutput = crate::memory::StageDiagnostics;

    async fn append_turn(
        &self,
        granted: Granted<MemoryAppendTurnAction>,
    ) -> Result<(), MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::AppendTurn {
                session_id: action.session_id().to_owned(),
                role: action.role().to_owned(),
                content: action.content().to_owned(),
            });
        Ok(())
    }

    async fn window(
        &self,
        granted: Granted<MemoryWindowAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::Window {
                session_id: action.session_id().to_owned(),
                limit: action.limit(),
                allow_extended_limit: action.allow_extended_limit(),
            });
        self.behavior.next_window()
    }

    async fn transcript(
        &self,
        granted: Granted<MemoryTranscriptAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::Transcript {
                session_id: action.session_id().to_owned(),
                page_size: action.page_size(),
            });
        self.behavior.stage_snapshot()
    }

    async fn replace_turns(
        &self,
        granted: Granted<MemoryReplaceTurnsAction>,
    ) -> Result<MemoryReplaceTurnsOutcome, MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::ReplaceTurns {
                session_id: action.session_id().to_owned(),
                turns: action.turns().to_vec(),
                expected_turn_count: action.expected_turn_count(),
            });

        match &self.behavior {
            ScriptedMemoryBehavior::Conflict(state) => {
                let mut state = state.lock().expect("typed memory conflict state");
                if !state.conflict_emitted {
                    state.turn_count = state.turn_count.saturating_add(1);
                    let concurrent_ts = i64::try_from(state.turn_count).unwrap_or(i64::MAX);
                    state.turns.push(MemoryTurn {
                        role: "user".to_owned(),
                        content: "concurrent ask".to_owned(),
                        ts: Some(concurrent_ts),
                    });
                    state.conflict_emitted = true;
                    return Ok(MemoryReplaceTurnsOutcome::Conflict);
                }
                if action.expected_turn_count() != Some(state.turn_count) {
                    return Ok(MemoryReplaceTurnsOutcome::Conflict);
                }
                state.turns = action.turns().to_vec();
                state.turn_count = state.turns.len();
                Ok(MemoryReplaceTurnsOutcome::Replaced)
            }
            ScriptedMemoryBehavior::Incomplete(_) => Err(MemoryBackendError::Execution {
                operation: "replace_turns",
                source: "replace_turns must not run for an incomplete snapshot".into(),
            }),
            ScriptedMemoryBehavior::Static(_)
            | ScriptedMemoryBehavior::Sequence(_)
            | ScriptedMemoryBehavior::FailingWindow(_) => Ok(MemoryReplaceTurnsOutcome::Replaced),
        }
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<Self::StageEnvelope, MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::ReadStageEnvelope {
                session_id: action.session_id().to_owned(),
                workspace_root: action.workspace_root().map(ToOwned::to_owned),
            });
        self.behavior
            .stage_snapshot()
            .map(stage_envelope_from_snapshot)
    }

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<Self::CompactOutput, MemoryBackendError> {
        let action = granted.as_ref();
        self.calls
            .lock()
            .expect("typed memory calls")
            .push(MemoryBackendCall::Compact {
                session_id: action.session_id().to_owned(),
                workspace_root: action.workspace_root().map(ToOwned::to_owned),
            });
        Ok(crate::memory::StageDiagnostics::succeeded(
            crate::memory::MemoryStageFamily::Compact,
        ))
    }
}

struct ConflictState {
    turn_count: usize,
    turns: Vec<MemoryTurn>,
    conflict_emitted: bool,
}

impl Default for ConflictState {
    fn default() -> Self {
        Self {
            turn_count: 6,
            turns: vec![
                memory_turn("user", "ask 1", 1),
                memory_turn("assistant", "reply 1", 2),
                memory_turn("user", "ask 2", 3),
                memory_turn("assistant", "reply 2", 4),
                memory_turn("user", "recent ask", 5),
                memory_turn("assistant", "recent reply", 6),
            ],
            conflict_emitted: false,
        }
    }
}

pub(super) fn memory_turn(role: &str, content: impl Into<String>, ts: i64) -> MemoryTurn {
    MemoryTurn {
        role: role.to_owned(),
        content: content.into(),
        ts: Some(ts),
    }
}

fn stage_envelope_from_snapshot(snapshot: MemorySnapshot) -> crate::memory::StageEnvelope {
    let recent_window = snapshot
        .turns
        .into_iter()
        .map(|turn| crate::memory::WindowTurn {
            role: turn.role,
            content: turn.content,
            ts: turn.ts,
        })
        .collect::<Vec<_>>();
    let entries = recent_window
        .iter()
        .map(|turn| crate::memory::MemoryContextEntry {
            kind: crate::memory::MemoryContextKind::Turn,
            role: turn.role.clone(),
            content: turn.content.clone(),
            provenance: Vec::new(),
        })
        .collect::<Vec<_>>();

    crate::memory::StageEnvelope {
        hydrated: crate::memory::HydratedMemoryContext {
            diagnostics: crate::memory::MemoryDiagnostics {
                system_id: "builtin".to_owned(),
                fail_open: true,
                strict_mode_requested: false,
                strict_mode_active: false,
                degraded: false,
                derivation_error: None,
                retrieval_error: None,
                rank_error: None,
                recent_window_count: recent_window.len(),
                entry_count: entries.len(),
            },
            entries,
            recent_window,
        },
        retrieval_request: None,
        retrieval_planner_snapshot: None,
        retrieval_outcome: None,
        diagnostics: vec![
            crate::memory::StageDiagnostics::succeeded(crate::memory::MemoryStageFamily::Derive),
            crate::memory::StageDiagnostics::succeeded(crate::memory::MemoryStageFamily::Retrieve),
            crate::memory::StageDiagnostics::succeeded(crate::memory::MemoryStageFamily::Rank),
        ],
    }
}
