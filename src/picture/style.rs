//! Pens: the Wigwam representation numbers DRAFT assigns to line styles.
//!
//! Standard numbering (Core3D `linestyle/DFSLPN`, `deviceadc/PRNSPE`;
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
//! The colour table (365 entries read out of core.dll / DruidNet) joins this
//! module in P2 / S4.

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

#[cfg(test)]
mod tests {
    use super::*;

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
