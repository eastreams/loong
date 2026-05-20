fn execute_skills_list_for_audience(
    tool_name: String,
    config: &super::runtime_config::ToolRuntimeConfig,
    audience: SkillAudience,
) -> Result<ToolCoreOutcome, String> {
    let inventory = discover_skill_inventory(config)?;
    let filtered = filter_inventory_for_audience(inventory, audience);
    let bundled_packs = match audience {
        SkillAudience::Operator => json!(super::bundled_skills::bundled_skill_packs()),
        SkillAudience::Model => json!([]),
    };
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "skills": serialize_skill_entries_for_audience(filtered.skills, audience),
            "shadowed_skills": serialize_skill_entries_for_audience(filtered.shadowed_skills, audience),
            "bundled_packs": bundled_packs,
        }),
    })
}

fn execute_skills_inspect_for_audience(
    tool_name: String,
    config: &super::runtime_config::ToolRuntimeConfig,
    skill_id: &str,
    audience: SkillAudience,
) -> Result<ToolCoreOutcome, String> {
    let inventory = discover_skill_inventory(config)?;
    let skill = resolve_discovered_skill(&inventory, skill_id)?;
    ensure_skill_access_for_audience(&skill, audience)?;
    let instructions = load_discovered_skill_markdown(config, &skill)?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "skill": serialize_skill_entry_for_audience(skill, audience),
            "instructions_preview": build_preview(instructions.as_str(), 240),
            "shadowed_skills": serialize_skill_entries_for_audience(
                inventory
                    .shadowed_skills
                    .into_iter()
                    .filter(|entry| entry.skill_id == skill_id)
                    .filter(|entry| skill_is_visible_to_audience(entry, audience))
                    .collect::<Vec<_>>(),
                audience,
            ),
        }),
    })
}

fn execute_skills_discovery_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
    mode: SkillDiscoveryMode,
) -> Result<ToolCoreOutcome, String> {
    let (trimmed_query, limit) = parse_skills_discovery_request(&request)?;
    let tool_name = discovery_tool_name(mode);

    let inventory = discover_skill_inventory(config)?;
    let visible_skill_count = inventory.skills.len();
    let shadowed_skill_count = inventory.shadowed_skills.len();
    let blocked_skill_count = inventory.blocked_skill_errors.len();

    let results = build_ranked_skill_discovery_results(
        inventory.skills.as_slice(),
        trimmed_query.as_str(),
        limit,
        SkillDiscoveryResolution::Active,
    );
    let shadowed_results = build_ranked_skill_discovery_results(
        inventory.shadowed_skills.as_slice(),
        trimmed_query.as_str(),
        limit,
        SkillDiscoveryResolution::Shadowed,
    );
    let blocked_results = build_ranked_blocked_skill_discovery_results(
        &inventory.blocked_skill_errors,
        trimmed_query.as_str(),
        limit,
    );
    let inventory_summary = SkillDiscoveryInventorySummary {
        visible_skill_count,
        shadowed_skill_count,
        blocked_skill_count,
    };

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "query": trimmed_query,
            "limit": limit,
            "inventory_summary": inventory_summary,
            "results": results,
            "shadowed_results": shadowed_results,
            "blocked_results": blocked_results,
        }),
    })
}

fn parse_skills_discovery_request(request: &ToolCoreRequest) -> Result<(String, usize), String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| format!("{} payload must be an object", request.tool_name))?;
    let trimmed_query = payload
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} requires payload.query", request.tool_name))?;
    let raw_limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{} requires payload.limit", request.tool_name))?;
    if raw_limit == 0 {
        return Err(format!(
            "{} payload.limit must be greater than zero",
            request.tool_name
        ));
    }
    let limit = usize::try_from(raw_limit)
        .map_err(|error| format!("invalid discovery limit `{raw_limit}`: {error}"))?;
    Ok((trimmed_query.to_owned(), limit))
}

fn discovery_tool_name(mode: SkillDiscoveryMode) -> &'static str {
    // Search and recommend currently share the same discovery pipeline.
    // If mode-specific ranking or filtering lands later, update both this
    // mapping and the downstream discovery behavior together.
    match mode {
        SkillDiscoveryMode::Search => "skills.search",
        SkillDiscoveryMode::Recommend => "skills.recommend",
    }
}

fn build_ranked_skill_discovery_results(
    skills: &[DiscoveredSkillEntry],
    query: &str,
    limit: usize,
    resolution: SkillDiscoveryResolution,
) -> Vec<RankedSkillDiscoveryResult> {
    let mut searchable_entries = Vec::new();
    let mut skills_by_identity = BTreeMap::new();

    for skill in skills {
        let search_identity = skill_search_identity(skill, resolution);
        let searchable_entry = searchable_entry_from_skill(skill, resolution, &search_identity);
        searchable_entries.push(searchable_entry);
        skills_by_identity.insert(search_identity, skill.clone());
    }

    let ranking = rank_searchable_entries(searchable_entries, query, limit);
    let mut ranked_results = Vec::new();
    for ranked_entry in ranking.results {
        let search_identity = ranked_entry.entry.canonical_name;
        let Some(skill) = skills_by_identity.remove(&search_identity) else {
            continue;
        };
        let match_reasons = render_skill_discovery_match_reasons(ranked_entry.why);
        let limitations = build_skill_discovery_limitations(&skill, resolution);
        let ranked_result = RankedSkillDiscoveryResult {
            skill,
            resolution,
            match_reasons,
            limitations,
        };
        ranked_results.push(ranked_result);
    }

    ranked_results
}

