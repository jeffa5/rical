//! Handing one message to the command that knows how to send it.
//!
//! The command runs under `sh -c`, with the message on stdin and the
//! envelope in the environment:
//!
//! - `RICAL_MAIL_ID`: the queue id
//! - `RICAL_MAIL_FROM`: the sender's address
//! - `RICAL_MAIL_RECIPIENTS`: every recipient (To, Cc and Bcc), separated
//!   by spaces
//!
//! The `Bcc` header is not in what it reads, so a command may use
//! `-t`-style options without leaking it; it must take the recipients from
//! `RICAL_MAIL_RECIPIENTS` to reach those on Bcc.
//!
//! Exit 0 means sent. Other codes follow sysexits: a few mean it will
//! never work (bad usage, bad data, no permission, bad configuration), the
//! rest, and being killed, are worth retrying.

use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub struct Job<'a> {
    pub id: &'a str,
    pub command: &'a str,
    pub from: &'a str,
    pub recipients: &'a [String],
    pub message: &'a [u8],
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// Try again later.
    Temporary(String),
    /// Trying again will not help.
    Permanent(String),
}

/// Whether a sysexits code says retrying is pointless: usage, data, no
/// input, no user, no host, cannot create, no permission, configuration.
fn permanent(code: i32) -> bool {
    matches!(code, 64..=68 | 73 | 77 | 78)
}

/// The last few lines of what the command said, for the queue to show.
fn tail(text: &[u8]) -> String {
    let text = String::from_utf8_lossy(text);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

fn drain(from: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(r) = from {
            // a command that says more than this has said enough
            let _ = r.take(1 << 20).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

pub fn run(job: &Job) -> Outcome {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(job.command)
        .env("RICAL_MAIL_ID", job.id)
        .env("RICAL_MAIL_FROM", job.from)
        .env("RICAL_MAIL_RECIPIENTS", job.recipients.join(" "))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // its own group, so a timeout can take down what it started too
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Outcome::Temporary(format!("could not start sh: {e}")),
    };
    let pid = child.id() as i32;

    let stdin = child.stdin.take();
    let message = job.message.to_vec();
    thread::spawn(move || {
        if let Some(mut s) = stdin {
            // a command that stops reading early closes the pipe: its exit
            // status is what matters
            let _ = s.write_all(&message);
        }
    });
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() >= job.timeout => {
                // SAFETY: killpg on the group this function made
                unsafe { libc::killpg(pid, libc::SIGKILL) };
                let _ = child.wait();
                return Outcome::Temporary(format!(
                    "timed out after {}s (it may have been sent)",
                    job.timeout.as_secs()
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => return Outcome::Temporary(format!("waiting for the command: {e}")),
        }
    };
    if status.success() {
        return Outcome::Sent;
    }

    // something it started and left running can hold the pipes open
    let wait = Duration::from_millis(500);
    let said = {
        let err = tail(&stderr.recv_timeout(wait).unwrap_or_default());
        if err.is_empty() {
            tail(&stdout.recv_timeout(wait).unwrap_or_default())
        } else {
            err
        }
    };
    let (code, how) = match status.code() {
        Some(c) => (Some(c), format!("exit {c}")),
        None => (None, "killed by a signal".to_owned()),
    };
    let text = if said.is_empty() {
        how
    } else {
        format!("{how}: {said}")
    };
    match code {
        Some(c) if permanent(c) => Outcome::Permanent(text),
        _ => Outcome::Temporary(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_cmd(command: &str, timeout_ms: u64) -> Outcome {
        run(&Job {
            id: "id1",
            command,
            from: "a@x.example",
            recipients: &["b@y.example".into(), "c@z.example".into()],
            message: b"Subject: s\n\nhi\n",
            timeout: Duration::from_millis(timeout_ms),
        })
    }

    #[test]
    fn success() {
        assert_eq!(run_cmd("cat >/dev/null", 5000), Outcome::Sent);
    }

    #[test]
    fn envelope_and_message_reach_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let cmd = format!(
            "{{ echo \"$RICAL_MAIL_ID $RICAL_MAIL_FROM $RICAL_MAIL_RECIPIENTS\"; cat; }} > {}",
            out.display()
        );
        assert_eq!(run_cmd(&cmd, 5000), Outcome::Sent);
        assert_eq!(
            std::fs::read_to_string(out).unwrap(),
            "id1 a@x.example b@y.example c@z.example\nSubject: s\n\nhi\n"
        );
    }

    #[test]
    fn exit_codes_decide_retry() {
        assert_eq!(
            run_cmd("echo greylisted >&2; exit 75", 5000),
            Outcome::Temporary("exit 75: greylisted".into())
        );
        assert_eq!(
            run_cmd("echo bad password >&2; exit 77", 5000),
            Outcome::Permanent("exit 77: bad password".into())
        );
        assert_eq!(run_cmd("exit 1", 5000), Outcome::Temporary("exit 1".into()));
    }

    #[test]
    fn a_command_that_does_not_read_is_fine() {
        assert_eq!(run_cmd("true", 5000), Outcome::Sent);
    }

    #[test]
    fn timeout_kills_what_it_started() {
        let start = Instant::now();
        let out = run_cmd("sleep 30 & sleep 30", 200);
        assert!(matches!(out, Outcome::Temporary(m) if m.starts_with("timed out")));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
