use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::ChatKeyModifiers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatMouseEventKind {
    Down(ChatMouseButton),
    Up(ChatMouseButton),
    Drag(ChatMouseButton),
    Moved,
    ScrollDown,
    ScrollUp,
    ScrollLeft,
    ScrollRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChatMouseEvent {
    pub(crate) kind: ChatMouseEventKind,
    pub(crate) column: u16,
    pub(crate) row: u16,
    pub(crate) modifiers: ChatKeyModifiers,
}

impl From<MouseButton> for ChatMouseButton {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => Self::Left,
            MouseButton::Right => Self::Right,
            MouseButton::Middle => Self::Middle,
        }
    }
}

impl From<MouseEventKind> for ChatMouseEventKind {
    fn from(value: MouseEventKind) -> Self {
        match value {
            MouseEventKind::Down(button) => Self::Down(button.into()),
            MouseEventKind::Up(button) => Self::Up(button.into()),
            MouseEventKind::Drag(button) => Self::Drag(button.into()),
            MouseEventKind::Moved => Self::Moved,
            MouseEventKind::ScrollDown => Self::ScrollDown,
            MouseEventKind::ScrollUp => Self::ScrollUp,
            MouseEventKind::ScrollLeft => Self::ScrollLeft,
            MouseEventKind::ScrollRight => Self::ScrollRight,
        }
    }
}

impl From<MouseEvent> for ChatMouseEvent {
    fn from(value: MouseEvent) -> Self {
        Self {
            kind: value.kind.into(),
            column: value.column,
            row: value.row,
            modifiers: value.modifiers.into(),
        }
    }
}
