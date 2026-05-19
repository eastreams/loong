fn format_cwd(runtime: &CliTurnRuntime) -> String {
    current_working_directory_display(runtime)
}

fn build_chat_startup_content(
    runtime: &CliTurnRuntime,
    _options: &CliChatOptions,
    _render_width: usize,
    i18n: &I18nService,
) -> (String, String, Vec<(String, Vec<String>)>, Vec<String>) {
    let version = startup_version_line();
    let mcp_count = runtime.effective_bootstrap_mcp_servers.len();
    let skills = detect_available_skills(runtime.effective_working_directory.as_deref());
    let skill_count = skills.len();

    let tutorial = i18n.text(SurfaceCopy::Tutorial).to_owned();
    let sections = vec![
        (
            i18n.text(SurfaceCopy::StartupSectionSkills).to_owned(),
            vec![skill_count.to_string()],
        ),
        (
            i18n.text(SurfaceCopy::StartupSectionMcp).to_owned(),
            vec![mcp_count.to_string()],
        ),
    ];

    let tips = vec![
        tutorial.clone(),
        i18n.text(SurfaceCopy::StartupTipCommands).to_owned(),
        i18n.text(SurfaceCopy::StartupTipSkills).to_owned(),
        i18n.text(SurfaceCopy::StartupTipQueue).to_owned(),
        i18n.text(SurfaceCopy::StartupTipHistory).to_owned(),
    ];

    (version, tutorial, sections, tips)
}

fn startup_version_line() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn detect_available_skills(root: Option<&Path>) -> Vec<SkillEntry> {
    let mut seen_dirs = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut skills = Vec::new();

    for source in skill_search_roots(root) {
        let normalized_dir = source
            .directory
            .canonicalize()
            .unwrap_or_else(|_| source.directory.clone());
        if !seen_dirs.insert(normalized_dir) {
            continue;
        }

        for skill_dir in skill_dirs_in(source.directory.as_path()) {
            let folder_name = skill_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "skill".to_owned());
            let skill = read_skill_metadata(
                folder_name,
                skill_dir.join("SKILL.md"),
                source.category_tag,
                source.search_label,
            );
            let name_key = skill.name.to_ascii_lowercase();
            if seen_names.insert(name_key) {
                skills.push(skill);
            }
        }
    }

    skills.sort_by(|left, right| {
        skill_source_priority(left.category_tag.as_str())
            .cmp(&skill_source_priority(right.category_tag.as_str()))
            .then_with(|| left.name.cmp(&right.name))
    });
    skills
}

struct SkillSearchRoot {
    directory: std::path::PathBuf,
    category_tag: &'static str,
    search_label: &'static str,
}

fn skill_search_roots(root: Option<&Path>) -> Vec<SkillSearchRoot> {
    let mut roots = Vec::new();
    let repo_skills_dir = root
        .map(|path| path.join("skills"))
        .unwrap_or_else(|| Path::new("skills").to_path_buf());
    roots.push(SkillSearchRoot {
        directory: repo_skills_dir,
        category_tag: "[Repo]",
        search_label: "repo",
    });

    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        roots.push(SkillSearchRoot {
            directory: std::path::PathBuf::from(codex_home).join("skills"),
            category_tag: "[Skill]",
            search_label: "global",
        });
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        roots.push(SkillSearchRoot {
            directory: home.join(".codex").join("skills"),
            category_tag: "[Skill]",
            search_label: "global",
        });
        roots.push(SkillSearchRoot {
            directory: home.join(".agents").join("skills"),
            category_tag: "[Skill]",
            search_label: "agent",
        });
    }

    roots
}

fn skill_dirs_in(skills_dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return Vec::new();
    };

    let mut skill_dirs = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        if path.join("SKILL.md").is_file() {
            skill_dirs.push(path);
            continue;
        }
        let Ok(children) = std::fs::read_dir(path) else {
            continue;
        };
        skill_dirs.extend(
            children
                .filter_map(|child| child.ok())
                .filter(|child| child.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
                .map(|child| child.path())
                .filter(|child| child.join("SKILL.md").is_file()),
        );
    }
    skill_dirs
}

fn skill_source_priority(category_tag: &str) -> u8 {
    match category_tag {
        "[Repo]" => 0,
        "[Skill]" => 1,
        _ => 2,
    }
}

fn read_skill_metadata(
    folder_name: String,
    skill_doc_path: std::path::PathBuf,
    category_tag: &'static str,
    search_label: &'static str,
) -> SkillEntry {
    let Ok(contents) = std::fs::read_to_string(skill_doc_path) else {
        return SkillEntry {
            name: folder_name.clone(),
            description: "available skill".to_owned(),
            search_terms: build_skill_search_terms(
                folder_name.as_str(),
                folder_name.as_str(),
                search_label,
            ),
            category_tag: category_tag.to_owned(),
            source_alias: None,
        };
    };

    let name = parse_skill_frontmatter_value(contents.as_str(), "name")
        .filter(|value| !value.is_empty())
        .unwrap_or(folder_name.clone());
    let description = parse_skill_frontmatter_value(contents.as_str(), "description")
        .filter(|value| !value.is_empty())
        .or_else(|| fallback_skill_description(contents.as_str()))
        .unwrap_or_else(|| "available skill".to_owned());
    let search_terms = build_skill_search_terms(folder_name.as_str(), name.as_str(), search_label);
    let source_alias = (folder_name != name).then_some(folder_name);

    SkillEntry {
        name,
        description,
        search_terms,
        category_tag: category_tag.to_owned(),
        source_alias,
    }
}

