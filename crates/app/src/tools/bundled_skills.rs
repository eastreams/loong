use include_dir::{Dir, include_dir};
use serde::Serialize;

pub(crate) const BROWSER_COMPANION_PREVIEW_SKILL_ID: &str = "browser-companion-preview";
pub(crate) const BROWSER_COMPANION_COMMAND: &str = "agent-browser";

static BUNDLED_SKILLS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../skills");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BundledExternalSkill {
    pub(crate) skill_id: &'static str,
    pub(crate) source_path: &'static str,
    pub(crate) relative_dir: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BundledSkillPack {
    pub pack_id: &'static str,
    pub display_name: &'static str,
    pub summary: &'static str,
    pub skill_ids: &'static [&'static str],
    pub onboarding_visible: bool,
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BundledPreinstallTargetKind {
    Skill,
    Pack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BundledPreinstallTarget {
    pub install_id: &'static str,
    pub display_name: &'static str,
    pub summary: &'static str,
    pub skill_ids: &'static [&'static str],
    pub kind: BundledPreinstallTargetKind,
    pub recommended: bool,
}

const BUNDLED_EXTERNAL_SKILLS: &[BundledExternalSkill] = &[
    // Standalone bundled skills remain at the top level under `skills/`.
    BundledExternalSkill {
        skill_id: "agent-browser",
        source_path: "bundled://agent-browser",
        relative_dir: "agent-browser",
    },
    BundledExternalSkill {
        skill_id: BROWSER_COMPANION_PREVIEW_SKILL_ID,
        source_path: "bundled://browser-companion-preview",
        relative_dir: "browser-companion-preview",
    },
    BundledExternalSkill {
        skill_id: "design-md",
        source_path: "bundled://design-md",
        relative_dir: "design-md",
    },
    BundledExternalSkill {
        skill_id: "find-skills",
        source_path: "bundled://find-skills",
        relative_dir: "find-skills",
    },
    BundledExternalSkill {
        skill_id: "github-issues",
        source_path: "bundled://github-issues",
        relative_dir: "github-issues",
    },
    // Pack members are grouped under `skills/<pack-id>/` to mirror the
    // operator-facing pack registry without an extra directory layer.
    BundledExternalSkill {
        skill_id: "docx",
        source_path: "bundled://docx",
        relative_dir: "anthropic-office/docx",
    },
    BundledExternalSkill {
        skill_id: "lark-approval",
        source_path: "bundled://lark-approval",
        relative_dir: "larksuite-cli/lark-approval",
    },
    BundledExternalSkill {
        skill_id: "lark-base",
        source_path: "bundled://lark-base",
        relative_dir: "larksuite-cli/lark-base",
    },
    BundledExternalSkill {
        skill_id: "lark-calendar",
        source_path: "bundled://lark-calendar",
        relative_dir: "larksuite-cli/lark-calendar",
    },
    BundledExternalSkill {
        skill_id: "lark-contact",
        source_path: "bundled://lark-contact",
        relative_dir: "larksuite-cli/lark-contact",
    },
    BundledExternalSkill {
        skill_id: "lark-doc",
        source_path: "bundled://lark-doc",
        relative_dir: "larksuite-cli/lark-doc",
    },
    BundledExternalSkill {
        skill_id: "lark-drive",
        source_path: "bundled://lark-drive",
        relative_dir: "larksuite-cli/lark-drive",
    },
    BundledExternalSkill {
        skill_id: "lark-event",
        source_path: "bundled://lark-event",
        relative_dir: "larksuite-cli/lark-event",
    },
    BundledExternalSkill {
        skill_id: "lark-im",
        source_path: "bundled://lark-im",
        relative_dir: "larksuite-cli/lark-im",
    },
    BundledExternalSkill {
        skill_id: "lark-mail",
        source_path: "bundled://lark-mail",
        relative_dir: "larksuite-cli/lark-mail",
    },
    BundledExternalSkill {
        skill_id: "lark-minutes",
        source_path: "bundled://lark-minutes",
        relative_dir: "larksuite-cli/lark-minutes",
    },
    BundledExternalSkill {
        skill_id: "lark-openapi-explorer",
        source_path: "bundled://lark-openapi-explorer",
        relative_dir: "larksuite-cli/lark-openapi-explorer",
    },
    BundledExternalSkill {
        skill_id: "lark-shared",
        source_path: "bundled://lark-shared",
        relative_dir: "larksuite-cli/lark-shared",
    },
    BundledExternalSkill {
        skill_id: "lark-sheets",
        source_path: "bundled://lark-sheets",
        relative_dir: "larksuite-cli/lark-sheets",
    },
    BundledExternalSkill {
        skill_id: "lark-skill-maker",
        source_path: "bundled://lark-skill-maker",
        relative_dir: "larksuite-cli/lark-skill-maker",
    },
    BundledExternalSkill {
        skill_id: "lark-task",
        source_path: "bundled://lark-task",
        relative_dir: "larksuite-cli/lark-task",
    },
    BundledExternalSkill {
        skill_id: "lark-vc",
        source_path: "bundled://lark-vc",
        relative_dir: "larksuite-cli/lark-vc",
    },
    BundledExternalSkill {
        skill_id: "lark-whiteboard",
        source_path: "bundled://lark-whiteboard",
        relative_dir: "larksuite-cli/lark-whiteboard",
    },
    BundledExternalSkill {
        skill_id: "lark-wiki",
        source_path: "bundled://lark-wiki",
        relative_dir: "larksuite-cli/lark-wiki",
    },
    BundledExternalSkill {
        skill_id: "lark-workflow-meeting-summary",
        source_path: "bundled://lark-workflow-meeting-summary",
        relative_dir: "larksuite-cli/lark-workflow-meeting-summary",
    },
    BundledExternalSkill {
        skill_id: "lark-workflow-standup-report",
        source_path: "bundled://lark-workflow-standup-report",
        relative_dir: "larksuite-cli/lark-workflow-standup-report",
    },
    BundledExternalSkill {
        skill_id: "pdf",
        source_path: "bundled://pdf",
        relative_dir: "anthropic-office/pdf",
    },
    BundledExternalSkill {
        skill_id: "plan",
        source_path: "bundled://plan",
        relative_dir: "plan",
    },
    BundledExternalSkill {
        skill_id: "pptx",
        source_path: "bundled://pptx",
        relative_dir: "anthropic-office/pptx",
    },
    BundledExternalSkill {
        skill_id: "skill-creator",
        source_path: "bundled://skill-creator",
        relative_dir: "skill-creator",
    },
    BundledExternalSkill {
        skill_id: "systematic-debugging",
        source_path: "bundled://systematic-debugging",
        relative_dir: "systematic-debugging",
    },
    BundledExternalSkill {
        skill_id: "xlsx",
        source_path: "bundled://xlsx",
        relative_dir: "anthropic-office/xlsx",
    },
    BundledExternalSkill {
        skill_id: "mcporter",
        source_path: "bundled://mcporter",
        relative_dir: "mcporter",
    },
    BundledExternalSkill {
        skill_id: "minimax-docx",
        source_path: "bundled://minimax-docx",
        relative_dir: "minimax-office/minimax-docx",
    },
    BundledExternalSkill {
        skill_id: "minimax-pdf",
        source_path: "bundled://minimax-pdf",
        relative_dir: "minimax-office/minimax-pdf",
    },
    BundledExternalSkill {
        skill_id: "minimax-xlsx",
        source_path: "bundled://minimax-xlsx",
        relative_dir: "minimax-office/minimax-xlsx",
    },
    BundledExternalSkill {
        skill_id: "native-mcp",
        source_path: "bundled://native-mcp",
        relative_dir: "native-mcp",
    },
];

