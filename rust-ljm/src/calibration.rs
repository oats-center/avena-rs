//! Calibration formulas stored with archived channel data.
//!
//! The archiver writes a serialized [`CalibrationSpec`] into each Parquet file's
//! metadata. The exporter reads that metadata back and applies the formula while
//! generating CSV rows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Formula used to convert a raw LabJack voltage into a calibrated value.
pub enum CalibrationFormula {
    /// Leaves the raw value unchanged.
    Identity,
    /// Applies `a * raw + b`.
    Linear { a: f64, b: f64 },
    /// Applies a polynomial where `coeffs[i] * raw.powi(i)`.
    Polynomial { coeffs: Vec<f64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Named calibration definition for one channel.
pub struct CalibrationSpec {
    /// Optional identifier included in exported CSV rows.
    pub id: Option<String>,
    /// Formula details flattened into the dashboard JSON shape.
    #[serde(flatten)]
    pub formula: CalibrationFormula,
}

#[allow(dead_code)]
impl CalibrationSpec {
    /// Applies the calibration formula to one raw sample value.
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

    /// Returns the configured calibration id, or `identity` when unnamed.
    pub fn id_or_default(&self) -> &str {
        self.id.as_deref().unwrap_or("identity")
    }
}

impl Default for CalibrationSpec {
    /// Creates an unnamed identity calibration.
    fn default() -> Self {
        Self {
            id: None,
            formula: CalibrationFormula::Identity,
        }
    }
}
