use crate::config::ProviderKind;

pub const PROVIDER_SELECTOR_PLACEHOLDER: &str = "<profile|model|kind>";
pub const PROVIDER_SELECTOR_HUMAN_SUMMARY: &str =
    "profile id, unique model name or suffix, or provider kind";
pub const PROVIDER_SELECTOR_TARGET_SUMMARY: &str =
    "target profile id, unique model name or suffix, or provider kind";
pub const PROVIDER_SELECTOR_NOTE: &str =
    "you can also enter a unique model name, model suffix, or provider kind";
pub const PROVIDER_SELECTOR_COMPACT_NOTE: &str = "type a model, suffix, or provider kind";

fn normalize_provider_selector_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_ascii_lowercase())
}

fn provider_model_suffix(raw: &str) -> Option<String> {
    let normalized = normalize_provider_selector_token(raw)?;
    normalized
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn push_unique_selector(selectors: &mut Vec<String>, candidate: &str) {
    if candidate.trim().is_empty() {
        return;
    }
    if selectors
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    selectors.push(candidate.to_owned());
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderSelectorProfileRef<'a> {
    pub profile_id: &'a str,
    pub kind: ProviderKind,
    pub model: &'a str,
    pub default_for_kind: bool,
}

impl<'a> ProviderSelectorProfileRef<'a> {
    pub const fn new(
        profile_id: &'a str,
        kind: ProviderKind,
        model: &'a str,
        default_for_kind: bool,
    ) -> Self {
        Self {
            profile_id,
            kind,
            model,
            default_for_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelectorResolution {
    Match(String),
    Ambiguous(Vec<String>),
    NoMatch,
}

pub fn accepted_provider_selectors<'a, I>(profiles: I, target_profile_id: &str) -> Vec<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).accepted_selectors(target_profile_id)
}

pub fn provider_selector_catalog<'a, I>(profiles: I) -> Vec<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).selector_catalog()
}

pub fn preferred_provider_selector<'a, I>(profiles: I, target_profile_id: &str) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).preferred_selector(target_profile_id)
}

pub fn describe_provider_selector_target<'a, I>(
    profiles: I,
    target_profile_id: &str,
) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).describe_profile(target_profile_id)
}

pub fn provider_selector_recommendation_hint<'a, I, J, S>(
    profiles: I,
    target_profile_ids: J,
) -> Option<String>
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
    J: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    ProviderSelectorIndex::new(profiles).recommendation_hint(target_profile_ids)
}

pub fn resolve_provider_selector<'a, I>(profiles: I, selector: &str) -> ProviderSelectorResolution
where
    I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
{
    ProviderSelectorIndex::new(profiles).resolve(selector)
}

struct ProviderSelectorIndex<'a> {
    profiles: Vec<ProviderSelectorProfileRef<'a>>,
}

impl<'a> ProviderSelectorIndex<'a> {
    fn new<I>(profiles: I) -> Self
    where
        I: IntoIterator<Item = ProviderSelectorProfileRef<'a>>,
    {
        Self {
            profiles: profiles.into_iter().collect(),
        }
    }

    fn accepted_selectors(&self, target_profile_id: &str) -> Vec<String> {
        let Some(profile) = self.find_profile(target_profile_id) else {
            return Vec::new();
        };

        let mut selectors = Vec::new();
        push_unique_selector(&mut selectors, profile.profile_id);

        if let Some(model) = normalize_provider_selector_token(profile.model)
            && self.model_matches(model.as_str()).len() == 1
        {
            push_unique_selector(&mut selectors, model.as_str());
        }

        if let Some(suffix) = provider_model_suffix(profile.model)
            && self.model_suffix_matches(suffix.as_str()).len() == 1
        {
            push_unique_selector(&mut selectors, suffix.as_str());
        }

        if self.kind_resolves_to_profile_id(profile.kind, profile.profile_id) {
            push_unique_selector(&mut selectors, profile.kind.as_str());
        }

        selectors
    }

    fn resolve(&self, selector: &str) -> ProviderSelectorResolution {
        let Some(normalized) = normalize_provider_selector_token(selector) else {
            return ProviderSelectorResolution::NoMatch;
        };

        if let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.profile_id.eq_ignore_ascii_case(normalized.as_str()))
        {
            return ProviderSelectorResolution::Match(profile.profile_id.to_owned());
        }

        let model_matches = self.model_matches(normalized.as_str());
        match model_matches.as_slice() {
            [profile_id] => return ProviderSelectorResolution::Match(profile_id.clone()),
            matches if matches.len() > 1 => {
                return ProviderSelectorResolution::Ambiguous(matches.to_vec());
            }
            _ => {}
        }

