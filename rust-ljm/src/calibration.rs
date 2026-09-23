//! Calibration formulas stored with archived channel data.
//!
//! Calibrations come from the `calibrations` map in the dashboard sensor settings.
//! The archiver writes the channel's [`CalibrationSpec`] as JSON into each Parquet
//! file's key-value metadata under the key `calibration`. The exporter reads that
//! metadata back and applies the formula while generating CSV rows, so an export
//! uses the calibration that was in force when the file was written.
//!
//! The JSON shape is the formula's fields plus an optional `id`, tagged by `type`:
//!
//! ```text
//! {"type":"identity"}
//! {"id":"tp3505","type":"linear","a":2.0,"b":-1.0}
//! {"type":"polynomial","coeffs":[0.1,2.0,0.03]}
//! ```

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Formula used to convert a raw LabJack reading into a calibrated value.
///
/// Serialized with an internal `type` tag in snake_case (`identity`, `linear`,
/// `polynomial`).
pub enum CalibrationFormula {
    /// Leaves the raw value unchanged.
    Identity,
    /// Applies `a * raw + b`, where `a` is the slope and `b` the offset.
    Linear { a: f64, b: f64 },
    /// Applies the polynomial `sum(coeffs[i] * raw.powi(i))`.
    ///
    /// `coeffs` is in ascending power order, so `coeffs[0]` is the constant term.
    /// An empty list evaluates to `0.0`.
    Polynomial { coeffs: Vec<f64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Named calibration definition for one channel.
///
/// Deserialized from the dashboard sensor settings by the archiver and from Parquet
/// metadata by the exporter. The default is an unnamed identity calibration.
pub struct CalibrationSpec {
    /// Optional identifier. The exporter writes it in the `calibration_id` CSV
    /// column, or `identity` when it is `None` (see [`Self::id_or_default`]).
    pub id: Option<String>,
    /// Formula; its `type` tag and fields sit at the top level of the JSON object
    /// next to `id` (serde `flatten`).
    #[serde(flatten)]
    pub formula: CalibrationFormula,
}

// The archiver compiles this module too but only stores specs, so it never calls
// these methods.
#[allow(dead_code)]
impl CalibrationSpec {
    /// Applies the calibration formula to one raw sample value.
    ///
    /// NaN and infinite inputs propagate through the arithmetic unchanged.
    ///
    /// # Arguments
    ///
    /// * `raw` - Raw sample value as stored in the archive.
    ///
    /// # Returns
    ///
    /// The calibrated value.
    ///
    /// # Examples
    ///
    /// ```text
    /// Linear { a: 2.0, b: -1.0 }             applied to 3.0 == 5.0
    /// Polynomial { coeffs: [1.0, 0.0, 2.0] } applied to 3.0 == 1 + 2 * 9 == 19.0
    /// ```
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
    ///
    /// The fallback is the string `identity` even if the formula is not the
    /// identity.
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
