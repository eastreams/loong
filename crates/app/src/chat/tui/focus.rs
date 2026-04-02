#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusLayer {
    Composer,
    Transcript,
    Help,
    ToolInspector,
    ClarifyDialog,
}

#[derive(Debug, Clone)]
pub(crate) struct FocusStack {
    base: FocusLayer,
    overlays: Vec<FocusLayer>,
}

impl FocusStack {
    pub(crate) fn new() -> Self {
        Self {
            base: FocusLayer::Composer,
            overlays: Vec::new(),
        }
    }

    pub(crate) fn top(&self) -> FocusLayer {
        self.overlays.last().copied().unwrap_or(self.base)
    }

    pub(crate) fn push(&mut self, layer: FocusLayer) {
        let is_base_layer = matches!(layer, FocusLayer::Composer | FocusLayer::Transcript);
        if is_base_layer {
            self.base = layer;
            return;
        }

        self.overlays.push(layer);
    }

    pub(crate) fn pop(&mut self) {
        let _ = self.overlays.pop();
    }

    pub(crate) fn has(&self, layer: FocusLayer) -> bool {
        if self.base == layer {
            return true;
        }

        self.overlays.contains(&layer)
    }

    pub(crate) fn focus_transcript(&mut self) {
        self.base = FocusLayer::Transcript;
    }

    pub(crate) fn focus_composer(&mut self) {
        self.base = FocusLayer::Composer;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_defaults_to_composer() {
        let stack = FocusStack::new();
        assert_eq!(stack.top(), FocusLayer::Composer);
    }

    #[test]
    fn push_and_pop() {
        let mut stack = FocusStack::new();
        stack.push(FocusLayer::Help);
        assert_eq!(stack.top(), FocusLayer::Help);
        assert!(stack.has(FocusLayer::Help));
        stack.pop();
        assert_eq!(stack.top(), FocusLayer::Composer);
        assert!(!stack.has(FocusLayer::Help));
    }

    #[test]
    fn pop_never_removes_composer() {
        let mut stack = FocusStack::new();
        stack.pop();
        assert_eq!(stack.top(), FocusLayer::Composer);
    }

    #[test]
    fn stacking_order() {
        let mut stack = FocusStack::new();
        stack.push(FocusLayer::Help);
        stack.push(FocusLayer::ClarifyDialog);
        assert_eq!(stack.top(), FocusLayer::ClarifyDialog);
        stack.pop();
        assert_eq!(stack.top(), FocusLayer::Help);
    }

    #[test]
    fn primary_focus_can_switch_between_composer_and_transcript() {
        let mut stack = FocusStack::new();

        stack.focus_transcript();

        assert_eq!(stack.top(), FocusLayer::Transcript);
        assert!(stack.has(FocusLayer::Transcript));

        stack.focus_composer();

        assert_eq!(stack.top(), FocusLayer::Composer);
        assert!(!stack.has(FocusLayer::Transcript));
    }
}
