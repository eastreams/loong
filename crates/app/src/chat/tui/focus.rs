#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusTarget {
    Composer,
    Drawer,
}

impl FocusTarget {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Composer => "composer",
            Self::Drawer => "drawer",
        }
    }
}
