use std::collections::BTreeSet;

use async_trait::async_trait;
use loongclaw_contracts::Capability;
use serde_json::{Value, json};

use crate::CliResult;
use crate::KernelContext;

#[cfg(feature = "memory-sqlite")]
use super::super::memory;
use super::super::{config::LoongClawConfig, provider};
use super::turn_engine::ProviderTurn;

pub struct DefaultConversationRuntime;

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    async fn build_messages(
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
    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        let mut messages = Vec::new();
        if let Some(system_message) = provider::build_system_message(config, include_system_prompt)
        {
            messages.push(system_message);
        }

        if let Some(ctx) = kernel_ctx {
            #[cfg(feature = "memory-sqlite")]
            {
                let request =
                    memory::build_window_request(session_id, config.memory.sliding_window);
                let caps = BTreeSet::from([Capability::MemoryRead]);
                let outcome = ctx
                    .kernel
                    .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                    .await
                    .map_err(|error| format!("load memory window via kernel failed: {error}"))?;
                let turns = memory::decode_window_turns(&outcome.payload);
                for turn in turns {
                    messages.push(json!({
                        "role": turn.role,
                        "content": turn.content,
                    }));
                }
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                let _ = (ctx, session_id, config);
            }
            return Ok(messages);
        }

        messages.extend(provider::load_memory_window_messages(config, session_id)?);
        Ok(messages)
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
            let request = memory::build_append_turn_request(session_id, role, content);
            let caps = BTreeSet::from([Capability::MemoryWrite]);
            ctx.kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("persist {role} turn via kernel failed: {error}"))?;
            return Ok(());
        }

        #[cfg(feature = "memory-sqlite")]
        {
            memory::append_turn_direct(
                session_id,
                role,
                content,
                memory::runtime_config::get_memory_runtime_config(),
            )
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_id, role, content);
        }

        Ok(())
    }
}
