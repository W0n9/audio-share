//! `server.log` writer for the Server GUI.
//!
//! v0.3.4's GUI wrote `server.log` next to `config.ini`; the rewrite writes the
//! same file from the GUI process. The file is truncated when the application
//! starts and receives timestamped `[info]`/`[error]` lines. Every line is also
//! written to stderr for developer visibility (the GUI has no console in
//! release builds, but the output is useful when launched from a terminal).

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// A log file sink. Tests construct one with a temporary path; the GUI installs
/// a process-wide instance at startup.
#[derive(Debug)]
pub struct Logger {
    file: Mutex<Option<File>>,
}

/// Process-wide logger installed by [`init`].
static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Logger {
    /// Create (and truncate) `path`, creating its parent directory if needed.
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            file: Mutex::new(Some(file)),
        })
    }

    /// A logger that only writes to stderr (used when the file cannot open).
    pub fn stderr_only() -> Self {
        Self {
            file: Mutex::new(None),
        }
    }

    /// Write one timestamped `[info]` line.
    pub fn info(&self, message: impl AsRef<str>) {
        self.write("info", message.as_ref());
    }

    /// Write one timestamped `[error]` line.
    pub fn error(&self, message: impl AsRef<str>) {
        self.write("error", message.as_ref());
    }

    fn write(&self, level: &str, message: &str) {
        let line = format!("[{}] [{}] {}\n", utc_timestamp(), level, message);
        // Keep stderr output for developer visibility; the release GUI has no
        // console, but terminal launches still show the same diagnostics.
        eprint!("{line}");
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(file) = file.as_mut() {
            // Logging failures are best effort; they must never break startup
            // or a start/stop transition.
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// Install the process-wide logger at `path`, truncating the file.
///
/// A failure to open the log file is reported on stderr but must not stop the
/// GUI from starting.
pub fn init(path: &Path) {
    let logger = match Logger::open(path) {
        Ok(logger) => logger,
        Err(error) => {
            eprintln!(
                "AudioShareServer: warning: cannot open log file {}: {error}",
                path.display()
            );
            Logger::stderr_only()
        }
    };
    let _ = LOGGER.set(logger);
}

/// Write an `[info]` line to the installed logger; no-op before [`init`].
pub fn info(message: impl AsRef<str>) {
    if let Some(logger) = LOGGER.get() {
        logger.info(message);
    }
}

/// Write an `[error]` line to the installed logger; no-op before [`init`].
pub fn error(message: impl AsRef<str>) {
    if let Some(logger) = LOGGER.get() {
        logger.error(message);
    }
}

/// `YYYY-MM-DD HH:MM:SS.mmm` in UTC, matching the v0.3.4 log line shape.
fn utc_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
}

/// Convert days since 1970-01-01 to a civil (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_parent_directories_and_truncates_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("server.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "stale line from a previous run\n").unwrap();

        let logger = Logger::open(&path).expect("open log file");
        drop(logger);

        assert!(path.is_file());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "",
            "startup must truncate the previous log"
        );

        let missing_parent = dir.path().join("another").join("server.log");
        let logger = Logger::open(&missing_parent).expect("create parent directory");
        drop(logger);
        assert!(missing_parent.is_file());
    }

    #[test]
    fn info_and_error_append_timestamped_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        let logger = Logger::open(&path).expect("open log file");

        logger.info("app started version 0.4.0");
        logger.error("server start failed: no endpoint");

        let contents = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "log was {contents:?}");
        assert_line_shape(lines[0], "info", "app started version 0.4.0");
        assert_line_shape(lines[1], "error", "server start failed: no endpoint");
    }

    /// `[YYYY-MM-DD HH:MM:SS.mmm] [level] message`
    fn assert_line_shape(line: &str, level: &str, message: &str) {
        let prefix_len = 1 + 23 + 2 + level.len() + 3;
        assert_eq!(line.len(), prefix_len + message.len(), "line was {line:?}");
        assert_eq!(&line[prefix_len..], message, "line was {line:?}");

        let level_tag = format!("[{level}] ");
        assert_eq!(&line[26..26 + level_tag.len()], level_tag);

        let timestamp = &line[1..24];
        assert_eq!(timestamp.len(), 23, "line was {line:?}");
        let bytes = timestamp.as_bytes();
        assert_eq!(bytes[4], b'-', "line was {line:?}");
        assert_eq!(bytes[7], b'-', "line was {line:?}");
        assert_eq!(bytes[10], b' ', "line was {line:?}");
        assert_eq!(bytes[13], b':', "line was {line:?}");
        assert_eq!(bytes[16], b':', "line was {line:?}");
        assert_eq!(bytes[19], b'.', "line was {line:?}");
        assert!(
            timestamp
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '-' | ' ' | ':' | '.')),
            "unexpected timestamp in {line:?}"
        );
    }

    #[test]
    fn a_failed_open_is_reported_as_an_error_not_a_startup_failure() {
        let dir = tempfile::tempdir().unwrap();
        // A directory cannot be opened as a file.
        let error = Logger::open(dir.path()).expect_err("a directory is not a log file");
        assert!(!error.to_string().is_empty());
        assert!(Logger::stderr_only().file.lock().unwrap().is_none());
    }
}
