use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::OnceLock;

use loong_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;
use serde_json::json;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

use super::catalog::{ToolDescriptor, ToolView};
use super::runtime_config;
use super::{
    LOONG_INTERNAL_TOOL_SEARCH_KEY, LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY,
    TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD, canonical_tool_name, issue_tool_lease, memory_tools,
};

const COARSE_FALLBACK_DISCOVERY_CONCEPTS: &[&str] =
    &["fetch", "inspect", "list", "read", "search", "status"];
const MAX_SEARCH_WHY_REASONS: usize = 4;

#[derive(Debug, Clone)]
pub(super) struct SearchableToolEntry {
    pub(super) canonical_name: String,
    pub(super) summary: String,
    pub(super) search_hint: String,
    pub(super) argument_hint: String,
    pub(super) required_fields: Vec<String>,
    pub(super) required_field_groups: Vec<Vec<String>>,
    pub(super) schema_preview: Value,
    pub(super) tags: Vec<String>,
    search_document: SearchDocument,
}

#[derive(Debug, Clone)]
pub(super) struct RankedSearchableToolEntry {
    pub(super) entry: SearchableToolEntry,
    pub(super) why: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ToolSearchRanking {
    pub(super) results: Vec<RankedSearchableToolEntry>,
    pub(super) diagnostics_reason: Option<&'static str>,
}

pub(super) fn execute_tool_search_tool_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "tool.search payload must be an object".to_owned())?;
    let query = tool_search_query_from_payload(payload).map(Cow::into_owned);
    let requested_exact_tool_id = payload
        .get("exact_tool_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let exact_tool_id = requested_exact_tool_id
        .as_deref()
        .map(canonical_tool_name)
        .map(str::to_owned);

    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 8) as usize)
        .unwrap_or(5);
    let granted_capabilities = payload
        .get(TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD)
        .cloned()
        .and_then(|value| serde_json::from_value::<BTreeSet<Capability>>(value).ok());
    let visible_tool_view = search_tool_view_from_payload(payload, config);

    let searchable_entries =
        super::runtime_discoverable_tool_entries(config, Some(&visible_tool_view))
            .into_iter()
            .filter(|entry| {
                tool_search_entry_is_capability_usable(
                    entry.canonical_name.as_str(),
                    granted_capabilities.as_ref(),
                )
            })
            .collect::<Vec<_>>();
    let exact_match_entry = exact_tool_id.as_ref().and_then(|exact_tool_id| {
        searchable_entries
            .iter()
            .find(|entry| entry.canonical_name == *exact_tool_id)
            .cloned()
    });
    let exact_match_found = exact_match_entry.is_some();
    let mut diagnostics_reason = None;
    let results: Vec<Value> = if let Some(entry) = exact_match_entry {
        let why = Vec::new();
        let entry_json = tool_search_result_entry_json(&entry, why, payload)?;
        vec![entry_json]
    } else if let Some(query) = query.as_deref() {
        let ranking = rank_searchable_entries(searchable_entries, query, limit);
        diagnostics_reason = ranking.diagnostics_reason;

        ranking
            .results
            .into_iter()
            .map(|ranked_entry| {
                let RankedSearchableToolEntry { entry, why } = ranked_entry;

                tool_search_result_entry_json(&entry, why, payload)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let ranking = rank_searchable_entries(searchable_entries, "", limit);
        diagnostics_reason = ranking.diagnostics_reason;

        ranking
            .results
            .into_iter()
            .map(|ranked_entry| {
                let RankedSearchableToolEntry { entry, why } = ranked_entry;

                tool_search_result_entry_json(&entry, why, payload)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let diagnostics = tool_search_diagnostics_json(
        requested_exact_tool_id.as_deref(),
        exact_match_found,
        query.as_deref(),
        diagnostics_reason,
    );
    let response_exact_tool_id = if exact_match_found {
        exact_tool_id
    } else {
        requested_exact_tool_id
    };

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "query": query,
            "exact_tool_id": response_exact_tool_id,
            "returned": results.len(),
            "results": results,
            "diagnostics": diagnostics,
        }),
    })
}

fn tool_search_result_entry_json(
    entry: &SearchableToolEntry,
    why: Vec<String>,
    payload: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let lease = issue_tool_lease(entry.canonical_name.as_str(), payload)?;
    Ok(json!({
        "tool_id": entry.canonical_name,
        "summary": entry.summary,
        "search_hint": entry.search_hint,
        "argument_hint": entry.argument_hint,
        "required_fields": entry.required_fields,
        "required_field_groups": entry.required_field_groups,
        "schema_preview": entry.schema_preview,
        "tags": entry.tags,
        "why": why,
        "lease": lease,
    }))
}

fn tool_search_diagnostics_json(
    requested_exact_tool_id: Option<&str>,
    exact_match_found: bool,
    query: Option<&str>,
    diagnostics_reason: Option<&str>,
) -> Value {
    if let Some(requested_exact_tool_id) = requested_exact_tool_id {
        if exact_match_found {
            return Value::Null;
        }

        return json!({
            "reason": "exact_tool_id_not_visible",
            "requested_tool_id": requested_exact_tool_id,
        });
    }

    if let Some(reason) = diagnostics_reason {
        let diagnostics_query = query.unwrap_or_default();

        return json!({
            "reason": reason,
            "query": diagnostics_query,
        });
    }

    Value::Null
}

fn tool_search_query_from_payload(
    payload: &serde_json::Map<String, Value>,
) -> Option<Cow<'_, str>> {
    const QUERY_KEYS: &[&str] = &["query", "input", "text", "prompt", "keyword", "keywords"];

    for key in QUERY_KEYS {
        let Some(value) = payload.get(*key) else {
            continue;
        };

        if let Some(query) = tool_search_query_from_value(value) {
            return Some(query);
        }
    }

    None
}

fn tool_search_query_from_value(value: &Value) -> Option<Cow<'_, str>> {
    let string_value = value.as_str();
    if let Some(string_value) = string_value {
        let trimmed_value = string_value.trim();
        if !trimmed_value.is_empty() {
            return Some(Cow::Borrowed(trimmed_value));
        }
    }

    let values = value.as_array()?;
    let joined_value = join_tool_search_query_values(values);
    if joined_value.is_empty() {
        return None;
    }

    Some(Cow::Owned(joined_value))
}

fn join_tool_search_query_values(values: &[Value]) -> String {
    let mut query_parts = Vec::new();

    for value in values {
        let query_part = tool_search_query_part(value);
        if query_part.is_empty() {
            continue;
        }

        query_parts.push(query_part);
    }

    query_parts.join(" ")
}

fn tool_search_query_part(value: &Value) -> String {
    let string_value = value.as_str();
    if let Some(string_value) = string_value {
        return string_value.trim().to_owned();
    }

    value.to_string()
}

