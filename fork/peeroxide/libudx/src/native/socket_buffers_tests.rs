// Tests of the socket buffer size request (socket.rs): the SO_RCVBUF ladder and the actual size after bind.
use std::io;

use socket2::SockRef;

use super::*;

const FLOOR: usize = 208 * 1024;

fn refuse_above(limit: usize, calls: &mut Vec<usize>) -> impl FnMut(usize) -> io::Result<()> + '_ {
    move |n| {
        calls.push(n);
        match n > limit {
            true => Err(io::Error::from_raw_os_error(55)),
            false => Ok(()),
        }
    }
}

#[test]
fn ladder_is_the_reference_ladder() {
    assert_eq!(RCVBUF_LADDER, [1 << 20, 512 << 10, 256 << 10, 208 << 10]);
    assert_eq!(SNDBUF_SIZE, 212_992);
}

#[test]
fn ladder_takes_first_size_when_system_accepts_it() {
    let mut calls = Vec::new();
    let got = set_first_accepted(&RCVBUF_LADDER, refuse_above(usize::MAX, &mut calls)).unwrap();
    assert_eq!((got, calls), (1 << 20, vec![1 << 20]));
}

#[test]
fn ladder_descends_while_system_refuses() {
    let mut calls = Vec::new();
    let got = set_first_accepted(&RCVBUF_LADDER, refuse_above(300 << 10, &mut calls)).unwrap();
    assert_eq!(
        (got, calls),
        (256 << 10, vec![1 << 20, 512 << 10, 256 << 10])
    );
}

#[test]
fn ladder_reaches_the_old_maximum() {
    let mut calls = Vec::new();
    let got = set_first_accepted(&RCVBUF_LADDER, refuse_above(FLOOR, &mut calls)).unwrap();
    assert_eq!((got, calls.len()), (FLOOR, 4));
}

#[test]
fn refusal_on_whole_ladder_is_an_error_with_text() {
    let mut calls = Vec::new();
    let err = set_first_accepted(&RCVBUF_LADDER, refuse_above(0, &mut calls)).unwrap_err();
    assert_eq!(calls.len(), 4);
    assert!(err.to_string().contains("SO_RCVBUF"), "{err}");
}

#[test]
fn linux_raw_size_is_halved_like_libuv() {
    let expected = if cfg!(any(target_os = "linux", target_os = "android")) {
        1 << 20
    } else {
        2 << 20
    };
    assert_eq!(as_requested(2 << 20), expected);
}

async fn bound() -> UdxSocket {
    let socket = UdxSocket::new();
    socket.bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    socket
}

#[tokio::test]
async fn bound_socket_recv_buffer_is_not_below_208_kib() {
    let socket = bound().await;
    let udp = socket.udp_arc().unwrap();
    let actual = as_requested(SockRef::from(&*udp).recv_buffer_size().unwrap());
    eprintln!(
        "SO_RCVBUF after bind: {actual} (requested 1 MiB; on Linux — after halving the kernel doubling)"
    );
    assert!(actual >= FLOOR, "SO_RCVBUF after bind: {actual}");
}

#[tokio::test]
async fn bound_socket_send_buffer_is_at_least_the_reference_default() {
    let socket = bound().await;
    let udp = socket.udp_arc().unwrap();
    let actual = as_requested(SockRef::from(&*udp).send_buffer_size().unwrap());
    assert!(actual >= SNDBUF_SIZE, "SO_SNDBUF after bind: {actual}");
}
