use loongclaw_app as mvp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportedProviderTransport {
    pub(crate) base_url: String,
    pub(crate) chat_completions_path: String,
    pub(crate) wire_api: mvp::config::ProviderWireApi,
}

impl ImportedProviderTransport {
    pub(crate) fn from_provider(provider: &mvp::config::ProviderConfig) -> Self {
        Self {
            base_url: provider.base_url.clone(),
            chat_completions_path: provider.chat_completions_path.clone(),
            wire_api: provider.wire_api,
        }
    }

    pub(crate) fn default_for_kind(kind: mvp::config::ProviderKind) -> Self {
        Self::from_provider(&mvp::config::ProviderConfig::fresh_for_kind(kind))
    }

    pub(crate) fn from_optional_overrides(
        kind: mvp::config::ProviderKind,
        base_url: Option<&str>,
        chat_completions_path: Option<&str>,
        wire_api: Option<mvp::config::ProviderWireApi>,
    ) -> Self {
        let defaults = Self::default_for_kind(kind);
        Self {
            base_url: trimmed_non_empty(base_url)
                .unwrap_or(defaults.base_url.as_str())
                .to_owned(),
            chat_completions_path: trimmed_non_empty(chat_completions_path)
                .unwrap_or(defaults.chat_completions_path.as_str())
                .to_owned(),
            wire_api: wire_api.unwrap_or(defaults.wire_api),
        }
    }

    pub(crate) fn apply_to_provider(&self, provider: &mut mvp::config::ProviderConfig) {
        provider.base_url = self.base_url.clone();
        provider.chat_completions_path = self.chat_completions_path.clone();
        provider.wire_api = self.wire_api;
    }
}

pub(crate) fn supplement_provider_transport(
    target: &mut mvp::config::ProviderConfig,
    source: &mvp::config::ProviderConfig,
) -> bool {
    if target.kind != source.kind {
        return false;
    }

    let source_transport = ImportedProviderTransport::from_provider(source);
    let default_transport = ImportedProviderTransport::default_for_kind(target.kind);
    let mut changed = false;

    if target.base_url_is_profile_default_like() && !source.base_url_is_profile_default_like() {
        target.base_url = source_transport.base_url;
        changed = true;
    }
    if target.chat_completions_path_is_profile_default_like()
        && !source.chat_completions_path_is_profile_default_like()
    {
        target.chat_completions_path = source_transport.chat_completions_path;
        changed = true;
    }
    if target.wire_api == default_transport.wire_api
        && source_transport.wire_api != default_transport.wire_api
    {
        target.wire_api = source_transport.wire_api;
        changed = true;
    }

    changed
}

fn trimmed_non_empty(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}
