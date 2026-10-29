//! Maildir file names: `unique` in `new/`, `unique:2,FLAGS` in `cur/`.

/// The flags a Maildir message can carry in its name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub draft: bool,
    pub flagged: bool,
    pub passed: bool,
    pub replied: bool,
    pub seen: bool,
    pub trashed: bool,
}

impl Flags {
    pub fn parse(letters: &str) -> Flags {
        let mut f = Flags::default();
        for c in letters.chars() {
            match c {
                'D' => f.draft = true,
                'F' => f.flagged = true,
                'P' => f.passed = true,
                'R' => f.replied = true,
                'S' => f.seen = true,
                'T' => f.trashed = true,
                _ => {}
            }
        }
        f
    }

    /// The letters, in the order the Maildir spec wants: ASCII.
    pub fn letters(&self) -> String {
        [
            (self.draft, 'D'),
            (self.flagged, 'F'),
            (self.passed, 'P'),
            (self.replied, 'R'),
            (self.seen, 'S'),
            (self.trashed, 'T'),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, c)| *c)
        .collect()
    }
}

/// A file name split into its unique part and its flags. Other info
/// (not `2,`) is treated as no flags; unknown letters are dropped.
pub fn split_name(name: &str) -> (&str, Flags) {
    match name.rsplit_once(':') {
        Some((unique, info)) => match info.strip_prefix("2,") {
            Some(letters) => (unique, Flags::parse(letters)),
            None => (unique, Flags::default()),
        },
        None => (name, Flags::default()),
    }
}

/// The name for a message with these flags (as it is in `cur/`).
pub fn name_with(unique: &str, flags: Flags) -> String {
    format!("{unique}:2,{}", flags.letters())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let (u, f) = split_name("1700000000.M1P2.host:2,FRS");
        assert_eq!(u, "1700000000.M1P2.host");
        assert!(f.flagged && f.replied && f.seen && !f.trashed);
        assert_eq!(name_with(u, f), "1700000000.M1P2.host:2,FRS");
        assert_eq!(split_name("plain").1, Flags::default());
        assert_eq!(split_name("x:1,abc").0, "x");
    }

    #[test]
    fn letters_are_in_ascii_order() {
        let f = Flags::parse("TSRPFD");
        assert_eq!(f.letters(), "DFPRST");
    }
}
