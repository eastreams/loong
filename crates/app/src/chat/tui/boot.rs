use std::future::Future;
use std::pin::Pin;

use crate::CliResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiBootScreen {
    pub lines: Vec<String>,
    pub prompt_hint: String,
    pub initial_value: String,
    pub escape_submit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiBootTransition {
    Screen(TuiBootScreen),
    StartChat { system_message: Option<String> },
    Exit,
}

pub trait TuiBootFlow: Send {
    fn begin(&mut self, width: usize) -> CliResult<TuiBootScreen>;

    fn submit<'a>(
        &'a mut self,
        input: String,
        width: usize,
    ) -> Pin<Box<dyn Future<Output = CliResult<TuiBootTransition>> + Send + 'a>>;
}
