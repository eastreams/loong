use tokio::process::Command;
use tokio::time::{Duration, timeout};

pub(super) enum CommandOutputError {
    TimedOut,
    Io(std::io::Error),
}

pub(super) async fn wait_for_command_output(
    command: &mut Command,
    timeout_duration: Duration,
) -> Result<std::process::Output, CommandOutputError> {
    command.kill_on_drop(true);
    timeout(timeout_duration, command.output())
        .await
        .map_err(|_timeout_error| CommandOutputError::TimedOut)?
        .map_err(CommandOutputError::Io)
}
