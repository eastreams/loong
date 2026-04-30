use crate::chat::chat_surface::i18n::{I18nService, Language, SurfaceCopy};
use crate::chat::chat_surface::scroll_state::ScrollState;
use crate::chat::chat_surface::utils::*;
use crate::config::{ProviderKind, ReasoningEffort};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, List, ListItem, ListState, Paragraph},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    RunCommand(&'static str),
    OpenSettings(SettingsSurfaceFocus),
    ApplySettings(SettingsCommandAction),
    OpenModelReasoning(String),
    ApplyModelSelection {
        model: String,
        reasoning_effort: Option<ReasoningEffort>,
    },
    InsertText(String),
    Noop,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSurfaceFocus {
    Overview,
    Provider,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsCommandAction {
    SetProvider(ProviderKind),
    SetWebProvider(String),
    InstallSkillPack(String),
    RemoveSkillPack(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSurfaceFocus {
    Models,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub command: &'static str,
    pub description: &'static str,
    pub ready: bool,
}

const SLASH_COMMAND_SPECS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        command: "/model",
        description: "inspect or switch the active model",
        ready: true,
    },
    SlashCommandSpec {
        command: "/permissions",
        description: "review tool and command permissions",
        ready: true,
    },
    SlashCommandSpec {
        command: "/settings",
        description: "review current setup and follow-up adjustments",
        ready: true,
    },
    SlashCommandSpec {
        command: "/experimental",
        description: "inspect active surface features",
        ready: true,
    },
    SlashCommandSpec {
        command: "/skills",
        description: "browse available skills",
        ready: true,
    },
    SlashCommandSpec {
        command: "/mcp",
        description: "inspect configured MCP servers",
        ready: true,
    },
    SlashCommandSpec {
        command: "/rename",
        description: "rename the current conversation",
        ready: true,
    },
    SlashCommandSpec {
        command: "/review",
        description: "inspect approval and review queue",
        ready: true,
    },
    SlashCommandSpec {
        command: "/new",
        description: "start a fresh conversation",
        ready: true,
    },
    SlashCommandSpec {
        command: "/resume",
        description: "resume a previous conversation",
        ready: true,
    },
    SlashCommandSpec {
        command: "/fork",
        description: "branch the current conversation",
        ready: true,
    },
    SlashCommandSpec {
        command: "/compact",
        description: "checkpoint conversation context",
        ready: true,
    },
    SlashCommandSpec {
        command: "/plan",
        description: "draft or inspect a plan",
        ready: true,
    },
    SlashCommandSpec {
        command: "/copy",
        description: "copy the latest answer or explicit text",
        ready: true,
    },
    SlashCommandSpec {
        command: "/diff",
        description: "show recent code changes",
        ready: true,
    },
    SlashCommandSpec {
        command: "/title",
        description: "set the visible chat title",
        ready: true,
    },
    SlashCommandSpec {
        command: "/feedback",
        description: "send product feedback",
        ready: true,
    },
    SlashCommandSpec {
        command: "/clear",
        description: "clear the visible transcript",
        ready: true,
    },
    SlashCommandSpec {
        command: "/cwd",
        description: "show or change working directory",
        ready: true,
    },
    SlashCommandSpec {
        command: "/language",
        description: "choose UI language",
        ready: true,
    },
    SlashCommandSpec {
        command: "/share",
        description: "write a local transcript artifact",
        ready: true,
    },
    SlashCommandSpec {
        command: "/export",
        description: "export the current transcript",
        ready: true,
    },
    SlashCommandSpec {
        command: "/import",
        description: "import a transcript or context bundle",
        ready: true,
    },
    SlashCommandSpec {
        command: "/themes",
        description: "inspect terminal theme surface",
        ready: true,
    },
    SlashCommandSpec {
        command: "/simplify",
        description: "simplify the latest answer or diff",
        ready: true,
    },
    SlashCommandSpec {
        command: "/usage",
        description: "show available slash commands",
        ready: true,
    },
    SlashCommandSpec {
        command: "/missions",
        description: "open mission-control lane status",
        ready: true,
    },
    SlashCommandSpec {
        command: "/subagents",
        description: "inspect delegated subagent lanes",
        ready: true,
    },
];

pub fn slash_command_specs() -> &'static [SlashCommandSpec] {
    SLASH_COMMAND_SPECS
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub search_terms: Vec<String>,
    pub category_tag: String,
    pub source_alias: Option<String>,
}

#[derive(Debug, Clone)]
struct CommandEntry {
    command: &'static str,
    description: String,
    action: CommandAction,
}

#[derive(Debug, Clone)]
pub struct SettingsEntry {
    pub label: String,
    pub category_tag: String,
    pub status_tag: Option<String>,
    pub description: String,
    pub action: CommandAction,
    pub selectable: bool,
}

pub struct CommandPalette {
    query: String,
    commands: Vec<CommandEntry>,
    settings: Vec<SettingsEntry>,
    settings_status: Option<String>,
    settings_focus: SettingsSurfaceFocus,
    model_entries: Vec<SettingsEntry>,
    model_status: Option<String>,
    model_focus: ModelSurfaceFocus,
    reasoning_entries: Vec<SettingsEntry>,
    reasoning_status: Option<String>,
    reasoning_model_label: Option<String>,
    skills: Vec<SkillEntry>,
    mode: PaletteMode,
    scroll_state: ScrollState,
    i18n: I18nService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteMode {
    Commands,
    Settings,
    Models,
    Reasoning,
    Skills,
}

impl CommandPalette {
    const VISIBLE_ROWS: usize = 7;
    const FOOTER_ROWS: usize = 1;
    const SKILL_HINT_ROWS: usize = 2;
    const SKILL_LABEL_TRUNCATE_LEN: usize = 24;

    pub fn new(lang: Language, skills: Vec<SkillEntry>) -> Self {
        Self {
            query: String::new(),
            commands: SLASH_COMMAND_SPECS
                .iter()
                .map(|spec| CommandEntry {
                    command: spec.command,
                    description: spec.description.to_owned(),
                    action: CommandAction::RunCommand(spec.command),
                })
                .collect(),
            settings: Vec::new(),
            settings_status: None,
            settings_focus: SettingsSurfaceFocus::Overview,
            model_entries: Vec::new(),
            model_status: None,
            model_focus: ModelSurfaceFocus::Models,
            reasoning_entries: Vec::new(),
            reasoning_status: None,
            reasoning_model_label: None,
            skills,
            mode: PaletteMode::Commands,
            scroll_state: ScrollState::new(),
            i18n: I18nService::new(lang),
        }
    }

    pub fn show_commands(&mut self, query: &str) {
        self.mode = PaletteMode::Commands;
        self.query = query.trim().trim_start_matches(['/', ':']).to_string();
        self.settings_status = None;
        self.scroll_state.reset();
    }

    pub fn show_settings(
        &mut self,
        focus: SettingsSurfaceFocus,
        entries: Vec<SettingsEntry>,
        status: Option<String>,
        selected_label: Option<&str>,
    ) {
        self.mode = PaletteMode::Settings;
        self.query.clear();
        self.settings_focus = focus;
        self.settings = entries;
        self.settings_status = status;
        let selected_index = selected_label
            .and_then(|label| {
                self.settings
                    .iter()
                    .position(|entry| entry.selectable && entry.label == label)
            })
            .or_else(|| self.first_selectable_index())
            .unwrap_or(0);
        self.scroll_state.selected_idx = Some(selected_index);
        self.scroll_state.scroll_top = 0;
    }

    pub fn show_model_selector(
        &mut self,
        entries: Vec<SettingsEntry>,
        status: Option<String>,
        selected_label: Option<&str>,
        query: &str,
    ) {
        self.mode = PaletteMode::Models;
        self.query = query.trim().to_owned();
        self.model_focus = ModelSurfaceFocus::Models;
        self.model_entries = entries;
        self.model_status = status;
        self.reasoning_entries.clear();
        self.reasoning_status = None;
        self.reasoning_model_label = None;
        self.scroll_state.selected_idx = Some(selection_index_for_entries(
            self.model_entries.as_slice(),
            selected_label,
        ));
        self.scroll_state.scroll_top = 0;
    }

    pub fn show_reasoning_selector(
        &mut self,
        model_label: &str,
        entries: Vec<SettingsEntry>,
        status: Option<String>,
        selected_label: Option<&str>,
    ) {
        self.mode = PaletteMode::Reasoning;
        self.query.clear();
        self.model_focus = ModelSurfaceFocus::Reasoning;
        self.reasoning_entries = entries;
        self.reasoning_status = status;
        self.reasoning_model_label = Some(model_label.to_owned());
        self.scroll_state.selected_idx = Some(selection_index_for_entries(
            self.reasoning_entries.as_slice(),
            selected_label,
        ));
        self.scroll_state.scroll_top = 0;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn show_skills(&mut self, query: &str) {
        self.mode = PaletteMode::Skills;
        self.query = query.trim().trim_start_matches('$').to_string();
        self.settings_status = None;
        self.scroll_state.reset();
    }

    pub fn has_skills(&self) -> bool {
        !self.skills.is_empty()
    }

    pub fn is_commands_mode(&self) -> bool {
        self.mode == PaletteMode::Commands
    }

    pub fn query_text(&self) -> &str {
        self.query.as_str()
    }

    pub fn desired_height(&self) -> usize {
        let footer_rows = match self.mode {
            PaletteMode::Commands
            | PaletteMode::Settings
            | PaletteMode::Models
            | PaletteMode::Reasoning => Self::FOOTER_ROWS,
            PaletteMode::Skills => Self::SKILL_HINT_ROWS,
        };
        Self::visible_rows_for_total(self.filtered_item_count()) + footer_rows
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let filtered = self.filtered_items();
        let visible_rows = Self::visible_rows_for_total(filtered.len());
        if self.mode == PaletteMode::Skills {
            self.render_skills_mode(f, area, filtered, visible_rows);
            return;
        }
        if self.mode == PaletteMode::Settings {
            self.render_settings_mode(f, area, filtered, visible_rows);
            return;
        }
        if matches!(self.mode, PaletteMode::Models | PaletteMode::Reasoning) {
            self.render_model_mode(f, area, filtered, visible_rows);
            return;
        }
        if filtered.is_empty() {
            let mut items = vec![ListItem::new(Line::from(vec![Span::styled(
                format!("  {}", self.i18n.text(SurfaceCopy::CommandDeckEmpty)),
                Style::default().fg(SURFACE_DIM_GRAY),
            )]))];
            while items.len() < visible_rows {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "(0/0)",
                Style::default().fg(SURFACE_DIM_GRAY),
            )])));
            let list = List::new(items).highlight_style(Style::default());
            let mut visible_state = ListState::default();
            f.render_stateful_widget(list, area, &mut visible_state);
            return;
        }

        let selected = self.selected_index_for(&filtered);
        let start = self
            .scroll_state
            .scroll_top
            .min(filtered.len().saturating_sub(1));
        let end = (start + visible_rows).min(filtered.len());
        let visible = filtered.get(start..end).unwrap_or(&[]);

        let label_width = filtered
            .iter()
            .map(|entry| crate::presentation::display_width(entry.label.as_str()))
            .max()
            .unwrap_or(0)
            .clamp(8, 18);

        let mut items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(visible_index, entry)| {
                let index = start + visible_index;
                let label = entry.label.clone();
                let is_selected = index == selected && entry.selectable;
                let prefix = if is_selected { "→ " } else { "  " };
                let status_tag = entry.status_tag.clone().unwrap_or_default();
                let status_width = if status_tag.is_empty() {
                    0
                } else {
                    crate::presentation::display_width(status_tag.as_str()) + 1
                };
                let gap = " ".repeat(
                    label_width.saturating_sub(crate::presentation::display_width(&label)) + 2,
                );
                let max_desc = area.width.saturating_sub(
                    (crate::presentation::display_width(prefix) + label_width + 2 + status_width)
                        as u16,
                ) as usize;
                let desc = truncate(entry.description.as_str(), max_desc);

                let mut spans = vec![
                    Span::styled(
                        prefix,
                        Style::default().fg(if is_selected {
                            SURFACE_CYAN
                        } else {
                            SURFACE_DIM_GRAY
                        }),
                    ),
                    Span::styled(
                        label,
                        Style::default()
                            .fg(if is_selected {
                                SURFACE_CYAN
                            } else {
                                ratatui::style::Color::White
                            })
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ];
                if !status_tag.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        status_tag,
                        Style::default().fg(if is_selected {
                            SURFACE_ACCENT
                        } else {
                            SURFACE_COTTON_CANDY
                        }),
                    ));
                }
                spans.push(Span::raw(gap));
                spans.push(Span::styled(
                    desc,
                    Style::default().fg(if is_selected {
                        SURFACE_ACCENT
                    } else {
                        SURFACE_GRAY
                    }),
                ));
                ListItem::new(Line::from(spans))
            })
            .collect();

        while items.len() < visible_rows {
            items.push(ListItem::new(Line::from("")));
        }

        let footer_line = match self.mode {
            PaletteMode::Commands => format!("({}/{})", selected + 1, filtered.len().max(1)),
            PaletteMode::Settings => self
                .settings_status
                .as_deref()
                .map(|status| truncate(status, area.width.saturating_sub(2) as usize))
                .unwrap_or_else(|| "Enter apply · Esc close · type to filter".to_owned()),
            PaletteMode::Models | PaletteMode::Reasoning => String::new(),
            PaletteMode::Skills => String::new(),
        };
        let count_line = ListItem::new(Line::from(vec![Span::styled(
            footer_line,
            Style::default().fg(SURFACE_DIM_GRAY),
        )]));

        items.push(count_line);
        let list = List::new(items).highlight_style(Style::default());
        let mut visible_state = ListState::default();
        visible_state.select(Some(selected.saturating_sub(start)));
        f.render_stateful_widget(list, area, &mut visible_state);
    }

    fn render_skills_mode(
        &mut self,
        f: &mut Frame,
        area: Rect,
        filtered: Vec<PaletteItem>,
        visible_rows: usize,
    ) {
        let list_height = area.height.saturating_sub(2).max(1);
        let list_area = Rect {
            x: area.x.saturating_add(2),
            y: area.y,
            width: area.width.saturating_sub(2),
            height: list_height,
        };
        let hint_area = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width.saturating_sub(2),
            height: 1,
        };

        if filtered.is_empty() {
            let mut items = vec![ListItem::new(Line::from(vec![Span::styled(
                "no matches",
                Style::default().fg(SURFACE_DIM_GRAY),
            )]))];
            while items.len() < visible_rows {
                items.push(ListItem::new(Line::from("")));
            }
            let list = List::new(items).highlight_style(Style::default());
            let mut visible_state = ListState::default();
            f.render_stateful_widget(list, list_area, &mut visible_state);
            f.render_widget(Paragraph::new(skill_popup_hint_line()), hint_area);
            return;
        }

        let selected = self.selected_index_for(&filtered);
        let start = self
            .scroll_state
            .scroll_top
            .min(filtered.len().saturating_sub(1));
        let end = (start + visible_rows).min(filtered.len());
        let visible = filtered.get(start..end).unwrap_or(&[]);

        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(visible_index, entry)| {
                let index = start + visible_index;
                let is_selected = index == selected && entry.selectable;
                let (category_tag, raw_description) =
                    split_category_tag(entry.description.as_str());
                let category_width = crate::presentation::display_width(category_tag);
                let (description_context, description_detail) =
                    split_description_context(raw_description);
                let source_has_detail = !description_detail.is_empty();
                let label_limit = entry
                    .source_skill
                    .as_ref()
                    .map(skill_label_max_width)
                    .unwrap_or(Self::SKILL_LABEL_TRUNCATE_LEN);
                let available_after_prefix = list_area.width.saturating_sub(2) as usize;
                let label_separator_width = 2usize;
                let desc_leading_space_width = usize::from(!raw_description.is_empty());
                let desc_context_separator_width =
                    if !description_context.is_empty() && source_has_detail {
                        3
                    } else {
                        0
                    };
                let estimated_context_width =
                    crate::presentation::display_width(description_context).min(
                        entry
                            .source_skill
                            .as_ref()
                            .map(skill_context_max_width)
                            .unwrap_or(18),
                    );
                let reserved_for_category = label_separator_width
                    + category_width
                    + desc_leading_space_width
                    + estimated_context_width
                    + desc_context_separator_width;
                let label_budget = available_after_prefix
                    .saturating_sub(reserved_for_category)
                    .max(1);
                let label = truncate(entry.label.as_str(), label_limit.min(label_budget));
                let label_width = crate::presentation::display_width(label.as_str());
                let desc_available_width = available_after_prefix.saturating_sub(
                    label_width + label_separator_width + category_width + desc_leading_space_width,
                );
                let min_detail_width = if source_has_detail && !description_context.is_empty() {
                    2
                } else if source_has_detail {
                    6
                } else {
                    0
                };
                let max_context_by_remaining = desc_available_width
                    .saturating_sub(desc_context_separator_width + min_detail_width)
                    .max(1);
                let max_context_by_balance = if source_has_detail && description_context.len() > 12
                {
                    (desc_available_width / 2).max(8)
                } else {
                    max_context_by_remaining
                };
                let context_limit = entry
                    .source_skill
                    .as_ref()
                    .map(skill_context_max_width)
                    .unwrap_or(18)
                    .min(max_context_by_remaining.min(max_context_by_balance).max(1));
                let truncated_context = truncate(description_context, context_limit);
                let desc_context_width =
                    crate::presentation::display_width(truncated_context.as_str());
                let max_desc = desc_available_width
                    .saturating_sub(desc_context_width + desc_context_separator_width);
                let truncated_detail = truncate(description_detail, max_desc);
                let description = if description_context.is_empty() {
                    truncated_detail
                } else if !source_has_detail || truncated_detail.is_empty() {
                    truncated_context.clone()
                } else {
                    format!("{truncated_context} · {truncated_detail}")
                };
                let label_spans = render_match_highlight_spans(
                    label.as_str(),
                    entry
                        .match_target
                        .clone()
                        .filter(|target| target.is_label()),
                    skill_label_style(is_selected),
                    skill_label_highlight_style(is_selected),
                );
                let description_spans = render_skill_description_spans(
                    truncated_context.as_str(),
                    description.as_str(),
                    entry.match_target.clone(),
                    is_selected,
                );
                let mut spans = vec![Span::styled(
                    if is_selected { "› " } else { "  " },
                    Style::default().fg(if is_selected {
                        SURFACE_CYAN
                    } else {
                        SURFACE_DIM_GRAY
                    }),
                )];
                spans.extend(label_spans);
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    category_tag.to_owned(),
                    skill_category_style(is_selected),
                ));
                if !description.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.extend(description_spans);
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        let mut padded_items = items;
        while padded_items.len() < visible_rows {
            padded_items.push(ListItem::new(Line::from("")));
        }

        let list = List::new(padded_items).highlight_style(Style::default());
        let mut visible_state = ListState::default();
        visible_state.select(Some(selected.saturating_sub(start)));
        f.render_stateful_widget(list, list_area, &mut visible_state);
        f.render_widget(Paragraph::new(skill_popup_hint_line()), hint_area);
    }

    fn render_settings_mode(
        &mut self,
        f: &mut Frame,
        area: Rect,
        filtered: Vec<PaletteItem>,
        visible_rows: usize,
    ) {
        let header_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let list_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(2).max(1),
        };
        let footer_area = Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width,
            height: 1,
        };

        let title = match self.settings_focus {
            SettingsSurfaceFocus::Overview => "settings · overview",
            SettingsSurfaceFocus::Provider => "settings · provider & web",
            SettingsSurfaceFocus::Workspace => "settings · workspace setup",
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                truncate(title, list_area.width as usize),
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            )])),
            header_area,
        );

        if filtered.is_empty() {
            let items = vec![ListItem::new(Line::from(vec![Span::styled(
                "  no settings available",
                Style::default().fg(SURFACE_DIM_GRAY),
            )]))];
            let list = List::new(items).highlight_style(Style::default());
            let mut visible_state = ListState::default();
            f.render_stateful_widget(list, list_area, &mut visible_state);
            f.render_widget(Paragraph::new(self.settings_footer_line()), footer_area);
            return;
        }

        let selected = self.selected_index_for(&filtered);
        let start = self
            .scroll_state
            .scroll_top
            .min(filtered.len().saturating_sub(1));
        let end = (start + visible_rows.min(list_area.height as usize)).min(filtered.len());
        let visible = filtered.get(start..end).unwrap_or(&[]);

        let label_width = filtered
            .iter()
            .map(|entry| crate::presentation::display_width(entry.label.as_str()))
            .max()
            .unwrap_or(0)
            .clamp(10, 20);
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(visible_index, entry)| {
                let index = start + visible_index;
                let is_selected = index == selected;
                let prefix = if is_selected { "→ " } else { "  " };
                let status_tag = entry.status_tag.clone().unwrap_or_default();
                let status_width = if status_tag.is_empty() {
                    0
                } else {
                    crate::presentation::display_width(status_tag.as_str()) + 1
                };
                let gap = " ".repeat(
                    label_width.saturating_sub(crate::presentation::display_width(&entry.label))
                        + 2,
                );
                let max_desc = list_area.width.saturating_sub(
                    (crate::presentation::display_width(prefix) + label_width + 2 + status_width)
                        as u16,
                ) as usize;
                let desc = truncate(entry.description.as_str(), max_desc);
                let mut spans = vec![
                    Span::styled(
                        prefix,
                        Style::default().fg(if is_selected {
                            SURFACE_CYAN
                        } else {
                            SURFACE_DIM_GRAY
                        }),
                    ),
                    Span::styled(
                        entry.label.clone(),
                        Style::default()
                            .fg(if is_selected {
                                SURFACE_CYAN
                            } else {
                                ratatui::style::Color::White
                            })
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ];
                if !status_tag.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        status_tag,
                        Style::default().fg(if is_selected {
                            SURFACE_ACCENT
                        } else {
                            SURFACE_COTTON_CANDY
                        }),
                    ));
                }
                spans.push(Span::raw(gap));
                spans.push(Span::styled(desc, Style::default().fg(SURFACE_GRAY)));
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).highlight_style(Style::default());
        let mut visible_state = ListState::default();
        visible_state.select(Some(selected.saturating_sub(start)));
        f.render_stateful_widget(list, list_area, &mut visible_state);
        f.render_widget(Paragraph::new(self.settings_footer_line()), footer_area);
    }

    fn render_model_mode(
        &mut self,
        f: &mut Frame,
        area: Rect,
        filtered: Vec<PaletteItem>,
        visible_rows: usize,
    ) {
        let header_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let list_area = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(2).max(1),
        };
        let footer_area = Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(1)),
            width: area.width,
            height: 1,
        };

        let title = match self.mode {
            PaletteMode::Models => "model · select".to_owned(),
            PaletteMode::Reasoning => self
                .reasoning_model_label
                .as_deref()
                .map(|label| format!("model · reasoning · {label}"))
                .unwrap_or_else(|| "model · reasoning".to_owned()),
            PaletteMode::Commands | PaletteMode::Settings | PaletteMode::Skills => String::new(),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                truncate(title.as_str(), list_area.width as usize),
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            )])),
            header_area,
        );

        if filtered.is_empty() {
            let empty_text = match self.mode {
                PaletteMode::Models => "  no models available",
                PaletteMode::Reasoning => "  no reasoning options available",
                PaletteMode::Commands | PaletteMode::Settings | PaletteMode::Skills => {
                    "  no results"
                }
            };
            let items = vec![ListItem::new(Line::from(vec![Span::styled(
                empty_text,
                Style::default().fg(SURFACE_DIM_GRAY),
            )]))];
            let list = List::new(items).highlight_style(Style::default());
            let mut visible_state = ListState::default();
            f.render_stateful_widget(list, list_area, &mut visible_state);
            f.render_widget(Paragraph::new(self.model_footer_line()), footer_area);
            return;
        }

        let selected = self.selected_index_for(&filtered);
        let start = self
            .scroll_state
            .scroll_top
            .min(filtered.len().saturating_sub(1));
        let end = (start + visible_rows.min(list_area.height as usize)).min(filtered.len());
        let visible = filtered.get(start..end).unwrap_or(&[]);

        let label_width = filtered
            .iter()
            .map(|entry| crate::presentation::display_width(entry.label.as_str()))
            .max()
            .unwrap_or(0)
            .clamp(10, 24);
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(visible_index, entry)| {
                let index = start + visible_index;
                let is_selected = index == selected;
                let prefix = if is_selected { "→ " } else { "  " };
                let status_tag = entry.status_tag.clone().unwrap_or_default();
                let status_width = if status_tag.is_empty() {
                    0
                } else {
                    crate::presentation::display_width(status_tag.as_str()) + 1
                };
                let gap = " ".repeat(
                    label_width.saturating_sub(crate::presentation::display_width(&entry.label))
                        + 2,
                );
                let max_desc = list_area.width.saturating_sub(
                    (crate::presentation::display_width(prefix) + label_width + 2 + status_width)
                        as u16,
                ) as usize;
                let desc = truncate(entry.description.as_str(), max_desc);
                let mut spans = vec![
                    Span::styled(
                        prefix,
                        Style::default().fg(if is_selected {
                            SURFACE_CYAN
                        } else {
                            SURFACE_DIM_GRAY
                        }),
                    ),
                    Span::styled(
                        entry.label.clone(),
                        Style::default()
                            .fg(if is_selected {
                                SURFACE_CYAN
                            } else {
                                ratatui::style::Color::White
                            })
                            .add_modifier(if is_selected {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
                ];
                if !status_tag.is_empty() {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        status_tag,
                        Style::default().fg(if is_selected {
                            SURFACE_ACCENT
                        } else {
                            SURFACE_COTTON_CANDY
                        }),
                    ));
                }
                spans.push(Span::raw(gap));
                spans.push(Span::styled(desc, Style::default().fg(SURFACE_GRAY)));
                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).highlight_style(Style::default());
        let mut visible_state = ListState::default();
        visible_state.select(Some(selected.saturating_sub(start)));
        f.render_stateful_widget(list, list_area, &mut visible_state);
        f.render_widget(Paragraph::new(self.model_footer_line()), footer_area);
    }

    fn settings_footer_line(&self) -> Line<'static> {
        let text = self
            .settings_status
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| match self.settings_focus {
                SettingsSurfaceFocus::Overview => {
                    "Enter open · Esc close · type to filter".to_owned()
                }
                SettingsSurfaceFocus::Provider | SettingsSurfaceFocus::Workspace => {
                    "Enter apply/open · Esc back · type to filter".to_owned()
                }
            });
        Line::from(vec![Span::styled(
            text,
            Style::default().fg(SURFACE_DIM_GRAY),
        )])
    }

    fn model_footer_line(&self) -> Line<'static> {
        let text = match self.mode {
            PaletteMode::Models => self
                .model_status
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| "Enter choose model · Esc close · type to filter".to_owned()),
            PaletteMode::Reasoning => self
                .reasoning_status
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| "Enter apply · Esc back · type to filter".to_owned()),
            PaletteMode::Commands | PaletteMode::Settings | PaletteMode::Skills => String::new(),
        };
        Line::from(vec![Span::styled(
            text,
            Style::default().fg(SURFACE_DIM_GRAY),
        )])
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<CommandAction> {
        match key.code {
            KeyCode::Esc => {
                if self.mode == PaletteMode::Settings
                    && self.settings_focus != SettingsSurfaceFocus::Overview
                {
                    Some(CommandAction::OpenSettings(SettingsSurfaceFocus::Overview))
                } else if self.mode == PaletteMode::Reasoning {
                    self.mode = PaletteMode::Models;
                    self.model_focus = ModelSurfaceFocus::Models;
                    self.query.clear();
                    let selected_label = self.reasoning_model_label.clone();
                    self.scroll_state.selected_idx = Some(selection_index_for_entries(
                        self.model_entries.as_slice(),
                        selected_label.as_deref(),
                    ));
                    self.scroll_state.scroll_top = 0;
                    None
                } else {
                    Some(CommandAction::Close)
                }
            }
            KeyCode::Enter => self.selected_action(),
            KeyCode::Up => {
                self.step_selection(-1);
                None
            }
            KeyCode::Down => {
                self.step_selection(1);
                None
            }
            KeyCode::PageUp => {
                let total = self.filtered_item_count();
                if total == 0 {
                    return None;
                }
                let page = Self::visible_rows_for_total(total).max(1);
                self.jump_selection_by_page(-(page as isize));
                None
            }
            KeyCode::PageDown => {
                let total = self.filtered_item_count();
                if total == 0 {
                    return None;
                }
                let page = Self::visible_rows_for_total(total).max(1);
                self.jump_selection_by_page(page as isize);
                None
            }
            KeyCode::Home => {
                let total = self.filtered_item_count();
                if total == 0 {
                    return None;
                }
                let filtered = self.filtered_items();
                let index = selectable_indices(&filtered)
                    .into_iter()
                    .next()
                    .unwrap_or(0);
                self.scroll_state.selected_idx = Some(index);
                self.scroll_state
                    .ensure_visible(total, Self::visible_rows_for_total(total));
                None
            }
            KeyCode::End => {
                let total = self.filtered_item_count();
                if total == 0 {
                    return None;
                }
                let filtered = self.filtered_items();
                let index = selectable_indices(&filtered)
                    .into_iter()
                    .last()
                    .unwrap_or(total - 1);
                self.scroll_state.selected_idx = Some(index);
                self.scroll_state
                    .ensure_visible(total, Self::visible_rows_for_total(total));
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.scroll_state.reset();
                None
            }
            KeyCode::Char(':') if self.mode == PaletteMode::Commands && self.query.is_empty() => {
                None
            }
            KeyCode::Char('$') if self.mode == PaletteMode::Skills && self.query.is_empty() => None,
            KeyCode::Char(c) => {
                self.query.push(c);
                self.scroll_state.reset();
                None
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Delete
            | KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Null
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => None,
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Option<CommandAction> {
        if !area_contains(area, mouse.column, mouse.row) {
            return None;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.step_selection(-1);
                None
            }
            MouseEventKind::ScrollDown => {
                self.step_selection(1);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let filtered = self.filtered_items();
                if filtered.is_empty() {
                    return None;
                }
                let visible_rows = match self.mode {
                    PaletteMode::Settings | PaletteMode::Models | PaletteMode::Reasoning => {
                        Self::visible_rows_for_total(filtered.len())
                            .min(area.height.saturating_sub(2) as usize)
                    }
                    PaletteMode::Commands | PaletteMode::Skills => {
                        Self::visible_rows_for_total(filtered.len())
                    }
                };
                let base_y = if matches!(
                    self.mode,
                    PaletteMode::Settings | PaletteMode::Models | PaletteMode::Reasoning
                ) {
                    area.y.saturating_add(1)
                } else {
                    area.y
                };
                let row = mouse.row.saturating_sub(base_y) as usize;
                if row >= visible_rows {
                    return None;
                }
                let index = self.scroll_state.scroll_top.saturating_add(row);
                if index >= filtered.len() {
                    return None;
                }
                if filtered.get(index).is_none_or(|entry| !entry.selectable) {
                    return None;
                }
                self.scroll_state.selected_idx = Some(index);
                self.scroll_state
                    .ensure_visible(filtered.len(), Self::visible_rows_for_total(filtered.len()));
                self.selected_action()
            }
            MouseEventKind::Down(_)
            | MouseEventKind::Up(_)
            | MouseEventKind::Drag(_)
            | MouseEventKind::Moved
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => None,
        }
    }

    fn filtered_item_count(&self) -> usize {
        self.filtered_items().len()
    }

    fn selected_action(&self) -> Option<CommandAction> {
        let filtered = self.filtered_items();
        let index = self
            .scroll_state
            .selected_idx
            .unwrap_or(0)
            .min(filtered.len().saturating_sub(1));
        filtered
            .get(index)
            .filter(|entry| entry.selectable)
            .map(|entry| entry.action.clone())
    }

    fn filtered_items(&self) -> Vec<PaletteItem> {
        match self.mode {
            PaletteMode::Commands => self.filtered_commands(),
            PaletteMode::Settings => self.filtered_settings(),
            PaletteMode::Models => self.filtered_models(),
            PaletteMode::Reasoning => self.filtered_reasoning(),
            PaletteMode::Skills => self.filtered_skills(),
        }
    }

    fn filtered_commands(&self) -> Vec<PaletteItem> {
        let query = self.query.trim().to_ascii_lowercase();
        self.commands
            .iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                let command = entry.command.to_ascii_lowercase();
                let desc = entry.description.to_ascii_lowercase();
                command.contains(query.as_str()) || desc.contains(query.as_str())
            })
            .cloned()
            .map(|entry| PaletteItem {
                label: self.display_label(&entry),
                status_tag: None,
                description: entry.description,
                action: entry.action,
                selectable: true,
                match_target: None,
                source_skill: None,
            })
            .collect()
    }

    fn filtered_settings(&self) -> Vec<PaletteItem> {
        self.filtered_selection_entries(self.settings.as_slice())
    }

    fn filtered_models(&self) -> Vec<PaletteItem> {
        self.filtered_selection_entries(self.model_entries.as_slice())
    }

    fn filtered_reasoning(&self) -> Vec<PaletteItem> {
        self.filtered_selection_entries(self.reasoning_entries.as_slice())
    }

    fn filtered_selection_entries(&self, entries: &[SettingsEntry]) -> Vec<PaletteItem> {
        let query = self.query.trim().to_ascii_lowercase();
        entries
            .iter()
            .filter(|entry| {
                if query.is_empty() {
                    return true;
                }
                entry.label.to_ascii_lowercase().contains(query.as_str())
                    || entry
                        .description
                        .to_ascii_lowercase()
                        .contains(query.as_str())
            })
            .cloned()
            .map(|entry| PaletteItem {
                label: entry.label,
                status_tag: entry.status_tag,
                description: if entry.category_tag.is_empty() {
                    entry.description
                } else {
                    format!("{} {}", entry.category_tag, entry.description)
                },
                action: entry.action,
                selectable: entry.selectable,
                match_target: None,
                source_skill: None,
            })
            .collect()
    }

    fn filtered_skills(&self) -> Vec<PaletteItem> {
        let query = self.query.trim().to_ascii_lowercase();
        let mut matches = self
            .skills
            .iter()
            .filter_map(|skill| {
                if query.is_empty() {
                    return Some((skill, SkillMatchTarget::Label(usize::MAX, 0)));
                }

                let name = skill.name.to_ascii_lowercase();
                let desc = skill.description.to_ascii_lowercase();
                if let Some(index) = name.find(query.as_str()) {
                    return Some((skill, SkillMatchTarget::Label(index, query.len())));
                }
                if let Some(indices) = fuzzy_match_positions(skill.name.as_str(), query.as_str()) {
                    return Some((skill, SkillMatchTarget::LabelFuzzy(indices)));
                }
                if let Some((index, term)) = skill.search_terms.iter().find_map(|term| {
                    let lower = term.to_ascii_lowercase();
                    lower
                        .find(query.as_str())
                        .map(|index| (index, term.to_owned()))
                }) {
                    return Some((skill, SkillMatchTarget::SearchTerm { index, term }));
                }
                desc.find(query.as_str())
                    .map(|index| (skill, SkillMatchTarget::Description(index, query.len())))
            })
            .collect::<Vec<_>>();

        matches.sort_by(|(left_skill, left_target), (right_skill, right_target)| {
            if query.is_empty() {
                skill_label_priority(left_skill)
                    .cmp(&skill_label_priority(right_skill))
                    .then_with(|| left_skill.name.cmp(&right_skill.name))
            } else {
                left_target
                    .sort_priority()
                    .cmp(&right_target.sort_priority())
                    .then_with(|| left_target.match_index().cmp(&right_target.match_index()))
                    .then_with(|| {
                        skill_label_priority(left_skill).cmp(&skill_label_priority(right_skill))
                    })
                    .then_with(|| left_skill.name.cmp(&right_skill.name))
            }
        });

        matches
            .into_iter()
            .map(|(skill, match_target)| PaletteItem {
                label: format!("${}", skill.name),
                status_tag: None,
                description: format_skill_popup_description(skill, &match_target),
                action: CommandAction::InsertText(format!("${} ", skill.name)),
                selectable: true,
                match_target: (!query.is_empty())
                    .then_some(adjust_skill_match_target_for_label(match_target, 1)),
                source_skill: Some(skill.clone()),
            })
            .collect()
    }

    fn display_label(&self, entry: &CommandEntry) -> String {
        entry.command.to_owned()
    }

    fn visible_rows_for_total(total: usize) -> usize {
        total.clamp(1, Self::VISIBLE_ROWS)
    }

    fn step_selection(&mut self, delta: isize) {
        let total = self.filtered_item_count();
        if total == 0 {
            return;
        }

        let filtered = self.filtered_items();
        let selectable = filtered
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| entry.selectable.then_some(idx))
            .collect::<Vec<_>>();
        let Some(first_selectable) = selectable.first().copied() else {
            self.scroll_state.reset();
            return;
        };
        let current = self
            .scroll_state
            .selected_idx
            .filter(|idx| filtered.get(*idx).is_some_and(|entry| entry.selectable))
            .unwrap_or(first_selectable);
        let current_pos = selectable
            .iter()
            .position(|idx| *idx == current)
            .unwrap_or(0);
        let next_pos = if delta < 0 {
            if current_pos == 0 {
                selectable.len() - 1
            } else {
                current_pos - 1
            }
        } else if current_pos + 1 >= selectable.len() {
            0
        } else {
            current_pos + 1
        };
        let Some(index) = selectable.get(next_pos).copied() else {
            return;
        };
        self.scroll_state.selected_idx = Some(index);
        self.scroll_state
            .ensure_visible(total, Self::visible_rows_for_total(total));
    }

    fn first_selectable_index(&self) -> Option<usize> {
        self.settings.iter().position(|entry| entry.selectable)
    }

    fn selected_index_for(&mut self, filtered: &[PaletteItem]) -> usize {
        let selectable = selectable_indices(filtered);
        if selectable.is_empty() {
            self.scroll_state.reset();
            return 0;
        }
        self.scroll_state.clamp_selection(filtered.len());
        let selected = self
            .scroll_state
            .selected_idx
            .filter(|idx| filtered.get(*idx).is_some_and(|entry| entry.selectable))
            .unwrap_or_else(|| selectable.first().copied().unwrap_or(0));
        self.scroll_state.selected_idx = Some(selected);
        self.scroll_state
            .ensure_visible(filtered.len(), Self::visible_rows_for_total(filtered.len()));
        selected
    }

    fn jump_selection_by_page(&mut self, delta: isize) {
        let filtered = self.filtered_items();
        let selectable = selectable_indices(&filtered);
        let Some(first_selectable) = selectable.first().copied() else {
            self.scroll_state.reset();
            return;
        };
        let current = self
            .scroll_state
            .selected_idx
            .filter(|idx| filtered.get(*idx).is_some_and(|entry| entry.selectable))
            .unwrap_or(first_selectable);
        let current_pos = selectable
            .iter()
            .position(|idx| *idx == current)
            .unwrap_or(0);
        let next_pos = if delta.is_negative() {
            current_pos.saturating_sub(delta.unsigned_abs())
        } else {
            current_pos
                .saturating_add(delta as usize)
                .min(selectable.len().saturating_sub(1))
        };
        let Some(index) = selectable.get(next_pos).copied() else {
            return;
        };
        self.scroll_state.selected_idx = Some(index);
        self.scroll_state
            .ensure_visible(filtered.len(), Self::visible_rows_for_total(filtered.len()));
    }
}

