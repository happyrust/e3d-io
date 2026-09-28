//! The picture table, one picture's directory, and the segment tree the
//! directory replays into.
//!
//! A GW file's picture table holds 8 words per picture: `[8, number, dir_rec,
//! int, real × 4]`, the reals being the paper window `(-W, -H, W, H)` in mm.
//! Every sample file carries exactly one picture. The directory the entry
//! points at starts with its own length and then runs 7-word entries
//! `[7, ref_w0, ref_w1, data_rec, rel | levels << 4, elem_type, nwords]`
//! (`docs/evidence/2026-09-28-gw-picture-format.md` §2, §2.1).
//!
//! Entries are the segment tree in the order `GSVPI0` saved it: a depth-first
//! walk in **reverse** — last child first, then its previous sibling, then up.
//! core.dll's load path (`sub_54DC674` op 9) rebuilds the tree by replaying
//! each entry against a cursor:
//!
//! ```text
//! cur = picture root
//! for entry in directory:
//!     cur = cur.parent, `levels` times          (HQTREE)
//!     rel 6: new node is cur's last child          (HINSEL 6)
//!     rel 3: new node is cur's previous sibling    (HINSEL 3)
//!     cur = new node
//! ```
//!
//! The tree is kept as indices into the entry list — `parent` and `children`
//! per node — so it needs no shared ownership and a walk is a small stack.

use super::word::Words;
use super::PictureError;

/// One 7-word directory entry, as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The 7 words of the entry, in file order.
    pub words: [i32; 7],
}

impl DirEntry {
    /// Words an entry occupies.
    pub const WORDS: usize = 7;

    /// The `(w0, w1)` DB reference the entry carries; non-zero only when the
    /// segment header's flag `0x20` is set (a handful of Marine segments).
    pub fn reference(&self) -> (i32, i32) {
        (self.words[1], self.words[2])
    }

    /// 1-based record where the element's data starts.
    pub fn rec(&self) -> i32 {
        self.words[3]
    }

    /// The raw `rel | levels << 4` word.
    pub fn rel_word(&self) -> i32 {
        self.words[4]
    }

    /// Tree relation to the cursor after climbing: 6 = last child,
    /// 3 = previous sibling.
    pub fn relation(&self) -> i32 {
        self.words[4] & 0xF
    }

    /// Parents to climb before applying the relation.
    pub fn levels(&self) -> i32 {
        self.words[4] >> 4
    }

    /// Element type; every sample entry is 3 (a segment).
    pub fn elem_type(&self) -> i32 {
        self.words[5]
    }

    /// Words of element data at [`Self::rec`].
    pub fn nwords(&self) -> i32 {
        self.words[6]
    }
}

/// A segment's place in the tree, by entry index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// Entry index of the parent; `None` for a child of the picture root.
    pub parent: Option<usize>,
    /// Entry indices of the children, in tree order (first child first).
    pub children: Vec<usize>,
    /// Depth below the picture root: a root child is 1.
    pub depth: usize,
}

/// One picture: its table entry, directory, and replayed tree.
///
/// Segment contents live beside the tree in [`Picture::segments`] once the
/// segment layer decodes them; here the picture knows where each element's
/// words are and how the elements nest.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Picture number (`GSVPI0`'s picture id; 76 in the small sample).
    pub number: i32,
    /// 1-based record of the directory.
    pub dir_rec: i32,
    /// The 4th table word: 0 on Plant, 1 on Marine files, meaning unrecorded.
    pub int4: i32,
    /// Paper window `(-W, -H, W, H)` in mm.
    pub window: [f64; 4],
    /// Word 1 of the directory: its own length in words.
    pub dir_len: i32,
    /// The entries in directory (save) order.
    pub entries: Vec<DirEntry>,
    nodes: Vec<Node>,
    roots: Vec<usize>,
}

impl Picture {
    /// Words one picture-table entry occupies.
    pub const TABLE_WORDS: usize = 8;

    /// Read the 8-word table entry starting at word 1 of `table`, then load
    /// and replay the directory it points at via `words_at(rec)`.
    pub(super) fn parse<'a>(
        table: Words<'a>,
        words_at: &impl Fn(i32) -> Result<Words<'a>, PictureError>,
    ) -> Result<Self, PictureError> {
        let number = table.int(2)?;
        let dir_rec = table.int(3)?;
        let int4 = table.int(4)?;
        let window = [
            table.real(5)?,
            table.real(6)?,
            table.real(7)?,
            table.real(8)?,
        ];
        let directory = words_at(dir_rec)?;
        let dir_len = directory.int(1)?;
        let mut entries = Vec::new();
        // Entry k occupies words 2 + 7k .. 8 + 7k; stop when a whole entry
        // would run past the declared length (exactly gwdump's `i + 6 <= n`).
        let mut i = 2usize;
        while i + 6 <= dir_len.max(0) as usize {
            let mut words = [0i32; 7];
            for (k, slot) in words.iter_mut().enumerate() {
                *slot = directory.int(i + k)?;
            }
            entries.push(DirEntry { words });
            i += DirEntry::WORDS;
        }
        let (nodes, roots) = replay(number, &entries)?;
        Ok(Self {
            number,
            dir_rec,
            int4,
            window,
            dir_len,
            entries,
            nodes,
            roots,
        })
    }

    /// The tree node of entry `index`.
    pub fn node(&self, index: usize) -> &Node {
        &self.nodes[index]
    }

    /// Every node, indexed like [`Picture::entries`].
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Entry indices of the picture root's children, first child first.
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    /// Pre-order walk of the tree: `(entry index, depth)`, parents before
    /// children, children in tree order. This is the order `gwdump.py`
    /// prints segments in and the order transforms compose down.
    pub fn walk(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        Walk {
            picture: self,
            stack: self.roots.iter().rev().copied().collect(),
        }
    }

    /// Entry indices along the path from the root child down to `index`,
    /// `index` last.
    pub fn ancestry(&self, index: usize) -> Vec<usize> {
        let mut path = vec![index];
        let mut cur = index;
        while let Some(parent) = self.nodes[cur].parent {
            path.push(parent);
            cur = parent;
        }
        path.reverse();
        path
    }
}

