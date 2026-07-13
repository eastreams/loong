use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Capability;

/// Effective capability authority exposed to policy evaluation.
///
/// The collection is opaque so policy consumers depend on set semantics rather
/// than its current storage representation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capabilities(BTreeSet<Capability>);

impl Capabilities {
    #[must_use]
    pub const fn new() -> Self {
        Self(BTreeSet::new())
    }

    #[must_use]
    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }

    #[must_use]
    pub fn is_subset(&self, other: &Self) -> bool {
        self.0.is_subset(&other.0)
    }

    pub fn difference<'a>(&'a self, other: &'a Self) -> impl Iterator<Item = Capability> + 'a {
        self.0.difference(&other.0).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
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
}
