use crate::time::now_iso;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// A lock-protected writer shared between the logger copies (tests pass buffers).
pub type SharedOut = Arc<Mutex<dyn Write + Send>>;

#[derive(Clone, Copy)]
enum Sev {
    Debug,
    Info,
    Warn,
    Error,
}

impl Sev {
    fn rank(self) -> f64 {
        match self {
            Sev::Debug => 0.0,
            Sev::Info => 1.0,
            Sev::Warn => 2.0,
            Sev::Error => 3.0,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Sev::Debug => "DEBUG",
            Sev::Info => "INFO",
            Sev::Warn => "WARN",
            Sev::Error => "ERROR",
        }
    }
    fn color(self) -> &'static str {
        match self {
            Sev::Debug => "90",
            Sev::Info => "32",
            Sev::Warn => "33",
            Sev::Error => "31",
        }
    }
}

/// Cheap-to-clone logger; all copies share streams and level.
#[derive(Clone)]
pub struct Logger {
    level: Option<f64>,
    stdout: SharedOut,
    stderr: SharedOut,
}

impl Logger {
    pub fn new(level: Option<f64>, stdout: SharedOut, stderr: SharedOut) -> Logger {
        Logger {
            level,
            stdout,
            stderr,
        }
    }

    pub fn debug(&self, msg: &str) {
        self.emit(Sev::Debug, msg);
    }
    pub fn info(&self, msg: &str) {
        self.emit(Sev::Info, msg);
    }
    pub fn warn(&self, msg: &str) {
        self.emit(Sev::Warn, msg);
    }
    pub fn error(&self, msg: &str) {
        self.emit(Sev::Error, msg);
    }

    fn emit(&self, sev: Sev, msg: &str) {
        let Some(level) = self.level else { return };
        // "skip if below", not "print if at least": NaN must print everything.
        if sev.rank() < level {
            return;
        }
        let line = format!(
            "\x1b[90m{}\x1b[0m \x1b[34m[Holesail]\x1b[0m \x1b[{}m[{}]\x1b[0m {msg}\n",
            now_iso(),
            sev.color(),
            sev.name()
        );
        let out = match sev {
            Sev::Debug | Sev::Info => &self.stdout,
            Sev::Warn | Sev::Error => &self.stderr,
        };
        // A poisoned lock still guards a usable writer; write errors are dropped.
        let mut w = out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(line.as_bytes());
    }
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod tests;
