//! Pens and colours: what a primitive header's pen and colour numbers mean.
//!
//! **Pens** (Core3D `linestyle/DFSLPN`, `deviceadc/PRNSPE`;
//! `docs/evidence/2026-09-28-gw-picture-format.md` §3.3):
//!
//! * 8001–8030 = `8000 + style + 10 × (width − 1)`, style 1..10 = SOLID /
//!   DASH / DOT / CHAIN / LDASH / DDASH / DDOT / FDOT / DCHAIN / TCHAIN,
//!   width 1..3 = THIN / MEDIUM / THICK;
//! * 8031–8045 = `8030 + k + 5 × (width − 1)`, k = 1 SOLID, k = 2..5 the
//!   user line patterns LPA 8032..8035;
//! * 8000 = the fill pen; 8100–8115 system pens (8107 draws the sheet
//!   frame, 8100 / 8115 the pick-point markers).
//!
//! **Colours.** The colour number indexes DRAFT's 365-entry colour table,
//! which is not in the file: E3D builds it at start-up. `PrepareEnvironment`
//! calls `CTINIT(maxcols = 365, extcols = 10000)` — DruidNet's
//! `ColourTableManager.Init`: entry 1 black, 2 white, 3..365 white named
//! by their number — then `fmdevice/FWSCOL` fills it from static data:
//! entries 1–16 take the base names (`CTCNAM`, a name in the 61-entry
//! dictionary brings its RGB), 17–272 the 256 RGB triples (`CTCRGB`), the
//! slots `DICT_INDEX_MAP` names get the dictionary name over their RGB,
//! 305–365 the 61 dictionary names, and 296 / 298–300 / 302–303 five
//! specials. 297 / 301 / 304 are set per device at run time (`FWSQGN`) and
//! 273–295 stay the white `Init` gave them. 8-bit values are
//! `(int)(c × 255)`, truncated, as `System.Drawing.Color.FromArgb` gets
//! them. A number above 365 is a layer id + 365, not a colour
//! (`WWPrimitive::getColour` returns −4). Proof: DRAW's own thumbnails paint
//! the sheet frame (pen 8107, colour 1) in exactly (130, 130, 130) =
//! int(0.51 × 255), and nine visible colours' hue clusters lie within 1.5°
//! of the table (round 13, `draft31\gw\colourverify.py`).

use std::sync::OnceLock;

use super::colour_data::{BASE16, DICT61, DICT_INDEX_MAP, RGB256};

/// Line patterns 1..10.
pub const LINE_PATTERNS: [&str; 10] = [
    "SOLID", "DASH", "DOT", "CHAIN", "LDASH", "DDASH", "DDOT", "FDOT", "DCHAIN", "TCHAIN",
];
/// Line widths 1..3.
pub const LINE_WIDTHS: [&str; 3] = ["THIN", "MEDIUM", "THICK"];

/// What a pen number stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pen {
    /// A standard line style: width 1..3, pattern 1..10.
    Line { width: u8, pattern: u8 },
    /// A user line pattern: width 1..3, `lpa` = 8031 (SOLID) or 8032..8035.
    UserLine { width: u8, lpa: u32 },
    /// 8000.
    Fill,
    /// 8100..8199.
    System(u32),
    /// Anything else.
    Other(u32),
}

impl Pen {
    /// Classify pen number `pen`.
    pub fn of(pen: u32) -> Self {
        match pen {
            8001..=8030 => Pen::Line {
                width: ((pen - 8001) / 10) as u8 + 1,
                pattern: ((pen - 8001) % 10) as u8 + 1,
            },
            8031..=8045 => Pen::UserLine {
                width: ((pen - 8031) / 5) as u8 + 1,
                lpa: 8031 + (pen - 8031) % 5,
            },
            8000 => Pen::Fill,
            8100..=8199 => Pen::System(pen),
            other => Pen::Other(other),
        }
    }

    /// `gwdump.py pen_name`: `THIN SOLID`, `MEDIUM LPA8033`, `FILL`,
    /// `SYSTEM`, or empty for a number outside the DRAFT ranges.
    pub fn name(&self) -> String {
        match self {
            Pen::Line { width, pattern } => format!(
                "{} {}",
                LINE_WIDTHS[*width as usize - 1],
                LINE_PATTERNS[*pattern as usize - 1]
            ),
            Pen::UserLine { width, lpa } => format!(
                "{} {}",
                LINE_WIDTHS[*width as usize - 1],
                if *lpa == 8031 {
                    "SOLID".to_string()
                } else {
                    format!("LPA{lpa}")
                }
            ),
            Pen::Fill => "FILL".to_string(),
            Pen::System(_) => "SYSTEM".to_string(),
            Pen::Other(_) => String::new(),
        }
    }
}