const LARKSUITE_CLI_PACK: BundledSkillPack = BundledSkillPack {
    pack_id: "larksuite-cli",
    display_name: "Larksuite CLI pack",
    summary: "install the bundled Lark/Feishu workflow skill collection",
    skill_ids: &[
        "lark-approval",
        "lark-base",
        "lark-calendar",
        "lark-contact",
        "lark-doc",
        "lark-drive",
        "lark-event",
        "lark-im",
        "lark-mail",
        "lark-minutes",
        "lark-openapi-explorer",
        "lark-shared",
        "lark-sheets",
        "lark-skill-maker",
        "lark-task",
        "lark-vc",
        "lark-whiteboard",
        "lark-wiki",
        "lark-workflow-meeting-summary",
        "lark-workflow-standup-report",
    ],
    onboarding_visible: true,
    recommended: false,
};

const ANTHROPIC_OFFICE_PACK: BundledSkillPack = BundledSkillPack {
    pack_id: "anthropic-office",
    display_name: "Anthropic Office pack",
    summary: "install Anthropic's docx, pdf, pptx, and xlsx document workflow skills",
    skill_ids: &["docx", "pdf", "pptx", "xlsx"],
    onboarding_visible: true,
    recommended: false,
};

const MINIMAX_OFFICE_PACK: BundledSkillPack = BundledSkillPack {
    pack_id: "minimax-office",
    display_name: "Minimax Office pack",
    summary: "install Minimax's advanced docx, pdf, and xlsx document workflow skills",
    skill_ids: &["minimax-docx", "minimax-pdf", "minimax-xlsx"],
    onboarding_visible: true,
    recommended: false,
};

