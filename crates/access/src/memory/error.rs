use loong_core::error::PolicyGrantError;

#[derive(Debug, thiserror::Error)]
pub enum MemoryAccessError {
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error(transparent)]
    Backend(#[from] MemoryBackendError),
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryBackendError {
    #[error("memory operation `{operation}` failed: {source}")]
    Execution {
        operation: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("memory operation `{operation}` returned malformed output: {source}")]
    MalformedOutput {
        operation: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}
