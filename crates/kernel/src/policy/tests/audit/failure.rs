use super::*;
use crate::access::fs::FsResolvePathAllowPolicy;

#[tokio::test]
async fn missing_capability_and_policy_deny_each_record_one_terminal_event() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::<AuditContextFactory>::new(),
        Arc::new(FixedClock::new(42)),
        audit.clone(),
    );
    let ctx = AuditContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
    };

    kernel
        .policy_engine()
        .grant(&ctx, AuditAction::filesystem_read())
        .await
        .expect_err("missing context capability must deny");
    kernel
        .policy_engine()
        .grant(&ctx, AuditAction::invoke_tool("tool"))
        .await
        .expect_err("default deny must reject");

    let events = audit
        .snapshot()
        .into_iter()
        .filter(|event| matches!(event.kind, AuditEventKind::Authorization { .. }))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0].kind,
        AuditEventKind::Authorization {
            evidence: AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::CapabilityDenied { .. },
                    ..
                },
                ..
            }
        }
    ));
    assert!(matches!(
        &events[1].kind,
        AuditEventKind::Authorization {
            evidence: AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::Policy {
                        event: AuthorizationPolicyEvent::Terminal(
                            AuthorizationTerminalOutcome::Deny { .. }
                        ),
                        ..
                    },
                    ..
                },
                ..
            }
        }
    ));
    assert!(
        !audit
            .snapshot()
            .iter()
            .any(|event| matches!(event.kind, AuditEventKind::AuthorizationDenied { .. }))
    );
}

struct FailingAuditSink;

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: AuditEvent) -> Result<(), AuditError> {
        Err(AuditError::Sink("typed audit sink failed".to_owned()))
    }
}

struct FsAuditContext {
    root: PathBuf,
    capabilities: Capabilities,
}

impl PolicyContext for FsAuditContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:kernel:sink-failure:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:kernel:sink-failure:session".to_owned(),
            },
        }
    }
}

impl FsResolutionContext for FsAuditContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.root
    }
}

impl FsPathPolicyContext for FsAuditContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        std::slice::from_ref(&self.root)
    }
}

struct FsAuditContextFactory;

impl ContextFactory for FsAuditContextFactory {
    type Cx<'a> = FsAuditContext;
}

#[tokio::test]
async fn sink_failure_returns_concrete_source_without_releasing_a_grant() {
    let policy = PolicyPipelineBuilder::<FsAuditContextFactory>::new()
        .with_policy(FsResolvePathAllowPolicy::target());
    let kernel = Kernel::with_policy_runtime(
        policy,
        Arc::new(FixedClock::new(42)),
        Arc::new(FailingAuditSink),
    );
    let ctx = FsAuditContext {
        root: PathBuf::from("/workspace"),
        capabilities: Capabilities::new(),
    };

    let error = AccessCx::new(&kernel, &ctx)
        .fs()
        .read_file("note.txt")
        .await
        .expect_err("failed terminal evidence must prevent grant release");

    let FsAccessError::Authorization(AuthorizationError::PolicyGrant(PolicyGrantError::Audit {
        source,
        ..
    })) = error
    else {
        panic!("expected typed audit failure");
    };
    assert!(matches!(
        source.downcast_ref::<AuditError>(),
        Some(AuditError::Sink(reason)) if reason == "typed audit sink failed"
    ));
}