fn build_ranked_blocked_skill_discovery_results(
    blocked_skill_errors: &BTreeMap<String, String>,
    query: &str,
    limit: usize,
) -> Vec<RankedBlockedSkillDiscoveryResult> {
    let mut searchable_entries = Vec::new();
    let mut blocked_entries_by_identity = BTreeMap::new();

    for (skill_id, error) in blocked_skill_errors {
        let search_identity = format!("{skill_id}::blocked");
        let required_fields = vec![skill_id.clone()];
        let required_field_groups = vec![vec![skill_id.clone()]];
        let tags = vec![
            "skills".to_owned(),
            "skill".to_owned(),
            "blocked".to_owned(),
        ];
        let searchable_entry = searchable_entry_from_manual_definition(
            search_identity.as_str(),
            error.as_str(),
            error.as_str(),
            required_fields,
            required_field_groups,
            tags,
        );
        searchable_entries.push(searchable_entry);
        blocked_entries_by_identity.insert(search_identity, (skill_id.clone(), error.clone()));
    }

    let ranking = rank_searchable_entries(searchable_entries, query, limit);
    let mut ranked_results = Vec::new();
    for ranked_entry in ranking.results {
        let search_identity = ranked_entry.entry.canonical_name;
        let Some((skill_id, error)) = blocked_entries_by_identity.remove(&search_identity) else {
            continue;
        };
        let match_reasons = render_skill_discovery_match_reasons(ranked_entry.why);
        let ranked_result = RankedBlockedSkillDiscoveryResult {
            skill_id,
            error,
            match_reasons,
        };
        ranked_results.push(ranked_result);
    }

    ranked_results
}

fn searchable_entry_from_skill(
    skill: &DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
    search_identity: &str,
) -> super::tool_search::SearchableToolEntry {
    let summary = build_skill_search_summary(skill);
    let argument_hint = build_skill_search_argument_hint(skill, resolution);
    let required_fields = build_skill_search_required_fields(skill);
    let required_field_groups = build_skill_search_required_field_groups(&required_fields);
    let tags = build_skill_search_tags(skill, resolution);
    searchable_entry_from_manual_definition(
        search_identity,
        summary.as_str(),
        argument_hint.as_str(),
        required_fields,
        required_field_groups,
        tags,
    )
}

fn build_skill_search_summary(skill: &DiscoveredSkillEntry) -> String {
    let display_name = skill.display_name.trim();
    let summary = skill.summary.trim();
    let display_name_missing = display_name.is_empty();
    if display_name_missing {
        return summary.to_owned();
    }
    let summary_missing = summary.is_empty();
    if summary_missing {
        return display_name.to_owned();
    }

    let display_name_matches_summary = display_name.eq_ignore_ascii_case(summary);
    if display_name_matches_summary {
        return display_name.to_owned();
    }

    format!("{display_name}. {summary}")
}

fn build_skill_search_argument_hint(
    skill: &DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
) -> String {
    let mut fragments = Vec::new();
    let display_name = skill.display_name.clone();
    fragments.push(display_name);

    let scope_fragment = format!("scope {}", discovered_skill_scope_id(skill.scope));
    fragments.push(scope_fragment);

    let source_kind_fragment = format!("source {}", skill.source_kind);
    fragments.push(source_kind_fragment);

    let visibility_fragment = format!(
        "visibility {}",
        skill_model_visibility_id(skill.model_visibility)
    );
    fragments.push(visibility_fragment);

    let invocation_fragment = format!(
        "invocation {}",
        invocation_policy_id(skill.invocation_policy)
    );
    fragments.push(invocation_fragment);
    if let Some(compatibility) = skill.compatibility.as_deref()
        && !compatibility.is_empty()
    {
        fragments.push(format!("compatibility {compatibility}"));
    }

    if resolution == SkillDiscoveryResolution::Shadowed {
        fragments.push("shadowed by a higher-precedence resolved skill".to_owned());
    }
    if !skill.active {
        fragments.push("inactive resolved skill".to_owned());
    }
    for issue in &skill.eligibility.issues {
        fragments.push(issue.clone());
    }
    for required_env in &skill.required_env {
        let fragment = format!("env {required_env}");
        fragments.push(fragment);
    }
    for required_bin in &skill.required_bin {
        let fragment = format!("binary {required_bin}");
        fragments.push(fragment);
    }
    for required_path in &skill.required_paths {
        let fragment = format!("path {required_path}");
        fragments.push(fragment);
    }
    for required_config in &skill.required_config {
        let fragment = format!("config {required_config}");
        fragments.push(fragment);
    }
    for allowed_tool in &skill.allowed_tools {
        let fragment = format!("allowed tool {allowed_tool}");
        fragments.push(fragment);
    }
    for blocked_tool in &skill.blocked_tools {
        let fragment = format!("blocked tool {blocked_tool}");
        fragments.push(fragment);
    }

    fragments.join("; ")
}

