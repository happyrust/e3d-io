//! The direction specifications catalogue point geometry stores for
//! `PTCA.Ptcdirection`.
//!
//! E3D prints these as `Z`, as `X ( 45 ) Z`, or as a whole chain of turns:
//! `AXIS -Y ( 45 ) -X ( 45 ) Z`. The record holds a length, a two word header,
//! an opcode, and then the directions with an expression between each pair:
//!
//! ```text
//! 16 5 2 22 | 11 | 9 1 101 6 23040 0 6 0 6 | 15 | 61
//!              X       ( 45 )                 Z
//! ```
//!
//! The expression is an ordinary [`crate::record::catalogue_pml`] program,
//! count word and all — the same one that renders `( 0.5 * ATTRIB PARA[20 ] )`
//! elsewhere in the catalogue. Only the frame around it is new here.
//!
//! Codes 11 to 16 are `X -X Y -Y Z -Z`. The 2026-08-27 capture read back all
//! six, in both the leading position and the trailing one, and no other code
//! appears in any of the 43 stored values `ams7000_0001` holds. The opcode
//! counts the directions, so `21` is a bare axis and `23` is two turns.
//!
//! `AXIS` prefixes the first direction only when it points down an axis
//! backwards: E3D writes `AXIS -X ( 45 ) Y` but plain `X ( 45 ) Y`, and a
//! negative direction later in the chain stays bare.
//!
//! An expression this cannot read makes the whole specification `None`. Half a
//! chain of turns would be a different direction, not a partial one.

/// Closes the specification. Outside the range any direction code can take.
const TERMINATOR: u32 = 61;
/// Word 2 of every sampled specification. Word 1 is 5 or 6 and does not change
/// what E3D prints — `16 5 2 22 …` and `16 6 2 22 …` print the same text.
const HEADER_WORD: u32 = 2;
const LEADING_WORDS: [u32; 2] = [5, 6];
/// The opcode is this plus the number of directions that follow.
const OPCODE_BASE: u32 = 20;
/// Direction codes count from here, in pairs of axis and reversed axis.
const FIRST_DIRECTION: u32 = 11;
const DIRECTIONS: &[&str] = &["X", "-X", "Y", "-Y", "Z", "-Z"];

fn direction(code: u32) -> Option<&'static str> {
    DIRECTIONS
        .get(usize::try_from(code.checked_sub(FIRST_DIRECTION)?).ok()?)
        .copied()
}