pub(super) fn tool_search_entry_is_runtime_usable(
    tool_name: &str,
    config: &runtime_config::ToolRuntimeConfig,
) -> bool {
    match tool_name {
        "shell.exec" => {
            !config.shell_allow.is_empty()
                || matches!(
                    config.shell_default_mode,
                    crate::tools::shell_policy_ext::ShellPolicyDefault::Allow
                )
        }
        "bash.exec" => config.bash_exec.is_discoverable(),
        "external_skills.fetch"
        | "external_skills.install"
        | "external_skills.inspect"
        | "external_skills.invoke"
        | "external_skills.list"
        | "external_skills.remove" => config.external_skills.enabled,
        #[cfg(feature = "tool-file")]
        "memory_search" => memory_tools::memory_corpus_available(config),
        #[cfg(feature = "tool-file")]
        "memory_get" => memory_tools::workspace_memory_corpus_available(config),
        _ => true,
    }
}

pub(super) fn tool_search_entry_is_capability_usable(
    tool_name: &str,
    granted_capabilities: Option<&BTreeSet<Capability>>,
) -> bool {
    let Some(granted_capabilities) = granted_capabilities else {
        return true;
    };
    let required = super::required_capabilities_for_tool_name_and_payload(tool_name, &json!({}));
    required
        .iter()
        .all(|capability| granted_capabilities.contains(capability))
}

pub(super) fn search_tool_view_from_payload(
    payload: &serde_json::Map<String, Value>,
    config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    let payload_value = Value::Object(payload.clone());
    let visible_tool_names = if super::trusted_internal_tool_payload_enabled() {
        super::trusted_internal_tool_context_from_payload(&payload_value)
            .and_then(|body| body.get(LOONG_INTERNAL_TOOL_SEARCH_KEY))
            .and_then(|body| body.get(LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY))
            .and_then(Value::as_array)
            .map(|tool_names| {
                tool_names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(canonical_tool_name)
                    .collect::<Vec<_>>()
            })
    } else {
        None
    };

    match visible_tool_names {
        Some(visible_tool_names) => ToolView::from_tool_names(visible_tool_names),
        None => super::full_runtime_tool_view_for_runtime_config(config),
    }
}

