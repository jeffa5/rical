//! The queue and worker against fake send commands that only write into a
//! temporary folder: nothing here can reach a mail server.

use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use chrono::{DateTime, Utc};
use rical_mail::{
    config::Config,
    message,
    queue::{self, Queue, State},
    worker::{Report, Worker},
};

const MAIL: &[u8] =
    b"From: Me <me@x.example>\nTo: you@y.example\nBcc: secret@z.example\nSubject: Hello\n\nBody\n";

/// A process that forks (to run a send command) while another thread holds a
/// lock gives the child a copy of it, which lasts until the child execs. A
/// worker just finds the message on its next look; these tests expect to
/// find it at once, so they take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Fixture {
    _turn: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    queue: Queue,
    config: Config,
}

impl Fixture {
    fn new(send: &str) -> Fixture {
        let turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            queue: dir.path().join("outbox"),
            sent: Some(dir.path().join("Sent")),
            send: Some(send.replace("OUT", &dir.path().join("out").display().to_string())),
            timeout: Duration::from_secs(5),
            max_attempts: 3,
            ..Config::default()
        };
        let queue = Queue::open(&config.queue).unwrap();
        Fixture {
            _turn: turn,
            dir,
            queue,
            config,
        }
    }

    fn run(&self, now: DateTime<Utc>) -> Report {
        Worker {
            queue: &self.queue,
            config: &self.config,
        }
        .run_once(now)
        .unwrap()
    }

    fn out(&self) -> String {
        fs::read_to_string(self.dir.path().join("out")).unwrap_or_default()
    }
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn sends_files_a_copy_and_leaves_the_queue_empty() {
    let f = Fixture::new("{ echo \"$RICAL_MAIL_RECIPIENTS\"; cat; } > OUT");
    let id = f.queue.enqueue(MAIL).unwrap();

    let r = f.run(Utc::now());
    assert_eq!((r.sent, r.retrying, r.failed), (1, 0, 0));

    // the command saw every recipient, and no Bcc header
    let out = f.out();
    assert!(out.starts_with("you@y.example secret@z.example\n"), "{out}");
    assert!(!out.contains("Bcc"), "{out}");
    assert!(out.contains("Message-ID: <"), "{out}");
    assert!(out.contains("\nBody\n"));

    assert!(f.queue.list().unwrap().is_empty());
    // the copy keeps Bcc and says when it went
    let sent = files(&f.dir.path().join("Sent/cur"));
    assert_eq!(sent, [format!("{id}.rical:2,S")]);
    let copy = fs::read(f.dir.path().join("Sent/cur").join(&sent[0])).unwrap();
    assert_eq!(message::header(&copy, "Bcc").unwrap(), "secret@z.example");
    assert!(message::header(&copy, queue::SENT_AT).is_some());
}

#[test]
fn enqueue_refuses_what_could_not_be_sent() {
    let f = Fixture::new("true");
    assert!(f.queue.enqueue(b"To: a@x.example\n\nhi\n").is_err());
    assert!(f.queue.enqueue(b"From: a@x.example\n\nhi\n").is_err());
    assert!(f.queue.list().unwrap().is_empty());
}

#[test]
fn a_temporary_failure_waits_then_succeeds_and_shows_why() {
    // fails until the marker file exists
    let f = Fixture::new("test -e OUT.ok || { echo 'server busy' >&2; exit 75; }; cat > OUT");
    f.queue.enqueue(MAIL).unwrap();
    let t0 = Utc::now();

    let r = f.run(t0);
    assert_eq!((r.sent, r.retrying, r.failed), (0, 1, 0));
    let e = &f.queue.list().unwrap()[0];
    assert_eq!(e.state, State::Retrying);
    assert_eq!(e.attempts, 1);
    assert_eq!(e.last_error.as_deref(), Some("exit 75: server busy"));
    assert!(e.next_attempt.unwrap() > t0);

    // not due yet: nothing is tried
    assert_eq!(f.run(t0 + chrono::Duration::seconds(1)), Report::default());

    fs::write(f.dir.path().join("out.ok"), "").unwrap();
    let r = f.run(t0 + chrono::Duration::minutes(5));
    assert_eq!(r.sent, 1);
    assert!(f.queue.list().unwrap().is_empty());
    // the error and the timetable do not follow it to Sent
    let sent = files(&f.dir.path().join("Sent/cur"));
    let copy = fs::read(f.dir.path().join("Sent/cur").join(&sent[0])).unwrap();
    assert!(message::header(&copy, queue::LAST_ERROR).is_none());
    assert_eq!(message::header(&copy, queue::ATTEMPTS).unwrap(), "1");
}

