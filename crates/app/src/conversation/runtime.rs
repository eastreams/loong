use std::collections::BTreeSet;

use async_trait::async_trait;
use loongclaw_contracts::{Capability, MemoryCoreRequest};
use serde_json::{json, Value};

use crate::CliResult;
use crate::KernelContext;

#[cfg(feature = "memory-sqlite")]
use super::super::memory;
use super::super::{config::LoongClawConfig, provider};
use super::turn_engine::ProviderTurn;

pub struct DefaultConversationRuntime;

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<String>;

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<ProviderTurn>;

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()>;
}

#[async_trait]
impl ConversationRuntime for DefaultConversationRuntime {
    // TODO(task-11): Route memory window loading through kernel when kernel_ctx is Some.
    // Currently `build_messages_for_session` couples system-prompt construction with
    // memory window loading in a single function. Routing the memory portion through
    // kernel requires splitting that function into (a) system prompt building and
    // (b) memory window loading. Deferred to avoid invasive refactoring of the
    // provider module.
    fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        provider::build_messages_for_session(config, session_id, include_system_prompt)
    }

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<String> {
        provider::request_completion(config, messages).await
    }

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<ProviderTurn> {
        provider::request_turn(config, messages).await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        if let Some(ctx) = kernel_ctx {
            let request = MemoryCoreRequest {
                operation: "append_turn".to_owned(),
                payload: json!({
                    "session_id": session_id,
                    "role": role,
                    "content": content,
                }),
            };
            let caps = BTreeSet::from([Capability::MemoryWrite]);
            ctx.kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("persist {role} turn via kernel failed: {error}"))?;
            return Ok(());
        }

        #[cfg(feature = "memory-sqlite")]
        {
            memory::append_turn_direct(session_id, role, content)
                .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_id, role, content);
        }

        Ok(())
    }
}