fn selectable_indices(filtered: &[PaletteItem]) -> Vec<usize> {
    filtered
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| entry.selectable.then_some(idx))
        .collect()
}

fn selection_index_for_entries(entries: &[SettingsEntry], selected_label: Option<&str>) -> usize {
    selected_label
        .and_then(|label| {
            entries
                .iter()
                .position(|entry| entry.selectable && entry.label == label)
        })
        .or_else(|| entries.iter().position(|entry| entry.selectable))
        .unwrap_or(0)
}

fn skill_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        Span::raw("Press "),
        Span::styled(
            "Enter",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" / "),
        Span::styled(
            "Tab",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to insert or "),
        Span::styled(
            "Esc",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to close"),
    ])
}

fn skill_label_max_width(skill: &SkillEntry) -> usize {
    match skill.category_tag.as_str() {
        "[Plugin]" | "[Connector]" => 22,
        _ => 24,
    }
}

fn skill_context_max_width(skill: &SkillEntry) -> usize {
    match skill.category_tag.as_str() {
        "[Plugin]" | "[Connector]" => 14,
        _ => 18,
    }
}

fn skill_label_priority(skill: &SkillEntry) -> u8 {
    let category = skill.category_tag.to_ascii_lowercase();
    if category.contains("repo") {
        0
    } else if category.contains("plugin") {
        1
    } else if category.contains("connector") {
        2
    } else if category.contains("skill") {
        3
    } else if skill.name.contains("browser") {
        4
    } else {
        5
    }
}

