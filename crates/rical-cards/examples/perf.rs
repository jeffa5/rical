//! How rical-cards does with many contacts: make a synthetic address book,
//! then time loading, indexing, searching and finding duplicates.
//!
//! ```sh
//! cargo run --release -p rical-cards --example perf -- gen /tmp/cards 50000
//! cargo run --release -p rical-cards --example perf -- run /tmp/cards
//! ```

use std::{path::Path, time::Instant};

use rical_cards::{Books, dedupe, search::Index};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

fn generate(dir: &Path, n: usize) {
    let book = dir.join("contacts");
    std::fs::create_dir_all(&book).unwrap();
    let given = [
        "Sam", "Priya", "Jo", "Alex", "Zoë", "Mateo", "Aisha", "Wei", "Olga", "Kofi", "Liam",
        "Sofía", "Hiro", "Fatima", "Noah", "Ines", "Ravi", "Emma", "Tomás", "Yuki",
    ];
    let family = [
        "Lee", "Shah", "Park", "Morgan", "Ørsted", "García", "Khan", "Chen", "Ivanova", "Mensah",
        "Murphy", "López", "Tanaka", "Ali", "Smith", "Silva", "Patel", "Brown", "O'Brien",
        "Nguyen",
    ];
    let orgs = ["Northwind", "Contoso", "Fabrikam", "Initech", "Globex", ""];
    let towns = ["Leeds", "Lisbon", "Osaka", "Accra", "Lyon", "Austin"];
    let mut rng = Rng(42);
    for i in 0..n {
        let (g, f) = (rng.pick(&given), rng.pick(&family));
        let email = format!(
            "{}.{}{}@example.com",
            g.to_lowercase(),
            f.to_lowercase().replace('\'', ""),
            i
        );
        let phone = format!("+44 7700 {:06}", rng.next() % 1_000_000);
        // one card in twenty is a copy of an earlier one, as sprawl goes
        let (email, phone) = if i > 20 && i % 20 == 0 {
            (
                format!(
                    "{}.{}{}@example.com",
                    g.to_lowercase(),
                    f.to_lowercase().replace('\'', ""),
                    i - 7
                ),
                phone,
            )
        } else {
            (email, phone)
        };
        let text = format!(
            "BEGIN:VCARD\r\nVERSION:{}\r\nUID:card-{i}\r\nFN:{g} {f}\r\nN:{f};{g};;;\r\n\
             EMAIL;TYPE=work:{email}\r\nTEL;TYPE=cell:{phone}\r\nORG:{}\r\n\
             ADR;TYPE=home:;;{} High Street;{};;;\r\nNOTE:Met through {} in {}\r\n\
             CATEGORIES:{}\r\nEND:VCARD\r\n",
            if i % 3 == 0 { "4.0" } else { "3.0" },
            rng.pick(&orgs),
            rng.next() % 200,
            rng.pick(&towns),
            rng.pick(&given),
            2000 + rng.next() % 26,
            rng.pick(&["Friends", "Work", "Family", "Climbing"]),
        );
        std::fs::write(book.join(format!("card-{i}.vcf")), text).unwrap();
    }
}

fn memory_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1024.0
}

fn run(dir: &Path) {
    let before = memory_mb();
    let t = Instant::now();
    let books = Books::load(dir).unwrap();
    println!("cards        {}", books.len());
    println!("load         {:>8.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let index = Index::build(&books);
    println!("index        {:>8.1} ms", t.elapsed().as_secs_f64() * 1e3);
    println!(
        "memory       {:>8.1} MB (index about {:.1} MB)",
        memory_mb() - before,
        index.size() as f64 / 1e6
    );
    for q in [
        "sam",
        "zoe orsted",
        "lisbon",
        "7700 12",
        "priya@",
        "o'brien",
        "climbing",
        "xyzzy",
        "",
    ] {
        let t = Instant::now();
        let n = (0..20).map(|_| index.search(q).len()).next_back().unwrap();
        println!(
            "search {:<12} {:>7.3} ms ({n} found)",
            format!("{q:?}"),
            t.elapsed().as_secs_f64() * 1e3 / 20.0
        );
    }
    let t = Instant::now();
    let groups = dedupe::find(&books);
    println!(
        "duplicates   {:>8.1} ms ({} groups, {} strong)",
        t.elapsed().as_secs_f64() * 1e3,
        groups.len(),
        groups.iter().filter(|g| g.is_strong()).count()
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => generate(
            Path::new(&args[2]),
            args.get(3).and_then(|n| n.parse().ok()).unwrap_or(50_000),
        ),
        Some("run") => run(Path::new(&args[2])),
        _ => eprintln!("perf gen DIR [N] | perf run DIR"),
    }
}
