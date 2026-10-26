//! A read-only view of one vCard, for showing it, searching it and
//! comparing it with others. Built on demand from the parsed card, so a
//! loaded address book costs only its compact form.
//!
//! The three versions in use differ in detail and are read alike: types
//! from `TYPE=work,pref` (3.0), `TYPE=work;PREF=1` (4.0) or bare `WORK`
//! (2.1, read as types by the parser); Apple's labels from `itemN.X-ABLabel`
//! grouped with the property; groups as `KIND:group` with `MEMBER`s, or
//! Apple's `X-ADDRESSBOOKSERVER-KIND`.

use compact_ics::{Component, Prop};

use crate::text::{split_structured, unescape};

/// One card, viewed.
#[derive(Clone, Copy)]
pub struct Card<'a>(pub &'a Component);

/// A value with what kind it is ("work", "mobile") and whether it is the
/// preferred one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Field {
    pub value: String,
    pub labels: Vec<String>,
    pub pref: bool,
}

/// The structured name (N): family; given; additional; prefixes; suffixes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Name {
    pub family: String,
    pub given: String,
    pub additional: String,
    pub prefixes: String,
    pub suffixes: String,
}

/// A postal address (ADR), by part.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Address {
    pub po_box: String,
    pub extended: String,
    pub street: String,
    pub locality: String,
    pub region: String,
    pub code: String,
    pub country: String,
    pub labels: Vec<String>,
    pub pref: bool,
}

