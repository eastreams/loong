use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use super::error::ApiResult;
use super::messaging::Pagination;

/// Calendar event/appointment
#[derive(Debug, Clone)]
pub struct CalendarEvent {
    /// Event ID
    pub id: String,
    /// Event title
    pub title: String,
    /// Event description
    pub description: Option<String>,
    /// Event start time
    pub start_time: DateTime<Utc>,
    /// Event end time
    pub end_time: DateTime<Utc>,
    /// Event organizer ID
    pub organizer_id: String,
    /// Attendee IDs
    pub attendee_ids: Vec<String>,
    /// Location (physical or virtual)
    pub location: Option<String>,
    /// Whether this is a recurring event
    pub is_recurring: bool,
    /// Platform-specific metadata
    pub metadata: Option<serde_json::Value>,
}

/// Calendar availability/freebusy information
#[derive(Debug, Clone)]
pub struct Availability {
    /// User ID
    pub user_id: String,
    /// List of busy periods
    pub busy_periods: Vec<TimeRange>,
}

/// Time range for availability queries
#[derive(Debug, Clone)]
pub struct TimeRange {
    /// Range start
    pub start: DateTime<Utc>,
    /// Range end
    pub end: DateTime<Utc>,
}

/// Trait for calendar management capabilities
///
/// Implement this trait for channels that support calendar operations
/// (like Google Calendar, Outlook, Feishu Calendar, etc.)
#[async_trait]
pub trait CalendarApi: Send + Sync {
    /// Create a calendar event
    async fn create_event(
        &self,
        title: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        attendee_ids: Option<&[&str]>,
    ) -> ApiResult<CalendarEvent>;

    /// Get an event by ID
    async fn get_event(&self, id: &str) -> ApiResult<Option<CalendarEvent>>;

    /// Update an existing event
    async fn update_event(
        &self,
        id: &str,
        title: Option<&str>,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> ApiResult<CalendarEvent>;

    /// Delete an event
    async fn delete_event(&self, id: &str) -> ApiResult<()>;

    /// List events in a time range
    async fn list_events(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        pagination: Option<Pagination>,
    ) -> ApiResult<Vec<CalendarEvent>>;

    /// Query availability/freebusy for users
    async fn query_availability(
        &self,
        user_ids: &[&str],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> ApiResult<Vec<Availability>>;

    /// Find available time slots for a meeting
    async fn find_available_slots(
        &self,
        user_ids: &[&str],
        duration: Duration,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> ApiResult<Vec<TimeRange>>;
}