fn build_skill_search_required_fields(skill: &DiscoveredSkillEntry) -> Vec<String> {
    let mut required_fields = BTreeSet::new();
    for required_env in &skill.required_env {
        required_fields.insert(required_env.clone());
    }
    for required_bin in &skill.required_bin {
        required_fields.insert(required_bin.clone());
    }
    for required_path in &skill.required_paths {
        required_fields.insert(required_path.clone());
    }
    for required_config in &skill.required_config {
        required_fields.insert(required_config.clone());
    }

    required_fields.into_iter().collect()
}

fn build_skill_search_required_field_groups(required_fields: &[String]) -> Vec<Vec<String>> {
    let fields_missing = required_fields.is_empty();
    if fields_missing {
        return Vec::new();
    }

    vec![required_fields.to_vec()]
}

fn build_skill_search_tags(
    skill: &DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
) -> Vec<String> {
    let mut tags = BTreeSet::new();
    tags.insert("skills".to_owned());
    tags.insert("skill".to_owned());
    tags.insert(discovered_skill_scope_id(skill.scope).to_owned());
    tags.insert(skill.source_kind.clone());
    tags.insert(skill_model_visibility_id(skill.model_visibility).to_owned());
    tags.insert(invocation_policy_id(skill.invocation_policy).to_owned());
    tags.insert(skill_discovery_resolution_id(resolution).to_owned());

    let eligibility_tag = if skill.eligibility.available {
        "eligible"
    } else {
        "ineligible"
    };
    tags.insert(eligibility_tag.to_owned());

    if !skill.active {
        tags.insert("inactive".to_owned());
    }
    for allowed_tool in &skill.allowed_tools {
        tags.insert(allowed_tool.clone());
    }
    for blocked_tool in &skill.blocked_tools {
        tags.insert(blocked_tool.clone());
    }

    tags.into_iter().collect()
}

fn build_skill_discovery_limitations(
    skill: &DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
) -> Vec<String> {
    let mut limitations = Vec::new();
    if resolution == SkillDiscoveryResolution::Shadowed {
        limitations.push(
            "shadowed by a higher-precedence resolved skill with the same `skill_id`".to_owned(),
        );
    }
    if !skill.active {
        limitations.push("resolved winner is inactive".to_owned());
    }
    if skill.model_visibility == SkillModelVisibility::Hidden {
        limitations.push("hidden from the model surface".to_owned());
    }
    if skill.invocation_policy == SkillInvocationPolicy::Manual {
        limitations.push("manual-only invocation".to_owned());
    }
    for issue in &skill.eligibility.issues {
        limitations.push(issue.clone());
    }

    limitations
}

fn render_skill_discovery_match_reasons(raw_reasons: Vec<String>) -> Vec<String> {
    let mut rendered_reasons = Vec::new();
    for raw_reason in raw_reasons {
        let rendered_reason = render_skill_discovery_match_reason(raw_reason.as_str());
        rendered_reasons.push(rendered_reason);
    }

    rendered_reasons
}

fn render_skill_discovery_match_reason(raw_reason: &str) -> String {
    if raw_reason == "name_phrase" {
        return "matched skill id directly".to_owned();
    }
    if raw_reason == "summary_phrase" {
        return "matched display name or summary directly".to_owned();
    }
    if raw_reason == "argument_phrase" {
        return "matched discovery metadata or prerequisite text".to_owned();
    }
    if raw_reason == "schema_phrase" {
        return "matched required capability or prerequisite fields".to_owned();
    }
    if raw_reason == "tag_phrase" {
        return "matched discovery tags directly".to_owned();
    }
    if raw_reason == "coarse_fallback" {
        return "fallback discovery ranking kept this candidate visible".to_owned();
    }
    if raw_reason == "coarse_discovery_tool" {
        return "discovery-oriented metadata boosted this candidate".to_owned();
    }

    let Some((kind, value)) = raw_reason.split_once(':') else {
        return raw_reason.to_owned();
    };
    match kind {
        "name" => format!("skill id matched `{value}`"),
        "summary" => format!("display name or summary matched `{value}`"),
        "argument" => format!("metadata matched `{value}`"),
        "schema" => format!("requirements matched `{value}`"),
        "tag" => format!("classification matched `{value}`"),
        "concept" => format!("query intent matched `{value}`"),
        "category" => format!("query category matched `{value}`"),
        _ => raw_reason.to_owned(),
    }
}

