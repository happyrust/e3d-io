//! The values SPAMAP maintenance is computed from
//! (`.kiro/specs/write-path-phase-3/design.md`, SPAMAP_Codec and Built_In_AABB):
//! a world-axis bounding box, the policy for an element without one, and a
//! read-only view of element state that the read side ([`state::ReadState`])
//! and the write side's batch end state both provide, so the same arithmetic
//! is checked against the corpus and then used to write.
//!
//! Nothing here is in `src/write/`: the corpus checks that gate the first
//! SPAMAP write run this code before any write-side commit exists.

pub mod aabb;
#[doc(hidden)]
pub mod candidates;
pub mod check;
pub mod codec;
pub mod noun_fields;
pub mod rules;
pub mod state;

use crate::core_rules::{Implementation, RuleId, Slot};
use crate::record::descriptor::DescriptorValue;
use crate::refno::RefNo;

/// A world axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
}

impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Z => "z",
        })
    }
}

/// An element's bounding box on the world axes, in millimetres.
///
/// Only [`WorldAabb::new`] makes one, and it refuses a coordinate that is not
/// finite and an axis whose min exceeds its max. Rounding to the 0.1 mm SPAMAP
/// stores is the codec's, under `spamap.rounding`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldAabb {
    min_mm: [f64; 3],
    max_mm: [f64; 3],
}

impl WorldAabb {
    pub fn new(min_mm: [f64; 3], max_mm: [f64; 3]) -> Result<Self, InvalidAabb> {
        for axis in Axis::ALL {
            let (min, max) = (min_mm[axis as usize], max_mm[axis as usize]);
            if !min.is_finite() || !max.is_finite() {
                return Err(InvalidAabb::NotFinite { axis });
            }
            if min > max {
                return Err(InvalidAabb::MinAboveMax { axis, min, max });
            }
        }
        Ok(Self { min_mm, max_mm })
    }

    /// From `[minx, miny, minz, maxx, maxy, maxz]`, the shape `e3d-write`'s
    /// `aabb` and `Transaction::set_aabb` take.
    pub fn from_mm(aabb_mm: [f64; 6]) -> Result<Self, InvalidAabb> {
        let [min_x, min_y, min_z, max_x, max_y, max_z] = aabb_mm;
        Self::new([min_x, min_y, min_z], [max_x, max_y, max_z])
    }

    pub fn min_mm(&self) -> [f64; 3] {
        self.min_mm
    }

    pub fn max_mm(&self) -> [f64; 3] {
        self.max_mm
    }
}

/// Why six numbers are not a bounding box.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum InvalidAabb {
    #[error("the {axis} axis has a coordinate that is not a finite number")]
    NotFinite { axis: Axis },
    #[error("the {axis} axis has min {min} above max {max}")]
    MinAboveMax { axis: Axis, min: f64, max: f64 },
}

/// What a batch does with an element that needs SPAMAP entries and has no
/// usable World_AABB. The default is the one statement of D3.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SpatialPolicy {
    /// Refuse the whole batch.
    #[default]
    Require,
    /// Write the batch; the element gets no entries and the report says so.
    Skip,
    /// Do not maintain SPAMAP: no table read, no page written.
    Off,
}

/// Element state as of one session, or as of the end of one batch.
pub trait SpatialState {
    /// The element's noun, or `None` when there is no such element.
    fn noun(&self, refno: RefNo) -> Option<u32>;
    /// The element's owner, or `None` for one without (the world, or no element).
    fn owner(&self, refno: RefNo) -> Option<RefNo>;
    /// The attribute `hash` of the element: `None` when the element has no
    /// such attribute or does not exist, an error when it has one that cannot
    /// be decoded.
    fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr>;
    /// Every element whose owner the element is, in refno order, or `None`
    /// when this state cannot list them.
    fn members(&self, _refno: RefNo) -> Option<Vec<RefNo>> {
        None
    }
}

/// What `spamap.entries_k` makes of one element: its entries as `(k, AABB)` in
/// 0.1 mm, or the input it would need and cannot have (requirement 6.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryBoxes {
    Entries(Vec<(u32, [i64; 6])>),
    MissingInput(&'static str),
}

/// An attribute a computation needs and cannot read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{refno}: attribute {name} ({hash}) cannot be decoded")]
pub struct UndecodableAttr {
    pub refno: RefNo,
    pub name: String,
    pub hash: u32,
}

/// The rules one computation consulted: those pending, and those a candidate
/// answered for, each once.
#[derive(Default)]
pub(crate) struct Walk {
    pub(crate) pending: Vec<RuleId>,
    pub(crate) relied_on: Vec<RuleId>,
}

impl Walk {
    /// The rule's function, or `None` with the rule noted as pending.
    pub(crate) fn take<T: Implementation<C>, C: Copy>(&mut self, id: RuleId, slot: &Slot<T, C>) -> Option<C> {
        match slot {
            Slot::Pending => {
                self.need(id);
                None
            }
            Slot::Confirmed(confirmed) => Some(confirmed.function()),
            Slot::Candidate(candidate) => {
                if !self.relied_on.contains(&id) {
                    self.relied_on.push(id);
                }
                Some(*candidate)
            }
        }
    }

    /// Note `id` as pending: a candidate that does not cover the case counts
    /// as no rule.
    pub(crate) fn need(&mut self, id: RuleId) {
        if !self.pending.contains(&id) {
            self.pending.push(id);
        }
    }

    pub(crate) fn sorted(mut ids: Vec<RuleId>) -> Vec<RuleId> {
        ids.sort();
        ids
    }
}
