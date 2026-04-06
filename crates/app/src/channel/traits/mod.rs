//! Channel API Traits - Platform-agnostic abstraction layer for messaging and document operations
//!
//! This module defines traits that platform adapters implement to provide
//! unified API access across different channels (Feishu, Telegram, Matrix, etc.)
//!
//! ## Architecture
//!
//! ```text
//! tools/ (generic handlers)
//!     ↓ uses dyn Trait
//! channel/traits/ (this module)
//!     - MessageSendApi / MessageQueryApi / ...
//!     - MessagingApi / DocumentsApi compatibility surfaces
//!     - CalendarApi
//!     ↓ implemented by
//! channel/{platform}/ (platform implementations)
//!     - Transport layer
//!     - API capabilities
//! ```
//!
//! ## Design Principles
//!
//! 1. **Capability-based**: Traits represent capabilities, not platforms
//! 2. **Optional features**: Each trait is optional - platforms implement what they support
//! 3. **Async-first**: All operations are async for I/O-bound channel operations
//! 4. **Error uniformity**: Standardized error types across all platforms
//! 5. **Send + Sync**: All trait objects are thread-safe for runtime use
//! 6. **Routing reuse**: Messaging traits reuse normalized channel targets and sessions
//!    instead of introducing a second routing vocabulary
//! 7. **Additive refinement**: Narrow capability traits coexist with wider compatibility traits

pub mod calendar;
pub mod documents;
pub mod error;
pub mod messaging;

// Re-export commonly used types
pub use calendar::{Availability, CalendarApi, CalendarEvent, TimeRange};
pub use documents::{
    Document, DocumentAppendApi, DocumentContent, DocumentCreateApi, DocumentReadApi,
    DocumentSearchApi, DocumentsApi,
};
pub use error::{ApiError, ApiResult};
pub use messaging::{
    Message, MessageContent, MessageDeleteApi, MessageEditApi, MessageQueryApi, MessageSendApi,
    MessagingApi, Pagination, RichMessagingApi, SendOptions,
};
