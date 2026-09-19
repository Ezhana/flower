use crate::CoreError;

#[derive(Debug, Default)]
pub struct Calculator;

impl Calculator {
    pub fn new() -> Self {
        Self
    }

    pub fn add(&self, a: i32, b: i32) -> Result<i32, CoreError> {
        Ok(a + b)
    }
}
