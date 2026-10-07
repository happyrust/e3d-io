//! Segments (directory element type 3): header flags, transform, user block,
//! and the groups of primitives that follow.
//!
//! ```text
//! word 1          u0 = 0x70000000 | flags (kind 14), u1 = hdrlen | hi << 16
//! words 2..7      flags & 0x08: transform (a, b, c, d, tx, ty)
//! (+4 words)      flags & 0x10: four words (never seen in the samples)
//! (+1+n words)    flags & 0x04: user block [n+1][n words]
//! word hdrlen+1…  groups until the element's last word
//! ```
//!
//! Flags (`GQUSGA`): bit 0 visible, bit 1 pickable, bit 2 has user block,
//! bit 3 has transform, bit 4 has the four extra words, bit 5 has a DB
//! reference (the directory entry's `ref` is then non-zero). A point `(x, y)`
//! in a segment is placed by `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`,
//! and transforms compose down the tree.
//!
//! The user block is application data — the DRAFT noun the segment stands
//! for and the refno of the element (`docs/evidence/2026-09-28-gw-picture-format.md`
//! §3.1). Its first word is a Dabacon base-27 noun hash (`SHEE`, `VIEW`,
//! `DESI`, `GEOM`, `PPTS`, `PLIN`, `TEXP`, `BACK`, …). `GEOM` and `PPTS`
//! blocks go on to list the members drawn in each group, group id `k` being
//! the `k`-th member. Marine `TEXP` blocks carry 8 reals, so the block is
//! kept as raw words and encoded back from them.

use e3d_attlib::db1_dehash;

use super::prim::{kind, Primitive};
use super::word::{Word, Words};
use super::PictureError;

/// Segment header flag bits.
pub mod flags {
    pub const VISIBLE: u32 = 0x01;
    pub const PICKABLE: u32 = 0x02;
    pub const USER_BLOCK: u32 = 0x04;
    pub const TRANSFORM: u32 = 0x08;
    pub const EXTRA4: u32 = 0x10;
    pub const DB_REFERENCE: u32 = 0x20;
}

/// The first word of a user block: a noun hash, or a number that is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// A Dabacon noun name (`db1_dehash` of the word).
    Noun(String),
    /// A word outside the base-27 noun range.
    Other(i32),
}

impl Role {
    /// The role of hash word `word`.
    pub fn from_word(word: i32) -> Self {
        match db1_dehash(word as u32) {
            Some(name) => Role::Noun(name),
            None => Role::Other(word),
        }
    }

    /// True when this is the noun `name` (`SHEE`, `GEOM`, …).
    pub fn is(&self, name: &str) -> bool {
        matches!(self, Role::Noun(n) if n == name)
    }

    /// The noun name, or `#<word>` — `gwdump.py`'s spelling.
    pub fn label(&self) -> String {
        match self {
            Role::Noun(name) => name.clone(),
            Role::Other(word) => format!("#{word}"),
        }
    }
}

/// One member a `GEOM` / `PPTS` segment lists: the element drawn by group
/// `k` is member `k` (1-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The member's noun, or the raw word as a string when it is not one
    /// (`gwdump.py`: `dehash(noun) or str(noun)`).
    pub noun: String,
    pub w0: i32,
    pub w1: i32,
}

/// A Marine `TEXP` (2D text annotation) user block's extra fields.
#[derive(Clone, Debug, PartialEq)]
pub struct TexpNote {
    /// The text box's four corners `(x, y)`.
    pub corners: [(f64, f64); 4],
    /// The seven integers after the corners: `[1][k][k][k][k][0][nw]`.
    pub ints: [i32; 7],
    /// The length-prefixed string that follows (`nw` words, 4 chars each).
    pub text: String,
}

/// The user block of a segment.
#[derive(Clone, Debug, PartialEq)]
pub struct UserBlock {
    /// The `n` words after the length word, as stored.
    pub raw: Vec<Word>,
    /// The first word as a role.
    pub role: Role,
    /// Words 2 and 3 as `(w0, w1)`, when the block has them.
    pub reference: Option<(i32, i32)>,
    /// `GEOM` / `PPTS` members, group id `k` ↔ `members[k − 1]`.
    pub members: Vec<Member>,
    /// `TEXP` layout, when the role is `TEXP` and the block fits it.
    pub texp: Option<TexpNote>,
}

