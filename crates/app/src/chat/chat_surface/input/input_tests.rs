use super::{
    ChatInputEvent, ChatKeyCode, ChatKeyEvent, ChatKeyEventKind, ChatKeyModifiers,
    ChatMouseEventKind, InputAdapter,
};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent, MouseEventKind,
};

#[test]
fn normalize_keeps_press_key_events() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Key(KeyEvent {
        code: KeyCode::Char('a'),
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }));

    assert_eq!(
        normalized,
        Some(ChatInputEvent::Key(ChatKeyEvent {
            code: ChatKeyCode::Char('a'),
            modifiers: ChatKeyModifiers::NONE,
            kind: ChatKeyEventKind::Press,
            state: Default::default(),
        }))
    );
}

#[test]
fn normalize_keeps_repeat_key_events() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Key(KeyEvent {
        code: KeyCode::Left,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Repeat,
        state: KeyEventState::NONE,
    }));

    let Some(ChatInputEvent::Key(key)) = normalized else {
        panic!("expected key event");
    };
    assert_eq!(key.code, ChatKeyCode::Left);
    assert_eq!(key.kind, ChatKeyEventKind::Repeat);
}

#[test]
fn normalize_drops_release_key_events() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Release,
        state: KeyEventState::NONE,
    }));

    assert_eq!(normalized, None);
}

#[test]
fn normalize_passes_mouse_events_through() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 3,
        row: 5,
        modifiers: KeyModifiers::NONE,
    }));

    let Some(ChatInputEvent::Mouse(mouse)) = normalized else {
        panic!("expected mouse event");
    };
    assert_eq!(mouse.kind, ChatMouseEventKind::ScrollDown);
    assert_eq!(mouse.column, 3);
    assert_eq!(mouse.row, 5);
}

#[test]
fn resize_is_exposed_as_structured_chat_input_event() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Resize(80, 24));

    assert_eq!(
        normalized,
        Some(ChatInputEvent::Resize {
            width: 80,
            height: 24,
        })
    );
}

#[test]
fn paste_is_exposed_as_structured_chat_input_event() {
    let normalized = InputAdapter::normalize_raw_event_for_test(Event::Paste("hello".to_owned()));

    assert_eq!(normalized, Some(ChatInputEvent::Paste("hello".to_owned())));
}
