// JS: none — a port extension; holesail JS starts every node with an empty table.
// `--save-nodes <file>`, `--save-nodes-ttl <hours>`, `--save-nodes-every <seconds>`.
use super::super::args::flag;
use std::path::PathBuf;
use std::time::Duration;
use townjump_jsval::JsValue;

const DEFAULT_TTL_HOURS: u64 = 168;
const DEFAULT_EVERY_SECS: u64 = 300;

const PATH_ERR: &str = "--save-nodes needs a file path";
const TTL_ERR: &str = "--save-nodes-ttl needs a positive number of hours";
const EVERY_ERR: &str = "--save-nodes-every needs a positive number of seconds";

/// Saved-nodes settings; present only when `--save-nodes` is given.
pub(crate) struct NodesOpts {
    pub path: PathBuf,
    pub ttl: Duration,
    pub every: Duration,
}

/// `Ok(None)` — the flag is absent; `Err` — text for stderr (exit code 1).
pub(crate) fn nodes_opts(args: &JsValue) -> Result<Option<NodesOpts>, String> {
    let Some(value) = flag(args, "save-nodes") else {
        return Ok(None);
    };
    let path = path_of(value)?;
    let ttl = seconds(
        args,
        "save-nodes-ttl",
        3600.0,
        DEFAULT_TTL_HOURS * 3600,
        TTL_ERR,
    )?;
    let every = seconds(args, "save-nodes-every", 1.0, DEFAULT_EVERY_SECS, EVERY_ERR)?;
    Ok(Some(NodesOpts { path, ttl, every }))
}

fn path_of(value: &JsValue) -> Result<PathBuf, String> {
    match value {
        JsValue::String(s) if !s.is_empty() => Ok(PathBuf::from(s)),
        JsValue::Number(_) => Ok(PathBuf::from(value.to_js_string())),
        _ => Err(PATH_ERR.to_owned()),
    }
}

fn seconds(
    args: &JsValue,
    name: &str,
    unit: f64,
    default: u64,
    err: &str,
) -> Result<Duration, String> {
    match flag(args, name) {
        None => Ok(Duration::from_secs(default)),
        Some(JsValue::Number(n)) if n.is_finite() && *n > 0.0 => {
            Ok(Duration::from_secs_f64(n * unit))
        }
        Some(_) => Err(err.to_owned()),
    }
}

#[cfg(test)]
#[path = "opts_tests.rs"]
mod tests;
