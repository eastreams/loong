use super::{ChatKeyEvent, ChatMouseEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatInputEvent {
    Key(ChatKeyEvent),
    Mouse(ChatMouseEvent),
    Resize { width: u16, height: u16 },
    Paste(String),
    FocusGained,
    FocusLost,
}
