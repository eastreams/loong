#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    ZhCn,
    ZhTw,
    Ja,
    Ru,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceCopy {
    Tutorial,
    StartupSectionMcp,
    StartupSectionSkills,
    StartupTipCommands,
    StartupTipSkills,
    StartupTipQueue,
    StartupTipHistory,
    CommandDeckEmpty,
    FooterQueueHint,
    FooterQueueShort,
    FooterRestoreQueued,
    FooterRestoreShort,
    FooterFollowHint,
    FooterFollowShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct I18nService {
    current_lang: Language,
}

impl I18nService {
    pub fn new(lang: Language) -> Self {
        Self { current_lang: lang }
    }

    pub fn language(&self) -> Language {
        self.current_lang
    }

    pub fn text(&self, key: SurfaceCopy) -> &'static str {
        text_for(self.current_lang, key)
    }
}

pub fn resolve_default_language() -> Language {
    let locale = std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if locale.contains("zh_tw") || locale.contains("zh-hant") || locale.contains("zh_hk") {
        Language::ZhTw
    } else if locale.contains("zh_cn") || locale.contains("zh-hans") || locale.contains("zh_sg") {
        Language::ZhCn
    } else if locale.contains("ja") {
        Language::Ja
    } else if locale.contains("ru") {
        Language::Ru
    } else {
        Language::En
    }
}

fn text_for(lang: Language, key: SurfaceCopy) -> &'static str {
    match lang {
        Language::En => en_text(key),
        Language::ZhCn => zh_cn_text(key),
        Language::ZhTw => zh_tw_text(key),
        Language::Ja => ja_text(key),
        Language::Ru => ru_text(key),
    }
}

fn en_text(key: SurfaceCopy) -> &'static str {
    match key {
        SurfaceCopy::Tutorial => {
            "ctrl+c exit · :/ commands · type $skill directly · ctrl+o compaction"
        }
        SurfaceCopy::StartupSectionMcp => "MCP",
        SurfaceCopy::StartupSectionSkills => "Skills",
        SurfaceCopy::StartupTipCommands => "open commands from an empty composer with / or :",
        SurfaceCopy::StartupTipSkills => "type $skill directly to jump into an available skill",
        SurfaceCopy::StartupTipQueue => "press Tab to queue the current draft without losing flow",
        SurfaceCopy::StartupTipHistory => "use PgUp / PgDn or j / k to inspect the transcript",
        SurfaceCopy::CommandDeckEmpty => "no matching commands",
        SurfaceCopy::FooterQueueHint => "Tab to queue message",
        SurfaceCopy::FooterQueueShort => "Tab to queue",
        SurfaceCopy::FooterRestoreQueued => "to restore queued message",
        SurfaceCopy::FooterRestoreShort => "restore queued",
        SurfaceCopy::FooterFollowHint => "PgDn / End to latest reply",
        SurfaceCopy::FooterFollowShort => "End to latest",
    }
}

fn zh_cn_text(key: SurfaceCopy) -> &'static str {
    match key {
        SurfaceCopy::Tutorial => "ctrl+c 退出 · :/ 命令 · 直接输入 $skill · ctrl+o 压缩",
        SurfaceCopy::StartupSectionMcp => "MCP",
        SurfaceCopy::StartupSectionSkills => "技能",
        SurfaceCopy::StartupTipCommands => "空白输入框里按 / 或 : 打开命令面板",
        SurfaceCopy::StartupTipSkills => "直接输入 $skill 可以立刻触发可用技能",
        SurfaceCopy::StartupTipQueue => "按 Tab 可把当前草稿排队，不打断当前节奏",
        SurfaceCopy::StartupTipHistory => "用 PgUp / PgDn 或 j / k 浏览转录历史",
        SurfaceCopy::CommandDeckEmpty => "没有匹配的命令",
        SurfaceCopy::FooterQueueHint => "按 Tab 将消息加入队列",
        SurfaceCopy::FooterQueueShort => "Tab 加入队列",
        SurfaceCopy::FooterRestoreQueued => "可恢复排队消息",
        SurfaceCopy::FooterRestoreShort => "恢复队列",
        SurfaceCopy::FooterFollowHint => "PgDn / End 跳到最新回复",
        SurfaceCopy::FooterFollowShort => "End 到最新",
    }
}

