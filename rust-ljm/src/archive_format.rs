//! On-disk format shared by the archiver and the recompress tool.

use parquet::{
    basic::{Compression, Encoding, ZstdLevel},
    file::{metadata::KeyValue, properties::WriterProperties},
    schema::types::ColumnPath,
};

/// Two-column schema of every archived sample file.
pub const SAMPLE_SCHEMA: &str = "
    message schema {
        REQUIRED INT64 timestamp_unix_ns;
        REQUIRED DOUBLE value;
    }
";

/// Parquet writer settings for the two-column sample schema.
///
/// Timestamps are strictly regular, so delta encoding reduces them to almost
/// nothing. Values come from a 16-bit converter and repeat heavily, so they
/// keep dictionary encoding. zstd compresses the result. Measured on a real
/// 2 kHz MU1 file this is about 14x smaller than uncompressed PLAIN output.
pub fn writer_properties(key_value_metadata: Vec<KeyValue>) -> WriterProperties {
    let timestamp = ColumnPath::from("timestamp_unix_ns");
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .set_column_dictionary_enabled(timestamp.clone(), false)
        .set_column_encoding(timestamp, Encoding::DELTA_BINARY_PACKED)
        .set_key_value_metadata(Some(key_value_metadata))
        .build()
}

/// Writer settings for a file carrying one calibration JSON document.
#[allow(dead_code)]
pub fn writer_properties_for_calibration(calibration_json: String) -> WriterProperties {
    writer_properties(vec![KeyValue::new(
        "calibration".to_string(),
        calibration_json,
    )])
}