impl UserBlock {
    fn parse(raw: Vec<Word>) -> Self {
        let ints: Vec<i32> = raw.iter().map(|w| w.as_int()).collect();
        let role = Role::from_word(ints.first().copied().unwrap_or(0));
        let reference = (ints.len() >= 3).then(|| (ints[1], ints[2]));
        let mut members = Vec::new();
        if role.is("GEOM") && ints.len() >= 6 {
            for &[noun, w0, w1] in ints[3..].as_chunks::<3>().0 {
                members.push(Member {
                    noun: noun_or_number(noun),
                    w0,
                    w1,
                });
            }
        } else if role.is("PPTS") && ints.len() >= 7 {
            for &[w0, w1, noun, _zero] in ints[3..].as_chunks::<4>().0 {
                members.push(Member {
                    noun: noun_or_number(noun),
                    w0,
                    w1,
                });
            }
        }
        let texp = if role.is("TEXP") {
            TexpNote::parse(&raw)
        } else {
            None
        };
        Self {
            raw,
            role,
            reference,
            members,
            texp,
        }
    }

    /// The member drawn by group `id` (1-based), if listed.
    pub fn member(&self, id: u32) -> Option<&Member> {
        usize::try_from(id)
            .ok()
            .and_then(|k| k.checked_sub(1))
            .and_then(|k| self.members.get(k))
    }

    /// The `(w0, w1)` reference when it is not `0/0`.
    pub fn nonzero_reference(&self) -> Option<(i32, i32)> {
        self.reference.filter(|&(w0, w1)| w0 != 0 || w1 != 0)
    }

    /// `[n+1][n words]` as written.
    pub fn encode(&self) -> Vec<Word> {
        let mut out = Vec::with_capacity(self.raw.len() + 1);
        out.push(Word::int(self.raw.len() as i32 + 1));
        out.extend_from_slice(&self.raw);
        out
    }
}

fn noun_or_number(word: i32) -> String {
    db1_dehash(word as u32).unwrap_or_else(|| word.to_string())
}

impl TexpNote {
    /// `[TEXP][w0][w1] real × 8, [1][k][k][k][k][0][nw], nw words` — the
    /// layout every Marine sample shows; `None` when the block is shorter.
    fn parse(raw: &[Word]) -> Option<Self> {
        if raw.len() < 3 + 8 + 7 {
            return None;
        }
        let mut corners = [(0.0, 0.0); 4];
        for (i, corner) in corners.iter_mut().enumerate() {
            *corner = (raw[3 + 2 * i].as_real(), raw[4 + 2 * i].as_real());
        }
        let mut ints = [0i32; 7];
        for (i, slot) in ints.iter_mut().enumerate() {
            *slot = raw[11 + i].as_int();
        }
        let nw = usize::try_from(ints[6]).ok()?;
        if 18 + nw > raw.len() {
            return None;
        }
        let mut packed = Vec::with_capacity(nw * 4);
        for word in &raw[18..18 + nw] {
            packed.extend_from_slice(&word.u1.to_be_bytes());
        }
        let n = packed.first().copied().unwrap_or(0) as usize;
        let text = packed
            .get(1..1 + n)
            .unwrap_or(&packed[1.min(packed.len())..])
            .iter()
            .map(|&b| b as char)
            .collect();
        Some(Self {
            corners,
            ints,
            text,
        })
    }
}

/// A group of primitives. A primitive the writer left outside any group
/// (its header sits where a group header should) is wrapped in a group with
/// no `id`, as `gwdump.py` does.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Group id from the header's payload; `None` for a bare primitive.
    pub id: Option<u32>,
    /// 1-based word of the group header (or of the bare primitive).
    pub pos: usize,
    /// Words including the header (the primitive's length for a bare one).
    pub len: u16,
    /// The header's high 16 bits (0 for a bare primitive).
    pub extra: u16,
    pub prims: Vec<Primitive>,
}