fn format_skill_popup_description(skill: &SkillEntry, match_target: &SkillMatchTarget) -> String {
    match match_target {
        SkillMatchTarget::SearchTerm { term, .. } if term != &skill.name => {
            format!("{} {} · {}", skill.category_tag, term, skill.description)
        }
        SkillMatchTarget::Label(..)
        | SkillMatchTarget::LabelFuzzy(..)
        | SkillMatchTarget::SearchTerm { .. }
        | SkillMatchTarget::Description(..) => {
            if let Some(alias) = skill.source_alias.as_deref() {
                format!("{} {} · {}", skill.category_tag, alias, skill.description)
            } else {
                format!("{} {}", skill.category_tag, skill.description)
            }
        }
    }
}

fn split_category_tag(description: &str) -> (&str, &str) {
    let trimmed = description.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((tag_body, remainder)) = rest.split_once(']')
    {
        let tag_len = tag_body.len() + 2;
        let (tag, _) = trimmed.split_at(tag_len);
        return (tag, remainder.trim_start());
    }
    ("", trimmed)
}

fn adjust_skill_match_target_for_label(
    match_target: SkillMatchTarget,
    offset: usize,
) -> SkillMatchTarget {
    match match_target {
        SkillMatchTarget::Label(start, len) => {
            SkillMatchTarget::Label(start.saturating_add(offset), len)
        }
        SkillMatchTarget::LabelFuzzy(indices) => SkillMatchTarget::LabelFuzzy(
            indices
                .into_iter()
                .map(|index| index.saturating_add(offset))
                .collect(),
        ),
        other @ SkillMatchTarget::SearchTerm { .. } | other @ SkillMatchTarget::Description(..) => {
            other
        }
    }
}

