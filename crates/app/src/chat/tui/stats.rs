use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, Duration, NaiveDate, Utc};

use crate::config::LoongClawConfig;
use crate::conversation::analytics::{TURN_USAGE_EVENT_NAME, parse_conversation_event};
#[cfg(feature = "memory-sqlite")]
use crate::memory::ConversationTurn;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalRequestStatus, SessionKind, SessionRepository, SessionState, SessionSummaryRecord,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatsTab {
    Overview,
    Models,
    Sessions,
}

impl StatsTab {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Models => "Models",
            Self::Sessions => "Sessions",
        }
    }

    pub(super) fn parse_token(token: &str) -> Option<Self> {
        let normalized = token.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "overview" => Some(Self::Overview),
            "models" => Some(Self::Models),
            "sessions" => Some(Self::Sessions),
            _ => None,
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::Overview => Self::Models,
            Self::Models => Self::Sessions,
            Self::Sessions => Self::Overview,
        }
    }

    pub(super) fn previous(self) -> Self {
        match self {
            Self::Overview => Self::Sessions,
            Self::Models => Self::Overview,
            Self::Sessions => Self::Models,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatsDateRange {
    All,
    Last7Days,
    Last30Days,
}

impl StatsDateRange {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "All time",
            Self::Last7Days => "Last 7 days",
            Self::Last30Days => "Last 30 days",
        }
    }

    pub(super) fn parse_token(token: &str) -> Option<Self> {
        let normalized = token.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "all" | "all-time" | "all_time" => Some(Self::All),
            "7d" | "last-7" | "last7" | "last-7-days" => Some(Self::Last7Days),
            "30d" | "last-30" | "last30" | "last-30-days" => Some(Self::Last30Days),
            _ => None,
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::All => Self::Last7Days,
            Self::Last7Days => Self::Last30Days,
            Self::Last30Days => Self::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatsOpenOptions {
    pub(super) tab: StatsTab,
    pub(super) date_range: StatsDateRange,
}

impl Default for StatsOpenOptions {
    fn default() -> Self {
        Self {
            tab: StatsTab::Overview,
            date_range: StatsDateRange::All,
        }
    }
}

pub(super) fn parse_stats_open_options(args: &str) -> Result<StatsOpenOptions, String> {
    let trimmed_args = args.trim();
    let mut options = StatsOpenOptions::default();

    if trimmed_args.is_empty() {
        return Ok(options);
    }

    for token in trimmed_args.split_whitespace() {
        let parsed_tab = StatsTab::parse_token(token);
        if let Some(tab) = parsed_tab {
            options.tab = tab;
            continue;
        }

        let parsed_range = StatsDateRange::parse_token(token);
        if let Some(date_range) = parsed_range {
            options.date_range = date_range;
            continue;
        }

        return Err("usage: `/stats [overview|models] [all|7d|30d]`".to_owned());
    }

    Ok(options)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionDurationStat {
    pub(super) session_id: String,
    pub(super) label: Option<String>,
    pub(super) duration_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatsSessionRow {
    pub(super) session_id: String,
    pub(super) label: Option<String>,
    pub(super) kind: String,
    pub(super) state: String,
    pub(super) turn_count: usize,
    pub(super) duration_seconds: u64,
    pub(super) last_activity_date: Option<NaiveDate>,
    pub(super) current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelTokenTotal {
    pub(super) model: String,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DailyTokenPoint {
    pub(super) date: NaiveDate,
    pub(super) total_input_tokens: u64,
    pub(super) total_output_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) model_tokens: BTreeMap<String, ModelTokenAccumulator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatsSnapshot {
    pub(super) visible_sessions: usize,
    pub(super) root_sessions: usize,
    pub(super) delegate_sessions: usize,
    pub(super) running_delegate_sessions: usize,
    pub(super) pending_approvals: usize,
    pub(super) usage_event_count: usize,
    pub(super) first_activity_date: Option<NaiveDate>,
    pub(super) last_activity_date: Option<NaiveDate>,
    pub(super) longest_session: Option<SessionDurationStat>,
    pub(super) session_rows: Vec<StatsSessionRow>,
    pub(super) active_dates: Vec<NaiveDate>,
    pub(super) daily_points: Vec<DailyTokenPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StatsRangeView {
    pub(super) date_range: StatsDateRange,
    pub(super) total_input_tokens: u64,
    pub(super) total_output_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) active_days: usize,
    pub(super) current_streak: usize,
    pub(super) longest_streak: usize,
    pub(super) top_model: Option<ModelTokenTotal>,
    pub(super) model_totals: Vec<ModelTokenTotal>,
    pub(super) session_rows: Vec<StatsSessionRow>,
    pub(super) daily_points: Vec<DailyTokenPoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct StatsChartSeries {
    pub(super) label: String,
    pub(super) points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct StatsChartView {
    pub(super) start_label: String,
    pub(super) middle_label: String,
    pub(super) end_label: String,
    pub(super) max_tokens: u64,
    pub(super) series: Vec<StatsChartSeries>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ModelTokenAccumulator {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
}

impl ModelTokenAccumulator {
    pub(super) fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DailyTokenAccumulator {
    models: BTreeMap<String, ModelTokenAccumulator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnUsageRecord {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
}

impl StatsSnapshot {
    fn date_bounds(&self, date_range: StatsDateRange) -> (NaiveDate, NaiveDate) {
        let today = Utc::now().date_naive();
        let default_end_date = self.last_activity_date.unwrap_or(today);
        let mut start_date = self.first_activity_date.unwrap_or(default_end_date);
        let end_date = match date_range {
            StatsDateRange::All => default_end_date,
            StatsDateRange::Last7Days => today,
            StatsDateRange::Last30Days => today,
        };

        if matches!(date_range, StatsDateRange::Last7Days) {
            let range_start = today - Duration::days(6);
            if range_start > start_date {
                start_date = range_start;
            }
        }

        if matches!(date_range, StatsDateRange::Last30Days) {
            let range_start = today - Duration::days(29);
            if range_start > start_date {
                start_date = range_start;
            }
        }

        if start_date > end_date {
            start_date = end_date;
        }

        (start_date, end_date)
    }

    pub(super) fn range_view(&self, date_range: StatsDateRange) -> StatsRangeView {
        let (start_date, end_date) = self.date_bounds(date_range);

        let mut by_date: BTreeMap<NaiveDate, DailyTokenPoint> = BTreeMap::new();
        for point in &self.daily_points {
            by_date.insert(point.date, point.clone());
        }

        let mut filtered_points = Vec::new();
        let mut total_input_tokens = 0_u64;
        let mut total_output_tokens = 0_u64;
        let mut model_accumulators: BTreeMap<String, ModelTokenAccumulator> = BTreeMap::new();
        let mut active_dates = Vec::new();
        let mut current_date = start_date;

        while current_date <= end_date {
            let maybe_point = by_date.get(&current_date).cloned();
            let point = maybe_point.unwrap_or_else(|| DailyTokenPoint {
                date: current_date,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_tokens: 0,
                model_tokens: BTreeMap::new(),
            });

            if point.total_tokens > 0 {
                active_dates.push(current_date);
            }

            for (model, totals) in &point.model_tokens {
                let model_entry = model_accumulators.entry(model.clone()).or_default();
                model_entry.input_tokens =
                    model_entry.input_tokens.saturating_add(totals.input_tokens);
                model_entry.output_tokens = model_entry
                    .output_tokens
                    .saturating_add(totals.output_tokens);
            }

            total_input_tokens = total_input_tokens.saturating_add(point.total_input_tokens);
            total_output_tokens = total_output_tokens.saturating_add(point.total_output_tokens);
            filtered_points.push(point);
            current_date += Duration::days(1);
        }

        let mut model_totals = model_accumulators
            .into_iter()
            .map(|(model, totals)| {
                let total_tokens = totals.total_tokens();
                ModelTokenTotal {
                    model,
                    input_tokens: totals.input_tokens,
                    output_tokens: totals.output_tokens,
                    total_tokens,
                }
            })
            .collect::<Vec<_>>();

        model_totals.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.model.cmp(&right.model))
        });

        let total_tokens = total_input_tokens.saturating_add(total_output_tokens);
        let current_streak = current_streak(active_dates.as_slice(), end_date);
        let longest_streak = longest_streak(active_dates.as_slice());
        let top_model = model_totals.first().cloned();
        let session_rows = self
            .session_rows
            .iter()
            .filter(|session_row| {
                let session_date = session_row.last_activity_date;
                let Some(session_date) = session_date else {
                    return matches!(date_range, StatsDateRange::All);
                };

                session_date >= start_date && session_date <= end_date
            })
            .cloned()
            .collect::<Vec<_>>();

        StatsRangeView {
            date_range,
            total_input_tokens,
            total_output_tokens,
            total_tokens,
            active_days: active_dates.len(),
            current_streak,
            longest_streak,
            top_model,
            model_totals,
            session_rows,
            daily_points: filtered_points,
        }
    }
}

impl StatsRangeView {
    pub(super) fn chart_view(&self, limit: usize) -> Option<StatsChartView> {
        if self.daily_points.len() < 2 {
            return None;
        }

        let mut series = Vec::new();
        let top_models = self
            .model_totals
            .iter()
            .take(limit)
            .map(|entry| entry.model.clone())
            .collect::<Vec<_>>();

        if top_models.is_empty() {
            return None;
        }

        for model in top_models {
            let mut points = Vec::new();

            for (index, point) in self.daily_points.iter().enumerate() {
                let x = index as f64;
                let maybe_totals = point.model_tokens.get(&model);
                let value = maybe_totals
                    .map(ModelTokenAccumulator::total_tokens)
                    .unwrap_or(0);
                let y = value as f64;
                points.push((x, y));
            }

            let any_tokens = points.iter().any(|(_x, y)| *y > 0.0);
            if !any_tokens {
                continue;
            }

            let series_entry = StatsChartSeries {
                label: model,
                points,
            };
            series.push(series_entry);
        }

        if series.is_empty() {
            return None;
        }

        let max_tokens = self
            .daily_points
            .iter()
            .map(|point| point.total_tokens)
            .max()
            .unwrap_or(0);

        if max_tokens == 0 {
            return None;
        }

        let first_point = self.daily_points.first()?;
        let middle_index = self.daily_points.len() / 2;
        let middle_point = self.daily_points.get(middle_index)?;
        let last_point = self.daily_points.last()?;

        let start_label = short_date_label(first_point.date);
        let middle_label = short_date_label(middle_point.date);
        let end_label = short_date_label(last_point.date);

        Some(StatsChartView {
            start_label,
            middle_label,
            end_label,
            max_tokens,
            series,
        })
    }
}

pub(super) fn load_stats_snapshot(
    config: &LoongClawConfig,
    current_session_id: &str,
) -> Result<StatsSnapshot, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, current_session_id);
        return Err("stats require sqlite memory support".to_owned());
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config)?;
        let visible_sessions = repo.list_visible_sessions(current_session_id)?;

        let mut active_dates = BTreeSet::new();
        let mut daily_accumulators: BTreeMap<NaiveDate, DailyTokenAccumulator> = BTreeMap::new();
        let mut visible_session_count = 0_usize;
        let mut root_session_count = 0_usize;
        let mut delegate_session_count = 0_usize;
        let mut running_delegate_sessions = 0_usize;
        let mut pending_approvals = 0_usize;
        let mut usage_event_count = 0_usize;
        let mut first_activity_date = None;
        let mut last_activity_date = None;
        let mut longest_session = None;
        let mut session_rows = Vec::new();

        for session in &visible_sessions {
            visible_session_count = visible_session_count.saturating_add(1);

            if session.kind == SessionKind::Root {
                root_session_count = root_session_count.saturating_add(1);
            }

            if session.kind == SessionKind::DelegateChild {
                delegate_session_count = delegate_session_count.saturating_add(1);
            }

            if session.kind == SessionKind::DelegateChild && session.state == SessionState::Running
            {
                running_delegate_sessions = running_delegate_sessions.saturating_add(1);
            }

            let session_pending_approvals = repo.list_approval_requests_for_session(
                session.session_id.as_str(),
                Some(ApprovalRequestStatus::Pending),
            )?;
            let session_pending_count = session_pending_approvals.len();
            pending_approvals = pending_approvals.saturating_add(session_pending_count);

            update_longest_session(&mut longest_session, session);
            let session_row = build_stats_session_row(session, current_session_id);
            session_rows.push(session_row);

            let turn_count = session.turn_count;
            if turn_count == 0 {
                continue;
            }

            let turns = crate::memory::window_direct(
                session.session_id.as_str(),
                turn_count,
                &memory_config,
            )?;

            for turn in turns {
                let maybe_date = utc_date_from_turn(turn.ts);
                let Some(turn_date) = maybe_date else {
                    continue;
                };

                active_dates.insert(turn_date);
                first_activity_date = min_date(first_activity_date, turn_date);
                last_activity_date = max_date(last_activity_date, turn_date);

                let maybe_usage = parse_turn_usage_record(&turn);
                let Some(usage_record) = maybe_usage else {
                    continue;
                };

                usage_event_count = usage_event_count.saturating_add(1);
                let daily_entry: &mut DailyTokenAccumulator =
                    daily_accumulators.entry(turn_date).or_default();
                let model_entry = daily_entry
                    .models
                    .entry(usage_record.model.clone())
                    .or_default();

                model_entry.input_tokens = model_entry
                    .input_tokens
                    .saturating_add(usage_record.input_tokens);
                model_entry.output_tokens = model_entry
                    .output_tokens
                    .saturating_add(usage_record.output_tokens);
            }
        }

        let daily_points = daily_accumulators
            .into_iter()
            .map(|(date, daily)| {
                let mut model_tokens: BTreeMap<String, ModelTokenAccumulator> = BTreeMap::new();
                let mut total_input_tokens = 0_u64;
                let mut total_output_tokens = 0_u64;
                let mut total_tokens = 0_u64;

                for (model, totals) in daily.models {
                    let model_total_tokens = totals.total_tokens();
                    total_input_tokens = total_input_tokens.saturating_add(totals.input_tokens);
                    total_output_tokens = total_output_tokens.saturating_add(totals.output_tokens);
                    total_tokens = total_tokens.saturating_add(model_total_tokens);
                    model_tokens.insert(model, totals);
                }

                DailyTokenPoint {
                    date,
                    total_input_tokens,
                    total_output_tokens,
                    total_tokens,
                    model_tokens,
                }
            })
            .collect::<Vec<_>>();

        let active_dates = active_dates.into_iter().collect::<Vec<_>>();

        Ok(StatsSnapshot {
            visible_sessions: visible_session_count,
            root_sessions: root_session_count,
            delegate_sessions: delegate_session_count,
            running_delegate_sessions,
            pending_approvals,
            usage_event_count,
            first_activity_date,
            last_activity_date,
            longest_session,
            session_rows,
            active_dates,
            daily_points,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_stats_session_row(
    session: &SessionSummaryRecord,
    current_session_id: &str,
) -> StatsSessionRow {
    let end_ts = session.last_turn_at.unwrap_or(session.updated_at);
    let raw_duration_seconds = end_ts.saturating_sub(session.created_at);
    let duration_seconds = u64::try_from(raw_duration_seconds).unwrap_or_default();
    let last_activity_date = utc_date_from_turn(end_ts);
    let kind = session.kind.as_str().to_owned();
    let state = session.state.as_str().to_owned();
    let current = session.session_id == current_session_id;

    StatsSessionRow {
        session_id: session.session_id.clone(),
        label: session.label.clone(),
        kind,
        state,
        turn_count: session.turn_count,
        duration_seconds,
        last_activity_date,
        current,
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_turn_usage_record(turn: &ConversationTurn) -> Option<TurnUsageRecord> {
    if turn.role != "assistant" {
        return None;
    }

    let record = parse_conversation_event(turn.content.as_str())?;
    if record.event != TURN_USAGE_EVENT_NAME {
        return None;
    }

    let payload = record.payload;
    let model = payload
        .get("model")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let input_tokens = payload
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let output_tokens = payload
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    Some(TurnUsageRecord {
        model,
        input_tokens,
        output_tokens,
    })
}

#[cfg(feature = "memory-sqlite")]
fn update_longest_session(
    longest_session: &mut Option<SessionDurationStat>,
    session: &SessionSummaryRecord,
) {
    let end_ts = session.last_turn_at.unwrap_or(session.updated_at);
    let raw_duration = end_ts.saturating_sub(session.created_at);
    let duration_seconds = u64::try_from(raw_duration).unwrap_or_default();

    let candidate = SessionDurationStat {
        session_id: session.session_id.clone(),
        label: session.label.clone(),
        duration_seconds,
    };

    let should_replace = match longest_session {
        Some(existing) => candidate.duration_seconds > existing.duration_seconds,
        None => true,
    };

    if should_replace {
        *longest_session = Some(candidate);
    }
}

fn min_date(current: Option<NaiveDate>, candidate: NaiveDate) -> Option<NaiveDate> {
    match current {
        Some(existing) if existing <= candidate => Some(existing),
        Some(_) => Some(candidate),
        None => Some(candidate),
    }
}

fn max_date(current: Option<NaiveDate>, candidate: NaiveDate) -> Option<NaiveDate> {
    match current {
        Some(existing) if existing >= candidate => Some(existing),
        Some(_) => Some(candidate),
        None => Some(candidate),
    }
}

#[cfg(feature = "memory-sqlite")]
fn utc_date_from_turn(ts: i64) -> Option<NaiveDate> {
    let datetime = chrono::DateTime::<Utc>::from_timestamp(ts, 0)?;
    Some(datetime.date_naive())
}

fn current_streak(active_dates: &[NaiveDate], end_date: NaiveDate) -> usize {
    if active_dates.is_empty() {
        return 0;
    }

    let active_set = active_dates.iter().copied().collect::<BTreeSet<_>>();
    let mut streak = 0_usize;
    let mut current_date = end_date;

    loop {
        if !active_set.contains(&current_date) {
            break;
        }

        streak = streak.saturating_add(1);
        current_date -= Duration::days(1);
    }

    streak
}

fn longest_streak(active_dates: &[NaiveDate]) -> usize {
    if active_dates.is_empty() {
        return 0;
    }

    let mut longest = 1_usize;
    let mut current = 1_usize;

    for pair in active_dates.windows(2) {
        let [previous_date, next_date] = pair else {
            continue;
        };
        let expected_next = *previous_date + Duration::days(1);

        if *next_date == expected_next {
            current = current.saturating_add(1);
        } else {
            current = 1;
        }

        if current > longest {
            longest = current;
        }
    }

    longest
}

pub(super) fn short_date_label(date: NaiveDate) -> String {
    let month = match date.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "Day",
    };
    let day = date.day();
    format!("{month} {day}")
}

pub(super) fn format_compact_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        let whole = value as f64 / 1_000_000.0;
        let rounded = format!("{whole:.1}");
        return format!("{rounded}M");
    }

    if value >= 1_000 {
        let whole = value as f64 / 1_000.0;
        let rounded = format!("{whole:.0}");
        return format!("{rounded}k");
    }

    value.to_string()
}

pub(super) fn format_duration_compact(duration_seconds: u64) -> String {
    let hours = duration_seconds / 3600;
    let minutes = (duration_seconds % 3600) / 60;

    if hours > 0 {
        return format!("{hours}h {minutes}m");
    }

    format!("{minutes}m")
}

pub(super) fn render_copy_text(
    snapshot: &StatsSnapshot,
    active_tab: StatsTab,
    date_range: StatsDateRange,
) -> String {
    let range_view = snapshot.range_view(date_range);
    let mut lines = Vec::new();

    lines.push("/stats".to_owned());
    lines.push(format!(
        "tab={} · range={}",
        active_tab.label(),
        date_range.label()
    ));
    lines.push(String::new());

    match active_tab {
        StatsTab::Overview => {
            let total_tokens = format_compact_tokens(range_view.total_tokens);
            let input_tokens = format_compact_tokens(range_view.total_input_tokens);
            let output_tokens = format_compact_tokens(range_view.total_output_tokens);
            let top_model = range_view
                .top_model
                .as_ref()
                .map(|entry| entry.model.clone())
                .unwrap_or_else(|| "(none)".to_owned());
            let longest_session = snapshot
                .longest_session
                .as_ref()
                .map(|entry| format_duration_compact(entry.duration_seconds))
                .unwrap_or_else(|| "(none)".to_owned());
            let first_activity = snapshot
                .first_activity_date
                .map(short_date_label)
                .unwrap_or_else(|| "(none)".to_owned());
            let last_activity = snapshot
                .last_activity_date
                .map(short_date_label)
                .unwrap_or_else(|| "(none)".to_owned());

            lines.push(format!("visible_sessions={}", snapshot.visible_sessions));
            lines.push(format!("delegate_sessions={}", snapshot.delegate_sessions));
            lines.push(format!("pending_approvals={}", snapshot.pending_approvals));
            lines.push(format!(
                "running_delegate_sessions={}",
                snapshot.running_delegate_sessions
            ));
            lines.push(format!("active_days={}", range_view.active_days));
            lines.push(format!("current_streak={}", range_view.current_streak));
            lines.push(format!("longest_streak={}", range_view.longest_streak));
            lines.push(format!("total_tokens={total_tokens}"));
            lines.push(format!("input_tokens={input_tokens}"));
            lines.push(format!("output_tokens={output_tokens}"));
            lines.push(format!("top_model={top_model}"));
            lines.push(format!("longest_session={longest_session}"));
            lines.push(format!(
                "activity_window={first_activity} -> {last_activity}"
            ));
        }
        StatsTab::Models => {
            lines.push(format!("models={}", range_view.model_totals.len()));

            for entry in range_view.model_totals.iter().take(8) {
                let total_tokens = format_compact_tokens(entry.total_tokens);
                let input_tokens = format_compact_tokens(entry.input_tokens);
                let output_tokens = format_compact_tokens(entry.output_tokens);
                let line = format!(
                    "{} · total {} · in {} · out {}",
                    entry.model, total_tokens, input_tokens, output_tokens,
                );
                lines.push(line);
            }
        }
        StatsTab::Sessions => {
            lines.push(format!("sessions={}", range_view.session_rows.len()));

            for row in range_view.session_rows.iter().take(8) {
                let duration_label = format_duration_compact(row.duration_seconds);
                let line = format!(
                    "{} · {} · {} · {} turns · {}",
                    row.session_id, row.state, row.kind, row.turn_count, duration_label,
                );
                lines.push(line);
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::TimeZone;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::memory::WindowTurn;

    fn stats_test_config(temp_root: &std::path::Path) -> crate::config::LoongClawConfig {
        let mut config = crate::config::LoongClawConfig::default();
        let sqlite_path = temp_root.join("memory.sqlite3");
        let sqlite_path = sqlite_path.display().to_string();
        config.memory.sqlite_path = sqlite_path;
        config.tools.sessions.enabled = true;
        config.tools.sessions.allow_mutation = true;
        config
    }

    fn unix_ts(days_ago: i64, hour: u32) -> i64 {
        let today = Utc::now().date_naive();
        let target_date = today - Duration::days(days_ago);
        let naive = target_date
            .and_hms_opt(hour, 0, 0)
            .expect("valid test timestamp");
        let datetime = Utc.from_utc_datetime(&naive);
        datetime.timestamp()
    }

    #[test]
    #[cfg(feature = "memory-sqlite")]
    fn load_stats_snapshot_aggregates_visible_usage_events() {
        let temp_dir = tempdir().expect("tempdir");
        let config = stats_test_config(temp_dir.path());
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = crate::session::repository::SessionRepository::new(&memory_config)
            .expect("session repo");

        repo.ensure_session(crate::session::repository::NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
            session: crate::session::repository::NewSessionRecord {
                session_id: "child-session".to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some("Child".to_owned()),
                state: SessionState::Running,
            },
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({ "task": "Investigate" }),
        })
        .expect("create child session");

        repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-1".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "tool-1".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: json!({ "tool_name": "delegate_async" }),
            governance_snapshot_json: json!({ "reason": "approval required" }),
        })
        .expect("create approval");

        let root_usage_event = crate::memory::build_conversation_event_content(
            TURN_USAGE_EVENT_NAME,
            json!({
                "model": "gpt-5",
                "input_tokens": 120,
                "output_tokens": 80,
            }),
        );
        let child_usage_event = crate::memory::build_conversation_event_content(
            TURN_USAGE_EVENT_NAME,
            json!({
                "model": "o4-mini",
                "input_tokens": 200,
                "output_tokens": 150,
            }),
        );

        crate::memory::replace_session_turns_direct(
            "root-session",
            &[
                WindowTurn {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                    ts: Some(unix_ts(3, 9)),
                },
                WindowTurn {
                    role: "assistant".to_owned(),
                    content: root_usage_event,
                    ts: Some(unix_ts(3, 10)),
                },
            ],
            &memory_config,
        )
        .expect("seed root turns");

        crate::memory::replace_session_turns_direct(
            "child-session",
            &[WindowTurn {
                role: "assistant".to_owned(),
                content: child_usage_event,
                ts: Some(unix_ts(1, 11)),
            }],
            &memory_config,
        )
        .expect("seed child turns");

        let snapshot = load_stats_snapshot(&config, "root-session").expect("load stats snapshot");
        let range_view = snapshot.range_view(StatsDateRange::All);

        assert_eq!(snapshot.visible_sessions, 2);
        assert_eq!(snapshot.root_sessions, 1);
        assert_eq!(snapshot.delegate_sessions, 1);
        assert_eq!(snapshot.running_delegate_sessions, 1);
        assert_eq!(snapshot.pending_approvals, 1);
        assert_eq!(snapshot.usage_event_count, 2);
        assert_eq!(range_view.total_input_tokens, 320);
        assert_eq!(range_view.total_output_tokens, 230);
        assert_eq!(range_view.total_tokens, 550);
        assert_eq!(range_view.active_days, 2);
        assert_eq!(
            range_view
                .top_model
                .as_ref()
                .map(|entry| entry.model.as_str()),
            Some("o4-mini")
        );
        assert_eq!(range_view.model_totals.len(), 2);
    }

    #[test]
    fn stats_open_options_accept_tab_and_range_tokens() {
        let options = parse_stats_open_options("models 30d").expect("parse stats options");

        assert_eq!(options.tab, StatsTab::Models);
        assert_eq!(options.date_range, StatsDateRange::Last30Days);
    }

    #[test]
    fn render_copy_text_formats_models_view() {
        let snapshot = sample_snapshot_for_copy_test();
        let output = render_copy_text(&snapshot, StatsTab::Models, StatsDateRange::All);

        assert!(output.contains("/stats"));
        assert!(output.contains("tab=Models"));
        assert!(output.contains("gpt-5"));
    }

    #[test]
    fn render_copy_text_formats_sessions_view() {
        let snapshot = sample_snapshot_for_copy_test();
        let output = render_copy_text(&snapshot, StatsTab::Sessions, StatsDateRange::All);

        assert!(output.contains("tab=Sessions"));
        assert!(output.contains("sessions=1"));
        assert!(output.contains("sess-1"));
    }

    fn sample_snapshot_for_copy_test() -> StatsSnapshot {
        let today = Utc::now().date_naive();
        let mut model_tokens = BTreeMap::new();
        model_tokens.insert(
            "gpt-5".to_owned(),
            ModelTokenAccumulator {
                input_tokens: 50,
                output_tokens: 25,
            },
        );

        StatsSnapshot {
            visible_sessions: 1,
            root_sessions: 1,
            delegate_sessions: 0,
            running_delegate_sessions: 0,
            pending_approvals: 0,
            usage_event_count: 1,
            first_activity_date: Some(today),
            last_activity_date: Some(today),
            longest_session: None,
            session_rows: vec![StatsSessionRow {
                session_id: "sess-1".to_owned(),
                label: Some("Root".to_owned()),
                kind: "root".to_owned(),
                state: "ready".to_owned(),
                turn_count: 1,
                duration_seconds: 75,
                last_activity_date: Some(today),
                current: true,
            }],
            active_dates: vec![today],
            daily_points: vec![DailyTokenPoint {
                date: today,
                total_input_tokens: 50,
                total_output_tokens: 25,
                total_tokens: 75,
                model_tokens,
            }],
        }
    }
}
