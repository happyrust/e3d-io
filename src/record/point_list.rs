//! The point lists catalogue wireframe geometry stores for `LINE.Pts`.
//!
//! E3D prints these as `P0 P3` or `P1 T0 P2`; the database stores
//! `[0, 3, 1000]` and `[1, 100, 2, 1000]`. Read as plain numbers the two rows
//! of the TTY sample disagreed on every item.
//!
//! `1000` closes the list rather than naming a point. Every one of the 82
//! `LINE` elements in `ams7000_0001` ends its list with it, and no list holds
//! it anywhere else (`tests/scratch_line_pts.rs`). The same census shows the
//! items themselves take only the values 0, 1, 2, 3, 4 and 100 — a block of
//! small numbers and one that sits a hundred above them.
//!
//! So the hundreds digit picks the letter and the rest is the number, which is
//! what the two sampled rows print: 3 is `P3` and 100 is `T0`. Only those two
//! letters appear, and the sample fixes `T` on one row, so a third block
//! returns `None` rather than a guessed letter.

/// Closes the list. Not a point.
const END_OF_LIST: u32 = 1000;
/// Which letter each hundred stands for. `P` is a p-point of the owning
/// primitive; `T` accompanies it on the one sampled list that has both.
const LETTERS: &[&str] = &["P", "T"];

/// Render a point list the way E3D's `Q` command does.
pub fn render(words: &[u32]) -> Option<String> {
    let (&END_OF_LIST, items) = words.split_last()? else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    let mut out = String::new();
    for item in items {
        let letter = LETTERS.get((item / 100) as usize)?;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(letter);
        out.push_str(&(item % 100).to_string());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both `LINE.Pts` rows of the 2026-08-26 TTY sample, and the one list
    /// shape the database census turned up that the sample does not cover.
    #[test]
    fn renders_sampled_point_lists_the_way_e3d_does() {
        assert_eq!(render(&[0, 3, 1000]).as_deref(), Some("P0 P3"));
        assert_eq!(render(&[1, 100, 2, 1000]).as_deref(), Some("P1 T0 P2"));
        assert_eq!(render(&[2, 100, 3, 1, 1000]).as_deref(), Some("P2 T0 P3 P1"));
    }

    #[test]
    fn declines_what_it_does_not_model() {
        // No terminator, so this is not a point list.
        assert_eq!(render(&[0, 3]), None);
        // A terminator and nothing else says nothing.
        assert_eq!(render(&[1000]), None);
        // A hundreds block with no letter behind it.
        assert_eq!(render(&[200, 1000]), None);
        assert_eq!(render(&[]), None);
    }
}
