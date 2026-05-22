use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MediaKeyCode, ModifierKeyCode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatMediaKeyCode {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatModifierKeyCode {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatKeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    F(u8),
    Char(char),
    Null,
    Esc,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    KeypadBegin,
    Media(ChatMediaKeyCode),
    Modifier(ChatModifierKeyCode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChatKeyEventKind {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChatKeyEventState(KeyEventState);

impl ChatKeyEventState {
    pub(crate) const NONE: Self = Self(KeyEventState::NONE);
}

impl Default for ChatKeyEventState {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChatKeyModifiers(KeyModifiers);

impl ChatKeyModifiers {
    pub(crate) const SHIFT: Self = Self(KeyModifiers::SHIFT);
    pub(crate) const CONTROL: Self = Self(KeyModifiers::CONTROL);
    pub(crate) const ALT: Self = Self(KeyModifiers::ALT);
    pub(crate) const SUPER: Self = Self(KeyModifiers::SUPER);
    pub(crate) const NONE: Self = Self(KeyModifiers::NONE);

    pub(crate) fn contains(self, other: Self) -> bool {
        self.0.contains(other.0)
    }

    pub(crate) fn intersects(self, other: Self) -> bool {
        self.0.intersects(other.0)
    }
}

impl std::ops::BitOr for ChatKeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl Default for ChatKeyModifiers {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChatKeyEvent {
    pub(crate) code: ChatKeyCode,
    pub(crate) modifiers: ChatKeyModifiers,
    pub(crate) kind: ChatKeyEventKind,
    pub(crate) state: ChatKeyEventState,
}

impl ChatKeyEvent {
    pub(crate) fn new(code: ChatKeyCode, modifiers: ChatKeyModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: ChatKeyEventKind::Press,
            state: ChatKeyEventState::NONE,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_kind(
        code: ChatKeyCode,
        modifiers: ChatKeyModifiers,
        kind: ChatKeyEventKind,
    ) -> Self {
        Self {
            code,
            modifiers,
            kind,
            state: ChatKeyEventState::NONE,
        }
    }
}

impl From<KeyEventState> for ChatKeyEventState {
    fn from(value: KeyEventState) -> Self {
        Self(value)
    }
}

impl From<KeyModifiers> for ChatKeyModifiers {
    fn from(value: KeyModifiers) -> Self {
        Self(value)
    }
}

impl From<MediaKeyCode> for ChatMediaKeyCode {
    fn from(value: MediaKeyCode) -> Self {
        match value {
            MediaKeyCode::Play => Self::Play,
            MediaKeyCode::Pause => Self::Pause,
            MediaKeyCode::PlayPause => Self::PlayPause,
            MediaKeyCode::Reverse => Self::Reverse,
            MediaKeyCode::Stop => Self::Stop,
            MediaKeyCode::FastForward => Self::FastForward,
            MediaKeyCode::Rewind => Self::Rewind,
            MediaKeyCode::TrackNext => Self::TrackNext,
            MediaKeyCode::TrackPrevious => Self::TrackPrevious,
            MediaKeyCode::Record => Self::Record,
            MediaKeyCode::LowerVolume => Self::LowerVolume,
            MediaKeyCode::RaiseVolume => Self::RaiseVolume,
            MediaKeyCode::MuteVolume => Self::MuteVolume,
        }
    }
}

impl From<ModifierKeyCode> for ChatModifierKeyCode {
    fn from(value: ModifierKeyCode) -> Self {
        match value {
            ModifierKeyCode::LeftShift => Self::LeftShift,
            ModifierKeyCode::LeftControl => Self::LeftControl,
            ModifierKeyCode::LeftAlt => Self::LeftAlt,
            ModifierKeyCode::LeftSuper => Self::LeftSuper,
            ModifierKeyCode::LeftHyper => Self::LeftHyper,
            ModifierKeyCode::LeftMeta => Self::LeftMeta,
            ModifierKeyCode::RightShift => Self::RightShift,
            ModifierKeyCode::RightControl => Self::RightControl,
            ModifierKeyCode::RightAlt => Self::RightAlt,
            ModifierKeyCode::RightSuper => Self::RightSuper,
            ModifierKeyCode::RightHyper => Self::RightHyper,
            ModifierKeyCode::RightMeta => Self::RightMeta,
            ModifierKeyCode::IsoLevel3Shift => Self::IsoLevel3Shift,
            ModifierKeyCode::IsoLevel5Shift => Self::IsoLevel5Shift,
        }
    }
}

impl From<KeyCode> for ChatKeyCode {
    fn from(value: KeyCode) -> Self {
        match value {
            KeyCode::Backspace => Self::Backspace,
            KeyCode::Enter => Self::Enter,
            KeyCode::Left => Self::Left,
            KeyCode::Right => Self::Right,
            KeyCode::Up => Self::Up,
            KeyCode::Down => Self::Down,
            KeyCode::Home => Self::Home,
            KeyCode::End => Self::End,
            KeyCode::PageUp => Self::PageUp,
            KeyCode::PageDown => Self::PageDown,
            KeyCode::Tab => Self::Tab,
            KeyCode::BackTab => Self::BackTab,
            KeyCode::Delete => Self::Delete,
            KeyCode::Insert => Self::Insert,
            KeyCode::F(value) => Self::F(value),
            KeyCode::Char(value) => Self::Char(value),
            KeyCode::Null => Self::Null,
            KeyCode::Esc => Self::Esc,
            KeyCode::CapsLock => Self::CapsLock,
            KeyCode::ScrollLock => Self::ScrollLock,
            KeyCode::NumLock => Self::NumLock,
            KeyCode::PrintScreen => Self::PrintScreen,
            KeyCode::Pause => Self::Pause,
            KeyCode::Menu => Self::Menu,
            KeyCode::KeypadBegin => Self::KeypadBegin,
            KeyCode::Media(value) => Self::Media(value.into()),
            KeyCode::Modifier(value) => Self::Modifier(value.into()),
        }
    }
}

impl From<KeyEventKind> for ChatKeyEventKind {
    fn from(value: KeyEventKind) -> Self {
        match value {
            KeyEventKind::Press => Self::Press,
            KeyEventKind::Repeat => Self::Repeat,
            KeyEventKind::Release => Self::Release,
        }
    }
}

impl From<KeyEvent> for ChatKeyEvent {
    fn from(value: KeyEvent) -> Self {
        Self {
            code: value.code.into(),
            modifiers: value.modifiers.into(),
            kind: value.kind.into(),
            state: value.state.into(),
        }
    }
}
