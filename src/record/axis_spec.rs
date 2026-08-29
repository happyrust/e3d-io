//! Axis specifications: the count-prefixed word pairs that catalogue geometry
//! uses for `Paxis`, `Pbaxis`, `Paaxis` and `Pcaxis`.
//!
//! E3D prints these as `X`, `-Y` or `P7`. The database stores a leading count
//! followed by a kind and a value, so `-Y` is `[2, 2, 2]` and `P7` is
//! `[2, 3, 7]`. Read as a plain word array they came out as three numbers and
//! every one of the 44 sampled axis attributes counted as a mismatch.
//!
//! A negative kind is a rotation from one axis towards another, and then the
//! kind is the pair itself: `-42` with `4500` prints `-X45Y`. Reading it as
//! `A1 * 10 + A2` over `X Y Z -X -Y -Z` numbered 1 to 6 accounts for every
//! pair the 2026-08-27 capture read back, `-15`, `-21`, `-26` and `-42`
//! included, and the second word is hundredths of a degree in all four.
//!
//! A *positive* pair names its angle instead of stating it, and the pair reads
//! the same way: `12` with `-4` prints `X DDANGLE Y`, `21` with `-4` prints
//! `Y DDANGLE X`, and `64` with `20` prints `-Z PARAM 20-X`. `-4` is the only
//! named angle any of the three catalogue databases holds, and the parameter
//! number is printed as stored.
//!
//! The kinds were recovered from the TTY sample; anything else returns `None`
//! rather than a guess.
//!
//! A census of `ams7000_0001`, `ams5052_0001` and `ams5053_0001` — 113343
//! elements across the seventeen nouns that have a `PAXI` — turns up 39
//! distinct pairs. Thirty-five of them were read back from E3D on 2026-08-27
//! and every one agrees with what this renders.

const KIND_AXIS: u32 = 1;
const KIND_NEGATED_AXIS: u32 = 2;
const KIND_POINT: u32 = 3;
/// The one named angle the census turns up, on seven different pairs.
const DESIGN_ANGLE: i32 = -4;

/// Axis numbering inside a rotation pair, in the order the codes count.
const PAIR_AXES: &[&str] = &["X", "Y", "Z", "-X", "-Y", "-Z"];

fn axis_letter(value: u32) -> Option<&'static str> {
    match value {
        1 => Some("X"),
        2 => Some("Y"),
        3 => Some("Z"),
        _ => None,
    }
}

fn pair_axis(index: i32) -> Option<&'static str> {
    PAIR_AXES.get(usize::try_from(index - 1).ok()?).copied()
}

/// A rotation from one axis towards another.
///
/// E3D runs the three parts together, so `-X`, `45` and `Y` print as `-X45Y`.
/// A named angle takes a space in front of it and `DDANGLE` one behind as
/// well, which is why the separators live in the angle rather than around it.
fn rotated_axes(kind: u32, angle: u32) -> Option<String> {
    let (pair, angle) = match (kind as i32, angle as i32) {
        // A negative pair states its angle in the second word.
        (pair, angle) if pair < 0 => (pair.checked_neg()?, degrees(angle)),
        (pair, DESIGN_ANGLE) if pair > 0 => (pair, " DDANGLE ".to_string()),
        (pair, parameter) if pair > 0 && parameter > 0 => (pair, format!(" PARAM {parameter}")),
        _ => return None,
    };
    let from = pair_axis(pair / 10)?;
    let towards = pair_axis(pair % 10)?;
    Some(format!("{from}{angle}{towards}"))
}

