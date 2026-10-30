//! Messages in conversations (by Message-ID, In-Reply-To and References, and
//! for replies that lost them, by subject), and how a list is ordered.

use std::collections::HashMap;

use crate::store::Summary;

/// One line of a conversation view: a message and how deep in its replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub depth: usize,
    /// First of its conversation.
    pub head: bool,
}

/// What a list is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Newest first (a conversation by its newest message).
    #[default]
    Date,
    /// By who it is from, A to Z.
    From,
    /// By subject, A to Z (without Re: and Fwd:).
    Subject,
    /// Biggest first.
    Size,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<SortKey> {
        match s.trim().to_lowercase().as_str() {
            "date" | "time" | "newest" => Some(SortKey::Date),
            "from" | "sender" | "who" => Some(SortKey::From),
            "subject" | "title" => Some(SortKey::Subject),
            "size" | "biggest" => Some(SortKey::Size),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SortKey::Date => "date",
            SortKey::From => "from",
            SortKey::Subject => "subject",
            SortKey::Size => "size",
        }
    }
}

/// A subject without the Re:, Fwd: and the like in front, lower case.
pub fn base_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let cut = ["re", "fwd", "fw", "aw", "sv", "tr", "wg", "vs"]
            .iter()
            .find_map(|p| {
                let rest = lower.strip_prefix(p)?;
                // "re:" and "re[2]:"
                let rest = match rest.strip_prefix('[') {
                    Some(r) => r.split_once(']')?.1,
                    None => rest,
                };
                rest.strip_prefix(':').map(|r| lower.len() - r.len())
            });
        match cut {
            Some(n) => s = s[n..].trim_start(),
            None => break,
        }
    }
    s.to_lowercase()
}

fn has_reply_prefix(subject: &str) -> bool {
    base_subject(subject) != subject.trim().to_lowercase()
}

/// Messages in conversations, the newest conversation first.
pub fn threads(messages: &[Summary]) -> Vec<Row> {
    threads_sorted(messages, SortKey::Date, false)
}

/// The messages in conversations, each in the order of the replies (oldest
/// first, replies below what they answer), the conversations ordered by
/// `key` (or the other way with `reverse`).
pub fn threads_sorted(messages: &[Summary], key: SortKey, reverse: bool) -> Vec<Row> {
    let by_id: HashMap<&str, usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| Some((m.message_id.as_deref()?, i)))
        .collect();
    // the parent: what it answers, else the latest earlier thing it refers to
    let parent = |i: usize| -> Option<usize> {
        let m = &messages[i];
        m.in_reply_to
            .iter()
            .rev()
            .chain(m.references.iter().rev())
            .find_map(|id| by_id.get(id.as_str()).copied())
            .filter(|&p| p != i)
    };
    let mut parents: Vec<Option<usize>> = (0..messages.len()).map(parent).collect();

    // a reply that lost its references still belongs under what it answers:
    // the latest earlier message with the same subject, within six weeks
    let mut by_subject: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        by_subject
            .entry(base_subject(&m.subject))
            .or_default()
            .push(i);
    }
    for i in 0..messages.len() {
        let m = &messages[i];
        if parents[i].is_some()
            || !has_reply_prefix(&m.subject)
            || base_subject(&m.subject).is_empty()
        {
            continue;
        }
        parents[i] = by_subject
            .get(&base_subject(&m.subject))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&j| {
                j != i
                    && messages[j].date <= m.date
                    && m.date - messages[j].date < 42 * 86_400
                    // not something that is itself a reply to this one
                    && !(messages[j].in_reply_to.iter().chain(&messages[j].references))
                        .any(|id| Some(id.as_str()) == m.message_id.as_deref())
            })
            .max_by_key(|&j| (messages[j].date, j));
    }

    // no loops: a parent chain that returns to itself is cut
    for i in 0..messages.len() {
        let mut seen = vec![i];
        let mut p = parents[i];
        while let Some(x) = p {
            if seen.contains(&x) {
                parents[i] = None;
                break;
            }
            seen.push(x);
            p = parents[x];
        }
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); messages.len()];
    let mut roots = Vec::new();
    for (i, p) in parents.iter().enumerate() {
        match p {
            Some(p) => children[*p].push(i),
            None => roots.push(i),
        }
    }
    for c in &mut children {
        c.sort_by_key(|&i| (messages[i].date, i));
    }
    // the newest message anywhere in each tree, and the size of all of it
    fn newest(i: usize, messages: &[Summary], children: &[Vec<usize>]) -> i64 {
        children[i]
            .iter()
            .map(|&c| newest(c, messages, children))
            .max()
            .unwrap_or(i64::MIN)
            .max(messages[i].date)
    }
    fn total(i: usize, messages: &[Summary], children: &[Vec<usize>]) -> u64 {
        messages[i].size
            + children[i]
                .iter()
                .map(|&c| total(c, messages, children))
                .sum::<u64>()
    }
    let mut roots: Vec<usize> = roots;
    roots.sort_by(|&a, &b| {
        let ord = match key {
            SortKey::Date => newest(b, messages, &children).cmp(&newest(a, messages, &children)),
            SortKey::From => from_key(&messages[a]).cmp(&from_key(&messages[b])),
            SortKey::Subject => {
                base_subject(&messages[a].subject).cmp(&base_subject(&messages[b].subject))
            }
            SortKey::Size => total(b, messages, &children).cmp(&total(a, messages, &children)),
        };
        ord.then(a.cmp(&b))
    });
    if reverse {
        roots.reverse();
    }

    let mut out = Vec::with_capacity(messages.len());
    fn emit(i: usize, depth: usize, head: bool, children: &[Vec<usize>], out: &mut Vec<Row>) {
        out.push(Row {
            index: i,
            depth,
            head,
        });
        for &c in &children[i] {
            emit(c, depth + 1, false, children, out);
        }
    }
    for r in roots {
        emit(r, 0, true, &children, &mut out);
    }
    out
}