#[derive(Debug, Clone)]
struct SearchSignalSet {
    normalized_text: String,
    tokens: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SearchDocument {
    name: SearchSignalSet,
    summary: SearchSignalSet,
    arguments: SearchSignalSet,
    schema: SearchSignalSet,
    tags: SearchSignalSet,
    concepts: BTreeSet<String>,
    categories: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SearchQuery {
    signal: SearchSignalSet,
    concepts: BTreeSet<String>,
    categories: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SearchScore {
    score: u32,
    why: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScoredSearchableToolEntry {
    entry: SearchableToolEntry,
    score: u32,
    why: Vec<String>,
}

#[derive(Debug, Clone)]
struct SchemaArgumentField {
    name: String,
    schema_type: String,
    required: bool,
    preferred_index: usize,
}

struct SearchConceptDefinition {
    id: &'static str,
    categories: &'static [&'static str],
    forms: &'static [&'static str],
}

struct NormalizedSearchConcept {
    id: &'static str,
    categories: &'static [&'static str],
    forms: Vec<String>,
}

impl SchemaArgumentField {
    fn format(self) -> String {
        let suffix = if self.required { "" } else { "?" };
        format!("{}{}:{}", self.name, suffix, self.schema_type)
    }
}

const SEARCH_CONCEPT_DEFINITIONS: &[SearchConceptDefinition] = &[
    SearchConceptDefinition {
        id: "search",
        categories: &["discovery"],
        forms: &[
            "search",
            "find",
            "discover",
            "lookup",
            "query",
            "buscar",
            "busqueda",
            "rechercher",
            "recherche",
            "suchen",
            "suche",
            "искать",
            "поиск",
            "найти",
            "بحث",
            "ابحث",
            "खोज",
            "तलाश",
            "搜索",
            "搜寻",
            "查找",
            "查询",
            "检索",
            "検索",
            "探す",
            "調べる",
            "검색",
            "찾기",
            "조회",
        ],
    },
    SearchConceptDefinition {
        id: "fetch",
        categories: &["network", "retrieval"],
        forms: &[
            "fetch",
            "download",
            "retrieve",
            "obtain",
            "descargar",
            "telecharger",
            "скачать",
            "получить",
            "جلب",
            "تنزيل",
            "डाउनलोड",
            "获取",
            "抓取",
            "拉取",
            "取得",
            "取得する",
            "가져오기",
            "다운로드",
        ],
    },
    SearchConceptDefinition {
        id: "read",
        categories: &["retrieval"],
        forms: &[
            "read",
            "view",
            "show",
            "display",
            "open",
            "leer",
            "lire",
            "anzeigen",
            "читать",
            "открыть",
            "قراءة",
            "عرض",
            "पढ़",
            "देख",
            "खोल",
            "读取",
            "阅读",
            "查看",
            "打开",
            "読む",
            "表示",
            "開く",
            "읽기",
            "보기",
            "열기",
        ],
    },
    SearchConceptDefinition {
        id: "inspect",
        categories: &["discovery", "retrieval"],
        forms: &[
            "inspect",
            "detail",
            "details",
            "metadata",
            "describe",
            "detalle",
            "detalles",
            "details",
            "metadonnees",
            "метаданные",
            "подробности",
            "تفاصيل",
            "بيانات وصفية",
            "विवरण",
            "मेटाडेटा",
            "详情",
            "元数据",
            "详细",
            "詳細",
            "メタデータ",
            "상세",
            "메타데이터",
        ],
    },
    SearchConceptDefinition {
        id: "list",
        categories: &["discovery"],
        forms: &[
            "list",
            "enumerate",
            "browse",
            "listar",
            "liste",
            "zeigen",
            "список",
            "показать",
            "قائمة",
            "اعرض",
            "सूची",
            "दिखा",
            "列表",
            "列出",
            "浏览",
            "一覧",
            "参照",
            "목록",
            "나열",
        ],
    },
    SearchConceptDefinition {
        id: "write",
        categories: &["mutation"],
        forms: &[
            "write",
            "create",
            "save",
            "append",
            "record",
            "escribir",
            "crear",
            "guardar",
            "ecrire",
            "creer",
            "enregistrer",
            "создать",
            "записать",
            "сохранить",
            "كتابة",
            "إنشاء",
            "احفظ",
            "लिख",
            "बन",
            "सहेज",
            "写入",
            "创建",
            "保存",
            "追加",
            "作成",
            "保存する",
            "쓰기",
            "저장",
            "생성",
        ],
    },
    SearchConceptDefinition {
        id: "edit",
        categories: &["mutation"],
        forms: &[
            "edit",
            "modify",
            "update",
            "replace",
            "patch",
            "editar",
            "modifier",
            "mettre a jour",
            "изменить",
            "обновить",
            "заменить",
            "تحرير",
            "تعديل",
            "تحديث",
            "संपादित",
            "बदल",
            "अपडेट",
            "编辑",
            "修改",
            "更新",
            "替换",
            "編集",
            "修正",
            "変更",
            "편집",
            "수정",
            "업데이트",
        ],
    },
    SearchConceptDefinition {
        id: "file",
        categories: &["workspace"],
        forms: &[
            "file",
            "files",
            "filesystem",
            "path",
            "repo",
            "repository",
            "workspace",
            "archivo",
            "fichier",
            "datei",
            "файл",
            "путь",
            "репозиторий",
            "ملف",
            "مسار",
            "مستودع",
            "फ़ाइल",
            "पथ",
            "रिपॉजिटरी",
            "वर्कस्पेस",
            "文件",
            "档案",
            "路径",
            "仓库",
            "工作区",
            "ファイル",
            "パス",
            "リポジトリ",
            "ワークスペース",
            "파일",
            "경로",
            "저장소",
            "작업공간",
        ],
    },
    SearchConceptDefinition {
        id: "directory",
        categories: &["workspace", "discovery"],
        forms: &[
            "directory",
            "directories",
            "folder",
            "folders",
            "current directory",
            "current folder",
            "dir",
            "directory tree",
            "folder tree",
            "carpeta",
            "repertoire",
            "dossier",
            "ordner",
            "verzeichnis",
            "папка",
            "каталог",
            "текущая папка",
            "دليل",
            "مجلد",
            "المجلد الحالي",
            "फ़ोल्डर",
            "डायरेक्टरी",
            "वर्तमान फ़ोल्डर",
            "目录",
            "目录树",
            "文件夹",
            "当前目录",
            "当前文件夹",
            "フォルダ",
            "ディレクトリ",
            "カレントディレクトリ",
            "폴더",
            "디렉터리",
            "현재 디렉터리",
        ],
    },
    SearchConceptDefinition {
        id: "memory",
        categories: &["workspace"],
        forms: &[
            "memory",
            "note",
            "notes",
            "memo",
            "recall",
            "durable",
            "knowledge",
            "memoria",
            "memoire",
            "заметки",
            "память",
            "ملاحظات",
            "ذاكرة",
            "नोट",
            "स्मृति",
            "याद",
            "记忆",
            "笔记",
            "备忘",
            "回忆",
            "メモ",
            "記憶",
            "ノート",
            "메모",
            "기억",
        ],
    },
    SearchConceptDefinition {
        id: "web",
        categories: &["network"],
        forms: &[
            "web",
            "website",
            "site",
            "url",
            "http",
            "https",
            "internet",
            "page",
            "pagina web",
            "site web",
            "страница",
            "веб",
            "сайт",
            "ويب",
            "موقع",
            "صفحة",
            "वेब",
            "साइट",
            "पेज",
            "网页",
            "页面",
            "网址",
            "网络",
            "ウェブ",
            "サイト",
            "ページ",
            "웹",
            "사이트",
            "웹페이지",
        ],
    },
    SearchConceptDefinition {
        id: "browser",
        categories: &["interactive", "network"],
        forms: &[
            "browser",
            "browse",
            "navigate",
            "click",
            "type",
            "selector",
            "tab",
            "navegador",
            "naviguer",
            "браузер",
            "клик",
            "переход",
            "متصفح",
            "انقر",
            "اكتب",
            "ब्राउज़र",
            "क्लिक",
            "टाइप",
            "浏览器",
            "导航",
            "点击",
            "输入",
            "ブラウザ",
            "クリック",
            "入力",
            "브라우저",
            "클릭",
            "입력",
            "탐색",
        ],
    },
    SearchConceptDefinition {
        id: "session",
        categories: &["coordination"],
        forms: &[
            "session",
            "thread",
            "conversation",
            "chat",
            "history",
            "event",
            "status",
            "queue",
            "session_id",
            "sesion",
            "conversation",
            "histoire",
            "сессия",
            "чат",
            "история",
            "событие",
            "статус",
            "جلسة",
            "محادثة",
            "سجل",
            "حالة",
            "सत्र",
            "चैट",
            "इतिहास",
            "स्थिति",
            "会话",
            "对话",
            "聊天",
            "历史",
            "事件",
            "状态",
            "セッション",
            "会話",
            "履歴",
            "状態",
            "세션",
            "대화",
            "이력",
            "상태",
        ],
    },
    SearchConceptDefinition {
        id: "message",
        categories: &["communication"],
        forms: &[
            "message",
            "messages",
            "send",
            "post",
            "reply",
            "mensaje",
            "enviar",
            "reponse",
            "envoyer",
            "сообщение",
            "отправить",
            "ответить",
            "رسالة",
            "إرسال",
            "رد",
            "संदेश",
            "भेज",
            "जवाब",
            "消息",
            "发送",
            "回复",
            "メッセージ",
            "送信",
            "返信",
            "메시지",
            "보내기",
            "답장",
        ],
    },
    SearchConceptDefinition {
        id: "delegate",
        categories: &["coordination"],
        forms: &[
            "delegate",
            "delegation",
            "child",
            "background",
            "async",
            "subtask",
            "delegar",
            "deleguer",
            "делегировать",
            "фоновый",
            "تفويض",
            "خلفية",
            "उपकार्य",
            "पृष्ठभूमि",
            "委派",
            "后台",
            "子任务",
            "委任",
            "バックグラウンド",
            "子タスク",
            "비동기",
            "위임",
            "하위 작업",
        ],
    },
    SearchConceptDefinition {
        id: "skill",
        categories: &["extension"],
        forms: &[
            "skill",
            "skills",
            "plugin",
            "extension",
            "package",
            "skillset",
            "plugin",
            "extension",
            "плагин",
            "расширение",
            "مهارة",
            "ملحق",
            "إضافة",
            "कौशल",
            "प्लगइन",
            "एक्सटेंशन",
            "技能",
            "插件",
            "扩展",
            "包",
            "スキル",
            "プラグイン",
            "拡張",
            "패키지",
            "플러그인",
            "확장",
        ],
    },
    SearchConceptDefinition {
        id: "install",
        categories: &["extension", "mutation"],
        forms: &[
            "install",
            "setup",
            "enable",
            "configure",
            "instalar",
            "installer",
            "einrichten",
            "установить",
            "настроить",
            "включить",
            "تثبيت",
            "إعداد",
            "تمكين",
            "स्थापित",
            "इंस्टॉल",
            "सक्षम",
            "सेटअप",
            "安装",
            "启用",
            "配置",
            "インストール",
            "セットアップ",
            "有効",
            "설치",
            "설정",
            "활성화",
        ],
    },
    SearchConceptDefinition {
        id: "remove",
        categories: &["extension", "mutation"],
        forms: &[
            "remove",
            "delete",
            "uninstall",
            "disable",
            "erase",
            "quitar",
            "supprimer",
            "удалить",
            "деинсталлировать",
            "выключить",
            "إزالة",
            "حذف",
            "تعطيل",
            "हट",
            "मिटा",
            "अनइंस्टॉल",
            "删除",
            "移除",
            "卸载",
            "禁用",
            "削除",
            "アンインストール",
            "無効",
            "제거",
            "삭제",
            "비활성화",
        ],
    },
    SearchConceptDefinition {
        id: "provider",
        categories: &["runtime"],
        forms: &[
            "provider",
            "model",
            "runtime",
            "profile",
            "engine",
            "backend",
            "proveedor",
            "fournisseur",
            "провайдер",
            "модель",
            "рантайм",
            "مزود",
            "نموذج",
            "وقت التشغيل",
            "प्रदाता",
            "मॉडल",
            "रनटाइम",
            "प्रोफाइल",
            "供应商",
            "模型",
            "运行时",
            "配置档",
            "プロバイダ",
            "モデル",
            "ランタイム",
            "프로바이더",
            "모델",
            "런타임",
        ],
    },
    SearchConceptDefinition {
        id: "switch",
        categories: &["mutation", "runtime"],
        forms: &[
            "switch",
            "change",
            "select",
            "swap",
            "choose",
            "toggle",
            "cambiar",
            "changer",
            "wechseln",
            "переключить",
            "сменить",
            "выбрать",
            "تبديل",
            "تغيير",
            "اختر",
            "बदल",
            "बदलें",
            "चुन",
            "स्विच",
            "切换",
            "更换",
            "选择",
            "切り替え",
            "変更",
            "選択",
            "전환",
            "변경",
            "선택",
        ],
    },
    SearchConceptDefinition {
        id: "approval",
        categories: &["governance"],
        forms: &[
            "approval",
            "approve",
            "permission",
            "policy",
            "security",
            "allow",
            "deny",
            "aprobacion",
            "autorisation",
            "politique",
            "одобрение",
            "разрешение",
            "политика",
            "безопасность",
            "موافقة",
            "إذن",
            "سياسة",
            "أمان",
            "अनुमति",
            "स्वीकृति",
            "नीति",
            "सुरक्षा",
            "审批",
            "批准",
            "权限",
            "策略",
            "安全",
            "承認",
            "許可",
            "ポリシー",
            "セキュリティ",
            "승인",
            "권한",
            "정책",
            "보안",
        ],
    },
    SearchConceptDefinition {
        id: "wait",
        categories: &["coordination"],
        forms: &[
            "wait",
            "poll",
            "watch",
            "monitor",
            "until",
            "esperar",
            "sonder",
            "ждать",
            "следить",
            "انتظر",
            "مراقبة",
            "प्रतीक्षा",
            "निगरानी",
            "等待",
            "轮询",
            "监控",
            "待つ",
            "監視",
            "대기",
            "감시",
        ],
    },
    SearchConceptDefinition {
        id: "cancel",
        categories: &["coordination", "mutation"],
        forms: &[
            "cancel",
            "stop",
            "abort",
            "kill",
            "terminate",
            "cancelar",
            "arreter",
            "остановить",
            "отменить",
            "прервать",
            "إلغاء",
            "إيقاف",
            "إنهاء",
            "रोक",
            "रद्द",
            "समाप्त",
            "取消",
            "停止",
            "终止",
            "キャンセル",
            "停止する",
            "취소",
            "중지",
        ],
    },
    SearchConceptDefinition {
        id: "archive",
        categories: &["coordination"],
        forms: &[
            "archive",
            "store",
            "retain",
            "archivar",
            "archiver",
            "архив",
            "архивировать",
            "أرشفة",
            "حفظ",
            "संग्रह",
            "अभिलेख",
            "归档",
            "存档",
            "アーカイブ",
            "보관",
        ],
    },
    SearchConceptDefinition {
        id: "recover",
        categories: &["coordination"],
        forms: &[
            "recover",
            "restore",
            "resume",
            "repair",
            "fix",
            "recuperar",
            "restaurer",
            "восстановить",
            "починить",
            "استعادة",
            "إصلاح",
            "पुनर्प्राप्त",
            "बहाल",
            "मरम्मत",
            "恢复",
            "还原",
            "復旧",
            "復元",
            "복구",
        ],
    },
];

impl SearchSignalSet {
    fn from_fragments(fragments: &[String]) -> Self {
        let mut normalized_fragments = Vec::new();
        let mut tokens = BTreeSet::new();

        for fragment in fragments {
            let normalized_fragment = normalize_search_text(fragment);
            if normalized_fragment.is_empty() {
                continue;
            }

            let fragment_tokens = tokenize_normalized_text(normalized_fragment.as_str());
            tokens.extend(fragment_tokens);
            normalized_fragments.push(normalized_fragment);
        }

        let normalized_text = normalized_fragments.join(" ");

        Self {
            normalized_text,
            tokens,
        }
    }

