use std::fmt;

#[derive(Debug)]
pub enum CoreError {
    InvalidInput,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => write!(f, "invalid input"),
        }
    }
}

impl std::error::Error for CoreError {}
