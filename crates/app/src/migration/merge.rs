use std::{cmp::Ordering, collections::BTreeMap};

use crate::CliResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProfileEntryLane {
    Prompt,
    Profile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMergeEntry {
    pub lane: ProfileEntryLane,
    pub canonical_text: String,
    pub source_id: String,
    pub source_confidence: u32,
    pub entry_confidence: u32,
    pub slot_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMergeConflict {
    pub slot_key: Option<String>,
    pub preferred_source_id: String,
    pub discarded_source_id: String,
    pub preferred_text: String,
    pub discarded_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergedProfilePlan {
    pub prompt_owner_source_id: Option<String>,
    pub kept_entries: Vec<ProfileMergeEntry>,
    pub dropped_duplicates: Vec<ProfileMergeEntry>,
    pub unresolved_conflicts: Vec<ProfileMergeConflict>,
    pub auto_apply_allowed: bool,
    pub merged_profile_note: String,
}

pub fn merge_profile_entries(entries: &[ProfileMergeEntry]) -> CliResult<MergedProfilePlan> {
    let prompt_owner_source_id = entries
        .iter()
        .filter(|entry| entry.lane == ProfileEntryLane::Prompt)
        .max_by(|left, right| compare_entries(left, right))
        .map(|entry| entry.source_id.clone());

    let mut profile_entries = entries
        .iter()
        .filter(|entry| entry.lane == ProfileEntryLane::Profile)
        .cloned()
        .collect::<Vec<_>>();
    profile_entries.sort_by(|left, right| compare_entries(right, left));

    let mut deduped_by_text = BTreeMap::new();
    let mut dropped_duplicates = Vec::new();
    for entry in profile_entries {
        let key = duplicate_key(&entry);
        match deduped_by_text.entry(key) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(entry);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if compare_entries(&entry, slot.get()).is_gt() {
                    dropped_duplicates.push(slot.insert(entry));
                } else {
                    dropped_duplicates.push(entry);
                }
            }
        }
    }

    let mut slot_winners = BTreeMap::new();
    let mut slotless_entries = Vec::new();
    let mut unresolved_conflicts = Vec::new();

    for entry in deduped_by_text.into_values() {
        let Some(slot_key) = entry.slot_key.clone() else {
            slotless_entries.push(entry);
            continue;
        };

        match slot_winners.entry(slot_key.clone()) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(entry);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if compare_entries(&entry, slot.get()).is_gt() {
                    unresolved_conflicts.push(ProfileMergeConflict {
                        slot_key: Some(slot_key),
                        preferred_source_id: entry.source_id.clone(),
                        discarded_source_id: slot.get().source_id.clone(),
                        preferred_text: entry.canonical_text.clone(),
                        discarded_text: slot.get().canonical_text.clone(),
                    });
                    slot.insert(entry);
                } else {
                    unresolved_conflicts.push(ProfileMergeConflict {
                        slot_key: Some(slot_key),
                        preferred_source_id: slot.get().source_id.clone(),
                        discarded_source_id: entry.source_id.clone(),
                        preferred_text: slot.get().canonical_text.clone(),
                        discarded_text: entry.canonical_text.clone(),
                    });
                }
            }
        }
    }

    let mut kept_entries = slot_winners.into_values().collect::<Vec<_>>();
    kept_entries.extend(slotless_entries);
    kept_entries.sort_by(compare_kept_entries);
    dropped_duplicates.sort_by(compare_kept_entries);
    unresolved_conflicts.sort_by(|left, right| {
        left.slot_key
            .cmp(&right.slot_key)
            .then_with(|| left.preferred_source_id.cmp(&right.preferred_source_id))
            .then_with(|| left.discarded_source_id.cmp(&right.discarded_source_id))
    });

    Ok(MergedProfilePlan {
        prompt_owner_source_id,
        merged_profile_note: kept_entries
            .iter()
            .map(|entry| entry.canonical_text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
        auto_apply_allowed: unresolved_conflicts.is_empty(),
        kept_entries,
        dropped_duplicates,
        unresolved_conflicts,
    })
}