    fn contains_term(&self, normalized_term: &str) -> bool {
        if normalized_term.is_empty() {
            return false;
        }

        let ascii_token_only = normalized_term
            .chars()
            .all(|character| character.is_ascii_alphanumeric());

        if ascii_token_only {
            return self.tokens.contains(normalized_term);
        }

        if self.tokens.contains(normalized_term) {
            return true;
        }

        self.normalized_text.contains(normalized_term)
    }
}

impl SearchDocument {
    fn new(
        name_fragments: Vec<String>,
        summary_fragments: Vec<String>,
        argument_fragments: Vec<String>,
        schema_fragments: Vec<String>,
        tag_fragments: Vec<String>,
    ) -> Self {
        let name = SearchSignalSet::from_fragments(&name_fragments);
        let summary = SearchSignalSet::from_fragments(&summary_fragments);
        let arguments = SearchSignalSet::from_fragments(&argument_fragments);
        let schema = SearchSignalSet::from_fragments(&schema_fragments);
        let tags = SearchSignalSet::from_fragments(&tag_fragments);

        let mut all_fragments = Vec::new();
        all_fragments.extend(name_fragments);
        all_fragments.extend(summary_fragments);
        all_fragments.extend(argument_fragments);
        all_fragments.extend(schema_fragments);
        all_fragments.extend(tag_fragments);

        let all_signals = SearchSignalSet::from_fragments(&all_fragments);
        let (concepts, categories) = extract_concepts_and_categories(&all_signals);

        Self {
            name,
            summary,
            arguments,
            schema,
            tags,
            concepts,
            categories,
        }
    }
}

impl SearchQuery {
    fn new(raw_query: &str) -> Self {
        let signal = search_query_signal_set(raw_query);
        let (mut concepts, mut categories) = extract_concepts_and_categories(&signal);
        apply_structural_query_hints(raw_query, &mut concepts, &mut categories);

        Self {
            signal,
            concepts,
            categories,
        }
    }
}

fn search_query_signal_set(raw_query: &str) -> SearchSignalSet {
    let cleaned_tokens = raw_query
        .split_whitespace()
        .map(trim_structural_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut fragments = Vec::new();

    for token in cleaned_tokens {
        let has_path_separator = token.contains('/') || token.contains('\\');
        let host_like_path_token = has_path_separator && token_has_host_like_prefix(token);
        let url_like_token = token_looks_like_url(token);
        let skip_token = url_like_token || host_like_path_token;

        if skip_token {
            continue;
        }

        fragments.push(token.to_owned());
    }

    SearchSignalSet::from_fragments(&fragments)
}

pub(super) fn searchable_entry_from_descriptor(descriptor: &ToolDescriptor) -> SearchableToolEntry {
    let definition = descriptor.provider_definition();
    let function = definition.get("function");

    let summary_value = function.and_then(|value| value.get("description"));
    let summary = summary_value
        .and_then(Value::as_str)
        .unwrap_or(descriptor.description)
        .to_owned();

    let parameters_value = function.and_then(|value| value.get("parameters"));
    let parameters = parameters_value.unwrap_or(&Value::Null);
    let tags = descriptor
        .tags()
        .iter()
        .map(|tag| (*tag).to_owned())
        .collect::<Vec<_>>();
    let search_hint = descriptor.search_hint().to_owned();

    searchable_entry_from_provider_definition(
        descriptor.name,
        descriptor.provider_name,
        descriptor.aliases,
        summary,
        search_hint,
        parameters,
        descriptor.parameter_types(),
        tags,
    )
}

pub(super) fn searchable_entry_from_provider_definition(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
    summary: String,
    search_hint: String,
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
    tags: Vec<String>,
) -> SearchableToolEntry {
    let required_fields = schema_required_fields(parameters);
    let required_field_groups = schema_required_field_groups(parameters);
    let required_field_groups =
        default_required_field_groups(&required_fields, required_field_groups);
    let argument_hint =
        search_argument_hint_from_provider_definition(parameters, preferred_parameter_order);
    let schema_preview = build_schema_preview(&required_fields, &required_field_groups, parameters);

    let name_fragments = build_name_fragments(canonical_name, provider_name, aliases);
    let mut summary_fragments = vec![summary.clone()];

    if search_hint.trim() != summary.trim() {
        summary_fragments.push(search_hint.clone());
    }

    let argument_fragments = build_argument_fragments(
        argument_hint.as_str(),
        &required_fields,
        &required_field_groups,
    );
    let schema_fragments = collect_schema_search_terms(parameters);
    let tag_fragments = tags.clone();
    let search_document = SearchDocument::new(
        name_fragments,
        summary_fragments,
        argument_fragments,
        schema_fragments,
        tag_fragments,
    );

    SearchableToolEntry {
        canonical_name: canonical_name.to_owned(),
        summary,
        search_hint,
        argument_hint,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        search_document,
    }
}

fn build_schema_preview(
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
    parameters: &Value,
) -> Value {
    let properties = parameters
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut required_field_names = BTreeSet::new();

    for required_field in required_fields {
        required_field_names.insert(required_field.clone());
    }

    for group in required_field_groups {
        for field_name in group {
            required_field_names.insert(field_name.clone());
        }
    }

    let mut common_optional_fields = Vec::new();

    for field_name in properties.keys() {
        let is_required = required_field_names.contains(field_name);

        if is_required {
            continue;
        }

        common_optional_fields.push(field_name.clone());
    }

    json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": common_optional_fields,
    })
}

