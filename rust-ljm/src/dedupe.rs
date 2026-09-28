//! Offline tool that removes duplicate samples from archived channel folders.
//!
//! When the archiver is fed the same JetStream messages again, it writes the same
//! samples a second time into new part files. Folders written that way hold two or more
//! copies of every sample, sometimes split at different window boundaries, and the
//! exporter returns every copy. This tool rewrites each affected channel folder so that
//! every distinct sample is stored once, in one file per aligned rotation window, the
//! same layout the archiver writes today. It does not talk to NATS.
//!
//! # Usage
//!
//! ```text
//! dedupe <parquet_root> [--dry-run] [--rotate-secs N]
//! ```
//!
//! * `<parquet_root>` - Archive root holding `asset<NNN>/<YYYY-MM-DD>/ch<NN>/`
//!   folders. The first argument that does not start with `--` is used. Required.
//! * `--dry-run` - Build and verify each new folder, then delete it and keep the
//!   original. Reports what a real run would change.
//! * `--rotate-secs N` - Window length in seconds (default 300). Must match the
//!   archiver's `rotate_secs`.
//!
//! # Design
//!
//! A sample is a `(timestamp_unix_ns, value)` pair plus the file's key-value metadata.
//! Two samples are duplicates only when all three match exactly, with values compared
//! by their bit patterns. Samples with the same timestamp but different values are all
//! kept and counted as conflicts.
//!
//! Each channel folder is handled on its own:
//!
//! 1. Every part file is read once to find its metadata and which windows its rows
//!    fall in.
//! 2. Window by window, the rows of every file touching that window are merged, sorted,
//!    and exact duplicates dropped. The result is written to a staging folder
//!    `ch<NN>.dedupe-tmp` next to the original, one file per window (and per distinct
//!    metadata, if a folder mixes calibrations). Each file is read back and compared
//!    with what was meant to be written.
//! 3. An independent check then reads every original file again and confirms each of
//!    its rows is present in the staged file for its window.
//! 4. The original folder is renamed to `ch<NN>.dedupe-old`, the staging folder takes
//!    its name, the day folder is synced, and the old folder is deleted.
//!
//! The exporter only reads `ch<NN>` folders, so it never sees the staging or old
//! folders. Folders for today and yesterday (UTC) are skipped because the archiver may
//! still be writing to them. Folders holding anything other than finished part files
//! (such as `.inprogress` or quarantined files) are skipped, as are folders that have
//! no duplicate rows and already hold one file per window. A rerun resumes safely: a
//! leftover staging folder is deleted, and a leftover old folder is either deleted
//! (swap finished) or renamed back (swap interrupted).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration, NaiveDate, Utc};
use parquet::file::{metadata::KeyValue, reader::SerializedFileReader};

mod archive_format;
mod archive_io;

use archive_io::{Archive, read_archive, write_archive};

/// Suffix of the staging folder that holds the rewritten files before the swap.
const STAGING_SUFFIX: &str = ".dedupe-tmp";
/// Suffix the original folder is renamed to while the staging folder takes its place.
const OLD_SUFFIX: &str = ".dedupe-old";

/// One sample, keyed by metadata group so different calibrations never merge.
///
/// Fields are the metadata group index, the timestamp in Unix nanoseconds and the
/// value's bit pattern (`f64::to_bits`).
type Row = (usize, i64, u64);

/// What was learned about one original part file in the first pass.
struct PartInfo {
    /// Path of the part file.
    path: PathBuf,
    /// Index into the folder's list of distinct key-value metadata.
    group: usize,
    /// Windows (see [`window_of`]) that at least one of the file's rows falls in.
    windows: BTreeSet<i64>,
}

/// Counts for one channel folder.
#[derive(Debug, Default, PartialEq, Eq)]
struct FolderStats {
    /// Part files in the original folder, including empty ones.
    files_before: usize,
    /// Part files written to the new folder.
    files_after: usize,
    /// Rows across all original files.
    rows_before: usize,
    /// Distinct rows kept.
    rows_after: usize,
    /// Kept rows that share a timestamp with a different value in the same group.
    conflicts: usize,
    /// Bytes of the original part files.
    bytes_before: u64,
    /// Bytes of the new part files.
    bytes_after: u64,
}

