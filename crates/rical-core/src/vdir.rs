//! A vdir: a folder of collections (calendars), each a folder of `.ics` items.
//!
//! See <https://vdirsyncer.pimutils.org/en/stable/vdir.html>.

use std::{
    fmt,
    fs::{self, read_dir},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context as _;
use log::warn;
use serde::Serialize;

use crate::{component::AnyComponent, ics::read_ics_to_calendar};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl FromStr for Rgb {
    type Err = String;

    /// Parse `#rrggbb` or `#rrggbbaa` (alpha is ignored).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim().trim_start_matches('#');
        if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("invalid colour {s:?}"));
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        Ok(Self {
            r: byte(0),
            g: byte(2),
            b: byte(4),
        })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// One collection in a vdir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Calendar {
    /// Folder name, stable across runs.
    pub id: String,
    pub path: PathBuf,
    /// From the `displayname` file, falling back to the id.
    pub name: String,
    /// From the `color` file.
    pub color: Option<Rgb>,
}

impl Calendar {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let id = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("calendar path has no name: {path:?}"))?
            .to_owned();

        let name = read_metadata(path, "displayname").unwrap_or_else(|| id.clone());
        let color = read_metadata(path, "color").and_then(|c| match c.parse() {
            Ok(c) => Some(c),
            Err(e) => {
                warn!("{path:?}: {e}");
                None
            }
        });

        Ok(Self {
            id,
            path: path.to_owned(),
            name,
            color,
        })
    }

    /// Paths of the `.ics` items in this calendar, sorted.
    pub fn item_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut paths = read_dir(&self.path)
            .with_context(|| format!("reading calendar {:?}", self.path))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "ics"))
            .collect::<Vec<_>>();
        paths.sort();
        Ok(paths)
    }

    /// Every component of every item in this calendar. Unparseable items are
    /// skipped with a warning.
    pub fn components(&self) -> anyhow::Result<Vec<AnyComponent>> {
        Ok(self
            .item_paths()?
            .into_iter()
            .filter_map(|p| match read_ics_to_calendar(&p) {
                Ok(cal) => Some((p, cal)),
                Err(e) => {
                    warn!("skipping {p:?}: {e}");
                    None
                }
            })
            .flat_map(|(p, cal)| {
                cal.components
                    .into_iter()
                    .map(move |c| AnyComponent::new(p.clone(), c))
            })
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Vdir {
    pub root: PathBuf,
    /// Sorted by id.
    pub calendars: Vec<Calendar>,
}

impl Vdir {
    /// Load the calendars under `root`. Each non-hidden subdirectory is a
    /// calendar. If there are none, `root` itself is treated as one calendar.
    pub fn load(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        if !root.is_dir() {
            anyhow::bail!("vdir root is not a directory: {root:?}");
        }

        let mut calendars = read_dir(root)
            .with_context(|| format!("reading vdir {root:?}"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !is_hidden(p))
            .map(|p| Calendar::load(&p))
            .collect::<anyhow::Result<Vec<_>>>()?;

        if calendars.is_empty() {
            calendars.push(Calendar::load(root)?);
        }

        calendars.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(Self {
            root: root.to_owned(),
            calendars,
        })
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    /// The calendar whose folder contains `path`.
    pub fn calendar_for_path(&self, path: &Path) -> Option<&Calendar> {
        self.calendars
            .iter()
            .find(|c| path.parent() == Some(c.path.as_path()))
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

fn read_metadata(dir: &Path, name: &str) -> Option<String> {
    let s = fs::read_to_string(dir.join(name)).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rgb() {
        assert_eq!(
            "#0082c9".parse::<Rgb>().unwrap(),
            Rgb {
                r: 0x00,
                g: 0x82,
                b: 0xc9
            }
        );
        assert_eq!(
            "#A5B872FF\n".parse::<Rgb>().unwrap(),
            Rgb {
                r: 0xa5,
                g: 0xb8,
                b: 0x72
            }
        );
        assert!("#123".parse::<Rgb>().is_err());
        assert!("#zzzzzz".parse::<Rgb>().is_err());
    }

    #[test]
    fn display_rgb() {
        assert_eq!(
            Rgb {
                r: 1,
                g: 0xab,
                b: 0xff
            }
            .to_string(),
            "#01abff"
        );
    }
}
