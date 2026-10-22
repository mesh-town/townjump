// JS: livefiles/index.js:constructor (defaults for role/username/password/port/host, path check).
use std::sync::Arc;
use townjump_jsval::{JsError, JsValue};

pub type FatalHook = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Default, Clone)]
pub struct FilemanagerOpts {
    pub path: Option<JsValue>,
    pub role: Option<JsValue>,
    pub username: Option<JsValue>,
    pub password: Option<JsValue>,
    pub host: Option<JsValue>,
    pub port: Option<JsValue>,
    /// `--safe-mode`: hardened mode (see `route/safe.rs`).
    pub safe_mode: bool,
    pub on_fatal: Option<FatalHook>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilemanagerError {
    NoPath,
    Port(JsError),
    DefaultCredentials,
    SafeRoot(String),
}

impl std::fmt::Display for FilemanagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilemanagerError::NoPath => f.write_str("Error: INCORRECT OR NO PATH SPECIFIED"),
            FilemanagerError::DefaultCredentials => f.write_str(
                "Error: --safe-mode refuses the default password; set --password (and --username)",
            ),
            FilemanagerError::SafeRoot(e) => {
                write!(
                    f,
                    "Error: --safe-mode cannot resolve the root directory: {e}"
                )
            }
            FilemanagerError::Port(JsError::Range { code, message }) => {
                write!(f, "RangeError [{code}]: {message}")
            }
            FilemanagerError::Port(e) => write!(f, "{}: {e}", e.name()),
        }
    }
}

impl std::error::Error for FilemanagerError {}

/// Ready server configuration (after defaults).
pub(crate) struct Config {
    pub root: String,
    pub admin: bool,
    pub username: JsValue,
    pub password: JsValue,
    pub host: JsValue,
    pub port: JsValue,
    pub on_fatal: Option<FatalHook>,
    /// `Some` — `--safe-mode` is on; holds the canonical root, resolved once at startup.
    pub safe_root: Option<std::path::PathBuf>,
    /// Windows path-name rules are active (`cfg!(windows)`; tests set it by hand).
    pub win_rules: bool,
}

impl Config {
    pub(crate) fn safe(&self) -> bool {
        self.safe_root.is_some()
    }
}

pub(crate) fn config(opts: FilemanagerOpts) -> Result<Config, FilemanagerError> {
    let root = match &opts.path {
        Some(JsValue::String(p)) if !p.is_empty() && std::path::Path::new(p).exists() => p.clone(),
        _ => return Err(FilemanagerError::NoPath),
    };
    let admin = matches!(&opts.role, Some(JsValue::String(r)) if r == "admin");
    let username = pick(opts.username, JsValue::String("admin".into()));
    let password = pick(opts.password, JsValue::String("admin".into()));
    let safe_root = if opts.safe_mode {
        if password == JsValue::String("admin".into()) {
            return Err(FilemanagerError::DefaultCredentials);
        }
        let canon =
            std::fs::canonicalize(&root).map_err(|e| FilemanagerError::SafeRoot(e.to_string()))?;
        Some(canon)
    } else {
        None
    };
    Ok(Config {
        root,
        admin,
        username,
        password,
        host: pick(opts.host, JsValue::String("127.0.0.1".into())),
        port: pick(opts.port, JsValue::Number(5409.0)),
        on_fatal: opts.on_fatal,
        safe_root,
        win_rules: cfg!(windows),
    })
}

/// `v && typeof v !== 'boolean' ? v : default`.
fn pick(v: Option<JsValue>, default: JsValue) -> JsValue {
    match v {
        Some(v) if v.truthy() && !matches!(v, JsValue::Bool(_)) => v,
        _ => default,
    }
}