/// Result of processing one channel folder.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The folder was rewritten (or, in a dry run, a verified copy was built and
    /// deleted).
    Rewritten(FolderStats),
    /// The folder has no duplicate rows and already holds one file per window.
    Clean,
    /// The folder was left alone, for the reason given.
    Skipped(String),
}

/// Returns the rotation window a timestamp falls in.
///
/// Windows start at multiples of `rotate_secs` since the Unix epoch, as in the
/// archiver.
///
/// # Arguments
///
/// * `timestamp_unix_ns` - Sample time in Unix nanoseconds.
/// * `rotate_secs` - Window length in seconds. `0` is treated as `1`.
fn window_of(timestamp_unix_ns: i64, rotate_secs: u64) -> i64 {
    let rotate_ns = (rotate_secs.max(1) as i64).saturating_mul(1_000_000_000);
    timestamp_unix_ns.div_euclid(rotate_ns)
}

/// Returns the metadata as comparable `(key, value)` pairs, in order.
///
/// # Arguments
///
/// * `metadata` - File-level key-value metadata.
fn metadata_key(metadata: &[KeyValue]) -> Vec<(String, Option<String>)> {
    metadata
        .iter()
        .map(|kv| (kv.key.clone(), kv.value.clone()))
        .collect()
}

/// Reads a whole part file.
///
/// # Arguments
///
/// * `path` - Part file to read.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
fn read_part(path: &Path) -> Result<Archive> {
    let reader = SerializedFileReader::new(fs::File::open(path)?)
        .with_context(|| format!("opening {}", path.display()))?;
    read_archive(&reader).with_context(|| format!("reading {}", path.display()))
}

/// Returns the rows of `archive` that fall in `window`, tagged with `group`.
///
/// # Arguments
///
/// * `archive` - Contents of one part file.
/// * `group` - Metadata group of the file.
/// * `window` - Window to select.
/// * `rotate_secs` - Window length in seconds.
fn rows_in_window(archive: &Archive, group: usize, window: i64, rotate_secs: u64) -> Vec<Row> {
    archive
        .timestamps
        .iter()
        .zip(&archive.values)
        .filter(|(ts, _)| window_of(**ts, rotate_secs) == window)
        .map(|(ts, v)| (group, *ts, v.to_bits()))
        .collect()
}

/// Loads part files on demand and drops them once no later window needs them.
struct PartCache<'a> {
    /// First-pass information for every non-empty part file.
    parts: &'a [PartInfo],
    /// Loaded contents, by index into `parts`.
    loaded: HashMap<usize, Archive>,
}

impl<'a> PartCache<'a> {
    /// Creates an empty cache over `parts`.
    fn new(parts: &'a [PartInfo]) -> Self {
        Self {
            parts,
            loaded: HashMap::new(),
        }
    }

    /// Returns every row of every part file in `window`, tagged with its group.
    ///
    /// Files are loaded the first time a window needs them and dropped once their
    /// last window has been served, so memory holds only the files around the
    /// current window.
    ///
    /// # Arguments
    ///
    /// * `window` - Window to collect. Windows must be requested in increasing order.
    /// * `rotate_secs` - Window length in seconds.
    ///
    /// # Returns
    ///
    /// The rows (unsorted, duplicates included) and, per source file, the rows that
    /// file contributed, for the independent check.
    ///
    /// # Errors
    ///
    /// Returns an error if a file cannot be read.
    fn window_rows(&mut self, window: i64, rotate_secs: u64) -> Result<Vec<(usize, Vec<Row>)>> {
        let mut out = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            if !part.windows.contains(&window) {
                continue;
            }
            if !self.loaded.contains_key(&i) {
                self.loaded.insert(i, read_part(&part.path)?);
            }
            out.push((
                i,
                rows_in_window(&self.loaded[&i], part.group, window, rotate_secs),
            ));
            if part.windows.last() == Some(&window) {
                self.loaded.remove(&i);
            }
        }
        Ok(out)
    }
}