impl Address {
    /// On one line, empty parts left out: "1 Park Row, Leeds, LS1 5HN, UK".
    pub fn one_line(&self) -> String {
        [
            &self.po_box,
            &self.extended,
            &self.street,
            &self.locality,
            &self.region,
            &self.code,
            &self.country,
        ]
        .into_iter()
        .map(|p| p.trim().replace('\n', ", "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// A property's name without a vCard group: `item1.EMAIL` → `EMAIL`.
pub fn base_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// A property's group, if it has one: `item1.EMAIL` → `item1`.
fn group(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(g, _)| g)
}

/// Kinds that say nothing to a reader.
const QUIET: &[&str] = &["pref", "voice", "internet", "x400", "other", "x-other"];

impl<'a> Card<'a> {
    fn props(&self, name: &'a str) -> impl Iterator<Item = &'a Prop> + 'a {
        self.0
            .all_props()
            .iter()
            .filter(move |p| base_name(&p.name) == name)
    }

    fn text(&self, name: &'a str) -> Option<String> {
        self.props(name)
            .next()
            .map(|p| unescape(p.value().trim()))
            .filter(|v| !v.is_empty())
    }

    fn texts(&self, name: &'a str) -> Vec<String> {
        self.props(name)
            .map(|p| unescape(p.value().trim()))
            .filter(|v| !v.is_empty())
            .collect()
    }

    /// Apple's label for a grouped property (`item1.X-ABLabel:_$!<Home>!$_`).
    fn ab_label(&self, p: &Prop) -> Option<String> {
        let g = group(&p.name)?;
        let label = self
            .0
            .all_props()
            .iter()
            .find(|q| group(&q.name) == Some(g) && base_name(&q.name) == "X-ABLABEL")?;
        let v = label.value().trim();
        let v = v
            .strip_prefix("_$!<")
            .and_then(|v| v.strip_suffix(">!$_"))
            .unwrap_or(v);
        Some(unescape(v).to_lowercase()).filter(|v| !v.is_empty())
    }

    /// A property's kinds and whether it is preferred.
    fn kinds(&self, p: &Prop) -> (Vec<String>, bool) {
        let mut labels = Vec::new();
        let mut pref = false;
        for (k, v) in p.params() {
            if k.eq_ignore_ascii_case("PREF") {
                pref = true;
                continue;
            }
            if !k.eq_ignore_ascii_case("TYPE") {
                continue;
            }
            for t in v.split(',') {
                let t = t.trim().trim_matches('"').to_lowercase();
                if t == "pref" {
                    pref = true;
                }
                // "cell" is the vCard word; people say mobile
                let t = if t == "cell" { "mobile".to_owned() } else { t };
                if !t.is_empty() && !QUIET.contains(&t.as_str()) && !labels.contains(&t) {
                    labels.push(t);
                }
            }
        }
        if let Some(l) = self.ab_label(p)
            && !labels.contains(&l)
        {
            labels.push(l);
        }
        (labels, pref)
    }

    fn fields(&self, name: &'a str, value: impl Fn(&str) -> String) -> Vec<Field> {
        self.props(name)
            .filter_map(|p| {
                let v = value(p.value().trim());
                if v.is_empty() {
                    return None;
                }
                let (labels, pref) = self.kinds(p);
                Some(Field {
                    value: v,
                    labels,
                    pref,
                })
            })
            .collect()
    }

    pub fn uid(&self) -> Option<&'a str> {
        self.props("UID").next().map(|p| p.value().trim())
    }

    /// "4.0", "3.0" or "2.1".
    pub fn version(&self) -> Option<&'a str> {
        self.props("VERSION").next().map(|p| p.value().trim())
    }

    /// FN, as written.
    pub fn formatted_name(&self) -> Option<String> {
        self.text("FN")
    }

    pub fn name(&self) -> Option<Name> {
        let p = self.props("N").next()?;
        let mut parts = split_structured(p.value()).into_iter();
        let mut next = || parts.next().unwrap_or_default();
        let n = Name {
            family: next(),
            given: next(),
            additional: next(),
            prefixes: next(),
            suffixes: next(),
        };
        (n != Name::default()).then_some(n)
    }

    /// What to call them: FN, else the name's parts, else the organisation,
    /// else an email address or phone number, else "No name".
    pub fn display_name(&self) -> String {
        if let Some(f) = self.formatted_name() {
            return f;
        }
        if let Some(n) = self.name() {
            let joined = [&n.prefixes, &n.given, &n.additional, &n.family, &n.suffixes]
                .into_iter()
                .filter(|p| !p.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if !joined.is_empty() {
                return joined;
            }
        }
        if let Some(o) = self.org().into_iter().next() {
            return o;
        }
        if let Some(e) = self.emails().into_iter().next() {
            return e.value;
        }
        if let Some(t) = self.phones().into_iter().next() {
            return t.value;
        }
        "No name".to_owned()
    }

    /// The key to sort by: family name then given, else the display name;
    /// folded so accents sort with their letters.
    pub fn sort_key(&self) -> String {
        let n = self.name().unwrap_or_default();
        let key = if n.family.is_empty() && n.given.is_empty() {
            self.display_name()
        } else {
            format!("{} {}", n.family, n.given)
        };
        crate::text::fold(key.trim())
    }

    pub fn nicknames(&self) -> Vec<String> {
        self.texts("NICKNAME")
            .iter()
            .flat_map(|n| n.split(','))
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect()
    }

    pub fn emails(&self) -> Vec<Field> {
        self.fields("EMAIL", |v| {
            let v = v.strip_prefix("mailto:").unwrap_or(v);
            unescape(v)
        })
    }

    pub fn phones(&self) -> Vec<Field> {
        self.fields("TEL", |v| v.strip_prefix("tel:").unwrap_or(v).to_owned())
    }

    pub fn addresses(&self) -> Vec<Address> {
        self.props("ADR")
            .filter_map(|p| {
                let mut parts = split_structured(p.value()).into_iter();
                let mut next = || parts.next().unwrap_or_default();
                let (labels, pref) = self.kinds(p);
                let a = Address {
                    po_box: next(),
                    extended: next(),
                    street: next(),
                    locality: next(),
                    region: next(),
                    code: next(),
                    country: next(),
                    labels,
                    pref,
                };
                (!a.one_line().is_empty()).then_some(a)
            })
            .collect()
    }

    /// The organisation, then its units.
    pub fn org(&self) -> Vec<String> {
        self.props("ORG")
            .next()
            .map(|p| {
                split_structured(p.value())
                    .into_iter()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn title(&self) -> Option<String> {
        self.text("TITLE")
    }

    pub fn role(&self) -> Option<String> {
        self.text("ROLE")
    }

    /// As written: "1990-01-31", "--0131" (no year)…
    pub fn birthday(&self) -> Option<String> {
        self.text("BDAY")
    }

    pub fn anniversary(&self) -> Option<String> {
        self.text("ANNIVERSARY")
            .or_else(|| self.text("X-ANNIVERSARY"))
    }

    pub fn urls(&self) -> Vec<Field> {
        self.fields("URL", |v| v.to_owned())
    }

    /// Instant messaging and social handles (IMPP, and the old X-JABBER and
    /// the like).
    pub fn chats(&self) -> Vec<Field> {
        let mut out = self.fields("IMPP", |v| v.to_owned());
        for (name, scheme) in [
            ("X-JABBER", "xmpp"),
            ("X-SKYPE", "skype"),
            ("X-AIM", "aim"),
            ("X-ICQ", "icq"),
        ] {
            for mut f in self.fields(name, unescape) {
                f.value = format!("{scheme}:{}", f.value);
                out.push(f);
            }
        }
        out
    }

    pub fn notes(&self) -> Vec<String> {
        self.texts("NOTE")
    }

    /// The groups or tags it is in (CATEGORIES).
    pub fn categories(&self) -> Vec<String> {
        self.props("CATEGORIES")
            .flat_map(|p| {
                // commas separate, unless escaped
                let mut out = vec![String::new()];
                let mut chars = p.value().chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                out.last_mut().unwrap().push(n);
                            }
                        }
                        ',' => out.push(String::new()),
                        c => out.last_mut().unwrap().push(c),
                    }
                }
                out
            })
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .collect()
    }

    /// Whether it has a photo (inline or by link).
    pub fn has_photo(&self) -> bool {
        self.props("PHOTO").next().is_some()
    }

    /// `individual` (the default), `group`, `org` or `location`.
    pub fn kind(&self) -> String {
        self.props("KIND")
            .next()
            .or_else(|| self.props("X-ADDRESSBOOKSERVER-KIND").next())
            .map(|p| p.value().trim().to_lowercase())
            .unwrap_or_else(|| "individual".to_owned())
    }

    /// For a group: its members' UIDs (`urn:uuid:` taken off).
    pub fn members(&self) -> Vec<String> {
        self.props("MEMBER")
            .chain(self.props("X-ADDRESSBOOKSERVER-MEMBER"))
            .map(|p| {
                let v = p.value().trim();
                v.strip_prefix("urn:uuid:").unwrap_or(v).to_owned()
            })
            .collect()
    }

    /// When it last changed (REV), as written.
    pub fn revision(&self) -> Option<&'a str> {
        self.props("REV").next().map(|p| p.value().trim())
    }

