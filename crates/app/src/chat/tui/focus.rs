#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusLayer {
    Composer,
    Help,
    ToolInspector,
    ClarifyDialog,
}

#[derive(Debug, Clone)]
pub(crate) struct FocusStack {
    layers: Vec<FocusLayer>,
}

impl FocusStack {
    pub(crate) fn new() -> Self {
        Self {
            layers: vec![FocusLayer::Composer],
        }
    }

    pub(crate) fn top(&self) -> FocusLayer {
        self.layers.last().copied().unwrap_or(FocusLayer::Composer)
    }

    pub(crate) fn push(&mut self, layer: FocusLayer) {
        self.layers.push(layer);
    }

    pub(crate) fn pop(&mut self) {
        if self.layers.len() > 1 {
            self.layers.pop();
        }
    }

    pub(crate) fn has(&self, layer: FocusLayer) -> bool {
        self.layers.contains(&layer)
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
}
