//! Local crash logging. Nothing is ever sent off the machine.
//!
//! A panic hook appends the panic message, location, and backtrace to
//! `~/Library/Application Support/CompareRS/crash.log`. On the next
//! launch the app offers to show the log if it grew since it was last seen.

use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_dirs;

const LOG_FILE: &str = "crash.log";
/// Stores the crash log length the user has already been told about.
const SEEN_FILE: &str = "crash.log.seen";

fn log_dir() -> Option<PathBuf> {
    app_dirs::support_dir()
}

/// Installs a panic hook that records panics in the crash log before running
/// the default hook.
pub fn install() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Logging is best effort: a failure here must not mask the panic.
        let _ = append_entry(
            EntryKind::Crash,
            &format!(
                "panic on thread '{}' at {}: {}\n{}",
                std::thread::current().name().unwrap_or("<unnamed>"),
                info.location().map_or_else(|| "<unknown>".to_owned(), |l| l.to_string()),
                panic_message(info.payload()),
                Backtrace::force_capture(),
            ),
        );
        default_hook(info);
    }));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "<non-string panic payload>"
    }
}

/// What an entry in the log records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Crash,
    Error,
}

/// Header that starts every crash entry; only these trigger the dialog.
const CRASH_HEADER: &str = "=== crash";

/// Records an error the user was shown (for example a failed comparison) so
/// it can be diagnosed later.
pub fn record_error(context: &str, message: &str) {
    // Best effort: the error has already been shown in the UI.
    let _ = append_entry(EntryKind::Error, &format!("{context}: {message}"));
}

fn append_entry(kind: EntryKind, entry: &str) -> std::io::Result<()> {
    let Some(dir) = log_dir() else {
        return Ok(());
    };
    fs::create_dir_all(&dir)?;
    let mut file = OpenOptions::new().create(true).append(true).open(dir.join(LOG_FILE))?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let header = match kind {
        EntryKind::Crash => CRASH_HEADER,
        EntryKind::Error => "=== error",
    };
    writeln!(file, "{header} at unix time {timestamp} ===\n{entry}\n")
}

/// Returns the crash log path if it contains crashes the user has not been
/// told about yet, and marks them as seen.
pub fn take_unseen_crash() -> Option<PathBuf> {
    let dir = log_dir()?;
    let log = dir.join(LOG_FILE);
    let len = fs::metadata(&log).ok()?.len();
    let seen_path = dir.join(SEEN_FILE);
    let seen: u64 =
        fs::read_to_string(&seen_path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if len <= seen {
        return None;
    }
    // Best effort: if this fails the user is simply told again next launch.
    let _ = fs::write(&seen_path, len.to_string());
    let contents = fs::read(&log).ok()?;
    let unseen = contents.get(seen as usize..).unwrap_or(&contents);
    let has_crash = unseen.windows(CRASH_HEADER.len()).any(|w| w == CRASH_HEADER.as_bytes());
    has_crash.then_some(log)
}