pub(super) fn searchable_entry_from_manual_definition(
    canonical_name: &str,
    summary: &str,
    argument_hint: &str,
    required_fields: Vec<String>,
    required_field_groups: Vec<Vec<String>>,
    tags: Vec<String>,
) -> SearchableToolEntry {
    let mut name_fragments = vec![canonical_name.to_owned()];
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    let variant_is_distinct = canonical_name_variant != canonical_name;
    if variant_is_distinct {
        name_fragments.push(canonical_name_variant);
    }

    let summary_text = summary.to_owned();
    let search_hint = summary.to_owned();
    let argument_hint_text = argument_hint.to_owned();
    let argument_fragments =
        build_argument_fragments(argument_hint, &required_fields, &required_field_groups);
    let schema_preview = json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": []
    });

    let mut schema_fragments = required_fields.clone();
    for required_field_group in &required_field_groups {
        let group_fragment = required_field_group.join(" ");
        schema_fragments.push(group_fragment);
    }

    let search_document = SearchDocument::new(
        name_fragments,
        vec![summary_text.clone()],
        argument_fragments,
        schema_fragments,
        tags.clone(),
    );

    SearchableToolEntry {
        canonical_name: canonical_name.to_owned(),
        summary: summary_text,
        search_hint,
        argument_hint: argument_hint_text,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        search_document,
    }
}

pub(super) fn search_argument_hint_from_provider_definition(
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
) -> String {
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return String::new();
    };

    let required = schema_required_fields(parameters)
        .into_iter()
        .collect::<BTreeSet<_>>();

    let mut fields = Vec::new();
    for (name, schema) in properties {
        let schema_type = schema_argument_type(schema);
        let is_required = required.contains(name.as_str());
        let preferred_index = preferred_parameter_index(name.as_str(), preferred_parameter_order);
        let field = SchemaArgumentField {
            name: name.to_owned(),
            schema_type,
            required: is_required,
            preferred_index,
        };
        fields.push(field);
    }

    fields.sort_by(|left, right| {
        let left_required_rank = if left.required { 0usize } else { 1usize };
        let right_required_rank = if right.required { 0usize } else { 1usize };

        left_required_rank
            .cmp(&right_required_rank)
            .then_with(|| left.preferred_index.cmp(&right.preferred_index))
            .then_with(|| left.name.cmp(&right.name))
    });

    let total_field_count = fields.len();
    let compact_fields = compact_argument_hint_fields(fields);
    let omitted_field_count = total_field_count.saturating_sub(compact_fields.len());
    let mut fragments = compact_fields
        .into_iter()
        .map(|field| field.format())
        .collect::<Vec<_>>();

    if omitted_field_count > 0 {
        fragments.push(format!("+{omitted_field_count} more"));
    }

    fragments.join(",")
}

