use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use clap::Subcommand;

use crate::kernel::{
    AuditEvent, AuditEventKind, AuditRepairOutcome, PluginTrustTier, repair_jsonl_audit_journal,
    verify_jsonl_audit_journal,
};
use loong_spec::CliResult;
use serde_json::{Map, Value, json};

const MAX_AUDIT_WINDOW: usize = 10_000;

#[path = "audit_cli_discovery_support.rs"]
mod discovery_support;

use self::discovery_support::{
    audit_discovery_groups_json, discovery_group_correlated_remediation_command,
    discovery_group_correlated_summary_command, discovery_group_drill_down_command,
};

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
    /// Verify the integrity chain of the durable audit journal
    Verify,
    /// Repair legacy journals missing integrity sidecars
    Repair,
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
    Verify {
        loaded_events: usize,
        verified_events: usize,
        valid: bool,
        last_entry_hash: Option<String>,
        first_invalid_line: Option<usize>,
        reason: Option<String>,
    },
    Repair {
        total_events: usize,
        repaired_events: usize,
        already_valid_events: usize,
        outcome: String,
        refused_line: Option<usize>,
        refused_reason: Option<String>,
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
        AuditCommands::Verify => (
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "audit verify",
        ),
        AuditCommands::Repair => (
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "audit repair",
        ),
    };
    let _limit = if matches!(command, AuditCommands::Verify | AuditCommands::Repair) {
        0
    } else {
        validate_audit_limit(limit, command_name)?
    };
    if !matches!(command, AuditCommands::Verify | AuditCommands::Repair) {
        validate_audit_time_range(since_epoch_s_filter, until_epoch_s_filter, command_name)?;
    }
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
    let result = match command {
        AuditCommands::Recent { limit, .. } => {
            let audit_window =
                load_audit_event_window(&config.audit, &journal_path, limit, &filter)?;
            let events = audit_window.events;
            AuditCommandResult::Recent { limit, events }
        }
        AuditCommands::Summary {
            limit, group_by, ..
        } => {
            let audit_window =
                load_audit_event_window(&config.audit, &journal_path, limit, &filter)?;
            let events = audit_window.events;
            summarize_audit_events(limit, group_by, &events)
        }
        AuditCommands::Discovery {
            limit, group_by, ..
        } => {
            let audit_window =
                load_audit_event_window(&config.audit, &journal_path, limit, &filter)?;
            let events = audit_window.events;
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
        } => {
            let audit_window =
                load_audit_event_window(&config.audit, &journal_path, limit, &filter)?;
            let total_matching_events = audit_window.total_matching_events;
            let events = audit_window.events;
            summarize_token_trail(limit, token_id, events, total_matching_events)
        }
        AuditCommands::Verify => {
            ensure_audit_journal_preflight(&config.audit, &journal_path)?;
            let report = verify_jsonl_audit_journal(&journal_path)
                .map_err(|error| format!("verify audit journal failed: {error}"))?;
            AuditCommandResult::Verify {
                loaded_events: report.total_events,
                verified_events: report.verified_events,
                valid: report.valid,
                last_entry_hash: report.last_entry_hash,
                first_invalid_line: report.first_invalid_line,
                reason: report.reason,
            }
        }
        AuditCommands::Repair => {
            ensure_audit_journal_preflight(&config.audit, &journal_path)?;
            let report = repair_jsonl_audit_journal(&journal_path)
                .map_err(|error| format!("repair audit journal failed: {error}"))?;
            let (outcome_str, refused_line, refused_reason) = match &report.outcome {
                AuditRepairOutcome::Healthy => ("healthy".to_owned(), None, None),
                AuditRepairOutcome::Repaired => ("repaired".to_owned(), None, None),
                AuditRepairOutcome::Refused { line, reason } => {
                    ("refused".to_owned(), Some(*line), Some(reason.clone()))
                }
            };
            AuditCommandResult::Repair {
                total_events: report.total_events,
                repaired_events: report.repaired_events,
                already_valid_events: report.already_valid_events,
                outcome: outcome_str,
                refused_line,
                refused_reason,
            }
        }
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
        AuditCommandResult::Verify {
            loaded_events,
            verified_events,
            valid,
            last_entry_hash,
            first_invalid_line,
            reason,
        } => {
            let mut payload = audit_cli_base_json(execution, "verify");
            payload.insert("loaded_events".to_owned(), json!(loaded_events));
            payload.insert("verified_events".to_owned(), json!(verified_events));
            payload.insert("valid".to_owned(), json!(valid));
            payload.insert("last_entry_hash".to_owned(), json!(last_entry_hash));
            payload.insert("first_invalid_line".to_owned(), json!(first_invalid_line));
            payload.insert("reason".to_owned(), json!(reason));
            Value::Object(payload)
        }
        AuditCommandResult::Repair {
            total_events,
            repaired_events,
            already_valid_events,
            outcome,
            refused_line,
            refused_reason,
        } => {
            let mut payload = audit_cli_base_json(execution, "repair");
            payload.insert("total_events".to_owned(), json!(total_events));
            payload.insert("repaired_events".to_owned(), json!(repaired_events));
            payload.insert(
                "already_valid_events".to_owned(),
                json!(already_valid_events),
            );
            payload.insert("outcome".to_owned(), json!(outcome));
            payload.insert("refused_line".to_owned(), json!(refused_line));
            payload.insert("refused_reason".to_owned(), json!(refused_reason));
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
        AuditCommandResult::Verify {
            loaded_events,
            verified_events,
            valid,
            last_entry_hash,
            first_invalid_line,
            reason,
        } => {
            lines.push(format!(
                "audit verify config={} journal={} loaded_events={} verified_events={} valid={}",
                execution.resolved_config_path,
                execution.journal_path,
                loaded_events,
                verified_events,
                valid
            ));
            lines.push(format!(
                "last_entry_hash={} first_invalid_line={} reason={}",
                last_entry_hash.as_deref().unwrap_or("-"),
                first_invalid_line
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                reason.as_deref().unwrap_or("-")
            ));
        }
        AuditCommandResult::Repair {
            total_events,
            repaired_events,
            already_valid_events,
            outcome,
            refused_line,
            refused_reason,
        } => {
            lines.push(format!(
                "audit repair config={} journal={} total_events={} repaired_events={} already_valid_events={} outcome={}",
                execution.resolved_config_path,
                execution.journal_path,
                total_events,
                repaired_events,
                already_valid_events,
                outcome
            ));
            if let (Some(line), Some(reason)) = (refused_line, refused_reason) {
                lines.push(format!("refused_line={line} refused_reason={reason}"));
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

fn audit_journal_missing_hint(audit: &crate::mvp::config::AuditConfig) -> &'static str {
    if audit.mode == crate::mvp::config::AuditMode::InMemory {
        return "durable audit retention is disabled because [audit].mode = \"in_memory\"";
    }

    "journal is created on first audit write"
}

fn ensure_audit_journal_preflight(
    audit: &crate::mvp::config::AuditConfig,
    journal_path: &Path,
) -> CliResult<()> {
    if !journal_path.exists() {
        let hint = audit_journal_missing_hint(audit);
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

    Ok(())
}

fn load_audit_event_window(
    audit: &crate::mvp::config::AuditConfig,
    journal_path: &Path,
    limit: usize,
    filter: &AuditEventFilter,
) -> CliResult<AuditEventWindow> {
    ensure_audit_journal_preflight(audit, journal_path)?;

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
        request_id,
        auth_error_code,
        ..
    } = kind
    {
        let summary = format!(
            "pack_id={} provider_id={} reason={} attempt={}/{} request_id={} auth_error_code={}",
            pack_id,
            provider_id,
            reason,
            attempt,
            max_attempts,
            request_id.as_deref().unwrap_or("-"),
            auth_error_code.as_deref().unwrap_or("-")
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
        // AuditEventKind is non_exhaustive in loong-contracts, so keep a visible
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
            request_id,
            auth_error_code,
            ..
        } => format!(
            "pack_id={} provider_id={} reason={} attempt={}/{} request_id={} auth_error_code={}",
            pack_id,
            provider_id,
            reason,
            attempt,
            max_attempts,
            request_id.as_deref().unwrap_or("-"),
            auth_error_code.as_deref().unwrap_or("-")
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
#[path = "audit_cli_tests.rs"]
mod tests;
