use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;

use super::frozen_result::FrozenResult;
use super::store::{self, SessionStoreConfig, SessionTranscriptTurn};
use crate::config::ToolConsentMode;
use crate::search_text::{build_search_index_text, normalize_search_text};
use crate::tools::runtime_config::ToolRuntimeNarrowing;

pub(crate) const SESSION_TRAJECTORY_TRANSCRIPT_PAGE_SIZE: usize = 200;
pub(crate) const ACTIVE_SESSION_HEAD_NAME: &str = "active";

#[cfg(feature = "memory-sqlite")]
mod persistence;

#[cfg(feature = "memory-sqlite")]
mod projections;

#[cfg(feature = "memory-sqlite")]
mod records;

#[cfg(feature = "memory-sqlite")]
mod tree;

#[cfg(feature = "memory-sqlite")]
pub use self::records::*;

#[cfg(test)]
mod repository_tests;

pub struct SessionRepository {
    db_path: PathBuf,
    max_total_artifacts: Option<usize>,
}

impl SessionRepository {
    pub fn new(config: &SessionStoreConfig) -> Result<Self, String> {
        let db_path = store::ensure_session_store_ready(config.sqlite_path.clone(), config)?;
        Ok(Self {
            db_path,
            max_total_artifacts: None,
        })
    }

    pub fn with_max_total_artifacts(mut self, max_total_artifacts: Option<usize>) -> Self {
        self.max_total_artifacts = max_total_artifacts;
        self
    }
}
