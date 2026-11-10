use super::*;

#[derive(Default)]
struct Buf(Vec<u8>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Failing;
impl Write for Failing {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("boom"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn setup(level: Option<f64>) -> (Logger, Arc<Mutex<Buf>>, Arc<Mutex<Buf>>) {
    let o = Arc::new(Mutex::new(Buf::default()));
    let e = Arc::new(Mutex::new(Buf::default()));
    let (so, se): (SharedOut, SharedOut) = (o.clone(), e.clone());
    (Logger::new(level, so, se), o, e)
}

fn text(b: &Arc<Mutex<Buf>>) -> String {
    String::from_utf8(b.lock().unwrap().0.clone()).unwrap()
}

fn all(l: &Logger) {
    l.debug("d");
    l.info("i");
    l.warn("w");
    l.error("e");
}

fn visible(level: Option<f64>) -> [bool; 4] {
    let (l, o, e) = setup(level);
    all(&l);
    let (o, e) = (text(&o), text(&e));
    let has = |s: &str, n: &str| s.contains(&format!("[{n}]"));
    [
        has(&o, "DEBUG"),
        has(&o, "INFO"),
        has(&e, "WARN"),
        has(&e, "ERROR"),
    ]
}

#[test]
fn level_table() {
    let t = true;
    let f = false;
    assert_eq!(visible(None), [f, f, f, f]);
    assert_eq!(visible(Some(0.0)), [t, t, t, t]);
    assert_eq!(visible(Some(1.0)), [f, t, t, t]);
    assert_eq!(visible(Some(2.0)), [f, f, t, t]);
    assert_eq!(visible(Some(3.0)), [f, f, f, t]);
    assert_eq!(visible(Some(4.0)), [f, f, f, f]);
    assert_eq!(visible(Some(0.5)), [f, t, t, t]);
    assert_eq!(visible(Some(2.5)), [f, f, f, t]);
    assert_eq!(visible(Some(3.5)), [f, f, f, f]);
    assert_eq!(visible(Some(-1.0)), [t, t, t, t]);
    assert_eq!(visible(Some(f64::NEG_INFINITY)), [t, t, t, t]);
    assert_eq!(visible(Some(f64::INFINITY)), [f, f, f, f]);
    assert_eq!(visible(Some(f64::NAN)), [t, t, t, t]);
}

#[test]
fn line_format_and_routing() {
    let (l, o, e) = setup(Some(0.0));
    all(&l);
    let (o, e) = (text(&o), text(&e));
    let lines: Vec<&str> = o.lines().chain(e.lines()).collect();
    assert_eq!(lines.len(), 4);
    for (line, (c, n, m)) in lines.iter().zip([
        ("90", "DEBUG", "d"),
        ("32", "INFO", "i"),
        ("33", "WARN", "w"),
        ("31", "ERROR", "e"),
    ]) {
        let rest = line.strip_prefix("\x1b[90m").unwrap();
        let (time, rest) = rest.split_once("\x1b[0m ").unwrap();
        assert_eq!(time.len(), 24);
        assert_eq!(
            rest,
            format!("\x1b[34m[Holesail]\x1b[0m \x1b[{c}m[{n}]\x1b[0m {m}")
        );
    }
    assert!(o.contains("[DEBUG]") && o.contains("[INFO]") && !o.contains("[WARN]"));
    assert!(e.contains("[WARN]") && e.contains("[ERROR]") && !e.contains("[INFO]"));
}

fn now_millis() -> u64 {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    u64::try_from(d.as_millis()).unwrap()
}

#[test]
fn line_time_is_current_utc() {
    let (l, o, _) = setup(Some(1.0));
    let before = crate::time::iso_from_millis(now_millis());
    l.info("x");
    let after = crate::time::iso_from_millis(now_millis());
    let out = text(&o);
    let rest = out.strip_prefix("\x1b[90m").unwrap();
    let (time, _) = rest.split_once("\x1b[0m ").unwrap();
    // fixed-width ISO strings compare in time order
    assert!(
        before.as_str() <= time && time <= after.as_str(),
        "{before} {time} {after}"
    );
}

#[test]
fn message_is_verbatim() {
    let (l, o, _) = setup(Some(0.0));
    l.info("  %s {} κ\nx  ");
    l.info("");
    let s = text(&o);
    assert!(s.contains("[INFO]\x1b[0m   %s {} κ\nx  \n"));
    assert!(s.ends_with("[INFO]\x1b[0m \n"));
}

#[test]
fn suppressed_message_writes_nothing() {
    let (l, o, e) = setup(Some(3.0));
    l.debug("x");
    l.info("x");
    assert!(text(&o).is_empty() && text(&e).is_empty());
}

#[test]
fn write_error_is_ignored() {
    let f: SharedOut = Arc::new(Mutex::new(Failing));
    let l = Logger::new(Some(0.0), f.clone(), f);
    all(&l);
}

#[test]
fn poisoned_lock_still_writes() {
    let (l, o, _) = setup(Some(0.0));
    let o2 = o.clone();
    let _ = std::thread::spawn(move || {
        let _g = o2.lock().unwrap();
        panic!("poison");
    })
    .join();
    l.info("after");
    assert!(o.lock().unwrap_or_else(|e| e.into_inner()).0.len() > 5);
}

#[test]
fn threads_do_not_interleave_and_clones_share() {
    let (l, o, _) = setup(Some(0.0));
    let hs: Vec<_> = (0..4)
        .map(|i| {
            let l = l.clone();
            std::thread::spawn(move || (0..50).for_each(|_| l.info(&format!("msg{i}"))))
        })
        .collect();
    hs.into_iter().for_each(|h| h.join().unwrap());
    let s = text(&o);
    assert_eq!(s.lines().count(), 200);
    assert!(s.lines().all(|x| x.contains("[INFO]\x1b[0m msg")));
}
