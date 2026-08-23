use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdentifierError {
    #[error("identifier must not be empty")]
    Empty,
    #[error("identifier `{value}` contains unsupported characters")]
    UnsupportedCharacters { value: String },
}

macro_rules! define_identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(IdentifierError::Empty);
                }
                if !value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
                {
                    return Err(IdentifierError::UnsupportedCharacters { value });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

define_identifier!(WorkflowId);
define_identifier!(NodeId);
define_identifier!(EdgeId);
define_identifier!(ExecutionId);
define_identifier!(EventId);
define_identifier!(EffectId);

impl EffectId {
    const DOMAIN_SEPARATOR: &'static [u8] = b"flower/effect/v1";
    const EXECUTE_NODE_KIND: &'static [u8] = b"execute-node";

    /// Derives the globally unique identity of one `ExecuteNode` intent.
    ///
    /// `execution_id` is required to be globally unique. The revision and node
    /// identity distinguish the intents emitted by that execution.
    pub fn derive_execute_node(
        execution_id: &ExecutionId,
        execution_revision: u64,
        node_id: &NodeId,
    ) -> Self {
        let mut canonical_input = Sha256::new();
        canonical_input.update(Self::DOMAIN_SEPARATOR);
        update_length_prefixed(&mut canonical_input, Self::EXECUTE_NODE_KIND);
        update_length_prefixed(&mut canonical_input, execution_id.as_str().as_bytes());
        update_length_prefixed(&mut canonical_input, &execution_revision.to_be_bytes());
        update_length_prefixed(&mut canonical_input, node_id.as_str().as_bytes());

        let digest = canonical_input.finalize();
        Self::new(format!("effect-{digest:x}"))
            .expect("a SHA-256 effect identity always satisfies the identifier grammar")
    }
}

fn update_length_prefixed(hasher: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("an in-memory identifier length fits into u64");
    hasher.update(length.to_be_bytes());
    hasher.update(value);
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PlanFingerprint(String);

impl PlanFingerprint {
    pub fn from_sha256(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentifierError::UnsupportedCharacters { value });
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlanFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for PlanFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_sha256(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_identifiers_that_cannot_be_shared_across_bindings() {
        assert_eq!(NodeId::new(""), Err(IdentifierError::Empty));
        assert!(matches!(
            NodeId::new("node/one"),
            Err(IdentifierError::UnsupportedCharacters { .. })
        ));
    }

    #[test]
    fn execute_node_effect_identity_is_a_frozen_canonical_hash() {
        let effect_id = EffectId::derive_execute_node(
            &ExecutionId::new("01J6083Y8Y5P7FSJ6Z3K1Q9R4X").unwrap(),
            42,
            &NodeId::new("billing.capture").unwrap(),
        );

        assert_eq!(
            effect_id.as_str(),
            "effect-4a41a09004b033cf3ebd4f6d23f0b319e43f59325597a2bce666b264216888ac"
        );
    }
}