pub(super) fn schema_required_fields(parameters: &Value) -> Vec<String> {
    parameters
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn schema_required_field_groups(parameters: &Value) -> Vec<Vec<String>> {
    let root_required_fields = schema_required_fields(parameters);
    let mut groups = Vec::new();

    for key in ["anyOf", "oneOf"] {
        let Some(options) = parameters.get(key).and_then(Value::as_array) else {
            continue;
        };

        for schema in options {
            let branch_required_fields = schema_required_fields(schema);
            let merged_required_fields = merge_required_field_group(
                root_required_fields.as_slice(),
                branch_required_fields.as_slice(),
            );
            let duplicate_group = groups.iter().any(|group| group == &merged_required_fields);

            if duplicate_group {
                continue;
            }

            groups.push(merged_required_fields);
        }
    }

    groups
}

fn merge_required_field_group(
    root_required_fields: &[String],
    branch_required_fields: &[String],
) -> Vec<String> {
    let mut merged_required_fields = root_required_fields.to_vec();

    for field_name in branch_required_fields {
        let already_present = merged_required_fields
            .iter()
            .any(|existing_name| existing_name == field_name);

        if already_present {
            continue;
        }

        merged_required_fields.push(field_name.clone());
    }

    merged_required_fields
}

pub(super) fn default_required_field_groups(
    required_fields: &[String],
    mut required_field_groups: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
    let missing_groups = required_field_groups.is_empty();
    let has_required_fields = !required_fields.is_empty();

    if missing_groups && has_required_fields {
        required_field_groups.push(required_fields.to_vec());
    }

    required_field_groups
}

pub(super) fn rank_searchable_entries(
    entries: Vec<SearchableToolEntry>,
    query: &str,
    limit: usize,
) -> ToolSearchRanking {
    if entries.is_empty() {
        return ToolSearchRanking {
            results: Vec::new(),
            diagnostics_reason: Some("no_visible_tools"),
        };
    }

    let search_query = SearchQuery::new(query);
    let mut ranked = Vec::new();

    for entry in &entries {
        let score = score_entry(entry, &search_query);
        let Some(score) = score else {
            continue;
        };

        let ranked_entry = ScoredSearchableToolEntry {
            entry: entry.clone(),
            score: score.score,
            why: score.why,
        };
        ranked.push(ranked_entry);
    }

    sort_scored_entries(&mut ranked);

    if !ranked.is_empty() {
        let results = ranked
            .into_iter()
            .take(limit)
            .map(|entry| RankedSearchableToolEntry {
                entry: entry.entry,
                why: entry.why,
            })
            .collect();

        return ToolSearchRanking {
            results,
            diagnostics_reason: None,
        };
    }

    coarse_fallback(entries, limit)
}

fn score_entry(entry: &SearchableToolEntry, query: &SearchQuery) -> Option<SearchScore> {
    let mut score = 0u32;
    let mut why = BTreeSet::new();

    let normalized_query = query.signal.normalized_text.as_str();
    let query_tokens = &query.signal.tokens;

    let _name_phrase_hit = add_phrase_score(
        "name",
        64,
        &entry.search_document.name,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _summary_phrase_hit = add_phrase_score(
        "summary",
        42,
        &entry.search_document.summary,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _argument_phrase_hit = add_phrase_score(
        "argument",
        30,
        &entry.search_document.arguments,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _schema_phrase_hit = add_phrase_score(
        "schema",
        28,
        &entry.search_document.schema,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _tag_phrase_hit = add_phrase_score(
        "tag",
        24,
        &entry.search_document.tags,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _name_token_hit = add_token_scores(
        "name",
        20,
        &entry.search_document.name,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _summary_token_hit = add_token_scores(
        "summary",
        12,
        &entry.search_document.summary,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _argument_token_hit = add_token_scores(
        "argument",
        10,
        &entry.search_document.arguments,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _schema_token_hit = add_token_scores(
        "schema",
        9,
        &entry.search_document.schema,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _tag_token_hit = add_token_scores(
        "tag",
        14,
        &entry.search_document.tags,
        query_tokens,
        &mut score,
        &mut why,
    );

    let concept_overlap = ordered_overlap(&query.concepts, &entry.search_document.concepts);
    for concept in concept_overlap {
        score += 26;
        why.insert(format!("concept:{concept}"));
    }

    let category_overlap = ordered_overlap(&query.categories, &entry.search_document.categories);
    for category in category_overlap {
        score += 12;
        why.insert(format!("category:{category}"));
    }

    if score == 0 {
        return None;
    }

    let mut why = why.into_iter().collect::<Vec<_>>();
    why.truncate(MAX_SEARCH_WHY_REASONS);

    Some(SearchScore { score, why })
}

fn add_phrase_score(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    normalized_query: &str,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let phrase_allowed = phrase_search_allowed(normalized_query);
    if !phrase_allowed {
        return false;
    }

    let contains_query = signal.normalized_text.contains(normalized_query);
    if !contains_query {
        return false;
    }

    *score += weight;
    why.insert(format!("{label}_phrase"));
    true
}

fn add_token_scores(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    query_tokens: &BTreeSet<String>,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let overlaps = ordered_overlap(query_tokens, &signal.tokens);
    if overlaps.is_empty() {
        return false;
    }

    for token in overlaps {
        *score += weight;
        why.insert(format!("{label}:{token}"));
    }

    true
}

fn phrase_search_allowed(normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return false;
    }

    let character_count = normalized_query.chars().count();
    if normalized_query.is_ascii() {
        return character_count >= 2;
    }

    character_count >= 1
}

fn coarse_fallback(entries: Vec<SearchableToolEntry>, limit: usize) -> ToolSearchRanking {
    let mut ranked = Vec::new();

    for entry in entries {
        let (score, why) = coarse_fallback_score(&entry);
        let ranked_entry = ScoredSearchableToolEntry { entry, score, why };
        ranked.push(ranked_entry);
    }

    sort_scored_entries(&mut ranked);

    let results = ranked
        .into_iter()
        .take(limit)
        .map(|entry| RankedSearchableToolEntry {
            entry: entry.entry,
            why: entry.why,
        })
        .collect();

    ToolSearchRanking {
        results,
        diagnostics_reason: Some("coarse_fallback"),
    }
}

fn coarse_fallback_score(entry: &SearchableToolEntry) -> (u32, Vec<String>) {
    let mut score = 1u32;
    let mut why = BTreeSet::new();

    why.insert("coarse_fallback".to_owned());

    let mut discovery_bonus = 0u32;
    for concept in COARSE_FALLBACK_DISCOVERY_CONCEPTS {
        let contains_concept = entry.search_document.concepts.contains(*concept);
        if !contains_concept {
            continue;
        }

        discovery_bonus += 1;
    }

    if discovery_bonus > 0 {
        let discovery_score = 40u32 + discovery_bonus * 6u32;
        score += discovery_score;
        why.insert("coarse_discovery_tool".to_owned());
    }

    let category_score = entry.search_document.categories.len() as u32;
    score += category_score;

    let concept_score = entry.search_document.concepts.len() as u32;
    score += concept_score;

    let why = why.into_iter().collect::<Vec<_>>();

    (score, why)
}

fn sort_scored_entries(entries: &mut [ScoredSearchableToolEntry]) {
    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.entry.canonical_name.cmp(&right.entry.canonical_name))
    });
}

fn build_name_fragments(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
) -> Vec<String> {
    let canonical_name_fragment = canonical_name.to_owned();
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    let provider_name_fragment = provider_name.to_owned();
    let provider_name_variant = identifier_phrase_variant(provider_name);
    let mut fragments = Vec::from([
        canonical_name_fragment,
        canonical_name_variant,
        provider_name_fragment,
        provider_name_variant,
    ]);

    for alias in aliases {
        fragments.push((*alias).to_owned());
        fragments.push(identifier_phrase_variant(alias));
    }

    fragments
}

fn build_argument_fragments(
    argument_hint: &str,
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
) -> Vec<String> {
    let mut fragments = Vec::new();

    if !argument_hint.is_empty() {
        fragments.push(argument_hint.to_owned());
    }

    if !required_fields.is_empty() {
        let required_joined = required_fields.join(" ");
        fragments.push(required_joined);
    }

    for group in required_field_groups {
        let group_joined = group.join(" ");
        fragments.push(group_joined);
    }

    fragments
}

fn collect_schema_search_terms(schema: &Value) -> Vec<String> {
    let mut fragments = Vec::new();
    collect_schema_search_terms_into(schema, &mut fragments);
    fragments
}

fn collect_schema_search_terms_into(schema: &Value, fragments: &mut Vec<String>) {
    let Value::Object(map) = schema else {
        return;
    };

    for key in ["title", "description"] {
        let value = map.get(key);
        let Some(text) = value.and_then(Value::as_str) else {
            continue;
        };

        fragments.push(text.to_owned());
    }

    let property_names = map.get("properties").and_then(Value::as_object);
    if let Some(property_names) = property_names {
        for (name, property_schema) in property_names {
            fragments.push(name.to_owned());
            collect_schema_search_terms_into(property_schema, fragments);
        }
    }

    for key in [
        "items",
        "additionalProperties",
        "contains",
        "if",
        "then",
        "else",
        "not",
    ] {
        let nested_schema = map.get(key);
        let Some(nested_schema) = nested_schema else {
            continue;
        };

        collect_schema_search_terms_into(nested_schema, fragments);
    }

    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        let nested_schemas = map.get(key).and_then(Value::as_array);
        let Some(nested_schemas) = nested_schemas else {
            continue;
        };

        for nested_schema in nested_schemas {
            collect_schema_search_terms_into(nested_schema, fragments);
        }
    }

    let enum_values = map.get("enum").and_then(Value::as_array);
    if let Some(enum_values) = enum_values {
        for enum_value in enum_values {
            let Some(text) = enum_value.as_str() else {
                continue;
            };

            fragments.push(text.to_owned());
        }
    }

    let example_values = map.get("examples").and_then(Value::as_array);
    if let Some(example_values) = example_values {
        for example_value in example_values {
            let Some(text) = example_value.as_str() else {
                continue;
            };

            fragments.push(text.to_owned());
        }
    }

    let const_value = map.get("const").and_then(Value::as_str);
    if let Some(const_value) = const_value {
        fragments.push(const_value.to_owned());
    }
}

fn identifier_phrase_variant(raw: &str) -> String {
    let normalized = normalize_search_text(raw);
    let mut characters = String::new();
    let mut last_was_space = false;

    for character in normalized.chars() {
        let replacement = if is_identifier_separator(character) {
            ' '
        } else {
            character
        };

        if replacement == ' ' {
            if last_was_space {
                continue;
            }

            last_was_space = true;
            characters.push(' ');
            continue;
        }

        last_was_space = false;
        characters.push(replacement);
    }

    characters.trim().to_owned()
}

fn extract_concepts_and_categories(
    signal: &SearchSignalSet,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut concepts = BTreeSet::new();
    let mut categories = BTreeSet::new();

    for concept in normalized_search_concepts() {
        let mut matched = false;

        for form in &concept.forms {
            let contains_form = signal.contains_term(form.as_str());
            if !contains_form {
                continue;
            }

            matched = true;
            break;
        }

        if !matched {
            continue;
        }

        concepts.insert(concept.id.to_owned());

        for category in concept.categories {
            categories.insert((*category).to_owned());
        }
    }

    (concepts, categories)
}

fn normalized_search_concepts() -> &'static Vec<NormalizedSearchConcept> {
    static SEARCH_CONCEPTS: OnceLock<Vec<NormalizedSearchConcept>> = OnceLock::new();

    SEARCH_CONCEPTS.get_or_init(|| {
        SEARCH_CONCEPT_DEFINITIONS
            .iter()
            .map(|concept| {
                let forms = concept
                    .forms
                    .iter()
                    .map(|form| normalize_search_text(form))
                    .filter(|form| !form.is_empty())
                    .collect::<Vec<_>>();

                NormalizedSearchConcept {
                    id: concept.id,
                    categories: concept.categories,
                    forms,
                }
            })
            .collect()
    })
}

fn apply_structural_query_hints(
    raw_query: &str,
    concepts: &mut BTreeSet<String>,
    categories: &mut BTreeSet<String>,
) {
    let looks_like_url = query_looks_like_url(raw_query);
    if looks_like_url {
        insert_concept_and_categories("web", concepts, categories);
    }

    let looks_like_file_reference = query_looks_like_file_reference(raw_query);
    if looks_like_file_reference {
        insert_concept_and_categories("file", concepts, categories);
    }
}

fn insert_concept_and_categories(
    concept_id: &str,
    concepts: &mut BTreeSet<String>,
    categories: &mut BTreeSet<String>,
) {
    concepts.insert(concept_id.to_owned());

    for concept in normalized_search_concepts() {
        if concept.id != concept_id {
            continue;
        }

        for category in concept.categories {
            categories.insert((*category).to_owned());
        }
    }
}

fn query_looks_like_url(raw_query: &str) -> bool {
    for token in raw_query.split_whitespace() {
        let cleaned = trim_structural_token(token);
        if cleaned.is_empty() {
            continue;
        }

        let has_path_separator = cleaned.contains('/') || cleaned.contains('\\');
        let host_like_path = has_path_separator && token_has_host_like_prefix(cleaned);
        let looks_like_url = token_looks_like_url(cleaned);
        if looks_like_url || host_like_path {
            return true;
        }
    }

    false
}

fn token_looks_like_url(token: &str) -> bool {
    token.contains("://") || token.starts_with("www.")
}

fn query_looks_like_file_reference(raw_query: &str) -> bool {
    let cleaned_tokens = raw_query
        .split_whitespace()
        .map(trim_structural_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let single_token_query = cleaned_tokens.len() == 1;

    for token in cleaned_tokens {
        let ambiguous_single_dot_token = token_is_ambiguous_single_dot_token(token);

        if single_token_query && ambiguous_single_dot_token {
            continue;
        }

        let looks_like_file_reference = token_looks_like_file_reference(token);
        if looks_like_file_reference {
            return true;
        }
    }

    false
}

fn token_looks_like_file_reference(token: &str) -> bool {
    if token_looks_like_url(token) {
        return false;
    }

    let has_path_separator = token.contains('/') || token.contains('\\');
    if has_path_separator {
        let host_like_prefix = token_has_host_like_prefix(token);

        if host_like_prefix {
            return false;
        }

        return true;
    }

    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };

    let single_dot_count = token.chars().filter(|character| *character == '.').count();
    let has_single_dot = single_dot_count == 1;
    if has_single_dot {
        let stem_has_alpha = stem.chars().any(|character| character.is_alphabetic());
        let extension_length = extension.chars().count();
        let extension_length_valid = (2..=4).contains(&extension_length);
        let extension_characters_valid = extension
            .chars()
            .all(|character| character.is_ascii_alphabetic());

        return stem_has_alpha && extension_length_valid && extension_characters_valid;
    }

    let stem_valid = stem
        .chars()
        .any(|character| character.is_alphanumeric() || character == '_' || character == '-');
    if !stem_valid {
        return false;
    }

    let extension_length = extension.chars().count();
    let extension_length_valid = (1..=8).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_alphanumeric());

    extension_length_valid && extension_characters_valid
}

fn token_has_host_like_prefix(token: &str) -> bool {
    let host_candidate = token.split(['/', '\\']).next().unwrap_or(token);
    let normalized_candidate = host_candidate.to_ascii_lowercase();
    let Some((stem, extension)) = normalized_candidate.rsplit_once('.') else {
        return false;
    };
    let extension_length = extension.chars().count();
    let extension_length_valid = (2..=4).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_ascii_lowercase());
    let stem_has_alpha = stem
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    let stem_characters_valid = stem.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '.'
    });

    stem_has_alpha && extension_length_valid && extension_characters_valid && stem_characters_valid
}

