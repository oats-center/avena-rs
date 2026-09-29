//! Protection against JetStream replaying messages the archiver already wrote.
//!
//! JetStream keeps a durable consumer's progress (delivered and ack floor sequences) in
//! a small state file next to the stream. The server replaces that file by writing a
//! temporary file and renaming it, without an fsync. After a power cut the file can be
//! left empty or missing. The server then recovers the consumer with no progress, and
//! with deliver policy `all` it delivers the whole stream again. A consumer whose state
//! file is corrupt is dropped at recovery, and the archiver creates it again, also from
//! the start of the stream.
//!
//! The archiver therefore keeps its own checkpoint per stream and subject: the highest
//! stream sequence such that every message on the subject up to it has been acked, which
//! means it is inside a closed and fsynced Parquet file. The value is the consumer's ack
//! floor as reported by the server, saved with an fsync of the file and its directory, so
//! it survives the same power cut. A message at or below the checkpoint is acked without
//! being written again.
//!
//! The checkpoint is only trusted while it clearly belongs to the same stream: the stream
//! name, its creation time and the subject must match, and the stream's last sequence must
//! not be below the checkpoint (a recreated or rewound stream reuses sequence numbers).
//! Otherwise it is ignored and every message is written, as without the guard.

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

/// Saved archiver progress for one stream and subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// JetStream stream name.
    pub stream: String,
    /// Stream creation time in Unix nanoseconds; changes if the stream is recreated.
    pub stream_created_unix_ns: i64,
    /// Subject the channel consumer filters on.
    pub subject: String,
    /// Every message on `subject` with a stream sequence up to this one is archived.
    pub archived_through_seq: u64,
}

/// Turns a stream name or subject into a safe file name part.
///
/// ASCII letters, digits, `-`, `_` and `.` are kept, anything else becomes `_`. A
/// leading `.` (or an empty result) gets a `_` prefix so no hidden or special name is
/// produced.
fn file_token(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with('.') {
        out.insert(0, '_');
    }
    out
}

/// Path of the checkpoint file for one stream and subject.
///
/// # Examples
///
/// ```text
/// checkpoint_path("/state", "labjack", "avenars.i69.mu1.lj2.live.ch08")
///     -> "/state/labjack/avenars.i69.mu1.lj2.live.ch08.json"
/// ```
pub fn checkpoint_path(state_dir: &Path, stream: &str, subject: &str) -> PathBuf {
    state_dir
        .join(file_token(stream))
        .join(format!("{}.json", file_token(subject)))
}

/// Reads a checkpoint file.
///
/// # Returns
///
/// `None` if the file does not exist.
///
/// # Errors
///
/// Returns an error if the file cannot be read or does not parse.
pub fn load(path: &Path) -> io::Result<Option<Checkpoint>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Writes a checkpoint durably.
///
/// The JSON is written to a temporary file that is fsynced, renamed over the checkpoint
/// and followed by an fsync of the directory, so after a power cut the file holds either
/// the old or the new checkpoint, never an empty one.
///
/// # Errors
///
/// Returns an error if the directory cannot be created or any write, sync or rename
/// fails.
pub fn save(path: &Path, checkpoint: &Checkpoint) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "checkpoint has no parent"))?;
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(checkpoint)?)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    fs::File::open(dir)?.sync_all()
}

/// Decides whether a saved checkpoint can be used for the stream as it is now.
///
/// # Arguments
///
/// * `checkpoint` - Saved checkpoint.
/// * `stream` - Current stream name.
/// * `stream_created_unix_ns` - Current stream creation time.
/// * `stream_last_seq` - Current last sequence of the stream.
/// * `subject` - Current channel subject.
///
/// # Returns
///
/// The trusted floor, or the reason the checkpoint is ignored.
pub fn trusted_floor(
    checkpoint: &Checkpoint,
    stream: &str,
    stream_created_unix_ns: i64,
    stream_last_seq: u64,
    subject: &str,
) -> Result<u64, String> {
    if checkpoint.stream != stream || checkpoint.subject != subject {
        return Err(format!(
            "it is for stream '{}' subject {}",
            checkpoint.stream, checkpoint.subject
        ));
    }
    if checkpoint.stream_created_unix_ns != stream_created_unix_ns {
        return Err("the stream was recreated since it was written".to_string());
    }
    if checkpoint.archived_through_seq > stream_last_seq {
        return Err(format!(
            "it is at sequence {} but the stream ends at {}, so sequences may have been reused",
            checkpoint.archived_through_seq, stream_last_seq
        ));
    }
    Ok(checkpoint.archived_through_seq)
}

