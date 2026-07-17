use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Capability;

/// Effective capability authority exposed to policy evaluation.
///
/// The collection is opaque so policy consumers depend on set semantics rather
/// than its current storage representation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities(Option<Arc<BTreeSet<Capability>>>);

impl Capabilities {
    #[must_use]
    pub const fn new() -> Self {
        // Recursive Context clones must not copy the current set-backed
        // representation. The wrapper remains opaque so a future bitset can
        // replace this shared storage without changing policy-facing APIs.
        Self(None)
    }

    #[must_use]
    pub fn contains(&self, capability: Capability) -> bool {
        self.0
            .as_ref()
            .is_some_and(|capabilities| capabilities.contains(&capability))
    }

    #[must_use]
    pub fn is_subset(&self, other: &Self) -> bool {
        self.iter().all(|capability| other.contains(capability))
    }

    pub fn difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = Capability> + 'a {
        self.iter()
            .filter(move |capability| !other.contains(*capability))
    }

    pub fn intersection<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = Capability> + 'a {
        self.iter()
            .filter(move |capability| other.contains(*capability))
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0
            .iter()
            .flat_map(|capabilities| capabilities.iter())
            .copied()
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        let capabilities = iter.into_iter().collect::<BTreeSet<_>>();
        if capabilities.is_empty() {
            Self::new()
        } else {
            Self(Some(Arc::new(capabilities)))
        }
    }
}

impl Serialize for Capabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Capabilities {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        BTreeSet::deserialize(deserializer).map(|capabilities| capabilities.into_iter().collect())
    }
}

impl<const N: usize> From<[Capability; N]> for Capabilities {
    fn from(capabilities: [Capability; N]) -> Self {
        capabilities.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Capabilities, Capability};

    #[test]
    fn capabilities_construct_from_array_and_iterator() {
        let from_array =
            Capabilities::from([Capability::FilesystemRead, Capability::NetworkEgress]);
        let from_iterator: Capabilities = [Capability::FilesystemRead, Capability::NetworkEgress]
            .into_iter()
            .collect();

        assert!(from_array.iter().eq(from_iterator.iter()));
    }

    #[test]
    fn capabilities_expose_policy_set_semantics() {
        let parent = Capabilities::from([
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ]);
        let child = Capabilities::from([Capability::FilesystemRead, Capability::NetworkEgress]);

        assert!(parent.contains(Capability::FilesystemWrite));
        assert!(child.is_subset(&parent));
        assert_eq!(
            parent.difference(&child).collect::<Vec<_>>(),
            vec![Capability::FilesystemWrite]
        );
        assert_eq!(
            parent.intersection(&child).collect::<Vec<_>>(),
            vec![Capability::FilesystemRead, Capability::NetworkEgress]
        );
        assert_eq!(
            parent.iter().collect::<Vec<_>>(),
            vec![
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
                Capability::NetworkEgress,
            ]
        );
    }

    #[test]
    fn capabilities_serde_uses_pascal_case_array_and_round_trips() {
        let capabilities = Capabilities::from([
            Capability::MemoryRead,
            Capability::FilesystemRead,
            Capability::NetworkEgress,
        ]);
        let wire = json!(["MemoryRead", "FilesystemRead", "NetworkEgress"]);

        assert_eq!(
            serde_json::to_value(&capabilities).expect("serialize Capabilities"),
            wire
        );

        let round_trip: Capabilities =
            serde_json::from_value(wire).expect("deserialize Capabilities");
        assert_eq!(round_trip, capabilities);
    }

    #[test]
    fn non_empty_capability_clones_share_the_current_storage() {
        let capabilities =
            Capabilities::from([Capability::FilesystemRead, Capability::FilesystemWrite]);
        let cloned = capabilities.clone();

        assert!(std::sync::Arc::ptr_eq(
            capabilities.0.as_ref().expect("non-empty storage"),
            cloned.0.as_ref().expect("cloned non-empty storage"),
        ));
    }
}