/// Hundredths of a degree, printed the way `q att` prints them.
fn degrees(hundredths: i32) -> String {
    let text = format!("{:.2}", f64::from(hundredths) / 100.0);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Render an axis specification the way E3D's `Q` command does.
///
/// The implicit reader strips the stored count before handing an array over,
/// so `-Y` arrives as `[2, 2]`; explicit storage still carries it as
/// `[2, 2, 2]`. Both forms are accepted.
pub fn render(words: &[u32]) -> Option<String> {
    render_payload(words).or_else(|| match words.split_first() {
        Some((count, rest)) if *count as usize == rest.len() => render_payload(rest),
        _ => None,
    })
}

fn render_payload(payload: &[u32]) -> Option<String> {
    match payload {
        [axis] => axis_letter(*axis).map(String::from),
        [kind, value] => match *kind {
            KIND_AXIS => axis_letter(*value).map(String::from),
            KIND_NEGATED_AXIS => axis_letter(*value).map(|letter| format!("-{letter}")),
            KIND_POINT => Some(format!("P{value}")),
            kind => rotated_axes(kind, *value),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every distinct axis tuple in the TTY sample, against E3D's own text.
    #[test]
    fn renders_sampled_axes_the_way_e3d_does() {
        let cases: &[(&[u32], &str)] = &[
            (&[1, 2], "Y"),
            (&[2, 1, 1], "X"),
            (&[2, 1, 2], "Y"),
            (&[2, 1, 3], "Z"),
            (&[2, 2, 2], "-Y"),
            (&[2, 2, 3], "-Z"),
            (&[2, 3, 1], "P1"),
            (&[2, 3, 3], "P3"),
            (&[2, 3, 5], "P5"),
            (&[2, 3, 7], "P7"),
            (&[2, 3, 30], "P30"),
            // the same specifications with the count already stripped off
            (&[2, 1], "-X"),
            (&[2, 2], "-Y"),
            (&[3, 7], "P7"),
        ];
        for (words, expected) in cases {
            assert_eq!(render(words).as_deref(), Some(*expected), "{words:?}");
        }
    }

    /// Every rotation pair the 2026-08-27 capture read back, against E3D's
    /// own text.
    #[test]
    fn renders_a_rotation_between_two_axes() {
        let cases: &[(&[u32], &str)] = &[
            (&[(-42i32) as u32, 4500], "-X45Y"),
            (&[(-15i32) as u32, 4500], "X45-Y"),
            (&[(-21i32) as u32, 4500], "Y45X"),
            (&[(-26i32) as u32, 4500], "Y45-Z"),
            // with the stored count still in front
            (&[2, (-26i32) as u32, 4500], "Y45-Z"),
        ];
        for (words, expected) in cases {
            assert_eq!(render(words).as_deref(), Some(*expected), "{words:?}");
        }
        // Hundredths, not whole degrees.
        assert_eq!(render(&[(-42i32) as u32, 2250]).as_deref(), Some("-X22.5Y"));
    }

    /// A positive pair names its angle. Every one E3D was asked to print on
    /// 2026-08-27, plus the `X DDANGLE Y` of the 2026-08-26 sample.
    #[test]
    fn renders_a_rotation_through_a_named_angle() {
        let cases: &[(&[u32], &str)] = &[
            (&[12, (-4i32) as u32], "X DDANGLE Y"),
            (&[15, (-4i32) as u32], "X DDANGLE -Y"),
            (&[16, (-4i32) as u32], "X DDANGLE -Z"),
            (&[21, (-4i32) as u32], "Y DDANGLE X"),
            (&[61, 20], "-Z PARAM 20X"),
            (&[64, 20], "-Z PARAM 20-X"),
            (&[2, 12, (-4i32) as u32], "X DDANGLE Y"),
        ];
        for (words, expected) in cases {
            assert_eq!(render(words).as_deref(), Some(*expected), "{words:?}");
        }
    }

    /// A count that does not match the words is not an axis specification at
    /// all, and no named angle but `-4` has ever been seen.
    #[test]
    fn declines_what_it_does_not_model() {
        assert_eq!(render(&[12, (-5i32) as u32]), None);
        assert_eq!(render(&[12, 0]), None);
        // No axis is numbered 0 or 7, so neither half of these is a pair.
        assert_eq!(render(&[(-7i32) as u32, 4500]), None);
        assert_eq!(render(&[(-70i32) as u32, 4500]), None);
        assert_eq!(render(&[2, 0, 10]), None);
        assert_eq!(render(&[3, 1, 1]), None);
        assert_eq!(render(&[]), None);
    }
}