impl Group {
    /// The group header word, `None` for a bare primitive.
    pub fn encode_header(&self) -> Option<Word> {
        self.id
            .map(|id| Word::header(kind::GROUP, id, self.len, self.extra))
    }
}

/// One segment's decoded contents.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// The header's kind field (14).
    pub kind: u8,
    /// `GQUSGA` flag bits (see [`flags`]).
    pub flags: u32,
    /// Header length in words as the header states it.
    pub hdrlen: u16,
    /// The header's high 16 bits.
    pub hdr_extra: u16,
    /// `(a, b, c, d, tx, ty)` when `flags & TRANSFORM`.
    pub transform: Option<[f64; 6]>,
    /// The four extra words when `flags & EXTRA4`.
    pub extra4: Option<[Word; 4]>,
    /// The user block when `flags & USER_BLOCK`.
    pub user: Option<UserBlock>,
    /// Words the header actually occupied: header word + transform + extra
    /// + user block (`gwdump.py header_words`).
    pub header_words: usize,
    /// Total words of the element (from the directory).
    pub nwords: usize,
    pub groups: Vec<Group>,
}

impl Segment {
    /// Decode the segment whose `nwords` words start at word 1 of `words`.
    pub fn parse(words: &Words<'_>, nwords: usize) -> Result<Self, PictureError> {
        let header = words.get(1)?;
        let kind = header.kind();
        let flags = header.payload();
        let hdrlen = header.length();
        let hdr_extra = header.extra();
        let mut p = 2usize;
        let transform = if flags & flags::TRANSFORM != 0 {
            let t = [
                words.real(p)?,
                words.real(p + 1)?,
                words.real(p + 2)?,
                words.real(p + 3)?,
                words.real(p + 4)?,
                words.real(p + 5)?,
            ];
            p += 6;
            Some(t)
        } else {
            None
        };
        let extra4 = if flags & flags::EXTRA4 != 0 {
            let e = [
                words.get(p)?,
                words.get(p + 1)?,
                words.get(p + 2)?,
                words.get(p + 3)?,
            ];
            p += 4;
            Some(e)
        } else {
            None
        };
        let user = if flags & flags::USER_BLOCK != 0 {
            let n = usize::try_from(words.int(p)? - 1).map_err(|_| PictureError::BadSegment {
                rec: words.rec(),
                word: p,
                reason: "user block length below 1".to_string(),
            })?;
            let raw = words.slice(p + 1, n)?;
            p += 1 + n;
            Some(UserBlock::parse(raw))
        } else {
            None
        };
        let header_words = p - 1;

        let mut groups = Vec::new();
        let mut pos = hdrlen as usize + 1;
        while pos <= nwords {
            let head = words.get(pos)?;
            let glen = head.length() as usize;
            if glen == 0 {
                return Err(PictureError::BadSegment {
                    rec: words.rec(),
                    word: pos,
                    reason: "zero-length group".to_string(),
                });
            }
            if head.kind() == kind::GROUP {
                let mut prims = Vec::new();
                let mut q = pos + 1;
                while q < pos + glen {
                    let prim = Primitive::decode(words, q)?;
                    q += prim.length as usize;
                    prims.push(prim);
                }
                groups.push(Group {
                    id: Some(head.payload()),
                    pos,
                    len: head.length(),
                    extra: head.extra(),
                    prims,
                });
            } else {
                groups.push(Group {
                    id: None,
                    pos,
                    len: head.length(),
                    extra: 0,
                    prims: vec![Primitive::decode(words, pos)?],
                });
            }
            pos += glen;
        }

        Ok(Self {
            kind,
            flags,
            hdrlen,
            hdr_extra,
            transform,
            extra4,
            user,
            header_words,
            nwords,
            groups,
        })
    }

    /// The header as written: header word, transform, extra words, user
    /// block — [`Segment::header_words`] words.
    pub fn encode_header(&self) -> Vec<Word> {
        let mut out = Vec::with_capacity(self.header_words);
        out.push(Word::header(
            self.kind,
            self.flags,
            self.hdrlen,
            self.hdr_extra,
        ));
        if let Some(t) = &self.transform {
            out.extend(t.iter().map(|&v| Word::real(v)));
        }
        if let Some(e) = &self.extra4 {
            out.extend_from_slice(e);
        }
        if let Some(user) = &self.user {
            out.extend(user.encode());
        }
        out
    }

