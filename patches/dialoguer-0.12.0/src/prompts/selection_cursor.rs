pub(crate) fn advance_selection_with_clamp(
    current: Option<usize>,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    match current {
        None => Some(0),
        Some(index) => Some(index.saturating_add(1).min(len.saturating_sub(1))),
    }
}

pub(crate) fn retreat_selection_with_clamp(
    current: Option<usize>,
    len: usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    match current {
        None => Some(len.saturating_sub(1)),
        Some(index) => Some(index.saturating_sub(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_selection_with_clamp_stops_at_last_item() {
        assert_eq!(advance_selection_with_clamp(None, 4), Some(0));
        assert_eq!(advance_selection_with_clamp(Some(0), 4), Some(1));
        assert_eq!(advance_selection_with_clamp(Some(2), 4), Some(3));
        assert_eq!(advance_selection_with_clamp(Some(3), 4), Some(3));
    }

    #[test]
    fn retreat_selection_with_clamp_stops_at_first_item() {
        assert_eq!(retreat_selection_with_clamp(None, 4), Some(3));
        assert_eq!(retreat_selection_with_clamp(Some(3), 4), Some(2));
        assert_eq!(retreat_selection_with_clamp(Some(1), 4), Some(0));
        assert_eq!(retreat_selection_with_clamp(Some(0), 4), Some(0));
    }
}
