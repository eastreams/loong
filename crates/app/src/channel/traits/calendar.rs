use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

use super::error::{ApiError, ApiResult};
use super::messaging::Pagination;

/// Calendar event/appointment
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Availability {
    /// User ID
    pub user_id: String,
    /// List of busy periods
    pub busy_periods: Vec<TimeRange>,
}

/// Time range for availability queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRange {
    /// Range start
    pub start: DateTime<Utc>,
    /// Range end
    pub end: DateTime<Utc>,
}

impl TimeRange {
    /// Create a validated time range with a non-decreasing boundary order.
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> ApiResult<Self> {
        let end_is_before_start = end < start;

        if end_is_before_start {
            let message = format!("time range end {end} is before start {start}");

            return Err(ApiError::InvalidRequest(message));
        }

        let time_range = Self { start, end };

        Ok(time_range)
    }

    /// Create a validated time range from a start instant and duration.
    pub fn from_duration(start: DateTime<Utc>, duration: Duration) -> ApiResult<Self> {
        let zero_duration = Duration::zero();
        let duration_is_negative = duration < zero_duration;

        if duration_is_negative {
            let message = format!("time range duration {duration:?} must not be negative");

            return Err(ApiError::InvalidRequest(message));
        }

        let end = start + duration;

        Self::new(start, end)
    }
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

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{ApiError, TimeRange};

    #[test]
    fn time_range_new_rejects_inverted_bounds() {
        let start = Utc
            .with_ymd_and_hms(2026, 3, 27, 10, 0, 0)
            .single()
            .expect("valid start");
        let end = Utc
            .with_ymd_and_hms(2026, 3, 27, 9, 0, 0)
            .single()
            .expect("valid end");
        let error = TimeRange::new(start, end).expect_err("inverted range should fail");

        let ApiError::InvalidRequest(message) = error else {
            panic!("expected invalid request error");
        };

        assert!(message.contains("time range end"));
        assert!(message.contains("is before start"));
    }

    #[test]
    fn time_range_from_duration_builds_valid_end_time() {
        let start = Utc
            .with_ymd_and_hms(2026, 3, 27, 10, 0, 0)
            .single()
            .expect("valid start");
        let duration = Duration::minutes(45);
        let time_range = TimeRange::from_duration(start, duration).expect("range should be valid");
        let expected_end = Utc
            .with_ymd_and_hms(2026, 3, 27, 10, 45, 0)
            .single()
            .expect("valid expected end");

        assert_eq!(time_range.start, start);
        assert_eq!(time_range.end, expected_end);
    }
}
