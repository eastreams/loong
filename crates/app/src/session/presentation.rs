use std::collections::hash_map::DefaultHasher;
use std::env;
#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
use std::fs;
use std::hash::{Hash, Hasher};
#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
use std::path::{Path, PathBuf};
#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
use crate::config::{default_config_path, expand_path};
use crate::conversation::SubagentProviderSnapshot;

#[cfg(feature = "memory-sqlite")]
use super::repository::{SessionEventRecord, SessionKind, SessionSummaryRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionPresentationLocale {
    ZhHans,
    ZhHant,
    En,
    Ja,
}

impl SessionPresentationLocale {
    pub(crate) fn detect_from_env() -> Self {
        let locale_keys = ["LC_ALL", "LC_MESSAGES", "LANG"];
        let locale_value = locale_keys.iter().find_map(|key| env::var(key).ok());
        let Some(locale_value) = locale_value else {
            return Self::En;
        };
        Self::from_tag(locale_value.as_str())
    }

    pub(crate) fn from_tag(raw: &str) -> Self {
        let normalized = normalize_locale_tag(raw);

        if normalized.starts_with("zh-hant")
            || normalized.starts_with("zh-tw")
            || normalized.starts_with("zh-hk")
            || normalized.starts_with("zh-mo")
        {
            return Self::ZhHant;
        }

        if normalized.starts_with("zh-hans")
            || normalized.starts_with("zh-cn")
            || normalized.starts_with("zh-sg")
            || normalized == "zh"
        {
            return Self::ZhHans;
        }

        if normalized.starts_with("ja") {
            return Self::Ja;
        }

        Self::En
    }
}

pub(crate) fn localized_root_thread_label(locale: SessionPresentationLocale) -> &'static str {
    match locale {
        SessionPresentationLocale::ZhHans => "主线",
        SessionPresentationLocale::ZhHant => "主線",
        SessionPresentationLocale::En => "Primary",
        SessionPresentationLocale::Ja => "主線",
    }
}

