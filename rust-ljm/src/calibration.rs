use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CalibrationFormula {
    Identity,
    Linear { a: f64, b: f64 },
    Polynomial { coeffs: Vec<f64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibrationSpec {
    pub id: Option<String>,
    #[serde(flatten)]
    pub formula: CalibrationFormula,
}

impl CalibrationSpec {
    pub fn apply(&self, raw: f64) -> f64 {
        match &self.formula {
            CalibrationFormula::Identity => raw,
            CalibrationFormula::Linear { a, b } => a * raw + b,
            CalibrationFormula::Polynomial { coeffs } => coeffs
                .iter()
                .enumerate()
                .map(|(idx, coeff)| coeff * raw.powi(idx as i32))
                .sum(),
        }
    }

    pub fn id_or_default(&self) -> &str {
        self.id.as_deref().unwrap_or("identity")
    }
}

impl Default for CalibrationSpec {
    fn default() -> Self {
        Self {
            id: None,
            formula: CalibrationFormula::Identity,
        }
    }
}