/// Sorts rows and removes exact duplicates.
///
/// # Arguments
///
/// * `rows` - Rows to deduplicate; sorted and deduplicated in place.
///
/// # Returns
///
/// The number of kept rows that share their group and timestamp with another kept
/// row (a different value at the same time).
fn sort_unique(rows: &mut Vec<Row>) -> usize {
    rows.sort_unstable();
    rows.dedup();
    let mut conflicts = 0;
    for pair in rows.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 {
            conflicts += 1;
        }
    }
    conflicts
}

/// Returns whether a folder is already clean: one file per window, no empty files, and
/// no file holding the same row twice.
///
/// Files covering different single windows cannot share rows, so this proves there
/// are no duplicates without merging anything.
///
/// # Arguments
///
/// * `parts` - First-pass information for the non-empty part files.
/// * `empty_files` - Part files with no rows.
/// * `rows_repeated_within_files` - Rows that repeat another row of the same file.
fn is_clean(parts: &[PartInfo], empty_files: usize, rows_repeated_within_files: usize) -> bool {
    if empty_files > 0 || rows_repeated_within_files > 0 {
        return false;
    }
    let mut seen = HashSet::new();
    parts
        .iter()
        .all(|p| p.windows.len() == 1 && seen.insert(*p.windows.first().unwrap()))
}

/// Restores a channel folder left half-done by an interrupted run.
///
/// A leftover staging folder is deleted. A leftover old folder is deleted when the
/// channel folder exists (the swap finished), or renamed back when it does not (the
/// swap was interrupted between the two renames).
///
/// # Arguments
///
/// * `channel_dir` - The `ch<NN>` folder.
///
/// # Errors
///
/// Returns an error if a folder cannot be removed or renamed.
fn recover(channel_dir: &Path) -> Result<()> {
    let staging = suffixed(channel_dir, STAGING_SUFFIX);
    let old = suffixed(channel_dir, OLD_SUFFIX);
    if old.exists() {
        if channel_dir.exists() {
            fs::remove_dir_all(&old)?;
            println!("removed finished {}", old.display());
        } else {
            fs::rename(&old, channel_dir)?;
            println!("restored {}", channel_dir.display());
        }
    }
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
        println!("removed stale {}", staging.display());
    }
    Ok(())
}

/// Returns `path` with `suffix` appended to its last component.
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

