// Tests of `Fatal`: the channel for failures that crash the process in JS (holesail.mjs, exit code 1); read synchronously, no runtime.
use super::*;
use tokio::sync::mpsc::error::TryRecvError;

mod raise {
    use super::*;

    #[test]
    fn message_reaches_the_receiver() {
        let (fatal, mut rx) = Fatal::channel();
        fatal.raise("boom");
        assert_eq!(rx.try_recv(), Ok("boom".to_string()));
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Empty),
            "no extra messages left"
        );
    }

    #[test]
    fn accepts_string_and_str() {
        let (fatal, mut rx) = Fatal::channel();
        fatal.raise(String::from("owned"));
        fatal.raise("borrowed");
        assert_eq!(drain(&mut rx), ["owned", "borrowed"]);
    }

    #[test]
    fn text_is_delivered_verbatim() {
        let (fatal, mut rx) = Fatal::channel();
        for text in [
            "",
            "Error: x\n    at f (file.js:1:1)",
            "κλωξ 😀",
            "\u{1b}[31mred\u{1b}[0m",
        ] {
            fatal.raise(text);
            assert_eq!(rx.try_recv(), Ok(text.to_string()), "text {text:?}");
        }
    }

    #[test]
    fn keeps_order_of_one_sender() {
        let (fatal, mut rx) = Fatal::channel();
        for i in 0..50 {
            fatal.raise(format!("m{i}"));
        }
        let want: Vec<String> = (0..50).map(|i| format!("m{i}")).collect();
        assert_eq!(drain(&mut rx), want);
    }

    #[test]
    fn lost_receiver_is_not_a_panic() {
        let (fatal, rx) = Fatal::channel();
        drop(rx);
        fatal.raise("nobody listens");
        fatal.clone().raise("nobody listens either");
    }
}

mod clones {
    use super::*;

    #[test]
    fn share_the_same_receiver() {
        let (fatal, mut rx) = Fatal::channel();
        let other = fatal.clone();
        fatal.raise("first");
        other.raise("second");
        assert_eq!(drain(&mut rx), ["first", "second"]);
    }

    #[test]
    fn many_threads_lose_nothing_and_keep_per_sender_order() {
        let (fatal, mut rx) = Fatal::channel();
        let workers: Vec<_> = (0..8)
            .map(|t| {
                let fatal = fatal.clone();
                std::thread::spawn(move || (0..100).for_each(|i| fatal.raise(format!("{t}:{i}"))))
            })
            .collect();
        workers.into_iter().for_each(|w| w.join().expect("thread"));
        let got = drain(&mut rx);
        assert_eq!(got.len(), 800, "all messages arrived");
        for t in 0..8 {
            let own: Vec<String> = got
                .iter()
                .filter(|m| m.starts_with(&format!("{t}:")))
                .cloned()
                .collect();
            let want: Vec<String> = (0..100).map(|i| format!("{t}:{i}")).collect();
            assert_eq!(own, want, "sender order {t}");
        }
    }
}

mod channel {
    use super::*;

    #[test]
    fn closes_when_every_sender_is_dropped() {
        let (fatal, mut rx) = Fatal::channel();
        let other = fatal.clone();
        drop(fatal);
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Empty),
            "one sender is still alive"
        );
        drop(other);
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }
}

fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}
