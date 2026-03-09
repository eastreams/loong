use std::collections::BTreeSet;

use loongclaw_contracts::{Capability, KernelError, ToolCoreRequest};
use serde::{Deserialize, Serialize};

use crate::context::KernelContext;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub assistant_text: String,
    pub tool_intents: Vec<ToolIntent>,
    pub raw_meta: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool_name: String,
    pub args_json: serde_json::Value,
    pub source: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecision {
    pub allow: bool,
    pub deny: bool,
    pub approval_required: bool,
    pub reason: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TurnResult {
    FinalText(String),
    NeedsApproval(String),
    ToolDenied(String),
    ToolError(String),
    ProviderError(String),
}

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self { max_tool_steps }
    }

    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent
        for intent in &turn.tool_intents {
            if !crate::tools::is_known_tool_name(&intent.tool_name) {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            }
        }

        // All tools validated — execution requires a kernel context
        TurnResult::NeedsApproval("kernel_context_required".to_owned())
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. No kernel context → `ToolDenied("no_kernel_context")`
    /// 5. Policy/capability check via kernel → `ToolDenied` with reason if denied
    /// 6. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::ToolDenied("max_tool_steps_exceeded".to_owned());
        }

        // Check each tool intent is known
        for intent in &turn.tool_intents {
            if !crate::tools::is_known_tool_name(&intent.tool_name) {
                return TurnResult::ToolDenied(format!("tool_not_found: {}", intent.tool_name));
            }
        }

        // Require kernel context for execution
        let ctx = match kernel_ctx {
            Some(ctx) => ctx,
            None => return TurnResult::ToolDenied("no_kernel_context".to_owned()),
        };

        // Execute each tool intent through the kernel
        let mut outputs = Vec::new();
        for intent in &turn.tool_intents {
            let request = ToolCoreRequest {
                tool_name: intent.tool_name.clone(),
                payload: intent.args_json.clone(),
            };
            let caps = BTreeSet::from([Capability::InvokeTool]);
            match ctx
                .kernel
                .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
            {
                Ok(outcome) => {
                    outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                }
                Err(e) => {
                    // Classify by error variant, not by Display string
                    return match &e {
                        KernelError::Policy(_) | KernelError::PackCapabilityBoundary { .. } => {
                            TurnResult::ToolDenied(format!("{e}"))
                        }
                        _ => TurnResult::ToolError(format!("{e}")),
                    };
                }
            }
        }

        TurnResult::FinalText(outputs.join("\n"))
    }
}
