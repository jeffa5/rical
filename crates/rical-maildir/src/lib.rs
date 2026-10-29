//! Maildir folders and mail messages: reading (with privacy: nothing is
//! ever fetched, and HTML is turned into text), flags, threads, searching,
//! and composing replies and new messages.

pub mod addr;
pub mod compose;
pub mod flags;
pub mod html;
pub mod invite;
pub mod markdown;
pub mod read;
pub mod search;
pub mod store;
pub mod text;
pub mod thread;
