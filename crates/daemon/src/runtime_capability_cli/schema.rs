use super::*;

pub(super) fn optional_arg(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(super) fn required_trimmed_arg(name: &str, raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("runtime capability {name} cannot be empty"));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn normalize_repeated_values(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn parse_required_capabilities(values: &[String]) -> CliResult<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for raw in values {
        let value = normalize_required_capability(raw)?;
        normalized.insert(value);
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_required_capability(raw: &str) -> CliResult<String> {
    Capability::parse(raw)
        .map(|capability| capability.as_str().to_owned())
        .ok_or_else(|| {
            format!(
                "runtime capability required capability `{}` is unknown",
                raw.trim()
            )
        })
}