fn skill_search_identity(
    skill: &DiscoveredSkillEntry,
    resolution: SkillDiscoveryResolution,
) -> String {
    let resolution_id = skill_discovery_resolution_id(resolution);
    format!("{}::{resolution_id}::{}", skill.skill_id, skill.sha256)
}

fn skill_discovery_resolution_id(resolution: SkillDiscoveryResolution) -> &'static str {
    match resolution {
        SkillDiscoveryResolution::Active => "active",
        SkillDiscoveryResolution::Shadowed => "shadowed",
    }
}

fn discovered_skill_scope_id(scope: DiscoveredSkillScope) -> &'static str {
    match scope {
        DiscoveredSkillScope::Managed => "managed",
        DiscoveredSkillScope::User => "user",
        DiscoveredSkillScope::Project => "project",
    }
}

fn skill_model_visibility_id(visibility: SkillModelVisibility) -> &'static str {
    match visibility {
        SkillModelVisibility::Visible => "visible",
        SkillModelVisibility::Hidden => "hidden",
    }
}

#[cfg(test)]
pub(super) fn installed_skill_snapshot_lines_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<String>, String> {
    let policy = resolve_effective_policy(config)?;
    if !policy.enabled || !policy.auto_expose_installed {
        return Ok(Vec::new());
    }
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    Ok(index
        .skills
        .into_iter()
        .filter_map(|entry| {
            if !entry.active {
                return None;
            }
            let rehydrated = rehydrate_installed_skill_entry(&install_root, entry).ok()?;
            let discovered = build_managed_discovered_skill_entry(config, rehydrated).ok()?;
            skill_is_visible_to_audience(&discovered, SkillAudience::Model).then(|| {
                format!(
                    "- {}: {}",
                    discovered.skill_id, INSTALLED_SKILL_SNAPSHOT_HINT
                )
            })
        })
        .collect())
}

pub(super) fn model_skill_catalog_section_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Option<String> {
    let visible_skills = model_visible_skill_catalog_entries_with_config(config);
    if visible_skills.is_empty() {
        return None;
    }

    let mut lines = vec![
        "[available_skills]".to_owned(),
        "The following skills provide specialized instructions for specific tasks.".to_owned(),
        "Only skills listed here are currently model-visible and runtime-eligible; manual-only or ineligible skills stay off this list.".to_owned(),
        "Use the read tool to load a listed skill's SKILL.md file when the task matches its description.".to_owned(),
        "Do not use tool.search or tool.invoke for routine model-driven skill loading; skills are read-first, not tool-discovery-first.".to_owned(),
        "When a skill file references a relative path, resolve it against the skill directory (the parent of SKILL.md) and use that absolute path in tool commands.".to_owned(),
        "<available_skills>".to_owned(),
    ];

    for skill in visible_skills {
        lines.push("  <skill>".to_owned());
        lines.push(format!(
            "    <name>{}</name>",
            xml_escape(skill.skill_id.as_str())
        ));
        lines.push(format!(
            "    <description>{}</description>",
            xml_escape(skill.description.as_str())
        ));
        lines.push(format!(
            "    <location>{}</location>",
            xml_escape(skill.location.as_str())
        ));
        lines.push("  </skill>".to_owned());
    }
    lines.push("</available_skills>".to_owned());

    Some(lines.join("\n"))
}

fn model_visible_skill_entries_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Vec<DiscoveredSkillEntry> {
    let policy = match resolve_effective_policy(config) {
        Ok(policy) => policy,
        Err(_) => return Vec::new(),
    };
    if !policy.enabled {
        return Vec::new();
    }

    let inventory = match discover_skill_inventory(config) {
        Ok(inventory) => inventory,
        Err(_) => return Vec::new(),
    };
    let filtered = filter_inventory_for_audience(inventory, SkillAudience::Model);

    filtered.skills
}

pub(crate) fn model_visible_skill_catalog_entries_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Vec<ModelVisibleSkillCatalogEntry> {
    model_visible_skill_entries_with_config(config)
        .into_iter()
        .map(|skill| {
            let skill_root = resolved_skill_root_path(&skill);
            ModelVisibleSkillCatalogEntry {
                skill_id: skill.skill_id,
                description: skill.summary,
                location: skill.skill_md_path,
                skill_root,
            }
        })
        .collect()
}

pub(crate) fn model_visible_skill_roots_with_config(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for skill in model_visible_skill_catalog_entries_with_config(config) {
        let Some(skill_root) = skill.skill_root else {
            continue;
        };
        let canonical = fs::canonicalize(&skill_root).unwrap_or(skill_root);
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }
    roots
}

