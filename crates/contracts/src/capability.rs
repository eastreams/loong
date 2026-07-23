//! Capabilities and capability sets.
//!
//! Serialize capabilities by their string names. Numeric IDs only index the
//! private bitset and must never appear in stored or exchanged data.

use std::{borrow::Cow, collections::BTreeSet};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

const CAPABILITY_BITS: usize = 8;
const CAPABILITY_BYTES: usize = CAPABILITY_BITS.div_ceil(u8::BITS as usize);

// Keep the private bit position and stable wire name in one declaration.
macro_rules! define_capabilities {
    ($($(#[$meta:meta])* $variant:ident = $id:literal => $wire_name:literal),* $(,)?) => {
        /// A named permission that policy may grant to an action.
        ///
        /// Bit positions are private storage details. Serialization uses only
        /// the explicit stable name attached to each variant.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, JsonSchema)]
        #[non_exhaustive]
        pub enum Capability {
            $(
                $(#[$meta])*
                #[schemars(rename = $wire_name)]
                $variant,
            )*
        }

        impl Capability {
            const WIRE_NAMES: &'static [&'static str] = &[$($wire_name,)*];

            const fn into_id(self) -> u8 {
                match self {
                    $(Self::$variant => $id,)*
                }
            }

            const fn from_id(id: u8) -> Option<Self> {
                match id {
                    $($id => Some(Self::$variant),)*
                    _ => None,
                }
            }

            const fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire_name,)*
                }
            }

            fn from_wire_name(wire_name: &str) -> Option<Self> {
                match wire_name {
                    $($wire_name => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        $(
            const _: () = assert!(
                ($id as usize) < CAPABILITY_BITS,
                "capability ID exceeds bitset capacity",
            );
        )*
    };
}

define_capabilities! {
    FsRead = 0 => "fs.read",
    FsWrite = 1 => "fs.write",
}

impl Serialize for Capability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.wire_name())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire_name = String::deserialize(deserializer)?;
        Self::from_wire_name(&wire_name)
            .ok_or_else(|| D::Error::unknown_variant(&wire_name, Self::WIRE_NAMES))
    }
}

/// A compact set of capabilities.
///
/// Its stable serialized form is a sequence of capability names sorted
/// lexicographically. The private bitset and numeric IDs never cross the
/// contract boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Capabilities([u8; CAPABILITY_BYTES]);

impl Capabilities {
    #[must_use]
    pub const fn empty() -> Self {
        Self([0; CAPABILITY_BYTES])
    }

    #[must_use]
    pub const fn singleton(capability: Capability) -> Self {
        Self::empty().with(capability)
    }

    #[must_use]
    pub const fn with(mut self, capability: Capability) -> Self {
        let id = capability.into_id() as usize;
        let byte = id / u8::BITS as usize;
        let bit = id % u8::BITS as usize;
        self.0[byte] |= 1u8 << bit;
        self
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::empty(), |caps, cap| caps.with(cap))
    }
}

impl From<Capability> for Capabilities {
    fn from(capability: Capability) -> Self {
        Self::singleton(capability)
    }
}

impl Serialize for Capabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut names: Vec<_> = (*self).into_iter().map(Capability::wire_name).collect();
        names.sort_unstable();
        names.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Capabilities {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<Capability>::deserialize(deserializer).map(|caps| caps.into_iter().collect())
    }
}

impl JsonSchema for Capabilities {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Capabilities")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <BTreeSet<Capability>>::json_schema(generator)
    }
}

/// An iterator over the capabilities stored in a set.
#[derive(Debug, Clone)]
pub struct CapabilitiesIntoIter {
    remaining: Capabilities,
    byte_index: usize,
}

impl Iterator for CapabilitiesIntoIter {
    type Item = Capability;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let byte_index = self.byte_index;
            let byte = self.remaining.0.get_mut(byte_index)?;

            if *byte == 0 {
                self.byte_index += 1;
                continue;
            }

            let bit_index = byte.trailing_zeros() as usize;
            *byte &= *byte - 1; // remove lowbit

            let id = byte_index * u8::BITS as usize + bit_index;
            let Ok(id) = u8::try_from(id) else {
                continue;
            };
            if let Some(capability) = Capability::from_id(id) {
                return Some(capability);
            }
        }
    }
}

impl std::iter::FusedIterator for CapabilitiesIntoIter {}

impl IntoIterator for Capabilities {
    type Item = Capability;
    type IntoIter = CapabilitiesIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        CapabilitiesIntoIter {
            remaining: self,
            byte_index: 0,
        }
    }
}

#[cfg(test)]
mod tests;