fn build_skill_search_terms(folder_name: &str, name: &str, source_label: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for value in [folder_name, name, source_label] {
        if !terms.iter().any(|term| term == value) {
            terms.push(value.to_owned());
        }
        for segment in value.split(|ch: char| ch == '-' || ch == '_' || ch.is_whitespace()) {
            let trimmed = segment.trim();
            if trimmed.len() >= 2 && !terms.iter().any(|term| term == trimmed) {
                terms.push(trimmed.to_owned());
            }
        }
    }
    terms
}

fn parse_skill_frontmatter_value(contents: &str, key: &str) -> Option<String> {
    let lines = contents.lines().collect::<Vec<_>>();
    let mut inside_frontmatter = false;
    let mut frontmatter_consumed = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            if !frontmatter_consumed {
                inside_frontmatter = !inside_frontmatter;
                if !inside_frontmatter {
                    frontmatter_consumed = true;
                }
            }
            continue;
        }

        if inside_frontmatter && let Some(value) = trimmed.strip_prefix(&format!("{key}:")) {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }

    None
}

fn fallback_skill_description(contents: &str) -> Option<String> {
    contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && *line != "---")
        .map(ToOwned::to_owned)
}

fn render_chat_surface_help_lines_with_width(width: usize) -> Vec<String> {
    let queue_restore_shortcut = queue_restore_shortcut_label();
    let mut slash_command_items = slash_command_specs()
        .iter()
        .map(|spec| TuiKeyValueSpec::Plain {
            key: spec.command.to_owned(),
            value: slash_command_help_value(spec),
        })
        .collect::<Vec<_>>();
    slash_command_items.push(TuiKeyValueSpec::Plain {
        key: "$skill-name <request>".to_owned(),
        value: "type an available skill invocation directly in the composer".to_owned(),
    });

    let message_spec = TuiMessageSpec {
        role: "help".to_owned(),
        caption: Some("chat surface".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("slash commands".to_owned()),
                items: slash_command_items,
            },
            TuiSectionSpec::Narrative {
                title: Some("surface controls".to_owned()),
                lines: vec![
                    "Use / or : from an empty composer to open the command palette.".to_owned(),
                    "Type $skill-name directly in the composer, then continue writing the rest of the request."
                        .to_owned(),
                    "When the inline $ suggestion popup is visible, Enter or Tab confirms the current skill."
                        .to_owned(),
                    "Use Ctrl+O to expand or collapse the latest compaction summary.".to_owned(),
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("keyboard".to_owned()),
                lines: vec![
                    "Enter sends the current draft. Shift+Enter inserts a new line."
                        .to_owned(),
                    format!(
                        "Tab moves between composer and transcript. While a turn is running, Tab queues the current draft and {queue_restore_shortcut} restores the latest queued message."
                    ),
                    "PgUp / PgDn and Home / End scroll the transcript; printable keys return to the composer immediately."
                        .to_owned(),
                ],
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("mouse".to_owned()),
                lines: vec![
                    "Mouse wheel scrolls the transcript where terminal alternate-scroll is supported."
                        .to_owned(),
                    "Native terminal drag-selection remains available by default.".to_owned(),
                ],
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("usage notes".to_owned()),
                lines: vec![
                    "Type any non-command text to send a normal assistant turn.".to_owned(),
                    "Available skill names can be invoked directly with $skill-name."
                        .to_owned(),
                    "Use Ctrl+C to leave chat.".to_owned(),
                ],
            },
        ],
        footer_lines: vec![
            "Send normal text to continue the transcript.".to_owned(),
            "Use /usage, /review, or /compact when you need to inspect or stabilize the current session."
                .to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    published_at: Option<String>,
    html_url: Option<String>,
    body: Option<String>,
}

async fn load_startup_release_lines(width: usize) -> Option<Vec<String>> {
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let client = reqwest::Client::builder()
        .user_agent("loongclaw-chat-surface")
        .build()
        .ok()?;
    let response = tokio::time::timeout(
        Duration::from_millis(1500),
        client
            .get("https://api.github.com/repos/eastreams/loong/releases/latest")
            .send(),
    )
    .await
    .ok()?
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let release: GithubRelease = response.json().await.ok()?;
    format_startup_release_lines(&release, &current, width)
}

fn format_startup_release_lines(
    release: &GithubRelease,
    current: &str,
    width: usize,
) -> Option<Vec<String>> {
    if normalize_tag(&release.tag_name) == normalize_tag(current) {
        return None;
    }

    let rule = "─".repeat(width.max(12));
    let mut lines = vec![
        rule.clone(),
        " What's New".to_owned(),
        String::new(),
        format!(
            " [{}]{}",
            release.tag_name,
            release
                .published_at
                .as_deref()
                .and_then(|value| value.get(..10))
                .map(|date| format!(" - {date}"))
                .unwrap_or_default()
        ),
        String::new(),
    ];

    let mut added = 0usize;
    for line in release.body.as_deref().unwrap_or_default().lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        lines.push(trimmed.to_owned());
        added += 1;
        if added >= 28 {
            break;
        }
    }

    if let Some(url) = release.html_url.as_deref() {
        lines.push(String::new());
        lines.push(format!(" Release: {url}"));
    }
    lines.push(rule);
    Some(lines)
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('v').to_ascii_lowercase()
}

fn resize_reflow_required(
    previous_width: u16,
    previous_height: u16,
    next_width: u16,
    next_height: u16,
) -> bool {
    previous_width != next_width || previous_height != next_height
}

fn resize_live_rerender_ready(
    pending_live_resize_rerender: bool,
    since_last_resize: Option<Duration>,
) -> bool {
    pending_live_resize_rerender
        && since_last_resize
            .map(|elapsed| elapsed >= Duration::from_millis(70))
            .unwrap_or(true)
}

