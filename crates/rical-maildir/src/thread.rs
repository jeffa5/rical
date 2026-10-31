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
    let bases: Vec<String> = messages.iter().map(|m| base_subject(&m.subject)).collect();
    let mut by_subject: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, b) in bases.iter().enumerate() {
        by_subject.entry(b).or_default().push(i);
    }
    // each subject's messages by date (then index), to find the latest
    // earlier one by halves
    for same in by_subject.values_mut() {
        same.sort_by_key(|&j| (messages[j].date, j));
    }
    for i in 0..messages.len() {
        let m = &messages[i];
        let base = bases[i].as_str();
        if parents[i].is_some()
            || base.is_empty()
            // no Re: or Fwd: before it
            || base == m.subject.trim().to_lowercase()
        {
            continue;
        }
        // the latest first, back to six weeks before
        let Some(same) = by_subject.get(base) else {
            continue;
        };
        let upto = same.partition_point(|&j| messages[j].date <= m.date);
        parents[i] = same[..upto]
            .iter()
            .rev()
            .copied()
            .take_while(|&j| m.date - messages[j].date < 42 * 86_400)
            .find(|&j| {
                j != i
                    // not something that is itself a reply to this one
                    && !(messages[j].in_reply_to.iter().chain(&messages[j].references))
                        .any(|id| Some(id.as_str()) == m.message_id.as_deref())
            });
    }

    // no loops: a parent chain that returns to itself is cut. A chain
    // known to end is not walked again, so this is linear.
    let mut ends = vec![false; messages.len()];
    let mut walk = vec![usize::MAX; messages.len()];
    let mut path = Vec::new();
    for i in 0..messages.len() {
        if ends[i] {
            continue;
        }
        path.clear();
        path.push(i);
        walk[i] = i;
        let mut p = parents[i];
        let mut cycle = false;
        while let Some(x) = p {
            if ends[x] {
                break;
            }
            if walk[x] == i {
                cycle = true;
                break;
            }
            walk[x] = i;
            path.push(x);
            p = parents[x];
        }
        if cycle {
            parents[i] = None;
            ends[i] = true;
        } else {
            for &x in &path {
                ends[x] = true;
            }
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
    // each tree in order, parents before children (no recursion: a long
    // chain of replies is as deep as it is long)
    let mut order: Vec<(usize, usize)> = Vec::with_capacity(messages.len());
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &r in &roots {
        stack.push((r, 0));
        while let Some((i, depth)) = stack.pop() {
            order.push((i, depth));
            stack.extend(children[i].iter().rev().map(|&c| (c, depth + 1)));
        }
    }
    // the newest message anywhere in each tree, and the size of all of it,
    // worked out once, children first
    let mut newest: Vec<i64> = messages.iter().map(|m| m.date).collect();
    let mut total: Vec<u64> = messages.iter().map(|m| m.size).collect();
    for &(i, _) in order.iter().rev() {
        if let Some(p) = parents[i] {
            newest[p] = newest[p].max(newest[i]);
            total[p] += total[i];
        }
    }
    let mut roots: Vec<usize> = roots;
    match key {
        SortKey::Date => roots.sort_by_key(|&r| (std::cmp::Reverse(newest[r]), r)),
        SortKey::Size => roots.sort_by_key(|&r| (std::cmp::Reverse(total[r]), r)),
        SortKey::From => roots.sort_by_cached_key(|&r| (from_key(&messages[r]), r)),
        SortKey::Subject => roots.sort_by_key(|&r| (bases[r].as_str(), r)),
    }
    if reverse {
        roots.reverse();
    }

    // where each tree starts in `order`, to lay them out in sorted order
    let mut start = vec![0usize; messages.len()];
    let mut at = 0;
    while at < order.len() {
        let r = order[at].0;
        start[r] = at;
        at += 1;
        while at < order.len() && order[at].1 > 0 {
            at += 1;
        }
    }
    let mut out = Vec::with_capacity(messages.len());
    for r in roots {
        let from = start[r];
        out.push(Row {
            index: r,
            depth: 0,
            head: true,
        });
        out.extend(
            order[from + 1..]
                .iter()
                .take_while(|(_, d)| *d > 0)
                .map(|&(index, depth)| Row {
                    index,
                    depth,
                    head: false,
                }),
        );
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
    fn a_long_chain_of_replies_and_a_loop_are_laid_out() {
        // each answers the one before, 50,000 deep: no recursion to run out
        // of stack, and in order
        let ids: Vec<String> = (0..50_000).map(|i| format!("m{i}")).collect();
        let mut msgs: Vec<Summary> = (0..ids.len())
            .map(|i| {
                let before: Vec<&str> = if i == 0 { vec![] } else { vec![&ids[i - 1]] };
                m(&ids[i], &before, i as i64)
            })
            .collect();
        // and two that answer each other
        msgs.push(m("x", &["y"], 1));
        msgs.push(m("y", &["x"], 2));
        let rows = threads(&msgs);
        assert_eq!(rows.len(), msgs.len());
        let chain: Vec<&Row> = rows.iter().filter(|r| r.index < 50_000).collect();
        assert!(
            chain
                .iter()
                .enumerate()
                .all(|(i, r)| r.index == i && r.depth == i)
        );
        assert_eq!(rows.iter().filter(|r| r.head).count(), 2);
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
