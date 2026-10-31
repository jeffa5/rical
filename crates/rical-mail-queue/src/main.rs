//! `rical-mail-queue`: put messages in the send queue, look at it, and run the
//! worker that sends them.

use std::{io::Read as _, path::PathBuf, sync::atomic::AtomicBool};

use anyhow::{Context as _, Result};
use chrono::Utc;
use clap::Parser as _;
use rical_mail_queue::{
    config::{self, Config},
    queue::Queue,
    worker::Worker,
};

#[derive(Debug, clap::Parser)]
#[command(
    version,
    about = "A send queue of files, with the sending left to any command"
)]
struct Args {
    /// The settings file
    #[clap(long, env = "RICAL_MAIL_QUEUE_CONFIG", global = true)]
    config: Option<PathBuf>,

    /// The queue folder, over the settings file
    #[clap(long, env = "RICAL_MAIL_QUEUE_DIR", global = true)]
    queue: Option<PathBuf>,

    /// More logging: -v for what happens to each message, -vv for more
    #[clap(short, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[clap(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, clap::Subcommand)]
enum Cmd {
    /// Add a message (a file, or stdin) to the queue and print its id
    Enqueue { file: Option<PathBuf> },
    /// Show what is in the queue
    List,
    /// Send what is due, then keep going: watch the queue and send what
    /// arrives as it arrives (with --once, send what is due and stop)
    Run {
        #[clap(long)]
        once: bool,
    },
    /// Move failed messages back into the queue (one by id, or all)
    Retry { id: Option<String> },
}

fn load(args: &Args) -> Result<Config> {
    let mut config = match &args.config {
        Some(p) => Config::load(p)?,
        None => {
            let p = config::default_path();
            if p.exists() {
                Config::load(&p)?
            } else {
                Config::default()
            }
        }
    };
    if let Some(q) = &args.queue {
        config.queue = q.clone();
    }
    Ok(config)
}

fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(match args.verbose {
            0 => log::LevelFilter::Warn,
            1 => log::LevelFilter::Info,
            2 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        })
        .init();
    let config = load(&args)?;
    let queue = Queue::open(&config.queue)?;
    match args.cmd {
        Cmd::Enqueue { file } => {
            let mut raw = Vec::new();
            match file {
                Some(p) => raw = std::fs::read(&p).with_context(|| format!("{}", p.display()))?,
                None => {
                    std::io::stdin().read_to_end(&mut raw)?;
                }
            }
            println!("{}", queue.enqueue(&raw)?);
        }
        Cmd::List => {
            let entries = queue.list()?;
            if entries.is_empty() {
                println!("The queue is empty.");
            }
            for e in entries {
                let next = e
                    .next_attempt
                    .map(|t| format!(", next {}", t.format("%H:%M:%S")))
                    .unwrap_or_default();
                println!(
                    "{}  {}  {} → {}  {:?}",
                    e.id,
                    e.state.label(),
                    e.from,
                    e.to,
                    e.subject
                );
                if e.attempts > 0 {
                    println!(
                        "    {} attempt{}{next}: {}",
                        e.attempts,
                        if e.attempts == 1 { "" } else { "s" },
                        e.last_error.unwrap_or_default()
                    );
                }
            }
        }
        Cmd::Run { once } => {
            if config.send.is_none() && config.routes.is_empty() {
                // EX_CONFIG, so a service stops rather than retrying what
                // cannot start until the settings change
                eprintln!(
                    "Error: no send command: set `send = …` in {}",
                    args.config
                        .clone()
                        .unwrap_or_else(config::default_path)
                        .display()
                );
                std::process::exit(78);
            }
            let worker = Worker {
                queue: &queue,
                config: &config,
            };
            if once {
                let r = worker.run_once(Utc::now())?;
                println!(
                    "{} sent, {} to retry, {} failed",
                    r.sent, r.retrying, r.failed
                );
            } else {
                // no signal handling to pull in: the kernel drops our locks
                // when we are killed, which is the case the queue is built for
                worker.run(&AtomicBool::new(false))?;
            }
        }
        Cmd::Retry { id } => {
            let n = queue.retry(id.as_deref())?;
            println!(
                "{n} message{} back in the queue",
                if n == 1 { "" } else { "s" }
            );
        }
    }
    Ok(())
}