/// Skips messages at or below a trusted floor and counts them for logging.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    floor: u64,
    skipped: u64,
    first_skipped: u64,
    last_skipped: u64,
}

/// A finished run of skipped messages, for one log line.
#[derive(Debug, PartialEq, Eq)]
pub struct SkipRun {
    /// Number of messages skipped.
    pub count: u64,
    /// Lowest stream sequence skipped.
    pub first: u64,
    /// Highest stream sequence skipped.
    pub last: u64,
}

impl ReplayGuard {
    /// Creates a guard that skips stream sequences `1..=floor`. A floor of 0 skips
    /// nothing.
    pub fn new(floor: u64) -> Self {
        Self {
            floor,
            ..Self::default()
        }
    }

    /// Current floor.
    pub fn floor(&self) -> u64 {
        self.floor
    }

    /// Raises the floor; a lower value is ignored.
    pub fn raise(&mut self, floor: u64) {
        self.floor = self.floor.max(floor);
    }

    /// Returns `true` if the message at `stream_seq` is already archived and must be
    /// acked without writing. Sequence 0 (unknown) is never skipped.
    pub fn is_archived(&mut self, stream_seq: u64) -> bool {
        if stream_seq == 0 || stream_seq > self.floor {
            return false;
        }
        if self.skipped == 0 {
            self.first_skipped = stream_seq;
        }
        self.first_skipped = self.first_skipped.min(stream_seq);
        self.last_skipped = self.last_skipped.max(stream_seq);
        self.skipped += 1;
        true
    }

    /// Number of messages skipped in the current run.
    pub fn skipped_in_run(&self) -> u64 {
        self.skipped
    }

    /// Ends the current run of skipped messages and returns it, if there was one.
    pub fn take_run(&mut self) -> Option<SkipRun> {
        if self.skipped == 0 {
            return None;
        }
        let run = SkipRun {
            count: self.skipped,
            first: self.first_skipped,
            last: self.last_skipped,
        };
        self.skipped = 0;
        self.first_skipped = 0;
        self.last_skipped = 0;
        Some(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(seq: u64) -> Checkpoint {
        Checkpoint {
            stream: "labjack".into(),
            stream_created_unix_ns: 42,
            subject: "avenars.s.b.src.live.ch08".into(),
            archived_through_seq: seq,
        }
    }

    #[test]
    fn checkpoint_round_trips_and_missing_file_is_none() {
        let dir = std::env::temp_dir().join(format!("replay-guard-{}", uuid::Uuid::new_v4()));
        let path = checkpoint_path(&dir, "labjack", "avenars.s.b.src.live.ch08");
        assert_eq!(load(&path).unwrap(), None);
        save(&path, &cp(7)).unwrap();
        save(&path, &cp(9)).unwrap();
        assert_eq!(load(&path).unwrap(), Some(cp(9)));
        assert!(!path.with_extension("json.tmp").exists());
        fs::write(&path, b"").unwrap();
        assert!(
            load(&path).is_err(),
            "an empty file is an error, not a zero floor"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_names_stay_inside_the_state_dir() {
        let p = checkpoint_path(Path::new("/s"), "../x", "a/b.*.>");
        assert_eq!(p, PathBuf::from("/s/_.._x/a_b._._.json"));
        assert_eq!(file_token(""), "_");
    }

    #[test]
    fn checkpoint_is_trusted_only_for_the_same_stream_generation() {
        let s = "avenars.s.b.src.live.ch08";
        assert_eq!(trusted_floor(&cp(100), "labjack", 42, 500, s), Ok(100));
        assert_eq!(trusted_floor(&cp(100), "labjack", 42, 100, s), Ok(100));
        assert!(trusted_floor(&cp(100), "other", 42, 500, s).is_err());
        assert!(trusted_floor(&cp(100), "labjack", 42, 500, "x.ch09").is_err());
        assert!(trusted_floor(&cp(100), "labjack", 43, 500, s).is_err());
        assert!(trusted_floor(&cp(100), "labjack", 42, 99, s).is_err());
    }

    #[test]
    fn guard_skips_up_to_the_floor_and_reports_runs() {
        let mut g = ReplayGuard::new(10);
        assert!(!g.is_archived(0));
        assert!(g.is_archived(3));
        assert!(g.is_archived(10));
        assert!(!g.is_archived(11));
        assert_eq!(
            g.take_run(),
            Some(SkipRun {
                count: 2,
                first: 3,
                last: 10
            })
        );
        assert_eq!(g.take_run(), None);
        g.raise(5);
        assert_eq!(g.floor(), 10);
        g.raise(20);
        assert!(g.is_archived(11));
        assert!(!ReplayGuard::new(0).is_archived(1));
    }
}