const BUNDLED_SKILL_PACKS: &[BundledSkillPack] = &[
    LARKSUITE_CLI_PACK,
    ANTHROPIC_OFFICE_PACK,
    MINIMAX_OFFICE_PACK,
];

const BUNDLED_PREINSTALL_TARGETS: &[BundledPreinstallTarget] = &[
    BundledPreinstallTarget {
        install_id: "find-skills",
        display_name: "find-skills",
        summary: "discover and install additional skills from curated sources",
        skill_ids: &["find-skills"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: true,
    },
    BundledPreinstallTarget {
        install_id: "github-issues",
        display_name: "github-issues",
        summary: "manage GitHub issues with gh-first workflow guidance",
        skill_ids: &["github-issues"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: true,
    },
    BundledPreinstallTarget {
        install_id: "agent-browser",
        display_name: "agent-browser",
        summary: "browser automation helper with packaged references and templates",
        skill_ids: &["agent-browser"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: true,
    },
    BundledPreinstallTarget {
        install_id: "skill-creator",
        display_name: "skill-creator",
        summary: "Anthropic's multi-file skill authoring and evaluation workflow",
        skill_ids: &["skill-creator"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: false,
    },
    BundledPreinstallTarget {
        install_id: ANTHROPIC_OFFICE_PACK.pack_id,
        display_name: ANTHROPIC_OFFICE_PACK.display_name,
        summary: ANTHROPIC_OFFICE_PACK.summary,
        skill_ids: ANTHROPIC_OFFICE_PACK.skill_ids,
        kind: BundledPreinstallTargetKind::Pack,
        recommended: false,
    },
    BundledPreinstallTarget {
        install_id: MINIMAX_OFFICE_PACK.pack_id,
        display_name: MINIMAX_OFFICE_PACK.display_name,
        summary: MINIMAX_OFFICE_PACK.summary,
        skill_ids: MINIMAX_OFFICE_PACK.skill_ids,
        kind: BundledPreinstallTargetKind::Pack,
        recommended: false,
    },
    BundledPreinstallTarget {
        install_id: "design-md",
        display_name: "design-md",
        summary: "generate semantic DESIGN.md files from Stitch design projects",
        skill_ids: &["design-md"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: false,
    },
    BundledPreinstallTarget {
        install_id: "systematic-debugging",
        display_name: "systematic-debugging",
        summary: "structured debugging workflow before proposing fixes",
        skill_ids: &["systematic-debugging"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: true,
    },
    BundledPreinstallTarget {
        install_id: "plan",
        display_name: "plan",
        summary: "write concise execution plans before implementation work",
        skill_ids: &["plan"],
        kind: BundledPreinstallTargetKind::Skill,
        recommended: false,
    },
    BundledPreinstallTarget {
        install_id: LARKSUITE_CLI_PACK.pack_id,
        display_name: LARKSUITE_CLI_PACK.display_name,
        summary: LARKSUITE_CLI_PACK.summary,
        skill_ids: LARKSUITE_CLI_PACK.skill_ids,
        kind: BundledPreinstallTargetKind::Pack,
        recommended: false,
    },
];

pub(crate) fn bundled_external_skills() -> &'static [BundledExternalSkill] {
    BUNDLED_EXTERNAL_SKILLS
}

pub fn bundled_skill_packs() -> &'static [BundledSkillPack] {
    BUNDLED_SKILL_PACKS
}

pub fn bundled_skill_pack(pack_id: &str) -> Option<&'static BundledSkillPack> {
    bundled_skill_packs()
        .iter()
        .find(|pack| pack.pack_id == pack_id.trim())
}

pub fn bundled_skill_pack_memberships(skill_id: &str) -> Vec<&'static BundledSkillPack> {
    let normalized = skill_id.trim();
    bundled_skill_packs()
        .iter()
        .filter(|pack| pack.skill_ids.contains(&normalized))
        .collect()
}

pub fn bundled_preinstall_targets() -> &'static [BundledPreinstallTarget] {
    BUNDLED_PREINSTALL_TARGETS
}

pub(crate) fn bundled_external_skill(skill_id: &str) -> Option<BundledExternalSkill> {
    bundled_external_skills()
        .iter()
        .copied()
        .find(|skill| skill.skill_id == skill_id.trim())
}

pub(crate) fn bundled_external_skill_dir(
    skill: &BundledExternalSkill,
) -> Option<&'static Dir<'static>> {
    BUNDLED_SKILLS_DIR.get_dir(skill.relative_dir)
}

pub(crate) fn bundled_external_skill_markdown(
    skill: &BundledExternalSkill,
) -> Result<&'static str, String> {
    let dir = bundled_external_skill_dir(skill)
        .ok_or_else(|| format!("missing bundled skill directory `{}`", skill.relative_dir))?;
    let file = dir
        .entries()
        .iter()
        .find_map(|entry| match entry {
            include_dir::DirEntry::File(file)
                if file
                    .path()
                    .file_name()
                    .is_some_and(|name| name == "SKILL.md") =>
            {
                Some(file)
            }
            include_dir::DirEntry::Dir(_) | include_dir::DirEntry::File(_) => None,
        })
        .ok_or_else(|| format!("missing bundled SKILL.md for `{}`", skill.skill_id))?;
    std::str::from_utf8(file.contents()).map_err(|error| {
        format!(
            "bundled SKILL.md for `{}` is not utf-8: {error}",
            skill.skill_id
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        bundled_external_skill, bundled_external_skill_markdown, bundled_preinstall_targets,
        bundled_skill_pack, bundled_skill_pack_memberships,
    };

    #[test]
    fn curated_bundled_inventory_contains_requested_preinstalls() {
        for skill_id in [
            "find-skills",
            "agent-browser",
            "skill-creator",
            "pdf",
            "docx",
            "pptx",
            "xlsx",
            "minimax-docx",
            "minimax-pdf",
            "minimax-xlsx",
            "design-md",
            "lark-doc",
            "native-mcp",
            "mcporter",
            "github-issues",
            "systematic-debugging",
            "plan",
        ] {
            assert!(
                bundled_external_skill(skill_id).is_some(),
                "expected bundled skill inventory to expose `{skill_id}`"
            );
        }
    }

    #[test]
    fn bundled_pack_registry_exposes_expected_memberships() {
        let anthropic_office =
            bundled_skill_pack("anthropic-office").expect("anthropic office pack should exist");
        assert_eq!(anthropic_office.skill_ids, &["docx", "pdf", "pptx", "xlsx"]);

        let memberships = bundled_skill_pack_memberships("docx");
        assert!(
            memberships
                .iter()
                .any(|pack| pack.pack_id == "anthropic-office"),
            "docx should advertise anthropic office membership"
        );
    }

    #[test]
    fn bundled_preinstall_targets_include_pack_entries() {
        let targets = bundled_preinstall_targets();
        assert!(
            targets
                .iter()
                .any(|target| target.install_id == "anthropic-office"),
            "onboarding registry should include the anthropic office pack"
        );
        assert!(
            targets
                .iter()
                .any(|target| target.install_id == "minimax-office"),
            "onboarding registry should include the minimax office pack"
        );
    }

    #[test]
    fn bundled_markdown_lookup_supports_nested_pack_directories() {
        let docx = bundled_external_skill("docx").expect("docx should exist");
        let markdown =
            bundled_external_skill_markdown(&docx).expect("docx markdown should load from pack");
        assert!(
            !markdown.trim().is_empty(),
            "docx bundled markdown should stay readable after pack reorganization"
        );
    }
}
