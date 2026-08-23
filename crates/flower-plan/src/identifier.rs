use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de};
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
}
