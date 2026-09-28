//! Text attributes: the mixed-radix `attr` word of TEXT / TEXT2 primitives
//! and the escape grammar TEXT2 runs are cut from.
//!
//! `GQUTXS` (core.dll `0x54C09F8`) unpacks one integer into four font
//! numbers, a direction, a horizontal and a vertical justification
//! (`docs/evidence/2026-09-28-gw-picture-format.md` §3.2):
//!
//! ```text
//! attr = (f4 − 32)·47185920 + f3·737280 + f2·11520 + f1·180 + dir·36 + hj·6 + vj
//! ```
//!
//! A negative word first gets `0x5A000000` (= 32 × 47185920) added; a
//! non-negative one gets 32 added to `f4` after the division. The three
//! small fields index the 9-character tables that follow
//! `"TEXT  MARKERLINEARFILL  "` in core.dll's `.rdata`, and the whole is
//! spelled the way `WWTextFormat::fromString` wants it:
//! `"FONT    11,RIGHT,CENTRE,HBODY"`.
//!
//! TEXT2 (kind 10) keeps its string as runs `[n][font][style][newline][z]`
//! followed by the characters. The writer (`sub_54F1B00`) cut the DRAFT
//! source string at the escape character (Wigwam text attribute 26, `&` in
//! every sample): `EB Eb` / `EI Ei` / `EU E:`·`Eu E;` / `EX Ex` toggle style
//! bits 1 / 2 / 4 / 8, `ER` clears them all, `E/` is a newline, `E>` / `E<`
//! push a super- / subscript (`z = 4z + 1` / `4z + 2`), `E=` pops, `E*`
//! clears, `EO` / `EN` select fonts f1 / f2, `E<d>c` sets one character in
//! font f(d+1), `EE` is a literal. [`source_from_runs`] rebuilds that source
//! the way the reader `sub_54F39D9` does, emitting the delta escapes before
//! each run.

use std::fmt;

/// `dir` 1..4.
pub const TEXT_DIR: [&str; 4] = ["RIGHT", "LEFT", "DOWN", "UP"];
/// `hj` 1..5.
pub const TEXT_HJUST: [&str; 5] = ["LBODY", "RBODY", "CENTRE", "LKERN", "RKERN"];
/// `vj` 1..5.
pub const TEXT_VJUST: [&str; 5] = ["BASE", "TBODY", "BBODY", "HHEIGHT", "HBODY"];

const RADIX_F4: i64 = 47_185_920;
const RADIX_F3: i64 = 737_280;
const RADIX_F2: i64 = 11_520;
const RADIX_F1: i64 = 180;
const RADIX_DIR: i64 = 36;
const RADIX_HJ: i64 = 6;
const NEGATIVE_BIAS: i64 = 0x5A00_0000;

/// The fields of a text attribute word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextFormat {
    /// Font numbers f1..f4; zero means unused.
    pub fonts: [i32; 4],
    /// Text direction, 1..4 into [`TEXT_DIR`].
    pub dir: i32,
    /// Horizontal justification, 1..5 into [`TEXT_HJUST`].
    pub hjust: i32,
    /// Vertical justification, 1..5 into [`TEXT_VJUST`].
    pub vjust: i32,
}

impl TextFormat {
    /// Unpack `attr` exactly as `GQUTXS` does.
    pub fn decode(attr: i32) -> Self {
        let mut v = attr as i64;
        if v < 0 {
            v += NEGATIVE_BIAS;
        }
        let mut f4 = v.div_euclid(RADIX_F4);
        v = v.rem_euclid(RADIX_F4);
        if attr >= 0 {
            f4 += 32;
        }
        let f3 = v / RADIX_F3;
        v %= RADIX_F3;
        let f2 = v / RADIX_F2;
        v %= RADIX_F2;
        let f1 = v / RADIX_F1;
        v %= RADIX_F1;
        let dir = v / RADIX_DIR;
        v %= RADIX_DIR;
        let hjust = v / RADIX_HJ;
        let vjust = v % RADIX_HJ;
        Self {
            fonts: [f1 as i32, f2 as i32, f3 as i32, f4 as i32],
            dir: dir as i32,
            hjust: hjust as i32,
            vjust: vjust as i32,
        }
    }

    /// Pack the fields back into the attribute word (inverse of
    /// [`TextFormat::decode`]).
    pub fn encode(&self) -> i32 {
        let [f1, f2, f3, f4] = self.fonts.map(i64::from);
        let v = (f4 - 32) * RADIX_F4
            + f3 * RADIX_F3
            + f2 * RADIX_F2
            + f1 * RADIX_F1
            + i64::from(self.dir) * RADIX_DIR
            + i64::from(self.hjust) * RADIX_HJ
            + i64::from(self.vjust);
        v as i32
    }

    /// The direction's name, or `None` when the index is out of the table.
    pub fn dir_name(&self) -> Option<&'static str> {
        table_name(&TEXT_DIR, self.dir)
    }

    /// The horizontal justification's name.
    pub fn hjust_name(&self) -> Option<&'static str> {
        table_name(&TEXT_HJUST, self.hjust)
    }

    /// The vertical justification's name.
    pub fn vjust_name(&self) -> Option<&'static str> {
        table_name(&TEXT_VJUST, self.vjust)
    }
}

