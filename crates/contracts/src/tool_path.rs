use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use thiserror::Error;

/// Stable identity of one tool across registration, policy, audit, and hosts.
///
/// Segments are case-sensitive opaque identifiers, not filesystem components:
/// `.` and `..` have no navigation semantics, and no Unicode normalization is
/// applied. The leading-slash text form is canonical, while a tool plane remains
/// free to choose its own index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolPath {
    segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ToolPathError {
    #[error("tool path must contain at least one segment")]
    Empty,
    #[error("tool path segment {index} is empty")]
    EmptySegment { index: usize },
    #[error("tool path segment {index} contains the `/` separator")]
    SegmentContainsSeparator { index: usize },
    #[error("tool path segment {index} contains a control character")]
    SegmentContainsControl { index: usize },
    #[error("tool path text must start with `/`")]
    MissingLeadingSlash,
}

impl ToolPath {
    /// Construct a path from explicit segment boundaries.
    ///
    /// This never splits dotted names or interprets segment contents as another
    /// path syntax.
    pub fn new(
        segments: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, ToolPathError> {
        let segments = segments.into_iter().map(Into::into).collect::<Vec<_>>();
        if segments.is_empty() {
            return Err(ToolPathError::Empty);
        }
        for (index, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                return Err(ToolPathError::EmptySegment { index });
            }
            if segment.contains('/') {
                return Err(ToolPathError::SegmentContainsSeparator { index });
            }
            if segment.chars().any(char::is_control) {
                return Err(ToolPathError::SegmentContainsControl { index });
            }
        }
        Ok(Self { segments })
    }

    #[must_use]
    pub fn segments(&self) -> &[String] {
        self.segments.as_slice()
    }
}

impl fmt::Display for ToolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for segment in &self.segments {
            formatter.write_str("/")?;
            formatter.write_str(segment)?;
        }
        Ok(())
    }
}

impl FromStr for ToolPath {
    type Err = ToolPathError;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let Some(path) = path.strip_prefix('/') else {
            return Err(ToolPathError::MissingLeadingSlash);
        };
        if path.is_empty() {
            return Err(ToolPathError::Empty);
        }
        Self::new(path.split('/'))
    }
}

impl Serialize for ToolPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ToolPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests;