fn split_description_context(description: &str) -> (&str, &str) {
    description
        .split_once(" · ")
        .map(|(context, detail)| (context.trim_end(), detail.trim_start()))
        .unwrap_or(("", description))
}

fn render_match_highlight_spans(
    text: &str,
    match_target: Option<SkillMatchTarget>,
    normal_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    let Some(ranges) = match_target.and_then(|target| target.highlight_ranges(text)) else {
        return vec![Span::styled(text.to_owned(), normal_style)];
    };
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for range in ranges {
        if range.start > cursor {
            spans.push(Span::styled(
                text[cursor..range.start].to_owned(),
                normal_style,
            ));
        }
        spans.push(Span::styled(
            text[range.clone()].to_owned(),
            highlight_style,
        ));
        cursor = range.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), normal_style));
    }
    spans
}

fn skill_category_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_GRAY
    } else {
        SURFACE_DIM_GRAY
    })
}

fn skill_label_style(selected: bool) -> Style {
    Style::default()
        .fg(if selected {
            SURFACE_CYAN
        } else {
            ratatui::style::Color::White
        })
        .add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

fn skill_label_highlight_style(selected: bool) -> Style {
    Style::default()
        .fg(if selected {
            SURFACE_CYAN
        } else {
            SURFACE_ACCENT
        })
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

fn skill_context_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_GRAY
    } else {
        SURFACE_DIM_GRAY
    })
}

