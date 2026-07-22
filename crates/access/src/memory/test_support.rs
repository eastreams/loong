use std::{
    borrow::Cow,
    convert::Infallible,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationAttemptId, AuthorizationEvidence, AuthorizationScope, AuthorizationSubject,
    Capabilities, GrantId, PolicyEntry, PolicyId, PolicyOutcome, PolicyRegistration,
    PolicyRegistrationSource, PolicyReport,
};
use loong_core::policy::{
    action::ActionMeta,
    context::{ContextFactory, PolicyContext},
    engine::PolicyEngineBackend,
    grant::Granted,
};

use super::{
    MemoryAppendTurnAction, MemoryBackend, MemoryBackendError, MemoryCompactAction,
    MemoryExecutionContext, MemoryReadStageEnvelopeAction, MemoryReplaceTurnsAction,
    MemoryReplaceTurnsOutcome, MemorySessionContext, MemorySnapshot, MemoryTranscriptAction,
    MemoryWindowAction, MemoryWorkspaceContext,
};

pub(super) struct MemoryTestContext {
    pub(super) capabilities: Capabilities,
    pub(super) session_id: String,
    pub(super) backend: MemoryTestBackend,
}

pub(super) struct MemoryTestBackend {
    pub(super) executions: Mutex<Vec<&'static str>>,
}

impl MemoryTestContext {
    pub(super) fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            session_id: "memory-test-session".to_owned(),
            backend: MemoryTestBackend {
                executions: Mutex::new(Vec::new()),
            },
        }
    }
}

impl MemoryTestBackend {
    fn record(&self, operation: &'static str) {
        self.executions
            .lock()
            .expect("memory execution log")
            .push(operation);
    }
}

impl PolicyContext for MemoryTestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "memory-test-actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: self.session_id.clone(),
            },
        }
    }
}

impl MemorySessionContext for MemoryTestContext {
    fn memory_session_id(&self) -> &str {
        &self.session_id
    }
}

impl MemoryWorkspaceContext for MemoryTestContext {
    fn memory_workspace_root(&self) -> Option<&std::path::Path> {
        None
    }
}

#[async_trait]
impl MemoryBackend for MemoryTestBackend {
    type StageEnvelope = String;
    type CompactOutput = String;

    async fn append_turn(
        &self,
        granted: Granted<MemoryAppendTurnAction>,
    ) -> Result<(), MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("append_turn");
        Ok(())
    }

    async fn window(
        &self,
        granted: Granted<MemoryWindowAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("window");
        Ok(MemorySnapshot {
            turns: Vec::new(),
            turn_count: 0,
        })
    }

    async fn transcript(
        &self,
        granted: Granted<MemoryTranscriptAction>,
    ) -> Result<MemorySnapshot, MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("transcript");
        Ok(MemorySnapshot {
            turns: Vec::new(),
            turn_count: 0,
        })
    }

    async fn replace_turns(
        &self,
        granted: Granted<MemoryReplaceTurnsAction>,
    ) -> Result<MemoryReplaceTurnsOutcome, MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("replace_turns");
        Ok(MemoryReplaceTurnsOutcome::Replaced)
    }

    async fn read_stage_envelope(
        &self,
        granted: Granted<MemoryReadStageEnvelopeAction>,
    ) -> Result<Self::StageEnvelope, MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("read_stage_envelope");
        Ok("typed-stage-envelope".to_owned())
    }

    async fn compact(
        &self,
        granted: Granted<MemoryCompactAction>,
    ) -> Result<Self::CompactOutput, MemoryBackendError> {
        assert_eq!(granted.as_ref().session_id(), "memory-test-session");
        self.record("compact");
        Ok("typed-compact-output".to_owned())
    }
}

impl MemoryExecutionContext for MemoryTestContext {
    type StageEnvelope = String;
    type CompactOutput = String;
    type Backend = MemoryTestBackend;

    fn memory_backend(&self) -> &Self::Backend {
        &self.backend
    }
}

pub(super) struct MemoryTestContextFactory;

impl ContextFactory for MemoryTestContextFactory {
    type Cx<'a> = MemoryTestContext;
}

pub(super) struct MemoryTestPolicyEngine {
    allow: bool,
    sequence: AtomicU64,
    pub(super) evidence: Mutex<Vec<AuthorizationEvidence>>,
}

impl MemoryTestPolicyEngine {
    pub(super) fn allowing() -> Self {
        Self {
            allow: true,
            sequence: AtomicU64::new(0),
            evidence: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn denying() -> Self {
        Self {
            allow: false,
            sequence: AtomicU64::new(0),
            evidence: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl PolicyEngineBackend<MemoryTestContextFactory> for MemoryTestPolicyEngine {
    type AuditError = Infallible;

    async fn decide<A: ActionMeta + 'static>(
        &self,
        _ctx: &<MemoryTestContextFactory as ContextFactory>::Cx<'_>,
        _action: &A,
    ) -> PolicyReport {
        let outcome = if self.allow {
            PolicyOutcome::Allow {
                source: PolicyEntry {
                    policy_name: Cow::Borrowed("memory-test-allow"),
                    policy_id: PolicyId::new(1),
                    registration: PolicyRegistration {
                        order: 1,
                        registered_at_unix_ms: 1,
                        source: PolicyRegistrationSource {
                            file: "memory/test_support.rs".to_owned(),
                            line: 1,
                            column: 1,
                        },
                    },
                },
                reason: Cow::Borrowed("allowed by memory test policy"),
            }
        } else {
            PolicyOutcome::Deny {
                grant_source: None,
                reason: Cow::Borrowed("denied by memory test policy"),
            }
        };
        PolicyReport {
            evaluations: Vec::new(),
            outcome,
        }
    }

    fn reserve_authorization_attempt_id(&self) -> Result<AuthorizationAttemptId, Self::AuditError> {
        Ok(AuthorizationAttemptId(
            self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
        ))
    }

    fn reserve_grant_id(&self) -> Result<GrantId, Self::AuditError> {
        Ok(GrantId::new())
    }

    fn write_authorization_evidence(
        &self,
        evidence: &AuthorizationEvidence,
    ) -> Result<(), Self::AuditError> {
        self.evidence
            .lock()
            .expect("memory authorization evidence log")
            .push(evidence.clone());
        Ok(())
    }
}
