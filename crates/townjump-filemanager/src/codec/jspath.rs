// JS: node/lib/path (posix: join, normalize, extname, basename) and livefiles/index.js (root-escape check).
use super::winname;

pub(crate) fn join(parts: &[&str]) -> String {
    let joined: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if joined.is_empty() {
        return ".".to_owned();
    }
    normalize(&joined.join("/"))
}

pub(crate) fn normalize(p: &str) -> String {
    if p.is_empty() {
        return ".".to_owned();
    }
    let abs = p.starts_with('/');
    let mut segs: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." if segs.last().is_some_and(|l| *l != "..") => {
                segs.pop();
            }
            ".." if abs => {}
            s => segs.push(s),
        }
    }
    let mut body = segs.join("/");
    if body.is_empty() {
        return match (abs, p.ends_with('/')) {
            (true, _) => "/".to_owned(),
            (false, true) => "./".to_owned(),
            (false, false) => ".".to_owned(),
        };
    }
    if p.ends_with('/') {
        body.push('/');
    }
    if abs { format!("/{body}") } else { body }
}

pub(crate) fn basename(p: &str) -> String {
    let t = p.trim_end_matches('/');
    t.rsplit('/').next().unwrap_or("").to_owned()
}

pub(crate) fn extname(p: &str) -> String {
    let base = basename(p);
    if base == ".." {
        return String::new();
    }
    match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_owned(),
        _ => String::new(),
    }
}

/// Component-wise comparison with `normalize(root)` without a trailing `/`.
pub(crate) fn escapes_root(root: &str, full: &str) -> bool {
    escapes_root_as(root, full, cfg!(windows))
}

/// `win`: `\` is a separator, a component with `:` after the root (drive, ADS) is a root escape.
pub(crate) fn escapes_root_as(root: &str, full: &str, win: bool) -> bool {
    let (root, full) = if win {
        (root.replace('\\', "/"), full.replace('\\', "/"))
    } else {
        (root.to_owned(), full.to_owned())
    };
    let (r, f) = (normalize(&root), normalize(&full));
    if r.starts_with('/') != f.starts_with('/') {
        return true;
    }
    let (rc, fc) = (components(&r), components(&f));
    if rc.is_empty() && !r.starts_with('/') {
        return fc.first() == Some(&"..") || (win && fc.iter().any(|c| c.contains(':')));
    }
    if !fc.starts_with(&rc) || fc[rc.len()..].contains(&"..") {
        return true;
    }
    win && fc[rc.len()..].iter().any(|c| c.contains(':'))
}

/// `win`: a component after the root is a Windows device name or ends with `.`/space.
pub(crate) fn win_names_as(root: &str, full: &str, win: bool) -> bool {
    if !win {
        return false;
    }
    let (r, f) = (
        normalize(&root.replace('\\', "/")),
        normalize(&full.replace('\\', "/")),
    );
    if r.starts_with('/') != f.starts_with('/') {
        return false;
    }
    let (rc, fc) = (components(&r), components(&f));
    fc.starts_with(&rc) && fc[rc.len()..].iter().any(|c| winname::unsafe_component(c))
}

fn components(p: &str) -> Vec<&str> {
    p.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

#[cfg(test)]
#[path = "jspath_tests.rs"]
mod tests;