/// Syncs a directory so renames and new entries in it are durable.
fn sync_dir(dir: &Path) -> Result<()> {
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Deduplicates one channel folder.
///
/// See the module documentation for the steps. The original folder is only replaced
/// after the staged copy has passed both checks; on any error the staging folder is
/// deleted and the original is left as it was.
///
/// # Arguments
///
/// * `channel_dir` - The `ch<NN>` folder to process.
/// * `rotate_secs` - Window length in seconds.
/// * `dry_run` - When `true`, build and verify the new folder, then delete it.
///
/// # Errors
///
/// Returns an error if a file cannot be read or written, a check fails, or the swap
/// fails.
fn dedupe_folder(channel_dir: &Path, rotate_secs: u64, dry_run: bool) -> Result<Outcome> {
    let mut names = Vec::new();
    for entry in fs::read_dir(channel_dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        let is_part = name
            .strip_prefix("part-")
            .and_then(|n| n.strip_suffix(".parquet"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !is_part {
            return Ok(Outcome::Skipped(format!("holds {name}")));
        }
        names.push(name);
    }
    names.sort();
    if names.is_empty() {
        return Ok(Outcome::Skipped("no part files".into()));
    }

    // Pass 1: metadata group and windows of every file.
    let mut stats = FolderStats {
        files_before: names.len(),
        ..Default::default()
    };
    let mut groups: Vec<Vec<KeyValue>> = Vec::new();
    let mut parts = Vec::new();
    let mut empty_files = 0;
    let mut repeated = 0;
    for name in &names {
        let path = channel_dir.join(name);
        stats.bytes_before += fs::metadata(&path)?.len();
        let archive = read_part(&path)?;
        if archive.timestamps.is_empty() {
            empty_files += 1;
            continue;
        }
        stats.rows_before += archive.timestamps.len();
        let key = metadata_key(&archive.metadata);
        let group = match groups.iter().position(|g| metadata_key(g) == key) {
            Some(g) => g,
            None => {
                groups.push(archive.metadata.clone());
                groups.len() - 1
            }
        };
        let mut rows: Vec<(i64, u64)> = archive
            .timestamps
            .iter()
            .zip(&archive.values)
            .map(|(ts, v)| (*ts, v.to_bits()))
            .collect();
        rows.sort_unstable();
        rows.dedup();
        repeated += archive.timestamps.len() - rows.len();
        let windows = archive
            .timestamps
            .iter()
            .map(|ts| window_of(*ts, rotate_secs))
            .collect();
        parts.push(PartInfo {
            path,
            group,
            windows,
        });
    }
    if parts.is_empty() {
        return Ok(Outcome::Skipped("only empty part files".into()));
    }
    if is_clean(&parts, empty_files, repeated) {
        return Ok(Outcome::Clean);
    }
    let all_windows: BTreeSet<i64> = parts
        .iter()
        .flat_map(|p| p.windows.iter().copied())
        .collect();

    // Pass 2: build the staged folder window by window.
    let staging = suffixed(channel_dir, STAGING_SUFFIX);
    fs::create_dir(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let result = (|| -> Result<()> {
        let mut cache = PartCache::new(&parts);
        let mut staged: BTreeMap<(i64, usize), PathBuf> = BTreeMap::new();
        for &window in &all_windows {
            let mut rows: Vec<Row> = cache
                .window_rows(window, rotate_secs)?
                .into_iter()
                .flat_map(|(_, r)| r)
                .collect();
            stats.conflicts += sort_unique(&mut rows);
            stats.rows_after += rows.len();
            for (group, metadata) in groups.iter().enumerate() {
                let archive = Archive {
                    timestamps: rows.iter().filter(|r| r.0 == group).map(|r| r.1).collect(),
                    values: rows
                        .iter()
                        .filter(|r| r.0 == group)
                        .map(|r| f64::from_bits(r.2))
                        .collect(),
                    metadata: metadata.clone(),
                };
                if archive.timestamps.is_empty() {
                    continue;
                }
                let path = staging.join(format!("part-{:04}.parquet", staged.len() + 1));
                write_archive(&path, &archive)?;
                let back = read_part(&path)?;
                if back.timestamps != archive.timestamps
                    || back.values.len() != archive.values.len()
                    || back
                        .values
                        .iter()
                        .zip(&archive.values)
                        .any(|(a, b)| a.to_bits() != b.to_bits())
                    || metadata_key(&back.metadata) != metadata_key(metadata)
                {
                    bail!("{} does not read back as written", path.display());
                }
                stats.bytes_after += fs::metadata(&path)?.len();
                staged.insert((window, group), path);
            }
        }
        stats.files_after = staged.len();

        // Pass 3: every original row must be in the staged file for its window.
        let mut cache = PartCache::new(&parts);
        let mut checked = 0usize;
        for &window in &all_windows {
            let mut present: HashSet<Row> = HashSet::new();
            for group in 0..groups.len() {
                if let Some(path) = staged.get(&(window, group)) {
                    let back = read_part(path)?;
                    if back
                        .timestamps
                        .iter()
                        .any(|ts| window_of(*ts, rotate_secs) != window)
                    {
                        bail!("{} holds rows outside its window", path.display());
                    }
                    present.extend(
                        back.timestamps
                            .iter()
                            .zip(&back.values)
                            .map(|(ts, v)| (group, *ts, v.to_bits())),
                    );
                }
            }
            for (i, rows) in cache.window_rows(window, rotate_secs)? {
                if let Some(missing) = rows.iter().find(|r| !present.contains(r)) {
                    bail!(
                        "row at {} ns from {} is missing from the new folder",
                        missing.1,
                        parts[i].path.display()
                    );
                }
                checked += rows.len();
            }
        }
        if checked != stats.rows_before {
            bail!(
                "checked {checked} original rows, expected {}",
                stats.rows_before
            );
        }
        sync_dir(&staging)
    })();
    if let Err(err) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(err);
    }

    if dry_run {
        fs::remove_dir_all(&staging)?;
        return Ok(Outcome::Rewritten(stats));
    }

    let old = suffixed(channel_dir, OLD_SUFFIX);
    let day_dir = channel_dir
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent", channel_dir.display()))?;
    fs::rename(channel_dir, &old)?;
    fs::rename(&staging, channel_dir)?;
    sync_dir(day_dir)?;
    fs::remove_dir_all(&old)?;
    sync_dir(day_dir)?;
    Ok(Outcome::Rewritten(stats))
}

/// Lists `asset<NNN>/<YYYY-MM-DD>/ch<NN>` folders under `root` older than `cutoff`.
///
/// Leftovers from an interrupted run are cleaned up with [`recover`] on the way.
///
/// # Arguments
///
/// * `root` - Archive root.
/// * `cutoff` - Days on or after this date are not listed.
///
/// # Errors
///
/// Returns an error if a folder cannot be listed or recovered.
fn channel_folders(root: &Path, cutoff: NaiveDate) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for asset in fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let asset = asset?.path();
        if !asset.is_dir()
            || !asset
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("asset"))
        {
            continue;
        }
        for day in fs::read_dir(&asset)? {
            let day = day?.path();
            let Some(date) = day
                .file_name()
                .and_then(|n| NaiveDate::parse_from_str(&n.to_string_lossy(), "%Y-%m-%d").ok())
            else {
                continue;
            };
            if date >= cutoff || !day.is_dir() {
                continue;
            }
            let mut channels = BTreeSet::new();
            for entry in fs::read_dir(&day)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                let base = name
                    .strip_suffix(STAGING_SUFFIX)
                    .or_else(|| name.strip_suffix(OLD_SUFFIX))
                    .unwrap_or(&name);
                if base.starts_with("ch") {
                    channels.insert(day.join(base));
                }
            }
            for channel_dir in channels {
                recover(&channel_dir)?;
                if channel_dir.is_dir() {
                    out.push(channel_dir);
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Parses the command line, deduplicates every eligible channel folder and prints a
/// summary.
///
/// # Errors
///
/// Returns an error if no `<parquet_root>` is given, `--rotate-secs` is not a number,
/// scanning fails, or any folder failed (failed folders are left untouched).
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: dedupe <parquet_root> [--dry-run] [--rotate-secs N]";
    let mut root = None;
    let mut rotate_secs = 300u64;
    let mut dry_run = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--rotate-secs" => {
                rotate_secs = iter
                    .next()
                    .ok_or_else(|| anyhow!(usage))?
                    .parse()
                    .context("--rotate-secs")?
            }
            a if !a.starts_with("--") && root.is_none() => root = Some(PathBuf::from(a)),
            _ => {}
        }
    }
    let root = root.ok_or_else(|| anyhow!(usage))?;
    let cutoff = Utc::now().date_naive() - Duration::days(1);
    let folders = channel_folders(&root, cutoff)?;
    println!(
        "{} channel folders before {cutoff} under {}{}",
        folders.len(),
        root.display(),
        if dry_run { " (dry run)" } else { "" }
    );

    let mut total = FolderStats::default();
    let (mut rewritten, mut clean, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize);
    for folder in &folders {
        match dedupe_folder(folder, rotate_secs, dry_run) {
            Ok(Outcome::Rewritten(s)) => {
                rewritten += 1;
                println!(
                    "{}: {} files -> {}, {} rows -> {}{}, {:.1} MB -> {:.1} MB",
                    folder.display(),
                    s.files_before,
                    s.files_after,
                    s.rows_before,
                    s.rows_after,
                    if s.conflicts > 0 {
                        format!(" ({} conflicting timestamps kept)", s.conflicts)
                    } else {
                        String::new()
                    },
                    s.bytes_before as f64 / 1e6,
                    s.bytes_after as f64 / 1e6
                );
                total.files_before += s.files_before;
                total.files_after += s.files_after;
                total.rows_before += s.rows_before;
                total.rows_after += s.rows_after;
                total.conflicts += s.conflicts;
                total.bytes_before += s.bytes_before;
                total.bytes_after += s.bytes_after;
            }
            Ok(Outcome::Clean) => clean += 1,
            Ok(Outcome::Skipped(why)) => {
                skipped += 1;
                println!("skipped {}: {why}", folder.display());
            }
            Err(err) => {
                failed += 1;
                eprintln!("FAILED {}: {err:#}", folder.display());
            }
        }
    }
    println!(
        "done: rewritten {rewritten} folders ({} files -> {}, {} rows -> {}, {} conflicts, {:.2} GB -> {:.2} GB), clean {clean}, skipped {skipped}, failed {failed}",
        total.files_before,
        total.files_after,
        total.rows_before,
        total.rows_after,
        total.conflicts,
        total.bytes_before as f64 / 1e9,
        total.bytes_after as f64 / 1e9
    );
    if failed > 0 {
        bail!("{failed} folder(s) failed; they were left untouched");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: i64 = 1_789_430_400_000_000_000; // 2026-09-15T00:00:00Z
    const STEP: i64 = 10_000_000; // 100 Hz

    fn calibration(text: &str) -> Vec<KeyValue> {
        vec![KeyValue::new("calibration".to_string(), text.to_string())]
    }

    fn write_part(dir: &Path, index: usize, rows: &[(i64, f64)], metadata: Vec<KeyValue>) {
        let archive = Archive {
            timestamps: rows.iter().map(|r| r.0).collect(),
            values: rows.iter().map(|r| r.1).collect(),
            metadata,
        };
        write_archive(&dir.join(format!("part-{index:04}.parquet")), &archive).unwrap();
    }

    fn samples(from: i64, count: i64) -> Vec<(i64, f64)> {
        (from..from + count)
            .map(|k| (BASE + k * STEP, ((k * 7919) % 97) as f64 * 0.01))
            .collect()
    }

    fn read_folder(dir: &Path) -> Vec<(String, Archive)> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|n| {
                let a = read_part(&dir.join(&n)).unwrap();
                (n, a)
            })
            .collect()
    }

    fn temp_channel(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("dedupe-{tag}-{}", uuid::Uuid::new_v4()));
        let dir = root.join("asset1001/2026-09-15/ch08");
        fs::create_dir_all(&dir).unwrap();
        (root, dir)
    }

    /// Two full copies, one of them split at different boundaries, plus an empty
    /// file, collapse to one aligned file per window holding each sample once.
    #[test]
    fn duplicate_copies_collapse_to_one_file_per_window() {
        let (root, dir) = temp_channel("collapse");
        let cal = calibration(r#"{"type":"linear","a":1.5,"b":0.1}"#);
        // Copy A: unaligned, like the old archiver (1093 rows, then 30000-row files).
        write_part(&dir, 1, &samples(0, 1093), cal.clone());
        write_part(&dir, 2, &samples(1093, 30000), cal.clone());
        write_part(&dir, 3, &samples(31093, 28907), cal.clone());
        // Copy B: aligned, written twice.
        write_part(&dir, 4, &samples(0, 30000), cal.clone());
        write_part(&dir, 5, &samples(30000, 30000), cal.clone());
        write_part(&dir, 6, &samples(0, 30000), cal.clone());
        write_part(&dir, 7, &samples(30000, 30000), cal.clone());
        write_part(&dir, 8, &[], cal.clone());

        let dry = dedupe_folder(&dir, 300, true).unwrap();
        assert!(matches!(
            dry,
            Outcome::Rewritten(FolderStats {
                rows_after: 60000,
                ..
            })
        ));
        assert_eq!(read_folder(&dir).len(), 8, "dry run keeps the original");

        let Outcome::Rewritten(stats) = dedupe_folder(&dir, 300, false).unwrap() else {
            panic!("expected a rewrite");
        };
        assert_eq!(stats.files_before, 8);
        assert_eq!(stats.files_after, 2);
        assert_eq!(stats.rows_before, 180000);
        assert_eq!(stats.rows_after, 60000);
        assert_eq!(stats.conflicts, 0);

        let files = read_folder(&dir);
        assert_eq!(files.len(), 2);
        let expected = samples(0, 60000);
        let mut got = Vec::new();
        for (i, (name, a)) in files.iter().enumerate() {
            assert_eq!(name, &format!("part-{:04}.parquet", i + 1));
            assert_eq!(metadata_key(&a.metadata), metadata_key(&cal));
            assert!(
                a.timestamps
                    .iter()
                    .all(|t| window_of(*t, 300) == window_of(a.timestamps[0], 300))
            );
            got.extend(a.timestamps.iter().copied().zip(a.values.iter().copied()));
        }
        assert_eq!(got.len(), expected.len());
        assert!(
            got.iter()
                .zip(&expected)
                .all(|(g, e)| g.0 == e.0 && g.1.to_bits() == e.1.to_bits())
        );
        assert!(!suffixed(&dir, STAGING_SUFFIX).exists());
        assert!(!suffixed(&dir, OLD_SUFFIX).exists());

        assert_eq!(dedupe_folder(&dir, 300, false).unwrap(), Outcome::Clean);
        fs::remove_dir_all(root).unwrap();
    }

    /// Same timestamp with a different value, NaN values, and files with different
    /// calibrations are all kept.
    #[test]
    fn distinct_rows_and_calibrations_are_kept() {
        let (root, dir) = temp_channel("distinct");
        let a = calibration(r#"{"id":"a"}"#);
        let b = calibration(r#"{"id":"b"}"#);
        let mut rows = samples(0, 100);
        rows[5].1 = f64::NAN;
        write_part(&dir, 1, &rows, a.clone());
        write_part(&dir, 2, &rows, a.clone());
        let mut changed = rows.clone();
        changed[7].1 += 1.0;
        write_part(&dir, 3, &changed, a.clone());
        write_part(&dir, 4, &rows, b.clone());

        let Outcome::Rewritten(stats) = dedupe_folder(&dir, 300, false).unwrap() else {
            panic!("expected a rewrite");
        };
        assert_eq!(stats.rows_after, 100 + 1 + 100);
        assert_eq!(stats.conflicts, 1);
        let files = read_folder(&dir);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].1.timestamps.len(), 101);
        assert!(files[0].1.values.iter().any(|v| v.is_nan()));
        assert_eq!(metadata_key(&files[1].1.metadata), metadata_key(&b));
        fs::remove_dir_all(root).unwrap();
    }

    /// Folders with in-progress files are skipped, and an interrupted swap is undone.
    #[test]
    fn unfinished_folders_are_skipped_and_interrupted_swaps_recovered() {
        let (root, dir) = temp_channel("recover");
        write_part(&dir, 1, &samples(0, 10), vec![]);
        fs::write(dir.join("part-0002.parquet.inprogress"), b"").unwrap();
        assert!(matches!(
            dedupe_folder(&dir, 300, false).unwrap(),
            Outcome::Skipped(_)
        ));
        fs::remove_file(dir.join("part-0002.parquet.inprogress")).unwrap();

        // Interrupted between the two renames: only the old folder and staging exist.
        let old = suffixed(&dir, OLD_SUFFIX);
        fs::rename(&dir, &old).unwrap();
        fs::create_dir(suffixed(&dir, STAGING_SUFFIX)).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        assert_eq!(channel_folders(&root, cutoff).unwrap(), vec![dir.clone()]);
        assert!(dir.join("part-0001.parquet").exists());
        assert!(!old.exists());
        assert!(!suffixed(&dir, STAGING_SUFFIX).exists());

        // Days on or after the cutoff are not listed.
        let early = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        assert!(channel_folders(&root, early).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