        let suffix_matches = self.model_suffix_matches(normalized.as_str());
        match suffix_matches.as_slice() {
            [profile_id] => return ProviderSelectorResolution::Match(profile_id.clone()),
            matches if matches.len() > 1 => {
                return ProviderSelectorResolution::Ambiguous(matches.to_vec());
            }
            _ => {}
        }

        let Some(kind) = ProviderKind::parse(normalized.as_str()) else {
            return ProviderSelectorResolution::NoMatch;
        };
        self.resolve_kind(kind)
    }

    fn selector_catalog(&self) -> Vec<String> {
        let mut selectors = Vec::new();
        for profile in &self.profiles {
            for selector in self.accepted_selectors(profile.profile_id) {
                push_unique_selector(&mut selectors, selector.as_str());
            }
        }
        selectors
    }

    fn preferred_selector(&self, target_profile_id: &str) -> Option<String> {
        let profile = self.find_profile(target_profile_id)?;
        let selectors = self.accepted_selectors(target_profile_id);
        if selectors.is_empty() {
            return None;
        }

        let profile_id = normalize_provider_selector_token(profile.profile_id);
        let model = normalize_provider_selector_token(profile.model);
        let suffix = provider_model_suffix(profile.model);
        let kind = normalize_provider_selector_token(profile.kind.as_str());
        let profile_id_len = profile_id.as_ref().map_or(usize::MAX, String::len);

        let preferred_candidates = [
            kind.as_deref(),
            suffix.as_deref(),
            model
                .as_deref()
                .filter(|model| model.len() <= profile_id_len),
            profile_id.as_deref(),
            model.as_deref(),
        ];

        for candidate in preferred_candidates.into_iter().flatten() {
            if let Some(selector) = selectors
                .iter()
                .find(|existing| existing.eq_ignore_ascii_case(candidate))
            {
                return Some(selector.clone());
            }
        }

        selectors.into_iter().next()
    }

    fn describe_profile(&self, target_profile_id: &str) -> Option<String> {
        let profile = self.find_profile(target_profile_id)?;
        let selectors = self.accepted_selectors(target_profile_id);
        let mut description = format!("{} [model={}", profile.profile_id, profile.model);
        if !selectors.is_empty() {
            description.push_str("; selectors=");
            description.push_str(selectors.join(", ").as_str());
        }
        description.push(']');
        Some(description)
    }

    fn recommendation_hint<J, S>(&self, target_profile_ids: J) -> Option<String>
    where
        J: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut selectors = Vec::new();
        for profile_id in target_profile_ids {
            let Some(selector) = self.preferred_selector(profile_id.as_ref()) else {
                continue;
            };
            push_unique_selector(&mut selectors, selector.as_str());
            if selectors.len() >= 3 {
                break;
            }
        }
        (!selectors.is_empty()).then(|| format!("try one of: {}", selectors.join(", ")))
    }

    fn find_profile(&self, profile_id: &str) -> Option<&ProviderSelectorProfileRef<'a>> {
        self.profiles
            .iter()
            .find(|profile| profile.profile_id == profile_id)
    }

    fn model_matches(&self, selector: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|profile| {
                normalize_provider_selector_token(profile.model).as_deref() == Some(selector)
            })
            .map(|profile| profile.profile_id.to_owned())
            .collect()
    }

    fn model_suffix_matches(&self, selector: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|profile| provider_model_suffix(profile.model).as_deref() == Some(selector))
            .map(|profile| profile.profile_id.to_owned())
            .collect()
    }

    fn resolve_kind(&self, kind: ProviderKind) -> ProviderSelectorResolution {
        let matches = self
            .profiles
            .iter()
            .filter(|profile| profile.kind == kind)
            .collect::<Vec<_>>();
        let Some(first) = matches.first().copied() else {
            return ProviderSelectorResolution::NoMatch;
        };
        if matches.len() == 1 {
            return ProviderSelectorResolution::Match(first.profile_id.to_owned());
        }

        let default_matches = matches
            .iter()
            .copied()
            .filter(|profile| profile.default_for_kind)
            .collect::<Vec<_>>();
        if let [default_match] = default_matches.as_slice() {
            return ProviderSelectorResolution::Match(default_match.profile_id.to_owned());
        }

        ProviderSelectorResolution::Ambiguous(
            matches
                .into_iter()
                .map(|profile| profile.profile_id.to_owned())
                .collect(),
        )
    }

    fn kind_resolves_to_profile_id(&self, kind: ProviderKind, profile_id: &str) -> bool {
        matches!(
            self.resolve_kind(kind),
            ProviderSelectorResolution::Match(resolved) if resolved == profile_id
        )
    }
}
