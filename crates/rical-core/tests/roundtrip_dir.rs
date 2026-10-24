//! Check that every `.ics` file under `RICAL_ROUNDTRIP_DIR` is reproduced
//! exactly by the editing model. Run against a real vdir with:
//!
//! RICAL_ROUNDTRIP_DIR=~/calendar cargo test -p rical-core --test roundtrip_dir -- --ignored

use std::{fs, path::Path};

use rical_core::doc::Document;

fn visit(dir: &Path, count: &mut usize, failures: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, count, failures);
        } else if path.extension().is_some_and(|e| e == "ics") {
            let text = fs::read_to_string(&path).unwrap();
            *count += 1;
            match Document::parse(&text) {
                Ok(doc) if doc.write() == text => {}
                Ok(_) => failures.push(format!("{path:?}: output differs")),
                Err(e) => failures.push(format!("{path:?}: {e}")),
            }
        }
    }
}

#[test]
#[ignore]
fn every_file_round_trips() {
    let dir = std::env::var("RICAL_ROUNDTRIP_DIR").expect("set RICAL_ROUNDTRIP_DIR");
    let mut count = 0;
    let mut failures = Vec::new();
    visit(Path::new(&dir), &mut count, &mut failures);
    println!("checked {count} files");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
