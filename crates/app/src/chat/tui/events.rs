#[derive(Debug, Clone)]
pub(super) enum UiEvent {
    Tick,
    Terminal(crossterm::event::Event),
    Token {
        content: String,
        is_thinking: bool,
    },
    ToolStart {
        tool_id: String,
        tool_name: String,
        args_preview: String,
    },
    ToolDone {
        tool_id: String,
        success: bool,
        output: String,
        duration_ms: u32,
    },
    PhaseChange {
        phase: String,
        iteration: u32,
        action: String,
    },
    ResponseDone {
        input_tokens: u32,
        output_tokens: u32,
    },
    ClarifyRequest {
        question: String,
        choices: Vec<String>,
    },
    TurnError(String),
}