fn token_is_ambiguous_single_dot_token(token: &str) -> bool {
    let has_path_separator = token.contains('/') || token.contains('\\');
    if has_path_separator {
        return false;
    }

    let Some((stem, extension)) = token.rsplit_once('.') else {
        return false;
    };
    let single_dot_count = token.chars().filter(|character| *character == '.').count();
    let has_single_dot = single_dot_count == 1;

    if !has_single_dot {
        return false;
    }

    let stem_has_alpha = stem.chars().any(|character| character.is_alphabetic());
    let extension_length = extension.chars().count();
    let extension_length_valid = (2..=4).contains(&extension_length);
    let extension_characters_valid = extension
        .chars()
        .all(|character| character.is_ascii_alphabetic());

    stem_has_alpha && extension_length_valid && extension_characters_valid
}

fn trim_structural_token(token: &str) -> &str {
    token.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '!' | '?'
            )
    })
}

fn schema_argument_type(schema: &Value) -> String {
    let schema_type = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("value");

    if schema_type != "array" {
        return schema_type.to_owned();
    }

    let item_type = schema
        .get("items")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);

    let Some(item_type) = item_type else {
        return "array".to_owned();
    };

    format!("{item_type}[]")
}

fn preferred_parameter_index(
    parameter_name: &str,
    preferred_parameter_order: &[(&str, &str)],
) -> usize {
    for (index, (preferred_name, _)) in preferred_parameter_order.iter().enumerate() {
        if *preferred_name == parameter_name {
            return index;
        }
    }

    usize::MAX
}

