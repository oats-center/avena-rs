//! On-disk format shared by the archiver and the recompress tool.
//!
//! The `archiver` binary writes every sample file with this schema and these writer
//! settings, and the `recompress` tool rewrites older files into the same format. The
//! exporter reads the files back and serves them as CSV over NATS. Keeping the format in
//! one module means both writers produce identical files.

use parquet::{
    basic::{Compression, Encoding, ZstdLevel},
    file::{metadata::KeyValue, properties::WriterProperties},
    schema::types::ColumnPath,
};

/// Two-column schema of every archived sample file.
///
/// `timestamp_unix_ns` is the sample time in Unix nanoseconds and `value` is the raw
/// (uncalibrated) sample value. Both columns are required.
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
///
/// In detail: zstd at the `parquet` crate's default level for all columns, dictionary
/// encoding off and `DELTA_BINARY_PACKED` on for `timestamp_unix_ns`, and the writer's
/// defaults for `value`.
///
/// # Arguments
///
/// * `key_value_metadata` - File-level key-value metadata to store in the footer, for
///   example the calibration entry.
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
///
/// Same as [`writer_properties`] with a single metadata entry under the key
/// `calibration`. Used by the archiver; `dead_code` is allowed because the recompress
/// tool, which also includes this module, does not call it.
///
/// # Arguments
///
/// * `calibration_json` - Serialized calibration spec for the file's channel.
#[allow(dead_code)]
pub fn writer_properties_for_calibration(calibration_json: String) -> WriterProperties {
    writer_properties(vec![KeyValue::new(
        "calibration".to_string(),
        calibration_json,
    )])
}