    /// `flags & VISIBLE`: whether DRAW draws the segment.
    pub fn is_visible(&self) -> bool {
        self.flags & flags::VISIBLE != 0
    }

    /// The user block's role, if any.
    pub fn role(&self) -> Option<&Role> {
        self.user.as_ref().map(|u| &u.role)
    }

    /// `gwdump.py Segment.label`: `seg` without a user block, else the role,
    /// with `=w0/w1` when the reference is not `0/0`.
    pub fn label(&self) -> String {
        match &self.user {
            None => "seg".to_string(),
            Some(user) => match user.nonzero_reference() {
                Some((w0, w1)) => format!("{}={w0}/{w1}", user.role.label()),
                None => user.role.label(),
            },
        }
    }

    /// Every primitive with its group, in file order.
    pub fn prims(&self) -> impl Iterator<Item = (&Group, &Primitive)> + '_ {
        self.groups
            .iter()
            .flat_map(|group| group.prims.iter().map(move |prim| (group, prim)))
    }

    /// Primitives in the segment.
    pub fn prim_count(&self) -> usize {
        self.groups.iter().map(|g| g.prims.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::word::to_bytes;
    use e3d_attlib::db1_hash;

    fn h(name: &str) -> i32 {
        db1_hash(name) as i32
    }

    #[test]
    fn the_sheet_segment_of_the_small_sample_parses_header_user_block_and_frame() {
        // SEG#0: flags 0x7, hdrlen 6 (header + [4][SHEE][w0][w1][0]), one
        // group (id 9) holding the 11-word frame LINE — 18 words.
        let mut words = vec![Word::header(kind::SEGMENT, 0x7, 6, 0)];
        words.push(Word::int(5));
        words.extend([h("SHEE"), 2013286677, 4350, 0].map(Word::int));
        words.push(Word::header(kind::GROUP, 9, 12, 0));
        words.push(Word::header(kind::LINE, 8107, 11, 1));
        for &(x, y) in &[
            (0.0, 0.0),
            (210.0, 0.0),
            (210.0, 297.0),
            (0.0, 297.0),
            (0.0, 0.0),
        ] {
            words.push(Word::real(x));
            words.push(Word::real(y));
        }
        assert_eq!(words.len(), 18);
        let bytes = to_bytes(&words);
        let seg = Segment::parse(&Words::new(&bytes, 15), 18).unwrap();
        assert_eq!(seg.flags, 0x7);
        assert!(seg.is_visible());
        assert_eq!(seg.transform, None);
        assert_eq!(seg.header_words, 6);
        let user = seg.user.as_ref().unwrap();
        assert!(user.role.is("SHEE"));
        assert_eq!(user.reference, Some((2013286677, 4350)));
        assert_eq!(seg.label(), "SHEE=2013286677/4350");
        assert_eq!(seg.groups.len(), 1);
        assert_eq!(seg.groups[0].id, Some(9));
        assert_eq!(seg.groups[0].len, 12);
        assert_eq!(seg.prim_count(), 1);
        assert_eq!(seg.encode_header(), words[..6]);
        assert_eq!(seg.groups[0].encode_header(), Some(words[6]));
        assert_eq!(seg.groups[0].prims[0].encode(), words[7..]);
    }

    #[test]
    fn a_view_segment_carries_its_translation_and_a_geom_block_lists_members() {
        // VIEW: flags 0xf, transform (1,0,0,1,60.98,177.81), user [3][VIEW][w0][w1].
        let mut words = vec![Word::header(kind::SEGMENT, 0xf, 11, 0)];
        words.extend([1.0, 0.0, 0.0, 1.0, 60.98, 177.81].map(Word::real));
        words.push(Word::int(4));
        words.extend([h("VIEW"), 2013286677, 4359].map(Word::int));
        let bytes = to_bytes(&words);
        let seg = Segment::parse(&Words::new(&bytes, 252), 11).unwrap();
        assert_eq!(seg.transform, Some([1.0, 0.0, 0.0, 1.0, 60.98, 177.81]));
        assert_eq!(seg.label(), "VIEW=2013286677/4359");
        assert!(seg.groups.is_empty());
        assert_eq!(seg.encode_header(), words);

        // GEOM: [GEOM][0][0] then (noun, w0, w1) × 2; group 2 is member 2.
        let mut words = vec![Word::header(kind::SEGMENT, 0x7, 11, 0)];
        words.push(Word::int(10));
        words.extend(
            [
                h("GEOM"),
                0,
                0,
                h("TUBE"),
                2013286676,
                2401,
                h("ELBO"),
                2013286676,
                2398,
            ]
            .map(Word::int),
        );
        let bytes = to_bytes(&words);
        let seg = Segment::parse(&Words::new(&bytes, 255), 11).unwrap();
        let user = seg.user.as_ref().unwrap();
        assert_eq!(seg.label(), "GEOM");
        assert_eq!(user.members.len(), 2);
        assert_eq!(
            user.member(2),
            Some(&Member {
                noun: "ELBO".to_string(),
                w0: 2013286676,
                w1: 2398
            })
        );
        assert_eq!(user.member(3), None);
        assert_eq!(user.member(0), None);

        // PPTS: [PPTS][1][0] then (w0, w1, noun, 0) × 1.
        let mut words = vec![Word::header(kind::SEGMENT, 0x6, 9, 0)];
        words.push(Word::int(8));
        words.extend([h("PPTS"), 1, 0, 2013286676, 2398, h("ELBO"), 0].map(Word::int));
        let bytes = to_bytes(&words);
        let seg = Segment::parse(&Words::new(&bytes, 293), 9).unwrap();
        assert!(!seg.is_visible());
        assert_eq!(seg.label(), "PPTS=1/0");
        assert_eq!(seg.user.as_ref().unwrap().member(1).unwrap().noun, "ELBO");
    }

    #[test]
    fn a_bare_primitive_outside_a_group_gets_a_group_without_an_id() {
        let mut words = vec![Word::header(kind::SEGMENT, 0x1, 1, 0)];
        words.push(Word::header(kind::MARKER, 8100, 3, 1));
        words.push(Word::real(1.0));
        words.push(Word::real(2.0));
        let bytes = to_bytes(&words);
        let seg = Segment::parse(&Words::new(&bytes, 1), 4).unwrap();
        assert_eq!(seg.label(), "seg");
        assert_eq!(seg.groups.len(), 1);
        assert_eq!(seg.groups[0].id, None);
        assert_eq!(seg.groups[0].encode_header(), None);
        assert_eq!(seg.groups[0].prims[0].kind, kind::MARKER);
    }

    #[test]
    fn a_texp_user_block_is_read_as_corners_ints_and_a_prefixed_string() {
        let mut raw = vec![Word::int(h("TEXP")), Word::int(15236), Word::int(5843)];
        for v in [1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 1.0, 4.0] {
            raw.push(Word::real(v));
        }
        raw.extend([1, 7, 7, 7, 7, 0, 3].map(Word::int));
        // "\x09OLAY AREA" packed 4 chars per word in u1.
        for chunk in [*b"\x09OLA", *b"Y AR", *b"EA\0\0"] {
            raw.push(Word::units(0, u32::from_be_bytes(chunk)));
        }
        let block = UserBlock::parse(raw.clone());
        assert!(block.role.is("TEXP"));
        let note = block.texp.as_ref().expect("the TEXP layout fits");
        assert_eq!(note.corners[1], (3.0, 2.0));
        assert_eq!(note.ints, [1, 7, 7, 7, 7, 0, 3]);
        assert_eq!(note.text, "OLAY AREA");
        assert_eq!(block.encode()[0], Word::int(raw.len() as i32 + 1));
        assert_eq!(block.encode()[1..], raw[..]);
    }

    #[test]
    fn a_word_that_is_not_a_noun_hash_is_kept_as_a_number() {
        assert_eq!(Role::from_word(0).label(), "#0");
        assert_eq!(Role::from_word(h("BNTE")).label(), "BNTE");
        assert!(Role::from_word(h("SAID")).is("SAID"));
    }
}
