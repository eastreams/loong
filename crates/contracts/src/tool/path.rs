use alloc::{string::String, vec::Vec};
use core::{
    ops::{Index, IndexMut},
    slice::SliceIndex,
};
use thiserror::Error;

/// Why a tool path segment could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolPathSegmentError {
    #[error("tool path segment contains reserved character {0:?}")]
    ReservedCharacter(char),
}

/// A valid part of tool path, which does not contain '.' or '/'
#[derive(Debug, Clone, Hash)]
pub struct ToolPathSegment(String);

impl TryFrom<String> for ToolPathSegment {
    type Error = ToolPathSegmentError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if let Some(character) = value
            .chars()
            .find(|&character| matches!(character, '.' | '/'))
        {
            Err(ToolPathSegmentError::ReservedCharacter(character))
        } else {
            Ok(Self(value))
        }
    }
}

impl From<ToolPathSegment> for String {
    fn from(value: ToolPathSegment) -> Self {
        value.0
    }
}

impl AsRef<str> for ToolPathSegment {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl core::fmt::Display for ToolPathSegment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Hash)]
pub struct ToolPath {
    parts: Vec<ToolPathSegment>,
}

impl Extend<ToolPathSegment> for ToolPath {
    fn extend<T: IntoIterator<Item = ToolPathSegment>>(&mut self, iter: T) {
        self.parts.extend(iter);
    }
}

impl FromIterator<ToolPathSegment> for ToolPath {
    fn from_iter<T: IntoIterator<Item = ToolPathSegment>>(iter: T) -> Self {
        Self {
            parts: iter.into_iter().collect(),
        }
    }
}

impl From<Vec<ToolPathSegment>> for ToolPath {
    fn from(value: Vec<ToolPathSegment>) -> Self {
        Self { parts: value }
    }
}

impl<I> Index<I> for ToolPath
where
    I: SliceIndex<[ToolPathSegment], Output = ToolPathSegment>,
{
    type Output = ToolPathSegment;
    fn index(&self, index: I) -> &Self::Output {
        self.parts.index(index)
    }
}

impl<I> IndexMut<I> for ToolPath
where
    I: SliceIndex<[ToolPathSegment], Output = ToolPathSegment>,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.parts.index_mut(index)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{borrow::ToOwned, string::ToString};

    use super::{ToolPathSegment, ToolPathSegmentError};

    #[test]
    fn segment_reports_the_reserved_character() {
        for (value, reserved) in [("namespace.tool", '.'), ("namespace/tool", '/')] {
            let result = ToolPathSegment::try_from(value.to_owned());

            assert!(matches!(
                result,
                Err(ToolPathSegmentError::ReservedCharacter(character))
                    if character == reserved
            ));
        }
    }

    #[test]
    fn segment_error_has_actionable_context() {
        let error = match ToolPathSegment::try_from("namespace.tool".to_owned()) {
            Ok(_) => panic!("reserved character should be rejected"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "tool path segment contains reserved character '.'"
        );
    }
}
