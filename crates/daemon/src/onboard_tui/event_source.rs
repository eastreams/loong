use std::collections::VecDeque;

use crossterm::event::{self, Event};

// These types are consumed by later tasks (runner, layout, widgets).
#[allow(dead_code)]
pub(crate) trait OnboardEventSource {
    fn next_event(&mut self) -> std::io::Result<Event>;
}

#[allow(dead_code)]
pub(crate) struct CrosstermEventSource;

impl OnboardEventSource for CrosstermEventSource {
    fn next_event(&mut self) -> std::io::Result<Event> {
        event::read()
    }
}

#[allow(dead_code)]
pub(crate) struct ScriptedEventSource {
    events: VecDeque<Event>,
}

impl ScriptedEventSource {
    #[cfg(test)]
    pub fn new(events: Vec<Event>) -> Self {
        Self {
            events: events.into(),
        }
    }
}

impl OnboardEventSource for ScriptedEventSource {
    fn next_event(&mut self) -> std::io::Result<Event> {
        self.events.pop_front().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "no more scripted events")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn scripted_source_yields_events_in_order() {
        let events = vec![
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ];
        let mut source = ScriptedEventSource::new(events.clone());
        assert_eq!(source.next_event().unwrap(), events[0]);
        assert_eq!(source.next_event().unwrap(), events[1]);
    }

    #[test]
    fn scripted_source_returns_error_when_exhausted() {
        let mut source = ScriptedEventSource::new(vec![]);
        assert!(source.next_event().is_err());
    }
}
