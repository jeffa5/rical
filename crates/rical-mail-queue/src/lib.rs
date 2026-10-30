//! Mail (rical): a send queue of files.
//!
//! Messages to send are files in a folder; a worker takes them one at a
//! time, hands each to a command that knows how to send it (msmtp, a
//! script, anything), and files a copy under Sent. See [`queue`] for the
//! layout and the guarantees, [`send`] for what the command is given, and
//! [`config`] for the settings.

pub mod config;
pub mod message;
pub mod queue;
pub mod send;
pub mod worker;
