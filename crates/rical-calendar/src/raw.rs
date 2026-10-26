//! The raw view: an item's `.ics` file exactly as it is on disk, for
//! debugging, with the selected component highlighted.

use adw::prelude::*;
use rical_core::Occurrence;

/// Line ranges (start, end inclusive) of the components with this UID.
fn component_lines(text: &str, uid: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut matched = false;
    for (i, line) in text.lines().enumerate() {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("BEGIN:V") && !upper.starts_with("BEGIN:VCALENDAR") && start.is_none()
        {
            // the top-level component; nested ones (VALARM) stay inside it
            if !upper.starts_with("BEGIN:VTIMEZONE") {
                start = Some(i);
                matched = false;
            }
        } else if let Some(s) = start {
            if upper.starts_with("UID:") && line[4..].trim() == uid {
                matched = true;
            }
            let ends_top = upper.starts_with("END:V")
                && !upper.starts_with("END:VALARM")
                && !upper.starts_with("END:VCALENDAR");
            if ends_top {
                if matched {
                    out.push((s, i));
                }
                start = None;
            }
        }
    }
    out
}

pub fn dialog(occ: &Occurrence) -> adw::Dialog {
    let path = occ.path.clone();
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| format!("could not read the file: {e}"));

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&text);
    let mark = buffer.create_tag(Some("this"), &[("background", &"rgba(53,132,228,0.18)")]);
    let ranges = component_lines(&text, &occ.uid);
    if mark.is_some() {
        for &(s, e) in &ranges {
            let (Some(start), Some(mut end)) =
                (buffer.iter_at_line(s as i32), buffer.iter_at_line(e as i32))
            else {
                continue;
            };
            end.forward_to_line_end();
            buffer.apply_tag_by_name("this", &start, &end);
        }
    }
    let view = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::None)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .build();
    view.update_property(&[gtk::accessible::Property::Label("File contents")]);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .build();
    // start at the highlighted component
    if let Some(&(s, _)) = ranges.first()
        && let Some(mut it) = buffer.iter_at_line(s as i32)
    {
        let v = view.clone();
        gtk::glib::idle_add_local_once(move || {
            v.scroll_to_iter(&mut it, 0.0, true, 0.0, 0.1);
        });
    }

    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy the file's text")
        .build();
    let t = text.clone();
    copy.connect_clicked(move |b| b.clipboard().set_text(&t));
    let folder = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Show in its folder")
        .build();
    let p = path.clone();
    folder.connect_clicked(move |b| {
        let window = b.root().and_downcast::<gtk::Window>();
        gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(&p))).open_containing_folder(
            window.as_ref(),
            None::<&gtk::gio::Cancellable>,
            |r| {
                if let Err(e) = r {
                    log::warn!("could not open the folder: {e}");
                }
            },
        );
    });

    let title = adw::WindowTitle::new(
        &path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        &path
            .parent()
            .map_or_else(String::new, |p| p.display().to_string()),
    );
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    header.pack_start(&copy);
    header.pack_start(&folder);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));
    adw::Dialog::builder()
        .title("Raw file")
        .content_width(760)
        .content_height(640)
        .child(&toolbar)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_components_of_a_uid() {
        let text = "BEGIN:VCALENDAR\r\n\
            BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nEND:VTIMEZONE\r\n\
            BEGIN:VEVENT\r\nUID:a\r\nBEGIN:VALARM\r\nEND:VALARM\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:b\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:a\r\nRECURRENCE-ID:20260101T090000Z\r\nEND:VEVENT\r\n\
            END:VCALENDAR\r\n";
        assert_eq!(component_lines(text, "a"), [(4, 8), (12, 15)]);
        assert_eq!(component_lines(text, "b"), [(9, 11)]);
        assert!(component_lines(text, "c").is_empty());
    }
}
