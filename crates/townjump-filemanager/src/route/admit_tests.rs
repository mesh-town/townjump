use super::*;
use crate::http::head::Headers;

fn cfg(win_rules: bool, safe: bool) -> Config {
    Config {
        root: "/srv/share".to_owned(),
        admin: false,
        username: JsValue::String("admin".into()),
        password: JsValue::String("admin".into()),
        host: JsValue::String("127.0.0.1".into()),
        port: JsValue::Number(0.0),
        on_fatal: None,
        safe_root: safe.then(|| "/srv/share".into()),
        win_rules,
    }
}

fn get(target: &str, auth: bool) -> RequestHead {
    let mut headers = Headers::default();
    if auth {
        let v = "Basic YWRtaW46YWRtaW4=".to_owned();
        headers.0.push(("authorization".to_owned(), v));
    }
    RequestHead {
        method: "GET".to_owned(),
        target: target.to_owned(),
        minor: 1,
        headers,
    }
}

fn is_forbidden(step: Step) -> bool {
    matches!(step, Step::Done(Outcome::Reply(r))
        if r.status == 403 && matches!(&r.body, Body::Bytes(b) if b == b"Forbidden"))
}

const BAD: [&str; 4] = ["/CON", "/nul.txt", "/f.", "/sub/COM1"];

#[test]
fn win_names_are_forbidden_in_safe_mode() {
    let c = cfg(true, true);
    for t in BAD {
        assert!(is_forbidden(admit(&c, &get(t, true))), "{t}");
    }
    assert!(matches!(admit(&c, &get("/ok.txt", true)), Step::Get { .. }));
}

#[test]
fn win_names_pass_without_rules_or_safe_mode() {
    for c in [cfg(false, true), cfg(true, false)] {
        for t in BAD {
            assert!(matches!(admit(&c, &get(t, true)), Step::Get { .. }), "{t}");
        }
    }
}

#[test]
fn password_check_comes_first() {
    let c = cfg(true, true);
    let Step::Done(Outcome::Reply(r)) = admit(&c, &get("/CON", false)) else {
        panic!("not Reply")
    };
    assert_eq!(r.status, 401);
}
