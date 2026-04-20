use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSecurityTier {
    Restricted,
    Balanced,
    Trusted,
}

impl ExecutionSecurityTier {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Restricted => "restricted",
            Self::Balanced => "balanced",
            Self::Trusted => "trusted",
        }
    }
}

impl fmt::Display for ExecutionSecurityTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
