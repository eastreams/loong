use super::messaging::ApiResult;

#[derive(Clone, Debug)]
pub struct TimeRange {
    pub start_timestamp: i64,
    pub end_timestamp: i64,
}

#[async_trait::async_trait]
pub trait CalendarApi: Send + Sync {
    type Calendar: Send + Sync + Clone + std::fmt::Debug;
    type CalendarList: Send + Sync + Clone + std::fmt::Debug;
    type FreeBusyResult: Send + Sync + Clone + std::fmt::Debug;

    async fn list_calendars(&self) -> ApiResult<Self::CalendarList>;

    async fn get_primary_calendar(&self) -> ApiResult<Self::Calendar>;

    async fn query_freebusy(
        &self,
        time_range: &TimeRange,
        participants: &[String],
    ) -> ApiResult<Self::FreeBusyResult>;
}