pub(crate) fn root_thread_search_terms() -> &'static [&'static str] {
    &[
        "main",
        "primary",
        "root",
        "default",
        "主线",
        "主線",
        "根会话",
        "根會話",
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalizedSubagentText {
    pub zh_hans: String,
    pub zh_hant: String,
    pub en: String,
    pub ja: String,
}

impl LocalizedSubagentText {
    pub(crate) fn for_locale(&self, locale: SessionPresentationLocale) -> &str {
        match locale {
            SessionPresentationLocale::ZhHans => self.zh_hans.as_str(),
            SessionPresentationLocale::ZhHant => self.zh_hant.as_str(),
            SessionPresentationLocale::En => self.en.as_str(),
            SessionPresentationLocale::Ja => self.ja.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DelegateAgentPresentation {
    pub persona_id: String,
    pub role_id: String,
    pub names: LocalizedSubagentText,
    pub roles: LocalizedSubagentText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

impl DelegateAgentPresentation {
    pub(crate) fn primary_label(&self, locale: SessionPresentationLocale) -> String {
        let name = self.names.for_locale(locale);
        let role = self.roles.for_locale(locale);
        format!("{name} · {role}")
    }

    pub(crate) fn provider_label(&self, _locale: SessionPresentationLocale) -> Option<String> {
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?;

        let mut parts = vec![model.to_owned()];
        let maybe_reasoning = self
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(reasoning_effort) = maybe_reasoning {
            parts.push(reasoning_effort.to_owned());
        }

        Some(parts.join(" · "))
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn derive_delegate_agent_presentation(
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
) -> Option<DelegateAgentPresentation> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let spawn_event = delegate_events
        .iter()
        .rev()
        .find(|event| is_delegate_spawn_event(event.event_kind.as_str()))?;
    let spawn_payload = &spawn_event.payload_json;
    let session_label = session.label.as_deref();
    let spawn_label = spawn_payload
        .get("label")
        .and_then(serde_json::Value::as_str);
    let task = spawn_payload
        .get("task")
        .and_then(serde_json::Value::as_str);
    let role = infer_delegate_agent_role(task, session_label.or(spawn_label));
    let persona = select_role_persona(role, session.session_id.as_str());
    let provider = SubagentProviderSnapshot::from_event_payload(spawn_payload);
    let model = provider
        .as_ref()
        .map(|value| value.model.trim().to_owned())
        .filter(|value| !value.is_empty());
    let reasoning_effort = provider
        .and_then(|value| value.reasoning_effort)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    Some(DelegateAgentPresentation {
        persona_id: persona.id,
        role_id: role.as_str().to_owned(),
        names: persona.names,
        roles: localized_role_label(role),
        model,
        reasoning_effort,
    })
}

fn normalize_locale_tag(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_encoding = trimmed
        .split_once('.')
        .map(|(prefix, _)| prefix)
        .unwrap_or(trimmed);
    let without_modifier = without_encoding
        .split_once('@')
        .map(|(prefix, _)| prefix)
        .unwrap_or(without_encoding);
    let normalized = without_modifier.replace('_', "-");
    normalized.to_ascii_lowercase()
}

#[cfg(feature = "memory-sqlite")]
fn is_delegate_spawn_event(event_kind: &str) -> bool {
    matches!(event_kind, "delegate_started" | "delegate_queued")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(feature = "memory-sqlite")]
enum DelegateAgentRole {
    Explorer,
    Strategist,
    Builder,
    Reviewer,
    Writer,
    Stylist,
}

#[cfg(feature = "memory-sqlite")]
impl DelegateAgentRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Strategist => "strategist",
            Self::Builder => "builder",
            Self::Reviewer => "reviewer",
            Self::Writer => "writer",
            Self::Stylist => "stylist",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "explorer" => Some(Self::Explorer),
            "strategist" => Some(Self::Strategist),
            "builder" => Some(Self::Builder),
            "reviewer" => Some(Self::Reviewer),
            "writer" => Some(Self::Writer),
            "stylist" => Some(Self::Stylist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg(feature = "memory-sqlite")]
struct PersonaSeed {
    id: &'static str,
    name_zh_hans: &'static str,
    name_zh_hant: &'static str,
    name_en: &'static str,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone)]
struct PersonaSelection {
    id: String,
    names: LocalizedSubagentText,
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
#[derive(Debug, Clone, Default, Deserialize)]
struct PersonaOverrideFile {
    #[serde(default = "persona_override_use_builtin_default")]
    use_builtin: bool,
    #[serde(default)]
    personas: Vec<PersonaOverrideRecord>,
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
#[derive(Debug, Clone, Deserialize)]
struct PersonaOverrideRecord {
    role: String,
    id: Option<String>,
    zh_hans: String,
    zh_hant: Option<String>,
    en: Option<String>,
    ja: Option<String>,
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
#[derive(Debug, Clone, Default)]
struct PersonaOverrideCatalog {
    use_builtin: bool,
    explorer: Vec<PersonaSelection>,
    strategist: Vec<PersonaSelection>,
    builder: Vec<PersonaSelection>,
    reviewer: Vec<PersonaSelection>,
    writer: Vec<PersonaSelection>,
    stylist: Vec<PersonaSelection>,
}

#[cfg(feature = "memory-sqlite")]
const EXPLORER_PERSONAS: &[PersonaSeed] = &[
    persona("jingwei", "精卫", "精衛", "Jingwei"),
    persona("kuafu", "夸父", "夸父", "Kuafu"),
    persona("xu-xiake", "徐霞客", "徐霞客", "Xu Xiake"),
    persona("zhang-qian", "张骞", "張騫", "Zhang Qian"),
    persona("zheng-he", "郑和", "鄭和", "Zheng He"),
    persona("xuanzang", "玄奘", "玄奘", "Xuanzang"),
    persona("jianzhen", "鉴真", "鑑真", "Jianzhen"),
    persona("faxian", "法显", "法顯", "Faxian"),
    persona("li-daoyuan", "郦道元", "酈道元", "Li Daoyuan"),
    persona("pei-xiu", "裴秀", "裴秀", "Pei Xiu"),
    persona("zhang-heng", "张衡", "張衡", "Zhang Heng"),
    persona("zu-chongzhi", "祖冲之", "祖沖之", "Zu Chongzhi"),
    persona("liu-hui", "刘徽", "劉徽", "Liu Hui"),
    persona("shen-kuo", "沈括", "沈括", "Shen Kuo"),
    persona("guo-shoujing", "郭守敬", "郭守敬", "Guo Shoujing"),
    persona("xu-guangqi", "徐光启", "徐光啟", "Xu Guangqi"),
    persona("song-yingxing", "宋应星", "宋應星", "Song Yingxing"),
];

#[cfg(feature = "memory-sqlite")]
const STRATEGIST_PERSONAS: &[PersonaSeed] = &[
    persona("zhou-wenwang", "周文王", "周文王", "King Wen of Zhou"),
    persona("fuxi", "伏羲", "伏羲", "Fuxi"),
    persona("laozi", "老子", "老子", "Laozi"),
    persona("zhuangzi", "庄子", "莊子", "Zhuangzi"),
    persona("kongzi", "孔子", "孔子", "Confucius"),
    persona("mengzi", "孟子", "孟子", "Mencius"),
    persona("xunzi", "荀子", "荀子", "Xunzi"),
    persona("mozi", "墨子", "墨子", "Mozi"),
    persona("han-feizi", "韩非子", "韓非子", "Han Feizi"),
    persona("guiguzi", "鬼谷子", "鬼谷子", "Guiguzi"),
    persona("zhang-zai", "张载", "張載", "Zhang Zai"),
    persona("zhou-dunyi", "周敦颐", "周敦頤", "Zhou Dunyi"),
    persona("zhu-xi", "朱熹", "朱熹", "Zhu Xi"),
    persona("wang-yangming", "王阳明", "王陽明", "Wang Yangming"),
    persona("wang-anshi", "王安石", "王安石", "Wang Anshi"),
    persona("ouyang-xiu", "欧阳修", "歐陽修", "Ouyang Xiu"),
    persona("fan-zhongyan", "范仲淹", "范仲淹", "Fan Zhongyan"),
    persona("gu-yanwu", "顾炎武", "顧炎武", "Gu Yanwu"),
    persona("wang-fuzhi", "王夫之", "王夫之", "Wang Fuzhi"),
    persona("huang-zongxi", "黄宗羲", "黃宗羲", "Huang Zongxi"),
    persona("dai-zhen", "戴震", "戴震", "Dai Zhen"),
    persona("yan-fu", "严复", "嚴復", "Yan Fu"),
];

#[cfg(feature = "memory-sqlite")]
const BUILDER_PERSONAS: &[PersonaSeed] = &[
    persona("pangu", "盘古", "盤古", "Pangu"),
    persona("nuwa", "女娲", "女媧", "Nuwa"),
    persona("shennong", "神农", "神農", "Shennong"),
    persona("dayu", "大禹", "大禹", "Yu the Great"),
    persona("cangjie", "仓颉", "倉頡", "Cangjie"),
    persona("houyi", "后羿", "后羿", "Houyi"),
    persona("li-bing", "李冰", "李冰", "Li Bing"),
    persona("ma-jun", "马钧", "馬鈞", "Ma Jun"),
    persona("su-song", "苏颂", "蘇頌", "Su Song"),
    persona("bian-que", "扁鹊", "扁鵲", "Bian Que"),
    persona("hua-tuo", "华佗", "華佗", "Hua Tuo"),
    persona("zhang-zhongjing", "张仲景", "張仲景", "Zhang Zhongjing"),
    persona("sun-simiao", "孙思邈", "孫思邈", "Sun Simiao"),
    persona("li-shizhen", "李时珍", "李時珍", "Li Shizhen"),
    persona("song-ci", "宋慈", "宋慈", "Song Ci"),
    persona("cai-lun", "蔡伦", "蔡倫", "Cai Lun"),
    persona("bi-sheng", "毕昇", "畢昇", "Bi Sheng"),
    persona("lu-ban", "鲁班", "魯班", "Lu Ban"),
];

#[cfg(feature = "memory-sqlite")]
const REVIEWER_PERSONAS: &[PersonaSeed] = &[
    persona("sima-qian", "司马迁", "司馬遷", "Sima Qian"),
    persona("han-yu", "韩愈", "韓愈", "Han Yu"),
    persona("liu-xie", "刘勰", "劉勰", "Liu Xie"),
    persona("liu-zongyuan", "柳宗元", "柳宗元", "Liu Zongyuan"),
    persona("wen-yiduo", "闻一多", "聞一多", "Wen Yiduo"),
    persona("ji-yun", "纪昀", "紀昀", "Ji Yun"),
    persona("ban-gu", "班固", "班固", "Ban Gu"),
    persona("wang-guowei", "王国维", "王國維", "Wang Guowei"),
    persona("jin-yuelin", "金岳霖", "金岳霖", "Jin Yuelin"),
    persona("feng-youlan", "冯友兰", "馮友蘭", "Feng Youlan"),
    persona("zhang-taiyan", "章太炎", "章太炎", "Zhang Taiyan"),
    persona("yan-yu", "严羽", "嚴羽", "Yan Yu"),
];

#[cfg(feature = "memory-sqlite")]
const WRITER_PERSONAS: &[PersonaSeed] = &[
    persona("qu-yuan", "屈原", "屈原", "Qu Yuan"),
    persona("tao-yuanming", "陶渊明", "陶淵明", "Tao Yuanming"),
    persona("su-shi", "苏轼", "蘇軾", "Su Shi"),
    persona("li-bai", "李白", "李白", "Li Bai"),
    persona("du-fu", "杜甫", "杜甫", "Du Fu"),
    persona("lin-bu", "林逋", "林逋", "Lin Bu"),
    persona("xin-qiji", "辛弃疾", "辛棄疾", "Xin Qiji"),
    persona("wen-tianxiang", "文天祥", "文天祥", "Wen Tianxiang"),
    persona("xu-zhimo", "徐志摩", "徐志摩", "Xu Zhimo"),
    persona("dai-wangshu", "戴望舒", "戴望舒", "Dai Wangshu"),
    persona("yu-guangzhong", "余光中", "余光中", "Yu Guangzhong"),
    persona("haizi", "海子", "海子", "Haizi"),
    persona("cangyang-gyatso", "仓央嘉措", "倉央嘉措", "Cangyang Gyatso"),
    persona("gu-cheng", "顾城", "顧城", "Gu Cheng"),
];

#[cfg(feature = "memory-sqlite")]
const STYLIST_PERSONAS: &[PersonaSeed] = &[
    persona("wang-xizhi", "王羲之", "王羲之", "Wang Xizhi"),
    persona("gu-kaizhi", "顾恺之", "顧愷之", "Gu Kaizhi"),
    persona("wu-daozi", "吴道子", "吳道子", "Wu Daozi"),
    persona("yan-zhenqing", "颜真卿", "顏真卿", "Yan Zhenqing"),
    persona("huaisu", "怀素", "懷素", "Huaisu"),
    persona("zhao-mengfu", "赵孟頫", "趙孟頫", "Zhao Mengfu"),
    persona("mi-fu", "米芾", "米芾", "Mi Fu"),
];

#[cfg(feature = "memory-sqlite")]
const fn persona(
    id: &'static str,
    name_zh_hans: &'static str,
    name_zh_hant: &'static str,
    name_en: &'static str,
) -> PersonaSeed {
    PersonaSeed {
        id,
        name_zh_hans,
        name_zh_hant,
        name_en,
    }
}

#[cfg(feature = "memory-sqlite")]
fn infer_delegate_agent_role(task: Option<&str>, label: Option<&str>) -> DelegateAgentRole {
    let task = task.unwrap_or("");
    let label = label.unwrap_or("");
    let combined = format!("{task} {label}");
    let normalized = combined.to_ascii_lowercase();

    if contains_any_keyword(normalized.as_str(), REVIEWER_KEYWORDS) {
        return DelegateAgentRole::Reviewer;
    }

    if contains_any_keyword(normalized.as_str(), WRITER_KEYWORDS) {
        return DelegateAgentRole::Writer;
    }

    if contains_any_keyword(normalized.as_str(), STRATEGIST_KEYWORDS) {
        return DelegateAgentRole::Strategist;
    }

    if contains_any_keyword(normalized.as_str(), EXPLORER_KEYWORDS) {
        return DelegateAgentRole::Explorer;
    }

    if contains_any_keyword(normalized.as_str(), STYLIST_KEYWORDS) {
        return DelegateAgentRole::Stylist;
    }

    if contains_any_keyword(normalized.as_str(), BUILDER_KEYWORDS) {
        return DelegateAgentRole::Builder;
    }

    DelegateAgentRole::Builder
}

#[cfg(feature = "memory-sqlite")]
fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

#[cfg(feature = "memory-sqlite")]
fn select_role_persona(role: DelegateAgentRole, session_id: &str) -> PersonaSelection {
    let mut personas = Vec::new();
    let builtin_personas = builtin_role_personas(role);

    #[cfg(feature = "config-toml")]
    {
        let override_catalog = load_persona_override_catalog();
        let override_personas = override_catalog.personas_for_role(role);
        let include_builtin = override_catalog.use_builtin || override_personas.is_empty();

        if include_builtin {
            let builtin_choices = builtin_personas.iter().map(builtin_persona_selection);
            personas.extend(builtin_choices);
        }

        personas.extend(override_personas.iter().cloned());
    }

    #[cfg(not(feature = "config-toml"))]
    {
        let builtin_choices = builtin_personas.iter().map(builtin_persona_selection);
        personas.extend(builtin_choices);
    }

    let index = stable_persona_index(session_id, role.as_str(), personas.len());
    if let Some(persona) = personas.into_iter().nth(index) {
        return persona;
    }

    if let Some(fallback_persona) = builtin_personas.first() {
        return builtin_persona_selection(fallback_persona);
    }

    let fallback_id = format!("builtin-{}", role.as_str());
    let fallback_name = role.as_str().to_owned();
    let fallback_names = LocalizedSubagentText {
        zh_hans: fallback_name.clone(),
        zh_hant: fallback_name.clone(),
        en: fallback_name.clone(),
        ja: fallback_name,
    };

    PersonaSelection {
        id: fallback_id,
        names: fallback_names,
    }
}

#[cfg(feature = "memory-sqlite")]
fn builtin_role_personas(role: DelegateAgentRole) -> &'static [PersonaSeed] {
    match role {
        DelegateAgentRole::Explorer => EXPLORER_PERSONAS,
        DelegateAgentRole::Strategist => STRATEGIST_PERSONAS,
        DelegateAgentRole::Builder => BUILDER_PERSONAS,
        DelegateAgentRole::Reviewer => REVIEWER_PERSONAS,
        DelegateAgentRole::Writer => WRITER_PERSONAS,
        DelegateAgentRole::Stylist => STYLIST_PERSONAS,
    }
}

#[cfg(feature = "memory-sqlite")]
fn stable_persona_index(session_id: &str, role_id: &str, pool_len: usize) -> usize {
    if pool_len == 0 {
        return 0;
    }

    let mut hasher = DefaultHasher::new();
    role_id.hash(&mut hasher);
    session_id.hash(&mut hasher);
    let hash = hasher.finish() as usize;
    hash % pool_len
}

#[cfg(feature = "memory-sqlite")]
fn localized_persona_name(persona: &PersonaSeed) -> LocalizedSubagentText {
    LocalizedSubagentText {
        zh_hans: persona.name_zh_hans.to_owned(),
        zh_hant: persona.name_zh_hant.to_owned(),
        en: persona.name_en.to_owned(),
        ja: persona.name_zh_hant.to_owned(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn builtin_persona_selection(persona: &PersonaSeed) -> PersonaSelection {
    let id = persona.id.to_owned();
    let names = localized_persona_name(persona);

    PersonaSelection { id, names }
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn persona_override_use_builtin_default() -> bool {
    true
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn load_persona_override_catalog() -> &'static PersonaOverrideCatalog {
    static CATALOG: OnceLock<PersonaOverrideCatalog> = OnceLock::new();
    CATALOG.get_or_init(load_persona_override_catalog_from_disk)
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn load_persona_override_catalog_from_disk() -> PersonaOverrideCatalog {
    let Some(path) = resolve_persona_override_path() else {
        return PersonaOverrideCatalog::default();
    };

    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return PersonaOverrideCatalog::default(),
    };

    parse_persona_override_catalog(raw.as_str())
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn parse_persona_override_catalog(raw: &str) -> PersonaOverrideCatalog {
    let parsed = match toml::from_str::<PersonaOverrideFile>(raw) {
        Ok(parsed) => parsed,
        Err(_) => return PersonaOverrideCatalog::default(),
    };

    let mut catalog = PersonaOverrideCatalog {
        use_builtin: parsed.use_builtin,
        ..PersonaOverrideCatalog::default()
    };

    for (index, record) in parsed.personas.into_iter().enumerate() {
        let Some(role) = DelegateAgentRole::parse(record.role.as_str()) else {
            continue;
        };
        let Some(persona) = parse_override_persona(role, &record, index) else {
            continue;
        };
        catalog.push(role, persona);
    }

    catalog
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn resolve_persona_override_path() -> Option<PathBuf> {
    let env_override = env::var("LOONGCLAW_SUBAGENTS_PATH")
        .ok()
        .map(|raw| expand_path(raw.as_str()))
        .filter(|path| path.is_file());
    if let Some(path) = env_override {
        return Some(path);
    }

    let cwd_override = env::current_dir()
        .ok()
        .map(|cwd| cwd.join(".loongclaw").join("subagents.toml"))
        .filter(|path| path.is_file());
    if let Some(path) = cwd_override {
        return Some(path);
    }

    let default_home = default_config_path();
    let default_dir = default_home.parent().unwrap_or(Path::new("."));
    let default_path = default_dir.join("subagents.toml");
    default_path.is_file().then_some(default_path)
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn parse_override_persona(
    role: DelegateAgentRole,
    record: &PersonaOverrideRecord,
    index: usize,
) -> Option<PersonaSelection> {
    let zh_hans = trimmed_non_empty(record.zh_hans.as_str())?;
    let zh_hant = record
        .zh_hant
        .as_deref()
        .and_then(trimmed_non_empty)
        .unwrap_or_else(|| zh_hans.clone());
    let en = record
        .en
        .as_deref()
        .and_then(trimmed_non_empty)
        .unwrap_or_else(|| zh_hans.clone());
    let ja = record
        .ja
        .as_deref()
        .and_then(trimmed_non_empty)
        .unwrap_or_else(|| zh_hant.clone());
    let id = record
        .id
        .as_deref()
        .and_then(trimmed_non_empty)
        .unwrap_or_else(|| format!("custom-{}-{}", role.as_str(), index + 1));
    let names = LocalizedSubagentText {
        zh_hans,
        zh_hant,
        en,
        ja,
    };

    Some(PersonaSelection { id, names })
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(all(feature = "memory-sqlite", feature = "config-toml"))]
impl PersonaOverrideCatalog {
    fn personas_for_role(&self, role: DelegateAgentRole) -> &[PersonaSelection] {
        match role {
            DelegateAgentRole::Explorer => self.explorer.as_slice(),
            DelegateAgentRole::Strategist => self.strategist.as_slice(),
            DelegateAgentRole::Builder => self.builder.as_slice(),
            DelegateAgentRole::Reviewer => self.reviewer.as_slice(),
            DelegateAgentRole::Writer => self.writer.as_slice(),
            DelegateAgentRole::Stylist => self.stylist.as_slice(),
        }
    }

    fn push(&mut self, role: DelegateAgentRole, persona: PersonaSelection) {
        let personas = match role {
            DelegateAgentRole::Explorer => &mut self.explorer,
            DelegateAgentRole::Strategist => &mut self.strategist,
            DelegateAgentRole::Builder => &mut self.builder,
            DelegateAgentRole::Reviewer => &mut self.reviewer,
            DelegateAgentRole::Writer => &mut self.writer,
            DelegateAgentRole::Stylist => &mut self.stylist,
        };
        personas.push(persona);
    }
}

#[cfg(feature = "memory-sqlite")]
fn localized_role_label(role: DelegateAgentRole) -> LocalizedSubagentText {
    match role {
        DelegateAgentRole::Explorer => LocalizedSubagentText {
            zh_hans: "行者".to_owned(),
            zh_hant: "行者".to_owned(),
            en: "Explorer".to_owned(),
            ja: "探索者".to_owned(),
        },
        DelegateAgentRole::Strategist => LocalizedSubagentText {
            zh_hans: "策士".to_owned(),
            zh_hant: "策士".to_owned(),
            en: "Strategist".to_owned(),
            ja: "策士".to_owned(),
        },
        DelegateAgentRole::Builder => LocalizedSubagentText {
            zh_hans: "匠人".to_owned(),
            zh_hant: "匠人".to_owned(),
            en: "Builder".to_owned(),
            ja: "実装者".to_owned(),
        },
        DelegateAgentRole::Reviewer => LocalizedSubagentText {
            zh_hans: "诤友".to_owned(),
            zh_hant: "諍友".to_owned(),
            en: "Reviewer".to_owned(),
            ja: "監修".to_owned(),
        },
        DelegateAgentRole::Writer => LocalizedSubagentText {
            zh_hans: "文士".to_owned(),
            zh_hant: "文士".to_owned(),
            en: "Writer".to_owned(),
            ja: "文筆家".to_owned(),
        },
        DelegateAgentRole::Stylist => LocalizedSubagentText {
            zh_hans: "绘手".to_owned(),
            zh_hant: "繪手".to_owned(),
            en: "Stylist".to_owned(),
            ja: "意匠".to_owned(),
        },
    }
}

#[cfg(feature = "memory-sqlite")]
const EXPLORER_KEYWORDS: &[&str] = &[
    "research",
    "investigate",
    "inspect",
    "search",
    "explore",
    "trace",
    "survey",
    "discover",
    "reference",
    "compare",
    "study",
    "look into",
    "find",
    "研究",
    "调研",
    "分析",
    "对比",
    "查找",
    "搜索",
    "探索",
    "勘察",
    "追踪",
    "深挖",
];

#[cfg(feature = "memory-sqlite")]
const STRATEGIST_KEYWORDS: &[&str] = &[
    "plan",
    "design",
    "architect",
    "architecture",
    "roadmap",
    "strategy",
    "modeling",
    "structure",
    "onboard",
    "workflow",
    "规划",
    "设计",
    "架构",
    "方案",
    "路线",
    "流程",
    "重构方案",
];

#[cfg(feature = "memory-sqlite")]
const BUILDER_KEYWORDS: &[&str] = &[
    "implement",
    "build",
    "fix",
    "wire",
    "integrate",
    "patch",
    "support",
    "refactor",
    "optimize",
    "code",
    "实现",
    "修复",
    "接入",
    "联调",
    "优化",
    "重构",
    "补丁",
];

#[cfg(feature = "memory-sqlite")]
const REVIEWER_KEYWORDS: &[&str] = &[
    "review",
    "verify",
    "validation",
    "validate",
    "audit",
    "test",
    "regression",
    "qa",
    "check",
    "inspection",
    "审查",
    "校验",
    "验证",
    "测试",
    "回归",
    "检查",
    "审计",
];

#[cfg(feature = "memory-sqlite")]
const WRITER_KEYWORDS: &[&str] = &[
    "write",
    "doc",
    "docs",
    "document",
    "explain",
    "summary",
    "summarize",
    "migration",
    "copy",
    "issue",
    "pr",
    "release notes",
    "文档",
    "说明",
    "解释",
    "总结",
    "概述",
    "迁移",
    "文案",
    "issue",
    "pr",
];

#[cfg(feature = "memory-sqlite")]
const STYLIST_KEYWORDS: &[&str] = &[
    "ui",
    "ux",
    "visual",
    "theme",
    "style",
    "palette",
    "layout",
    "motion",
    "animation",
    "render",
    "diff view",
    "design language",
    "界面",
    "视觉",
    "配色",
    "布局",
    "动效",
    "渲染",
    "样式",
    "设计语言",
];

#[cfg(all(test, feature = "memory-sqlite"))]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::session::repository::{SessionEventRecord, SessionState, SessionSummaryRecord};

    fn sample_delegate_session() -> SessionSummaryRecord {
        SessionSummaryRecord {
            session_id: "delegate:test-1".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root".to_owned()),
            label: Some("research tui polish".to_owned()),
            state: SessionState::Running,
            created_at: 1,
            updated_at: 2,
            archived_at: None,
            turn_count: 0,
            last_turn_at: None,
            last_error: None,
        }
    }

    #[test]
    fn locale_parser_supports_hans_hant_en_and_ja() {
        assert_eq!(
            SessionPresentationLocale::from_tag("zh-CN.UTF-8"),
            SessionPresentationLocale::ZhHans
        );
        assert_eq!(
            SessionPresentationLocale::from_tag("zh-Hant"),
            SessionPresentationLocale::ZhHant
        );
        assert_eq!(
            SessionPresentationLocale::from_tag("ja_JP.UTF-8"),
            SessionPresentationLocale::Ja
        );
        assert_eq!(
            SessionPresentationLocale::from_tag("en_US.UTF-8"),
            SessionPresentationLocale::En
        );
    }

    #[test]
    fn delegate_agent_presentation_uses_spawn_task_and_provider_snapshot() {
        let session = sample_delegate_session();
        let events = vec![SessionEventRecord {
            id: 1,
            session_id: session.session_id.clone(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root".to_owned()),
            payload_json: json!({
                "task": "research reference implementations",
                "label": "reference-study",
                "provider": {
                    "profile_id": "openai-reasoning",
                    "provider_kind": "openai",
                    "model": "gpt-5",
                    "reasoning_effort": "high"
                }
            }),
            ts: 10,
        }];

        let presentation =
            derive_delegate_agent_presentation(&session, events.as_slice()).expect("presentation");

        assert_eq!(presentation.role_id, "explorer");
        assert_eq!(presentation.model.as_deref(), Some("gpt-5"));
        assert_eq!(presentation.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn delegate_agent_provider_label_localizes_reasoning_prefix() {
        let presentation = DelegateAgentPresentation {
            persona_id: "xu-xiake".to_owned(),
            role_id: "explorer".to_owned(),
            names: LocalizedSubagentText {
                zh_hans: "徐霞客".to_owned(),
                zh_hant: "徐霞客".to_owned(),
                en: "Xu Xiake".to_owned(),
                ja: "徐霞客".to_owned(),
            },
            roles: LocalizedSubagentText {
                zh_hans: "行者".to_owned(),
                zh_hant: "行者".to_owned(),
                en: "Explorer".to_owned(),
                ja: "探索役".to_owned(),
            },
            model: Some("gpt-5".to_owned()),
            reasoning_effort: Some("high".to_owned()),
        };

        assert_eq!(
            presentation.provider_label(SessionPresentationLocale::En),
            Some("gpt-5 · high".to_owned())
        );
        assert_eq!(
            presentation.provider_label(SessionPresentationLocale::ZhHans),
            Some("gpt-5 · high".to_owned())
        );
    }

    #[cfg(feature = "config-toml")]
    #[test]
    fn parse_override_catalog_accepts_custom_persona_pool() {
        let raw = r#"
use_builtin = false

[[personas]]
role = "builder"
zh_hans = "河图"
zh_hant = "河圖"
en = "Hetu"
"#;

        let catalog = parse_persona_override_catalog(raw);
        let builder_personas = catalog.personas_for_role(DelegateAgentRole::Builder);
        let persona = builder_personas.first().expect("custom builder persona");

        assert!(!catalog.use_builtin);
        assert_eq!(persona.id, "custom-builder-1");
        assert_eq!(persona.names.zh_hans, "河图");
        assert_eq!(persona.names.zh_hant, "河圖");
        assert_eq!(persona.names.en, "Hetu");
        assert_eq!(persona.names.ja, "河圖");
    }
}