fn duplicate_key(entry: &ProfileMergeEntry) -> String {
    entry.canonical_text.trim().to_ascii_lowercase()
}

fn compare_entries(left: &ProfileMergeEntry, right: &ProfileMergeEntry) -> Ordering {
    left.slot_key
        .is_some()
        .cmp(&right.slot_key.is_some())
        .then_with(|| entry_score(left).cmp(&entry_score(right)))
        .then_with(|| left.entry_confidence.cmp(&right.entry_confidence))
        .then_with(|| right.source_id.cmp(&left.source_id))
        .then_with(|| right.canonical_text.cmp(&left.canonical_text))
}

fn compare_kept_entries(left: &ProfileMergeEntry, right: &ProfileMergeEntry) -> Ordering {
    left.slot_key
        .cmp(&right.slot_key)
        .then_with(|| left.canonical_text.cmp(&right.canonical_text))
        .then_with(|| left.source_id.cmp(&right.source_id))
}

fn entry_score(entry: &ProfileMergeEntry) -> u32 {
    entry
        .source_confidence
        .saturating_mul(100)
        .saturating_add(entry.entry_confidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_profile_entries_deduplicates_equivalent_entries() {
        let entries = vec![
            ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: "prefers terse shell output".to_owned(),
                source_id: "openclaw".to_owned(),
                source_confidence: 40,
                entry_confidence: 4,
                slot_key: Some("style".to_owned()),
            },
            ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: "prefers terse shell output".to_owned(),
                source_id: "nanobot".to_owned(),
                source_confidence: 18,
                entry_confidence: 2,
                slot_key: None,
            },
        ];

        let result = merge_profile_entries(&entries).expect("merge should succeed");
        assert_eq!(result.kept_entries.len(), 1);
        assert_eq!(result.dropped_duplicates.len(), 1);
        assert_eq!(result.kept_entries[0].slot_key.as_deref(), Some("style"));
    }

    #[test]
    fn merge_profile_entries_reports_same_slot_conflict() {
        let entries = vec![
            ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: "release copilot".to_owned(),
                source_id: "openclaw".to_owned(),
                source_confidence: 40,
                entry_confidence: 4,
                slot_key: Some("role".to_owned()),
            },
            ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: "personal operations assistant".to_owned(),
                source_id: "nanobot".to_owned(),
                source_confidence: 18,
                entry_confidence: 3,
                slot_key: Some("role".to_owned()),
            },
        ];

        let result = merge_profile_entries(&entries).expect("merge should succeed");
        assert_eq!(result.unresolved_conflicts.len(), 1);
        assert!(!result.auto_apply_allowed);
    }

    #[test]
    fn merge_profile_entries_never_changes_prompt_owner() {
        let entries = vec![
            ProfileMergeEntry {
                lane: ProfileEntryLane::Prompt,
                canonical_text: "openclaw prompt overlay".to_owned(),
                source_id: "openclaw".to_owned(),
                source_confidence: 40,
                entry_confidence: 5,
                slot_key: None,
            },
            ProfileMergeEntry {
                lane: ProfileEntryLane::Prompt,
                canonical_text: "nanobot prompt overlay".to_owned(),
                source_id: "nanobot".to_owned(),
                source_confidence: 18,
                entry_confidence: 5,
                slot_key: None,
            },
            ProfileMergeEntry {
                lane: ProfileEntryLane::Profile,
                canonical_text: "prefers short summaries".to_owned(),
                source_id: "nanobot".to_owned(),
                source_confidence: 18,
                entry_confidence: 2,
                slot_key: Some("style".to_owned()),
            },
        ];

        let result = merge_profile_entries(&entries).expect("merge should succeed");
        assert_eq!(result.prompt_owner_source_id.as_deref(), Some("openclaw"));
    }
}
