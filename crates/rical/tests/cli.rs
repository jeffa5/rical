use std::{path::PathBuf, process::Command};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rical-core/tests/fixtures/vdir")
}

fn rical(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rical"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn lines(s: &str) -> Vec<&str> {
    s.lines().map(str::trim_end).collect()
}

#[test]
fn list_table() {
    let out = rical(&["list", fixtures().to_str().unwrap(), "--recursive"]);
    assert_eq!(
        lines(&out),
        [
            " short_id  kind   start                    end                      duration  status        summary                                           attendees",
            " b         todo                                                               NEEDS-ACTION  Write report",
            " aaa       event  2025-03-03 09:00:00 UTC  2025-03-03 09:15:00 UTC  15m       CONFIRMED     Standup",
            " aab       event  2025-03-04 14:00:00 UTC  2025-03-04 15:30:00 UTC  1h30m     TENTATIVE     Design review with a long summary that is folded",
            " c         event  2025-03-05 19:00:00 UTC  2025-03-05 21:00:00 UTC  2h        CONFIRMED     Dinner",
        ]
    );
}

#[test]
fn list_json() {
    let out = rical(&[
        "list",
        fixtures().to_str().unwrap(),
        "--recursive",
        "--kinds",
        "todo",
        "-o",
        "json",
    ]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["kind"], "todo");
    assert_eq!(v[0]["short_id"], "b");
    assert_eq!(v[0]["properties"]["summary"], "Write report");
}

#[test]
fn stats_table() {
    let out = rical(&["stats", fixtures().to_str().unwrap(), "--recursive"]);
    assert_eq!(lines(&out), [" total  VEVENT  VTODO", " 4      3       1"]);
}

#[test]
fn show_by_short_id() {
    let out = rical(&[
        "show",
        fixtures().to_str().unwrap(),
        "--recursive",
        "--id",
        "c",
    ]);
    let rows = lines(&out);
    assert_eq!(rows.len(), 2);
    assert!(rows[1].contains("c33-dinner"));
    assert!(rows[1].contains("home/dinner.ics"));
}

#[test]
fn agenda_table() {
    let out = Command::new(env!("CARGO_BIN_EXE_rical"))
        .args([
            "agenda",
            fixtures().to_str().unwrap(),
            "--from",
            "2025-03-03",
            "--days",
            "3",
        ])
        .env("TZ", "Europe/London")
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        lines(&out),
        [
            " start             end               calendar  summary                                           location",
            " 2025-03-03 09:00  2025-03-03 09:15  Work      Standup",
            " 2025-03-04 14:00  2025-03-04 15:30  Work      Design review with a long summary that is folded",
            " 2025-03-05 19:00  2025-03-05 21:00  home      Dinner",
        ]
    );
}
