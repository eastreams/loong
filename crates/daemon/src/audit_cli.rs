use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use clap::Subcommand;

use crate::kernel::{AuditEvent, AuditEventKind, PluginTrustTier};
use loongclaw_spec::CliResult;
use serde_json::{Map, Value, json};

const MAX_AUDIT_WINDOW: usize = 10_000;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AuditCommands {
    /// Print the last N retained audit events
    Recent {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        since_epoch_s: Option<u64>,
        #[arg(long)]
        until_epoch_s: Option<u64>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        pack_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        agent_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        event_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        token_id: Option<String>,
        #[arg(long, value_parser = parse_audit_event_kind_filter)]
        kind: Option<String>,
        #[arg(long, value_parser = parse_audit_triage_label_filter)]
        triage_label: Option<String>,
        #[arg(long, value_parser = parse_audit_query_contains_filter)]
        query_contains: Option<String>,
        #[arg(long, value_parser = parse_plugin_trust_tier_filter)]
        trust_tier: Option<String>,
    },
    /// Print a compact rollup over the last N retained audit events
    Summary {
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        since_epoch_s: Option<u64>,
        #[arg(long)]
        until_epoch_s: Option<u64>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        pack_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        agent_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        event_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        token_id: Option<String>,
        #[arg(long, value_parser = parse_audit_event_kind_filter)]
        kind: Option<String>,
        #[arg(long, value_parser = parse_audit_triage_label_filter)]
        triage_label: Option<String>,
        #[arg(long, value_parser = parse_audit_summary_group_by)]
        group_by: Option<String>,
    },
    /// Summarize trust-aware tool discovery events with dedicated trust filters
    Discovery {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        since_epoch_s: Option<u64>,
        #[arg(long)]
        until_epoch_s: Option<u64>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        pack_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        agent_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        event_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        token_id: Option<String>,
        #[arg(long, value_parser = parse_tool_search_triage_label_filter)]
        triage_label: Option<String>,
        #[arg(long, value_parser = parse_audit_query_contains_filter)]
        query_contains: Option<String>,
        #[arg(long, value_parser = parse_plugin_trust_tier_filter)]
        trust_tier: Option<String>,
        #[arg(long, value_parser = parse_audit_discovery_group_by)]
        group_by: Option<String>,
    },
    /// Reconstruct the retained lifecycle for one capability token
    TokenTrail {
        #[arg(long, value_parser = parse_audit_identity_filter)]
        token_id: String,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        since_epoch_s: Option<u64>,
        #[arg(long)]
        until_epoch_s: Option<u64>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        pack_id: Option<String>,
        #[arg(long, value_parser = parse_audit_identity_filter)]
        agent_id: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct AuditCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub command: AuditCommands,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCommandExecution {
    pub resolved_config_path: String,
    pub journal_path: String,
    pub since_epoch_s_filter: Option<u64>,
    pub until_epoch_s_filter: Option<u64>,
    pub pack_id_filter: Option<String>,
    pub agent_id_filter: Option<String>,
    pub event_id_filter: Option<String>,
    pub token_id_filter: Option<String>,
    pub kind_filter: Option<String>,
    pub triage_label_filter: Option<String>,
    pub query_contains_filter: Option<String>,
    pub trust_tier_filter: Option<String>,
    pub result: AuditCommandResult,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditSummaryGroup {
    pub group_value: Option<String>,
    pub loaded_events: usize,
    pub event_kind_counts: BTreeMap<String, usize>,
    pub triage_counts: BTreeMap<String, usize>,
    pub first_timestamp_epoch_s: Option<u64>,
    pub last_event_id: Option<String>,
    pub last_timestamp_epoch_s: Option<u64>,
    pub last_agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditDiscoveryGroup {
    pub group_value: Option<String>,
    pub loaded_events: usize,
    pub triage_counts: BTreeMap<String, usize>,
    pub query_requested_tier_counts: BTreeMap<String, usize>,
    pub structured_requested_tier_counts: BTreeMap<String, usize>,
    pub effective_tier_counts: BTreeMap<String, usize>,
    pub filtered_out_tier_counts: BTreeMap<String, usize>,
    pub trust_filter_applied_events: usize,
    pub conflicting_requested_tier_events: usize,
    pub trust_filtered_empty_events: usize,
    pub first_timestamp_epoch_s: Option<u64>,
    pub last_event_id: Option<String>,
    pub last_timestamp_epoch_s: Option<u64>,
    pub last_agent_id: Option<String>,
    pub last_pack_id: Option<String>,
    pub last_query: Option<String>,
    pub last_returned: Option<usize>,
    pub correlated_summary: Option<AuditSummaryGroup>,
    pub correlated_additional_events: usize,
    pub correlated_non_discovery_event_kind_counts: BTreeMap<String, usize>,
    pub correlated_non_discovery_triage_counts: BTreeMap<String, usize>,
    pub correlated_attention_hint: Option<String>,
    pub correlated_remediation_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditCommandResult {
    Recent {
        limit: usize,
        events: Vec<AuditEvent>,
    },
    Summary {
        limit: usize,
        loaded_events: usize,
        event_kind_counts: BTreeMap<String, usize>,
        triage_counts: BTreeMap<String, usize>,
        group_by: Option<String>,
        groups: Vec<AuditSummaryGroup>,
        first_timestamp_epoch_s: Option<u64>,
        last_event_id: Option<String>,
        last_timestamp_epoch_s: Option<u64>,
        last_agent_id: Option<String>,
        last_triage_event_id: Option<String>,
        last_triage_label: Option<String>,
        last_triage_event_kind: Option<String>,
        last_triage_timestamp_epoch_s: Option<u64>,
        last_triage_agent_id: Option<String>,
        last_triage_summary: Option<String>,
        last_triage_hint: Option<String>,
    },
    Discovery {
        limit: usize,
        loaded_events: usize,
        triage_counts: BTreeMap<String, usize>,
        query_requested_tier_counts: BTreeMap<String, usize>,
        structured_requested_tier_counts: BTreeMap<String, usize>,
        effective_tier_counts: BTreeMap<String, usize>,
        filtered_out_tier_counts: BTreeMap<String, usize>,
        trust_filter_applied_events: usize,
        conflicting_requested_tier_events: usize,
        trust_filtered_empty_events: usize,
        group_by: Option<String>,
        groups: Vec<AuditDiscoveryGroup>,
        first_timestamp_epoch_s: Option<u64>,
        last_event_id: Option<String>,
        last_timestamp_epoch_s: Option<u64>,
        last_agent_id: Option<String>,
        last_pack_id: Option<String>,
        last_query: Option<String>,
        last_returned: Option<usize>,
        last_trust_filter_applied: Option<bool>,
        last_conflicting_requested_tiers: Option<bool>,
        last_query_requested_tiers: Vec<String>,
        last_structured_requested_tiers: Vec<String>,
        last_effective_tiers: Vec<String>,
        last_filtered_out_candidates: Option<usize>,
        last_filtered_out_tier_counts: BTreeMap<String, usize>,
        last_top_provider_ids: Vec<String>,
        last_triage_event_id: Option<String>,
        last_triage_label: Option<String>,
        last_triage_timestamp_epoch_s: Option<u64>,
        last_triage_agent_id: Option<String>,
        last_triage_summary: Option<String>,
        last_triage_hint: Option<String>,
    },
    TokenTrail {
        limit: usize,
        token_id: String,
        loaded_events: usize,
        total_matching_events: usize,
        truncated_matching_events: usize,
        event_kind_counts: BTreeMap<String, usize>,
        first_timestamp_epoch_s: Option<u64>,
        last_event_id: Option<String>,
        last_timestamp_epoch_s: Option<u64>,
        last_agent_id: Option<String>,
        issued_event_id: Option<String>,
        issued_timestamp_epoch_s: Option<u64>,
        issued_pack_id: Option<String>,
        issued_agent_id: Option<String>,
        issued_generation: Option<u64>,
        issued_expires_at_epoch_s: Option<u64>,
        issued_capability_count: Option<usize>,
        issued_capabilities: Vec<String>,
        authorization_denied_count: usize,
        authorization_denied_reason_counts: BTreeMap<String, usize>,
        last_denied_event_id: Option<String>,
        last_denied_timestamp_epoch_s: Option<u64>,
        last_denied_pack_id: Option<String>,
        last_denied_agent_id: Option<String>,
        last_denied_reason: Option<String>,
        revoked_event_id: Option<String>,
        revoked_timestamp_epoch_s: Option<u64>,
        revoked_agent_id: Option<String>,
        timeline: Vec<AuditEvent>,
    },
}

pub fn run_audit_cli(options: AuditCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_audit_command(options)?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&audit_cli_json(&execution))
            .map_err(|error| format!("serialize audit CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("{}", render_audit_cli_text(&execution)?);
    Ok(())
}

pub fn execute_audit_command(options: AuditCommandOptions) -> CliResult<AuditCommandExecution> {
    let AuditCommandOptions {
        config,
        json: _,
        command,
    } = options;

    let (
        limit,
        since_epoch_s_filter,
        until_epoch_s_filter,
        pack_id_filter,
        agent_id_filter,
        event_id_filter,
        token_id_filter,
        kind_filter,
        triage_label_filter,
        query_contains_filter,
        trust_tier_filter,
        command_name,
    ) = match &command {
        AuditCommands::Recent {
            limit,
            since_epoch_s,
            until_epoch_s,
            pack_id,
            agent_id,
            event_id,
            token_id,
            kind,
            triage_label,
            query_contains,
            trust_tier,
        } => (
            *limit,
            *since_epoch_s,
            *until_epoch_s,
            pack_id.clone(),
            agent_id.clone(),
            event_id.clone(),
            token_id.clone(),
            kind.clone(),
            triage_label.clone(),
            query_contains.clone(),
            trust_tier.clone(),
            "audit recent",
        ),
        AuditCommands::Summary {
            limit,
            since_epoch_s,
            until_epoch_s,
            pack_id,
            agent_id,
            event_id,
            token_id,
            kind,
            triage_label,
            group_by: _,
        } => (
            *limit,
            *since_epoch_s,
            *until_epoch_s,
            pack_id.clone(),
            agent_id.clone(),
            event_id.clone(),
            token_id.clone(),
            kind.clone(),
            triage_label.clone(),
            None,
            None,
            "audit summary",
        ),
        AuditCommands::Discovery {
            limit,
            since_epoch_s,
            until_epoch_s,
            pack_id,
            agent_id,
            event_id,
            token_id,
            triage_label,
            query_contains,
            trust_tier,
            group_by: _,
        } => (
            *limit,
            *since_epoch_s,
            *until_epoch_s,
            pack_id.clone(),
            agent_id.clone(),
            event_id.clone(),
            token_id.clone(),
            Some("ToolSearchEvaluated".to_owned()),
            triage_label.clone(),
            query_contains.clone(),
            trust_tier.clone(),
            "audit discovery",
        ),
        AuditCommands::TokenTrail {
            token_id,
            limit,
            since_epoch_s,
            until_epoch_s,
            pack_id,
            agent_id,
        } => (
            *limit,
            *since_epoch_s,
            *until_epoch_s,
            pack_id.clone(),
            agent_id.clone(),
            None,
            Some(token_id.clone()),
            None,
            None,
            None,
            None,
            "audit token-trail",
        ),
    };
    let limit = validate_audit_limit(limit, command_name)?;
    validate_audit_time_range(since_epoch_s_filter, until_epoch_s_filter, command_name)?;
    let filter = AuditEventFilter {
        since_epoch_s: since_epoch_s_filter,
        until_epoch_s: until_epoch_s_filter,
        pack_id: pack_id_filter.clone(),
        agent_id: agent_id_filter.clone(),
        event_id: event_id_filter.clone(),
        token_id: token_id_filter.clone(),
        kind: kind_filter.clone(),
        triage_label: triage_label_filter.clone(),
        query_contains: query_contains_filter.clone(),
        trust_tier: trust_tier_filter.clone(),
    };

    let (resolved_path, config) = crate::mvp::config::load(config.as_deref())?;
    let journal_path = config.audit.resolved_path();
    let audit_window = load_audit_event_window(&config.audit, &journal_path, limit, &filter)?;
    let total_matching_events = audit_window.total_matching_events;
    let events = audit_window.events;
    let result = match command {
        AuditCommands::Recent { limit, .. } => AuditCommandResult::Recent { limit, events },
        AuditCommands::Summary {
            limit, group_by, ..
        } => summarize_audit_events(limit, group_by, &events),
        AuditCommands::Discovery {
            limit, group_by, ..
        } => {
            let correlated_summary_groups = if group_by.is_some() {
                let broader_window = load_audit_event_window(
                    &config.audit,
                    &journal_path,
                    limit,
                    &AuditEventFilter {
                        since_epoch_s: since_epoch_s_filter,
                        until_epoch_s: until_epoch_s_filter,
                        pack_id: pack_id_filter.clone(),
                        agent_id: agent_id_filter.clone(),
                        ..Default::default()
                    },
                )?;
                summarize_audit_groups(&broader_window.events, group_by.as_deref())
            } else {
                Vec::new()
            };
            summarize_discovery_events(limit, group_by, &events, &correlated_summary_groups)
        }
        AuditCommands::TokenTrail {
            limit, token_id, ..
        } => summarize_token_trail(limit, token_id, events, total_matching_events),
    };

    Ok(AuditCommandExecution {
        resolved_config_path: resolved_path.display().to_string(),
        journal_path: journal_path.display().to_string(),
        since_epoch_s_filter,
        until_epoch_s_filter,
        pack_id_filter,
        agent_id_filter,
        event_id_filter,
        token_id_filter,
        kind_filter,
        triage_label_filter,
        query_contains_filter,
        trust_tier_filter,
        result,
    })
}

pub fn audit_cli_json(execution: &AuditCommandExecution) -> Value {
    match &execution.result {
        AuditCommandResult::Recent { limit, events } => {
            let mut payload = audit_cli_base_json(execution, "recent");
            payload.insert("limit".to_owned(), json!(limit));
            payload.insert("loaded_events".to_owned(), json!(events.len()));
            payload.insert("events".to_owned(), json!(events));
            Value::Object(payload)
        }
        AuditCommandResult::Summary {
            limit,
            loaded_events,
            event_kind_counts,
            triage_counts,
            group_by,
            groups,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            last_triage_event_id,
            last_triage_label,
            last_triage_event_kind,
            last_triage_timestamp_epoch_s,
            last_triage_agent_id,
            last_triage_summary,
            last_triage_hint,
        } => {
            let mut payload = audit_cli_base_json(execution, "summary");
            payload.insert("limit".to_owned(), json!(limit));
            payload.insert("loaded_events".to_owned(), json!(loaded_events));
            payload.insert("event_kind_counts".to_owned(), json!(event_kind_counts));
            payload.insert("triage_counts".to_owned(), json!(triage_counts));
            payload.insert("group_by".to_owned(), json!(group_by));
            payload.insert("groups".to_owned(), json!(groups));
            payload.insert(
                "first_timestamp_epoch_s".to_owned(),
                json!(first_timestamp_epoch_s),
            );
            payload.insert("last_event_id".to_owned(), json!(last_event_id));
            payload.insert(
                "last_timestamp_epoch_s".to_owned(),
                json!(last_timestamp_epoch_s),
            );
            payload.insert("last_agent_id".to_owned(), json!(last_agent_id));
            payload.insert(
                "last_triage_event_id".to_owned(),
                json!(last_triage_event_id),
            );
            payload.insert("last_triage_label".to_owned(), json!(last_triage_label));
            payload.insert(
                "last_triage_event_kind".to_owned(),
                json!(last_triage_event_kind),
            );
            payload.insert(
                "last_triage_timestamp_epoch_s".to_owned(),
                json!(last_triage_timestamp_epoch_s),
            );
            payload.insert(
                "last_triage_agent_id".to_owned(),
                json!(last_triage_agent_id),
            );
            payload.insert("last_triage_summary".to_owned(), json!(last_triage_summary));
            payload.insert("last_triage_hint".to_owned(), json!(last_triage_hint));
            Value::Object(payload)
        }
        AuditCommandResult::Discovery {
            limit,
            loaded_events,
            triage_counts,
            query_requested_tier_counts,
            structured_requested_tier_counts,
            effective_tier_counts,
            filtered_out_tier_counts,
            trust_filter_applied_events,
            conflicting_requested_tier_events,
            trust_filtered_empty_events,
            group_by,
            groups,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            last_pack_id,
            last_query,
            last_returned,
            last_trust_filter_applied,
            last_conflicting_requested_tiers,
            last_query_requested_tiers,
            last_structured_requested_tiers,
            last_effective_tiers,
            last_filtered_out_candidates,
            last_filtered_out_tier_counts,
            last_top_provider_ids,
            last_triage_event_id,
            last_triage_label,
            last_triage_timestamp_epoch_s,
            last_triage_agent_id,
            last_triage_summary,
            last_triage_hint,
        } => {
            let mut payload = audit_cli_base_json(execution, "discovery");
            payload.insert("limit".to_owned(), json!(limit));
            payload.insert("loaded_events".to_owned(), json!(loaded_events));
            payload.insert("triage_counts".to_owned(), json!(triage_counts));
            payload.insert(
                "query_requested_tier_counts".to_owned(),
                json!(query_requested_tier_counts),
            );
            payload.insert(
                "structured_requested_tier_counts".to_owned(),
                json!(structured_requested_tier_counts),
            );
            payload.insert(
                "effective_tier_counts".to_owned(),
                json!(effective_tier_counts),
            );
            payload.insert(
                "filtered_out_tier_counts".to_owned(),
                json!(filtered_out_tier_counts),
            );
            payload.insert(
                "trust_filter_applied_events".to_owned(),
                json!(trust_filter_applied_events),
            );
            payload.insert(
                "conflicting_requested_tier_events".to_owned(),
                json!(conflicting_requested_tier_events),
            );
            payload.insert(
                "trust_filtered_empty_events".to_owned(),
                json!(trust_filtered_empty_events),
            );
            payload.insert("group_by".to_owned(), json!(group_by));
            payload.insert(
                "groups".to_owned(),
                audit_discovery_groups_json(execution, *limit, group_by.as_deref(), groups),
            );
            payload.insert(
                "first_timestamp_epoch_s".to_owned(),
                json!(first_timestamp_epoch_s),
            );
            payload.insert("last_event_id".to_owned(), json!(last_event_id));
            payload.insert(
                "last_timestamp_epoch_s".to_owned(),
                json!(last_timestamp_epoch_s),
            );
            payload.insert("last_agent_id".to_owned(), json!(last_agent_id));
            payload.insert("last_pack_id".to_owned(), json!(last_pack_id));
            payload.insert("last_query".to_owned(), json!(last_query));
            payload.insert("last_returned".to_owned(), json!(last_returned));
            payload.insert(
                "last_trust_filter_applied".to_owned(),
                json!(last_trust_filter_applied),
            );
            payload.insert(
                "last_conflicting_requested_tiers".to_owned(),
                json!(last_conflicting_requested_tiers),
            );
            payload.insert(
                "last_query_requested_tiers".to_owned(),
                json!(last_query_requested_tiers),
            );
            payload.insert(
                "last_structured_requested_tiers".to_owned(),
                json!(last_structured_requested_tiers),
            );
            payload.insert(
                "last_effective_tiers".to_owned(),
                json!(last_effective_tiers),
            );
            payload.insert(
                "last_filtered_out_candidates".to_owned(),
                json!(last_filtered_out_candidates),
            );
            payload.insert(
                "last_filtered_out_tier_counts".to_owned(),
                json!(last_filtered_out_tier_counts),
            );
            payload.insert(
                "last_top_provider_ids".to_owned(),
                json!(last_top_provider_ids),
            );
            payload.insert(
                "last_triage_event_id".to_owned(),
                json!(last_triage_event_id),
            );
            payload.insert("last_triage_label".to_owned(), json!(last_triage_label));
            payload.insert(
                "last_triage_timestamp_epoch_s".to_owned(),
                json!(last_triage_timestamp_epoch_s),
            );
            payload.insert(
                "last_triage_agent_id".to_owned(),
                json!(last_triage_agent_id),
            );
            payload.insert("last_triage_summary".to_owned(), json!(last_triage_summary));
            payload.insert("last_triage_hint".to_owned(), json!(last_triage_hint));
            Value::Object(payload)
        }
        AuditCommandResult::TokenTrail {
            limit,
            token_id,
            loaded_events,
            total_matching_events,
            truncated_matching_events,
            event_kind_counts,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            issued_event_id,
            issued_timestamp_epoch_s,
            issued_pack_id,
            issued_agent_id,
            issued_generation,
            issued_expires_at_epoch_s,
            issued_capability_count,
            issued_capabilities,
            authorization_denied_count,
            authorization_denied_reason_counts,
            last_denied_event_id,
            last_denied_timestamp_epoch_s,
            last_denied_pack_id,
            last_denied_agent_id,
            last_denied_reason,
            revoked_event_id,
            revoked_timestamp_epoch_s,
            revoked_agent_id,
            timeline,
        } => {
            let mut payload = audit_cli_base_json(execution, "token-trail");
            payload.insert("limit".to_owned(), json!(limit));
            payload.insert("token_id".to_owned(), json!(token_id));
            payload.insert("loaded_events".to_owned(), json!(loaded_events));
            payload.insert(
                "total_matching_events".to_owned(),
                json!(total_matching_events),
            );
            payload.insert(
                "truncated_matching_events".to_owned(),
                json!(truncated_matching_events),
            );
            payload.insert("event_kind_counts".to_owned(), json!(event_kind_counts));
            payload.insert(
                "first_timestamp_epoch_s".to_owned(),
                json!(first_timestamp_epoch_s),
            );
            payload.insert("last_event_id".to_owned(), json!(last_event_id));
            payload.insert(
                "last_timestamp_epoch_s".to_owned(),
                json!(last_timestamp_epoch_s),
            );
            payload.insert("last_agent_id".to_owned(), json!(last_agent_id));
            payload.insert("issued_event_id".to_owned(), json!(issued_event_id));
            payload.insert(
                "issued_timestamp_epoch_s".to_owned(),
                json!(issued_timestamp_epoch_s),
            );
            payload.insert("issued_pack_id".to_owned(), json!(issued_pack_id));
            payload.insert("issued_agent_id".to_owned(), json!(issued_agent_id));
            payload.insert("issued_generation".to_owned(), json!(issued_generation));
            payload.insert(
                "issued_expires_at_epoch_s".to_owned(),
                json!(issued_expires_at_epoch_s),
            );
            payload.insert(
                "issued_capability_count".to_owned(),
                json!(issued_capability_count),
            );
            payload.insert("issued_capabilities".to_owned(), json!(issued_capabilities));
            payload.insert(
                "authorization_denied_count".to_owned(),
                json!(authorization_denied_count),
            );
            payload.insert(
                "authorization_denied_reason_counts".to_owned(),
                json!(authorization_denied_reason_counts),
            );
            payload.insert(
                "last_denied_event_id".to_owned(),
                json!(last_denied_event_id),
            );
            payload.insert(
                "last_denied_timestamp_epoch_s".to_owned(),
                json!(last_denied_timestamp_epoch_s),
            );
            payload.insert("last_denied_pack_id".to_owned(), json!(last_denied_pack_id));
            payload.insert(
                "last_denied_agent_id".to_owned(),
                json!(last_denied_agent_id),
            );
            payload.insert("last_denied_reason".to_owned(), json!(last_denied_reason));
            payload.insert("revoked_event_id".to_owned(), json!(revoked_event_id));
            payload.insert(
                "revoked_timestamp_epoch_s".to_owned(),
                json!(revoked_timestamp_epoch_s),
            );
            payload.insert("revoked_agent_id".to_owned(), json!(revoked_agent_id));
            payload.insert("timeline".to_owned(), json!(timeline));
            Value::Object(payload)
        }
    }
}

fn audit_cli_base_json(execution: &AuditCommandExecution, command: &str) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("command".to_owned(), json!(command));
    payload.insert("config".to_owned(), json!(&execution.resolved_config_path));
    payload.insert("journal_path".to_owned(), json!(&execution.journal_path));
    payload.insert(
        "since_epoch_s_filter".to_owned(),
        json!(execution.since_epoch_s_filter),
    );
    payload.insert(
        "until_epoch_s_filter".to_owned(),
        json!(execution.until_epoch_s_filter),
    );
    payload.insert("pack_id_filter".to_owned(), json!(execution.pack_id_filter));
    payload.insert(
        "agent_id_filter".to_owned(),
        json!(execution.agent_id_filter),
    );
    payload.insert(
        "event_id_filter".to_owned(),
        json!(execution.event_id_filter),
    );
    payload.insert(
        "token_id_filter".to_owned(),
        json!(execution.token_id_filter),
    );
    payload.insert("kind_filter".to_owned(), json!(execution.kind_filter));
    payload.insert(
        "triage_label_filter".to_owned(),
        json!(execution.triage_label_filter),
    );
    payload.insert(
        "query_contains_filter".to_owned(),
        json!(execution.query_contains_filter),
    );
    payload.insert(
        "trust_tier_filter".to_owned(),
        json!(execution.trust_tier_filter),
    );
    payload
}

fn serialize_json_object_or_empty<T>(value: &T) -> Map<String, Value>
where
    T: serde::Serialize,
{
    let serialized_result = serde_json::to_value(value);
    let Ok(serialized_value) = serialized_result else {
        return Map::new();
    };

    let Value::Object(payload) = serialized_value else {
        return Map::new();
    };

    payload
}

fn audit_discovery_groups_json(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    groups: &[AuditDiscoveryGroup],
) -> Value {
    Value::Array(
        groups
            .iter()
            .map(|group| {
                let mut payload = serialize_json_object_or_empty(group);
                payload.insert(
                    "drill_down_command".to_owned(),
                    json!(discovery_group_drill_down_command(
                        execution, limit, group_by, group
                    )),
                );
                payload.insert(
                    "correlated_summary_command".to_owned(),
                    json!(discovery_group_correlated_summary_command(
                        execution, limit, group_by, group
                    )),
                );
                payload.insert(
                    "correlated_remediation_command".to_owned(),
                    json!(discovery_group_correlated_remediation_command(
                        execution, limit, group_by, group
                    )),
                );
                Value::Object(payload)
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CorrelatedRemediationCommandTarget {
    RecentTriage { triage_label: String },
    SummaryByToken { triage_label: String },
    RecentKind { kind: String },
    SummaryScope,
}

fn discovery_group_drill_down_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let mut parts = discovery_group_scoped_command_parts(
        "audit recent",
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );
    push_optional_shell_flag(
        &mut parts,
        "--event-id",
        execution.event_id_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--token-id",
        execution.token_id_filter.as_deref(),
    );
    push_optional_shell_flag(&mut parts, "--kind", execution.kind_filter.as_deref());
    push_optional_shell_flag(
        &mut parts,
        "--triage-label",
        execution.triage_label_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--query-contains",
        execution.query_contains_filter.as_deref(),
    );
    push_optional_shell_flag(
        &mut parts,
        "--trust-tier",
        execution.trust_tier_filter.as_deref(),
    );

    Some(parts.join(" "))
}

fn discovery_group_correlated_summary_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let parts = discovery_group_scoped_command_parts(
        "audit summary",
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );

    Some(parts.join(" "))
}

fn discovery_group_correlated_remediation_command(
    execution: &AuditCommandExecution,
    limit: usize,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<String> {
    let target = correlated_remediation_command_target(
        group.correlated_additional_events,
        &group.correlated_non_discovery_event_kind_counts,
        &group.correlated_non_discovery_triage_counts,
    )?;
    let (pack_id_filter, agent_id_filter) =
        discovery_group_identity_filters(execution, group_by, group)?;
    let pack_id_filter = pack_id_filter.as_deref();
    let agent_id_filter = agent_id_filter.as_deref();
    let subcommand = remediation_command_subcommand(&target);
    let mut parts = discovery_group_scoped_command_parts(
        subcommand,
        execution,
        limit,
        pack_id_filter,
        agent_id_filter,
    );

    match target {
        CorrelatedRemediationCommandTarget::RecentTriage { triage_label } => {
            push_shell_argument_flag(&mut parts, "--triage-label", &triage_label);
        }
        CorrelatedRemediationCommandTarget::SummaryByToken { triage_label } => {
            push_shell_argument_flag(&mut parts, "--triage-label", &triage_label);
            push_shell_argument_flag(&mut parts, "--group-by", "token");
        }
        CorrelatedRemediationCommandTarget::RecentKind { kind } => {
            push_shell_argument_flag(&mut parts, "--kind", &kind);
        }
        CorrelatedRemediationCommandTarget::SummaryScope => {}
    }

    Some(parts.join(" "))
}

fn remediation_command_subcommand(target: &CorrelatedRemediationCommandTarget) -> &'static str {
    match target {
        CorrelatedRemediationCommandTarget::RecentTriage { .. } => "audit recent",
        CorrelatedRemediationCommandTarget::SummaryByToken { .. } => "audit summary",
        CorrelatedRemediationCommandTarget::RecentKind { .. } => "audit recent",
        CorrelatedRemediationCommandTarget::SummaryScope => "audit summary",
    }
}

fn correlated_remediation_command_target(
    additional_events: usize,
    non_discovery_event_kind_counts: &BTreeMap<String, usize>,
    non_discovery_triage_counts: &BTreeMap<String, usize>,
) -> Option<CorrelatedRemediationCommandTarget> {
    let top_triage_label = top_rollup_label(non_discovery_triage_counts);
    if let Some(top_triage_label) = top_triage_label {
        let triage_label = top_triage_label.to_owned();
        if top_triage_label == "authorization_denied" {
            return Some(CorrelatedRemediationCommandTarget::SummaryByToken { triage_label });
        }
        return Some(CorrelatedRemediationCommandTarget::RecentTriage { triage_label });
    }

    let top_event_kind = top_rollup_label(non_discovery_event_kind_counts);
    if let Some(top_event_kind) = top_event_kind {
        let kind = top_event_kind.to_owned();
        return Some(CorrelatedRemediationCommandTarget::RecentKind { kind });
    }

    if additional_events > 0 {
        return Some(CorrelatedRemediationCommandTarget::SummaryScope);
    }

    None
}

fn discovery_group_scoped_command_parts(
    subcommand: &str,
    execution: &AuditCommandExecution,
    limit: usize,
    pack_id_filter: Option<&str>,
    agent_id_filter: Option<&str>,
) -> Vec<String> {
    let mut parts = Vec::new();
    let base_command = crate::cli_handoff::format_subcommand_with_config(
        subcommand,
        &execution.resolved_config_path,
    );
    parts.push(base_command);
    parts.push("--limit".to_owned());
    parts.push(limit.to_string());
    push_optional_numeric_flag(
        &mut parts,
        "--since-epoch-s",
        execution.since_epoch_s_filter,
    );
    push_optional_numeric_flag(
        &mut parts,
        "--until-epoch-s",
        execution.until_epoch_s_filter,
    );
    push_optional_shell_flag(&mut parts, "--pack-id", pack_id_filter);
    push_optional_shell_flag(&mut parts, "--agent-id", agent_id_filter);
    parts
}

fn discovery_group_identity_filters(
    execution: &AuditCommandExecution,
    group_by: Option<&str>,
    group: &AuditDiscoveryGroup,
) -> Option<(Option<String>, Option<String>)> {
    let group_by = group_by?;

    let mut pack_id_filter = execution.pack_id_filter.clone();
    let mut agent_id_filter = execution.agent_id_filter.clone();

    match group_by {
        "pack" => merge_discovery_group_filter(&mut pack_id_filter, group.group_value.as_ref())?,
        "agent" => merge_discovery_group_filter(&mut agent_id_filter, group.group_value.as_ref())?,
        _ => return None,
    }

    Some((pack_id_filter, agent_id_filter))
}

fn merge_discovery_group_filter(
    filter: &mut Option<String>,
    group_value: Option<&String>,
) -> Option<()> {
    let group_value = group_value?;
    match filter.as_deref() {
        Some(existing) if existing != group_value => None,
        None => {
            *filter = Some(group_value.clone());
            Some(())
        }
        _ => Some(()),
    }
}

fn push_optional_numeric_flag(parts: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        parts.push(flag.to_owned());
        parts.push(value.to_string());
    }
}

fn push_optional_shell_flag(parts: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        push_shell_argument_flag(parts, flag, value);
    }
}

fn push_shell_argument_flag(parts: &mut Vec<String>, flag: &str, value: &str) {
    parts.push(flag.to_owned());
    parts.push(crate::cli_handoff::shell_quote_argument(value));
}

pub fn render_audit_cli_text(execution: &AuditCommandExecution) -> CliResult<String> {
    let mut lines = Vec::new();
    match &execution.result {
        AuditCommandResult::Recent { limit, events } => {
            lines.push(format!(
                "audit recent config={} journal={} limit={} loaded_events={}",
                execution.resolved_config_path,
                execution.journal_path,
                limit,
                events.len()
            ));
            lines.push(format!(
                "filters since_epoch_s={} until_epoch_s={} pack_id={} agent_id={} event_id={} token_id={} kind={} triage_label={} query_contains={} trust_tier={}",
                execution
                    .since_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution
                    .until_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution.pack_id_filter.as_deref().unwrap_or("-"),
                execution.agent_id_filter.as_deref().unwrap_or("-"),
                execution.event_id_filter.as_deref().unwrap_or("-"),
                execution.token_id_filter.as_deref().unwrap_or("-"),
                execution.kind_filter.as_deref().unwrap_or("-"),
                execution.triage_label_filter.as_deref().unwrap_or("-"),
                execution.query_contains_filter.as_deref().unwrap_or("-"),
                execution.trust_tier_filter.as_deref().unwrap_or("-")
            ));
            for event in events {
                let detail = format_audit_event_detail(&event.kind);
                lines.push(format!(
                    "- ts={} event_id={} agent_id={} kind={} {}",
                    event.timestamp_epoch_s,
                    event.event_id,
                    event.agent_id.as_deref().unwrap_or("-"),
                    audit_event_kind_label(&event.kind),
                    detail
                ));
            }
        }
        AuditCommandResult::Summary {
            limit,
            loaded_events,
            event_kind_counts,
            triage_counts,
            group_by,
            groups,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            last_triage_event_id,
            last_triage_label,
            last_triage_event_kind,
            last_triage_timestamp_epoch_s,
            last_triage_agent_id,
            last_triage_summary,
            last_triage_hint,
        } => {
            lines.push(format!(
                "audit summary config={} journal={} limit={} loaded_events={}",
                execution.resolved_config_path, execution.journal_path, limit, loaded_events
            ));
            lines.push(format!(
                "filters since_epoch_s={} until_epoch_s={} pack_id={} agent_id={} event_id={} token_id={} kind={} triage_label={}",
                execution
                    .since_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution
                    .until_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution.pack_id_filter.as_deref().unwrap_or("-"),
                execution.agent_id_filter.as_deref().unwrap_or("-"),
                execution.event_id_filter.as_deref().unwrap_or("-"),
                execution.token_id_filter.as_deref().unwrap_or("-"),
                execution.kind_filter.as_deref().unwrap_or("-"),
                execution.triage_label_filter.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "event_kind_counts={}",
                format_equals_rollup(event_kind_counts)
            ));
            lines.push(format!(
                "triage_counts={}",
                format_equals_rollup(triage_counts)
            ));
            lines.push(format!(
                "group_by={} group_count={}",
                group_by.as_deref().unwrap_or("-"),
                groups.len()
            ));
            for group in groups {
                lines.push(format!(
                    "group[{}]={} loaded_events={} event_kind_counts={} triage_counts={} first_timestamp_epoch_s={} last_event_id={} last_timestamp_epoch_s={} last_agent_id={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    format_optional_summary_group_label(&group.group_value),
                    group.loaded_events,
                    format_equals_rollup(&group.event_kind_counts),
                    format_equals_rollup(&group.triage_counts),
                    group
                        .first_timestamp_epoch_s
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    group.last_event_id.as_deref().unwrap_or("-"),
                    group
                        .last_timestamp_epoch_s
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    group.last_agent_id.as_deref().unwrap_or("-")
                ));
            }
            lines.push(format!(
                "first_timestamp_epoch_s={}",
                first_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ));
            lines.push(format!(
                "last_event_id={} last_timestamp_epoch_s={} last_agent_id={}",
                last_event_id.as_deref().unwrap_or("-"),
                last_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_agent_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_triage_event_id={} last_triage_label={} last_triage_event_kind={} last_triage_timestamp_epoch_s={} last_triage_agent_id={}",
                last_triage_event_id.as_deref().unwrap_or("-"),
                last_triage_label.as_deref().unwrap_or("-"),
                last_triage_event_kind.as_deref().unwrap_or("-"),
                last_triage_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_triage_agent_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_triage_summary={}",
                last_triage_summary.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_triage_hint={}",
                last_triage_hint.as_deref().unwrap_or("-")
            ));
        }
        AuditCommandResult::Discovery {
            limit,
            loaded_events,
            triage_counts,
            query_requested_tier_counts,
            structured_requested_tier_counts,
            effective_tier_counts,
            filtered_out_tier_counts,
            trust_filter_applied_events,
            conflicting_requested_tier_events,
            trust_filtered_empty_events,
            group_by,
            groups,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            last_pack_id,
            last_query,
            last_returned,
            last_trust_filter_applied,
            last_conflicting_requested_tiers,
            last_query_requested_tiers,
            last_structured_requested_tiers,
            last_effective_tiers,
            last_filtered_out_candidates,
            last_filtered_out_tier_counts,
            last_top_provider_ids,
            last_triage_event_id,
            last_triage_label,
            last_triage_timestamp_epoch_s,
            last_triage_agent_id,
            last_triage_summary,
            last_triage_hint,
        } => {
            lines.push(format!(
                "audit discovery config={} journal={} limit={} loaded_events={}",
                execution.resolved_config_path, execution.journal_path, limit, loaded_events
            ));
            lines.push(format!(
                "filters since_epoch_s={} until_epoch_s={} pack_id={} agent_id={} event_id={} token_id={} kind={} triage_label={} query_contains={} trust_tier={}",
                execution
                    .since_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution
                    .until_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution.pack_id_filter.as_deref().unwrap_or("-"),
                execution.agent_id_filter.as_deref().unwrap_or("-"),
                execution.event_id_filter.as_deref().unwrap_or("-"),
                execution.token_id_filter.as_deref().unwrap_or("-"),
                execution.kind_filter.as_deref().unwrap_or("-"),
                execution.triage_label_filter.as_deref().unwrap_or("-"),
                execution.query_contains_filter.as_deref().unwrap_or("-"),
                execution.trust_tier_filter.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "triage_counts={}",
                format_equals_rollup(triage_counts)
            ));
            lines.push(format!(
                "query_requested_tier_counts={}",
                format_equals_rollup(query_requested_tier_counts)
            ));
            lines.push(format!(
                "structured_requested_tier_counts={}",
                format_equals_rollup(structured_requested_tier_counts)
            ));
            lines.push(format!(
                "effective_tier_counts={}",
                format_equals_rollup(effective_tier_counts)
            ));
            lines.push(format!(
                "filtered_out_tier_counts={}",
                format_equals_rollup(filtered_out_tier_counts)
            ));
            lines.push(format!(
                "trust_filter_applied_events={} conflicting_requested_tier_events={} trust_filtered_empty_events={}",
                trust_filter_applied_events,
                conflicting_requested_tier_events,
                trust_filtered_empty_events
            ));
            lines.push(format!(
                "group_by={} group_count={}",
                group_by.as_deref().unwrap_or("-"),
                groups.len()
            ));
            for group in groups {
                let group_label = format_optional_summary_group_label(&group.group_value);
                lines.push(format!(
                    "group[{}]={} loaded_events={} triage_counts={} query_requested_tier_counts={} structured_requested_tier_counts={} effective_tier_counts={} filtered_out_tier_counts={} trust_filter_applied_events={} conflicting_requested_tier_events={} trust_filtered_empty_events={} first_timestamp_epoch_s={} last_event_id={} last_timestamp_epoch_s={} last_agent_id={} last_pack_id={} last_query={:?} last_returned={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    group_label,
                    group.loaded_events,
                    format_equals_rollup(&group.triage_counts),
                    format_equals_rollup(&group.query_requested_tier_counts),
                    format_equals_rollup(&group.structured_requested_tier_counts),
                    format_equals_rollup(&group.effective_tier_counts),
                    format_equals_rollup(&group.filtered_out_tier_counts),
                    group.trust_filter_applied_events,
                    group.conflicting_requested_tier_events,
                    group.trust_filtered_empty_events,
                    group
                        .first_timestamp_epoch_s
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    group.last_event_id.as_deref().unwrap_or("-"),
                    group
                        .last_timestamp_epoch_s
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    group.last_agent_id.as_deref().unwrap_or("-"),
                    group.last_pack_id.as_deref().unwrap_or("-"),
                    group.last_query.as_deref().unwrap_or("-"),
                    group
                        .last_returned
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "group_drill_down[{}]={} command={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    group_label,
                    discovery_group_drill_down_command(
                        execution,
                        *limit,
                        group_by.as_deref(),
                        group
                    )
                    .unwrap_or_else(|| "-".to_owned())
                ));
                if let Some(correlated_summary) = &group.correlated_summary {
                    lines.push(format!(
                        "group_correlated_preview[{}]={} loaded_events={} event_kind_counts={} triage_counts={} last_event_id={} last_timestamp_epoch_s={} last_agent_id={}",
                        group_by.as_deref().unwrap_or("unknown"),
                        group_label,
                        correlated_summary.loaded_events,
                        format_equals_rollup(&correlated_summary.event_kind_counts),
                        format_equals_rollup(&correlated_summary.triage_counts),
                        correlated_summary.last_event_id.as_deref().unwrap_or("-"),
                        correlated_summary
                            .last_timestamp_epoch_s
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_owned()),
                        correlated_summary.last_agent_id.as_deref().unwrap_or("-"),
                    ));
                } else {
                    lines.push(format!(
                        "group_correlated_preview[{}]={} preview=-",
                        group_by.as_deref().unwrap_or("unknown"),
                        group_label,
                    ));
                }
                lines.push(format!(
                    "group_correlated_focus[{}]={} additional_events={} non_discovery_event_kind_counts={} non_discovery_triage_counts={} attention_hint={} remediation_hint={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    group_label,
                    group.correlated_additional_events,
                    format_equals_rollup(&group.correlated_non_discovery_event_kind_counts),
                    format_equals_rollup(&group.correlated_non_discovery_triage_counts),
                    group
                        .correlated_attention_hint
                        .as_deref()
                        .unwrap_or("-"),
                    group
                        .correlated_remediation_hint
                        .as_deref()
                        .unwrap_or("-"),
                ));
                lines.push(format!(
                    "group_correlated_summary[{}]={} command={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    group_label,
                    discovery_group_correlated_summary_command(
                        execution,
                        *limit,
                        group_by.as_deref(),
                        group
                    )
                    .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "group_correlated_remediation[{}]={} command={}",
                    group_by.as_deref().unwrap_or("unknown"),
                    group_label,
                    discovery_group_correlated_remediation_command(
                        execution,
                        *limit,
                        group_by.as_deref(),
                        group
                    )
                    .unwrap_or_else(|| "-".to_owned())
                ));
            }
            lines.push(format!(
                "first_timestamp_epoch_s={}",
                first_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ));
            lines.push(format!(
                "last_event_id={} last_timestamp_epoch_s={} last_agent_id={} last_pack_id={}",
                last_event_id.as_deref().unwrap_or("-"),
                last_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_agent_id.as_deref().unwrap_or("-"),
                last_pack_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_query={:?} last_returned={} last_trust_filter_applied={} last_conflicting_requested_tiers={}",
                last_query.as_deref().unwrap_or("-"),
                last_returned
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_trust_filter_applied
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_conflicting_requested_tiers
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ));
            lines.push(format!(
                "last_query_requested_tiers={} last_structured_requested_tiers={} last_effective_tiers={}",
                format_list_or_dash(last_query_requested_tiers),
                format_list_or_dash(last_structured_requested_tiers),
                format_list_or_dash(last_effective_tiers)
            ));
            lines.push(format!(
                "last_filtered_out_candidates={} last_filtered_out_tier_counts={}",
                last_filtered_out_candidates
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                format_equals_rollup(last_filtered_out_tier_counts)
            ));
            lines.push(format!(
                "last_top_provider_ids={}",
                format_list_or_dash(last_top_provider_ids)
            ));
            lines.push(format!(
                "last_triage_event_id={} last_triage_label={} last_triage_timestamp_epoch_s={} last_triage_agent_id={}",
                last_triage_event_id.as_deref().unwrap_or("-"),
                last_triage_label.as_deref().unwrap_or("-"),
                last_triage_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_triage_agent_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_triage_summary={}",
                last_triage_summary.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "last_triage_hint={}",
                last_triage_hint.as_deref().unwrap_or("-")
            ));
        }
        AuditCommandResult::TokenTrail {
            limit,
            token_id,
            loaded_events,
            total_matching_events,
            truncated_matching_events,
            event_kind_counts,
            first_timestamp_epoch_s,
            last_event_id,
            last_timestamp_epoch_s,
            last_agent_id,
            issued_event_id,
            issued_timestamp_epoch_s,
            issued_pack_id,
            issued_agent_id,
            issued_generation,
            issued_expires_at_epoch_s,
            issued_capability_count,
            issued_capabilities,
            authorization_denied_count,
            authorization_denied_reason_counts,
            last_denied_event_id,
            last_denied_timestamp_epoch_s,
            last_denied_pack_id,
            last_denied_agent_id,
            last_denied_reason,
            revoked_event_id,
            revoked_timestamp_epoch_s,
            revoked_agent_id,
            timeline,
        } => {
            lines.push(format!(
                "audit token-trail config={} journal={} token_id={} limit={} loaded_events={} total_matching_events={} truncated_matching_events={}",
                execution.resolved_config_path,
                execution.journal_path,
                token_id,
                limit,
                loaded_events,
                total_matching_events,
                truncated_matching_events
            ));
            lines.push(format!(
                "filters since_epoch_s={} until_epoch_s={} pack_id={} agent_id={} event_id={} token_id={} kind={} triage_label={}",
                execution
                    .since_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution
                    .until_epoch_s_filter
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                execution.pack_id_filter.as_deref().unwrap_or("-"),
                execution.agent_id_filter.as_deref().unwrap_or("-"),
                execution.event_id_filter.as_deref().unwrap_or("-"),
                execution.token_id_filter.as_deref().unwrap_or("-"),
                execution.kind_filter.as_deref().unwrap_or("-"),
                execution.triage_label_filter.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "event_kind_counts={}",
                format_equals_rollup(event_kind_counts)
            ));
            lines.push(format!(
                "first_timestamp_epoch_s={} last_event_id={} last_timestamp_epoch_s={} last_agent_id={}",
                first_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_event_id.as_deref().unwrap_or("-"),
                last_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_agent_id.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "issued_event_id={} issued_timestamp_epoch_s={} issued_pack_id={} issued_agent_id={} issued_generation={} issued_expires_at_epoch_s={}",
                issued_event_id.as_deref().unwrap_or("-"),
                issued_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                issued_pack_id.as_deref().unwrap_or("-"),
                issued_agent_id.as_deref().unwrap_or("-"),
                issued_generation
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                issued_expires_at_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            ));
            lines.push(format!(
                "issued_capability_count={} issued_capabilities={}",
                issued_capability_count
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                format_list_or_dash(issued_capabilities)
            ));
            lines.push(format!(
                "authorization_denied_count={} authorization_denied_reason_counts={}",
                authorization_denied_count,
                format_equals_rollup(authorization_denied_reason_counts)
            ));
            lines.push(format!(
                "last_denied_event_id={} last_denied_timestamp_epoch_s={} last_denied_pack_id={} last_denied_agent_id={} last_denied_reason={}",
                last_denied_event_id.as_deref().unwrap_or("-"),
                last_denied_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                last_denied_pack_id.as_deref().unwrap_or("-"),
                last_denied_agent_id.as_deref().unwrap_or("-"),
                last_denied_reason.as_deref().unwrap_or("-")
            ));
            lines.push(format!(
                "revoked_event_id={} revoked_timestamp_epoch_s={} revoked_agent_id={}",
                revoked_event_id.as_deref().unwrap_or("-"),
                revoked_timestamp_epoch_s
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                revoked_agent_id.as_deref().unwrap_or("-")
            ));
            lines.push("timeline:".to_owned());
            for event in timeline {
                let detail = format_audit_event_detail(&event.kind);
                lines.push(format!(
                    "- ts={} event_id={} agent_id={} kind={} {}",
                    event.timestamp_epoch_s,
                    event.event_id,
                    event.agent_id.as_deref().unwrap_or("-"),
                    audit_event_kind_label(&event.kind),
                    detail
                ));
            }
        }
    }
    Ok(lines.join("\n"))
}

fn validate_audit_limit(limit: usize, command_name: &str) -> CliResult<usize> {
    if !(1..=MAX_AUDIT_WINDOW).contains(&limit) {
        return Err(format!(
            "{command_name} limit must be between 1 and {MAX_AUDIT_WINDOW}"
        ));
    }

    Ok(limit)
}

fn validate_audit_time_range(
    since_epoch_s: Option<u64>,
    until_epoch_s: Option<u64>,
    command_name: &str,
) -> CliResult<()> {
    if let (Some(since_epoch_s), Some(until_epoch_s)) = (since_epoch_s, until_epoch_s)
        && until_epoch_s < since_epoch_s
    {
        return Err(format!(
            "{command_name} until_epoch_s must be greater than or equal to since_epoch_s"
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AuditEventFilter {
    since_epoch_s: Option<u64>,
    until_epoch_s: Option<u64>,
    pack_id: Option<String>,
    agent_id: Option<String>,
    event_id: Option<String>,
    token_id: Option<String>,
    kind: Option<String>,
    triage_label: Option<String>,
    query_contains: Option<String>,
    trust_tier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuditEventWindow {
    events: Vec<AuditEvent>,
    total_matching_events: usize,
}

fn load_audit_event_window(
    audit: &crate::mvp::config::AuditConfig,
    journal_path: &Path,
    limit: usize,
    filter: &AuditEventFilter,
) -> CliResult<AuditEventWindow> {
    if !journal_path.exists() {
        let hint = if audit.mode == crate::mvp::config::AuditMode::InMemory {
            "durable audit retention is disabled because [audit].mode = \"in_memory\""
        } else {
            "journal is created on first audit write"
        };
        return Err(format!(
            "audit journal not found at {} ({hint})",
            journal_path.display()
        ));
    }
    if !journal_path.is_file() {
        return Err(format!(
            "audit journal path {} exists but is not a file",
            journal_path.display()
        ));
    }

    let file = File::open(journal_path).map_err(|error| {
        format!(
            "open audit journal {} failed: {error}",
            journal_path.display()
        )
    })?;
    file.lock_shared().map_err(|error| {
        format!(
            "lock audit journal {} for reading failed: {error}",
            journal_path.display()
        )
    })?;
    let reader = BufReader::new(file);
    let mut window = VecDeque::new();
    let mut total_matching_events = 0;
    for (index, line_result) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line_result.map_err(|error| {
            format!(
                "read audit journal {} failed at line {}: {error}",
                journal_path.display(),
                line_number
            )
        })?;
        let event = serde_json::from_str::<AuditEvent>(&line).map_err(|error| {
            format!(
                "decode audit journal {} failed at line {}: {error}",
                journal_path.display(),
                line_number
            )
        })?;
        if !event_matches_filter(&event, filter) {
            continue;
        }
        total_matching_events += 1;
        if window.len() == limit {
            let _ = window.pop_front();
        }
        window.push_back(event);
    }
    Ok(AuditEventWindow {
        events: window.into_iter().collect(),
        total_matching_events,
    })
}

fn summarize_audit_events(
    limit: usize,
    group_by: Option<String>,
    events: &[AuditEvent],
) -> AuditCommandResult {
    let mut event_kind_counts = BTreeMap::new();
    let mut triage_counts = BTreeMap::new();
    for event in events {
        let label = audit_event_kind_label(&event.kind).to_owned();
        *event_kind_counts.entry(label).or_insert(0) += 1;
        if let Some(triage_label) = triage_event_label(&event.kind) {
            *triage_counts.entry(triage_label.to_owned()).or_insert(0) += 1;
        }
    }
    let first = events.first();
    let last = events.last();
    let last_triage = events
        .iter()
        .rev()
        .find(|event| triage_event_label(&event.kind).is_some());
    let groups = summarize_audit_groups(events, group_by.as_deref());

    AuditCommandResult::Summary {
        limit,
        loaded_events: events.len(),
        event_kind_counts,
        triage_counts,
        group_by,
        groups,
        first_timestamp_epoch_s: first.map(|event| event.timestamp_epoch_s),
        last_event_id: last.map(|event| event.event_id.clone()),
        last_timestamp_epoch_s: last.map(|event| event.timestamp_epoch_s),
        last_agent_id: last.and_then(|event| event.agent_id.clone()),
        last_triage_event_id: last_triage.map(|event| event.event_id.clone()),
        last_triage_label: last_triage
            .and_then(|event| triage_event_label(&event.kind))
            .map(str::to_owned),
        last_triage_event_kind: last_triage
            .map(|event| audit_event_kind_label(&event.kind).to_owned()),
        last_triage_timestamp_epoch_s: last_triage.map(|event| event.timestamp_epoch_s),
        last_triage_agent_id: last_triage.and_then(|event| event.agent_id.clone()),
        last_triage_summary: last_triage.and_then(|event| triage_event_summary(&event.kind)),
        last_triage_hint: last_triage.and_then(|event| triage_event_hint(&event.kind)),
    }
}

fn summarize_audit_groups(events: &[AuditEvent], group_by: Option<&str>) -> Vec<AuditSummaryGroup> {
    let Some(group_by) = group_by else {
        return Vec::new();
    };

    let mut grouped_events: BTreeMap<Option<String>, Vec<&AuditEvent>> = BTreeMap::new();
    for event in events {
        grouped_events
            .entry(summary_group_value(event, group_by))
            .or_default()
            .push(event);
    }

    let mut groups = grouped_events
        .into_iter()
        .map(|(group_value, group_events)| {
            let mut event_kind_counts = BTreeMap::new();
            let mut triage_counts = BTreeMap::new();
            for event in &group_events {
                let label = audit_event_kind_label(&event.kind).to_owned();
                *event_kind_counts.entry(label).or_insert(0) += 1;
                if let Some(triage_label) = triage_event_label(&event.kind) {
                    *triage_counts.entry(triage_label.to_owned()).or_insert(0) += 1;
                }
            }
            let first = group_events.first().copied();
            let last = group_events.last().copied();

            AuditSummaryGroup {
                group_value,
                loaded_events: group_events.len(),
                event_kind_counts,
                triage_counts,
                first_timestamp_epoch_s: first.map(|event| event.timestamp_epoch_s),
                last_event_id: last.map(|event| event.event_id.clone()),
                last_timestamp_epoch_s: last.map(|event| event.timestamp_epoch_s),
                last_agent_id: last.and_then(|event| event.agent_id.clone()),
            }
        })
        .collect::<Vec<_>>();

    groups.sort_by(|left, right| {
        right.loaded_events.cmp(&left.loaded_events).then_with(|| {
            match (&left.group_value, &right.group_value) {
                (Some(left_value), Some(right_value)) => left_value.cmp(right_value),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        })
    });
    groups
}

fn summarize_discovery_events(
    limit: usize,
    group_by: Option<String>,
    events: &[AuditEvent],
    correlated_summary_groups: &[AuditSummaryGroup],
) -> AuditCommandResult {
    let mut triage_counts = BTreeMap::new();
    let mut query_requested_tier_counts = BTreeMap::new();
    let mut structured_requested_tier_counts = BTreeMap::new();
    let mut effective_tier_counts = BTreeMap::new();
    let mut filtered_out_tier_counts = BTreeMap::new();
    let mut trust_filter_applied_events = 0;
    let mut conflicting_requested_tier_events = 0;
    let mut trust_filtered_empty_events = 0;

    for event in events {
        let Some(context) = tool_search_event_context(&event.kind) else {
            continue;
        };

        let triage_label = triage_event_label(&event.kind);
        if let Some(triage_label) = triage_label {
            *triage_counts.entry(triage_label.to_owned()).or_insert(0) += 1;
        }
        if context.trust_filter_applied {
            trust_filter_applied_events += 1;
        }
        if context.conflicting_requested_tiers {
            conflicting_requested_tier_events += 1;
        }
        if triage_label == Some("tool_search_trust_empty") {
            trust_filtered_empty_events += 1;
        }

        increment_tier_counts(
            &mut query_requested_tier_counts,
            context.query_requested_tiers,
        );
        increment_tier_counts(
            &mut structured_requested_tier_counts,
            context.structured_requested_tiers,
        );
        increment_tier_counts(&mut effective_tier_counts, context.effective_tiers);
        increment_count_rollup(
            &mut filtered_out_tier_counts,
            context.filtered_out_tier_counts,
        );
    }

    let first = events.first();
    let last = events.last();
    let last_context = last.and_then(|event| tool_search_event_context(&event.kind));
    let last_triage = events
        .iter()
        .rev()
        .find(|event| triage_event_label(&event.kind).is_some());
    let correlated_summary_groups = correlated_summary_groups
        .iter()
        .cloned()
        .map(|group| (group.group_value.clone(), group))
        .collect::<BTreeMap<_, _>>();
    let groups =
        summarize_discovery_groups(events, group_by.as_deref(), &correlated_summary_groups);

    AuditCommandResult::Discovery {
        limit,
        loaded_events: events.len(),
        triage_counts,
        query_requested_tier_counts,
        structured_requested_tier_counts,
        effective_tier_counts,
        filtered_out_tier_counts,
        trust_filter_applied_events,
        conflicting_requested_tier_events,
        trust_filtered_empty_events,
        group_by,
        groups,
        first_timestamp_epoch_s: first.map(|event| event.timestamp_epoch_s),
        last_event_id: last.map(|event| event.event_id.clone()),
        last_timestamp_epoch_s: last.map(|event| event.timestamp_epoch_s),
        last_agent_id: last.and_then(|event| event.agent_id.clone()),
        last_pack_id: last_context.map(|context| context.pack_id.to_owned()),
        last_query: last_context.map(|context| context.query.to_owned()),
        last_returned: last_context.map(|context| context.returned),
        last_trust_filter_applied: last_context.map(|context| context.trust_filter_applied),
        last_conflicting_requested_tiers: last_context
            .map(|context| context.conflicting_requested_tiers),
        last_query_requested_tiers: last_context
            .map(|context| context.query_requested_tiers.to_vec())
            .unwrap_or_default(),
        last_structured_requested_tiers: last_context
            .map(|context| context.structured_requested_tiers.to_vec())
            .unwrap_or_default(),
        last_effective_tiers: last_context
            .map(|context| context.effective_tiers.to_vec())
            .unwrap_or_default(),
        last_filtered_out_candidates: last_context.map(|context| context.filtered_out_candidates),
        last_filtered_out_tier_counts: last_context
            .map(|context| context.filtered_out_tier_counts.clone())
            .unwrap_or_default(),
        last_top_provider_ids: last_context
            .map(|context| context.top_provider_ids.to_vec())
            .unwrap_or_default(),
        last_triage_event_id: last_triage.map(|event| event.event_id.clone()),
        last_triage_label: last_triage
            .and_then(|event| triage_event_label(&event.kind))
            .map(str::to_owned),
        last_triage_timestamp_epoch_s: last_triage.map(|event| event.timestamp_epoch_s),
        last_triage_agent_id: last_triage.and_then(|event| event.agent_id.clone()),
        last_triage_summary: last_triage.and_then(|event| triage_event_summary(&event.kind)),
        last_triage_hint: last_triage.and_then(|event| triage_event_hint(&event.kind)),
    }
}

fn summarize_discovery_groups(
    events: &[AuditEvent],
    group_by: Option<&str>,
    correlated_summary_groups: &BTreeMap<Option<String>, AuditSummaryGroup>,
) -> Vec<AuditDiscoveryGroup> {
    let Some(group_by) = group_by else {
        return Vec::new();
    };

    let mut grouped_events: BTreeMap<Option<String>, Vec<&AuditEvent>> = BTreeMap::new();
    for event in events {
        grouped_events
            .entry(discovery_group_value(event, group_by))
            .or_default()
            .push(event);
    }

    let mut groups = grouped_events
        .into_iter()
        .map(|(group_value, group_events)| {
            let mut triage_counts = BTreeMap::new();
            let mut query_requested_tier_counts = BTreeMap::new();
            let mut structured_requested_tier_counts = BTreeMap::new();
            let mut effective_tier_counts = BTreeMap::new();
            let mut filtered_out_tier_counts = BTreeMap::new();
            let mut trust_filter_applied_events = 0;
            let mut conflicting_requested_tier_events = 0;
            let mut trust_filtered_empty_events = 0;

            for event in &group_events {
                let Some(context) = tool_search_event_context(&event.kind) else {
                    continue;
                };
                let triage_label = triage_event_label(&event.kind);
                if let Some(triage_label) = triage_label {
                    *triage_counts.entry(triage_label.to_owned()).or_insert(0) += 1;
                }
                if context.trust_filter_applied {
                    trust_filter_applied_events += 1;
                }
                if context.conflicting_requested_tiers {
                    conflicting_requested_tier_events += 1;
                }
                if triage_label == Some("tool_search_trust_empty") {
                    trust_filtered_empty_events += 1;
                }
                increment_tier_counts(
                    &mut query_requested_tier_counts,
                    context.query_requested_tiers,
                );
                increment_tier_counts(
                    &mut structured_requested_tier_counts,
                    context.structured_requested_tiers,
                );
                increment_tier_counts(&mut effective_tier_counts, context.effective_tiers);
                increment_count_rollup(
                    &mut filtered_out_tier_counts,
                    context.filtered_out_tier_counts,
                );
            }

            let first = group_events.first().copied();
            let last = group_events.last().copied();
            let last_context = last.and_then(|event| tool_search_event_context(&event.kind));
            let correlated_summary = correlated_summary_groups.get(&group_value).cloned();
            let correlated_additional_events = correlated_summary
                .as_ref()
                .map(|summary| summary.loaded_events.saturating_sub(group_events.len()))
                .unwrap_or(0);
            let correlated_non_discovery_event_kind_counts = correlated_summary
                .as_ref()
                .map(|summary| non_discovery_event_kind_counts(&summary.event_kind_counts))
                .unwrap_or_default();
            let correlated_non_discovery_triage_counts = correlated_summary
                .as_ref()
                .map(|summary| non_discovery_triage_counts(&summary.triage_counts))
                .unwrap_or_default();
            let correlated_attention_hint = correlated_attention_hint(
                correlated_additional_events,
                &correlated_non_discovery_event_kind_counts,
                &correlated_non_discovery_triage_counts,
            );
            let correlated_remediation_hint = correlated_remediation_hint(
                correlated_additional_events,
                &correlated_non_discovery_event_kind_counts,
                &correlated_non_discovery_triage_counts,
            );

            AuditDiscoveryGroup {
                correlated_summary,
                group_value,
                loaded_events: group_events.len(),
                triage_counts,
                query_requested_tier_counts,
                structured_requested_tier_counts,
                effective_tier_counts,
                filtered_out_tier_counts,
                trust_filter_applied_events,
                conflicting_requested_tier_events,
                trust_filtered_empty_events,
                first_timestamp_epoch_s: first.map(|event| event.timestamp_epoch_s),
                last_event_id: last.map(|event| event.event_id.clone()),
                last_timestamp_epoch_s: last.map(|event| event.timestamp_epoch_s),
                last_agent_id: last.and_then(|event| event.agent_id.clone()),
                last_pack_id: last_context.map(|context| context.pack_id.to_owned()),
                last_query: last_context.map(|context| context.query.to_owned()),
                last_returned: last_context.map(|context| context.returned),
                correlated_additional_events,
                correlated_non_discovery_event_kind_counts,
                correlated_non_discovery_triage_counts,
                correlated_attention_hint,
                correlated_remediation_hint,
            }
        })
        .collect::<Vec<_>>();

    groups.sort_by(|left, right| {
        right.loaded_events.cmp(&left.loaded_events).then_with(|| {
            match (&left.group_value, &right.group_value) {
                (Some(left_value), Some(right_value)) => left_value.cmp(right_value),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        })
    });
    groups
}

fn summarize_token_trail(
    limit: usize,
    token_id: String,
    events: Vec<AuditEvent>,
    total_matching_events: usize,
) -> AuditCommandResult {
    let mut event_kind_counts = BTreeMap::new();
    let mut authorization_denied_reason_counts = BTreeMap::new();
    let mut authorization_denied_count = 0;

    for event in &events {
        let label = audit_event_kind_label(&event.kind).to_owned();
        *event_kind_counts.entry(label).or_insert(0) += 1;

        if let AuditEventKind::AuthorizationDenied { reason, .. } = &event.kind {
            authorization_denied_count += 1;
            *authorization_denied_reason_counts
                .entry(reason.clone())
                .or_insert(0) += 1;
        }
    }

    let first = events.first();
    let last = events.last();
    let issued = events.iter().find_map(|event| {
        if let AuditEventKind::TokenIssued { token } = &event.kind {
            return Some((event, token));
        }

        None
    });
    let last_denied = events.iter().rev().find_map(|event| {
        if let AuditEventKind::AuthorizationDenied {
            pack_id, reason, ..
        } = &event.kind
        {
            return Some((event, pack_id.as_str(), reason.as_str()));
        }

        None
    });
    let revoked = events
        .iter()
        .rev()
        .find(|event| matches!(&event.kind, AuditEventKind::TokenRevoked { .. }));

    let (issued_capability_count, issued_capabilities) = issued
        .map(|(_, token)| {
            let capabilities = token
                .allowed_capabilities
                .iter()
                .map(|capability| capability.as_str().to_owned())
                .collect::<Vec<_>>();
            (Some(capabilities.len()), capabilities)
        })
        .unwrap_or_else(|| (None, Vec::new()));

    AuditCommandResult::TokenTrail {
        limit,
        token_id,
        loaded_events: events.len(),
        total_matching_events,
        truncated_matching_events: total_matching_events.saturating_sub(events.len()),
        event_kind_counts,
        first_timestamp_epoch_s: first.map(|event| event.timestamp_epoch_s),
        last_event_id: last.map(|event| event.event_id.clone()),
        last_timestamp_epoch_s: last.map(|event| event.timestamp_epoch_s),
        last_agent_id: last.and_then(|event| event.agent_id.clone()),
        issued_event_id: issued.map(|(event, _)| event.event_id.clone()),
        issued_timestamp_epoch_s: issued.map(|(event, _)| event.timestamp_epoch_s),
        issued_pack_id: issued.map(|(_, token)| token.pack_id.clone()),
        issued_agent_id: issued.map(|(_, token)| token.agent_id.clone()),
        issued_generation: issued.map(|(_, token)| token.generation),
        issued_expires_at_epoch_s: issued.map(|(_, token)| token.expires_at_epoch_s),
        issued_capability_count,
        issued_capabilities,
        authorization_denied_count,
        authorization_denied_reason_counts,
        last_denied_event_id: last_denied.map(|(event, _, _)| event.event_id.clone()),
        last_denied_timestamp_epoch_s: last_denied.map(|(event, _, _)| event.timestamp_epoch_s),
        last_denied_pack_id: last_denied.map(|(_, pack_id, _)| pack_id.to_owned()),
        last_denied_agent_id: last_denied.and_then(|(event, _, _)| event.agent_id.clone()),
        last_denied_reason: last_denied.map(|(_, _, reason)| reason.to_owned()),
        revoked_event_id: revoked.map(|event| event.event_id.clone()),
        revoked_timestamp_epoch_s: revoked.map(|event| event.timestamp_epoch_s),
        revoked_agent_id: revoked.and_then(|event| event.agent_id.clone()),
        timeline: events,
    }
}

fn parse_audit_event_kind_filter(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "tokenissued" => "TokenIssued",
        "tokenrevoked" => "TokenRevoked",
        "taskdispatched" => "TaskDispatched",
        "connectorinvoked" => "ConnectorInvoked",
        "planeinvoked" => "PlaneInvoked",
        "securityscanevaluated" => "SecurityScanEvaluated",
        "plugintrustevaluated" => "PluginTrustEvaluated",
        "toolsearchevaluated" => "ToolSearchEvaluated",
        "providerfailover" => "ProviderFailover",
        "authorizationdenied" => "AuthorizationDenied",
        _ => {
            return Err(format!(
                "unsupported audit event kind filter `{raw}` (expected one of: TokenIssued, TokenRevoked, TaskDispatched, ConnectorInvoked, PlaneInvoked, SecurityScanEvaluated, PluginTrustEvaluated, ToolSearchEvaluated, ProviderFailover, AuthorizationDenied)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn parse_audit_triage_label_filter(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "authorizationdenied" => "authorization_denied",
        "providerfailover" => "provider_failover",
        "securityscanblocked" => "security_scan_blocked",
        "plugintrustblocked" => "plugin_trust_blocked",
        "toolsearchtrustconflict" => "tool_search_trust_conflict",
        "toolsearchtrustempty" => "tool_search_trust_empty",
        _ => {
            return Err(format!(
                "unsupported audit triage label filter `{raw}` (expected one of: authorization_denied, provider_failover, security_scan_blocked, plugin_trust_blocked, tool_search_trust_conflict, tool_search_trust_empty)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn parse_audit_summary_group_by(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "pack" | "packid" => "pack",
        "agent" | "agentid" => "agent",
        "token" | "tokenid" => "token",
        _ => {
            return Err(format!(
                "unsupported audit summary group-by `{raw}` (expected one of: pack, agent, token)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn parse_audit_discovery_group_by(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "pack" | "packid" => "pack",
        "agent" | "agentid" => "agent",
        _ => {
            return Err(format!(
                "unsupported audit discovery group-by `{raw}` (expected one of: pack, agent)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn parse_tool_search_triage_label_filter(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "conflict" | "trustconflict" | "toolsearchtrustconflict" => "tool_search_trust_conflict",
        "empty" | "trustempty" | "toolsearchtrustempty" => "tool_search_trust_empty",
        _ => {
            return Err(format!(
                "unsupported discovery triage label filter `{raw}` (expected one of: tool_search_trust_conflict, tool_search_trust_empty, conflict, empty)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn parse_audit_query_contains_filter(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("query filter must not be empty".to_owned());
    }

    Ok(trimmed.to_owned())
}

fn parse_audit_identity_filter(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("identity filter must not be empty".to_owned());
    }

    Ok(trimmed.to_owned())
}

fn parse_plugin_trust_tier_filter(raw: &str) -> Result<String, String> {
    let normalized = normalize_audit_filter_token(raw);
    let canonical = match normalized.as_str() {
        "official" => PluginTrustTier::Official.as_str(),
        "verifiedcommunity" => PluginTrustTier::VerifiedCommunity.as_str(),
        "unverified" => PluginTrustTier::Unverified.as_str(),
        _ => {
            return Err(format!(
                "unsupported trust tier filter `{raw}` (expected one of: official, verified-community, unverified)"
            ));
        }
    };

    Ok(canonical.to_owned())
}

fn normalize_audit_filter_token(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn summary_group_value(event: &AuditEvent, group_by: &str) -> Option<String> {
    match group_by {
        "pack" => audit_event_pack_id(&event.kind).map(str::to_owned),
        "agent" => event.agent_id.clone(),
        "token" => audit_event_token_id(&event.kind).map(str::to_owned),
        _ => None,
    }
}

fn discovery_group_value(event: &AuditEvent, group_by: &str) -> Option<String> {
    match group_by {
        "pack" => audit_event_pack_id(&event.kind).map(str::to_owned),
        "agent" => event.agent_id.clone(),
        _ => None,
    }
}

fn event_matches_filter(event: &AuditEvent, filter: &AuditEventFilter) -> bool {
    if let Some(since_epoch_s) = filter.since_epoch_s
        && event.timestamp_epoch_s < since_epoch_s
    {
        return false;
    }

    if let Some(until_epoch_s) = filter.until_epoch_s
        && event.timestamp_epoch_s > until_epoch_s
    {
        return false;
    }

    if let Some(pack_id_filter) = filter.pack_id.as_deref()
        && audit_event_pack_id(&event.kind) != Some(pack_id_filter)
    {
        return false;
    }

    if let Some(agent_id_filter) = filter.agent_id.as_deref()
        && event.agent_id.as_deref() != Some(agent_id_filter)
    {
        return false;
    }

    if let Some(event_id_filter) = filter.event_id.as_deref()
        && event.event_id != event_id_filter
    {
        return false;
    }

    if let Some(token_id_filter) = filter.token_id.as_deref()
        && audit_event_token_id(&event.kind) != Some(token_id_filter)
    {
        return false;
    }

    if let Some(kind_filter) = filter.kind.as_deref()
        && audit_event_kind_label(&event.kind) != kind_filter
    {
        return false;
    }

    if let Some(triage_label_filter) = filter.triage_label.as_deref()
        && triage_event_label(&event.kind) != Some(triage_label_filter)
    {
        return false;
    }

    if let Some(query_contains_filter) = filter.query_contains.as_deref() {
        let Some(context) = tool_search_event_context(&event.kind) else {
            return false;
        };
        if !context
            .query
            .to_lowercase()
            .contains(&query_contains_filter.to_lowercase())
        {
            return false;
        }
    }

    if let Some(trust_tier_filter) = filter.trust_tier.as_deref() {
        let Some(context) = tool_search_event_context(&event.kind) else {
            return false;
        };
        if !context
            .query_requested_tiers
            .iter()
            .chain(context.structured_requested_tiers.iter())
            .chain(context.effective_tiers.iter())
            .any(|tier| tier == trust_tier_filter)
        {
            return false;
        }
    }

    true
}

#[derive(Debug, Clone, Copy)]
struct ToolSearchAuditEventContext<'a> {
    pack_id: &'a str,
    query: &'a str,
    returned: usize,
    trust_filter_applied: bool,
    query_requested_tiers: &'a [String],
    structured_requested_tiers: &'a [String],
    effective_tiers: &'a [String],
    conflicting_requested_tiers: bool,
    filtered_out_candidates: usize,
    filtered_out_tier_counts: &'a BTreeMap<String, usize>,
    top_provider_ids: &'a [String],
}

fn tool_search_event_context(kind: &AuditEventKind) -> Option<ToolSearchAuditEventContext<'_>> {
    if let AuditEventKind::ToolSearchEvaluated {
        pack_id,
        query,
        returned,
        trust_filter_applied,
        query_requested_tiers,
        structured_requested_tiers,
        effective_tiers,
        conflicting_requested_tiers,
        filtered_out_candidates,
        filtered_out_tier_counts,
        top_provider_ids,
    } = kind
    {
        return Some(ToolSearchAuditEventContext {
            pack_id,
            query,
            returned: *returned,
            trust_filter_applied: *trust_filter_applied,
            query_requested_tiers,
            structured_requested_tiers,
            effective_tiers,
            conflicting_requested_tiers: *conflicting_requested_tiers,
            filtered_out_candidates: *filtered_out_candidates,
            filtered_out_tier_counts,
            top_provider_ids,
        });
    }

    None
}

fn audit_event_pack_id(kind: &AuditEventKind) -> Option<&str> {
    match kind {
        AuditEventKind::TokenIssued { token } => Some(token.pack_id.as_str()),
        AuditEventKind::TaskDispatched { pack_id, .. }
        | AuditEventKind::ConnectorInvoked { pack_id, .. }
        | AuditEventKind::PlaneInvoked { pack_id, .. }
        | AuditEventKind::SecurityScanEvaluated { pack_id, .. }
        | AuditEventKind::PluginTrustEvaluated { pack_id, .. }
        | AuditEventKind::ToolSearchEvaluated { pack_id, .. }
        | AuditEventKind::ProviderFailover { pack_id, .. }
        | AuditEventKind::AuthorizationDenied { pack_id, .. } => Some(pack_id.as_str()),
        AuditEventKind::TokenRevoked { .. } => None,
        _ => None,
    }
}

fn audit_event_token_id(kind: &AuditEventKind) -> Option<&str> {
    if let AuditEventKind::TokenIssued { token } = kind {
        return Some(token.token_id.as_str());
    }

    if let AuditEventKind::TokenRevoked { token_id } = kind {
        return Some(token_id.as_str());
    }

    if let AuditEventKind::AuthorizationDenied { token_id, .. } = kind {
        return Some(token_id.as_str());
    }

    None
}

fn increment_tier_counts(counts: &mut BTreeMap<String, usize>, tiers: &[String]) {
    for tier in tiers {
        *counts.entry(tier.clone()).or_insert(0) += 1;
    }
}

fn increment_count_rollup(
    counts: &mut BTreeMap<String, usize>,
    additions: &BTreeMap<String, usize>,
) {
    for (label, count) in additions {
        *counts.entry(label.clone()).or_insert(0) += count;
    }
}

fn triage_event_summary(kind: &AuditEventKind) -> Option<String> {
    if let AuditEventKind::AuthorizationDenied {
        pack_id,
        token_id,
        reason,
    } = kind
    {
        let summary = format!(
            "pack_id={} token_id={} reason={}",
            pack_id, token_id, reason
        );
        return Some(summary);
    }

    if let AuditEventKind::ProviderFailover {
        pack_id,
        provider_id,
        reason,
        attempt,
        max_attempts,
        ..
    } = kind
    {
        let summary = format!(
            "pack_id={} provider_id={} reason={} attempt={}/{}",
            pack_id, provider_id, reason, attempt, max_attempts
        );
        return Some(summary);
    }

    if let AuditEventKind::SecurityScanEvaluated {
        pack_id,
        total_findings,
        high_findings,
        block_reason,
        ..
    } = kind
    {
        let block_reason_label = block_reason.as_deref().unwrap_or("-");
        let summary = format!(
            "pack_id={} total_findings={} high_findings={} block_reason={}",
            pack_id, total_findings, high_findings, block_reason_label
        );
        return Some(summary);
    }

    if let AuditEventKind::PluginTrustEvaluated {
        pack_id,
        blocked_auto_apply_plugins,
        review_required_plugin_ids,
        ..
    } = kind
    {
        let review_required_plugins = if review_required_plugin_ids.is_empty() {
            "-".to_owned()
        } else {
            review_required_plugin_ids.join(",")
        };
        let summary = format!(
            "pack_id={} blocked_auto_apply_plugins={} review_required_plugins={}",
            pack_id, blocked_auto_apply_plugins, review_required_plugins
        );
        return Some(summary);
    }

    if let AuditEventKind::ToolSearchEvaluated {
        query,
        effective_tiers,
        conflicting_requested_tiers,
        filtered_out_candidates,
        top_provider_ids,
        ..
    } = kind
    {
        let trust_scope = if effective_tiers.is_empty() {
            "-".to_owned()
        } else {
            effective_tiers.join(",")
        };
        let provider_ids = if top_provider_ids.is_empty() {
            "-".to_owned()
        } else {
            top_provider_ids.join(",")
        };
        let summary = format!(
            "query={query:?} trust_scope={} conflicting_requested_tiers={} filtered_out_candidates={} top_provider_ids={}",
            trust_scope, conflicting_requested_tiers, filtered_out_candidates, provider_ids
        );
        return Some(summary);
    }

    None
}

fn triage_event_hint(kind: &AuditEventKind) -> Option<String> {
    triage_event_label(kind)
        .and_then(triage_label_remediation_hint)
        .map(str::to_owned)
}

fn triage_event_label(kind: &AuditEventKind) -> Option<&'static str> {
    match kind {
        AuditEventKind::AuthorizationDenied { .. } => Some("authorization_denied"),
        AuditEventKind::ProviderFailover { .. } => Some("provider_failover"),
        AuditEventKind::SecurityScanEvaluated { blocked: true, .. } => {
            Some("security_scan_blocked")
        }
        AuditEventKind::PluginTrustEvaluated {
            blocked_auto_apply_plugins,
            ..
        } if *blocked_auto_apply_plugins > 0 => Some("plugin_trust_blocked"),
        AuditEventKind::ToolSearchEvaluated {
            conflicting_requested_tiers: true,
            ..
        } => Some("tool_search_trust_conflict"),
        AuditEventKind::ToolSearchEvaluated {
            trust_filter_applied: true,
            returned: 0,
            filtered_out_candidates,
            ..
        } if *filtered_out_candidates > 0 => Some("tool_search_trust_empty"),
        AuditEventKind::TokenIssued { .. }
        | AuditEventKind::TokenRevoked { .. }
        | AuditEventKind::TaskDispatched { .. }
        | AuditEventKind::ConnectorInvoked { .. }
        | AuditEventKind::PlaneInvoked { .. }
        | AuditEventKind::SecurityScanEvaluated { .. }
        | AuditEventKind::PluginTrustEvaluated { .. }
        | AuditEventKind::ToolSearchEvaluated { .. } => None,
        _ => None,
    }
}

fn audit_event_kind_label(kind: &AuditEventKind) -> &'static str {
    match kind {
        AuditEventKind::TokenIssued { .. } => "TokenIssued",
        AuditEventKind::TokenRevoked { .. } => "TokenRevoked",
        AuditEventKind::TaskDispatched { .. } => "TaskDispatched",
        AuditEventKind::ConnectorInvoked { .. } => "ConnectorInvoked",
        AuditEventKind::PlaneInvoked { .. } => "PlaneInvoked",
        AuditEventKind::SecurityScanEvaluated { .. } => "SecurityScanEvaluated",
        AuditEventKind::PluginTrustEvaluated { .. } => "PluginTrustEvaluated",
        AuditEventKind::ToolSearchEvaluated { .. } => "ToolSearchEvaluated",
        AuditEventKind::ProviderFailover { .. } => "ProviderFailover",
        AuditEventKind::AuthorizationDenied { .. } => "AuthorizationDenied",
        // AuditEventKind is non_exhaustive in loongclaw-contracts, so keep a visible
        // fallback label instead of silently collapsing future variants into "Unknown".
        _ => "UnknownAuditEventKind",
    }
}

fn format_audit_event_detail(kind: &AuditEventKind) -> String {
    match kind {
        AuditEventKind::TokenIssued { token } => format!(
            "pack_id={} token_id={} expires_at_epoch_s={}",
            token.pack_id, token.token_id, token.expires_at_epoch_s
        ),
        AuditEventKind::TokenRevoked { token_id } => format!("token_id={token_id}"),
        AuditEventKind::TaskDispatched {
            pack_id,
            task_id,
            route,
            ..
        } => format!(
            "pack_id={} task_id={} harness={:?} adapter={}",
            pack_id,
            task_id,
            route.harness_kind,
            route.adapter.as_deref().unwrap_or("-")
        ),
        AuditEventKind::ConnectorInvoked {
            pack_id,
            connector_name,
            operation,
            ..
        } => format!(
            "pack_id={} connector={} operation={}",
            pack_id, connector_name, operation
        ),
        AuditEventKind::PlaneInvoked {
            pack_id,
            plane,
            tier,
            primary_adapter,
            operation,
            ..
        } => format!(
            "pack_id={} plane={:?} tier={:?} adapter={} operation={}",
            pack_id, plane, tier, primary_adapter, operation
        ),
        AuditEventKind::SecurityScanEvaluated {
            pack_id,
            total_findings,
            blocked,
            ..
        } => format!(
            "pack_id={} total_findings={} blocked={}",
            pack_id, total_findings, blocked
        ),
        AuditEventKind::PluginTrustEvaluated {
            pack_id,
            scanned_plugins,
            high_risk_unverified_plugins,
            blocked_auto_apply_plugins,
            ..
        } => format!(
            "pack_id={} scanned_plugins={} high_risk_unverified_plugins={} blocked_auto_apply_plugins={}",
            pack_id, scanned_plugins, high_risk_unverified_plugins, blocked_auto_apply_plugins
        ),
        AuditEventKind::ToolSearchEvaluated {
            pack_id,
            query,
            returned,
            trust_filter_applied,
            effective_tiers,
            conflicting_requested_tiers,
            filtered_out_candidates,
            filtered_out_tier_counts,
            top_provider_ids,
            ..
        } => format!(
            "pack_id={} query={query:?} returned={} trust_filter_applied={} trust_scope={} conflicting_requested_tiers={} filtered_out_candidates={} filtered_out_tier_counts={} top_provider_ids={}",
            pack_id,
            returned,
            trust_filter_applied,
            if effective_tiers.is_empty() {
                "-".to_owned()
            } else {
                effective_tiers.join(",")
            },
            conflicting_requested_tiers,
            filtered_out_candidates,
            format_equals_rollup(filtered_out_tier_counts),
            if top_provider_ids.is_empty() {
                "-".to_owned()
            } else {
                top_provider_ids.join(",")
            }
        ),
        AuditEventKind::ProviderFailover {
            pack_id,
            provider_id,
            reason,
            attempt,
            max_attempts,
            ..
        } => format!(
            "pack_id={} provider_id={} reason={} attempt={}/{}",
            pack_id, provider_id, reason, attempt, max_attempts
        ),
        AuditEventKind::AuthorizationDenied {
            pack_id,
            token_id,
            reason,
        } => format!(
            "pack_id={} token_id={} reason={}",
            pack_id, token_id, reason
        ),
        _ => "detail unavailable for unknown/non-exhaustive audit event variant".to_owned(),
    }
}

fn format_equals_rollup(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "-".to_owned();
    }
    counts
        .iter()
        .map(|(label, count)| format!("{label}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_list_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    values.join(",")
}

fn format_optional_summary_group_label(group_value: &Option<String>) -> &str {
    group_value.as_deref().unwrap_or("(none)")
}

fn non_discovery_event_kind_counts(counts: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    counts
        .iter()
        .filter(|(label, _)| label.as_str() != "ToolSearchEvaluated")
        .map(|(label, count)| (label.clone(), *count))
        .collect()
}

fn non_discovery_triage_counts(counts: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    counts
        .iter()
        .filter(|(label, _)| {
            !matches!(
                label.as_str(),
                "tool_search_trust_conflict" | "tool_search_trust_empty"
            )
        })
        .map(|(label, count)| (label.clone(), *count))
        .collect()
}

fn correlated_attention_hint(
    additional_events: usize,
    non_discovery_event_kind_counts: &BTreeMap<String, usize>,
    non_discovery_triage_counts: &BTreeMap<String, usize>,
) -> Option<String> {
    if !non_discovery_triage_counts.is_empty() {
        return Some(format!(
            "adjacent_triage={}",
            format_top_rollup(non_discovery_triage_counts, 2)
        ));
    }

    if !non_discovery_event_kind_counts.is_empty() {
        return Some(format!(
            "adjacent_event_kinds={}",
            format_top_rollup(non_discovery_event_kind_counts, 2)
        ));
    }

    if additional_events > 0 {
        return Some(format!(
            "broader_window_additional_events={additional_events}"
        ));
    }

    None
}

fn correlated_remediation_hint(
    additional_events: usize,
    non_discovery_event_kind_counts: &BTreeMap<String, usize>,
    non_discovery_triage_counts: &BTreeMap<String, usize>,
) -> Option<String> {
    if let Some(label) = top_rollup_label(non_discovery_triage_counts) {
        return triage_label_remediation_hint(label).map(str::to_owned);
    }

    if let Some(label) = top_rollup_label(non_discovery_event_kind_counts) {
        return event_kind_remediation_hint(label).map(str::to_owned);
    }

    if additional_events > 0 {
        return Some(
            "inspect the widened audit summary before retrying discovery to identify adjacent workload drift"
                .to_owned(),
        );
    }

    None
}

fn triage_label_remediation_hint(label: &str) -> Option<&'static str> {
    match label {
        "authorization_denied" => Some(
            "grant the required capability or retry with a token scoped for the requested operation",
        ),
        "provider_failover" => Some(
            "inspect provider health, fallback routing, and model compatibility before retrying",
        ),
        "security_scan_blocked" => {
            Some("remediate or suppress the blocking findings before retrying plugin bootstrap")
        }
        "plugin_trust_blocked" => Some(
            "review plugin provenance and bootstrap policy before enabling auto-apply for the blocked plugins",
        ),
        "tool_search_trust_conflict" => {
            Some("align query trust prefixes with structured trust_tiers before retrying discovery")
        }
        "tool_search_trust_empty" => Some(
            "broaden the requested trust scope or review lower-trust candidates before retrying discovery",
        ),
        _ => None,
    }
}

fn event_kind_remediation_hint(label: &str) -> Option<&'static str> {
    match label {
        "AuthorizationDenied" => triage_label_remediation_hint("authorization_denied"),
        "ProviderFailover" => triage_label_remediation_hint("provider_failover"),
        "SecurityScanEvaluated" => triage_label_remediation_hint("security_scan_blocked"),
        "PluginTrustEvaluated" => triage_label_remediation_hint("plugin_trust_blocked"),
        _ => None,
    }
}

fn top_rollup_label(counts: &BTreeMap<String, usize>) -> Option<&str> {
    counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
        .map(|(label, _)| label.as_str())
}

fn format_top_rollup(counts: &BTreeMap<String, usize>, limit: usize) -> String {
    if counts.is_empty() || limit == 0 {
        return "-".to_owned();
    }

    let mut entries = counts
        .iter()
        .map(|(label, count)| (label.as_str(), *count))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    entries
        .into_iter()
        .take(limit)
        .map(|(label, count)| format!("{label}={count}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::fs::OpenOptions;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::kernel::{
        AuditEvent, AuditEventKind, Capability, CapabilityToken, ExecutionPlane, PlaneTier,
    };
    use crate::mvp::test_support::ScopedEnv;

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{counter}", std::process::id()))
    }

    fn write_audit_config_with_mode(
        root: &Path,
        journal_path: &Path,
        mode: crate::mvp::config::AuditMode,
    ) -> PathBuf {
        fs::create_dir_all(root).expect("create config root");
        let config_path = root.join("loongclaw.toml");
        let mut config = crate::mvp::config::LoongClawConfig::default();
        config.audit.mode = mode;
        config.audit.path = journal_path.display().to_string();
        crate::mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
            .expect("write audit config");
        config_path
    }

    fn write_audit_config(root: &Path, journal_path: &Path) -> PathBuf {
        write_audit_config_with_mode(root, journal_path, crate::mvp::config::AuditMode::Fanout)
    }

    fn sample_audit_event(
        event_id: &str,
        timestamp_epoch_s: u64,
        agent_id: Option<&str>,
        kind: AuditEventKind,
    ) -> AuditEvent {
        AuditEvent {
            event_id: event_id.to_owned(),
            timestamp_epoch_s,
            agent_id: agent_id.map(str::to_owned),
            kind,
        }
    }

    fn write_journal(path: &Path, events: &[AuditEvent]) {
        let parent = path.parent().expect("journal path should have parent");
        fs::create_dir_all(parent).expect("create journal parent");
        let encoded = events
            .iter()
            .map(|event| serde_json::to_string(event).expect("serialize audit event"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{encoded}\n")).expect("write audit journal");
    }

    #[test]
    fn audit_recent_execution_keeps_last_events_in_order() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_001,
                    Some("agent-a"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-1".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_002,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-2".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_003,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Tool,
                        tier: PlaneTier::Core,
                        primary_adapter: "mvp-tools".to_owned(),
                        delegated_core_adapter: None,
                        operation: "tool.call".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 2,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent");

        assert_eq!(execution.journal_path, journal_path.display().to_string());
        match execution.result {
            AuditCommandResult::Recent { limit, events } => {
                assert_eq!(limit, 2);
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_json_includes_loaded_events_and_journal_path() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-json");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[sample_audit_event(
                "evt-json",
                1_700_010_010,
                Some("agent-json"),
                AuditEventKind::TokenRevoked {
                    token_id: "token-json".to_owned(),
                },
            )],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: true,
            command: AuditCommands::Recent {
                limit: 5,
                since_epoch_s: Some(1_700_010_000),
                until_epoch_s: Some(1_700_010_050),
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-json".to_owned()),
                token_id: Some("token-json".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent");
        let payload = audit_cli_json(&execution);

        assert_eq!(payload["journal_path"], journal_path.display().to_string());
        assert_eq!(payload["limit"], 5);
        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_000_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_050_u64);
        assert_eq!(payload["event_id_filter"], "evt-json");
        assert_eq!(payload["token_id_filter"], "token-json");
        assert_eq!(payload["loaded_events"], 1);
        assert_eq!(payload["events"][0]["event_id"], "evt-json");
    }

    #[test]
    fn audit_recent_waits_for_existing_audit_journal_lock_before_reading() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-lock");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[sample_audit_event(
                "evt-lock",
                1_700_010_020,
                Some("agent-lock"),
                AuditEventKind::TokenRevoked {
                    token_id: "token-lock".to_owned(),
                },
            )],
        );

        let external_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&journal_path)
            .expect("open external audit journal handle");
        external_lock
            .lock()
            .expect("hold external audit journal lock");

        let (tx, rx) = mpsc::channel();
        let config = config_path.display().to_string();
        let handle = thread::spawn(move || {
            let result = execute_audit_command(AuditCommandOptions {
                config: Some(config),
                json: false,
                command: AuditCommands::Recent {
                    limit: 10,
                    since_epoch_s: None,
                    until_epoch_s: None,
                    pack_id: None,
                    agent_id: None,
                    event_id: None,
                    token_id: None,
                    kind: None,
                    triage_label: None,
                    query_contains: None,
                    trust_tier: None,
                },
            });
            tx.send(result).expect("send audit recent result");
        });

        match rx.recv_timeout(Duration::from_millis(100)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(result) => {
                panic!("audit recent should block on external journal lock, got {result:?}")
            }
            Err(error) => panic!("audit recent channel closed unexpectedly: {error:?}"),
        }

        external_lock
            .unlock()
            .expect("release external audit journal lock");
        let execution = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("audit recent should finish after lock release")
            .expect("audit recent should succeed after lock release");
        handle.join().expect("join audit recent reader");

        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, "evt-lock");
            }
            other => panic!("unexpected audit command result after lock release: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_rejects_zero_limit() {
        let mut env = ScopedEnv::new();
        env.set("HOME", unique_temp_dir("loongclaw-audit-cli-missing-home"));

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Recent {
                limit: 0,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("zero recent limit should fail");

        assert!(error.contains("audit recent limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_recent_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loongclaw-audit-cli-large-limit-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Recent {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("excessive recent limit should fail");

        assert!(error.contains("audit recent limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_recent_text_renders_tool_search_trust_conflict_details() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_000),
            until_epoch_s_filter: Some(1_700_010_060),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-tool-search".to_owned()),
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 5,
                events: vec![sample_audit_event(
                    "evt-tool-search",
                    1_700_010_021,
                    Some("agent-search"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                )],
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit recent");

        assert!(rendered.contains("since_epoch_s=1700010000"));
        assert!(rendered.contains("until_epoch_s=1700010060"));
        assert!(rendered.contains(
            "pack_id=- agent_id=- event_id=evt-tool-search token_id=- kind=- triage_label=-"
        ));
        assert!(rendered.contains("query_contains=trust:official"));
        assert!(rendered.contains("trust_tier=official"));
        assert!(rendered.contains("kind=ToolSearchEvaluated"));
        assert!(rendered.contains("query=\"trust:official search\""));
        assert!(rendered.contains("returned=0"));
        assert!(rendered.contains("trust_scope=-"));
        assert!(rendered.contains("conflicting_requested_tiers=true"));
        assert!(rendered.contains("filtered_out_candidates=2"));
        assert!(rendered.contains("filtered_out_tier_counts=official=1,verified-community=1"));
    }

    #[test]
    fn audit_recent_filters_by_kind_and_uses_filtered_window_limit() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-kind-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_030,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_031,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_032,
                    Some("agent-c"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        official_plugins: 0,
                        verified_community_plugins: 0,
                        unverified_plugins: 1,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 1,
                        blocked_auto_apply_plugins: 1,
                        review_required_plugin_ids: vec!["stdio-review".to_owned()],
                        review_required_bridges: vec!["process_stdio".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_033,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 1,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: Some("ToolSearchEvaluated".to_owned()),
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute filtered audit recent");

        assert_eq!(
            execution.kind_filter.as_deref(),
            Some("ToolSearchEvaluated")
        );
        assert_eq!(execution.triage_label_filter, None);
        match execution.result {
            AuditCommandResult::Recent { limit, events } => {
                assert_eq!(limit, 1);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_id, "evt-4");
                assert!(matches!(
                    events[0].kind,
                    AuditEventKind::ToolSearchEvaluated { .. }
                ));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_query_contains_and_trust_tier() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-tool-search-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_034,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_035,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_036,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_037,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: Some("ToolSearchEvaluated".to_owned()),
                triage_label: None,
                query_contains: Some("trust:official".to_owned()),
                trust_tier: Some("official".to_owned()),
            },
        })
        .expect("execute audit recent with tool search filters");

        assert_eq!(
            execution.query_contains_filter.as_deref(),
            Some("trust:official")
        );
        assert_eq!(execution.trust_tier_filter.as_deref(), Some("official"));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_time_window_inclusively() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-time-window");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_030,
                    Some("agent-a"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-1".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_031,
                    Some("agent-b"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-2".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_032,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-3".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_033,
                    Some("agent-d"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-4".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: Some(1_700_010_031),
                until_epoch_s: Some(1_700_010_032),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with time window");

        assert_eq!(execution.since_epoch_s_filter, Some(1_700_010_031));
        assert_eq!(execution.until_epoch_s_filter, Some(1_700_010_032));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_pack_id_and_agent_id() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-pack-agent-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_035,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-1".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_035,
                            expires_at_epoch_s: 1_700_010_135,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_036,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "ops-pack".to_owned(),
                        token_id: "token-2".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_037,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-3".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_038,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 1,
                        trust_filter_applied: false,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 0,
                        filtered_out_tier_counts: BTreeMap::new(),
                        top_provider_ids: vec!["official-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: Some("sales-intel".to_owned()),
                agent_id: Some("agent-a".to_owned()),
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with pack and agent filters");

        assert_eq!(execution.pack_id_filter.as_deref(), Some("sales-intel"));
        assert_eq!(execution.agent_id_filter.as_deref(), Some("agent-a"));
        match execution.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_recent_filters_by_event_id_and_token_id() {
        let root = unique_temp_dir("loongclaw-audit-cli-recent-event-token-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_039,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_039,
                            expires_at_epoch_s: 1_700_010_139,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_040,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_041,
                    Some("agent-c"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_042,
                    Some("agent-d"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-other".to_owned(),
                    },
                ),
            ],
        );

        let by_event = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-2".to_owned()),
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with event id filter");

        let by_token = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with token id filter");

        let combined = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-3".to_owned()),
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with event and token filters");

        let mismatch = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: Some("evt-4".to_owned()),
                token_id: Some("token-shared".to_owned()),
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect("execute audit recent with mismatched event and token filters");

        assert_eq!(by_event.event_id_filter.as_deref(), Some("evt-2"));
        assert_eq!(by_event.token_id_filter, None);
        match by_event.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-2"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert_eq!(by_token.event_id_filter, None);
        assert_eq!(by_token.token_id_filter.as_deref(), Some("token-shared"));
        match by_token.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-2", "evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert_eq!(combined.event_id_filter.as_deref(), Some("evt-3"));
        assert_eq!(combined.token_id_filter.as_deref(), Some("token-shared"));
        match combined.result {
            AuditCommandResult::Recent { events, .. } => {
                let ids = events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-3"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        match mismatch.result {
            AuditCommandResult::Recent { events, .. } => {
                assert!(events.is_empty());
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_filters_token_events_and_summarizes_lifecycle() {
        let root = unique_temp_dir("loongclaw-audit-cli-token-trail");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_060,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: BTreeSet::from([
                                Capability::InvokeTool,
                                Capability::NetworkEgress,
                            ]),
                            issued_at_epoch_s: 1_700_010_060,
                            expires_at_epoch_s: 1_700_010_160,
                            generation: 3,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_061,
                    Some("agent-deny-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_062,
                    Some("agent-deny-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_063,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_064,
                    Some("agent-other"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-other".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::TokenTrail {
                token_id: "token-shared".to_owned(),
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
            },
        })
        .expect("execute audit token trail");

        assert_eq!(execution.token_id_filter.as_deref(), Some("token-shared"));
        match execution.result {
            AuditCommandResult::TokenTrail {
                limit,
                token_id,
                loaded_events,
                total_matching_events,
                truncated_matching_events,
                event_kind_counts,
                issued_event_id,
                issued_timestamp_epoch_s,
                issued_pack_id,
                issued_agent_id,
                issued_generation,
                issued_expires_at_epoch_s,
                issued_capability_count,
                issued_capabilities,
                authorization_denied_count,
                authorization_denied_reason_counts,
                last_denied_event_id,
                last_denied_timestamp_epoch_s,
                last_denied_pack_id,
                last_denied_agent_id,
                last_denied_reason,
                revoked_event_id,
                revoked_timestamp_epoch_s,
                revoked_agent_id,
                timeline,
                ..
            } => {
                assert_eq!(limit, 10);
                assert_eq!(token_id, "token-shared");
                assert_eq!(loaded_events, 4);
                assert_eq!(total_matching_events, 4);
                assert_eq!(truncated_matching_events, 0);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 2_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                        ("TokenRevoked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(issued_event_id.as_deref(), Some("evt-1"));
                assert_eq!(issued_timestamp_epoch_s, Some(1_700_010_060));
                assert_eq!(issued_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(issued_agent_id.as_deref(), Some("agent-issue"));
                assert_eq!(issued_generation, Some(3));
                assert_eq!(issued_expires_at_epoch_s, Some(1_700_010_160));
                assert_eq!(issued_capability_count, Some(2));
                assert_eq!(
                    issued_capabilities,
                    vec!["invoke_tool".to_owned(), "network_egress".to_owned()]
                );
                assert_eq!(authorization_denied_count, 2);
                assert_eq!(
                    authorization_denied_reason_counts,
                    BTreeMap::from([
                        ("missing capability".to_owned(), 1_usize),
                        ("network egress denied".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_denied_event_id.as_deref(), Some("evt-3"));
                assert_eq!(last_denied_timestamp_epoch_s, Some(1_700_010_062));
                assert_eq!(last_denied_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(last_denied_agent_id.as_deref(), Some("agent-deny-b"));
                assert_eq!(last_denied_reason.as_deref(), Some("network egress denied"));
                assert_eq!(revoked_event_id.as_deref(), Some("evt-4"));
                assert_eq!(revoked_timestamp_epoch_s, Some(1_700_010_063));
                assert_eq!(revoked_agent_id.as_deref(), Some("agent-revoke"));
                let ids = timeline
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-1", "evt-2", "evt-3", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_reports_truncated_matching_events() {
        let root = unique_temp_dir("loongclaw-audit-cli-token-trail-truncated");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_070,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-shared".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_070,
                            expires_at_epoch_s: 1_700_010_170,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_071,
                    Some("agent-deny-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_072,
                    Some("agent-deny-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-shared".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_073,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-shared".to_owned(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::TokenTrail {
                token_id: "token-shared".to_owned(),
                limit: 2,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
            },
        })
        .expect("execute truncated audit token trail");

        match execution.result {
            AuditCommandResult::TokenTrail {
                loaded_events,
                total_matching_events,
                truncated_matching_events,
                issued_event_id,
                revoked_event_id,
                timeline,
                ..
            } => {
                assert_eq!(loaded_events, 2);
                assert_eq!(total_matching_events, 4);
                assert_eq!(truncated_matching_events, 2);
                assert_eq!(issued_event_id, None);
                assert_eq!(revoked_event_id.as_deref(), Some("evt-4"));
                let ids = timeline
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>();
                assert_eq!(ids, vec!["evt-3", "evt-4"]);
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_token_trail_text_and_json_render_lifecycle() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_500),
            until_epoch_s_filter: Some(1_700_010_599),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: Some("agent-issue".to_owned()),
            event_id_filter: None,
            token_id_filter: Some("token-shared".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::TokenTrail {
                limit: 25,
                token_id: "token-shared".to_owned(),
                loaded_events: 3,
                total_matching_events: 4,
                truncated_matching_events: 1,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 1_usize),
                    ("TokenIssued".to_owned(), 1_usize),
                    ("TokenRevoked".to_owned(), 1_usize),
                ]),
                first_timestamp_epoch_s: Some(1_700_010_500),
                last_event_id: Some("evt-3".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_502),
                last_agent_id: Some("agent-revoke".to_owned()),
                issued_event_id: Some("evt-1".to_owned()),
                issued_timestamp_epoch_s: Some(1_700_010_500),
                issued_pack_id: Some("sales-intel".to_owned()),
                issued_agent_id: Some("agent-issue".to_owned()),
                issued_generation: Some(2),
                issued_expires_at_epoch_s: Some(1_700_010_800),
                issued_capability_count: Some(2),
                issued_capabilities: vec!["invoke_tool".to_owned(), "network_egress".to_owned()],
                authorization_denied_count: 1,
                authorization_denied_reason_counts: BTreeMap::from([(
                    "missing capability".to_owned(),
                    1_usize,
                )]),
                last_denied_event_id: Some("evt-2".to_owned()),
                last_denied_timestamp_epoch_s: Some(1_700_010_501),
                last_denied_pack_id: Some("sales-intel".to_owned()),
                last_denied_agent_id: Some("agent-deny".to_owned()),
                last_denied_reason: Some("missing capability".to_owned()),
                revoked_event_id: Some("evt-3".to_owned()),
                revoked_timestamp_epoch_s: Some(1_700_010_502),
                revoked_agent_id: Some("agent-revoke".to_owned()),
                timeline: vec![
                    sample_audit_event(
                        "evt-1",
                        1_700_010_500,
                        Some("agent-issue"),
                        AuditEventKind::TokenIssued {
                            token: CapabilityToken {
                                token_id: "token-shared".to_owned(),
                                pack_id: "sales-intel".to_owned(),
                                agent_id: "agent-issue".to_owned(),
                                allowed_capabilities: BTreeSet::from([
                                    Capability::InvokeTool,
                                    Capability::NetworkEgress,
                                ]),
                                issued_at_epoch_s: 1_700_010_500,
                                expires_at_epoch_s: 1_700_010_800,
                                generation: 2,
                            },
                        },
                    ),
                    sample_audit_event(
                        "evt-2",
                        1_700_010_501,
                        Some("agent-deny"),
                        AuditEventKind::AuthorizationDenied {
                            pack_id: "sales-intel".to_owned(),
                            token_id: "token-shared".to_owned(),
                            reason: "missing capability".to_owned(),
                        },
                    ),
                    sample_audit_event(
                        "evt-3",
                        1_700_010_502,
                        Some("agent-revoke"),
                        AuditEventKind::TokenRevoked {
                            token_id: "token-shared".to_owned(),
                        },
                    ),
                ],
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit token trail");
        let payload = audit_cli_json(&execution);

        assert!(rendered.contains("audit token-trail"));
        assert!(rendered.contains("token_id=token-shared"));
        assert!(
            rendered
                .contains("loaded_events=3 total_matching_events=4 truncated_matching_events=1")
        );
        assert!(rendered.contains("since_epoch_s=1700010500"));
        assert!(rendered.contains("until_epoch_s=1700010599"));
        assert!(rendered.contains(
            "pack_id=sales-intel agent_id=agent-issue event_id=- token_id=token-shared kind=- triage_label=-"
        ));
        assert!(
            rendered
                .contains("event_kind_counts=AuthorizationDenied=1,TokenIssued=1,TokenRevoked=1")
        );
        assert!(rendered.contains("issued_event_id=evt-1"));
        assert!(
            rendered.contains(
                "issued_capability_count=2 issued_capabilities=invoke_tool,network_egress"
            )
        );
        assert!(rendered.contains(
            "authorization_denied_count=1 authorization_denied_reason_counts=missing capability=1"
        ));
        assert!(rendered.contains("revoked_event_id=evt-3 revoked_timestamp_epoch_s=1700010502 revoked_agent_id=agent-revoke"));
        assert!(rendered.contains("timeline:"));
        assert!(rendered.contains("- ts=1700010502 event_id=evt-3 agent_id=agent-revoke kind=TokenRevoked token_id=token-shared"));

        assert_eq!(payload["command"], "token-trail");
        assert_eq!(payload["token_id"], "token-shared");
        assert_eq!(payload["token_id_filter"], "token-shared");
        assert_eq!(payload["pack_id_filter"], "sales-intel");
        assert_eq!(payload["agent_id_filter"], "agent-issue");
        assert_eq!(payload["loaded_events"], 3);
        assert_eq!(payload["total_matching_events"], 4);
        assert_eq!(payload["truncated_matching_events"], 1);
        assert_eq!(payload["event_kind_counts"]["TokenIssued"], 1);
        assert_eq!(payload["issued_generation"], 2);
        assert_eq!(payload["issued_capability_count"], 2);
        assert_eq!(payload["authorization_denied_count"], 1);
        assert_eq!(
            payload["authorization_denied_reason_counts"]["missing capability"],
            1
        );
        assert_eq!(payload["last_denied_reason"], "missing capability");
        assert_eq!(payload["revoked_event_id"], "evt-3");
        assert_eq!(payload["timeline"][0]["event_id"], "evt-1");
        assert_eq!(payload["timeline"][2]["event_id"], "evt-3");
    }

    #[test]
    fn audit_summary_filters_by_triage_label() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-triage-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_040,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_041,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_042,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: Some("tool_search_trust_conflict".to_owned()),
                group_by: None,
            },
        })
        .expect("execute filtered audit summary");

        assert_eq!(execution.kind_filter, None);
        assert_eq!(
            execution.triage_label_filter.as_deref(),
            Some("tool_search_trust_conflict")
        );
        match execution.result {
            AuditCommandResult::Summary {
                loaded_events,
                event_kind_counts,
                triage_counts,
                last_triage_event_id,
                last_triage_label,
                ..
            } => {
                assert_eq!(loaded_events, 1);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([("ToolSearchEvaluated".to_owned(), 1_usize)])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("tool_search_trust_conflict".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-3"));
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_filters_by_agent_id() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-agent-filter");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_045,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_046,
                    Some("agent-b"),
                    AuditEventKind::ProviderFailover {
                        pack_id: "sales-intel".to_owned(),
                        provider_id: "openai".to_owned(),
                        reason: "rate_limited".to_owned(),
                        stage: "response".to_owned(),
                        model: "gpt-5.1".to_owned(),
                        attempt: 1,
                        max_attempts: 3,
                        status_code: Some(429),
                        try_next_model: true,
                        auto_model_mode: true,
                        candidate_index: 0,
                        candidate_count: 2,
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_047,
                    Some("agent-b"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 2,
                        high_findings: 1,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: true,
                        block_reason: Some("unsigned plugin".to_owned()),
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: Some("agent-b".to_owned()),
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary with agent filter");

        assert_eq!(execution.agent_id_filter.as_deref(), Some("agent-b"));
        match execution.result {
            AuditCommandResult::Summary {
                loaded_events,
                event_kind_counts,
                triage_counts,
                last_event_id,
                ..
            } => {
                assert_eq!(loaded_events, 2);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("ProviderFailover".to_owned(), 1_usize),
                        ("SecurityScanEvaluated".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("provider_failover".to_owned(), 1_usize),
                        ("security_scan_blocked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_event_id.as_deref(), Some("evt-3"));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_discovery_filters_tool_search_events_and_rolls_up_trust_context() {
        let root = unique_temp_dir("loongclaw-audit-cli-discovery");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_050,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_051,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("unverified".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_052,
                    Some("agent-c"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official catalog search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_053,
                    Some("agent-d"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:verified-community catalog search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "unverified".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: Some(1_700_010_051),
                until_epoch_s: Some(1_700_010_052),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: Some("catalog".to_owned()),
                trust_tier: Some("official".to_owned()),
                group_by: None,
            },
        })
        .expect("execute audit discovery");

        assert_eq!(execution.since_epoch_s_filter, Some(1_700_010_051));
        assert_eq!(execution.until_epoch_s_filter, Some(1_700_010_052));
        assert_eq!(
            execution.kind_filter.as_deref(),
            Some("ToolSearchEvaluated")
        );
        assert_eq!(execution.triage_label_filter, None);
        assert_eq!(execution.query_contains_filter.as_deref(), Some("catalog"));
        assert_eq!(execution.trust_tier_filter.as_deref(), Some("official"));
        match execution.result {
            AuditCommandResult::Discovery {
                limit,
                loaded_events,
                triage_counts,
                query_requested_tier_counts,
                structured_requested_tier_counts,
                effective_tier_counts,
                filtered_out_tier_counts,
                trust_filter_applied_events,
                conflicting_requested_tier_events,
                trust_filtered_empty_events,
                last_event_id,
                last_pack_id,
                last_query,
                last_returned,
                last_trust_filter_applied,
                last_conflicting_requested_tiers,
                last_query_requested_tiers,
                last_structured_requested_tiers,
                last_effective_tiers,
                last_filtered_out_candidates,
                last_filtered_out_tier_counts,
                last_top_provider_ids,
                last_triage_event_id,
                last_triage_label,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(limit, 10);
                assert_eq!(loaded_events, 2);
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    query_requested_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    structured_requested_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    effective_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    filtered_out_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("unverified".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 2_usize),
                    ])
                );
                assert_eq!(trust_filter_applied_events, 2);
                assert_eq!(conflicting_requested_tier_events, 1);
                assert_eq!(trust_filtered_empty_events, 1);
                assert_eq!(last_event_id.as_deref(), Some("evt-3"));
                assert_eq!(last_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(last_query.as_deref(), Some("trust:official catalog search"));
                assert_eq!(last_returned, Some(0));
                assert_eq!(last_trust_filter_applied, Some(true));
                assert_eq!(last_conflicting_requested_tiers, Some(true));
                assert_eq!(last_query_requested_tiers, vec!["official".to_owned()]);
                assert_eq!(
                    last_structured_requested_tiers,
                    vec!["verified-community".to_owned()]
                );
                assert_eq!(last_effective_tiers, Vec::<String>::new());
                assert_eq!(last_filtered_out_candidates, Some(2));
                assert_eq!(
                    last_filtered_out_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(last_top_provider_ids, Vec::<String>::new());
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-3"));
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "query=\"trust:official catalog search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "align query trust prefixes with structured trust_tiers before retrying discovery"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_discovery_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loongclaw-audit-cli-large-discovery-limit-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Discovery {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: None,
            },
        })
        .expect_err("excessive discovery limit should fail");

        assert!(error.contains("audit discovery limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_discovery_rejects_until_before_since() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loongclaw-audit-cli-invalid-time-range-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: Some(1_700_010_100),
                until_epoch_s: Some(1_700_010_099),
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: None,
            },
        })
        .expect_err("invalid discovery time range should fail");

        assert!(error.contains(
            "audit discovery until_epoch_s must be greater than or equal to since_epoch_s"
        ));
    }

    #[test]
    fn audit_discovery_groups_by_agent() {
        let root = unique_temp_dir("loongclaw-audit-cli-discovery-group-by-agent");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-0",
                    1_700_010_299,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-a".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-1",
                    1_700_010_300,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "catalog".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_301,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official catalog".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_302,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "ops-pack".to_owned(),
                        query: "trust:verified-community search".to_owned(),
                        returned: 1,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["verified-community".to_owned()],
                        structured_requested_tiers: Vec::new(),
                        effective_tiers: vec!["verified-community".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "unverified".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: vec!["community-search".to_owned()],
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Discovery {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
                group_by: Some("agent".to_owned()),
            },
        })
        .expect("execute grouped audit discovery");
        let rendered = render_audit_cli_text(&execution).expect("render grouped audit discovery");
        let payload = audit_cli_json(&execution);

        match execution.result {
            AuditCommandResult::Discovery {
                group_by, groups, ..
            } => {
                assert_eq!(group_by.as_deref(), Some("agent"));
                assert_eq!(groups.len(), 2);

                assert_eq!(groups[0].group_value.as_deref(), Some("agent-a"));
                assert_eq!(groups[0].loaded_events, 2);
                assert_eq!(
                    groups[0].triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    groups[0].query_requested_tier_counts,
                    BTreeMap::from([("official".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].structured_requested_tier_counts,
                    BTreeMap::from([
                        ("official".to_owned(), 1_usize),
                        ("verified-community".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(groups[0].trust_filter_applied_events, 2);
                assert_eq!(groups[0].conflicting_requested_tier_events, 1);
                assert_eq!(groups[0].trust_filtered_empty_events, 1);
                assert_eq!(groups[0].last_pack_id.as_deref(), Some("sales-intel"));
                assert_eq!(
                    groups[0].last_query.as_deref(),
                    Some("trust:official catalog")
                );
                assert_eq!(groups[0].last_returned, Some(0));
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .loaded_events,
                    3
                );
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("ToolSearchEvaluated".to_owned(), 2_usize),
                    ])
                );
                assert_eq!(
                    groups[0]
                        .correlated_summary
                        .as_ref()
                        .expect("agent-a correlated summary should exist")
                        .triage_counts,
                    BTreeMap::from([
                        ("authorization_denied".to_owned(), 1_usize),
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(groups[0].correlated_additional_events, 1);
                assert_eq!(
                    groups[0].correlated_non_discovery_event_kind_counts,
                    BTreeMap::from([("AuthorizationDenied".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].correlated_non_discovery_triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(
                    groups[0].correlated_attention_hint.as_deref(),
                    Some("adjacent_triage=authorization_denied=1")
                );
                assert_eq!(
                    groups[0].correlated_remediation_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );

                assert_eq!(groups[1].group_value.as_deref(), Some("agent-b"));
                assert_eq!(groups[1].loaded_events, 1);
                assert_eq!(
                    groups[1].effective_tier_counts,
                    BTreeMap::from([("verified-community".to_owned(), 1_usize)])
                );
                assert_eq!(groups[1].last_pack_id.as_deref(), Some("ops-pack"));
                assert_eq!(groups[1].last_returned, Some(1));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert!(rendered.contains("group_by=agent group_count=2"));
        assert!(rendered.contains("group[agent]=agent-a loaded_events=2"));
        assert!(rendered.contains(
            "group[agent]=agent-b loaded_events=1 triage_counts=- query_requested_tier_counts=verified-community=1"
        ));
        assert!(
            rendered
                .contains("group_drill_down[agent]=agent-a command=loong audit recent --config")
        );
        assert!(rendered.contains(
            "group_correlated_preview[agent]=agent-a loaded_events=3 event_kind_counts=AuthorizationDenied=1,ToolSearchEvaluated=2 triage_counts=authorization_denied=1,tool_search_trust_conflict=1,tool_search_trust_empty=1"
        ));
        assert!(rendered.contains(
            "group_correlated_focus[agent]=agent-a additional_events=1 non_discovery_event_kind_counts=AuthorizationDenied=1 non_discovery_triage_counts=authorization_denied=1 attention_hint=adjacent_triage=authorization_denied=1 remediation_hint=grant the required capability or retry with a token scoped for the requested operation"
        ));
        assert!(rendered.contains(
            "group_correlated_summary[agent]=agent-a command=loong audit summary --config"
        ));
        assert!(rendered.contains(
            "group_correlated_remediation[agent]=agent-a command=loong audit summary --config"
        ));
        assert!(rendered.contains("--agent-id 'agent-a'"));
        assert!(rendered.contains("--kind 'ToolSearchEvaluated'"));

        assert_eq!(payload["group_by"], "agent");
        assert_eq!(payload["groups"][0]["group_value"], "agent-a");
        assert_eq!(payload["groups"][0]["loaded_events"], 2);
        assert_eq!(payload["groups"][0]["trust_filter_applied_events"], 2);
        assert_eq!(
            payload["groups"][0]["drill_down_command"],
            json!(format!(
                "loong audit recent --config '{}' --limit 10 --agent-id 'agent-a' --kind 'ToolSearchEvaluated'",
                config_path.display()
            ))
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary_command"],
            json!(format!(
                "loong audit summary --config '{}' --limit 10 --agent-id 'agent-a'",
                config_path.display()
            ))
        );
        assert_eq!(payload["groups"][0]["correlated_additional_events"], 1);
        assert_eq!(
            payload["groups"][0]["correlated_non_discovery_event_kind_counts"]["AuthorizationDenied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_non_discovery_triage_counts"]["authorization_denied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_attention_hint"],
            "adjacent_triage=authorization_denied=1"
        );
        assert_eq!(
            payload["groups"][0]["correlated_remediation_hint"],
            "grant the required capability or retry with a token scoped for the requested operation"
        );
        assert_eq!(
            payload["groups"][0]["correlated_remediation_command"],
            json!(format!(
                "loong audit summary --config '{}' --limit 10 --agent-id 'agent-a' --triage-label 'authorization_denied' --group-by 'token'",
                config_path.display()
            ))
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["loaded_events"],
            3
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["event_kind_counts"]["AuthorizationDenied"],
            1
        );
        assert_eq!(
            payload["groups"][0]["correlated_summary"]["triage_counts"]["authorization_denied"],
            1
        );
        assert_eq!(
            payload["groups"][1]["effective_tier_counts"]["verified-community"],
            1
        );
    }

    #[test]
    fn audit_discovery_group_drill_down_command_preserves_filters() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: None,
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 0,
            correlated_non_discovery_event_kind_counts: BTreeMap::new(),
            correlated_non_discovery_triage_counts: BTreeMap::new(),
            correlated_attention_hint: None,
            correlated_remediation_hint: None,
        };

        let command = discovery_group_drill_down_command(&execution, 25, Some("agent"), &group)
            .expect("group drill-down command should render");

        assert_eq!(
            command,
            "loong audit recent --config '/tmp/loongclaw.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b' --event-id 'evt-2' --kind 'ToolSearchEvaluated' --triage-label 'tool_search_trust_conflict' --query-contains 'trust:official' --trust-tier 'official'"
        );
    }

    #[test]
    fn audit_discovery_group_correlated_summary_command_broadens_to_workload_window() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 0,
            correlated_non_discovery_event_kind_counts: BTreeMap::new(),
            correlated_non_discovery_triage_counts: BTreeMap::new(),
            correlated_attention_hint: None,
            correlated_remediation_hint: None,
        };

        let command =
            discovery_group_correlated_summary_command(&execution, 25, Some("agent"), &group)
                .expect("group correlated summary command should render");

        assert_eq!(
            command,
            "loong audit summary --config '/tmp/loongclaw.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b'"
        );
    }

    #[test]
    fn audit_discovery_group_correlated_remediation_command_targets_token_summary() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Recent {
                limit: 1,
                events: Vec::new(),
            },
        };
        let group = AuditDiscoveryGroup {
            group_value: Some("agent-b".to_owned()),
            loaded_events: 2,
            triage_counts: BTreeMap::new(),
            query_requested_tier_counts: BTreeMap::new(),
            structured_requested_tier_counts: BTreeMap::new(),
            effective_tier_counts: BTreeMap::new(),
            filtered_out_tier_counts: BTreeMap::new(),
            trust_filter_applied_events: 0,
            conflicting_requested_tier_events: 0,
            trust_filtered_empty_events: 0,
            first_timestamp_epoch_s: Some(1_700_010_400),
            last_event_id: Some("evt-2".to_owned()),
            last_timestamp_epoch_s: Some(1_700_010_401),
            last_agent_id: Some("agent-b".to_owned()),
            last_pack_id: Some("sales-intel".to_owned()),
            last_query: Some("trust:official search".to_owned()),
            last_returned: Some(0),
            correlated_summary: None,
            correlated_additional_events: 1,
            correlated_non_discovery_event_kind_counts: BTreeMap::from([(
                "AuthorizationDenied".to_owned(),
                1_usize,
            )]),
            correlated_non_discovery_triage_counts: BTreeMap::from([(
                "authorization_denied".to_owned(),
                1_usize,
            )]),
            correlated_attention_hint: Some("adjacent_triage=authorization_denied=1".to_owned()),
            correlated_remediation_hint: Some(
                "grant the required capability or retry with a token scoped for the requested operation"
                    .to_owned(),
            ),
        };

        let command =
            discovery_group_correlated_remediation_command(&execution, 25, Some("agent"), &group)
                .expect("group correlated remediation command should render");

        assert_eq!(
            command,
            "loong audit summary --config '/tmp/loongclaw.toml' --limit 25 --since-epoch-s 1700010400 --until-epoch-s 1700010499 --pack-id 'sales-intel' --agent-id 'agent-b' --triage-label 'authorization_denied' --group-by 'token'"
        );
    }

    #[test]
    fn audit_discovery_text_and_json_render_trust_rollups() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_400),
            until_epoch_s_filter: Some(1_700_010_499),
            pack_id_filter: Some("sales-intel".to_owned()),
            agent_id_filter: Some("agent-b".to_owned()),
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: None,
            kind_filter: Some("ToolSearchEvaluated".to_owned()),
            triage_label_filter: Some("tool_search_trust_conflict".to_owned()),
            query_contains_filter: Some("trust:official".to_owned()),
            trust_tier_filter: Some("official".to_owned()),
            result: AuditCommandResult::Discovery {
                limit: 25,
                loaded_events: 2,
                triage_counts: BTreeMap::from([(
                    "tool_search_trust_conflict".to_owned(),
                    2_usize,
                )]),
                query_requested_tier_counts: BTreeMap::from([("official".to_owned(), 2_usize)]),
                structured_requested_tier_counts: BTreeMap::from([(
                    "verified-community".to_owned(),
                    2_usize,
                )]),
                effective_tier_counts: BTreeMap::new(),
                filtered_out_tier_counts: BTreeMap::from([
                    ("official".to_owned(), 2_usize),
                    ("verified-community".to_owned(), 2_usize),
                ]),
                trust_filter_applied_events: 2,
                conflicting_requested_tier_events: 2,
                trust_filtered_empty_events: 0,
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_400),
                last_event_id: Some("evt-2".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_401),
                last_agent_id: Some("agent-b".to_owned()),
                last_pack_id: Some("sales-intel".to_owned()),
                last_query: Some("trust:official search".to_owned()),
                last_returned: Some(0),
                last_trust_filter_applied: Some(true),
                last_conflicting_requested_tiers: Some(true),
                last_query_requested_tiers: vec!["official".to_owned()],
                last_structured_requested_tiers: vec!["verified-community".to_owned()],
                last_effective_tiers: Vec::new(),
                last_filtered_out_candidates: Some(2),
                last_filtered_out_tier_counts: BTreeMap::from([
                    ("official".to_owned(), 1_usize),
                    ("verified-community".to_owned(), 1_usize),
                ]),
                last_top_provider_ids: Vec::new(),
                last_triage_event_id: Some("evt-2".to_owned()),
                last_triage_label: Some("tool_search_trust_conflict".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_401),
                last_triage_agent_id: Some("agent-b".to_owned()),
                last_triage_summary: Some(
                    "query=\"trust:official search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                        .to_owned(),
                ),
                last_triage_hint: Some(
                    "align query trust prefixes with structured trust_tiers before retrying discovery"
                        .to_owned(),
                ),
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit discovery");
        let payload = audit_cli_json(&execution);

        assert!(rendered.contains("audit discovery"));
        assert!(rendered.contains("since_epoch_s=1700010400"));
        assert!(rendered.contains("until_epoch_s=1700010499"));
        assert!(rendered.contains("pack_id=sales-intel"));
        assert!(rendered.contains("agent_id=agent-b"));
        assert!(rendered.contains(
            "pack_id=sales-intel agent_id=agent-b event_id=evt-2 token_id=- kind=ToolSearchEvaluated"
        ));
        assert!(rendered.contains("kind=ToolSearchEvaluated"));
        assert!(rendered.contains("query_contains=trust:official"));
        assert!(rendered.contains("trust_tier=official"));
        assert!(rendered.contains("query_requested_tier_counts=official=2"));
        assert!(rendered.contains("structured_requested_tier_counts=verified-community=2"));
        assert!(rendered.contains("group_by=- group_count=0"));
        assert!(rendered.contains(
            "last_query=\"trust:official search\" last_returned=0 last_trust_filter_applied=true last_conflicting_requested_tiers=true"
        ));
        assert!(rendered.contains(
            "last_triage_hint=align query trust prefixes with structured trust_tiers before retrying discovery"
        ));

        assert_eq!(payload["command"], "discovery");
        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_400_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_499_u64);
        assert_eq!(payload["pack_id_filter"], "sales-intel");
        assert_eq!(payload["agent_id_filter"], "agent-b");
        assert_eq!(payload["event_id_filter"], "evt-2");
        assert_eq!(payload["token_id_filter"], Value::Null);
        assert_eq!(payload["kind_filter"], "ToolSearchEvaluated");
        assert_eq!(payload["triage_label_filter"], "tool_search_trust_conflict");
        assert_eq!(payload["query_contains_filter"], "trust:official");
        assert_eq!(payload["trust_tier_filter"], "official");
        assert_eq!(payload["group_by"], Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(payload["triage_counts"]["tool_search_trust_conflict"], 2);
        assert_eq!(payload["query_requested_tier_counts"]["official"], 2);
        assert_eq!(
            payload["structured_requested_tier_counts"]["verified-community"],
            2
        );
        assert_eq!(payload["last_pack_id"], "sales-intel");
        assert_eq!(payload["last_query"], "trust:official search");
        assert_eq!(payload["last_conflicting_requested_tiers"], true);
        assert_eq!(
            payload["last_triage_hint"],
            "align query trust prefixes with structured trust_tiers before retrying discovery"
        );
    }

    #[test]
    fn audit_recent_reports_missing_journal_with_first_write_hint() {
        let root = unique_temp_dir("loongclaw-audit-cli-missing");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("missing journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("first audit write"));
    }

    #[test]
    fn audit_recent_reports_in_memory_mode_when_journal_is_missing() {
        let root = unique_temp_dir("loongclaw-audit-cli-in-memory");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config_with_mode(
            &root,
            &journal_path,
            crate::mvp::config::AuditMode::InMemory,
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Recent {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                query_contains: None,
                trust_tier: None,
            },
        })
        .expect_err("missing in-memory journal should fail");

        assert!(error.contains("audit journal not found"));
        assert!(error.contains("durable audit retention is disabled"));
        assert!(error.contains("[audit].mode = \"in_memory\""));
    }

    #[test]
    fn audit_summary_rolls_up_event_kinds_and_last_seen_fields() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_100,
                    Some("agent-a"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-0".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-a".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_100,
                            expires_at_epoch_s: 1_700_010_200,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_101,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_102,
                    Some("agent-c"),
                    AuditEventKind::ProviderFailover {
                        pack_id: "sales-intel".to_owned(),
                        provider_id: "openai".to_owned(),
                        reason: "rate_limited".to_owned(),
                        stage: "response".to_owned(),
                        model: "gpt-5.1".to_owned(),
                        attempt: 1,
                        max_attempts: 3,
                        status_code: Some(429),
                        try_next_model: true,
                        auto_model_mode: true,
                        candidate_index: 0,
                        candidate_count: 2,
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_103,
                    Some("agent-d"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 2,
                        high_findings: 1,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: true,
                        block_reason: Some("unsigned plugin".to_owned()),
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_104,
                    Some("agent-e"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 2,
                        official_plugins: 1,
                        verified_community_plugins: 0,
                        unverified_plugins: 1,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 1,
                        blocked_auto_apply_plugins: 1,
                        review_required_plugin_ids: vec!["stdio-review".to_owned()],
                        review_required_bridges: vec!["process_stdio".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-6",
                    1_700_010_105,
                    Some("agent-f"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                limit,
                loaded_events,
                event_kind_counts,
                triage_counts,
                group_by,
                groups,
                first_timestamp_epoch_s,
                last_event_id,
                last_timestamp_epoch_s,
                last_agent_id,
                last_triage_event_id,
                last_triage_label,
                last_triage_event_kind,
                last_triage_timestamp_epoch_s,
                last_triage_agent_id,
                last_triage_summary,
                last_triage_hint,
            } => {
                assert_eq!(limit, 10);
                assert_eq!(loaded_events, 6);
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("PlaneInvoked".to_owned(), 1_usize),
                        ("PluginTrustEvaluated".to_owned(), 1_usize),
                        ("ProviderFailover".to_owned(), 1_usize),
                        ("SecurityScanEvaluated".to_owned(), 1_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("authorization_denied".to_owned(), 1_usize),
                        ("plugin_trust_blocked".to_owned(), 1_usize),
                        ("provider_failover".to_owned(), 1_usize),
                        ("security_scan_blocked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(group_by, None);
                assert!(groups.is_empty());
                assert_eq!(first_timestamp_epoch_s, Some(1_700_010_100));
                assert_eq!(last_event_id.as_deref(), Some("evt-6"));
                assert_eq!(last_timestamp_epoch_s, Some(1_700_010_105));
                assert_eq!(last_agent_id.as_deref(), Some("agent-f"));
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-5"));
                assert_eq!(last_triage_label.as_deref(), Some("plugin_trust_blocked"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("PluginTrustEvaluated")
                );
                assert_eq!(last_triage_timestamp_epoch_s, Some(1_700_010_104));
                assert_eq!(last_triage_agent_id.as_deref(), Some("agent-e"));
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "pack_id=sales-intel blocked_auto_apply_plugins=1 review_required_plugins=stdio-review"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "review plugin provenance and bootstrap policy before enabling auto-apply for the blocked plugins"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_groups_by_token() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-group-by-token");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_120,
                    Some("agent-issue"),
                    AuditEventKind::TokenIssued {
                        token: CapabilityToken {
                            token_id: "token-a".to_owned(),
                            pack_id: "sales-intel".to_owned(),
                            agent_id: "agent-issue".to_owned(),
                            allowed_capabilities: Default::default(),
                            issued_at_epoch_s: 1_700_010_120,
                            expires_at_epoch_s: 1_700_010_220,
                            generation: 0,
                        },
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_121,
                    Some("agent-deny"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-a".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_122,
                    Some("agent-revoke"),
                    AuditEventKind::TokenRevoked {
                        token_id: "token-a".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-4",
                    1_700_010_123,
                    Some("agent-b"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "ops-pack".to_owned(),
                        token_id: "token-b".to_owned(),
                        reason: "network egress denied".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-5",
                    1_700_010_124,
                    Some("agent-no-token"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Tool,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "tool.call".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: Some("token".to_owned()),
            },
        })
        .expect("execute audit summary grouped by token");
        let rendered = render_audit_cli_text(&execution).expect("render grouped audit summary");
        let payload = audit_cli_json(&execution);

        match execution.result {
            AuditCommandResult::Summary {
                group_by, groups, ..
            } => {
                assert_eq!(group_by.as_deref(), Some("token"));
                assert_eq!(groups.len(), 3);

                assert_eq!(groups[0].group_value.as_deref(), Some("token-a"));
                assert_eq!(groups[0].loaded_events, 3);
                assert_eq!(
                    groups[0].event_kind_counts,
                    BTreeMap::from([
                        ("AuthorizationDenied".to_owned(), 1_usize),
                        ("TokenIssued".to_owned(), 1_usize),
                        ("TokenRevoked".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    groups[0].triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(groups[0].last_event_id.as_deref(), Some("evt-3"));

                assert_eq!(groups[1].group_value.as_deref(), Some("token-b"));
                assert_eq!(groups[1].loaded_events, 1);
                assert_eq!(groups[1].last_event_id.as_deref(), Some("evt-4"));

                assert_eq!(groups[2].group_value, None);
                assert_eq!(groups[2].loaded_events, 1);
                assert_eq!(groups[2].last_event_id.as_deref(), Some("evt-5"));
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }

        assert!(rendered.contains("group_by=token group_count=3"));
        assert!(rendered.contains(
            "group[token]=token-a loaded_events=3 event_kind_counts=AuthorizationDenied=1,TokenIssued=1,TokenRevoked=1"
        ));
        assert!(rendered.contains("group[token]=(none) loaded_events=1"));

        assert_eq!(payload["group_by"], "token");
        assert_eq!(payload["groups"][0]["group_value"], "token-a");
        assert_eq!(payload["groups"][0]["loaded_events"], 3);
        assert_eq!(payload["groups"][1]["group_value"], "token-b");
        assert_eq!(payload["groups"][2]["group_value"], Value::Null);
    }

    #[test]
    fn audit_summary_ignores_non_blocking_security_scan_for_triage_rollups() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-non-blocking-scan");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_150,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_151,
                    Some("agent-b"),
                    AuditEventKind::SecurityScanEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 1,
                        total_findings: 1,
                        high_findings: 0,
                        medium_findings: 1,
                        low_findings: 0,
                        blocked: false,
                        block_reason: None,
                        categories: vec!["signature".to_owned()],
                        finding_ids: vec!["finding-1".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_152,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                triage_counts,
                last_triage_event_id,
                last_triage_label,
                last_triage_event_kind,
                last_triage_timestamp_epoch_s,
                last_triage_agent_id,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_event_id.as_deref(), Some("evt-1"));
                assert_eq!(last_triage_label.as_deref(), Some("authorization_denied"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("AuthorizationDenied")
                );
                assert_eq!(last_triage_timestamp_epoch_s, Some(1_700_010_150));
                assert_eq!(last_triage_agent_id.as_deref(), Some("agent-a"));
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some("pack_id=sales-intel token_id=token-1 reason=missing capability")
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_ignores_non_blocking_plugin_trust_for_triage_rollups() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-non-blocking-plugin-trust");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_250,
                    Some("agent-a"),
                    AuditEventKind::AuthorizationDenied {
                        pack_id: "sales-intel".to_owned(),
                        token_id: "token-1".to_owned(),
                        reason: "missing capability".to_owned(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_251,
                    Some("agent-b"),
                    AuditEventKind::PluginTrustEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        scanned_plugins: 2,
                        official_plugins: 1,
                        verified_community_plugins: 1,
                        unverified_plugins: 0,
                        high_risk_plugins: 1,
                        high_risk_unverified_plugins: 0,
                        blocked_auto_apply_plugins: 0,
                        review_required_plugin_ids: vec!["ffi-reviewed".to_owned()],
                        review_required_bridges: vec!["native_ffi".to_owned()],
                    },
                ),
                sample_audit_event(
                    "evt-3",
                    1_700_010_252,
                    Some("agent-c"),
                    AuditEventKind::PlaneInvoked {
                        pack_id: "sales-intel".to_owned(),
                        plane: ExecutionPlane::Runtime,
                        tier: PlaneTier::Core,
                        primary_adapter: "runtime".to_owned(),
                        delegated_core_adapter: None,
                        operation: "turn.complete".to_owned(),
                        required_capabilities: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                triage_counts,
                last_triage_label,
                last_triage_event_kind,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([("authorization_denied".to_owned(), 1_usize)])
                );
                assert_eq!(last_triage_label.as_deref(), Some("authorization_denied"));
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("AuthorizationDenied")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some("pack_id=sales-intel token_id=token-1 reason=missing capability")
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "grant the required capability or retry with a token scoped for the requested operation"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_tracks_tool_search_trust_conflict_triage() {
        let root = unique_temp_dir("loongclaw-audit-cli-summary-tool-search-trust");
        let journal_path = root.join("audit").join("events.jsonl");
        let config_path = write_audit_config(&root, &journal_path);
        write_journal(
            &journal_path,
            &[
                sample_audit_event(
                    "evt-1",
                    1_700_010_260,
                    Some("agent-a"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: Vec::new(),
                        structured_requested_tiers: vec!["official".to_owned()],
                        effective_tiers: vec!["official".to_owned()],
                        conflicting_requested_tiers: false,
                        filtered_out_candidates: 1,
                        filtered_out_tier_counts: BTreeMap::from([(
                            "verified-community".to_owned(),
                            1_usize,
                        )]),
                        top_provider_ids: Vec::new(),
                    },
                ),
                sample_audit_event(
                    "evt-2",
                    1_700_010_261,
                    Some("agent-b"),
                    AuditEventKind::ToolSearchEvaluated {
                        pack_id: "sales-intel".to_owned(),
                        query: "trust:official search".to_owned(),
                        returned: 0,
                        trust_filter_applied: true,
                        query_requested_tiers: vec!["official".to_owned()],
                        structured_requested_tiers: vec!["verified-community".to_owned()],
                        effective_tiers: Vec::new(),
                        conflicting_requested_tiers: true,
                        filtered_out_candidates: 2,
                        filtered_out_tier_counts: BTreeMap::from([
                            ("official".to_owned(), 1_usize),
                            ("verified-community".to_owned(), 1_usize),
                        ]),
                        top_provider_ids: Vec::new(),
                    },
                ),
            ],
        );

        let execution = execute_audit_command(AuditCommandOptions {
            config: Some(config_path.display().to_string()),
            json: false,
            command: AuditCommands::Summary {
                limit: 10,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect("execute audit summary");

        match execution.result {
            AuditCommandResult::Summary {
                event_kind_counts,
                triage_counts,
                last_triage_label,
                last_triage_event_kind,
                last_triage_summary,
                last_triage_hint,
                ..
            } => {
                assert_eq!(
                    event_kind_counts,
                    BTreeMap::from([("ToolSearchEvaluated".to_owned(), 2_usize)])
                );
                assert_eq!(
                    triage_counts,
                    BTreeMap::from([
                        ("tool_search_trust_conflict".to_owned(), 1_usize),
                        ("tool_search_trust_empty".to_owned(), 1_usize),
                    ])
                );
                assert_eq!(
                    last_triage_label.as_deref(),
                    Some("tool_search_trust_conflict")
                );
                assert_eq!(
                    last_triage_event_kind.as_deref(),
                    Some("ToolSearchEvaluated")
                );
                assert_eq!(
                    last_triage_summary.as_deref(),
                    Some(
                        "query=\"trust:official search\" trust_scope=- conflicting_requested_tiers=true filtered_out_candidates=2 top_provider_ids=-"
                    )
                );
                assert_eq!(
                    last_triage_hint.as_deref(),
                    Some(
                        "align query trust prefixes with structured trust_tiers before retrying discovery"
                    )
                );
            }
            other => panic!("unexpected audit command result: {other:?}"),
        }
    }

    #[test]
    fn audit_summary_rejects_excessive_limit() {
        let mut env = ScopedEnv::new();
        env.set(
            "HOME",
            unique_temp_dir("loongclaw-audit-cli-large-summary-limit-home"),
        );

        let error = execute_audit_command(AuditCommandOptions {
            config: None,
            json: false,
            command: AuditCommands::Summary {
                limit: 10_001,
                since_epoch_s: None,
                until_epoch_s: None,
                pack_id: None,
                agent_id: None,
                event_id: None,
                token_id: None,
                kind: None,
                triage_label: None,
                group_by: None,
            },
        })
        .expect_err("excessive summary limit should fail");

        assert!(error.contains("audit summary limit must be between 1 and 10000"));
    }

    #[test]
    fn audit_summary_text_includes_triage_counts_and_last_seen_fields() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_100),
            until_epoch_s_filter: Some(1_700_010_199),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-3".to_owned()),
            token_id_filter: Some("token-2".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 50,
                loaded_events: 3,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 2_usize),
                    ("PlaneInvoked".to_owned(), 1_usize),
                ]),
                triage_counts: BTreeMap::from([
                    ("authorization_denied".to_owned(), 2_usize),
                    ("security_scan_blocked".to_owned(), 1_usize),
                ]),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_100),
                last_event_id: Some("evt-3".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_102),
                last_agent_id: Some("agent-c".to_owned()),
                last_triage_event_id: Some("evt-2".to_owned()),
                last_triage_label: Some("authorization_denied".to_owned()),
                last_triage_event_kind: Some("AuthorizationDenied".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_101),
                last_triage_agent_id: Some("agent-b".to_owned()),
                last_triage_summary: Some(
                    "pack_id=sales-intel token_id=token-2 reason=missing capability".to_owned(),
                ),
                last_triage_hint: Some(
                    "grant the required capability or retry with a token scoped for the requested operation"
                        .to_owned(),
                ),
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit summary");

        assert!(rendered.contains("audit summary"));
        assert!(rendered.contains("since_epoch_s=1700010100"));
        assert!(rendered.contains("until_epoch_s=1700010199"));
        assert!(rendered.contains(
            "pack_id=- agent_id=- event_id=evt-3 token_id=token-2 kind=- triage_label=-"
        ));
        assert!(rendered.contains("loaded_events=3"));
        assert!(rendered.contains("first_timestamp_epoch_s=1700010100"));
        assert!(rendered.contains("AuthorizationDenied=2"));
        assert!(rendered.contains("PlaneInvoked=1"));
        assert!(rendered.contains("triage_counts=authorization_denied=2,security_scan_blocked=1"));
        assert!(rendered.contains("group_by=- group_count=0"));
        assert!(rendered.contains("last_event_id=evt-3"));
        assert!(rendered.contains("last_agent_id=agent-c"));
        assert!(rendered.contains("last_triage_event_id=evt-2"));
        assert!(rendered.contains("last_triage_label=authorization_denied"));
        assert!(rendered.contains("last_triage_event_kind=AuthorizationDenied"));
        assert!(rendered.contains("last_triage_agent_id=agent-b"));
        assert!(rendered.contains(
            "last_triage_summary=pack_id=sales-intel token_id=token-2 reason=missing capability"
        ));
        assert!(rendered.contains(
            "last_triage_hint=grant the required capability or retry with a token scoped for the requested operation"
        ));
    }

    #[test]
    fn audit_summary_json_includes_triage_fields() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: Some(1_700_010_200),
            until_epoch_s_filter: Some(1_700_010_299),
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: Some("evt-2".to_owned()),
            token_id_filter: Some("token-1".to_owned()),
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 25,
                loaded_events: 2,
                event_kind_counts: BTreeMap::from([
                    ("AuthorizationDenied".to_owned(), 1_usize),
                    ("PlaneInvoked".to_owned(), 1_usize),
                ]),
                triage_counts: BTreeMap::from([("authorization_denied".to_owned(), 1_usize)]),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_200),
                last_event_id: Some("evt-2".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_201),
                last_agent_id: Some("agent-b".to_owned()),
                last_triage_event_id: Some("evt-1".to_owned()),
                last_triage_label: Some("authorization_denied".to_owned()),
                last_triage_event_kind: Some("AuthorizationDenied".to_owned()),
                last_triage_timestamp_epoch_s: Some(1_700_010_200),
                last_triage_agent_id: Some("agent-a".to_owned()),
                last_triage_summary: Some(
                    "pack_id=sales-intel token_id=token-1 reason=missing capability".to_owned(),
                ),
                last_triage_hint: Some(
                    "grant the required capability or retry with a token scoped for the requested operation"
                        .to_owned(),
                ),
            },
        };

        let payload = audit_cli_json(&execution);

        assert_eq!(payload["since_epoch_s_filter"], 1_700_010_200_u64);
        assert_eq!(payload["until_epoch_s_filter"], 1_700_010_299_u64);
        assert_eq!(payload["event_id_filter"], "evt-2");
        assert_eq!(payload["token_id_filter"], "token-1");
        assert_eq!(payload["group_by"], Value::Null);
        assert_eq!(payload["groups"], json!([]));
        assert_eq!(payload["first_timestamp_epoch_s"], 1_700_010_200_u64);
        assert_eq!(payload["triage_counts"]["authorization_denied"], 1);
        assert_eq!(payload["last_triage_event_id"], "evt-1");
        assert_eq!(payload["last_triage_label"], "authorization_denied");
        assert_eq!(payload["last_triage_event_kind"], "AuthorizationDenied");
        assert_eq!(payload["last_triage_timestamp_epoch_s"], 1_700_010_200_u64);
        assert_eq!(payload["last_triage_agent_id"], "agent-a");
        assert_eq!(
            payload["last_triage_summary"],
            "pack_id=sales-intel token_id=token-1 reason=missing capability"
        );
        assert_eq!(
            payload["last_triage_hint"],
            "grant the required capability or retry with a token scoped for the requested operation"
        );
    }

    #[test]
    fn audit_summary_json_uses_empty_and_null_triage_fields_when_no_triage_events_exist() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: None,
            until_epoch_s_filter: None,
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: None,
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 10,
                loaded_events: 1,
                event_kind_counts: BTreeMap::from([("TokenIssued".to_owned(), 1_usize)]),
                triage_counts: BTreeMap::new(),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_300),
                last_event_id: Some("evt-1".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_300),
                last_agent_id: Some("agent-a".to_owned()),
                last_triage_event_id: None,
                last_triage_label: None,
                last_triage_event_kind: None,
                last_triage_timestamp_epoch_s: None,
                last_triage_agent_id: None,
                last_triage_summary: None,
                last_triage_hint: None,
            },
        };

        let payload = audit_cli_json(&execution);

        assert_eq!(
            payload["triage_counts"].as_object(),
            Some(&serde_json::Map::new())
        );
        assert_eq!(payload["last_triage_event_id"], Value::Null);
        assert_eq!(payload["last_triage_label"], Value::Null);
        assert_eq!(payload["last_triage_event_kind"], Value::Null);
        assert_eq!(payload["last_triage_timestamp_epoch_s"], Value::Null);
        assert_eq!(payload["last_triage_agent_id"], Value::Null);
        assert_eq!(payload["last_triage_summary"], Value::Null);
        assert_eq!(payload["last_triage_hint"], Value::Null);
    }

    #[test]
    fn audit_summary_text_uses_placeholders_when_no_triage_events_exist() {
        let execution = AuditCommandExecution {
            resolved_config_path: "/tmp/loongclaw.toml".to_owned(),
            journal_path: "/tmp/audit/events.jsonl".to_owned(),
            since_epoch_s_filter: None,
            until_epoch_s_filter: None,
            pack_id_filter: None,
            agent_id_filter: None,
            event_id_filter: None,
            token_id_filter: None,
            kind_filter: None,
            triage_label_filter: None,
            query_contains_filter: None,
            trust_tier_filter: None,
            result: AuditCommandResult::Summary {
                limit: 10,
                loaded_events: 1,
                event_kind_counts: BTreeMap::from([("TokenIssued".to_owned(), 1_usize)]),
                triage_counts: BTreeMap::new(),
                group_by: None,
                groups: Vec::new(),
                first_timestamp_epoch_s: Some(1_700_010_300),
                last_event_id: Some("evt-1".to_owned()),
                last_timestamp_epoch_s: Some(1_700_010_300),
                last_agent_id: Some("agent-a".to_owned()),
                last_triage_event_id: None,
                last_triage_label: None,
                last_triage_event_kind: None,
                last_triage_timestamp_epoch_s: None,
                last_triage_agent_id: None,
                last_triage_summary: None,
                last_triage_hint: None,
            },
        };

        let rendered = render_audit_cli_text(&execution).expect("render audit summary");

        assert!(rendered.contains("triage_counts=-"));
        assert!(rendered.contains("last_triage_event_id=-"));
        assert!(rendered.contains("last_triage_label=-"));
        assert!(rendered.contains("last_triage_event_kind=-"));
        assert!(rendered.contains("last_triage_summary=-"));
        assert!(rendered.contains("last_triage_hint=-"));
    }
}