fn build_skill_context_payload(
    config: &super::runtime_config::ToolRuntimeConfig,
    skill: &DiscoveredSkillEntry,
) -> Result<Value, String> {
    let raw_instructions = load_discovered_skill_markdown(config, skill)?;
    let skill_root = resolved_skill_root_path(skill);
    let resource_listing = skill_root
        .as_deref()
        .map(|path| list_skill_resources(path, DEFAULT_SKILL_RESOURCE_LIST_LIMIT))
        .transpose()?
        .unwrap_or_default();
    let instructions = render_structured_skill_instructions(
        skill,
        raw_instructions.as_str(),
        skill_root.as_deref(),
        &resource_listing,
    );

    Ok(json!({
        "skill_id": skill.skill_id,
        "display_name": skill.display_name,
        "summary": skill.summary,
        "scope": skill.scope,
        "source_path": skill.source_path,
        "install_path": skill.install_path,
        "skill_md_path": skill.skill_md_path,
        "skill_root": skill_root,
        "resource_listing": resource_listing,
        "instructions": instructions,
        "metadata": metadata_payload_from_skill(skill),
        "eligibility": skill.eligibility,
    }))
}

pub(crate) fn model_visible_skill_context_payload_for_path(
    config: &super::runtime_config::ToolRuntimeConfig,
    raw_path: &Path,
) -> Result<Option<Value>, String> {
    let requested_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else if let Some(file_root) = config.file_root.as_deref() {
        file_root.join(raw_path)
    } else {
        raw_path.to_path_buf()
    };
    let normalized_requested_path = fs::canonicalize(&requested_path).unwrap_or(requested_path);

    for skill in model_visible_skill_entries_with_config(config) {
        let skill_md_path = PathBuf::from(skill.skill_md_path.as_str());
        let normalized_skill_md_path = fs::canonicalize(&skill_md_path).unwrap_or(skill_md_path);
        if normalized_skill_md_path == normalized_requested_path {
            return build_skill_context_payload(config, &skill).map(Some);
        }
    }

    Ok(None)
}

pub(crate) fn model_visible_skill_context_payload_for_skill_id(
    config: &super::runtime_config::ToolRuntimeConfig,
    skill_id: &str,
) -> Result<Option<Value>, String> {
    let normalized_skill_id = skill_id.trim();
    if normalized_skill_id.is_empty() {
        return Ok(None);
    }

    for skill in model_visible_skill_entries_with_config(config) {
        if skill.skill_id != normalized_skill_id {
            continue;
        }
        return build_skill_context_payload(config, &skill).map(Some);
    }

    Ok(None)
}

fn discover_managed_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    let install_root = resolve_install_root(config);
    let index = load_installed_skill_index(&install_root)?;
    let mut discovery = SkillCandidateDiscovery::default();
    for entry in index.skills {
        let skill_id = entry.skill_id.clone();
        let source_path = entry.source_path.clone();
        let entry = match rehydrate_installed_skill_entry(&install_root, entry) {
            Ok(entry) => entry,
            Err(error) => {
                discovery.blocked_candidates.push(BlockedSkillCandidate {
                    skill_id,
                    scope: DiscoveredSkillScope::Managed,
                    probe_rank: 0,
                    root_rank: 0,
                    source_path,
                    error,
                });
                continue;
            }
        };
        let entry = match build_managed_discovered_skill_entry(config, entry) {
            Ok(entry) => entry,
            Err(error) => {
                discovery.blocked_candidates.push(BlockedSkillCandidate {
                    skill_id,
                    scope: DiscoveredSkillScope::Managed,
                    probe_rank: 0,
                    root_rank: 0,
                    source_path,
                    error,
                });
                continue;
            }
        };
        discovery.candidates.push(DiscoveredSkillCandidate {
            probe_rank: 0,
            root_rank: 0,
            entry,
        });
    }
    Ok(discovery)
}

fn discover_user_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    let Some(home_root) = user_home_dir() else {
        return Ok(SkillCandidateDiscovery::default());
    };
    discover_scoped_skill_candidates(
        config,
        &[home_root],
        DiscoveredSkillScope::User,
        &USER_DISCOVERY_DIRS,
    )
}

fn discover_project_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<SkillCandidateDiscovery, String> {
    discover_scoped_skill_candidates(
        config,
        &project_discovery_probe_roots(config),
        DiscoveredSkillScope::Project,
        &PROJECT_DISCOVERY_DIRS,
    )
}

