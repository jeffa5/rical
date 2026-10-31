//! Settings: `name = value` lines in `~/.config/rical-mail-queue/config`.
//!
//! ```text
//! # where the queue lives; keep it out of any folder a mail sync touches
//! queue = ~/.local/share/rical-mail-queue/outbox
//! # a Maildir for copies of what was sent (default: sent/ in the queue)
//! sent = ~/mail/Sent
//! # the command that sends; see the send module for what it is given
//! send = msmtp --read-envelope-from -- $RICAL_MAIL_QUEUE_RECIPIENTS
//! # different commands by sender, the first that matches wins; * matches
//! # any run of characters
//! send-from *@work.example = msmtp -a work -- $RICAL_MAIL_QUEUE_RECIPIENTS
//! timeout = 60s
//! max-attempts = 8
//! retry-delay = 30s        # doubles each time, up to an hour
//! poll = 1m               # the longest it waits; arrivals wake it at once
//! ```
//!
//! There is no default `send`: until one is configured, nothing is sent.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub queue: PathBuf,
    pub sent: Option<PathBuf>,
    pub send: Option<String>,
    pub routes: Vec<(String, String)>,
    pub timeout: Duration,
    pub max_attempts: u32,
    pub retry_delay: Duration,
    pub poll: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            queue: data_home().join("rical-mail-queue/outbox"),
            sent: None,
            send: None,
            routes: Vec::new(),
            timeout: Duration::from_secs(60),
            max_attempts: 8,
            retry_delay: Duration::from_secs(30),
            poll: Duration::from_secs(60),
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
}

pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
        .join("rical-mail-queue/config")
}

fn expand(path: &str, base: &Path) -> PathBuf {
    let p = match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    };
    if p.is_relative() { base.join(p) } else { p }
}

/// "30", "30s", "5m", "2h".
pub fn duration(text: &str) -> Result<Duration> {
    let text = text.trim();
    let (n, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => text.split_at(i),
        None => (text, "s"),
    };
    let n: u64 = n
        .parse()
        .with_context(|| format!("not a length: {text:?}"))?;
    let secs = match unit.trim() {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => bail!("not a length: {text:?} (use 30s, 5m or 2h)"),
    };
    Ok(Duration::from_secs(secs))
}

/// Whether `pattern` matches `text`, ignoring case, with `*` as any run.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.to_lowercase(), text.to_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == t;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !t.starts_with(first) || t.len() < first.len() + last.len() || !t.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = t.len() - last.len();
    for mid in &parts[1..parts.len() - 1] {
        match t[at..end].find(mid) {
            Some(i) => at += i + mid.len(),
            None => return false,
        }
    }
    true
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Config::parse(&text, base).with_context(|| format!("{}", path.display()))
    }

    pub fn parse(text: &str, base: &Path) -> Result<Config> {
        let mut c = Config::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: anyhow::Error| e.context(format!("line {}", i + 1));
            let Some((name, value)) = line.split_once('=') else {
                return Err(at(anyhow::anyhow!("expected name = value")));
            };
            let (name, value) = (name.trim(), value.trim());
            // a command may contain a #, so only whole-line comments
            let (key, arg) = name.split_once(char::is_whitespace).unwrap_or((name, ""));
            let arg = arg.trim();
            (|| -> Result<()> {
                match key {
                    "queue" => c.queue = expand(value, base),
                    "sent" => c.sent = Some(expand(value, base)),
                    "send" => c.send = Some(value.to_owned()),
                    "send-from" => {
                        if arg.is_empty() {
                            bail!("send-from needs an address pattern: send-from *@x = command");
                        }
                        c.routes.push((arg.to_owned(), value.to_owned()));
                    }
                    "timeout" => c.timeout = duration(value)?,
                    "max-attempts" => c.max_attempts = value.parse().context("not a number")?,
                    "retry-delay" => c.retry_delay = duration(value)?,
                    "poll" => c.poll = duration(value)?,
                    _ => bail!("unknown setting {key:?}"),
                }
                Ok(())
            })()
            .map_err(at)?;
        }
        Ok(c)
    }

    /// The command to send a message from `from` with, if there is one.
    pub fn command_for(&self, from: &str) -> Option<&str> {
        self.routes
            .iter()
            .find(|(pattern, _)| glob(pattern, from))
            .map(|(_, cmd)| cmd.as_str())
            .or(self.send.as_deref())
    }

    /// Where copies of sent mail go.
    pub fn sent_dir(&self) -> PathBuf {
        self.sent.clone().unwrap_or_else(|| self.queue.join("sent"))
    }

    /// How long to wait after the `attempts`th failure.
    pub fn delay_after(&self, attempts: u32) -> Duration {
        let shift = attempts.saturating_sub(1).min(16);
        (self.retry_delay * 2u32.pow(shift)).min(Duration::from_secs(3600))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("*@work.example", "Me@Work.example"));
        assert!(!glob("*@work.example", "me@home.example"));
        assert!(glob("me@x.example", "ME@x.example"));
        assert!(glob("a*b*c", "a--b--c"));
        assert!(!glob("a*b*c", "a--c--b"));
        assert!(glob("*", "anything"));
    }

    #[test]
    fn lengths() {
        assert_eq!(duration("90").unwrap(), Duration::from_secs(90));
        assert_eq!(duration("5m").unwrap(), Duration::from_secs(300));
        assert!(duration("5 weeks").is_err());
    }

    #[test]
    fn routes_first_match_then_default() {
        let c = Config::parse(
            "send = default-cmd\n\
             send-from *@work.example = work-cmd # not a comment\n\
             send-from * = never\n",
            Path::new("/"),
        )
        .unwrap();
        assert_eq!(
            c.command_for("a@work.example"),
            Some("work-cmd # not a comment")
        );
        assert_eq!(c.command_for("a@home.example"), Some("never"));
        let none = Config::parse("", Path::new("/")).unwrap();
        assert_eq!(none.command_for("a@x.example"), None);
    }

    #[test]
    fn mistakes_name_the_line() {
        let e = Config::parse("poll = 5s\nbogus = 1\n", Path::new("/")).unwrap_err();
        assert!(format!("{e:#}").contains("line 2"), "{e:#}");
    }

    #[test]
    fn backoff_doubles_to_an_hour() {
        let c = Config::default();
        assert_eq!(c.delay_after(1), Duration::from_secs(30));
        assert_eq!(c.delay_after(3), Duration::from_secs(120));
        assert_eq!(c.delay_after(20), Duration::from_secs(3600));
    }
}
