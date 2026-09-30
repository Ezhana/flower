#[derive(Debug, Clone, Copy)]
pub struct Calculator {
    accumulator: f64,
}

impl Calculator {
    pub fn new(initial: f64) -> Self {
        Self { accumulator: initial }
    }

    pub fn add(&mut self, value: f64) {
        self.accumulator += value;
    }

    pub fn multiply(&mut self, value: f64) {
        self.accumulator *= value;
    }

    pub fn value(&self) -> f64 {
        self.accumulator
    }

    /// Core-level domain rule.
    pub fn apply_discount(&mut self, percent: f64) -> Result<(), &'static str> {
        if !(0.0..=100.0).contains(&percent) {
            return Err("discount percent must be between 0 and 100");
        }

        self.accumulator *= 1.0 - percent / 100.0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Calculator;

    #[test]
    fn calculator_flow() {
        let mut calc = Calculator::new(100.0);

        calc.add(50.0);
        calc.multiply(2.0);
        calc.apply_discount(10.0).unwrap();

        assert_eq!(calc.value(), 270.0);
    }
}