//! Where searching happens: in this app (the folders' headers), or by a
//! tool that has the whole mailbox indexed. A tool is a command, run with
//! the query, that gives back a Maildir of symlinks to the matches (mu's
//! `--format=links`) or a list of file names (notmuch's `--output=files`).

use std::path::Path;

use crate::search::Syntax;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// The app's own: one folder as you type, or every folder.
    Builtin,
    Mu,
    Notmuch,
    /// A command with `{query}` and, if it makes links, `{dir}`.
    Custom(String),
}

/// What the command gives back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gives {
    /// A Maildir of symlinks in `{dir}`.
    Links,
    /// File names, one a line, on stdout.
    Paths,
}

pub const MU: &str = "mu find --clearlinks --format=links --linksdir={dir} -- {query}";
pub const NOTMUCH: &str = "notmuch search --output=files --sort=newest-first -- {query}";

impl Backend {
    /// From the `search` setting: `builtin`, `mu`, `notmuch`, or a command.
    pub fn from_setting(s: &str) -> Backend {
        match s.trim() {
            "" | "builtin" | "own" => Backend::Builtin,
            "mu" => Backend::Mu,
            "notmuch" => Backend::Notmuch,
            other => Backend::Custom(other.to_owned()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Backend::Builtin => "built-in search",
            Backend::Mu => "mu",
            Backend::Notmuch => "notmuch",
            Backend::Custom(_) => "your search command",
        }
    }

    /// The words its queries are written in.
    pub fn syntax(&self) -> Syntax {
        match self {
            Backend::Builtin | Backend::Custom(_) => Syntax::Builtin,
            Backend::Mu => Syntax::Mu,
            Backend::Notmuch => Syntax::Notmuch,
        }
    }

    fn template(&self) -> Option<&str> {
        match self {
            Backend::Builtin => None,
            Backend::Mu => Some(MU),
            Backend::Notmuch => Some(NOTMUCH),
            Backend::Custom(c) => Some(c),
        }
    }

    pub fn gives(&self) -> Gives {
        match self.template() {
            Some(t) if !t.contains("{dir}") => Gives::Paths,
            _ => Gives::Links,
        }
    }

    /// The shell command to search for `query`, with the links made in `dir`.
    pub fn command(&self, query: &str, dir: &Path) -> Option<String> {
        Some(
            self.template()?
                .replace("{dir}", &shell_quote(&dir.to_string_lossy()))
                .replace("{query}", &shell_quote(query)),
        )
    }

    /// The command that brings the index up to date after new mail.
    pub fn index_command(&self) -> Option<&'static str> {
        match self {
            Backend::Mu => Some("mu index"),
            Backend::Notmuch => Some("notmuch new"),
            _ => None,
        }
    }
}

/// `s` as one shell word.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_quoted() {
        let b = Backend::from_setting("mu");
        assert_eq!(
            b.command("from:o'brien subject:\"a b\"", Path::new("/tmp/my links"))
                .unwrap(),
            "mu find --clearlinks --format=links --linksdir='/tmp/my links' -- 'from:o'\\''brien subject:\"a b\"'"
        );
        assert_eq!(b.gives(), Gives::Links);
        assert_eq!(Backend::from_setting("notmuch").gives(), Gives::Paths);
        let c = Backend::from_setting("mysearch {query} > {dir}/x");
        assert_eq!(c.gives(), Gives::Links);
        assert_eq!(
            Backend::from_setting("my-paths {query}").gives(),
            Gives::Paths
        );
        assert_eq!(
            Backend::from_setting("").command("x", Path::new("/d")),
            None
        );
        assert_eq!(Backend::from_setting("mu").syntax(), Syntax::Mu);
    }
}
