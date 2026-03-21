use super::messaging::ApiResult;

#[async_trait::async_trait]
pub trait DocumentsApi: Send + Sync {
    type Document: Send + Sync + Clone + std::fmt::Debug;
    type DocumentContent: Send + Sync + Clone + std::fmt::Debug;

    async fn create_document(
        &self,
        title: &str,
        content: Option<&str>,
    ) -> ApiResult<Self::Document>;

    async fn read_document(&self, doc_id: &str) -> ApiResult<Self::DocumentContent>;

    async fn append_to_document(&self, doc_id: &str, content: &str) -> ApiResult<()>;
}