fn table_name(table: &'static [&'static str], index: i32) -> Option<&'static str> {
    usize::try_from(index)
        .ok()
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| table.get(i).copied())
}

/// The `WWTextFormat::fromString` spelling `GQUTXS` builds: `FONT` + each
/// non-zero font right-justified in 6 columns + `,DIR,HJUST,VJUST`, an
/// out-of-table index written `,?n`.
impl fmt::Display for TextFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FONT")?;
        for font in self.fonts.iter().filter(|&&font| font > 0) {
            write!(f, "{font:6}")?;
        }
        for (name, index) in [
            (self.dir_name(), self.dir),
            (self.hjust_name(), self.hjust),
            (self.vjust_name(), self.vjust),
        ] {
            match name {
                Some(name) => write!(f, ",{name}")?,
                None => write!(f, ",?{index}")?,
            }
        }
        Ok(())
    }
}

/// One TEXT2 run: a stretch of characters sharing font, style, line and
/// script level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Character count `n` (the run's first word).
    pub n: i32,
    /// Font number in force.
    pub font: i32,
    /// Style bits: 1 bold, 2 italic, 4 underline, 8 `X`.
    pub style: i32,
    /// Newline escapes (`E/`) immediately before this run.
    pub newline: i32,
    /// Super- / subscript stack (`E>` → `4z + 1`, `E<` → `4z + 2`).
    pub z: i32,
    /// The `n` character codes, one per 4-byte unit.
    pub codes: Vec<u32>,
    /// The unit after the last character when `n` is odd (the writer left
    /// 0 there), kept so the run encodes back exactly.
    pub pad: Vec<u32>,
}

/// Style letters in bit order, as the escapes spell them on / off.
pub const STYLE_ON: [char; 4] = ['B', 'I', 'U', 'X'];
pub const STYLE_OFF: [char; 4] = ['b', 'i', 'u', 'x'];

impl Run {
    /// The characters as a string; a code that is not a scalar value
    /// becomes U+FFFD.
    pub fn text(&self) -> String {
        codes_to_string(&self.codes)
    }

    /// `B+I+U+X` for the bits set, `-` for none — `gwdump.py style_name`.
    pub fn style_name(&self) -> String {
        style_name(self.style)
    }
}

/// `B+I` and the like for a style word, `-` when no bit is set.
pub fn style_name(style: i32) -> String {
    let names: Vec<String> = STYLE_ON
        .iter()
        .enumerate()
        .filter(|(k, _)| (style >> k) & 1 == 1)
        .map(|(_, c)| c.to_string())
        .collect();
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join("+")
    }
}

