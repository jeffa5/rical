//! Picking a colour per calendar that can be told apart from the others.
//! A calendar's own colour is kept unless an earlier one already looks
//! like it (servers often give the same colours to every account), in
//! which case its hue is turned just far enough to stand apart.

use rical_core::Rgb;

const fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
    }
}

/// For calendars without a colour, most distinct first.
const PALETTE: [Rgb; 12] = [
    rgb(0x3584e4), // blue
    rgb(0xe66100), // orange
    rgb(0x2ec27e), // green
    rgb(0x9141ac), // purple
    rgb(0xe01b24), // red
    rgb(0xf5c211), // yellow
    rgb(0x1c71d8), // dark blue
    rgb(0x986a44), // brown
    rgb(0x0e9f9f), // teal
    rgb(0xc061cb), // pink
    rgb(0x26a269), // dark green
    rgb(0x77767b), // grey
];

/// Colours closer than this look alike.
const ALIKE: f64 = 110.0;

/// How different two colours look: the "redmean" weighted distance.
fn distance(a: Rgb, b: Rgb) -> f64 {
    let rm = (a.r as f64 + b.r as f64) / 2.0;
    let (dr, dg, db) = (
        a.r as f64 - b.r as f64,
        a.g as f64 - b.g as f64,
        a.b as f64 - b.b as f64,
    );
    ((2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db).sqrt()
}

fn nearest(c: Rgb, used: &[Rgb]) -> f64 {
    used.iter()
        .map(|u| distance(c, *u))
        .fold(f64::INFINITY, f64::min)
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb {
        r: to(r),
        g: to(g),
        b: to(b),
    }
}

/// The first candidate that looks unlike everything used, or else the
/// one that looks least like any of it.
fn pick(candidates: impl IntoIterator<Item = Rgb>, used: &[Rgb]) -> Rgb {
    let mut best = (f64::NEG_INFINITY, PALETTE[0]);
    for c in candidates {
        let d = nearest(c, used);
        if d >= ALIKE {
            return c;
        }
        if d > best.0 {
            best = (d, c);
        }
    }
    best.1
}

/// A colour for each calendar, given their own colours, in order.
/// Calendars with their own colour are placed first, so adding one without
/// a colour never changes the others.
pub fn assign(own: &[Option<Rgb>]) -> Vec<Rgb> {
    let mut used: Vec<Rgb> = Vec::with_capacity(own.len());
    let mut out = vec![PALETTE[0]; own.len()];
    for (i, c) in own.iter().enumerate() {
        let Some(c) = *c else { continue };
        let chosen = if nearest(c, &used) >= ALIKE {
            c
        } else {
            // turn the hue a little at a time, either way
            let (h, s, l) = to_hsl(c);
            // greys have no hue to turn: give them some colour
            let s = s.max(0.45);
            let turns = (1..=6).flat_map(|k| [k as f64 * 30.0, -(k as f64) * 30.0]);
            pick(turns.map(|t| from_hsl(h + t, s, l)), &used)
        };
        used.push(chosen);
        out[i] = chosen;
    }
    for (i, c) in own.iter().enumerate() {
        if c.is_none() {
            let chosen = pick(PALETTE, &used);
            used.push(chosen);
            out[i] = chosen;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trip() {
        for c in PALETTE {
            let (h, s, l) = to_hsl(c);
            let back = from_hsl(h, s, l);
            assert!(distance(c, back) < 3.0, "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn own_colours_kept_when_distinct() {
        let own = [
            Some(rgb(0xb6469d)),
            Some(rgb(0xa5b872)),
            Some(rgb(0x0082c9)),
        ];
        assert_eq!(assign(&own), own.map(Option::unwrap));
    }

    #[test]
    fn repeated_colours_are_told_apart() {
        // two accounts given the same colours by the server
        let one = [
            Some(rgb(0xb6469d)),
            Some(rgb(0xa5b872)),
            Some(rgb(0x0082c9)),
        ];
        let own = [one, one].concat();
        let got = assign(&own);
        for (i, a) in got.iter().enumerate() {
            for b in &got[i + 1..] {
                assert!(distance(*a, *b) >= ALIKE, "{got:?}");
            }
        }
        assert_eq!(got[..3], one.map(Option::unwrap));
    }

    #[test]
    fn uncoloured_calendars_differ() {
        let got = assign(&[None; 6]);
        for (i, a) in got.iter().enumerate() {
            for b in &got[i + 1..] {
                assert!(distance(*a, *b) >= ALIKE, "{got:?}");
            }
        }
        // and avoid colours calendars already have
        let got = assign(&[Some(rgb(0x3584e4)), None]);
        assert_ne!(got[1], PALETTE[0]);
    }

    #[test]
    fn new_uncoloured_calendar_leaves_others_alone() {
        let one = [
            Some(rgb(0xb6469d)),
            Some(rgb(0xa5b872)),
            Some(rgb(0x0082c9)),
        ];
        let before = assign(&[one, one].concat());
        let mut own = [one, one].concat();
        own.insert(2, None);
        let mut after = assign(&own);
        after.remove(2);
        assert_eq!(before, after);
    }
}
