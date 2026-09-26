use std::sync::mpsc;
use std::time::Duration;

use tachyon_platform::Instance;

fn unique_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .subsec_nanos();
    format!("tachyon-test-{}-{nanos}", std::process::id())
}

#[test]
fn second_claim_forwards_arguments_to_primary() {
    let id = unique_id();
    let Instance::Primary(listener) = Instance::acquire(&id).unwrap() else {
        panic!("first claim must be primary");
    };
    let (tx, rx) = mpsc::channel();
    listener.spawn(move |args| tx.send(args).unwrap());

    for batch in [vec!["--paste".to_owned()], vec!["--".to_owned(), "/tmp/a b.md".to_owned()]] {
        let Instance::Secondary(client) = Instance::acquire(&id).unwrap() else {
            panic!("claim while primary is alive must be secondary");
        };
        client.send(&batch).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), batch);
    }
}
