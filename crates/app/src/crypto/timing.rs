use subtle::ConstantTimeEq;

/// Compares two secret byte sequences in constant time when lengths match.
///
/// Returns `false` immediately for length mismatches. That is safe for the
/// current call sites because they compare fixed-length tokens or fixed-length
/// hex digests; callers that need to hide length differences must normalize
/// inputs before calling this helper.
#[must_use]
pub fn timing_safe_eq(expected: &[u8], actual: &[u8]) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected.ct_eq(actual).into()
}

#[cfg(test)]
mod tests {
    use super::timing_safe_eq;

    #[test]
    fn timing_safe_eq_accepts_equal_inputs() {
        assert!(timing_safe_eq(b"secret", b"secret"));
    }

    #[test]
    fn timing_safe_eq_rejects_unequal_inputs() {
        assert!(!timing_safe_eq(b"secret", b"public"));
    }

    #[test]
    fn timing_safe_eq_rejects_length_mismatch() {
        assert!(!timing_safe_eq(b"secret", b"secret!"));
    }
}