/// [`Pen::of`] then [`Pen::name`].
pub fn pen_name(pen: u32) -> String {
    Pen::of(pen).name()
}

/// Entries in the DRAFT colour table (`CTINIT maxcols`).
pub const COLOUR_TABLE_SIZE: u16 = 365;

/// One colour-table entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Colour {
    /// The name `CTQNAM` answers: a dictionary name, or the number itself.
    pub name: String,
    /// RGB as doubles 0..1, as the table stores them.
    pub rgb: (f64, f64, f64),
}

impl Colour {
    /// The table entry for colour number `n` (1..=365); `None` for 0 and
    /// for numbers above the table, which are layer ids.
    pub fn of(n: u16) -> Option<&'static Colour> {
        if n == 0 || n > COLOUR_TABLE_SIZE {
            return None;
        }
        Some(&table()[n as usize - 1])
    }

    /// The whole table, entry `n` at index `n − 1`.
    pub fn table() -> &'static [Colour] {
        table()
    }

    /// 8-bit RGB the way `Color.FromArgb((int)(r * 255.0), …)` truncates.
    pub fn rgb8(&self) -> (u8, u8, u8) {
        let (r, g, b) = self.rgb;
        ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }

    /// `#rrggbb` of [`Colour::rgb8`].
    pub fn hex(&self) -> String {
        let (r, g, b) = self.rgb8();
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

/// 8-bit RGB of colour number `n`, `None` when it is not a colour.
pub fn colour_rgb8(n: u16) -> Option<(u8, u8, u8)> {
    Colour::of(n).map(Colour::rgb8)
}

/// The name of colour number `n`, `None` when it is not a colour.
pub fn colour_name(n: u16) -> Option<&'static str> {
    Colour::of(n).map(|c| c.name.as_str())
}

fn table() -> &'static [Colour] {
    static TABLE: OnceLock<Vec<Colour>> = OnceLock::new();
    TABLE.get_or_init(build_table)
}

