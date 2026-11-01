//! A small log file so a problem can be reported with the app's own account of what happened.
//!
//! Everything stays on this computer: the file is only ever written here and read back when the
//! user chooses to show or copy it (Settings > Troubleshooting). Nothing is sent anywhere.
//! One file of up to `MAX_BYTES`, plus the previous one, keeps the total small.

use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const FILE_NAME: &str = "recording-fixer.log";
const MAX_BYTES: u64 = 1024 * 1024;

pub fn log_file(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

fn previous_file(dir: &Path) -> PathBuf {
    dir.join(format!("{FILE_NAME}.1"))
}

struct Inner {
    dir: PathBuf,
    /// None only while rotating: Windows cannot rename a file that is still open.
    file: Option<File>,
    size: u64,
    max: u64,
}

impl Inner {
    fn open(dir: &Path, max: u64) -> io::Result<Inner> {
        fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file(dir))?;
        let size = file.metadata()?.len();
        Ok(Inner {
            dir: dir.to_path_buf(),
            file: Some(file),
            size,
            max,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let _ = fs::remove_file(previous_file(&self.dir));
        fs::rename(log_file(&self.dir), previous_file(&self.dir))?;
        *self = Inner::open(&self.dir, self.max)?;
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.size > 0 && self.size + line.len() as u64 > self.max {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log file is closed"))?;
        file.write_all(line.as_bytes())?;
        self.size += line.len() as u64;
        Ok(())
    }
}

struct FileLogger {
    inner: Mutex<Inner>,
}

impl Log for FileLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        // Our own messages at the configured level; dependencies only when they report a problem.
        m.level() <= LevelFilter::Warn || m.target().starts_with("recording_fixer")
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {}\n",
            format_utc(unix_secs()),
            r.level(),
            r.args()
        );
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.write_line(&line);
        }
    }

    fn flush(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(f) = inner.file.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `2023-11-14T22:13:20Z`. UTC, so logs from different machines line up.
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Start logging into `dir`. Also records panics, with a backtrace, before the default handler runs.
pub fn init(dir: &Path) -> io::Result<()> {
    let level = match std::env::var("RECORDING_FIXER_LOG").as_deref() {
        Ok("debug") => LevelFilter::Debug,
        Ok("trace") => LevelFilter::Trace,
        _ => LevelFilter::Info,
    };
    let logger = FileLogger {
        inner: Mutex::new(Inner::open(dir, MAX_BYTES)?),
    };
    if log::set_boxed_logger(Box::new(logger)).is_err() {
        return Ok(()); // already set (tests)
    }
    log::set_max_level(level);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        log::logger().flush();
        previous(info);
    }));

    log::info!(
        "Recording Fixer for Serato DJ {} starting on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(())
}

/// The end of the log (previous file first), at most `max_bytes`, for showing or copying.
pub fn read_tail(dir: &Path, max_bytes: usize) -> String {
    let mut bytes = fs::read(previous_file(dir)).unwrap_or_default();
    bytes.extend(fs::read(log_file(dir)).unwrap_or_default());
    let start = bytes.len().saturating_sub(max_bytes);
    let text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    // Don't begin in the middle of a line.
    match (start > 0, text.split_once('\n')) {
        (true, Some((_, rest))) => rest.to_string(),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_iso() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_utc(1_709_164_800), "2024-02-29T00:00:00Z"); // leap day
    }

    #[test]
    fn rotation_keeps_the_newest_lines_and_one_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut inner = Inner::open(dir.path(), 100).unwrap();
        for i in 0..20 {
            inner.write_line(&format!("line number {i:02}\n")).unwrap();
        }
        let current = fs::read_to_string(log_file(dir.path())).unwrap();
        let previous = fs::read_to_string(previous_file(dir.path())).unwrap();
        assert!(current.len() <= 100 && previous.len() <= 100);
        assert!(current.contains("line number 19"));
        assert!(!dir.path().join("recording-fixer.log.2").exists());
        // The tail reads oldest to newest across both files.
        let tail = read_tail(dir.path(), 10_000);
        assert!(tail.ends_with("line number 19\n"));
        let last_old = previous.lines().last().unwrap();
        assert!(tail.find(last_old).unwrap() < tail.find("line number 19").unwrap());
    }

    #[test]
    fn tail_is_bounded_and_starts_on_a_line() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            log_file(dir.path()),
            "first line\nsecond line\nthird line\n",
        )
        .unwrap();
        assert_eq!(
            read_tail(dir.path(), 1000),
            "first line\nsecond line\nthird line\n"
        );
        assert_eq!(read_tail(dir.path(), 15), "third line\n");
    }
}