fn discover_scoped_skill_candidates(
    config: &super::runtime_config::ToolRuntimeConfig,
    probe_roots: &[PathBuf],
    scope: DiscoveredSkillScope,
    dir_specs: &[(&str, usize)],
) -> Result<SkillCandidateDiscovery, String> {
    let mut discovery = SkillCandidateDiscovery::default();
    let mut seen = BTreeSet::new();
    for (probe_rank, probe_root) in probe_roots.iter().enumerate() {
        for (relative_dir, root_rank) in dir_specs {
            let container = probe_root.join(relative_dir);
            if !container.is_dir() {
                continue;
            }
            for skill_root in find_discoverable_skill_roots(&container) {
                let skill_md_path = skill_root.join(DEFAULT_SKILL_FILENAME);
                let key = skill_md_path.display().to_string();
                if !seen.insert(key.clone()) {
                    continue;
                }
                let skill_markdown = match load_directory_skill_markdown(&skill_root) {
                    Ok(skill_markdown) => skill_markdown,
                    Err(error) => {
                        discovery.blocked_candidates.push(BlockedSkillCandidate {
                            skill_id: derive_skill_id(&skill_root),
                            scope,
                            probe_rank,
                            root_rank: *root_rank,
                            source_path: skill_root.display().to_string(),
                            error,
                        });
                        continue;
                    }
                };
                let skill_id = derive_skill_id_from_markdown(&skill_root, skill_markdown.as_str());
                let entry = match build_discovered_skill_entry(
                    config,
                    scope,
                    "directory".to_owned(),
                    skill_root.display().to_string(),
                    key,
                    skill_id.clone(),
                    skill_markdown.as_str(),
                    true,
                    None,
                ) {
                    Ok(entry) => entry,
                    Err(error) => {
                        discovery.blocked_candidates.push(BlockedSkillCandidate {
                            skill_id,
                            scope,
                            probe_rank,
                            root_rank: *root_rank,
                            source_path: skill_root.display().to_string(),
                            error,
                        });
                        continue;
                    }
                };
                discovery.candidates.push(DiscoveredSkillCandidate {
                    probe_rank,
                    root_rank: *root_rank,
                    entry,
                });
            }
        }
    }
    Ok(discovery)
}

fn project_discovery_probe_roots(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Vec<PathBuf> {
    let Some(project_root) = project_discovery_root(config) else {
        return Vec::new();
    };
    let project_root = dunce::canonicalize(&project_root).unwrap_or(project_root);

    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        let current_dir = dunce::canonicalize(&current_dir).unwrap_or(current_dir);
        if current_dir.starts_with(&project_root) {
            let mut next = Some(current_dir.as_path());
            while let Some(path) = next {
                roots.push(path.to_path_buf());
                if path == project_root.as_path() {
                    break;
                }
                next = path.parent();
            }
        } else {
            roots.push(project_root);
        }
    } else {
        roots.push(project_root);
    }

    let mut seen = BTreeSet::new();
    roots.retain(|root| seen.insert(root.display().to_string()));
    roots
}

fn project_discovery_root(config: &super::runtime_config::ToolRuntimeConfig) -> Option<PathBuf> {
    config
        .config_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| config.file_root.clone())
        .or_else(|| std::env::current_dir().ok())
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn resolve_discovered_skill(
    inventory: &SkillDiscoveryInventory,
    skill_id: &str,
) -> Result<DiscoveredSkillEntry, String> {
    if let Some(skill) = inventory
        .skills
        .iter()
        .find(|entry| entry.skill_id == skill_id)
        .cloned()
    {
        return Ok(skill);
    }
    if let Some(error) = inventory.blocked_skill_errors.get(skill_id) {
        return Err(error.clone());
    }
    Err(format!("external skill `{skill_id}` is not available"))
}

fn load_discovered_skill_markdown(
    config: &super::runtime_config::ToolRuntimeConfig,
    skill: &DiscoveredSkillEntry,
) -> Result<String, String> {
    match skill.scope {
        DiscoveredSkillScope::Managed => {
            let install_root = resolve_install_root(config);
            let (_entry, instructions) =
                load_installed_skill_material(&install_root, skill.skill_id.as_str())?;
            Ok(instructions)
        }
        DiscoveredSkillScope::User | DiscoveredSkillScope::Project => {
            load_directory_skill_markdown(Path::new(&skill.source_path))
        }
    }
}

fn resolved_skill_root_path(skill: &DiscoveredSkillEntry) -> Option<PathBuf> {
    if let Some(install_path) = skill.install_path.as_deref()
        && !install_path.trim().is_empty()
    {
        return Some(PathBuf::from(install_path));
    }
    let source_path = skill.source_path.trim();
    (!source_path.is_empty()).then(|| PathBuf::from(source_path))
}

fn list_skill_resources(skill_root: &Path, limit: usize) -> Result<SkillResourceListing, String> {
    let mut files = Vec::new();
    collect_skill_resource_paths(skill_root, skill_root, &mut files)?;
    files.sort();

    let truncated = files.len() > limit;
    if truncated {
        files.truncate(limit);
    }

    Ok(SkillResourceListing { files, truncated })
}

fn collect_skill_resource_paths(
    root: &Path,
    current_path: &Path,
    files: &mut Vec<String>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(current_path).map_err(|error| {
        format!(
            "failed to inspect external skill resource path {}: {error}",
            current_path.display()
        )
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(());
    }
    if file_type.is_dir() {
        for entry in fs::read_dir(current_path).map_err(|error| {
            format!(
                "failed to read external skill resource directory {}: {error}",
                current_path.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to traverse external skill resource directory {}: {error}",
                    current_path.display()
                )
            })?;
            collect_skill_resource_paths(root, &entry.path(), files)?;
        }
        return Ok(());
    }
    if !file_type.is_file() {
        return Ok(());
    }

    let relative = current_path
        .strip_prefix(root)
        .unwrap_or(current_path)
        .display()
        .to_string();
    let relative = relative.replace('\\', "/");
    if relative == DEFAULT_SKILL_FILENAME {
        return Ok(());
    }
    files.push(relative);
    Ok(())
}