fn compact_argument_hint_fields(fields: Vec<SchemaArgumentField>) -> Vec<SchemaArgumentField> {
    if fields.len() <= 4 {
        return fields;
    }

    let mut compacted = Vec::new();
    let mut required_fields = 0usize;
    let mut optional_fields = 0usize;

    for field in fields {
        if field.required {
            if required_fields >= 2 {
                continue;
            }

            required_fields += 1;
            compacted.push(field);
            continue;
        }

        if optional_fields >= 1 {
            continue;
        }

        optional_fields += 1;
        compacted.push(field);
    }

    if compacted.is_empty() {
        return Vec::new();
    }

    compacted
}

fn normalize_search_text(raw: &str) -> String {
    let compatibility = raw.nfkc().collect::<String>();
    let lowercased = compatibility.to_lowercase();

    let mut normalized = String::new();
    let mut last_was_space = false;

    for character in lowercased.nfd() {
        if is_combining_mark(character) {
            continue;
        }

        let normalized_character = if character.is_whitespace() {
            ' '
        } else {
            character
        };
        if normalized_character == ' ' {
            if last_was_space {
                continue;
            }

            last_was_space = true;
            normalized.push(' ');
            continue;
        }

        last_was_space = false;
        normalized.push(normalized_character);
    }

    normalized.trim().to_owned()
}

fn tokenize_normalized_text(normalized: &str) -> BTreeSet<String> {
    let surface = identifier_phrase_variant(normalized);
    let mut tokens = BTreeSet::new();

    for word in UnicodeSegmentation::unicode_words(surface.as_str()) {
        let token = word.trim();
        let keep_token = should_keep_token(token);
        if !keep_token {
            continue;
        }

        tokens.insert(token.to_owned());
    }

    tokens
}

fn should_keep_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }

    let ascii_token = token
        .chars()
        .all(|character| character.is_ascii_alphanumeric());

    if ascii_token {
        return token.len() >= 2;
    }

    true
}

fn is_identifier_separator(character: char) -> bool {
    matches!(
        character,
        '.' | '_' | '-' | '/' | '\\' | ':' | ',' | ';' | '|' | '(' | ')' | '[' | ']'
    )
}

fn ordered_overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_search_text_folds_diacritics_and_width_variants() {
        let normalized = normalize_search_text("Ｂúsqueda　网页");
        assert_eq!(normalized, "busqueda 网页");
    }

    #[test]
    fn multilingual_concepts_extract_from_non_latin_queries() {
        let fragments = vec!["تثبيت مهارة".to_owned()];
        let signal = SearchSignalSet::from_fragments(&fragments);
        let (concepts, categories) = extract_concepts_and_categories(&signal);

        assert!(concepts.contains("install"));
        assert!(concepts.contains("skill"));
        assert!(categories.contains("extension"));
        assert!(categories.contains("mutation"));
    }

    #[test]
    fn structural_query_hints_detect_file_references() {
        let query = SearchQuery::new("read note.md");
        assert!(query.concepts.contains("file"));
        assert!(query.categories.contains("workspace"));
    }

    #[test]
    fn schema_required_field_groups_merge_root_and_branch_requirements() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {"type": "string"},
                "content": {"type": "string"},
                "content_path": {"type": "string"}
            },
            "anyOf": [
                {"required": ["content"]},
                {}
            ]
        });
        let required_field_groups = schema_required_field_groups(&schema);

        assert_eq!(
            required_field_groups,
            vec![
                vec!["url".to_owned(), "content".to_owned()],
                vec!["url".to_owned()],
            ]
        );
    }

    #[test]
    fn structural_query_hints_do_not_treat_lone_domains_as_files() {
        let query = SearchQuery::new("example.com");

        assert!(!query.concepts.contains("file"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_domain_paths_as_files() {
        let query = SearchQuery::new("example.com/path");

        assert!(!query.concepts.contains("file"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_version_tokens_as_files() {
        let version_query = SearchQuery::new("gpt-4.1");
        let numeric_query = SearchQuery::new("3.14");

        assert!(!version_query.concepts.contains("file"));
        assert!(!numeric_query.concepts.contains("file"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_generic_tree_queries_as_directories() {
        let query = SearchQuery::new("binary tree traversal");

        assert!(!query.concepts.contains("directory"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn single_dotted_identifier_queries_keep_search_signals() {
        let query = SearchQuery::new("bash.exec");

        assert!(query.signal.contains_term("bash"));
        assert!(query.signal.contains_term("exec"));
        assert!(query.signal.normalized_text.contains("bash.exec"));
        assert!(!query.concepts.contains("file"));
    }
}