fn from_key(m: &Summary) -> String {
    m.from_shown().to_lowercase()
}

/// The messages one by one, ordered by `key`.
pub fn sorted(messages: &[Summary], key: SortKey, reverse: bool) -> Vec<Row> {
    let mut idx: Vec<usize> = (0..messages.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&messages[a], &messages[b]);
        let ord = match key {
            SortKey::Date => y.date.cmp(&x.date),
            SortKey::From => from_key(x).cmp(&from_key(y)),
            SortKey::Subject => base_subject(&x.subject).cmp(&base_subject(&y.subject)),
            SortKey::Size => y.size.cmp(&x.size),
        };
        ord.then(a.cmp(&b))
    });
    if reverse {
        idx.reverse();
    }
    idx.into_iter()
        .map(|index| Row {
            index,
            depth: 0,
            head: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;

    fn m(id: &str, reply_to: &[&str], date: i64) -> Summary {
        Summary {
            path: id.into(),
            folder: String::new(),
            flags: Flags::default(),
            is_new: false,
            date,
            from_name: String::new(),
            from_address: String::new(),
            to: String::new(),
            subject: id.into(),
            message_id: Some(id.into()),
            in_reply_to: reply_to.iter().map(|s| s.to_string()).collect(),
            references: Vec::new(),
            size: 0,
            attachments: false,
            preview: String::new(),
        }
    }

    #[test]
    fn replies_sit_under_what_they_answer_and_new_conversations_come_first() {
        let msgs = vec![
            m("a", &[], 10),
            m("b", &["a"], 20),
            m("c", &["a"], 15),
            m("d", &["b"], 30),
            m("x", &[], 25),
        ];
        let rows = threads(&msgs);
        let shape: Vec<(&str, usize)> = rows
            .iter()
            .map(|r| (msgs[r.index].subject.as_str(), r.depth))
            .collect();
        // the conversation a-b-c-d was last written at 30, x at 25
        assert_eq!(shape, [("a", 0), ("c", 1), ("b", 1), ("d", 2), ("x", 0)]);
        assert!(rows[0].head && !rows[1].head && rows[4].head);
    }

    #[test]
    fn loops_and_missing_parents_do_not_hang() {
        let msgs = vec![m("a", &["b"], 1), m("b", &["a"], 2), m("c", &["gone"], 3)];
        assert_eq!(threads(&msgs).len(), 3);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::flags::Flags;

    fn msg(id: &str, subject: &str, from: &str, date: i64, size: u64) -> Summary {
        Summary {
            path: id.into(),
            folder: String::new(),
            flags: Flags::default(),
            is_new: false,
            date,
            from_name: from.into(),
            from_address: String::new(),
            to: String::new(),
            subject: subject.into(),
            message_id: Some(id.into()),
            in_reply_to: vec![],
            references: vec![],
            size,
            attachments: false,
            preview: String::new(),
        }
    }

    #[test]
    fn subjects_lose_their_prefixes() {
        assert_eq!(base_subject("Re: RE[2]: Fwd: Q4 plan"), "q4 plan");
        assert_eq!(base_subject("Retirement party"), "retirement party");
        assert_eq!(base_subject("AW: Frage"), "frage");
    }

    #[test]
    fn a_reply_that_lost_its_references_joins_by_subject() {
        let day = 86_400;
        let msgs = vec![
            msg("a", "Q4 plan", "ann", 10 * day, 100),
            msg("b", "Re: Q4 plan", "bo", 11 * day, 100),
            // too long after, and not a reply: its own conversation
            msg("c", "Re: Q4 plan", "cy", 200 * day, 100),
            msg("d", "Q4 plan", "di", 12 * day, 100),
        ];
        let rows = threads(&msgs);
        let shape: Vec<(&str, usize)> = rows
            .iter()
            .map(|r| (msgs[r.index].message_id.as_deref().unwrap(), r.depth))
            .collect();
        assert_eq!(shape, [("c", 0), ("d", 0), ("a", 0), ("b", 1)]);
    }

    #[test]
    fn sorting_by_each_key_and_back() {
        let msgs = vec![
            msg("1", "Banana", "Zed", 30, 5),
            msg("2", "Apple", "Amy", 10, 50),
            msg("3", "Cherry", "Mia", 20, 500),
        ];
        let order = |rows: Vec<Row>| -> Vec<&str> {
            rows.iter()
                .map(|r| msgs[r.index].message_id.as_deref().unwrap())
                .collect()
        };
        assert_eq!(order(sorted(&msgs, SortKey::Date, false)), ["1", "3", "2"]);
        assert_eq!(order(sorted(&msgs, SortKey::Date, true)), ["2", "3", "1"]);
        assert_eq!(order(sorted(&msgs, SortKey::From, false)), ["2", "3", "1"]);
        assert_eq!(
            order(sorted(&msgs, SortKey::Subject, false)),
            ["2", "1", "3"]
        );
        assert_eq!(order(sorted(&msgs, SortKey::Size, false)), ["3", "2", "1"]);
        // conversations sort the same way
        assert_eq!(
            order(threads_sorted(&msgs, SortKey::Subject, false)),
            ["2", "1", "3"]
        );
        assert_eq!(SortKey::parse("Sender"), Some(SortKey::From));
    }
}
