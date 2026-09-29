//! Reading and writing whole archived sample files.
//!
//! Shared by the `recompress` and `dedupe` tools. Both read an archived file fully
//! into memory with [`read_archive`] and write files in the current format with
//! [`write_archive`], which uses the same schema and writer settings as the archiver.

use std::{fs, path::Path, sync::Arc};

use anyhow::{Result, anyhow, bail};
use parquet::{
    column::{reader::get_typed_column_reader, writer::ColumnWriter},
    data_type::{DataType, DoubleType, Int64Type},
    file::{
        metadata::KeyValue,
        reader::{FileReader, RowGroupReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    schema::parser::parse_message_type,
};

use crate::archive_format::{SAMPLE_SCHEMA, writer_properties};

/// Rows per row group in rewritten files (1,048,576); matches the archiver.
///
/// Old files may have many small row groups. Merging them into large groups gives
/// zstd and delta encoding longer runs to work on.
pub const ROWS_PER_ROW_GROUP: usize = 1 << 20;
/// Contents of one archived file, held fully in memory.
pub struct Archive {
    /// `timestamp_unix_ns` column, in file order (Unix nanoseconds).
    pub timestamps: Vec<i64>,
    /// `value` column, in file order, one per timestamp.
    pub values: Vec<f64>,
    /// File-level key-value metadata, such as the `calibration` JSON document.
    pub metadata: Vec<KeyValue>,
}

/// Reads every value of one column in a row group.
///
/// Records are read in batches of up to the row group's row count until the reader
/// returns none. The columns are `REQUIRED`, so no definition or repetition levels are
/// requested.
///
/// # Arguments
///
/// * `row_group` - Row group to read from.
/// * `index` - Column index (0 for `timestamp_unix_ns`, 1 for `value`).
///
/// # Returns
///
/// The column's values in file order.
///
/// # Errors
///
/// Returns an error if the column reader cannot be created or a page cannot be read
/// or decoded.
///
/// # Panics
///
/// Panics if the physical type of column `index` does not match `T`.
fn read_column<T: DataType>(row_group: &dyn RowGroupReader, index: usize) -> Result<Vec<T::T>> {
    let rows = usize::try_from(row_group.metadata().num_rows()).unwrap_or(0);
    let mut reader = get_typed_column_reader::<T>(row_group.get_column_reader(index)?);
    let mut values = Vec::with_capacity(rows);
    loop {
        let (records, _, _) = reader.read_records(rows.max(1), None, None, &mut values)?;
        if records == 0 {
            break;
        }
    }
    Ok(values)
}

/// Reads every row and the key-value metadata of an archived file.
///
/// Row groups are concatenated in order. A file without key-value metadata gives an
/// empty `metadata` list.
///
/// # Arguments
///
/// * `reader` - Open reader for the file.
///
/// # Errors
///
/// Returns an error if the columns are not exactly `timestamp_unix_ns` and `value`, if
/// a row group cannot be read, or if a row group's timestamp count, value count and
/// row count disagree.
pub fn read_archive(reader: &SerializedFileReader<fs::File>) -> Result<Archive> {
    let schema = reader.metadata().file_metadata().schema_descr();
    let names: Vec<&str> = schema.columns().iter().map(|c| c.name()).collect();
    if names != ["timestamp_unix_ns", "value"] {
        bail!("unexpected columns {names:?}");
    }
    let mut timestamps = Vec::new();
    let mut values = Vec::new();
    for i in 0..reader.num_row_groups() {
        let rg = reader.get_row_group(i)?;
        let ts = read_column::<Int64Type>(rg.as_ref(), 0)?;
        let vs = read_column::<DoubleType>(rg.as_ref(), 1)?;
        if ts.len() != vs.len() || ts.len() as i64 != rg.metadata().num_rows() {
            bail!(
                "row group {i}: {} timestamps, {} values",
                ts.len(),
                vs.len()
            );
        }
        timestamps.extend(ts);
        values.extend(vs);
    }
    let metadata = reader
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .cloned()
        .unwrap_or_default();
    Ok(Archive {
        timestamps,
        values,
        metadata,
    })
}

/// Writes rows in the current archive format and syncs the file to disk.
///
/// Uses [`SAMPLE_SCHEMA`] and [`writer_properties`] (the same settings as the
/// archiver) with the archive's original key-value metadata. Rows are split into row
/// groups of [`ROWS_PER_ROW_GROUP`]. An existing file at `path` is truncated.
///
/// # Arguments
///
/// * `path` - Destination file, normally the temporary `*.recompress-tmp` path.
/// * `archive` - Rows and metadata to write.
///
/// # Errors
///
/// Returns an error if the schema cannot be parsed, or if creating, writing, closing
/// or syncing the file fails.
pub fn write_archive(path: &Path, archive: &Archive) -> Result<()> {
    let schema = Arc::new(parse_message_type(SAMPLE_SCHEMA)?);
    let props = Arc::new(writer_properties(archive.metadata.clone()));
    let mut writer = SerializedFileWriter::new(fs::File::create(path)?, schema, props)?;
    for (ts, vs) in archive
        .timestamps
        .chunks(ROWS_PER_ROW_GROUP)
        .zip(archive.values.chunks(ROWS_PER_ROW_GROUP))
    {
        let mut rg = writer.next_row_group()?;
        let mut col = rg
            .next_column()?
            .ok_or_else(|| anyhow!("missing column 0"))?;
        if let ColumnWriter::Int64ColumnWriter(w) = col.untyped() {
            w.write_batch(ts, None, None)?;
        }
        col.close()?;
        let mut col = rg
            .next_column()?
            .ok_or_else(|| anyhow!("missing column 1"))?;
        if let ColumnWriter::DoubleColumnWriter(w) = col.untyped() {
            w.write_batch(vs, None, None)?;
        }
        col.close()?;
        rg.close()?;
    }
    writer.into_inner()?.sync_all()?;
    Ok(())
}
