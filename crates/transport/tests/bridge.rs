//! The simulator file bridge, both ends: requests come out in order and whole, answers land under
//! their request id, and a pinned seed is private.

use std::time::{Duration, Instant};

use fps_protocol::{encode, Envelope, FixedBytes, Response};
use fps_transport::bridge::Bridge;

fn bridge() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    let bridge = Bridge::new(dir.path().join("sim"));
    bridge.create().unwrap();
    (dir, bridge)
}

#[test]
fn requests_come_out_in_the_order_they_went_in() {
    let (_dir, bridge) = bridge();
    for n in 0..5u8 {
        bridge.submit(&[n; 3]).unwrap();
    }
    for n in 0..5u8 {
        assert_eq!(bridge.take_request().unwrap(), Some(vec![n; 3]));
    }
    assert_eq!(bridge.take_request().unwrap(), None);
}

#[test]
fn a_request_still_being_written_is_left_alone() {
    let (_dir, bridge) = bridge();
    std::fs::write(bridge.inbox().join("0001.part"), b"half").unwrap();
    std::fs::write(bridge.inbox().join("notes.txt"), b"not a message").unwrap();
    assert_eq!(bridge.take_request().unwrap(), None);
    assert!(bridge.inbox().join("0001.part").exists());
}

#[test]
fn a_taken_request_is_gone() {
    let (_dir, bridge) = bridge();
    bridge.submit(b"once").unwrap();
    assert_eq!(bridge.take_request().unwrap(), Some(b"once".to_vec()));
    let left: Vec<_> = std::fs::read_dir(bridge.inbox()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn an_answer_is_filed_under_its_request_id() {
    let (_dir, bridge) = bridge();
    let id = FixedBytes([7; 16]);
    let answer = encode(&Envelope::new(id, Response::Pending));
    let path = bridge.write_response(&answer).unwrap();
    assert_eq!(
        path.file_name().unwrap(),
        "07070707070707070707070707070707.cbor"
    );
    assert_eq!(bridge.take_response(&id).unwrap(), Some(answer));
    assert_eq!(bridge.take_response(&id).unwrap(), None);
}

#[test]
fn an_unreadable_answer_is_filed_as_unaddressed() {
    let (_dir, bridge) = bridge();
    let path = bridge.write_response(b"junk").unwrap();
    assert_eq!(path.file_name().unwrap(), "unaddressed.cbor");
}

#[test]
fn waiting_for_an_answer_gives_up_on_time() {
    let (_dir, bridge) = bridge();
    let started = Instant::now();
    let answer = bridge
        .wait_response(&FixedBytes([1; 16]), Duration::from_millis(120))
        .unwrap();
    assert_eq!(answer, None);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn waiting_returns_an_answer_that_arrives() {
    let (_dir, bridge) = bridge();
    let id = FixedBytes([3; 16]);
    let answer = encode(&Envelope::new(id, Response::Cancelled));
    let writer = {
        let bridge = bridge.clone();
        let answer = answer.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            bridge.write_response(&answer).unwrap();
        })
    };
    let received = bridge.wait_response(&id, Duration::from_secs(5)).unwrap();
    writer.join().unwrap();
    assert_eq!(received, Some(answer));
}

#[test]
fn without_a_pinned_seed_the_app_uses_its_own() {
    let (_dir, bridge) = bridge();
    assert_eq!(bridge.pinned_app_seed().unwrap(), None);
}

#[test]
fn a_pinned_seed_is_private_and_replaceable() {
    let (_dir, bridge) = bridge();
    bridge.pin_app_seed(&[0x11; 32]).unwrap();
    assert_eq!(bridge.pinned_app_seed().unwrap(), Some([0x11; 32]));
    bridge.pin_app_seed(&[0x5e; 32]).unwrap();
    assert_eq!(bridge.pinned_app_seed().unwrap(), Some([0x5e; 32]));
    assert_eq!(
        std::fs::read_to_string(bridge.seed_file()).unwrap(),
        "5e".repeat(32)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(bridge.seed_file())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "only the owner may read the seed");
    }
}

#[test]
fn a_damaged_seed_file_is_an_error_not_a_fallback() {
    let (_dir, bridge) = bridge();
    std::fs::write(bridge.seed_file(), "not hex").unwrap();
    assert!(bridge.pinned_app_seed().unwrap_err().contains("not hex"));
    std::fs::write(bridge.seed_file(), "abcd").unwrap();
    assert!(bridge
        .pinned_app_seed()
        .unwrap_err()
        .contains("2 bytes, expected 32"));
}

#[test]
fn a_reset_request_is_taken_once() {
    let (_dir, bridge) = bridge();
    assert!(!bridge.take_reset());
    bridge.request_reset().unwrap();
    assert!(bridge.take_reset());
    assert!(!bridge.take_reset());
}

#[test]
fn clearing_the_queues_empties_both() {
    let (_dir, bridge) = bridge();
    bridge.submit(b"request").unwrap();
    bridge.write_response(b"answer").unwrap();
    bridge.clear_queues().unwrap();
    assert_eq!(bridge.take_request().unwrap(), None);
    assert_eq!(std::fs::read_dir(bridge.outbox()).unwrap().count(), 0);
}
