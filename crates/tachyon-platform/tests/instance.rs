use std::sync::mpsc;
use std::time::Duration;

use tachyon_platform::{Instance, Listener};

fn unique_id(test: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .subsec_nanos();
    format!("tachyon-test-{test}-{}-{nanos}", std::process::id())
}

fn primary(id: &str) -> Listener {
    match Instance::acquire(id).expect("instance claim") {
        Instance::Primary(listener) => listener,
        Instance::Secondary(_) => panic!("first claim must be primary"),
    }
}

fn secondary(id: &str) -> tachyon_platform::Client {
    match Instance::acquire(id).expect("instance claim") {
        Instance::Secondary(client) => client,
        Instance::Primary(_) => panic!("claim while a primary is alive must be secondary"),
    }
}

#[test]
fn second_claim_forwards_arguments_to_primary() {
    let id = unique_id("forward");
    let (tx, rx) = mpsc::channel();
    primary(&id).spawn(move |args| tx.send(args).unwrap());

    for batch in [vec!["--paste".to_owned()], vec!["--".to_owned(), "/tmp/a b.md".to_owned()]] {
        secondary(&id).send(&batch).unwrap();
        // `send` returns only after the primary acknowledged the launch.
        assert_eq!(rx.try_recv().or_else(|_| rx.recv_timeout(Duration::from_secs(1))), Ok(batch));
    }
}

#[test]
fn launch_sent_before_the_primary_serves_is_still_delivered() {
    let id = unique_id("early");
    let listener = primary(&id);
    let client = secondary(&id);
    let sender = std::thread::spawn(move || client.send(&["early.md".to_owned()]));

    std::thread::sleep(Duration::from_millis(200));
    let (tx, rx) = mpsc::channel();
    listener.spawn(move |args| tx.send(args).unwrap());

    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(vec!["early.md".to_owned()]));
    sender.join().unwrap().unwrap();
}

#[test]
fn send_fails_when_the_primary_is_gone() {
    let id = unique_id("gone");
    let listener = primary(&id);
    let client = secondary(&id);
    drop(listener);

    assert!(client.send(&["lost.md".to_owned()]).is_err());
}