fn zh_tw_text(key: SurfaceCopy) -> &'static str {
    match key {
        SurfaceCopy::Tutorial => "ctrl+c 離開 · :/ 命令 · 直接輸入 $skill · ctrl+o 壓縮",
        SurfaceCopy::StartupSectionMcp => "MCP",
        SurfaceCopy::StartupSectionSkills => "技能",
        SurfaceCopy::StartupTipCommands => "空白輸入框裡按 / 或 : 打開命令面板",
        SurfaceCopy::StartupTipSkills => "直接輸入 $skill 可立即觸發可用技能",
        SurfaceCopy::StartupTipQueue => "按 Tab 可把當前草稿排隊，不打斷節奏",
        SurfaceCopy::StartupTipHistory => "用 PgUp / PgDn 或 j / k 瀏覽逐字稿歷史",
        SurfaceCopy::CommandDeckEmpty => "沒有符合的命令",
        SurfaceCopy::FooterQueueHint => "按 Tab 將訊息加入佇列",
        SurfaceCopy::FooterQueueShort => "Tab 加入佇列",
        SurfaceCopy::FooterRestoreQueued => "可還原排隊訊息",
        SurfaceCopy::FooterRestoreShort => "還原佇列",
        SurfaceCopy::FooterFollowHint => "PgDn / End 跳到最新回覆",
        SurfaceCopy::FooterFollowShort => "End 到最新",
    }
}

fn ja_text(key: SurfaceCopy) -> &'static str {
    match key {
        SurfaceCopy::Tutorial => "ctrl+c で終了 · :/ コマンド · $skill を直接入力 · ctrl+o 圧縮",
        SurfaceCopy::StartupSectionMcp => "MCP",
        SurfaceCopy::StartupSectionSkills => "スキル",
        SurfaceCopy::StartupTipCommands => {
            "空のコンポーザーで / または : を押すとコマンドを開けます"
        }
        SurfaceCopy::StartupTipSkills => {
            "$skill を直接入力すると利用可能なスキルを即時起動できます"
        }
        SurfaceCopy::StartupTipQueue => "Tab で現在の下書きを流れを切らずにキューへ送れます",
        SurfaceCopy::StartupTipHistory => "PgUp / PgDn または j / k で履歴をたどれます",
        SurfaceCopy::CommandDeckEmpty => "一致するコマンドがありません",
        SurfaceCopy::FooterQueueHint => "Tab でメッセージをキューへ",
        SurfaceCopy::FooterQueueShort => "Tab でキューへ",
        SurfaceCopy::FooterRestoreQueued => "でキュー済みメッセージを復元",
        SurfaceCopy::FooterRestoreShort => "キュー復元",
        SurfaceCopy::FooterFollowHint => "PgDn / End で最新返信へ",
        SurfaceCopy::FooterFollowShort => "End で最新へ",
    }
}

fn ru_text(key: SurfaceCopy) -> &'static str {
    match key {
        SurfaceCopy::Tutorial => "ctrl+c выйти · :/ команды · вводите $skill прямо · ctrl+o сжатие",
        SurfaceCopy::StartupSectionMcp => "MCP",
        SurfaceCopy::StartupSectionSkills => "навыки",
        SurfaceCopy::StartupTipCommands => "откройте команды из пустого ввода через / или :",
        SurfaceCopy::StartupTipSkills => {
            "введите $skill напрямую, чтобы сразу запустить доступный навык"
        }
        SurfaceCopy::StartupTipQueue => {
            "нажмите Tab, чтобы поставить черновик в очередь без потери темпа"
        }
        SurfaceCopy::StartupTipHistory => "используйте PgUp / PgDn или j / k для просмотра истории",
        SurfaceCopy::CommandDeckEmpty => "нет подходящих команд",
        SurfaceCopy::FooterQueueHint => "Tab — поставить сообщение в очередь",
        SurfaceCopy::FooterQueueShort => "Tab — в очередь",
        SurfaceCopy::FooterRestoreQueued => "чтобы вернуть сообщение из очереди",
        SurfaceCopy::FooterRestoreShort => "вернуть очередь",
        SurfaceCopy::FooterFollowHint => "PgDn / End к последнему ответу",
        SurfaceCopy::FooterFollowShort => "End к последнему",
    }
}
