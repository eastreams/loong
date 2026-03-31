use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::error::ApiResult;
use super::messaging::Pagination;

/// Document content types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentContent {
    /// Plain text content
    Text(String),
    /// Markdown content
    Markdown(String),
    /// Binary content (for files)
    Binary(Vec<u8>),
    /// Structured JSON content
    Json(serde_json::Value),
}

/// Document type enumeration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentType {
    /// Docx document (Feishu, Office 365, etc.)
    Docx,
}

/// Document metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Platform-specific document ID
    pub id: String,
    /// Document title/name
    pub title: Option<String>,
    /// Document owner/creator ID (None if not available)
    pub owner_id: Option<String>,
    /// Creation timestamp (None if not available)
    pub created_at: Option<DateTime<Utc>>,
    /// Last modification timestamp (None if not available)
    pub updated_at: Option<DateTime<Utc>>,
    /// Document content (optional, for list operations)
    pub content: Option<DocumentContent>,
    /// Document type/format
    pub doc_type: DocumentType,
    /// Platform-specific metadata
    pub metadata: Option<serde_json::Value>,
}

/// Trait for document management capabilities
///
/// Implement this trait for channels that support document creation,
/// editing, and management (like Feishu Docs, Notion, etc.)
#[async_trait]
pub trait DocumentsApi: Send + Sync {
    /// Create a new document
    ///
    /// # Arguments
    /// * `title` - Document title
    /// * `content` - Initial content
    /// * `parent_id` - Optional parent folder/container ID
    async fn create_document(
        &self,
        title: &str,
        content: Option<&DocumentContent>,
        parent_id: Option<&str>,
    ) -> ApiResult<Document>;

    /// Get a document by ID
    async fn get_document(&self, id: &str) -> ApiResult<Option<Document>>;

    /// Get document content only
    async fn get_document_content(&self, id: &str) -> ApiResult<Option<DocumentContent>>;

    /// Update document content
    async fn update_document(&self, id: &str, content: &DocumentContent) -> ApiResult<()>;

    /// Append content to an existing document
    async fn append_to_document(&self, id: &str, content: &DocumentContent) -> ApiResult<()>;

    /// List documents in a container
    async fn list_documents(
        &self,
        parent_id: Option<&str>,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>>;

    /// Search documents
    async fn search_documents(
        &self,
        query: &str,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>>;

    /// Delete a document
    async fn delete_document(&self, id: &str) -> ApiResult<()>;

    /// Move document to a different parent
    async fn move_document(&self, id: &str, new_parent_id: &str) -> ApiResult<Document>;
}