fn skill_separator_style() -> Style {
    Style::default().fg(SURFACE_DIM_GRAY)
}

fn skill_description_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_ACCENT
    } else {
        SURFACE_GRAY
    })
}

fn skill_highlight_style() -> Style {
    Style::default()
        .fg(ratatui::style::Color::White)
        .add_modifier(Modifier::UNDERLINED)
}

fn render_skill_description_spans(
    context: &str,
    full_description: &str,
    match_target: Option<SkillMatchTarget>,
    selected: bool,
) -> Vec<Span<'static>> {
    let (context_part, detail_part) = split_description_context(full_description);
    let highlighted_range = description_match_target(full_description, match_target)
        .and_then(|target| target.highlight_ranges(full_description))
        .and_then(|mut ranges| ranges.drain(..).next());
    let mut spans = Vec::new();
    if !context_part.is_empty() {
        spans.extend(render_segment_highlight_spans(
            context_part,
            0,
            skill_context_style(selected),
            skill_highlight_style(),
            highlighted_range.clone(),
        ));
    }
    if !context_part.is_empty() && !detail_part.is_empty() {
        spans.push(Span::styled(" · ", skill_separator_style()));
    }
    if !detail_part.is_empty() {
        let detail_start = if context_part.is_empty() {
            0
        } else {
            context_part.len() + 3
        };
        spans.extend(render_segment_highlight_spans(
            detail_part,
            detail_start,
            skill_description_style(selected),
            skill_highlight_style(),
            highlighted_range,
        ));
    }
    if context_part.is_empty() && detail_part.is_empty() && !context.is_empty() {
        spans.push(Span::styled(
            context.to_owned(),
            skill_context_style(selected),
        ));
    }
    spans
}