#[test]
fn it_gives_up_after_max_attempts_and_retry_puts_it_back() {
    let f = Fixture::new("exit 75");
    let id = f.queue.enqueue(MAIL).unwrap();
    let mut now = Utc::now();
    for _ in 0..2 {
        let r = f.run(now);
        assert_eq!(r.retrying, 1, "{r:?} {:?}", f.queue.list());
        now += chrono::Duration::hours(2);
    }
    assert_eq!(f.run(now).failed, 1);
    let e = &f.queue.list().unwrap()[0];
    assert_eq!(e.state, State::Failed);
    assert!(
        e.last_error
            .as_deref()
            .unwrap()
            .starts_with("gave up after 3 attempts")
    );
    assert_eq!(f.run(now + chrono::Duration::days(1)), Report::default());

    assert_eq!(f.queue.retry(Some(&id)).unwrap(), 1);
    let e = &f.queue.list().unwrap()[0];
    assert_eq!((e.state, e.attempts), (State::Queued, 0));
    assert!(f.queue.retry(Some("nope")).is_err());
}

#[test]
fn a_permanent_failure_is_not_retried() {
    let f = Fixture::new("echo 'bad password' >&2; exit 77");
    f.queue.enqueue(MAIL).unwrap();
    assert_eq!(f.run(Utc::now()).failed, 1);
    let e = &f.queue.list().unwrap()[0];
    assert_eq!((e.state, e.attempts), (State::Failed, 1));
    assert_eq!(e.last_error.as_deref(), Some("exit 77: bad password"));
}

#[test]
fn the_sender_picks_the_command() {
    let mut f = Fixture::new("echo default > OUT");
    f.config.routes = vec![(
        "*@x.example".into(),
        f.config.send.clone().unwrap().replace("default", "routed"),
    )];
    f.queue.enqueue(MAIL).unwrap();
    f.queue
        .enqueue(b"From: other@elsewhere.example\nTo: you@y.example\n\nhi\n")
        .unwrap();
    assert_eq!(f.run(Utc::now()).sent, 2);
    // the second one ran last, with the default command
    assert_eq!(f.out(), "default\n");
}

#[test]
fn no_command_for_a_sender_fails_rather_than_going_another_way() {
    let mut g = Fixture::new("true");
    g.config.send = None;
    g.config.routes = vec![("*@nowhere.example".into(), "true".into())];
    g.queue.enqueue(MAIL).unwrap();
    assert_eq!(g.run(Utc::now()).failed, 1);
    assert_eq!(
        g.queue.list().unwrap()[0].last_error.as_deref(),
        Some("no send command for me@x.example")
    );
}

#[test]
fn a_dead_workers_message_is_put_back_but_a_live_ones_is_left() {
    let f = Fixture::new("cat > OUT");
    let id = f.queue.enqueue(MAIL).unwrap();
    let now = Utc::now();

    // a live worker: holds the claim
    let claim = f.queue.claim_next(now).unwrap().unwrap();
    assert_eq!(f.queue.list().unwrap()[0].state, State::Sending);
    assert_eq!(f.queue.recover_stale(now).unwrap(), 0);
    // and a second worker cannot take it
    assert!(f.queue.claim_next(now).unwrap().is_none());

    // a dead one: the lock goes with the process (here, the claim)
    drop(claim);
    let n = f.queue.recover_stale(now).unwrap();
    assert_eq!(n, 1, "{:?}", f.queue.list());
    let e = &f.queue.list().unwrap()[0];
    assert_eq!(
        (e.id.as_str(), e.state, e.attempts),
        (id.as_str(), State::Retrying, 1)
    );
    assert_eq!(f.run(now).sent, 1);
}

#[test]
fn two_workers_never_send_the_same_message() {
    let f = Fixture::new("echo sent >> OUT");
    for _ in 0..30 {
        f.queue.enqueue(MAIL).unwrap();
    }
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| f.run(Utc::now()));
        }
    });
    assert_eq!(f.out().lines().count(), 30);
    assert_eq!(files(&f.dir.path().join("Sent/cur")).len(), 30);
    assert!(f.queue.list().unwrap().is_empty());
}
