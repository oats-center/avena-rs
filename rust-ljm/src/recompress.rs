//! Rewrites older archived Parquet files in the current archive format.
//!
//! Usage: `recompress <parquet_root> [--dry-run]`
//!
//! Each `part-*.parquet` file that is not already zstd with delta-encoded
//! timestamps is rewritten next to itself as `*.parquet.recompress-tmp`,
//! synced, read back and compared value by value (bit for bit, so NaN counts
//! as equal only to the identical NaN) together with its key-value metadata.
//! Only then is the original replaced by an atomic rename, so every file is
//! at all times either the complete old version or the verified new one.
//! Unfinished (`.inprogress`) and quarantined files are never touched, and the
//! tool can be interrupted and rerun: finished files are skipped.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use parquet::{
    basic::{Compression, Encoding},
    column::{reader::get_typed_column_reader, writer::ColumnWriter},
    data_type::{DataType, DoubleType, Int64Type},
    file::{
        metadata::KeyValue,
        reader::{FileReader, RowGroupReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    schema::parser::parse_message_type,
};

mod archive_format;

use archive_format::{SAMPLE_SCHEMA, writer_properties};

/// Rows per row group in rewritten files; matches the archiver.
const ROWS_PER_ROW_GROUP: usize = 1 << 20;
const TMP_SUFFIX: &str = ".recompress-tmp";

/// Contents of one archived file.
struct Archive {
    timestamps: Vec<i64>,
    values: Vec<f64>,
    metadata: Vec<KeyValue>,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Rewritten {
        bytes_before: u64,
        bytes_after: u64,
        rows: usize,
    },
    AlreadyCurrent,
}

/// Returns whether a file already uses the current archive format.
fn is_current_format(reader: &SerializedFileReader<fs::File>) -> bool {
    let meta = reader.metadata();
    meta.num_row_groups() > 0
        && (0..meta.num_row_groups()).all(|i| {
            let rg = meta.row_group(i);
            rg.num_columns() == 2
                && (0..2).all(|c| matches!(rg.column(c).compression(), Compression::ZSTD(_)))
                && rg
                    .column(0)
                    .encodings()
                    .contains(&Encoding::DELTA_BINARY_PACKED)
        })
}

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
fn read_archive(reader: &SerializedFileReader<fs::File>) -> Result<Archive> {
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
fn write_archive(path: &Path, archive: &Archive) -> Result<()> {
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

/// Checks that two archives hold exactly the same rows and metadata.
fn verify_identical(original: &Archive, rewritten: &Archive) -> Result<()> {
    if original.timestamps != rewritten.timestamps {
        bail!("timestamps differ");
    }
    if original.values.len() != rewritten.values.len()
        || original
            .values
            .iter()
            .zip(&rewritten.values)
            .any(|(a, b)| a.to_bits() != b.to_bits())
    {
        bail!("values differ");
    }
    let kv = |a: &Archive| {
        a.metadata
            .iter()
            .map(|k| (k.key.clone(), k.value.clone()))
            .collect::<Vec<_>>()
    };
    if kv(original) != kv(rewritten) {
        bail!("key-value metadata differs");
    }
    Ok(())
}

/// Rewrites one file in the current format after verifying the copy.
fn recompress_file(path: &Path, dry_run: bool) -> Result<Outcome> {
    let reader = SerializedFileReader::new(fs::File::open(path)?)?;
    if is_current_format(&reader) {
        return Ok(Outcome::AlreadyCurrent);
    }
    let original = read_archive(&reader)?;
    let bytes_before = fs::metadata(path)?.len();
    let rows = original.timestamps.len();

    let tmp = PathBuf::from(format!("{}{TMP_SUFFIX}", path.display()));
    let result = (|| -> Result<u64> {
        write_archive(&tmp, &original)?;
        let copy = SerializedFileReader::new(fs::File::open(&tmp)?)?;
        if !is_current_format(&copy) {
            bail!("rewritten file is not in the current format");
        }
        verify_identical(&original, &read_archive(&copy)?)?;
        Ok(fs::metadata(&tmp)?.len())
    })();
    let bytes_after = match result {
        Ok(size) => size,
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    };

    if dry_run {
        fs::remove_file(&tmp)?;
    } else {
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            fs::File::open(dir)?.sync_all()?;
        }
    }
    Ok(Outcome::Rewritten {
        bytes_before,
        bytes_after,
        rows,
    })
}

/// Lists archived `part-*.parquet` files and removes stale temporary copies.
fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, files)?;
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(TMP_SUFFIX) {
            fs::remove_file(&path)?;
            println!("removed stale {}", path.display());
        } else if name.starts_with("part-") && name.ends_with(".parquet") {
            files.push(path);
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or_else(|| anyhow!("usage: recompress <parquet_root> [--dry-run]"))?;
    let dry_run = args.iter().any(|a| a == "--dry-run");

    let mut files = Vec::new();
    collect_files(Path::new(root), &mut files)?;
    files.sort();
    println!(
        "{} archived files under {root}{}",
        files.len(),
        if dry_run { " (dry run)" } else { "" }
    );

    let (mut rewritten, mut current, mut failed) = (0usize, 0usize, 0usize);
    let (mut before, mut after, mut rows) = (0u64, 0u64, 0usize);
    for (i, path) in files.iter().enumerate() {
        match recompress_file(path, dry_run) {
            Ok(Outcome::Rewritten {
                bytes_before,
                bytes_after,
                rows: n,
            }) => {
                rewritten += 1;
                before += bytes_before;
                after += bytes_after;
                rows += n;
            }
            Ok(Outcome::AlreadyCurrent) => current += 1,
            Err(err) => {
                failed += 1;
                eprintln!("FAILED {}: {err:#}", path.display());
            }
        }
        if (i + 1) % 1000 == 0 {
            println!(
                "{}/{} files; rewritten {rewritten}, {:.2} GB -> {:.2} GB",
                i + 1,
                files.len(),
                before as f64 / 1e9,
                after as f64 / 1e9
            );
        }
    }
    println!(
        "done: rewritten {rewritten} ({rows} rows, {:.2} GB -> {:.2} GB), already current {current}, failed {failed}",
        before as f64 / 1e9,
        after as f64 / 1e9
    );
    if failed > 0 {
        bail!("{failed} file(s) failed; their originals were left untouched");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parquet::file::properties::WriterProperties;

    /// Writes a file the way the archiver did before zstd: plain, many row groups.
    fn write_old_format(path: &Path, groups: &[Vec<(i64, f64)>], calibration: &str) {
        let schema = Arc::new(parse_message_type(SAMPLE_SCHEMA).unwrap());
        let props = Arc::new(
            WriterProperties::builder()
                .set_key_value_metadata(Some(vec![KeyValue::new(
                    "calibration".to_string(),
                    calibration.to_string(),
                )]))
                .build(),
        );
        let mut writer =
            SerializedFileWriter::new(fs::File::create(path).unwrap(), schema, props).unwrap();
        for group in groups {
            let mut rg = writer.next_row_group().unwrap();
            let ts: Vec<i64> = group.iter().map(|r| r.0).collect();
            let vs: Vec<f64> = group.iter().map(|r| r.1).collect();
            let mut col = rg.next_column().unwrap().unwrap();
            if let ColumnWriter::Int64ColumnWriter(w) = col.untyped() {
                w.write_batch(&ts, None, None).unwrap();
            }
            col.close().unwrap();
            let mut col = rg.next_column().unwrap().unwrap();
            if let ColumnWriter::DoubleColumnWriter(w) = col.untyped() {
                w.write_batch(&vs, None, None).unwrap();
            }
            col.close().unwrap();
            rg.close().unwrap();
        }
        writer.close().unwrap();
    }

    #[test]
    fn old_file_is_rewritten_identically_then_skipped() {
        let dir = std::env::temp_dir().join(format!("recompress-test-{}", uuid::Uuid::new_v4()));
        let day = dir.join("asset1001/2026-09-01/ch08");
        fs::create_dir_all(&day).unwrap();
        let path = day.join("part-0001.parquet");
        let base = 1_788_220_800_000_000_000_i64;
        let groups: Vec<Vec<(i64, f64)>> = (0..5)
            .map(|g| {
                (0..1000)
                    .map(|i| {
                        let k = g * 1000 + i;
                        (
                            base + k * 500_000,
                            3.7 + ((k * 7919) % 97) as f64 * 0.000_315_6,
                        )
                    })
                    .collect()
            })
            .collect();
        let mut groups = groups;
        groups[2][10].1 = f64::NAN;
        groups[4][999].0 -= 3_000_000_000; // an out-of-order timestamp survives as-is
        let calibration = r#"{"id":"tp3505","type":"linear","a":70.2,"b":-9.1}"#;
        write_old_format(&path, &groups, calibration);
        let original =
            read_archive(&SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap())
                .unwrap();

        // A dry run verifies but leaves the original in place.
        let dry = recompress_file(&path, true).unwrap();
        assert!(matches!(dry, Outcome::Rewritten { rows: 5000, .. }));
        let reader = SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap();
        assert!(!is_current_format(&reader));

        let outcome = recompress_file(&path, false).unwrap();
        let Outcome::Rewritten {
            bytes_before,
            bytes_after,
            rows,
        } = outcome
        else {
            panic!("expected a rewrite");
        };
        assert_eq!(rows, 5000);
        assert!(bytes_after < bytes_before);

        let reader = SerializedFileReader::new(fs::File::open(&path).unwrap()).unwrap();
        assert!(is_current_format(&reader));
        assert_eq!(reader.num_row_groups(), 1);
        verify_identical(&original, &read_archive(&reader).unwrap()).unwrap();
        assert!(!PathBuf::from(format!("{}{TMP_SUFFIX}", path.display())).exists());

        assert_eq!(
            recompress_file(&path, false).unwrap(),
            Outcome::AlreadyCurrent
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verification_detects_any_difference() {
        let a = Archive {
            timestamps: vec![1, 2, 3],
            values: vec![1.0, f64::NAN, 3.0],
            metadata: vec![KeyValue::new("calibration".into(), "{}".to_string())],
        };
        let same = Archive {
            timestamps: a.timestamps.clone(),
            values: a.values.clone(),
            metadata: a.metadata.clone(),
        };
        verify_identical(&a, &same).unwrap();
        let mut b = Archive {
            timestamps: vec![1, 2, 4],
            ..same
        };
        assert!(verify_identical(&a, &b).is_err());
        b.timestamps = a.timestamps.clone();
        b.values[2] = 3.000_000_000_000_001;
        assert!(verify_identical(&a, &b).is_err());
        b.values = a.values.clone();
        b.metadata = vec![KeyValue::new("calibration".into(), "{ }".to_string())];
        assert!(verify_identical(&a, &b).is_err());
    }

    #[test]
    fn stale_temporary_files_are_removed_and_others_ignored() {
        let dir = std::env::temp_dir().join(format!("recompress-walk-{}", uuid::Uuid::new_v4()));
        let day = dir.join("asset1001/2026-09-01/ch08");
        fs::create_dir_all(&day).unwrap();
        for name in [
            "part-0001.parquet",
            "part-0002.parquet.inprogress",
            "part-0003.parquet.inprogress.unfinished.quarantined-1-0",
            "part-0004.parquet.recompress-tmp",
            "notes.txt",
        ] {
            fs::write(day.join(name), b"x").unwrap();
        }
        let mut files = Vec::new();
        collect_files(&dir, &mut files).unwrap();
        assert_eq!(files, vec![day.join("part-0001.parquet")]);
        assert!(!day.join("part-0004.parquet.recompress-tmp").exists());
        assert!(day.join("part-0002.parquet.inprogress").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