    /// How much it says: used to pick which of two duplicates to keep.
    pub fn richness(&self) -> usize {
        self.0
            .all_props()
            .iter()
            .filter(|p| {
                !matches!(
                    base_name(&p.name),
                    "VERSION" | "PRODID" | "UID" | "REV" | "X-ABLABEL"
                )
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(text: &str) -> Component {
        let (mut cards, _) = compact_ics::parse(text).unwrap();
        cards.remove(0)
    }

    #[test]
    fn reads_a_3_0_card_from_apple() {
        let c = card(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Shah;Priya;;Dr;\r\nFN:Dr Priya Shah\r\n\
             NICKNAME:Pri\r\nORG:Northwind\\, Ltd;Platform\r\nTITLE:Lead\r\n\
             EMAIL;type=INTERNET;type=WORK;type=pref:priya@example.com\r\n\
             item1.EMAIL;type=INTERNET:pri@home.example\r\nitem1.X-ABLabel:_$!<Home>!$_\r\n\
             TEL;type=CELL;type=VOICE;type=pref:+44 7700 900123\r\n\
             item2.ADR;type=HOME:;;1 Park Row;Leeds;;LS1 5HN;UK\r\nitem2.X-ABLabel:Flat\r\n\
             BDAY;VALUE=date:1990-01-31\r\nURL:https://priya.example\r\n\
             CATEGORIES:Work,Friends\r\nNOTE:Met at the\\nconference\r\nUID:abc\r\n\
             END:VCARD\r\n",
        );
        let c = Card(&c);
        assert_eq!(c.uid(), Some("abc"));
        assert_eq!(c.version(), Some("3.0"));
        assert_eq!(c.display_name(), "Dr Priya Shah");
        assert_eq!(c.name().unwrap().given, "Priya");
        assert_eq!(c.sort_key(), "shah priya");
        assert_eq!(c.nicknames(), ["Pri"]);
        assert_eq!(c.org(), ["Northwind, Ltd", "Platform"]);
        let emails = c.emails();
        assert_eq!(emails[0].value, "priya@example.com");
        assert_eq!(emails[0].labels, ["work"]);
        assert!(emails[0].pref);
        assert_eq!(emails[1].labels, ["home"]);
        assert_eq!(c.phones()[0].labels, ["mobile"]);
        let adr = &c.addresses()[0];
        assert_eq!(adr.one_line(), "1 Park Row, Leeds, LS1 5HN, UK");
        assert_eq!(adr.labels, ["home", "flat"]);
        assert_eq!(c.birthday().as_deref(), Some("1990-01-31"));
        assert_eq!(c.categories(), ["Work", "Friends"]);
        assert_eq!(c.notes(), ["Met at the\nconference"]);
        assert_eq!(c.kind(), "individual");
    }

    #[test]
    fn reads_4_0_and_2_1() {
        let c = card(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Sam Lee\r\n\
             EMAIL;TYPE=work;PREF=1:mailto:sam@example.com\r\n\
             TEL;VALUE=uri;TYPE=\"voice,cell\":tel:+44-7700-900123\r\n\
             IMPP:xmpp:sam@chat.example\r\nEND:VCARD\r\n",
        );
        let c = Card(&c);
        assert_eq!(c.emails()[0].value, "sam@example.com");
        assert!(c.emails()[0].pref);
        assert_eq!(c.phones()[0].value, "+44-7700-900123");
        assert_eq!(c.phones()[0].labels, ["mobile"]);
        assert_eq!(c.chats()[0].value, "xmpp:sam@chat.example");

        let old = card(
            "BEGIN:VCARD\r\nVERSION:2.1\r\nN:Park;Jo\r\nTEL;HOME;VOICE:0113 496 0000\r\nEND:VCARD\r\n",
        );
        let old = Card(&old);
        assert_eq!(old.display_name(), "Jo Park");
        assert_eq!(old.phones()[0].labels, ["home"]);
    }

    #[test]
    fn names_fall_back() {
        let only_mail = card("BEGIN:VCARD\r\nVERSION:3.0\r\nEMAIL:a@b.c\r\nEND:VCARD\r\n");
        assert_eq!(Card(&only_mail).display_name(), "a@b.c");
        let org = card("BEGIN:VCARD\r\nVERSION:3.0\r\nORG:Acme\r\nEND:VCARD\r\n");
        assert_eq!(Card(&org).display_name(), "Acme");
        let none = card("BEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n");
        assert_eq!(Card(&none).display_name(), "No name");
    }

    #[test]
    fn groups() {
        let g = card(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Family\r\n\
             MEMBER:urn:uuid:aaa\r\nMEMBER:urn:uuid:bbb\r\nEND:VCARD\r\n",
        );
        assert_eq!(Card(&g).kind(), "group");
        assert_eq!(Card(&g).members(), ["aaa", "bbb"]);
    }
}