fn render_structured_skill_instructions(
    skill: &DiscoveredSkillEntry,
    raw_instructions: &str,
    skill_root: Option<&Path>,
    resource_listing: &SkillResourceListing,
) -> String {
    let body = extract_skill_body(raw_instructions);
    let mut sections = vec![format!(
        "<skill_content name=\"{}\" skill_id=\"{}\" scope=\"{}\" source_kind=\"{}\">",
        xml_escape(skill.display_name.as_str()),
        xml_escape(skill.skill_id.as_str()),
        discovered_skill_scope_id(skill.scope),
        xml_escape(skill.source_kind.as_str())
    )];

    let has_metadata = skill.license.is_some()
        || skill.compatibility.is_some()
        || !skill.metadata.is_empty()
        || !skill.allowed_tools.is_empty()
        || !skill.blocked_tools.is_empty();
    if has_metadata {
        sections.push("<skill_metadata>".to_owned());
        if let Some(license) = skill.license.as_deref() {
            sections.push(format!("<license>{}</license>", xml_escape(license)));
        }
        if let Some(compatibility) = skill.compatibility.as_deref() {
            sections.push(format!(
                "<compatibility>{}</compatibility>",
                xml_escape(compatibility)
            ));
        }
        for (key, value) in &skill.metadata {
            sections.push(format!(
                "<metadata key=\"{}\">{}</metadata>",
                xml_escape(key),
                xml_escape(value)
            ));
        }
        if !skill.allowed_tools.is_empty() {
            sections.push(format!(
                "<allowed_tools>{}</allowed_tools>",
                xml_escape(skill.allowed_tools.join(" ").as_str())
            ));
        }
        if !skill.blocked_tools.is_empty() {
            sections.push(format!(
                "<blocked_tools>{}</blocked_tools>",
                xml_escape(skill.blocked_tools.join(" ").as_str())
            ));
        }
        sections.push("</skill_metadata>".to_owned());
    }

    sections.push("<skill_instructions format=\"markdown\">".to_owned());
    sections.push(body);
    sections.push("</skill_instructions>".to_owned());

    if let Some(skill_root) = skill_root {
        sections.push(format!("Skill directory: {}", skill_root.display()));
        sections.push(
            "Relative paths referenced by this skill resolve against the skill directory."
                .to_owned(),
        );
    }

    sections.push(format!(
        "<skill_resources truncated=\"{}\">",
        resource_listing.truncated
    ));
    for file in &resource_listing.files {
        sections.push(format!("<file>{}</file>", xml_escape(file)));
    }
    sections.push("</skill_resources>".to_owned());
    sections.push("</skill_content>".to_owned());

    sections.join("\n")
}

fn extract_skill_body(skill_markdown: &str) -> String {
    let body = skill_content_lines(skill_markdown)
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return skill_markdown.trim().to_owned();
    }
    trimmed.to_owned()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn load_directory_skill_markdown(skill_root: &Path) -> Result<String, String> {
    let metadata = fs::metadata(skill_root).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            skill_root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "external skill source {} must be a directory",
            skill_root.display()
        ));
    }
    let skill_md_path = skill_root.join(DEFAULT_SKILL_FILENAME);
    if !skill_md_path.is_file() {
        return Err(format!(
            "external skill source {} is missing `{DEFAULT_SKILL_FILENAME}`",
            skill_root.display()
        ));
    }
    let skill_md_metadata = fs::metadata(&skill_md_path).map_err(|error| {
        format!(
            "failed to inspect external skill source {}: {error}",
            skill_md_path.display()
        )
    })?;
    if skill_md_metadata.len() > DEFAULT_MAX_DOWNLOAD_BYTES as u64 {
        return Err(format!(
            "external skill source {} exceeds the {} byte size limit",
            skill_md_path.display(),
            DEFAULT_MAX_DOWNLOAD_BYTES
        ));
    }
    fs::read_to_string(&skill_md_path).map_err(|error| {
        format!(
            "failed to read external skill source {}: {error}",
            skill_md_path.display()
        )
    })
}

fn find_discoverable_skill_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut visited = BTreeSet::new();
    visit_discoverable_skill_roots(root, &mut roots, &mut visited);
    roots.sort();
    roots.dedup();
    roots
}