fn render_segment_highlight_spans(
    text: &str,
    offset: usize,
    normal_style: Style,
    highlight_style: Style,
    highlight_range: Option<std::ops::Range<usize>>,
) -> Vec<Span<'static>> {
    let Some(range) = highlight_range else {
        return vec![Span::styled(text.to_owned(), normal_style)];
    };
    let segment_start = offset;
    let segment_end = offset.saturating_add(text.len());
    let overlap_start = range.start.max(segment_start);
    let overlap_end = range.end.min(segment_end);
    if overlap_start >= overlap_end {
        return vec![Span::styled(text.to_owned(), normal_style)];
    }

    let relative_start = overlap_start.saturating_sub(segment_start);
    let relative_end = overlap_end.saturating_sub(segment_start);
    let mut spans = Vec::new();
    if relative_start > 0 {
        spans.push(Span::styled(
            text[..relative_start].to_owned(),
            normal_style,
        ));
    }
    spans.push(Span::styled(
        text[relative_start..relative_end].to_owned(),
        highlight_style,
    ));
    if relative_end < text.len() {
        spans.push(Span::styled(text[relative_end..].to_owned(), normal_style));
    }
    spans
}

fn fuzzy_match_positions(text: &str, query: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return Some(Vec::new());
    }

    let query_chars = query
        .chars()
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut matched_positions = Vec::new();
    let mut query_index = 0usize;

    for (index, ch) in text.char_indices() {
        let Some(expected) = query_chars.get(query_index) else {
            break;
        };
        if ch.to_ascii_lowercase() == *expected {
            matched_positions.push(index);
            query_index += 1;
        }
    }

    (query_index == query_chars.len()).then_some(matched_positions)
}

fn description_match_target(
    description: &str,
    match_target: Option<SkillMatchTarget>,
) -> Option<SkillMatchTarget> {
    match match_target {
        Some(SkillMatchTarget::SearchTerm { term, .. }) => {
            let lower_description = description.to_ascii_lowercase();
            let lower_term = term.to_ascii_lowercase();
            lower_description
                .find(lower_term.as_str())
                .map(|index| SkillMatchTarget::Description(index, term.len()))
        }
        other => other.filter(|target| target.is_description()),
    }
}

#[derive(Debug, Clone)]
struct PaletteItem {
    label: String,
    status_tag: Option<String>,
    description: String,
    action: CommandAction,
    selectable: bool,
    match_target: Option<SkillMatchTarget>,
    source_skill: Option<SkillEntry>,
}

#[derive(Debug, Clone)]
enum SkillMatchTarget {
    Label(usize, usize),
    LabelFuzzy(Vec<usize>),
    SearchTerm { index: usize, term: String },
    Description(usize, usize),
}

impl SkillMatchTarget {
    fn sort_priority(&self) -> usize {
        match self {
            Self::Label(..) => 0,
            Self::LabelFuzzy(..) => 1,
            Self::SearchTerm { .. } => 2,
            Self::Description(..) => 3,
        }
    }

    fn match_index(&self) -> usize {
        match self {
            Self::Label(index, _) | Self::Description(index, _) => *index,
            Self::LabelFuzzy(indices) => indices.first().copied().unwrap_or(usize::MAX),
            Self::SearchTerm { index, .. } => *index,
        }
    }

    fn is_label(&self) -> bool {
        matches!(self, Self::Label(..) | Self::LabelFuzzy(..))
    }

    fn is_description(&self) -> bool {
        matches!(self, Self::Description(..))
    }

    fn highlight_ranges(&self, text: &str) -> Option<Vec<std::ops::Range<usize>>> {
        match self {
            Self::Label(start, len) | Self::Description(start, len) => {
                let end = start.saturating_add(*len);
                if *start <= text.len()
                    && end <= text.len()
                    && text.is_char_boundary(*start)
                    && text.is_char_boundary(end)
                {
                    Some(std::iter::once(*start..end).collect())
                } else {
                    None
                }
            }
            Self::LabelFuzzy(indices) => {
                let mut ranges = Vec::new();
                for start in indices {
                    if !text.is_char_boundary(*start) {
                        return None;
                    }
                    let end = text[*start..]
                        .chars()
                        .next()
                        .map(|ch| start.saturating_add(ch.len_utf8()))?;
                    ranges.push(*start..end);
                }
                Some(ranges)
            }
            Self::SearchTerm { .. } => None,
        }
    }
}

