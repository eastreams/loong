use std::fmt;
use std::path::PathBuf;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SecretRef {
    Env { env: String },
    File { file: PathBuf },
    Exec { exec: Vec<String> },
    Inline(String),
}

impl SecretRef {
    pub fn is_configured(&self) -> bool {
        match self {
            Self::Inline(value) => !value.trim().is_empty(),
            Self::Env { .. } | Self::File { .. } | Self::Exec { .. } => true,
        }
    }

    pub fn env_name(&self) -> Option<&str> {
        match self {
            Self::Env { env } => Some(env.as_str()),
            Self::File { .. } | Self::Exec { .. } | Self::Inline(_) => None,
        }
    }

    pub fn explicit_env_name(&self) -> Option<String> {
        match self {
            Self::Env { env } => Some(env.clone()),
            Self::Inline(value) => parse_explicit_env_reference(value.as_str()),
            Self::File { .. } | Self::Exec { .. } => None,
        }
    }

    pub fn inline_value(&self) -> Option<&str> {
        match self {
            Self::Inline(value) => Some(value.as_str()),
            Self::Env { .. } | Self::File { .. } | Self::Exec { .. } => None,
        }
    }

    pub fn inline_literal_value(&self) -> Option<&str> {
        let Self::Inline(value) = self else {
            return None;
        };

        if parse_explicit_env_reference(value.as_str()).is_some() {
            return None;
        }

        let trimmed_value = value.trim();
        if trimmed_value.is_empty() {
            return None;
        }

        Some(trimmed_value)
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env { env } => formatter
                .debug_struct("SecretRef::Env")
                .field("env", env)
                .finish(),
            Self::File { file } => formatter
                .debug_struct("SecretRef::File")
                .field("file", file)
                .finish(),
            Self::Exec { exec } => formatter
                .debug_struct("SecretRef::Exec")
                .field("exec", exec)
                .finish(),
            Self::Inline(_) => formatter.write_str("SecretRef::Inline(<redacted>)"),
        }
    }
}

impl Serialize for SecretRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Inline(value) => serializer.serialize_str(value.as_str()),
            Self::Env { env } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("env", env)?;
                map.end()
            }
            Self::File { file } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("file", file)?;
                map.end()
            }
            Self::Exec { exec } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("exec", exec)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(SecretRefVisitor)
    }
}

struct SecretRefVisitor;

impl<'de> Visitor<'de> for SecretRefVisitor {
    type Value = SecretRef;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string or a table with exactly one of `env`, `file`, or `exec`")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(classify_string_secret_ref(value))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(classify_owned_string_secret_ref(value))
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let table = SecretRefTable::deserialize(de::value::MapAccessDeserializer::new(map))?;

        let env = normalize_non_empty_string(table.env);
        let file = normalize_non_empty_string(table.file);
        let exec = table.exec;

        match (env, file, exec) {
            (Some(env), None, None) => Ok(SecretRef::Env { env }),
            (None, Some(file), None) => {
                let path = PathBuf::from(file);
                Ok(SecretRef::File { file: path })
            }
            (None, None, Some(exec)) => {
                validate_exec_command(exec.as_slice()).map_err(de::Error::custom)?;
                Ok(SecretRef::Exec { exec })
            }
            (None, None, None) => Err(de::Error::custom(
                "secret reference table must contain one of `env`, `file`, or `exec`",
            )),
            _ => Err(de::Error::custom(
                "secret reference table must contain exactly one of `env`, `file`, or `exec`",
            )),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretRefTable {
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    exec: Option<Vec<String>>,
}

fn classify_owned_string_secret_ref(value: String) -> SecretRef {
    let explicit_env_name = parse_explicit_env_reference(value.as_str());
    if let Some(env_name) = explicit_env_name {
        return SecretRef::Env { env: env_name };
    }
    SecretRef::Inline(value)
}

fn classify_string_secret_ref(value: &str) -> SecretRef {
    let explicit_env_name = parse_explicit_env_reference(value);
    if let Some(env_name) = explicit_env_name {
        return SecretRef::Env { env: env_name };
    }
    SecretRef::Inline(value.to_owned())
}

fn normalize_non_empty_string(raw: Option<String>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn validate_exec_command(exec: &[String]) -> Result<(), &'static str> {
    let Some(program) = exec.first() else {
        return Err("secret exec command must not be empty");
    };
    let trimmed_program = program.trim();
    if trimmed_program.is_empty() {
        return Err("secret exec command program must not be empty");
    }
    Ok(())
}

fn parse_explicit_env_reference(raw: &str) -> Option<String> {
    parse_dollar_env_reference(raw)
        .or_else(|| parse_env_prefix_reference(raw))
        .or_else(|| parse_percent_env_reference(raw))
}

fn parse_dollar_env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix('$')?;
    let stripped = stripped.trim();
    if stripped.is_empty() {
        return None;
    }

    let wrapped = stripped.strip_prefix('{');
    let candidate = wrapped
        .and_then(|value| value.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(stripped);

    normalize_env_name(candidate)
}

fn parse_env_prefix_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let prefix = trimmed.get(..4)?;
    if !prefix.eq_ignore_ascii_case("env:") {
        return None;
    }

    let candidate = trimmed.get(4..)?;
    normalize_env_name(candidate)
}

fn parse_percent_env_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let body = trimmed.strip_prefix('%')?;
    let body = body.strip_suffix('%')?;
    normalize_env_name(body)
}

fn normalize_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !looks_like_compatible_env_name(trimmed) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn looks_like_compatible_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_env_prefixed_strings_as_env_refs() {
        let parsed =
            serde_json::from_str::<SecretRef>("\"env:OPENAI_API_KEY\"").expect("parse env ref");

        assert_eq!(
            parsed,
            SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            }
        );
    }

    #[test]
    fn deserializing_non_ascii_inline_string_does_not_panic() {
        let parsed = serde_json::from_str::<SecretRef>("\"हéx\"").expect("parse inline secret");

        assert_eq!(parsed, SecretRef::Inline("हéx".to_owned()));
    }
}