fn visit_discoverable_skill_roots(
    root: &Path,
    roots: &mut Vec<PathBuf>,
    visited: &mut BTreeSet<String>,
) {
    let canonical = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let key = canonical.display().to_string();
    if !visited.insert(key) {
        return;
    }

    let Ok(metadata) = fs::metadata(&canonical) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }

    let skill_md_path = canonical.join(DEFAULT_SKILL_FILENAME);
    if skill_md_path.is_file() {
        roots.push(canonical);
        return;
    }

    let Ok(entries) = fs::read_dir(&canonical) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        visit_discoverable_skill_roots(&entry.path(), roots, visited);
    }
}

fn contains_regular_skill_markdown(root: &Path) -> Result<bool, String> {
    let skill_md_path = root.join(DEFAULT_SKILL_FILENAME);
    let metadata = match fs::symlink_metadata(&skill_md_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to inspect external skill source {}: {error}",
                skill_md_path.display()
            ));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(format!(
            "external skill source {} cannot use a symlinked `{DEFAULT_SKILL_FILENAME}`",
            root.display()
        ));
    }
    if !file_type.is_file() {
        return Err(format!(
            "external skill source {} must contain a regular `{DEFAULT_SKILL_FILENAME}` file",
            root.display()
        ));
    }
    Ok(true)
}

fn normalize_loaded_skill_entry(
    install_root: &Path,
    mut entry: InstalledSkillEntry,
) -> Result<InstalledSkillEntry, String> {
    let normalized_skill_id = normalize_skill_id(entry.skill_id.as_str())?;
    if normalized_skill_id != entry.skill_id {
        return Err(format!(
            "skills index contains non-normalized skill id `{}`",
            entry.skill_id
        ));
    }
    let install_path = managed_skill_install_path(install_root, entry.skill_id.as_str())?;
    let skill_md_path = install_path.join(DEFAULT_SKILL_FILENAME);
    entry.install_path = install_path.display().to_string();
    entry.skill_md_path = skill_md_path.display().to_string();
    Ok(entry)
}

fn load_installed_skill_material(
    install_root: &Path,
    skill_id: &str,
) -> Result<(InstalledSkillEntry, String), String> {
    let entry = installed_skill_by_id(&load_installed_skill_index(install_root)?, skill_id)?;
    let entry = rehydrate_installed_skill_entry(install_root, entry)?;
    let instructions = load_managed_skill_markdown(&entry)?;
    Ok((entry, instructions))
}

fn rehydrate_installed_skill_entry(
    install_root: &Path,
    mut entry: InstalledSkillEntry,
) -> Result<InstalledSkillEntry, String> {
    let install_path = managed_skill_install_path(install_root, entry.skill_id.as_str())?;
    let install_metadata = fs::symlink_metadata(&install_path).map_err(|error| {
        format!(
            "failed to inspect managed skill install {}: {error}",
            install_path.display()
        )
    })?;
    let install_file_type = install_metadata.file_type();
    if install_file_type.is_symlink() {
        return Err(format!(
            "managed skill install {} cannot be a symlink",
            install_path.display()
        ));
    }
    if !install_file_type.is_dir() {
        return Err(format!(
            "managed skill install {} must be a directory",
            install_path.display()
        ));
    }

    entry.install_path = install_path.display().to_string();
    entry.skill_md_path = install_path
        .join(DEFAULT_SKILL_FILENAME)
        .display()
        .to_string();

    let skill_markdown = load_managed_skill_markdown(&entry)?;
    entry.display_name =
        derive_skill_display_name(skill_markdown.as_str(), entry.skill_id.as_str());
    entry.summary = derive_skill_summary(skill_markdown.as_str());
    entry.sha256 = hex::encode(Sha256::digest(skill_markdown.as_bytes()));
    Ok(entry)
}

fn load_managed_skill_markdown(entry: &InstalledSkillEntry) -> Result<String, String> {
    let install_path = PathBuf::from(entry.install_path.as_str());
    if !contains_regular_skill_markdown(&install_path)? {
        return Err(format!(
            "managed skill install {} is missing `{DEFAULT_SKILL_FILENAME}`",
            install_path.display()
        ));
    }
    fs::read_to_string(&entry.skill_md_path).map_err(|error| {
        format!(
            "failed to read installed skill {}: {error}",
            entry.skill_md_path
        )
    })
}

fn managed_skill_install_path(install_root: &Path, skill_id: &str) -> Result<PathBuf, String> {
    let normalized_skill_id = normalize_skill_id(skill_id)?;
    if normalized_skill_id != skill_id {
        return Err(format!(
            "external skill id `{skill_id}` must be normalized before path resolution"
        ));
    }
    Ok(install_root.join(skill_id))
}

fn policy_payload(policy: &super::runtime_config::SkillsRuntimePolicy) -> Value {
    json!({
        "enabled": policy.enabled,
        "require_download_approval": policy.require_download_approval,
        "allowed_domains": policy.allowed_domains.iter().cloned().collect::<Vec<_>>(),
        "blocked_domains": policy.blocked_domains.iter().cloned().collect::<Vec<_>>(),
        "install_root": policy.install_root.as_ref().map(|path| path.display().to_string()),
        "auto_expose_installed": policy.auto_expose_installed,
    })
}
