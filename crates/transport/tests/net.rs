//! The simulator network link: a phone's request reaches the app, waits for its answer on the
//! same connection, and nothing oversized gets through.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

use fps_protocol::MAX_MESSAGE_SIZE;
use fps_transport::net::{
    exchange, read_frame, resolve, write_frame, Connection, Listener, DEFAULT_PORT,
};

/// Waits for the listener to hold a request.
fn taken(listener: &Listener) -> (Connection, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(request) = listener.take_request() {
            return request;
        }
        assert!(Instant::now() < deadline, "no request arrived");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn frames_round_trip() {
    let mut buffer = Vec::new();
    write_frame(&mut buffer, b"hello").unwrap();
    assert_eq!(&buffer[..4], &5u32.to_be_bytes());
    assert_eq!(read_frame(&mut &buffer[..]).unwrap(), b"hello");
    write_frame(&mut buffer, b"").unwrap();
    let mut reader = &buffer[..];
    assert_eq!(read_frame(&mut reader).unwrap(), b"hello");
    assert_eq!(read_frame(&mut reader).unwrap(), b"");
}

#[test]
fn oversized_frames_are_refused_before_they_are_read() {
    let mut buffer = Vec::new();
    buffer.extend((MAX_MESSAGE_SIZE as u32 + 1).to_be_bytes());
    let error = read_frame(&mut &buffer[..]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let error = write_frame(&mut Vec::new(), &vec![0; MAX_MESSAGE_SIZE + 1]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_request_is_answered_on_its_connection_once_someone_decides() {
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().to_string();
    let phone = thread::spawn(move || exchange(&address, b"request one", Duration::from_secs(10)));

    let (connection, request) = taken(&listener);
    assert_eq!(request, b"request one");
    assert!(connection.peer().ip().is_loopback());
    // The review takes a moment.
    thread::sleep(Duration::from_millis(300));
    connection.answer(b"answer one").unwrap();
    assert_eq!(phone.join().unwrap().unwrap(), b"answer one");
}

#[test]
fn requests_wait_in_order_while_one_is_on_screen() {
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().to_string();
    let first = {
        let address = address.clone();
        thread::spawn(move || exchange(&address, b"first", Duration::from_secs(10)))
    };
    let (on_screen, request) = taken(&listener);
    assert_eq!(request, b"first");
    let second = thread::spawn(move || exchange(&address, b"second", Duration::from_secs(10)));
    // The second waits in the queue until the app takes it, after the first is answered.
    let deadline = Instant::now() + Duration::from_secs(5);
    while listener.waiting() == 0 {
        assert!(
            Instant::now() < deadline,
            "the second request never arrived"
        );
        thread::sleep(Duration::from_millis(10));
    }
    on_screen.answer(b"one").unwrap();
    assert_eq!(first.join().unwrap().unwrap(), b"one");
    let (connection, request) = taken(&listener);
    assert_eq!(request, b"second");
    connection.answer(b"two").unwrap();
    assert_eq!(second.join().unwrap().unwrap(), b"two");
}

#[test]
fn the_phone_gives_up_when_no_answer_comes() {
    let listener = Listener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().to_string();
    let error = exchange(&address, b"unanswered", Duration::from_millis(200)).unwrap_err();
    assert!(
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ),
        "{error:?}"
    );
    // The request still reached the app; the answer just has no one to go to.
    let (connection, request) = taken(&listener);
    assert_eq!(request, b"unanswered");
    let _ = connection.answer(b"late");
}

#[test]
fn nobody_listening_is_an_error_not_a_wait() {
    // Port 1 is reserved and never listening. Guessing a closed port near a live one is not
    // safe while the other tests here hold listeners of their own.
    let started = Instant::now();
    let error = exchange("127.0.0.1:1", b"x", Duration::from_secs(10)).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(6), "{error}");
}

#[test]
fn a_bare_host_gets_the_default_port() {
    assert_eq!(resolve("127.0.0.1").unwrap().port(), DEFAULT_PORT);
    assert_eq!(resolve(" 127.0.0.1:9000 ").unwrap().port(), 9000);
    assert!(resolve("").is_err());
}
