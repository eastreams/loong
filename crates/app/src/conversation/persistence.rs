use crate::CliResult;
use crate::KernelContext;

use super::runtime::ConversationRuntime;

pub(super) fn format_provider_error_reply(error: &str) -> String {
    format!("[provider_error] {error}")
}

pub(super) async fn persist_success_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, "user", user_input, kernel_ctx)
        .await?;
    runtime
        .persist_turn(session_id, "assistant", assistant_reply, kernel_ctx)
        .await?;
    Ok(())
}

pub(super) async fn persist_error_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    synthetic_reply: &str,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, "user", user_input, kernel_ctx)
        .await?;
    runtime
        .persist_turn(session_id, "assistant", synthetic_reply, kernel_ctx)
        .await?;
    Ok(())
}
