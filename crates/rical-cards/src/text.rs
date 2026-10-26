//! vCard text: escaping, structured values, and the normal forms search
//! and duplicate-finding compare by.

/// A TEXT value as written: `\n` `\,` `\;` `\\` undone.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// TEXT for writing: the reverse of [`unescape`].
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// A structured value (N, ADR, ORG) split at unescaped `;`, each part
/// unescaped.
pub fn split_structured(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let last = parts.last_mut().unwrap();
                last.push('\\');
                if let Some(next) = chars.next() {
                    last.push(next);
                }
            }
            ';' => parts.push(String::new()),
            c => parts.last_mut().unwrap().push(c),
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

/// Parts joined back into a structured value, each escaped.
pub fn join_structured(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| escape(p))
        .collect::<Vec<_>>()
        .join(";")
}

/// Lower case with accents taken off (é → e, ß → ss, ø → o), for matching
/// what people type against what is written.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä'
            | 'Å' | 'Ā' | 'Ă' | 'Ą' => out.push('a'),
            'æ' | 'Æ' => out.push_str("ae"),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => out.push('c'),
            'ď' | 'đ' | 'Ď' | 'Đ' | 'ð' | 'Ð' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē'
            | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => out.push('e'),
            'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => out.push('g'),
            'ĥ' | 'ħ' | 'Ĥ' | 'Ħ' => out.push('h'),
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ'
            | 'Ī' | 'Ĭ' | 'Į' | 'İ' => out.push('i'),
            'ĵ' | 'Ĵ' => out.push('j'),
            'ķ' | 'Ķ' => out.push('k'),
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => out.push('l'),
            'ñ' | 'ń' | 'ņ' | 'ň' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö'
            | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => out.push('o'),
            'œ' | 'Œ' => out.push_str("oe"),
            'ŕ' | 'ŗ' | 'ř' | 'Ŕ' | 'Ŗ' | 'Ř' => out.push('r'),
            'ś' | 'ŝ' | 'ş' | 'š' | 'Ś' | 'Ŝ' | 'Ş' | 'Š' => out.push('s'),
            'ß' => out.push_str("ss"),
            'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => out.push('t'),
            'þ' | 'Þ' => out.push_str("th"),
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'Ù' | 'Ú' | 'Û' | 'Ü'
            | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => out.push('u'),
            'ŵ' | 'Ŵ' => out.push('w'),
            'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => out.push('y'),
            'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => out.push('z'),
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// The words of `s`, folded: letters and digits, split at anything else
/// ("O'Brien-Smith" → "o", "brien", "smith"), plus the whole run of
/// letters across apostrophes ("obrien"), so either way of typing finds it.
pub fn words(s: &str) -> Vec<String> {
    let folded = fold(s);
    let mut out: Vec<String> = folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    for joined in folded.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')) {
        let w: String = joined.chars().filter(|c| c.is_alphanumeric()).collect();
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// A phone number's digits, for comparing however it is written; a
/// leading `+` is kept, so "+44 7700 900123" and "07700 900123" differ but
/// [`phone_tail`] can match them.
pub fn phone_digits(s: &str) -> String {
    let s = s.strip_prefix("tel:").unwrap_or(s);
    let plus = s.trim_start().starts_with('+');
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if plus { format!("+{digits}") } else { digits }
}

/// The last nine digits of a number: the part that is the same whether
/// it is written with a country code or a trunk 0, for spotting the same
/// number written two ways. None for short numbers.
pub fn phone_tail(s: &str) -> Option<String> {
    let digits: Vec<char> = s.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 9).then(|| digits[digits.len() - 9..].iter().collect())
}

/// An email address as compared: trimmed, `mailto:` off, lower case.
pub fn email_key(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("mailto:")
        .or_else(|| s.strip_prefix("MAILTO:"))
        .unwrap_or(s);
    s.to_lowercase()
}

/// A name as compared: its words folded and sorted, so "Lee, Sam", "Sam
/// Lee" and "SAM LEE" are the same.
pub fn name_key(s: &str) -> String {
    let mut w: Vec<String> = fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    w.sort();
    w.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_round_trips() {
        let s = "Lee, Sam; Jr\\\nsecond line";
        assert_eq!(unescape(&escape(s)), s);
        assert_eq!(escape("a,b;c"), "a\\,b\\;c");
    }

    #[test]
    fn structured_values() {
        assert_eq!(
            split_structured("Lee;Sam;;Dr\\, Prof;"),
            ["Lee", "Sam", "", "Dr, Prof", ""]
        );
        let parts = split_structured(";;1 Park Row\\nFlat 2;Leeds;;LS1;UK");
        assert_eq!(parts[2], "1 Park Row\nFlat 2");
        assert_eq!(split_structured(&join_structured(&parts)), parts);
    }

    #[test]
    fn folding_and_words() {
        assert_eq!(fold("Zoë Ørsted-Straße"), "zoe orsted-strasse");
        assert_eq!(words("O'Brien-Smith"), ["o", "brien", "smith", "obrien"]);
        assert_eq!(words("José García"), ["jose", "garcia"]);
    }

    #[test]
    fn keys() {
        assert_eq!(phone_digits("+44 (0)7700 900-123"), "+4407700900123");
        assert_eq!(phone_tail("+44 7700 900123"), phone_tail("07700 900123"));
        assert_eq!(phone_tail("123"), None);
        assert_eq!(email_key(" MAILTO:Sam@Example.COM "), "sam@example.com");
        assert_eq!(name_key("Lee, Sam"), name_key("SAM  LEE"));
    }
}
