use loong_contracts::{
    AuthorizationAttempt, AuthorizationAttemptEvent, AuthorizationPolicyEvent,
    AuthorizationTerminalOutcome, Capabilities, Capability,
};

use super::test_support::{MemoryTestContext, MemoryTestContextFactory, MemoryTestPolicyEngine};
use super::{MemoryAccess, MemoryAccessError, MemoryBackendError};

fn all_memory_capabilities() -> Capabilities {
    Capabilities::from([Capability::MemoryRead, Capability::MemoryWrite])
}

#[tokio::test]
async fn append_uses_the_context_session_and_runs_only_after_grant() {
    let engine = MemoryTestPolicyEngine::allowing();
    let ctx = MemoryTestContext::new(all_memory_capabilities());

    MemoryAccess::<MemoryTestContextFactory, _>::new(&engine, &ctx)
        .append_turn("user", "hello")
        .await
        .expect("append should be granted");

    assert_eq!(
        *ctx.backend.executions.lock().expect("memory execution log"),
        ["append_turn"]
    );
    let evidence = engine
        .evidence
        .lock()
        .expect("memory authorization evidence log");
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].action.kind, "memory.append_turn");
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Allow { .. }
                ),
                ..
            },
            ..
        }
    ));
}

#[tokio::test]
async fn policy_denial_prevents_memory_execution() {
    let engine = MemoryTestPolicyEngine::denying();
    let ctx = MemoryTestContext::new(all_memory_capabilities());

    let error = MemoryAccess::<MemoryTestContextFactory, _>::new(&engine, &ctx)
        .append_turn("user", "must not persist")
        .await
        .expect_err("policy should deny append");

    assert!(matches!(error, MemoryAccessError::Authorization(_)));
    assert!(
        ctx.backend
            .executions
            .lock()
            .expect("memory execution log")
            .is_empty()
    );
    let evidence = engine
        .evidence
        .lock()
        .expect("memory authorization evidence log");
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].action.kind, "memory.append_turn");
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Deny { .. }
                ),
                ..
            },
            ..
        }
    ));
}

#[tokio::test]
async fn missing_capability_prevents_policy_authorized_execution() {
    let engine = MemoryTestPolicyEngine::allowing();
    let ctx = MemoryTestContext::new(Capabilities::from([Capability::MemoryRead]));

    let error = MemoryAccess::<MemoryTestContextFactory, _>::new(&engine, &ctx)
        .append_turn("user", "must not persist")
        .await
        .expect_err("MemoryWrite is required");

    assert!(matches!(error, MemoryAccessError::Authorization(_)));
    assert!(
        ctx.backend
            .executions
            .lock()
            .expect("memory execution log")
            .is_empty()
    );
}

#[tokio::test]
async fn stage_and_compact_outputs_come_from_granted_context_execution() {
    let engine = MemoryTestPolicyEngine::allowing();
    let ctx = MemoryTestContext::new(all_memory_capabilities());

    let stage = MemoryAccess::<MemoryTestContextFactory, _>::new(&engine, &ctx)
        .read_stage_envelope()
        .await
        .expect("stage read should be granted");
    let compact = MemoryAccess::<MemoryTestContextFactory, _>::new(&engine, &ctx)
        .compact()
        .await
        .expect("compact should be granted");

    assert_eq!(stage, "typed-stage-envelope");
    assert_eq!(compact, "typed-compact-output");
    assert_eq!(
        *ctx.backend.executions.lock().expect("memory execution log"),
        ["read_stage_envelope", "compact"]
    );
    let evidence = engine
        .evidence
        .lock()
        .expect("memory authorization evidence log");
    assert_eq!(evidence.len(), 2);
    let mut grant_ids = Vec::new();
    for (item, kind) in evidence
        .iter()
        .zip(["memory.read_stage_envelope", "memory.compact"])
    {
        assert_eq!(item.action.kind, kind);
        let AuthorizationAttempt::Started {
            event:
                AuthorizationAttemptEvent::Policy {
                    event:
                        AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow {
                            grant_id,
                        }),
                    ..
                },
            ..
        } = &item.attempt
        else {
            panic!("memory action should record a terminal allow grant");
        };
        grant_ids.push(*grant_id);
    }
    assert_ne!(grant_ids[0], grant_ids[1]);
}

#[test]
fn backend_error_preserves_its_typed_source() {
    let error = MemoryBackendError::Execution {
        operation: "window",
        source: Box::new(std::io::Error::other("memory database unavailable")),
    };

    let source = std::error::Error::source(&error).expect("backend error source");
    assert!(source.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(source.to_string(), "memory database unavailable");
}
