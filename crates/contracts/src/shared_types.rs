use serde::{Deserialize, Serialize};
use std::borrow::{Borrow, Cow};
use std::ops::Deref;
use std::sync::Arc;

/// An ergonomic type for str sharing betweem threads
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SharedStr(pub Arc<Cow<'static, str>>);

impl Serialize for SharedStr {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SharedStr {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(SharedStr(Arc::new(Cow::Owned(s))))
    }
}

impl From<&'static str> for SharedStr {
    fn from(s: &'static str) -> Self {
        Self(Arc::new(Cow::Borrowed(s)))
    }
}

impl From<String> for SharedStr {
    fn from(s: String) -> Self {
        Self(Arc::new(Cow::Owned(s)))
    }
}

impl From<Cow<'static, str>> for SharedStr {
    fn from(s: Cow<'static, str>) -> Self {
        Self(Arc::new(s))
    }
}

impl From<Arc<Cow<'static, str>>> for SharedStr {
    fn from(s: Arc<Cow<'static, str>>) -> Self {
        Self(s)
    }
}

impl Deref for SharedStr {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<str> for SharedStr {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<SharedStr> for String {
    fn from(s: SharedStr) -> Self {
        (*s).to_owned()
    }
}