/// Replay `ColourTableManager.Init` + `FWSCOL` (`draft31\gw\draft_colours.py
/// build_table`).
fn build_table() -> Vec<Colour> {
    let dictionary = |name: &str| -> Option<(f64, f64, f64)> {
        DICT61
            .iter()
            .find(|(n, _, _, _)| *n == name)
            .map(|&(_, r, g, b)| (r, g, b))
    };
    let size = COLOUR_TABLE_SIZE as usize;
    // Init: 1 black, 2 white, the rest white named by number.
    let mut table: Vec<Colour> = (1..=size)
        .map(|i| Colour {
            name: i.to_string(),
            rgb: (1.0, 1.0, 1.0),
        })
        .collect();
    table[0] = Colour {
        name: "black".to_string(),
        rgb: dictionary("black").unwrap(),
    };
    table[1] = Colour {
        name: "white".to_string(),
        rgb: dictionary("white").unwrap(),
    };
    // setColourName: a dictionary name brings its RGB; a number copies that
    // entry's RGB and keeps the given name.
    fn set_name(table: &mut [Colour], index: usize, name: &str, rgb: Option<(f64, f64, f64)>) {
        let rgb = match rgb {
            Some(rgb) => rgb,
            None => {
                let from: usize = name
                    .parse()
                    .expect("a colour name is a dictionary word or a number");
                table[from - 1].rgb
            }
        };
        table[index - 1] = Colour {
            name: name.to_string(),
            rgb,
        };
    }
    // setRGB: the entry is named by its number.
    fn set_rgb(table: &mut [Colour], index: usize, rgb: (f64, f64, f64)) {
        table[index - 1] = Colour {
            name: index.to_string(),
            rgb,
        };
    }
    for (i, name) in BASE16.iter().enumerate() {
        set_name(&mut table, i + 1, name, dictionary(name));
    }
    for (j, &rgb) in RGB256.iter().enumerate() {
        set_rgb(&mut table, 17 + j, rgb);
    }
    for (k, &m) in DICT_INDEX_MAP.iter().enumerate() {
        if m >= 0 {
            let name = DICT61[k].0;
            set_name(&mut table, m as usize + 17, name, dictionary(name));
        }
    }
    for (k, (name, _, _, _)) in DICT61.iter().enumerate() {
        set_name(&mut table, 305 + k, name, dictionary(name));
    }
    for (index, name) in [
        (303, "gold"),
        (302, "lightyellow"),
        (298, "lightyellow"),
        (300, "midnight"),
        (299, "white"),
    ] {
        set_name(&mut table, index, name, dictionary(name));
    }
    set_rgb(&mut table, 296, (0.341, 0.506, 0.682));
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_table_has_the_values_draw_paints_with() {
        assert_eq!(Colour::table().len(), 365);
        let grey = Colour::of(1).unwrap();
        assert_eq!(grey.name, "grey");
        assert_eq!(grey.rgb8(), (130, 130, 130), "int(0.51 × 255)");
        assert_eq!(grey.hex(), "#828282");
        assert_eq!(colour_name(2), Some("red"));
        assert_eq!(colour_rgb8(2), Some((188, 0, 0)));
        assert_eq!(colour_name(7), Some("blue"));
        assert_eq!(colour_rgb8(7), Some((0, 0, 255)));
        assert_eq!(colour_name(15), Some("black"));
        assert_eq!(colour_rgb8(15), Some((0, 0, 0)));
        assert_eq!(
            colour_name(17),
            Some("black"),
            "RGB256[0] carries the dictionary name"
        );
        assert_eq!(colour_name(18), Some("brightred"));
        assert_eq!(colour_name(19), Some("yellow"));
        assert_eq!(colour_name(59), Some("sandybrown"));
        assert_eq!(colour_rgb8(59), Some((188, 141, 0)));
        assert_eq!(colour_name(101), Some("forestgreen"));
        assert_eq!(colour_rgb8(101), Some((31, 129, 0)));
        assert_eq!(colour_name(109), Some("green"));
        assert_eq!(colour_rgb8(109), Some((0, 188, 0)));
        assert_eq!(colour_name(160), Some("steelblue"));
        assert_eq!(colour_rgb8(160), Some((0, 141, 188)));
        assert_eq!(colour_name(271), Some("lightgrey"));
        assert_eq!(colour_rgb8(271), Some((189, 189, 189)));
        for n in 273..=295u16 {
            assert_eq!(colour_name(n), Some(n.to_string().as_str()), "{n} is unset");
            assert_eq!(colour_rgb8(n), Some((255, 255, 255)), "{n} is white");
        }
        assert_eq!(colour_rgb8(296), Some((86, 129, 173)));
        for n in [297u16, 301, 304] {
            assert_eq!(
                colour_rgb8(n),
                Some((255, 255, 255)),
                "{n} is set per device at run time"
            );
        }
        assert_eq!(colour_name(298), Some("lightyellow"));
        assert_eq!(colour_name(299), Some("white"));
        assert_eq!(colour_name(300), Some("midnight"));
        assert_eq!(colour_rgb8(300), Some((0, 19, 79)));
        assert_eq!(colour_name(303), Some("gold"));
        assert_eq!(colour_rgb8(303), Some((255, 190, 0)));
        assert_eq!(colour_name(305), Some("black"));
        assert_eq!(colour_name(365), Some("stateblue"));
        assert_eq!(colour_rgb8(365), Some((125, 125, 188)));
        assert_eq!(Colour::of(0), None);
        assert_eq!(Colour::of(366), None, "366 is layer 1");
        assert_eq!(colour_rgb8(u16::MAX), None);
    }

    #[test]
    fn the_standard_pen_numbers_name_themselves() {
        assert_eq!(pen_name(8001), "THIN SOLID");
        assert_eq!(pen_name(8004), "THIN CHAIN");
        assert_eq!(pen_name(8005), "THIN LDASH");
        assert_eq!(pen_name(8011), "MEDIUM SOLID");
        assert_eq!(pen_name(8030), "THICK TCHAIN");
        assert_eq!(pen_name(8031), "THIN SOLID");
        assert_eq!(pen_name(8033), "THIN LPA8033");
        assert_eq!(pen_name(8036), "MEDIUM SOLID");
        assert_eq!(pen_name(8045), "THICK LPA8035");
        assert_eq!(pen_name(8000), "FILL");
        assert_eq!(pen_name(8107), "SYSTEM");
        assert_eq!(pen_name(8100), "SYSTEM");
        assert_eq!(pen_name(0), "");
        assert_eq!(pen_name(7999), "");
        assert_eq!(
            Pen::of(8014),
            Pen::Line {
                width: 2,
                pattern: 4
            }
        );
    }
}
