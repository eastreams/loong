use std::fmt;

/// Standardized error type for all Channel API operations
///
/// This error type abstracts platform-specific errors into a uniform
/// interface that tools can handle without knowing implementation details.
#[derive(Debug, Clone)]
pub enum ApiError {
    /// Authentication or authorization failure
    Auth(String),
    /// Resource not found (message, document, user, etc.)
    NotFound(String),
    /// Rate limit exceeded
    RateLimited { retry_after_secs: u64 },
    /// Invalid request parameters
    InvalidRequest(String),
    /// Network or transport error
    Network(String),
    /// Server-side error
    Server(String),
    /// Operation not supported by this platform
    NotSupported(String),
    /// Platform-specific error wrapped for uniformity
    Platform {
        platform: String,
        code: Option<String>,
        message: String,
    },
    /// Generic catch-all for unexpected errors
    Other(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth(msg) => write!(f, "authentication failed: {msg}"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::RateLimited { retry_after_secs } => {
                write!(f, "rate limited: retry after {retry_after_secs}s")
            }
            Self::InvalidRequest(msg) => write!(f, "invalid request: {msg}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
            Self::NotSupported(msg) => write!(f, "operation not supported: {msg}"),
            Self::Platform {
                platform,
                code,
                message,
            } => {
                if let Some(code) = code {
                    write!(f, "platform error [{platform}:{code}]: {message}")
                } else {
                    write!(f, "platform error [{platform}]: {message}")
                }
            }
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Create a platform-specific error
    pub fn platform(platform: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Platform {
            platform: platform.into(),
            code: None,
            message: message.into(),
        }
    }

    /// Create a platform-specific error with code
    pub fn platform_with_code(
        platform: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Platform {
            platform: platform.into(),
            code: Some(code.into()),
            message: message.into(),
        }
    }

    /// Check if this error indicates a transient failure that might succeed on retry
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Network(_) | Self::Server(_)
        )
    }

    /// Check if this error indicates the operation is not supported by the platform
    pub fn is_not_supported(&self) -> bool {
        matches!(self, Self::NotSupported(_))
    }
}

/// Result type alias for API operations
pub type ApiResult<T> = Result<T, ApiError>;
