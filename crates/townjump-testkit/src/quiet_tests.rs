use super::*;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

/// A thread writing into `capture` every `every` until stopped; returns the stop flag and the handle.
fn writer(capture: &Capture, every: Duration) -> (Arc<AtomicBool>, thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, mut out) = (stop.clone(), capture.clone());
    let handle = thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            out.write_all(b"x").unwrap();
            thread::sleep(every);
        }
    });
    (stop, handle)
}

fn finish(writer: (Arc<AtomicBool>, thread::JoinHandle<()>)) {
    writer.0.store(true, Ordering::SeqCst);
    writer.1.join().unwrap();
}

#[test]
fn completed_returns_code() {
    let started = Instant::now();
    assert_eq!(run_until_quiet(async { 7 }, &[], QUIET), Some(7));
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[test]
fn silent_returns_none_after_quiet() {
    let quiet = Duration::from_millis(200);
    let started = Instant::now();
    assert_eq!(run_until_quiet(std::future::pending(), &[], quiet), None);
    assert!(started.elapsed() >= quiet);
}

#[test]
fn output_resets_quiet() {
    let capture = Capture::new();
    // Gap tolerance is quiet - every = 230 ms: the writer thread may stall that long on a loaded
    // runner (Windows timer ticks are ~16 ms) before the silence window fires.
    let chatty = writer(&capture, Duration::from_millis(20));
    let run = async {
        tokio::time::sleep(Duration::from_millis(700)).await;
        0
    };
    let outcome = run_until_quiet(run, &[&capture], Duration::from_millis(250));
    finish(chatty);
    assert_eq!(outcome, Some(0));
}

#[test]
fn max_caps_endless_output() {
    let capture = Capture::new();
    let chatty = writer(&capture, Duration::from_millis(20));
    let started = Instant::now();
    let (quiet, max) = (Duration::from_millis(250), Duration::from_millis(700));
    let outcome = run_capped(std::future::pending(), &[&capture], quiet, max);
    let elapsed = started.elapsed();
    finish(chatty);
    assert_eq!(outcome, None);
    assert!(
        elapsed >= max && elapsed < Duration::from_millis(1500),
        "{elapsed:?}"
    );
}

#[test]
fn spawned_tasks_are_dropped() {
    let (tx, rx) = mpsc::channel::<SocketAddr>();
    let run = async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        tx.send(listener.local_addr().unwrap()).unwrap();
        tokio::spawn(async move { while listener.accept().await.is_ok() {} });
        std::future::pending().await
    };
    let outcome = run_until_quiet(run, &[], Duration::from_millis(100));
    let addr = rx.recv().unwrap();
    assert_eq!(outcome, None);
    let reached = TcpStream::connect_timeout(&addr, Duration::from_millis(500));
    assert!(reached.is_err(), "the listening socket outlived the case");
}

#[test]
#[should_panic(expected = "boom")]
fn panic_propagates() {
    run_until_quiet(async { panic!("boom") }, &[], QUIET);
}

#[test]
fn then_signal_gives_code_after() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let run = async move {
        rx.recv().await;
        130
    };
    let quiet = Duration::from_millis(100);
    let outcome = run_until_quiet_then(run, &[], quiet, || tx.send(()).unwrap());
    assert_eq!(outcome, (None, Some(130)));
}

#[test]
fn then_deaf_run_gives_none_within_cap() {
    let started = Instant::now();
    let quiet = Duration::from_millis(300);
    let outcome = run_until_quiet_then(std::future::pending(), &[], quiet, || {});
    assert_eq!(outcome, (None, None));
    assert!(started.elapsed() <= Duration::from_millis(3500));
}

#[test]
fn then_finished_run_skips_signal() {
    let outcome = run_until_quiet_then(async { 3 }, &[], QUIET, || panic!("no signal needed"));
    assert_eq!(outcome, (Some(3), None));
}