struct Walk<'a> {
    picture: &'a Picture,
    stack: Vec<usize>,
}

impl Iterator for Walk<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.stack.pop()?;
        let node = &self.picture.nodes[index];
        self.stack.extend(node.children.iter().rev().copied());
        Some((index, node.depth))
    }
}

/// Replay `entries` into a tree. Returns the nodes (indexed like `entries`)
/// and the root's children in tree order.
fn replay(picture: i32, entries: &[DirEntry]) -> Result<(Vec<Node>, Vec<usize>), PictureError> {
    let mut nodes: Vec<Node> = vec![Node::default(); entries.len()];
    let mut roots: Vec<usize> = Vec::new();
    // `None` is the picture root itself.
    let mut cur: Option<usize> = None;
    for (index, entry) in entries.iter().enumerate() {
        let bad = |reason: String| PictureError::BadDirectory {
            picture,
            entry: index,
            reason,
        };
        for _ in 0..entry.levels().max(0) {
            cur = match cur {
                Some(node) => nodes[node].parent,
                None => {
                    return Err(bad(format!(
                        "climbs {} levels but is already at the picture root",
                        entry.levels()
                    )))
                }
            };
        }
        match entry.relation() {
            6 => {
                // Last child of the cursor.
                nodes[index].parent = cur;
                nodes[index].depth = cur.map_or(1, |c| nodes[c].depth + 1);
                match cur {
                    Some(c) => nodes[c].children.push(index),
                    None => roots.push(index),
                }
            }
            3 => {
                // Previous sibling of the cursor: same parent, inserted just
                // before it.
                let sibling = cur.ok_or_else(|| {
                    bad("relation 3 (previous sibling) with the picture root as cursor".to_string())
                })?;
                let parent = nodes[sibling].parent;
                nodes[index].parent = parent;
                nodes[index].depth = nodes[sibling].depth;
                let list = match parent {
                    Some(p) => &mut nodes[p].children,
                    None => &mut roots,
                };
                let at = list
                    .iter()
                    .position(|&c| c == sibling)
                    .expect("the cursor is in its parent's child list");
                list.insert(at, index);
            }
            other => {
                return Err(bad(format!(
                    "tree relation {other} (word {:#x}); only 6 (last child) and 3 (previous sibling) occur",
                    entry.rel_word()
                )))
            }
        }
        cur = Some(index);
    }
    Ok((nodes, roots))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: i32, levels: i32) -> DirEntry {
        DirEntry {
            words: [7, 0, 0, 1, rel | (levels << 4), 3, 1],
        }
    }

    /// The small sample's first eight entries: `6 6 6 6 6 3 3 35` build
    /// SHEE > VIEW > DESI > PSPOOL > {GEOM, PPTS, PLIN} and then VAID as the
    /// VIEW's previous sibling (up two, rel 3).
    #[test]
    fn replay_appends_last_children_and_inserts_previous_siblings() {
        let entries = [
            entry(6, 0),
            entry(6, 0),
            entry(6, 0),
            entry(6, 0),
            entry(6, 0),
            entry(3, 0),
            entry(3, 0),
            entry(3, 2),
        ];
        let (nodes, roots) = replay(1, &entries).unwrap();
        assert_eq!(roots, vec![0]);
        assert_eq!(nodes[0].children, vec![1]);
        assert_eq!(nodes[1].children, vec![7, 2]);
        assert_eq!(nodes[2].children, vec![3]);
        assert_eq!(nodes[3].children, vec![6, 5, 4]);
        assert_eq!(nodes[7].parent, Some(1));
        assert_eq!(nodes[7].depth, 3);
        assert_eq!(nodes[4].depth, 5);
    }

    #[test]
    fn climbing_past_the_root_and_unknown_relations_are_errors() {
        let err = replay(9, &[entry(6, 1)]).unwrap_err();
        assert!(
            matches!(
                err,
                PictureError::BadDirectory {
                    picture: 9,
                    entry: 0,
                    ..
                }
            ),
            "{err}"
        );
        let err = replay(9, &[entry(3, 0)]).unwrap_err();
        assert!(
            matches!(err, PictureError::BadDirectory { entry: 0, .. }),
            "{err}"
        );
        let err = replay(9, &[entry(6, 0), entry(5, 0)]).unwrap_err();
        assert!(
            matches!(err, PictureError::BadDirectory { entry: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn the_walk_is_pre_order_in_tree_order() {
        let entries = [entry(6, 0), entry(6, 0), entry(3, 0), entry(3, 1)];
        // 0 > {2, 1}; then 3 inserted before 0 at the root.
        let (nodes, roots) = replay(1, &entries).unwrap();
        let picture = Picture {
            number: 1,
            dir_rec: 1,
            int4: 0,
            window: [0.0; 4],
            dir_len: 0,
            entries: entries.to_vec(),
            nodes,
            roots,
        };
        let order: Vec<(usize, usize)> = picture.walk().collect();
        assert_eq!(order, vec![(3, 1), (0, 1), (2, 2), (1, 2)]);
        assert_eq!(picture.ancestry(1), vec![0, 1]);
    }
}