fn truncate(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    let count = crate::presentation::display_width(text);
    if count <= max_len {
        return text.to_owned();
    }
    if max_len == 1 {
        return "…".to_owned();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let width = crate::presentation::char_display_width(ch);
        if used + width > max_len.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push('…');
    out
}

fn area_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::{CommandAction, CommandPalette, SettingsEntry, SettingsSurfaceFocus, SkillEntry};
    use crate::chat::chat_surface::i18n::Language;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn render_to_string(palette: &mut CommandPalette, area: Rect) -> String {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| palette.render(frame, area))
            .expect("draw");
        format!("{:?}", terminal.backend().buffer())
    }

    fn render_to_buffer(palette: &mut CommandPalette, area: Rect) -> Buffer {
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| palette.render(frame, area))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, row: u16, width: u16) -> String {
        (0..width)
            .map(|x| buffer[(x, row)].symbol())
            .collect::<String>()
    }

    fn skill(name: &str, description: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            search_terms: vec![name.to_owned()],
            category_tag: "[Skill]".to_owned(),
            source_alias: None,
        }
    }

    fn plugin(name: &str, description: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            search_terms: vec![name.to_owned()],
            category_tag: "[Plugin]".to_owned(),
            source_alias: None,
        }
    }

    #[test]
    fn enter_uses_filtered_selection_instead_of_raw_index() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("approval");

        let action = palette.handle_key(key(KeyCode::Enter));

        match action {
            Some(CommandAction::RunCommand("/review")) => {}
            other => panic!("expected /review action, got {other:?}"),
        }
    }

    #[test]
    fn backspace_updates_query_without_panic() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("/compactx");
        palette.handle_key(key(KeyCode::Backspace));

        let action = palette.handle_key(key(KeyCode::Enter));

        match action {
            Some(CommandAction::RunCommand("/compact")) => {}
            other => panic!("expected /compact action after backspace, got {other:?}"),
        }
    }

    #[test]
    fn requested_slash_commands_keep_product_order_and_ready_entries() {
        let commands = super::slash_command_specs()
            .iter()
            .map(|spec| spec.command)
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec![
                "/model",
                "/permissions",
                "/settings",
                "/experimental",
                "/skills",
                "/mcp",
                "/rename",
                "/review",
                "/new",
                "/resume",
                "/fork",
                "/compact",
                "/plan",
                "/copy",
                "/diff",
                "/title",
                "/feedback",
                "/clear",
                "/cwd",
                "/language",
                "/share",
                "/export",
                "/import",
                "/themes",
                "/simplify",
                "/usage",
                "/missions",
                "/subagents",
            ]
        );
        assert!(super::slash_command_specs().iter().all(|spec| spec.ready));
    }

    #[test]
    fn settings_mode_selects_the_first_actionable_row() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_settings(
            SettingsSurfaceFocus::Workspace,
            vec![
                SettingsEntry {
                    label: "Current workspace".to_owned(),
                    category_tag: "[State]".to_owned(),
                    status_tag: Some("state".to_owned()),
                    description: "summary".to_owned(),
                    action: CommandAction::Noop,
                    selectable: false,
                },
                SettingsEntry {
                    label: "Back to settings".to_owned(),
                    category_tag: "[Navigation]".to_owned(),
                    status_tag: None,
                    description: "return".to_owned(),
                    action: CommandAction::OpenSettings(SettingsSurfaceFocus::Overview),
                    selectable: true,
                },
            ],
            None,
            None,
        );

        let action = palette.handle_key(key(KeyCode::Enter));

        assert_eq!(
            action,
            Some(CommandAction::OpenSettings(SettingsSurfaceFocus::Overview))
        );
    }

    #[test]
    fn reasoning_escape_returns_to_model_selector_parent_view() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_model_selector(
            vec![SettingsEntry {
                label: "gpt-5".to_owned(),
                category_tag: "[Model]".to_owned(),
                status_tag: Some("current".to_owned()),
                description: "OpenAI model · choose reasoning next".to_owned(),
                action: CommandAction::OpenModelReasoning("gpt-5".to_owned()),
                selectable: true,
            }],
            None,
            Some("gpt-5"),
            "",
        );
        palette.show_reasoning_selector(
            "gpt-5",
            vec![SettingsEntry {
                label: "default".to_owned(),
                category_tag: "[Reasoning]".to_owned(),
                status_tag: Some("current".to_owned()),
                description: "use the provider or model default reasoning behavior".to_owned(),
                action: CommandAction::ApplyModelSelection {
                    model: "gpt-5".to_owned(),
                    reasoning_effort: None,
                },
                selectable: true,
            }],
            None,
            Some("default"),
        );

        assert_eq!(palette.handle_key(key(KeyCode::Esc)), None);
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::OpenModelReasoning(model)) if model == "gpt-5" => {}
            other => panic!("expected escape to restore model selector, got {other:?}"),
        }
    }

    #[test]
    fn requested_slash_commands_do_not_show_placeholder_copy() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        for entry in palette.filtered_commands() {
            assert!(!entry.description.contains("coming soon"));
            assert!(!entry.description.contains("placeholder"));
            assert!(!entry.description.contains("not wired"));
        }
    }

    #[test]
    fn query_matches_requested_command_descriptions() {
        let mut palette = CommandPalette::new(Language::ZhCn, Vec::new());
        palette.show_commands("mission-control");

        let action = palette.handle_key(key(KeyCode::Enter));

        match action {
            Some(CommandAction::RunCommand("/missions")) => {}
            other => panic!("expected /missions action, got {other:?}"),
        }
    }

    #[test]
    fn desired_height_shrinks_with_filtered_results() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("mission-control");
        assert_eq!(palette.desired_height(), 2);

        palette.show_commands("");
        assert_eq!(palette.desired_height(), 8);
    }

    #[test]
    fn desired_height_for_no_matches_keeps_only_result_and_footer() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("zzz-no-match");
        assert_eq!(palette.desired_height(), 2);
    }

    #[test]
    fn truncate_respects_display_cell_width_for_cjk() {
        let truncated = super::truncate("帮助命令说明", 5);

        assert!(crate::presentation::display_width(&truncated) <= 5);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn page_navigation_moves_by_visible_rows() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let commands = super::slash_command_specs();
        let expected_page_down = commands[CommandPalette::VISIBLE_ROWS].command;
        let _ = palette.handle_key(key(KeyCode::PageDown));
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand(command)) if command == expected_page_down => {}
            other => panic!("expected page-down to land on {expected_page_down}, got {other:?}"),
        }

        let _ = palette.handle_key(key(KeyCode::PageUp));
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/model")) => {}
            other => panic!("expected page-up to return to /model, got {other:?}"),
        }
    }

    #[test]
    fn home_and_end_jump_to_extremes() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let _ = palette.handle_key(key(KeyCode::End));
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/subagents")) => {}
            other => panic!("expected end to land on /subagents, got {other:?}"),
        }

        let _ = palette.handle_key(key(KeyCode::Home));
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/model")) => {}
            other => panic!("expected home to land on /model, got {other:?}"),
        }
    }

    #[test]
    fn arrow_navigation_wraps_at_edges() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let action = palette.handle_key(key(KeyCode::Up));
        match action {
            None => {}
            other => panic!("unexpected action while wrapping up: {other:?}"),
        }
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/subagents")) => {}
            other => panic!("expected wrap-up to land on /subagents, got {other:?}"),
        }

        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");
        for _ in 0..super::slash_command_specs().len().saturating_sub(1) {
            let _ = palette.handle_key(key(KeyCode::Down));
        }
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/subagents")) => {}
            other => panic!("expected repeated down to reach /subagents, got {other:?}"),
        }
        let _ = palette.handle_key(key(KeyCode::Down));
        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/model")) => {}
            other => panic!("expected wrap-down to return to /model, got {other:?}"),
        }
    }

    #[test]
    fn skill_palette_inserts_selected_skill_invocation() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![
                skill("demo-skill", "demo query description"),
                skill("other-helper", "unrelated helper description"),
            ],
        );
        palette.show_skills("$demo");

        let action = palette.handle_key(key(KeyCode::Enter));

        match action {
            Some(CommandAction::InsertText(text)) => assert_eq!(text, "$demo-skill "),
            other => panic!("expected skill insertion action, got {other:?}"),
        }
    }

    #[test]
    fn skill_palette_uses_popup_height_with_hint_row() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![skill("demo-skill", "demo skill description")],
        );
        palette.show_skills("$dem");

        assert_eq!(palette.desired_height(), 3);
    }

    #[test]
    fn skill_palette_renders_inline_hint_text() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![skill("demo-skill", "demo skill description")],
        );
        palette.show_skills("$dem");

        let rendered = render_to_string(&mut palette, Rect::new(0, 0, 64, 3));

        assert!(rendered.contains("Press Enter / Tab to insert or Esc to close"));
        assert!(rendered.contains("[Skill]"));
        assert!(rendered.contains("demo skill description"));
    }

    #[test]
    fn split_category_tag_separates_tag_from_description() {
        let (tag, description) = super::split_category_tag("[Skill] demo description");
        assert_eq!(tag, "[Skill]");
        assert_eq!(description, "demo description");

        let (tag, description) = super::split_category_tag("demo description");
        assert_eq!(tag, "");
        assert_eq!(description, "demo description");
    }

    #[test]
    fn split_description_context_preserves_alias_prefix() {
        let (context, detail) =
            super::split_description_context("babysit-pr · triage pull requests");
        assert_eq!(context, "babysit-pr");
        assert_eq!(detail, "triage pull requests");

        let (context, detail) = super::split_description_context("plain description");
        assert_eq!(context, "");
        assert_eq!(detail, "plain description");
    }

    #[test]
    fn adjust_skill_match_target_for_label_offsets_name_matches() {
        match super::adjust_skill_match_target_for_label(super::SkillMatchTarget::Label(0, 4), 1) {
            super::SkillMatchTarget::Label(start, len) => {
                assert_eq!(start, 1);
                assert_eq!(len, 4);
            }
            other @ super::SkillMatchTarget::LabelFuzzy(_)
            | other @ super::SkillMatchTarget::SearchTerm { .. }
            | other @ super::SkillMatchTarget::Description(..) => {
                panic!("expected shifted label target, got {other:?}")
            }
        }
    }

    #[test]
    fn render_match_highlight_spans_splits_highlighted_segment() {
        let spans = super::render_match_highlight_spans(
            "$demo-skill",
            Some(super::SkillMatchTarget::Label(1, 4)),
            Style::default(),
            Style::default().add_modifier(Modifier::BOLD),
        );

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "$");
        assert_eq!(spans[1].content.as_ref(), "demo");
        assert_eq!(spans[2].content.as_ref(), "-skill");
    }

    #[test]
    fn render_match_highlight_spans_supports_fuzzy_positions() {
        let spans = super::render_match_highlight_spans(
            "$github",
            Some(super::SkillMatchTarget::LabelFuzzy(vec![1, 3, 6])),
            Style::default(),
            Style::default().add_modifier(Modifier::BOLD),
        );

        assert_eq!(spans.len(), 6);
        assert_eq!(spans[0].content.as_ref(), "$");
        assert_eq!(spans[1].content.as_ref(), "g");
        assert_eq!(spans[2].content.as_ref(), "i");
        assert_eq!(spans[3].content.as_ref(), "t");
        assert_eq!(spans[4].content.as_ref(), "hu");
        assert_eq!(spans[5].content.as_ref(), "b");
    }

    #[test]
    fn skill_highlight_style_uses_underlined_white_text() {
        let style = super::skill_highlight_style();
        assert_eq!(style.fg, Some(ratatui::style::Color::White));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_highlight_style_uses_accent_when_unselected() {
        let style = super::skill_label_highlight_style(false);
        assert_eq!(style.fg, Some(super::SURFACE_ACCENT));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_highlight_style_uses_cyan_when_selected() {
        let style = super::skill_label_highlight_style(true);
        assert_eq!(style.fg, Some(super::SURFACE_CYAN));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_style_selected_matches_popup_emphasis() {
        let style = super::skill_label_style(true);
        assert_eq!(style.fg, Some(super::SURFACE_CYAN));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn skill_category_style_dims_unselected_rows() {
        assert_eq!(
            super::skill_category_style(false).fg,
            Some(super::SURFACE_DIM_GRAY)
        );
        assert_eq!(
            super::skill_category_style(true).fg,
            Some(super::SURFACE_GRAY)
        );
    }

    #[test]
    fn render_skill_description_spans_splits_context_and_detail_without_match() {
        let spans = super::render_skill_description_spans(
            "babysit-pr",
            "babysit-pr · triage pull requests",
            None,
            false,
        );

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert_eq!(spans[1].content.as_ref(), " · ");
        assert_eq!(spans[2].content.as_ref(), "triage pull requests");
        assert_eq!(spans[0].style.fg, Some(super::SURFACE_DIM_GRAY));
        assert_eq!(spans[2].style.fg, Some(super::SURFACE_GRAY));
    }

    #[test]
    fn render_skill_description_spans_preserves_context_and_detail_styles_around_highlight() {
        let spans = super::render_skill_description_spans(
            "babysit-pr",
            "babysit-pr · triage pull requests",
            Some(super::SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
            false,
        );

        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert_eq!(spans[1].content.as_ref(), " · ");
        assert_eq!(spans[2].content.as_ref(), "triage pull requests");
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(spans[2].style.fg, Some(super::SURFACE_GRAY));
    }

    #[test]
    fn description_match_target_uses_alias_term_when_search_term_matched() {
        let target = super::description_match_target(
            "babysit-pr · triage pull requests",
            Some(super::SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
        );

        match target {
            Some(super::SkillMatchTarget::Description(index, len)) => {
                assert_eq!(index, 0);
                assert_eq!(len, "babysit-pr".len());
            }
            other => panic!("expected description highlight target, got {other:?}"),
        }
    }

    #[test]
    fn skill_palette_name_match_sorts_before_description_only_match() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![
                plugin("github-issues", "manage repository issues"),
                skill("browser-companion", "preview browser automation flow"),
                skill("issue-helper", "issue notes helper"),
            ],
        );
        palette.show_skills("$iss");

        let first = palette.handle_key(key(KeyCode::Enter));

        match first {
            Some(CommandAction::InsertText(text)) => assert_eq!(text, "$issue-helper "),
            other => panic!("expected name match to sort first, got {other:?}"),
        }
    }

    #[test]
    fn skill_palette_fuzzy_name_match_still_surfaces_candidate() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![plugin("github-issues", "manage repository issues")],
        );
        palette.show_skills("$gti");

        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::InsertText(text)) => assert_eq!(text, "$github-issues "),
            other => panic!("expected fuzzy name match to surface candidate, got {other:?}"),
        }
    }

    #[test]
    fn skill_palette_empty_query_prefers_plugin_category_then_name() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![
                skill("zebra-skill", "zebra"),
                plugin("github-plugin", "plugin"),
                skill("alpha-skill", "alpha"),
            ],
        );
        palette.show_skills("$");

        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::InsertText(text)) => assert_eq!(text, "$github-plugin "),
            other => panic!("expected plugin category to sort first on empty query, got {other:?}"),
        }
    }

    #[test]
    fn plugin_labels_use_compact_width_budget() {
        let plugin = plugin("very-long-plugin-name-for-popup", "plugin description");
        assert_eq!(super::skill_label_max_width(&plugin), 22);

        let skill = skill("very-long-skill-name-for-popup", "skill description");
        assert_eq!(super::skill_label_max_width(&skill), 24);
    }

    #[test]
    fn plugin_context_uses_compact_width_budget() {
        let plugin = plugin("very-long-plugin-name-for-popup", "plugin description");
        assert_eq!(super::skill_context_max_width(&plugin), 14);

        let skill = skill("very-long-skill-name-for-popup", "skill description");
        assert_eq!(super::skill_context_max_width(&skill), 18);
    }

    #[test]
    fn skill_popup_keeps_category_tag_visible_when_width_is_tight() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![plugin(
                "very-long-plugin-name-for-popup",
                "plugin description that should truncate first",
            )],
        );
        palette.show_skills("$");

        let area = Rect::new(0, 0, 28, 3);
        let rendered = render_to_string(&mut palette, area);

        assert!(rendered.contains("[Plugin]"));
    }

    #[test]
    fn skill_popup_keeps_alias_visible_when_width_is_tight() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![SkillEntry {
                name: "PR Babysitter".to_owned(),
                description: "triage pull requests".to_owned(),
                search_terms: vec!["PR Babysitter".to_owned(), "babysit-pr".to_owned()],
                category_tag: "[Skill]".to_owned(),
                source_alias: Some("babysit-pr".to_owned()),
            }],
        );
        palette.show_skills("$");

        let area = Rect::new(0, 0, 36, 3);
        let buffer = render_to_buffer(&mut palette, area);
        let row = row_text(&buffer, 0, area.width);
        assert!(row.contains("babysit"));
    }

    #[test]
    fn skill_popup_truncates_long_alias_context_before_dropping_detail() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![SkillEntry {
                name: "Long Skill".to_owned(),
                description: "extra detail still visible".to_owned(),
                search_terms: vec![
                    "Long Skill".to_owned(),
                    "very-long-folder-alias-for-skill".to_owned(),
                ],
                category_tag: "[Skill]".to_owned(),
                source_alias: Some("very-long-folder-alias-for-skill".to_owned()),
            }],
        );
        palette.show_skills("$");

        let area = Rect::new(0, 0, 40, 3);
        let buffer = render_to_buffer(&mut palette, area);
        let row = row_text(&buffer, 0, area.width);
        assert!(row.contains("very"));
        assert!(row.contains("extra"));
    }

    #[test]
    fn skill_popup_renders_unselected_category_tag_with_dim_style() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![
                plugin("github-plugin", "plugin"),
                skill("alpha-skill", "alpha skill description"),
            ],
        );
        palette.show_skills("$");

        let area = Rect::new(0, 0, 64, 4);
        let buffer = render_to_buffer(&mut palette, area);
        let second_row = row_text(&buffer, 1, area.width);
        let tag_index = second_row.find("[Skill]").expect("skill tag");

        assert_eq!(buffer[(tag_index as u16, 1)].fg, super::SURFACE_DIM_GRAY);
    }

    #[test]
    fn skill_popup_alias_match_highlights_alias_in_description() {
        let target = super::description_match_target(
            "babysit-pr · triage pull requests",
            Some(super::SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
        );
        let spans = super::render_match_highlight_spans(
            "babysit-pr · triage pull requests",
            target,
            Style::default(),
            Style::default().add_modifier(Modifier::UNDERLINED),
        );

        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_popup_fuzzy_label_highlights_matching_characters_in_row_buffer() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![plugin("github-issues", "manage repository issues")],
        );
        palette.show_skills("$gti");

        let area = Rect::new(0, 0, 72, 3);
        let buffer = render_to_buffer(&mut palette, area);
        let row = row_text(&buffer, 0, area.width);
        let label_index = row.find("$github-issues").expect("label");
        let fuzzy_positions =
            super::fuzzy_match_positions("github-issues", "gti").expect("fuzzy positions");
        for relative in fuzzy_positions.into_iter().map(|index| index + 1) {
            let idx = label_index + relative;
            let cell = &buffer[(idx as u16, 0)];
            assert_eq!(cell.fg, super::SURFACE_CYAN);
        }
    }

    #[test]
    fn skill_popup_selected_row_keeps_context_dim_and_detail_accent() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![SkillEntry {
                name: "PR Babysitter".to_owned(),
                description: "triage pull requests".to_owned(),
                search_terms: vec!["PR Babysitter".to_owned(), "babysit-pr".to_owned()],
                category_tag: "[Skill]".to_owned(),
                source_alias: Some("babysit-pr".to_owned()),
            }],
        );
        palette.show_skills("$");

        let area = Rect::new(0, 0, 72, 3);
        let buffer = render_to_buffer(&mut palette, area);
        let row = row_text(&buffer, 0, area.width);
        let context_index = row.find("babysit-pr").expect("alias context");
        let detail_index = row.find("triage").expect("detail text");

        assert_eq!(buffer[(context_index as u16, 0)].fg, super::SURFACE_GRAY);
        assert_eq!(buffer[(detail_index as u16, 0)].fg, super::SURFACE_ACCENT);
    }

    #[test]
    fn skill_palette_matches_folder_alias_search_term() {
        let mut palette = CommandPalette::new(
            Language::En,
            vec![SkillEntry {
                name: "PR Babysitter".to_owned(),
                description: "triage pull requests".to_owned(),
                search_terms: vec!["PR Babysitter".to_owned(), "babysit-pr".to_owned()],
                category_tag: "[Skill]".to_owned(),
                source_alias: Some("babysit-pr".to_owned()),
            }],
        );
        palette.show_skills("$baby");

        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::InsertText(text)) => assert_eq!(text, "$PR Babysitter "),
            other => panic!("expected alias search to match mention, got {other:?}"),
        }
    }

    #[test]
    fn format_skill_popup_description_includes_alias_when_present() {
        let skill = SkillEntry {
            name: "PR Babysitter".to_owned(),
            description: "triage pull requests".to_owned(),
            search_terms: vec!["PR Babysitter".to_owned(), "babysit-pr".to_owned()],
            category_tag: "[Skill]".to_owned(),
            source_alias: Some("babysit-pr".to_owned()),
        };

        let description =
            super::format_skill_popup_description(&skill, &super::SkillMatchTarget::Label(0, 2));
        assert_eq!(description, "[Skill] babysit-pr · triage pull requests");
    }

    #[test]
    fn command_palette_renders_single_count_footer_even_when_area_is_tall() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let buffer = render_to_buffer(&mut palette, Rect::new(0, 0, 80, 9));
        let footer = format!("(1/{})", super::slash_command_specs().len());
        let footer_rows = (0..9)
            .filter(|row| row_text(&buffer, *row, 80).contains(footer.as_str()))
            .count();

        assert_eq!(footer_rows, 1);
    }

    #[test]
    fn mouse_scroll_moves_palette_selection() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let _ = palette.handle_mouse(
            mouse(MouseEventKind::ScrollDown, 1, 1),
            Rect::new(0, 0, 40, 8),
        );

        match palette.handle_key(key(KeyCode::Enter)) {
            Some(CommandAction::RunCommand("/permissions")) => {}
            other => panic!("expected mouse scroll to land on /permissions, got {other:?}"),
        }
    }

    #[test]
    fn mouse_click_runs_selected_palette_entry() {
        let mut palette = CommandPalette::new(Language::En, Vec::new());
        palette.show_commands("");

        let action = palette.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
            Rect::new(0, 0, 40, 8),
        );

        match action {
            Some(CommandAction::RunCommand("/permissions")) => {}
            other => panic!("expected mouse click to select /permissions, got {other:?}"),
        }
    }
}
