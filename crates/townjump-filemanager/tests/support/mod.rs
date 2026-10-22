//! Shared helpers of the `--filemanager` server integration tests.
//! Each `tests/*.rs` is a separate crate and uses only some helpers, so dead_code and unused_imports are silenced.
#![allow(dead_code, unused_imports)]

mod wire;

pub use wire::{Conn, Reply, raw, request};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use townjump_filemanager::{FatalHook, Filemanager, FilemanagerOpts};
use townjump_jsval::JsValue;

/// Default credentials (`admin:admin`) as a header.
pub const ADMIN_AUTH: (&str, &str) = ("Authorization", "Basic YWRtaW46YWRtaW4=");
/// Credentials of the `--safe-mode` tests (`admin:s3cret`).
pub const SAFE_AUTH: (&str, &str) = ("Authorization", "Basic YWRtaW46czNjcmV0");
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// `<tmp>/hs-fm-<pid>-<n>/root` is the server root; next to it (`outside_exists`) lies what does not belong to the root.
/// The directory is removed in `Drop`.
pub struct TempRoot {
    base: PathBuf,
}

impl TempRoot {
    pub fn new() -> TempRoot {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("hs-fm-{}-{n}", std::process::id()));
        std::fs::create_dir_all(base.join("root")).expect("create temp root");
        TempRoot { base }
    }

    /// Server root as a string (value of `FilemanagerOpts.path`).
    pub fn path(&self) -> String {
        self.base.join("root").to_string_lossy().into_owned()
    }

    /// File with content; missing directories are created.
    pub fn write(&self, rel: &str, content: &str) {
        let file = self.base.join("root").join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).expect("create directory");
        std::fs::write(file, content).expect("write file");
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.base.join("root").join(rel)).expect("read file")
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.base.join("root").join(rel).is_dir()
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.base.join("root").join(rel).exists()
    }

    /// File next to the root (outside it).
    pub fn write_outside(&self, name: &str, content: &str) {
        let file = self.base.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).expect("create directory");
        std::fs::write(file, content).expect("write file");
    }

    /// Path `name` outside the root.
    pub fn outside(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }

    /// Link `rel` in the root pointing to `target`. The error is returned so a test can skip when links
    /// are not permitted (Windows without the symlink privilege, os error 1314).
    pub fn symlink(&self, rel: &str, target: &std::path::Path) -> std::io::Result<()> {
        let link = self.base.join("root").join(rel);
        std::fs::create_dir_all(link.parent().unwrap()).expect("create directory");
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        return if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
    }

    /// Whether `name` exists in the directory above the root (no file must end up there).
    pub fn outside_exists(&self, name: &str) -> bool {
        self.base.join(name).exists()
    }
}

impl Default for TempRoot {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

pub type Crashes = Arc<Mutex<Vec<String>>>;

/// `on_fatal` hook that collects crash texts into a shared list.
pub fn crashes() -> (FatalHook, Crashes) {
    let log: Crashes = Arc::default();
    let sink = Arc::clone(&log);
    let hook: FatalHook = Arc::new(move |msg| sink.lock().unwrap().push(msg));
    (hook, log)
}

/// Crash texts; the first is awaited up to 2 s (the hook is called from the connection task).
pub async fn crash_texts(log: &Crashes) -> Vec<String> {
    for _ in 0..200 {
        if !log.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    log.lock().unwrap().clone()
}

/// Server on port `"0"`: `Filemanager::new` + `ready` + address.
pub async fn start(mut opts: FilemanagerOpts) -> (Filemanager, SocketAddr) {
    opts.port = Some(JsValue::String("0".into()));
    let mut fm = Filemanager::new(opts).expect("Filemanager::new");
    fm.ready().await.expect("ready");
    let addr = fm.local_addr().expect("local_addr after ready");
    (fm, addr)
}

/// Server preconditions: role and credentials (`None` — livefiles defaults) and root files `(path, content)`.
#[derive(Default)]
pub struct Setup {
    pub role: Option<&'static str>,
    pub username: Option<&'static str>,
    pub password: Option<&'static str>,
    pub files: &'static [(&'static str, &'static str)],
    pub safe_mode: bool,
}

/// Running server over a temporary root.
pub struct Site {
    pub addr: SocketAddr,
    pub root: TempRoot,
    pub log: Crashes,
    _fm: Filemanager,
}

pub async fn setup(setup: Setup) -> Site {
    let root = TempRoot::new();
    for (rel, content) in setup.files {
        root.write(rel, content);
    }
    start_site(setup, root).await
}

/// `--safe-mode` with `admin:s3cret` (header `SAFE_AUTH`).
pub async fn safe_site(role: &'static str, files: &'static [(&'static str, &'static str)]) -> Site {
    let setup = Setup {
        role: Some(role),
        username: Some("admin"),
        password: Some("s3cret"),
        files,
        safe_mode: true,
    };
    let root = TempRoot::new();
    for (rel, content) in setup.files {
        root.write(rel, content);
    }
    start_site(setup, root).await
}

/// Server over an already populated root (for tests that need links before startup).
pub async fn start_site(setup: Setup, root: TempRoot) -> Site {
    let (hook, log) = crashes();
    let text = |v: Option<&str>| v.map(|s| JsValue::String(s.into()));
    let opts = FilemanagerOpts {
        path: Some(JsValue::String(root.path())),
        role: text(setup.role),
        username: text(setup.username),
        password: text(setup.password),
        safe_mode: setup.safe_mode,
        on_fatal: Some(hook),
        ..Default::default()
    };
    let (fm, addr) = start(opts).await;
    Site {
        addr,
        root,
        log,
        _fm: fm,
    }
}

impl Site {
    /// GET with `admin:admin`.
    pub async fn get(&self, target: &str) -> Reply {
        self.get_with(target, &[ADMIN_AUTH]).await
    }

    /// GET with exactly these headers (adds no authorization).
    pub async fn get_with(&self, target: &str, headers: &[(&str, &str)]) -> Reply {
        Reply::from_bytes(&self.raw_get(target, headers).await)
    }

    /// Same, but raw bytes: empty if the server closed the socket without a response (crash).
    pub async fn raw_get(&self, target: &str, headers: &[(&str, &str)]) -> Vec<u8> {
        raw(self.addr, &request("GET", target, headers, b"")).await
    }

    /// POST of a form with `admin:admin`.
    pub async fn post(&self, target: &str, form: &str) -> Reply {
        Reply::from_bytes(&self.raw_post(target, form.as_bytes()).await)
    }

    /// POST of a form with the given headers (adds no authorization).
    pub async fn post_with(&self, target: &str, headers: &[(&str, &str)], form: &str) -> Reply {
        Reply::from_bytes(
            &raw(
                self.addr,
                &request("POST", target, headers, form.as_bytes()),
            )
            .await,
        )
    }

    pub async fn raw_post(&self, target: &str, body: &[u8]) -> Vec<u8> {
        raw(self.addr, &request("POST", target, &[ADMIN_AUTH], body)).await
    }

    /// Crash texts that already reached the hook (no waiting).
    pub fn crashes_now(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Crash texts that reached the `on_fatal` hook.
    pub async fn crashes(&self) -> Vec<String> {
        crash_texts(&self.log).await
    }
}

/// All `parts` occur in `text` in the given order, each after the previous one.
pub fn assert_in_order(text: &str, parts: &[&str]) {
    let mut from = 0;
    for part in parts {
        let Some(at) = text[from..].find(part) else {
            let tail: String = text[from..].chars().take(800).collect();
            panic!("no `{part}` after position {from}; the rest of the body:\n{tail}");
        };
        from += at + part.len();
    }
}
