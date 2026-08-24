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
define_identifier!(EffectId);
define_identifier!(NodeActivationId);
define_identifier!(AttemptId);
define_identifier!(FailureCode);
define_identifier!(TimerId);

impl NodeActivationId {
    pub fn derive(execution_id: &ExecutionId, activation_revision: u64, node_id: &NodeId) -> Self {
        let digest = canonical_hash(
            b"flower/node-activation/v2",
            [
                execution_id.as_str().as_bytes(),
                activation_revision.to_be_bytes().as_slice(),
                node_id.as_str().as_bytes(),
            ],
        );
        Self::new(format!("activation-{digest:x}"))
            .expect("a SHA-256 activation identity satisfies the identifier grammar")
    }
}

impl AttemptId {
    pub fn derive(activation_id: &NodeActivationId, attempt_number: u32) -> Self {
        let digest = canonical_hash(
            b"flower/attempt/v2",
            [
                activation_id.as_str().as_bytes(),
                attempt_number.to_be_bytes().as_slice(),
            ],
        );
        Self::new(format!("attempt-{digest:x}"))
            .expect("a SHA-256 attempt identity satisfies the identifier grammar")
    }
}

impl EffectId {
    pub fn derive_execute_node_attempt(
        activation_id: &NodeActivationId,
        attempt_id: &AttemptId,
        attempt_number: u32,
        node_id: &NodeId,
    ) -> Self {
        let digest = canonical_hash(
            b"flower/effect/execute-node-attempt/v2",
            [
                activation_id.as_str().as_bytes(),
                attempt_id.as_str().as_bytes(),
                attempt_number.to_be_bytes().as_slice(),
                node_id.as_str().as_bytes(),
            ],
        );
        Self::new(format!("effect-{digest:x}"))
            .expect("a SHA-256 effect identity satisfies the identifier grammar")
    }

    pub fn derive_schedule_timer(
        timer_id: &TimerId,
        activation_id: &NodeActivationId,
        failed_attempt_id: &AttemptId,
        next_attempt_number: u32,
    ) -> Self {
        let digest = canonical_hash(
            b"flower/effect/schedule-timer/v2",
            [
                timer_id.as_str().as_bytes(),
                activation_id.as_str().as_bytes(),
                failed_attempt_id.as_str().as_bytes(),
                next_attempt_number.to_be_bytes().as_slice(),
            ],
        );
        Self::new(format!("effect-{digest:x}"))
            .expect("a SHA-256 effect identity satisfies the identifier grammar")
    }
}

impl TimerId {
    pub fn derive_retry(
        activation_id: &NodeActivationId,
        failed_attempt_id: &AttemptId,
        next_attempt_number: u32,
    ) -> Self {
        let digest = canonical_hash(
            b"flower/timer/retry/v2",
            [
                activation_id.as_str().as_bytes(),
                failed_attempt_id.as_str().as_bytes(),
                next_attempt_number.to_be_bytes().as_slice(),
            ],
        );
        Self::new(format!("timer-{digest:x}"))
            .expect("a SHA-256 timer identity satisfies the identifier grammar")
    }
}

fn canonical_hash<'a>(
    domain_separator: &[u8],
    fields: impl IntoIterator<Item = &'a [u8]>,
) -> sha2::digest::Output<Sha256> {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, domain_separator);
    for field in fields {
        update_length_prefixed(&mut hasher, field);
    }
    hasher.finalize()
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
    fn attempt_identity_chain_uses_frozen_canonical_hashes() {
        let execution_id = ExecutionId::new("01J6083Y8Y5P7FSJ6Z3K1Q9R4X").unwrap();
        let node_id = NodeId::new("billing.capture").unwrap();
        let activation_id = NodeActivationId::derive(&execution_id, 42, &node_id);
        let attempt_id = AttemptId::derive(&activation_id, 1);
        let effect_id =
            EffectId::derive_execute_node_attempt(&activation_id, &attempt_id, 1, &node_id);

        assert_eq!(
            activation_id.as_str(),
            "activation-dcf726e6fe44d79080e73756e5e590d5884c15bf2eaf6c8ff6f9c273858788ab"
        );
        assert_eq!(
            attempt_id.as_str(),
            "attempt-09323bff6cd3a444e59fb34c1116e8713b9c93293c4e6d19964f67e968b17f1e"
        );
        assert_eq!(
            effect_id.as_str(),
            "effect-bce6eb32033fe614537ddb77295e1f2d8d125a80d14893831887a598223f661f"
        );
    }

    #[test]
    fn retry_timer_identity_chain_is_deterministic() {
        let activation_id = NodeActivationId::new("activation-golden").unwrap();
        let failed_attempt_id = AttemptId::new("attempt-golden").unwrap();
        let timer_id = TimerId::derive_retry(&activation_id, &failed_attempt_id, 2);
        let effect_id =
            EffectId::derive_schedule_timer(&timer_id, &activation_id, &failed_attempt_id, 2);

        assert!(timer_id.as_str().starts_with("timer-"));
        assert!(effect_id.as_str().starts_with("effect-"));
        assert_eq!(
            timer_id,
            TimerId::derive_retry(&activation_id, &failed_attempt_id, 2)
        );
    }
}
