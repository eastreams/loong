use std::time::Duration;

use crossterm::event::{self, Event};

use super::{ChatInputEvent, ChatKeyEvent, ChatKeyEventKind, ChatMouseEvent};

pub(crate) struct InputAdapter;

impl InputAdapter {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<ChatInputEvent>, String> {
        if !event::poll(timeout).map_err(|e| format!("poll error: {}", e))? {
            return Ok(None);
        }

        let event = event::read().map_err(|e| format!("read error: {}", e))?;
        Ok(normalize_crossterm_event(event))
    }

    #[cfg(test)]
    pub(crate) fn normalize_raw_event_for_test(event: Event) -> Option<ChatInputEvent> {
        normalize_crossterm_event(event)
    }
}

fn normalize_crossterm_event(event: Event) -> Option<ChatInputEvent> {
    match event {
        Event::Key(key) => {
            let key = ChatKeyEvent::from(key);
            match key.kind {
                ChatKeyEventKind::Release => None,
                ChatKeyEventKind::Press | ChatKeyEventKind::Repeat => {
                    Some(ChatInputEvent::Key(key))
                }
            }
        }
        Event::Mouse(mouse) => Some(ChatInputEvent::Mouse(ChatMouseEvent::from(mouse))),
        Event::Resize(width, height) => Some(ChatInputEvent::Resize { width, height }),
        Event::Paste(text) => Some(ChatInputEvent::Paste(text)),
        Event::FocusGained => Some(ChatInputEvent::FocusGained),
        Event::FocusLost => Some(ChatInputEvent::FocusLost),
    }
}
