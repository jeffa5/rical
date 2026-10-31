//! How long the work behind the message list takes, for a big folder:
//!
//! ```sh
//! scripts/demo-big.sh /tmp/big 0 0 50000
//! cargo run --release -p rical-maildir --example perf -- /tmp/big/mail/home/INBOX
//! ```

use std::time::Instant;

use rical_maildir::{search::Query, store, thread};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let folder = store::folders(dir.parent().unwrap())
        .unwrap()
        .into_iter()
        .find(|f| f.path == dir)
        .unwrap();
    let t = Instant::now();
    let messages = store::list(&folder);
    println!("list {} messages {:?}", messages.len(), t.elapsed());
    for _ in 0..2 {
        let t = Instant::now();
        let rows = thread::threads_sorted(&messages, thread::SortKey::Date, false);
        println!("threads {} rows {:?}", rows.len(), t.elapsed());
        let t = Instant::now();
        let rows = thread::sorted(&messages, thread::SortKey::Subject, false);
        println!("sorted by subject {} {:?}", rows.len(), t.elapsed());
        for q in ["p", "plan", "from:person unread", "thread 42"] {
            let t = Instant::now();
            let query = Query::parse(q);
            let n = messages.iter().filter(|m| query.matches(m)).count();
            println!("filter {q:?}: {n} in {:?}", t.elapsed());
        }
    }
}
