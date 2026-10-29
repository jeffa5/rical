//! Messages in conversations: by Message-ID, In-Reply-To and References.

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

/// The messages in conversations: the conversation with the newest message
/// first, each in the order of the replies (oldest first, replies below
/// what they answer).
pub fn threads(messages: &[Summary]) -> Vec<Row> {
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
    // the newest message anywhere in each tree
    fn newest(i: usize, messages: &[Summary], children: &[Vec<usize>]) -> i64 {
        children[i]
            .iter()
            .map(|&c| newest(c, messages, children))
            .max()
            .unwrap_or(i64::MIN)
            .max(messages[i].date)
    }
    let mut roots: Vec<(i64, usize)> = roots
        .into_iter()
        .map(|r| (newest(r, messages, &children), r))
        .collect();
    roots.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

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
    for (_, r) in roots {
        emit(r, 0, true, &children, &mut out);
    }
    out
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
