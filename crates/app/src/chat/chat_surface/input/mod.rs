mod adapter;
mod event;
mod keyboard;
mod mouse;

pub(crate) use adapter::InputAdapter;
pub(crate) use event::ChatInputEvent;
pub(crate) use keyboard::{ChatKeyCode, ChatKeyEvent, ChatKeyEventKind, ChatKeyModifiers};
pub(crate) use mouse::{ChatMouseButton, ChatMouseEvent, ChatMouseEventKind};

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
