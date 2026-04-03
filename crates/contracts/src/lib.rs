#![forbid(unsafe_code)]

mod audit_types;
mod child_process_env;
mod clock;
mod contracts;
mod errors;
mod execution_security_types;
mod fault;
mod memory_types;
mod namespace;
mod pack;
mod policy_types;
mod runtime_types;
mod secret_ref;
mod secret_resolver;
mod secret_value;
mod task_state;
mod tool_types;
mod workflow_types;

pub use audit_types::{AuditEvent, AuditEventKind, AuditIntegrity, ExecutionPlane, PlaneTier};
pub use child_process_env::{
    HIGH_RISK_CHILD_PROCESS_ENV_VARS, child_process_env_var_is_allowed, sanitized_child_process_env,
};
pub use clock::{Clock, FixedClock, SystemClock};
pub use contracts::{
    Capability, CapabilityToken, ConnectorCommand, ConnectorOutcome, ExecutionRoute, HarnessKind,
    HarnessOutcome, HarnessRequest, TaskIntent,
};
pub use errors::{
    AuditError, ConnectorError, HarnessError, IntegrationError, KernelError, MemoryPlaneError,
    PackError, PolicyError, RuntimePlaneError, ToolPlaneError,
};
pub use execution_security_types::ExecutionSecurityTier;
pub use fault::Fault;
pub use memory_types::{
    MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionOutcome, MemoryExtensionRequest,
    MemoryTier,
};
pub use namespace::Namespace;
pub use pack::VerticalPackManifest;
pub use policy_types::{PolicyContext, PolicyDecision, PolicyRequest};
pub use runtime_types::{
    RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionOutcome, RuntimeExtensionRequest,
    RuntimeTier,
};
pub use secret_ref::SecretRef;
pub use secret_resolver::{SecretResolutionError, SecretResolver};
pub use secret_value::SecretValue;
pub use task_state::TaskState;
pub use tool_types::{
    ToolCoreOutcome, ToolCoreRequest, ToolExtensionOutcome, ToolExtensionRequest, ToolTier,
};
pub use workflow_types::{
    GovernedSessionBindingDescriptor, GovernedSessionMode, TaskScopeDescriptor,
    WorkflowOperationKind, WorkflowOperationScope, WorktreeBindingDescriptor,
};

#[cfg(test)]
mod secret_contract_tests {
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};

    use super::{SecretRef, SecretValue};

    #[derive(Debug, Deserialize, Serialize)]
    struct SecretHolder {
        secret: SecretRef,
    }

    #[test]
    fn secret_ref_toml_string_parses_as_inline_literal() {
        let raw = r#"secret = "sk-inline-secret""#;

        let parsed = toml::from_str::<SecretHolder>(raw).expect("inline secret should parse");

        assert_eq!(
            parsed.secret,
            SecretRef::Inline("sk-inline-secret".to_owned())
        );
    }

    #[test]
    fn secret_ref_toml_dollar_reference_parses_as_env_variant() {
        let raw = r#"secret = "${OPENAI_API_KEY}""#;

        let parsed = toml::from_str::<SecretHolder>(raw).expect("env reference should parse");

        assert_eq!(
            parsed.secret,
            SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            }
        );
    }

    #[test]
    fn secret_ref_toml_file_table_parses_as_file_variant() {
        let raw = r#"secret = { file = "/run/secrets/openai" }"#;

        let parsed = toml::from_str::<SecretHolder>(raw).expect("file secret should parse");

        assert_eq!(
            parsed.secret,
            SecretRef::File {
                file: PathBuf::from("/run/secrets/openai"),
            }
        );
    }

    #[test]
    fn secret_ref_toml_exec_table_parses_as_exec_variant() {
        let raw = r#"secret = { exec = ["op", "read", "op://vault/openai/token"] }"#;

        let parsed = toml::from_str::<SecretHolder>(raw).expect("exec secret should parse");

        assert_eq!(
            parsed.secret,
            SecretRef::Exec {
                exec: vec![
                    "op".to_owned(),
                    "read".to_owned(),
                    "op://vault/openai/token".to_owned(),
                ],
            }
        );
    }

    #[test]
    fn secret_ref_rejects_table_with_multiple_variants() {
        let raw = r#"secret = { env = "OPENAI_API_KEY", file = "/run/secrets/openai" }"#;

        let error = toml::from_str::<SecretHolder>(raw)
            .expect_err("multiple variant keys should be rejected");

        let rendered = error.to_string();

        assert!(rendered.contains("exactly one"));
        assert!(rendered.contains("env"));
        assert!(rendered.contains("file"));
    }

    #[test]
    fn secret_ref_rejects_unknown_table_key() {
        let raw = r#"secret = { provider = "vault" }"#;

        let error =
            toml::from_str::<SecretHolder>(raw).expect_err("unknown key should be rejected");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn secret_ref_debug_redacts_inline_value() {
        let secret = SecretRef::Inline("sk-inline-secret".to_owned());

        let rendered = format!("{secret:?}");

        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("sk-inline-secret"));
    }

    #[test]
    fn secret_ref_toml_serialize_round_trips_inline_literal() {
        let holder = SecretHolder {
            secret: SecretRef::Inline("sk-inline-secret".to_owned()),
        };

        let encoded = toml::to_string(&holder).expect("inline secret should serialize");

        assert!(encoded.contains("sk-inline-secret"));
        assert!(!encoded.contains("<redacted>"));
    }

    #[test]
    fn secret_ref_explicit_env_name_supports_inline_compatibility_formats() {
        let cases = vec![
            ("${LOONGCLAW_SECRET}", Some("LOONGCLAW_SECRET")),
            ("$LOONGCLAW_SECRET", Some("LOONGCLAW_SECRET")),
            ("env:LOONGCLAW_SECRET", Some("LOONGCLAW_SECRET")),
            ("%LOONGCLAW_SECRET%", Some("LOONGCLAW_SECRET")),
            ("sk-inline-secret", None),
        ];

        for (raw_value, expected_env_name) in cases {
            let secret = SecretRef::Inline(raw_value.to_owned());
            let env_name = secret.explicit_env_name();
            assert_eq!(env_name.as_deref(), expected_env_name);
        }
    }

    #[test]
    fn secret_ref_inline_literal_value_excludes_compatibility_env_references() {
        let literal_secret = SecretRef::Inline(" sk-inline-secret ".to_owned());
        let env_reference = SecretRef::Inline("${LOONGCLAW_SECRET}".to_owned());

        assert_eq!(
            literal_secret.inline_literal_value(),
            Some("sk-inline-secret")
        );
        assert_eq!(env_reference.inline_literal_value(), None);
        assert!(literal_secret.is_configured());
        assert!(env_reference.is_configured());
        assert!(!SecretRef::Inline("   ".to_owned()).is_configured());
    }

    #[test]
    fn secret_value_exposes_inner_string() {
        let secret = SecretValue::new("value".to_owned());

        assert_eq!(secret.expose(), "value");
    }
}
