use std::path::{Path, PathBuf};

use icalendar::Component as _;
use rical_core::{
    Rgb, Vdir,
    query::{self, ListKind, ListQuery},
    stats,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vdir")
}

fn query() -> ListQuery {
    ListQuery {
        paths: vec![fixtures()],
        recursive: true,
        limit: usize::MAX,
        from: None,
        to: None,
        kinds: vec![ListKind::All],
        short_id: None,
    }
}

fn summaries(listing: &query::Listing) -> Vec<&str> {
    listing
        .components
        .iter()
        .map(|c| c.get_summary().unwrap_or_default())
        .collect()
}

#[test]
fn vdir_calendars() {
    let vdir = Vdir::load(fixtures()).unwrap();

    let ids = vdir
        .calendars
        .iter()
        .map(|c| c.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        ["home", "work"],
        "hidden dirs are skipped, sorted by id"
    );

    let work = vdir.calendar("work").unwrap();
    assert_eq!(work.name, "Work");
    assert_eq!(
        work.color,
        Some(Rgb {
            r: 0xff,
            g: 0x88,
            b: 0x00
        })
    );

    let home = vdir.calendar("home").unwrap();
    assert_eq!(home.name, "home", "name falls back to id");
    assert_eq!(home.color, None, "invalid colour is ignored");
}

#[test]
fn vdir_root_without_subdirs_is_one_calendar() {
    let vdir = Vdir::load(fixtures().join("work")).unwrap();
    assert_eq!(vdir.calendars.len(), 1);
    assert_eq!(vdir.calendars[0].name, "Work");
}

#[test]
fn vdir_missing_root() {
    assert!(Vdir::load(fixtures().join("missing")).is_err());
}

#[test]
fn calendar_items_and_components() {
    let vdir = Vdir::load(fixtures()).unwrap();

    let work = vdir.calendar("work").unwrap();
    let names = work
        .item_paths()
        .unwrap()
        .into_iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["report.ics", "review.ics", "standup.ics"]);

    let components = work.components().unwrap();
    assert_eq!(components.len(), 3);
    let review = components
        .iter()
        .find(|c| c.get_uid() == Some("aab222-review"))
        .unwrap();
    assert_eq!(
        review.get_summary(),
        Some("Design review with a long summary that is folded"),
        "folded lines are joined"
    );
    assert_eq!(review.property_value("X-RICAL-TEST"), Some("kept"));
    assert_eq!(vdir.calendar_for_path(review.path()), Some(work));

    // broken.ics is skipped, notes.txt is not an item
    let home = vdir.calendar("home").unwrap();
    assert_eq!(home.item_paths().unwrap().len(), 2);
    assert_eq!(home.components().unwrap().len(), 1);
}

#[test]
fn list_all_sorted_by_start() {
    let listing = query::list(&query());
    assert_eq!(
        summaries(&listing),
        [
            // todo has no start, so sorts by CREATED
            "Write report",
            "Standup",
            "Design review with a long summary that is folded",
            "Dinner",
        ]
    );
}

#[test]
fn list_by_kind() {
    let listing = query::list(&ListQuery {
        kinds: vec![ListKind::Todo],
        ..query()
    });
    assert_eq!(summaries(&listing), ["Write report"]);
}

#[test]
fn list_by_range() {
    let listing = query::list(&ListQuery {
        from: Some("2025-03-04T00:00:00".parse().unwrap()),
        to: Some("2025-03-06T00:00:00".parse().unwrap()),
        kinds: vec![ListKind::Event],
        ..query()
    });
    assert_eq!(
        summaries(&listing),
        ["Design review with a long summary that is folded", "Dinner"]
    );
}

#[test]
fn list_by_short_id() {
    let listing = query::list(&ListQuery {
        short_id: Some("aab".to_owned()),
        ..query()
    });
    assert_eq!(
        summaries(&listing),
        ["Design review with a long summary that is folded"]
    );
}

#[test]
fn short_ids_cover_all_matches() {
    let listing = query::list(&ListQuery {
        limit: 1,
        ..query()
    });
    assert_eq!(listing.components.len(), 1);

    let short_ids = listing.short_ids();
    assert_eq!(short_ids.len(), 4);
    assert_eq!(short_ids["aaa111-standup"], "aaa");
    assert_eq!(short_ids["aab222-review"], "aab");
    assert_eq!(short_ids["b44-report"], "b");
    assert_eq!(short_ids["c33-dinner"], "c");
}

#[test]
fn single_component_short_id() {
    let listing = query::list(&ListQuery {
        paths: vec![fixtures().join("home")],
        ..query()
    });
    assert_eq!(listing.short_ids()["c33-dinner"], "c");
}

#[test]
fn stats_counts() {
    let s = stats::stats(&[fixtures()], true);
    assert_eq!(s.total, 4);
    assert_eq!(s.by_kind["VEVENT"], 3);
    assert_eq!(s.by_kind["VTODO"], 1);

    // not recursive: the root has no items of its own
    assert_eq!(stats::stats(&[fixtures()], false).total, 0);
}
