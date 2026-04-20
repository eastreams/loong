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

/// Trait for document creation capabilities.
///
/// Implement this trait for channels that can create a new document from a
/// title, optional initial content, and optional parent/container ID, and
/// return the created document metadata.
#[async_trait]
pub trait DocumentCreateApi: Send + Sync {
    /// Create a new document and return the resulting document metadata.
    async fn create_document(
        &self,
        title: &str,
        content: Option<&DocumentContent>,
        parent_id: Option<&str>,
    ) -> ApiResult<Document>;
}

/// Trait for document read capabilities.
#[async_trait]
pub trait DocumentReadApi: Send + Sync {
    /// Get a document by ID.
    async fn get_document(&self, id: &str) -> ApiResult<Option<Document>>;

    /// Get document content only.
    async fn get_document_content(&self, id: &str) -> ApiResult<Option<DocumentContent>>;
}

/// Trait for document append capabilities.
#[async_trait]
pub trait DocumentAppendApi: Send + Sync {
    /// Append content to an existing document.
    async fn append_to_document(&self, id: &str, content: &DocumentContent) -> ApiResult<()>;
}

/// Trait for document search and listing capabilities.
#[async_trait]
pub trait DocumentSearchApi: Send + Sync {
    /// List documents in a container.
    async fn list_documents(
        &self,
        parent_id: Option<&str>,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>>;

    /// Search documents.
    async fn search_documents(
        &self,
        query: &str,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<Document>>;
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

#[cfg(test)]
mod tests {
    fn assert_document_create_api<T: super::DocumentCreateApi>() {}
    fn assert_document_read_api<T: super::DocumentReadApi>() {}
    fn assert_document_append_api<T: super::DocumentAppendApi>() {}
    fn assert_document_search_api<T: super::DocumentSearchApi>() {}

    #[test]
    fn narrow_trait_assertions_compile() {
        struct TestDocumentsApi;

        #[async_trait::async_trait]
        impl super::DocumentCreateApi for TestDocumentsApi {
            async fn create_document(
                &self,
                _title: &str,
                _content: Option<&super::DocumentContent>,
                _parent_id: Option<&str>,
            ) -> super::ApiResult<super::Document> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::DocumentReadApi for TestDocumentsApi {
            async fn get_document(&self, _id: &str) -> super::ApiResult<Option<super::Document>> {
                panic!("compile-time assertion only")
            }

            async fn get_document_content(
                &self,
                _id: &str,
            ) -> super::ApiResult<Option<super::DocumentContent>> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::DocumentAppendApi for TestDocumentsApi {
            async fn append_to_document(
                &self,
                _id: &str,
                _content: &super::DocumentContent,
            ) -> super::ApiResult<()> {
                panic!("compile-time assertion only")
            }
        }

        #[async_trait::async_trait]
        impl super::DocumentSearchApi for TestDocumentsApi {
            async fn list_documents(
                &self,
                _parent_id: Option<&str>,
                _pagination: Option<crate::channel::traits::Pagination>,
            ) -> super::ApiResult<Vec<super::Document>> {
                panic!("compile-time assertion only")
            }

            async fn search_documents(
                &self,
                _query: &str,
                _pagination: Option<crate::channel::traits::Pagination>,
            ) -> super::ApiResult<Vec<super::Document>> {
                panic!("compile-time assertion only")
            }
        }

        assert_document_create_api::<TestDocumentsApi>();
        assert_document_read_api::<TestDocumentsApi>();
        assert_document_append_api::<TestDocumentsApi>();
        assert_document_search_api::<TestDocumentsApi>();
    }
}
