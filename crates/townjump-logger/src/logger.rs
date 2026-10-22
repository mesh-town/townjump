// JS: holesail-logger/index.js:HolesailLogger.log (line format, ANSI colors, levels 0..3).
// Related: fatal.rs (errors after which the process exits with code 1).
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared output stream: the CLI and tests substitute their own (a buffer instead of stdout/stderr).
pub type SharedOut = Arc<Mutex<dyn Write + Send>>;

/// `[Holesail]` log: DEBUG and INFO go to stdout, WARN and ERROR to stderr; `Clone` shares the streams.
#[derive(Clone)]
pub struct Logger {
    level: Option<f64>,
    stdout: SharedOut,
    stderr: SharedOut,
}

impl Logger {
    /// `level`: `None` — log off; otherwise records with level number `>= level` are printed (0 DEBUG … 3 ERROR).
    pub fn new(level: Option<f64>, stdout: SharedOut, stderr: SharedOut) -> Logger {
        Logger {
            level,
            stdout,
            stderr,
        }
    }

    /// `<ISO-time> [Holesail] [DEBUG] msg` to stdout.
    pub fn debug(&self, msg: &str) {
        self.log(0.0, ("90", "DEBUG"), &self.stdout, msg);
    }

    /// `<ISO-time> [Holesail] [INFO] msg` to stdout.
    pub fn info(&self, msg: &str) {
        self.log(1.0, ("32", "INFO"), &self.stdout, msg);
    }

    /// `<ISO-time> [Holesail] [WARN] msg` to stderr.
    pub fn warn(&self, msg: &str) {
        self.log(2.0, ("33", "WARN"), &self.stderr, msg);
    }

    /// `<ISO-time> [Holesail] [ERROR] msg` to stderr.
    pub fn error(&self, msg: &str) {
        self.log(3.0, ("31", "ERROR"), &self.stderr, msg);
    }

    // JS: `!enabled || type < minLevel` — with NaN the comparison is false, the record is printed.
    fn log(&self, kind: f64, (color, name): (&str, &str), out: &SharedOut, msg: &str) {
        let Some(min) = self.level else { return };
        if kind < min {
            return;
        }
        let line = format!(
            "\x1b[90m{}\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[{color}m[{name}]\x1b[0m {msg}\n",
            iso_now()
        );
        let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = out.write_all(line.as_bytes());
    }
}

/// Like `new Date().toISOString()`: `2026-01-02T03:04:05.678Z`.
fn iso_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (secs, ms) = (now.as_secs(), now.subsec_millis());
    let (y, m, d) = civil(secs / 86_400);
    let day = secs % 86_400;
    let (h, mi, s) = (day / 3600, day % 3600 / 60, day % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
}

/// Days since 1970-01-01 to (year, month, day) in the Gregorian calendar.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + u64::from(m <= 2), m, d)
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod tests;