/// Characters from one-per-unit codes, U+FFFD for anything that is not a
/// scalar value.
pub fn codes_to_string(codes: &[u32]) -> String {
    codes
        .iter()
        .map(|&c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Rebuild the escaped DRAFT source string from `runs` the way the reader
/// `sub_54F39D9` does: start from font f1 of `attr` with no style and no
/// script, and before each run emit the escapes that take the previous
/// state to the run's. `esc` is the escape character (`&` in every sample).
pub fn source_from_runs(attr: i32, runs: &[Run], esc: char) -> String {
    let fonts = TextFormat::decode(attr).fonts;
    let mut prev_font = fonts[0];
    let mut prev_style = 0i32;
    let mut prev_z = 0i32;
    let mut out = String::new();
    let push_esc = |out: &mut String, c: char| {
        out.push(esc);
        out.push(c);
    };
    for run in runs {
        for _ in 0..run.newline.max(0) {
            push_esc(&mut out, '/');
        }
        if run.newline != 0 {
            prev_z = 0;
        }
        if run.z != prev_z {
            let c = if run.z == 0 {
                '*'
            } else if run.z % 2 != 0 {
                '>'
            } else {
                '<'
            };
            push_esc(&mut out, c);
        }
        let changed = run.style ^ prev_style;
        for (k, &off) in STYLE_OFF.iter().enumerate() {
            if (changed >> k) & 1 == 1 && (run.style >> k) & 1 == 0 {
                push_esc(&mut out, off);
            }
        }
        let mut font = run.font;
        if font != prev_font {
            let m = fonts.iter().position(|&f| f == font).map_or(5, |i| i + 1);
            match m {
                1 => push_esc(&mut out, 'O'),
                2 => push_esc(&mut out, 'N'),
                3 | 4 => {
                    // `E2c` / `E3c` set one character in f3 / f4; doubled
                    // they persist. A one-character run does not carry the
                    // font forward.
                    let code = char::from_u32(m as u32 + 47).expect("'2' or '3'");
                    if run.n <= 1 {
                        font = prev_font;
                        push_esc(&mut out, code);
                    } else {
                        push_esc(&mut out, code);
                        push_esc(&mut out, code);
                    }
                }
                _ => {}
            }
        }
        for (k, &on) in STYLE_ON.iter().enumerate() {
            if (changed >> k) & 1 == 1 && (run.style >> k) & 1 == 1 {
                push_esc(&mut out, on);
            }
        }
        for c in run.text().chars() {
            if c == esc {
                out.push(esc);
            }
            out.push(c);
        }
        prev_font = font;
        prev_style = run.style;
        prev_z = run.z;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_attribute_words_unpack_as_gqutxs_spells_them() {
        // The Marine title-block TEXT: box x0 = −x1, y0 = −y1 ↔ CENTRE / HBODY.
        let format = TextFormat::decode(-1_509_947_401);
        assert_eq!(format.fonts, [11, 0, 0, 0]);
        assert_eq!((format.dir, format.hjust, format.vjust), (1, 3, 5));
        assert_eq!(format.to_string(), "FONT    11,RIGHT,CENTRE,HBODY");
        assert_eq!(format.encode(), -1_509_947_401);

        assert_eq!(
            TextFormat::decode(-1_509_948_495).to_string(),
            "FONT     5,RIGHT,LBODY,BBODY"
        );
        assert_eq!(
            TextFormat::decode(-1_509_948_483).to_string(),
            "FONT     5,RIGHT,CENTRE,BBODY"
        );
        assert_eq!(
            TextFormat::decode(-1_509_948_497).to_string(),
            "FONT     5,RIGHT,LBODY,BASE"
        );

        // A non-negative word: f4 carries the +32.
        let positive = TextFormat {
            fonts: [3, 7, 0, 33],
            dir: 2,
            hjust: 1,
            vjust: 4,
        };
        let attr = positive.encode();
        assert!(attr >= 0);
        assert_eq!(TextFormat::decode(attr), positive);
        assert_eq!(
            positive.to_string(),
            "FONT     3     7    33,LEFT,LBODY,HHEIGHT"
        );

        // Out-of-table indices are written `?n`, as gwdump does.
        let odd = TextFormat {
            fonts: [1, 0, 0, 0],
            dir: 0,
            hjust: 6,
            vjust: 5,
        };
        assert_eq!(odd.to_string(), "FONT     1,?0,?6,HBODY");
    }

    fn run(n: i32, font: i32, style: i32, newline: i32, z: i32, text: &str) -> Run {
        let codes: Vec<u32> = text.chars().map(|c| c as u32).collect();
        Run {
            n,
            font,
            style,
            newline,
            z,
            codes,
            pad: if n % 2 == 1 { vec![0] } else { vec![] },
        }
    }

    #[test]
    fn a_single_plain_run_is_its_own_source() {
        let attr = -1_509_948_495; // font 5
        let runs = [run(8, 5, 0, 0, 0, "?drawno?")];
        assert_eq!(source_from_runs(attr, &runs, '&'), "?drawno?");
        assert_eq!(runs[0].style_name(), "-");
    }

    #[test]
    fn escapes_are_emitted_as_deltas_before_each_run() {
        let attr = -1_509_948_495; // f1 = 5
                                   // 'SHOP&/TEST': a newline between two runs of the same font.
        let runs = [run(4, 5, 0, 0, 0, "SHOP"), run(4, 5, 0, 1, 0, "TEST")];
        assert_eq!(source_from_runs(attr, &runs, '&'), "SHOP&/TEST");

        // '&B&IP': bold and italic switched on before a one-character run.
        let runs = [run(1, 5, 3, 0, 0, "P")];
        assert_eq!(source_from_runs(attr, &runs, '&'), "&B&IP");
        assert_eq!(runs[0].style_name(), "B+I");

        // '&USEC.(Right)': underline on.
        let runs = [run(12, 5, 4, 0, 0, "SEC.(Right)")];
        assert_eq!(source_from_runs(attr, &runs, '&'), "&USEC.(Right)");

        // Style switched off again, a literal escape doubled, fonts f2 and f3.
        let fonts = TextFormat {
            fonts: [5, 7, 9, 0],
            dir: 1,
            hjust: 1,
            vjust: 1,
        }
        .encode();
        let runs = [
            run(2, 5, 1, 0, 0, "ab"),
            run(3, 7, 0, 0, 0, "c&d"),
            run(1, 9, 0, 0, 0, "e"),
            run(2, 5, 0, 0, 0, "fg"),
        ];
        assert_eq!(source_from_runs(fonts, &runs, '&'), "&Bab&b&Nc&&d&2e&Ofg");
        // f3 for more than one character persists: doubled escape.
        let runs = [run(2, 9, 0, 0, 0, "ee")];
        assert_eq!(source_from_runs(fonts, &runs, '&'), "&2&2ee");

        // Super- and subscript: push, then clear.
        let runs = [
            run(1, 5, 0, 0, 0, "m"),
            run(1, 5, 0, 0, 1, "2"),
            run(1, 5, 0, 0, 0, "x"),
            run(1, 5, 0, 0, 2, "i"),
        ];
        assert_eq!(source_from_runs(attr, &runs, '&'), "m&>2&*x&<i");
    }
}
