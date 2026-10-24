use std::collections::BTreeMap;

/// Map each id to its shortest prefix that is unique among `all_ids`.
pub fn build_prefixes_map(all_ids: &mut [String]) -> BTreeMap<String, String> {
    let prefixes = build_prefixes(all_ids);
    all_ids.iter().cloned().zip(prefixes).collect()
}

fn build_prefixes(all_ids: &mut [String]) -> Vec<String> {
    if all_ids.is_empty() {
        return Vec::new();
    }

    all_ids.sort_unstable();

    let mut prefixes = Vec::new();

    prefixes.push(find_prefix(
        "",
        &all_ids[0],
        all_ids.get(1).map_or("", |s| s.as_str()),
    ));

    for window in all_ids.windows(3) {
        let [u1, u2, u3] = window else {
            panic!("window length not what was expected");
        };
        prefixes.push(find_prefix(u1, u2, u3));
    }

    let len = all_ids.len();
    if len > 1 {
        prefixes.push(find_prefix(&all_ids[len - 2], &all_ids[len - 1], ""));
    }

    assert_eq!(
        all_ids.len(),
        prefixes.len(),
        "should have the same number of prefixes as original ids"
    );

    prefixes
}

fn find_prefix(u1: &str, u2: &str, u3: &str) -> String {
    let mut before_finished = false;
    let mut after_finished = false;
    let mut matched = false;
    let prefix = u1
        .chars()
        .map(Some)
        .chain(std::iter::repeat(None))
        .zip(u2.chars().map(Some))
        .zip(u3.chars().map(Some).chain(std::iter::repeat(None)))
        .take_while(|((c1, c2), c3)| {
            before_finished |= c1 != c2;
            after_finished |= c3 != c2;
            if !before_finished || !after_finished {
                true
            } else {
                // get one more past what matches
                matched = !matched;
                matched
            }
        })
        .map(|((_, c2), _)| c2.unwrap())
        .collect::<String>();
    assert!(!prefix.is_empty());
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes(ids: &[&str]) -> BTreeMap<String, String> {
        let mut ids = ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        build_prefixes_map(&mut ids)
    }

    #[test]
    fn empty() {
        assert!(prefixes(&[]).is_empty());
    }

    #[test]
    fn single() {
        assert_eq!(prefixes(&["abc"])["abc"], "a");
    }

    #[test]
    fn distinct_first_chars() {
        let p = prefixes(&["abc", "bcd", "cde"]);
        assert_eq!(p["abc"], "a");
        assert_eq!(p["bcd"], "b");
        assert_eq!(p["cde"], "c");
    }

    #[test]
    fn shared_prefixes() {
        let p = prefixes(&["abcd", "abce", "abx", "z"]);
        assert_eq!(p["abcd"], "abcd");
        assert_eq!(p["abce"], "abce");
        assert_eq!(p["abx"], "abx");
        assert_eq!(p["z"], "z");
    }

    #[test]
    fn prefixes_are_unique() {
        let ids = ["4b31", "4b9a", "4c00", "a1", "a2b", "a2c", "ff"];
        let p = prefixes(&ids);
        for (id, prefix) in &p {
            assert!(id.starts_with(prefix.as_str()));
            let matching = ids
                .iter()
                .filter(|i| i.starts_with(prefix.as_str()))
                .count();
            assert_eq!(matching, 1, "prefix {prefix:?} of {id:?} is ambiguous");
        }
    }
}