/// Render a direction specification the way E3D's `Q` command does.
pub fn render(words: &[u32]) -> Option<String> {
    let [declared, leading, HEADER_WORD, opcode, body @ ..] = words else {
        return None;
    };
    if *declared as usize != words.len() || !LEADING_WORDS.contains(leading) {
        return None;
    }
    let turns = opcode.checked_sub(OPCODE_BASE)?.checked_sub(1)?;
    let (&TERMINATOR, body) = body.split_last()? else {
        return None;
    };

    let (first, mut rest) = body.split_first()?;
    let first = direction(*first)?;
    // Only a leading reversed axis is announced.
    let mut out = match first.starts_with('-') {
        true => format!("AXIS {first}"),
        false => first.to_string(),
    };
    for _ in 0..turns {
        let length = *rest.first()? as usize;
        let program = rest.get(..length)?;
        let program: Vec<i32> = program.iter().map(|word| *word as i32).collect();
        out.push(' ');
        out.push_str(&crate::record::catalogue_pml::render(&program)?);
        rest = rest.get(length..)?;
        let (next, tail) = rest.split_first()?;
        out.push(' ');
        out.push_str(direction(*next)?);
        rest = tail;
    }
    rest.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both `Ptcdirection` rows of the 2026-08-26 TTY sample and the shapes
    /// the 2026-08-27 capture added, against E3D's own text.
    #[test]
    fn renders_sampled_directions_the_way_e3d_does() {
        let cases: &[(&[u32], &str)] = &[
            // A bare axis, in both header spellings.
            (&[6, 5, 2, 21, 15, 61], "Z"),
            (&[6, 5, 2, 21, 16, 61], "AXIS -Z"),
            (&[6, 6, 2, 21, 11, 61], "X"),
            (&[6, 6, 2, 21, 12, 61], "AXIS -X"),
            // One turn. All six codes appear at one end or the other.
            (
                &[16, 5, 2, 22, 11, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 15, 61],
                "X ( 45 ) Z",
            ),
            (
                &[16, 5, 2, 22, 11, 9, 1, 101, 6, 23040, 0, 5, 0, 6, 13, 61],
                "X ( 22.5 ) Y",
            ),
            (
                &[16, 5, 2, 22, 11, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 14, 61],
                "X ( 45 ) -Y",
            ),
            (
                &[16, 5, 2, 22, 13, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 12, 61],
                "Y ( 45 ) -X",
            ),
            (
                &[16, 5, 2, 22, 12, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 15, 61],
                "AXIS -X ( 45 ) Z",
            ),
            (
                &[16, 5, 2, 22, 14, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 12, 61],
                "AXIS -Y ( 45 ) -X",
            ),
            (
                &[16, 6, 2, 22, 11, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 13, 61],
                "X ( 45 ) Y",
            ),
            // The double-flagged number shape, and a fraction of a degree.
            (
                &[
                    16, 5, 2, 22, 11, 9, 1, 101, 6, 1966080, 0, 1073742853, 0, 6, 16, 61,
                ],
                "X ( 120 ) -Z",
            ),
            (
                &[
                    16, 5, 2, 22, 11, 9, 1, 101, 6, 30719, 22720, 5, 0, 6, 13, 61,
                ],
                "X ( 29.999701 ) Y",
            ),
            // Two turns.
            (
                &[
                    26, 5, 2, 23, 13, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 12, 9, 1, 101, 6, 23040, 0,
                    6, 0, 6, 15, 61,
                ],
                "Y ( 45 ) -X ( 45 ) Z",
            ),
            (
                &[
                    26, 5, 2, 23, 11, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 13, 9, 1, 101, 6, 17920,
                    1664, 6, 0, 6, 15, 61,
                ],
                "X ( 45 ) Y ( 35.000099 ) Z",
            ),
            // A turn through a whole formula rather than a fixed angle.
            (
                &[
                    56, 5, 2, 22, 12, 49, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289,
                    4294967295, 4294967295, 0, 1601, 1701, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2,
                    564289, 4294967295, 4294967295, 0, 1601, 1701, 101, 6, 1835008, 0, 1073742849,
                    0, 6, 106, 2, 564289, 4294967295, 4294967295, 0, 1601, 1701, 802, 907, 16, 61,
                ],
                "AXIS -X ( ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] + ATTRIB PARA[7 ] ) ) -Z",
            ),
            (
                &[
                    64, 5, 2, 22, 13, 57, 1, 101, 6, 1048576, 0, 1073742849, 0, 6, 106, 2, 564289,
                    4294967295, 4294967295, 0, 1601, 1701, 101, 6, 0, 0, 1073742848, 0, 6, 106, 2,
                    564289, 4294967295, 4294967295, 0, 1601, 1701, 101, 6, 1887436, 3435973837,
                    1073742847, 0, 6, 101, 6, 1835008, 0, 1073742849, 0, 6, 106, 2, 564289,
                    4294967295, 4294967295, 0, 1601, 1701, 804, 803, 907, 16, 61,
                ],
                "Y ( ATANT ( ATTRIB PARA[4 ] , ATTRIB PARA[2 ] - 1.8 * ATTRIB PARA[7 ] ) ) -Z",
            ),
        ];
        for (words, expected) in cases {
            assert_eq!(render(words).as_deref(), Some(*expected), "{words:?}");
        }
    }

    #[test]
    fn declines_what_it_does_not_model() {
        // A length that does not match the words.
        assert_eq!(render(&[7, 5, 2, 21, 15, 61]), None);
        // No terminator.
        assert_eq!(render(&[5, 5, 2, 21, 15]), None);
        // A direction code outside the six.
        assert_eq!(render(&[6, 5, 2, 21, 17, 61]), None);
        assert_eq!(render(&[6, 5, 2, 21, 10, 61]), None);
        // An opcode that counts more directions than the body holds.
        assert_eq!(render(&[6, 5, 2, 22, 15, 61]), None);
        // An opcode that counts fewer, leaving words no one read.
        assert_eq!(
            render(&[16, 5, 2, 21, 11, 9, 1, 101, 6, 23040, 0, 6, 0, 6, 15, 61]),
            None
        );
        // A turn whose expression this cannot read stops the whole chain.
        assert_eq!(
            render(&[16, 5, 2, 22, 11, 9, 1, 999, 6, 23040, 0, 6, 0, 6, 15, 61]),
            None
        );
        assert_eq!(render(&[]), None);
        // A catalogue tuple and a point list must not be claimed here.
        assert_eq!(render(&[4, 40, 1, 2, 0]), None);
        assert_eq!(render(&[0, 3, 1000]), None);
    }
}
