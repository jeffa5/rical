//! Logging: to a file in the XDG state folder, for debugging after the
//! fact, and warnings to the terminal as well. RUST_LOG picks the levels
//! (as env_logger reads it); the file gets info and up by default.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Past this the log is moved aside (to `rical.log.old`) when rical starts.
const MAX_BYTES: u64 = 1024 * 1024;

/// Where the log goes: `$XDG_STATE_HOME/rical/rical.log`, by default
/// `~/.local/state/rical/rical.log` (the XDG folder for logs and history).
pub fn path() -> Option<PathBuf> {
    crate::uistate::dir().map(|d| d.join("rical.log"))
}

struct Logger {
    filter: env_filter::Filter,
    file: Option<Mutex<File>>,
    /// RUST_LOG was given: everything it lets through goes to the
    /// terminal too, as it did before there was a file.
    verbose: bool,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.filter.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if !self.filter.matches(record) {
            return;
        }
        let line = format!(
            "{} {:5} {}: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        );
        if let Some(file) = &self.file
            && let Ok(mut f) = file.lock()
        {
            let _ = f.write_all(line.as_bytes());
        }
        if self.verbose || record.level() <= Level::Warn {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file
            && let Ok(mut f) = file.lock()
        {
            let _ = f.flush();
        }
    }
}

/// Start logging. Without a log file (no home folder, a read-only one)
/// it still logs to the terminal.
pub fn init() {
    let verbose = std::env::var_os("RUST_LOG").is_some_and(|v| !v.is_empty());
    let filter = env_filter::Builder::new()
        .filter_level(LevelFilter::Info)
        .parse(&std::env::var("RUST_LOG").unwrap_or_default())
        .build();
    let file = path().and_then(|p| {
        std::fs::create_dir_all(p.parent()?).ok()?;
        if std::fs::metadata(&p).is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = std::fs::rename(&p, p.with_extension("log.old"));
        }
        OpenOptions::new().create(true).append(true).open(&p).ok()
    });
    let max = filter.filter();
    let logger = Logger {
        filter,
        file: file.map(Mutex::new),
        verbose,
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max);
    }
}

/// The end of the log, for a bug report.
pub fn tail(lines: usize) -> String {
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return String::new();
    };
    let all = text.lines().collect::<Vec<_>>();
    all[all.len().saturating_sub(lines)..].join("\n")
}
