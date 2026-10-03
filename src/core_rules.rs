//! The rule registry (`.kiro/specs/write-path-phase-3/design.md`, 规则注册表
//! and D7 回退): every place the SPAMAP codec, the built-in AABB, the allocator
//! or the page-1 writer depends on how E3D_Core behaves, by name, with whether
//! that behaviour is confirmed and where the evidence is.
//!
//! A rule's implementation sits in a typed [`Slot`] of a [`RuleSet`]. The write
//! side reaches one rule set, [`RuleSet::confirmed`], and in it the slot of
//! every rule [`RULES`] does not call confirmed is [`Slot::Pending`]: no build
//! runs an unconfirmed rule. Whoever needs a pending rule treats the element
//! as having no usable World_AABB and names the rule; the other fallbacks are
//! [`recompute_row`], [`member_order_row`] and [`descendants_included`].
//!
//! Every implementation type starts empty, so nothing about E3D_Core is
//! presumed before its evidence exists. Confirming a rule is one commit: the
//! Evidence_Doc, a variant in the rule's implementation type, the slot filled
//! in `RuleSet::confirmed`, and the status in `RULES`. A compile-time check
//! refuses a filled slot whose rule is not confirmed.
//!
//! Candidates — assumptions checked against the corpus — come only from
//! [`RuleSet::candidates`], for Codec_Check and tests; the write side never
//! names it (`tests/core_rules_registry.rs`).

/// One row of the design's rule table. A family named per noun
/// (`spamap.recompute.<NOUN>`, `spamap.record_item.<NOUN>`) is one id until
/// its evidence names the nouns.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum RuleId {
    SpamapTableShape,
    SpamapTableHeaderRest,
    SpamapWordSign,
    SpamapRounding,
    SpamapKey0,
    SpamapShortAxis,
    SpamapZeroExtent,
    SpamapBucket,
    SpamapEntriesK,
    SpamapFlags,
    SpamapCoverage,
    SpamapRecompute,
    SpamapDescendantsMove,
    SpamapDescendantsPos,
    SpamapDescendantsOri,
    SpamapDeleteResidue,
    SpamapRecordItem,
    AabbOriForm,
    AabbCompose,
    AabbBox,
    AabbCyli,
    AabbDegenerate,
    AabbArithmetic,
    AllocCounterSemantics,
    AllocIndexOrder,
    BucketCollision,
    BucketPersist,
}

impl RuleId {
    pub const ALL: [RuleId; 27] = [
        RuleId::SpamapTableShape,
        RuleId::SpamapTableHeaderRest,
        RuleId::SpamapWordSign,
        RuleId::SpamapRounding,
        RuleId::SpamapKey0,
        RuleId::SpamapShortAxis,
        RuleId::SpamapZeroExtent,
        RuleId::SpamapBucket,
        RuleId::SpamapEntriesK,
        RuleId::SpamapFlags,
        RuleId::SpamapCoverage,
        RuleId::SpamapRecompute,
        RuleId::SpamapDescendantsMove,
        RuleId::SpamapDescendantsPos,
        RuleId::SpamapDescendantsOri,
        RuleId::SpamapDeleteResidue,
        RuleId::SpamapRecordItem,
        RuleId::AabbOriForm,
        RuleId::AabbCompose,
        RuleId::AabbBox,
        RuleId::AabbCyli,
        RuleId::AabbDegenerate,
        RuleId::AabbArithmetic,
        RuleId::AllocCounterSemantics,
        RuleId::AllocIndexOrder,
        RuleId::BucketCollision,
        RuleId::BucketPersist,
    ];

    /// The rule's name, as error reasons and Evidence_Docs spell it.
    pub fn name(self) -> &'static str {
        rule(self).name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub id: RuleId,
    pub name: &'static str,
    /// What the rule decides, in a line.
    pub governs: &'static str,
    pub status: RuleStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleStatus {
    /// `evidence` is the Evidence_Doc and the section whose 结论 states the
    /// rule, as `path#section`.
    Confirmed { evidence: &'static str },
    /// `assumption` is what a candidate assumes, `inputs` the inputs that need
    /// the rule, `evidence` a document that has looked at it without
    /// confirming it.
    Unconfirmed { assumption: &'static str, inputs: &'static str, evidence: Option<&'static str> },
}

impl RuleStatus {
    pub const fn is_confirmed(&self) -> bool {
        matches!(self, RuleStatus::Confirmed { .. })
    }
}

const NO_CANDIDATE: &str = "no candidate yet";
const T3: Option<&str> = Some("docs/evidence/2026-09-22-e3d-spamap-consequences.md#0. 结论");

const fn unconfirmed(inputs: &'static str, evidence: Option<&'static str>) -> RuleStatus {
    RuleStatus::Unconfirmed { assumption: NO_CANDIDATE, inputs, evidence }
}

/// Every rule, in the design's order.
pub const RULES: &[Rule] = &[
    Rule {
        id: RuleId::SpamapTableShape,
        name: "spamap.table_shape",
        governs: "page type, name hash, key width 4, leaf value width 7 and internal slots of a SPAMAP page",
        status: unconfirmed("every SPAMAP page written", T3),
    },
    Rule {
        id: RuleId::SpamapTableHeaderRest,
        name: "spamap.table_header_rest",
        governs: "the other header words of a new table's pages and of internal pages",
        status: unconfirmed("a batch that creates the table or splits a page", None),
    },
    Rule {
        id: RuleId::SpamapWordSign,
        name: "spamap.word_sign",
        governs: "whether key and value words are signed, how keys compare, Key0's width and overflow",
        status: unconfirmed("coordinates below zero, Key0 at or above 2^31", None),
    },
    Rule {
        id: RuleId::SpamapRounding,
        name: "spamap.rounding",
        governs: "how min and max round when millimetres become 0.1 mm",
        status: unconfirmed("a World_AABB off the 0.1 mm grid", None),
    },
    Rule {
        id: RuleId::SpamapKey0,
        name: "spamap.key0",
        governs: "Key0 = N * 10^8 + the short axis' min, N = 6 * axis order + bucket",
        status: unconfirmed("every entry", T3),
    },
    Rule {
        id: RuleId::SpamapShortAxis,
        name: "spamap.short_axis",
        governs: "whether the short axis is judged in mm or 0.1 mm, and which axis a tie takes",
        status: unconfirmed("every entry; ties decide on boxes with two equal shortest extents", None),
    },
    Rule {
        id: RuleId::SpamapZeroExtent,
        name: "spamap.zero_extent",
        governs: "a short axis of extent 0",
        status: unconfirmed("flat elements", None),
    },
    Rule {
        id: RuleId::SpamapBucket,
        name: "spamap.bucket",
        governs: "how buckets divide, which side a boundary falls on, values beyond the corpus' range",
        status: unconfirmed("every entry", T3),
    },
    Rule {
        id: RuleId::SpamapEntriesK,
        name: "spamap.entries_k",
        governs: "how many entries an element has, what k means, where a k >= 1 entry's value words come from",
        status: unconfirmed("every element with an entry; k >= 1 on elements across buckets", None),
    },
    Rule {
        id: RuleId::SpamapFlags,
        name: "spamap.flags",
        governs: "the meaning of each Spatial_Flags bit",
        status: unconfirmed("every entry", T3),
    },
    Rule {
        id: RuleId::SpamapCoverage,
        name: "spamap.coverage",
        governs: "which nouns are Geometric_Noun: yes, no or unconfirmed, per noun",
        status: unconfirmed("every inserted or changed element", T3),
    },
    Rule {
        id: RuleId::SpamapRecompute,
        name: "spamap.recompute.<NOUN>",
        governs: "which own attributes, and member order, make E3D recompute an element's entries",
        status: unconfirmed("every attribute write and member reorder", None),
    },
    Rule {
        id: RuleId::SpamapDescendantsMove,
        name: "spamap.descendants.move",
        governs: "which levels of descendants are recomputed when an element changes owner",
        status: unconfirmed("move_to on an element with descendants", None),
    },
    Rule {
        id: RuleId::SpamapDescendantsPos,
        name: "spamap.descendants.pos",
        governs: "which levels of descendants are recomputed when an element's POS changes",
        status: unconfirmed("a POS write on an element with descendants", None),
    },
    Rule {
        id: RuleId::SpamapDescendantsOri,
        name: "spamap.descendants.ori",
        governs: "which levels of descendants are recomputed when an element's ORI changes",
        status: unconfirmed("an ORI write on an element with descendants", None),
    },
    Rule {
        id: RuleId::SpamapDeleteResidue,
        name: "spamap.delete_residue",
        governs: "whether a deleted element's descendants keep entries (the write side deletes them either way)",
        status: unconfirmed("a delete of an element with descendants", None),
    },
    Rule {
        id: RuleId::SpamapRecordItem,
        name: "spamap.record_item.<NOUN>",
        governs: "whether E3D_Core writes SPAMAP_Record_Item into the element record, and its layout",
        status: unconfirmed("every element given or changing entries", None),
    },
    Rule {
        id: RuleId::AabbOriForm,
        name: "aabb.ori_form",
        governs: "how ORI is stored",
        status: unconfirmed("a built-in AABB with a rotated element or owner", None),
    },
    Rule {
        id: RuleId::AabbCompose,
        name: "aabb.compose",
        governs: "the order poses compose in, and which owner nouns take part",
        status: unconfirmed("every built-in AABB", None),
    },
    Rule {
        id: RuleId::AabbBox,
        name: "aabb.box",
        governs: "how a BOX's bounding box follows from XLEN / YLEN / ZLEN",
        status: unconfirmed("every BOX", None),
    },
    Rule {
        id: RuleId::AabbCyli,
        name: "aabb.cyli",
        governs: "how a CYLI's bounding box follows from DIAM / HEIG",
        status: unconfirmed("every CYLI", None),
    },
    Rule {
        id: RuleId::AabbDegenerate,
        name: "aabb.degenerate",
        governs: "a size of zero or below",
        status: unconfirmed("a BOX or CYLI with a zero or negative size", None),
    },
    Rule {
        id: RuleId::AabbArithmetic,
        name: "aabb.arithmetic",
        governs: "the precision of the arithmetic, and the criterion used where it cannot be reproduced exactly",
        status: unconfirmed("every built-in AABB", None),
    },
    Rule {
        id: RuleId::AllocCounterSemantics,
        name: "alloc.counter_semantics",
        governs: "what page 1's per-extent counter says about the next word1 (Counter_Semantics)",
        status: unconfirmed("every insert into an extent page 1 lists", None),
    },
    Rule {
        id: RuleId::AllocIndexOrder,
        name: "alloc.index_order",
        governs: "whether index keys sort by (word0, word1), and how they compare",
        status: unconfirmed("every Transaction::open that would seek instead of scan", None),
    },
    Rule {
        id: RuleId::BucketCollision,
        name: "bucket.collision",
        governs: "whether E3D hands out a word1 e3d-io has already used",
        status: unconfirmed("an E3D NEW after an e3d-io insert in the same extent", None),
    },
    Rule {
        id: RuleId::BucketPersist,
        name: "bucket.persist",
        governs: "how page 1 reaches the disk: copied on write and pointed to by page 0, or rewritten in place",
        status: unconfirmed("every page-1 write", None),
    },
];

/// The row of `id`.
pub fn rule(id: RuleId) -> &'static Rule {
    RULES.iter().find(|rule| rule.id == id).expect("every rule id has a row in RULES")
}

/// One rule's implementation in a rule set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot<T> {
    /// Not confirmed: whoever needs the rule falls back as D7 says.
    Pending,
    /// As the rule's Evidence_Doc states it.
    Confirmed(T),
    /// An assumption under test; only in rule sets from [`RuleSet::candidates`].
    Candidate(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Pending,
    Confirmed,
    Candidate,
}

impl<T> Slot<T> {
    pub const fn state(&self) -> SlotState {
        match self {
            Slot::Pending => SlotState::Pending,
            Slot::Confirmed(_) => SlotState::Confirmed,
            Slot::Candidate(_) => SlotState::Candidate,
        }
    }
}

macro_rules! implementations {
    ($($(#[$doc:meta])* $name:ident;)*) => {
        $(
            $(#[$doc])*
            ///
            /// Empty until the rule is confirmed or a candidate is written.
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub enum $name {}
        )*
    };
}

implementations! {
    /// `spamap.table_shape`.
    TableShape;
    /// `spamap.table_header_rest`.
    TableHeaderRest;
    /// `spamap.word_sign`.
    WordSign;
    /// `spamap.rounding`.
    Rounding;
    /// `spamap.key0`.
    Key0;
    /// `spamap.short_axis`.
    ShortAxis;
    /// `spamap.zero_extent`.
    ZeroExtent;
    /// `spamap.bucket`.
    Bucket;
    /// `spamap.entries_k`.
    EntriesK;
    /// `spamap.flags`.
    Flags;
    /// `spamap.coverage`.
    Coverage;
    /// `spamap.recompute.<NOUN>`: the recompute table and member-order rows.
    Recompute;
    /// `spamap.descendants.{move,pos,ori}`.
    DescendantRecompute;
    /// `spamap.delete_residue`.
    DeleteResidue;
    /// `spamap.record_item.<NOUN>`.
    RecordItem;
    /// `aabb.ori_form`.
    OriForm;
    /// `aabb.compose`.
    Compose;
    /// `aabb.box`.
    BoxAabb;
    /// `aabb.cyli`.
    CyliAabb;
    /// `aabb.degenerate`.
    Degenerate;
    /// `aabb.arithmetic`.
    Arithmetic;
    /// `alloc.counter_semantics`.
    CounterSemantics;
    /// `alloc.index_order`.
    IndexOrder;
    /// `bucket.collision`.
    BucketCollision;
    /// `bucket.persist`.
    BucketPersist;
}

/// One implementation slot per rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleSet {
    pub(crate) table_shape: Slot<TableShape>,
    pub(crate) table_header_rest: Slot<TableHeaderRest>,
    pub(crate) word_sign: Slot<WordSign>,
    pub(crate) rounding: Slot<Rounding>,
    pub(crate) key0: Slot<Key0>,
    pub(crate) short_axis: Slot<ShortAxis>,
    pub(crate) zero_extent: Slot<ZeroExtent>,
    pub(crate) bucket: Slot<Bucket>,
    pub(crate) entries_k: Slot<EntriesK>,
    pub(crate) flags: Slot<Flags>,
    pub(crate) coverage: Slot<Coverage>,
    pub(crate) recompute: Slot<Recompute>,
    pub(crate) descendants_move: Slot<DescendantRecompute>,
    pub(crate) descendants_pos: Slot<DescendantRecompute>,
    pub(crate) descendants_ori: Slot<DescendantRecompute>,
    pub(crate) delete_residue: Slot<DeleteResidue>,
    pub(crate) record_item: Slot<RecordItem>,
    pub(crate) ori_form: Slot<OriForm>,
    pub(crate) compose: Slot<Compose>,
    pub(crate) box_aabb: Slot<BoxAabb>,
    pub(crate) cyli_aabb: Slot<CyliAabb>,
    pub(crate) degenerate: Slot<Degenerate>,
    pub(crate) arithmetic: Slot<Arithmetic>,
    pub(crate) counter_semantics: Slot<CounterSemantics>,
    pub(crate) index_order: Slot<IndexOrder>,
    pub(crate) bucket_collision: Slot<BucketCollision>,
    pub(crate) bucket_persist: Slot<BucketPersist>,
}

/// What the evidence has confirmed. No rule is confirmed yet.
const CONFIRMED: RuleSet = RuleSet {
    table_shape: Slot::Pending,
    table_header_rest: Slot::Pending,
    word_sign: Slot::Pending,
    rounding: Slot::Pending,
    key0: Slot::Pending,
    short_axis: Slot::Pending,
    zero_extent: Slot::Pending,
    bucket: Slot::Pending,
    entries_k: Slot::Pending,
    flags: Slot::Pending,
    coverage: Slot::Pending,
    recompute: Slot::Pending,
    descendants_move: Slot::Pending,
    descendants_pos: Slot::Pending,
    descendants_ori: Slot::Pending,
    delete_residue: Slot::Pending,
    record_item: Slot::Pending,
    ori_form: Slot::Pending,
    compose: Slot::Pending,
    box_aabb: Slot::Pending,
    cyli_aabb: Slot::Pending,
    degenerate: Slot::Pending,
    arithmetic: Slot::Pending,
    counter_semantics: Slot::Pending,
    index_order: Slot::Pending,
    bucket_collision: Slot::Pending,
    bucket_persist: Slot::Pending,
};

static CONFIRMED_SET: RuleSet = CONFIRMED;

// A slot filled in the confirmed set belongs to a rule RULES calls confirmed,
// and the confirmed set holds no candidate.
const _: () = {
    let mut index = 0;
    while index < RULES.len() {
        let rule = &RULES[index];
        match CONFIRMED.state(rule.id) {
            SlotState::Pending => {}
            SlotState::Confirmed => {
                assert!(rule.status.is_confirmed(), "RuleSet::confirmed fills the slot of a rule RULES does not call confirmed")
            }
            SlotState::Candidate => panic!("RuleSet::confirmed holds a candidate"),
        }
        index += 1;
    }
};

impl RuleSet {
    /// The rules the evidence has confirmed: the only rule set the write side uses.
    pub fn confirmed() -> &'static RuleSet {
        &CONFIRMED_SET
    }

    /// The confirmed rules with the picked candidates in the pending slots —
    /// for Codec_Check and tests, never for writing.
    #[doc(hidden)]
    pub fn candidates(pick: &CandidatePick) -> RuleSet {
        let CandidatePick {} = pick;
        CONFIRMED
    }

    /// Whether `id`'s slot is pending, confirmed or a candidate in this set.
    pub const fn state(&self, id: RuleId) -> SlotState {
        match id {
            RuleId::SpamapTableShape => self.table_shape.state(),
            RuleId::SpamapTableHeaderRest => self.table_header_rest.state(),
            RuleId::SpamapWordSign => self.word_sign.state(),
            RuleId::SpamapRounding => self.rounding.state(),
            RuleId::SpamapKey0 => self.key0.state(),
            RuleId::SpamapShortAxis => self.short_axis.state(),
            RuleId::SpamapZeroExtent => self.zero_extent.state(),
            RuleId::SpamapBucket => self.bucket.state(),
            RuleId::SpamapEntriesK => self.entries_k.state(),
            RuleId::SpamapFlags => self.flags.state(),
            RuleId::SpamapCoverage => self.coverage.state(),
            RuleId::SpamapRecompute => self.recompute.state(),
            RuleId::SpamapDescendantsMove => self.descendants_move.state(),
            RuleId::SpamapDescendantsPos => self.descendants_pos.state(),
            RuleId::SpamapDescendantsOri => self.descendants_ori.state(),
            RuleId::SpamapDeleteResidue => self.delete_residue.state(),
            RuleId::SpamapRecordItem => self.record_item.state(),
            RuleId::AabbOriForm => self.ori_form.state(),
            RuleId::AabbCompose => self.compose.state(),
            RuleId::AabbBox => self.box_aabb.state(),
            RuleId::AabbCyli => self.cyli_aabb.state(),
            RuleId::AabbDegenerate => self.degenerate.state(),
            RuleId::AabbArithmetic => self.arithmetic.state(),
            RuleId::AllocCounterSemantics => self.counter_semantics.state(),
            RuleId::AllocIndexOrder => self.index_order.state(),
            RuleId::BucketCollision => self.bucket_collision.state(),
            RuleId::BucketPersist => self.bucket_persist.state(),
        }
    }
}

/// Which candidate each rule takes in [`RuleSet::candidates`]. Empty until the
/// candidates are written (`src/spatial/candidates.rs`).
#[doc(hidden)]
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidatePick {}

/// Whether a change makes E3D_Core recompute an element's entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Triggers,
    DoesNotTrigger,
    /// Not confirmed either way; the write side treats it as triggering (D7).
    Unconfirmed,
}

/// Whether writing `attr` on an element of `noun` recomputes that element's
/// entries (`spamap.recompute.<NOUN>`). An attribute the confirmed table does
/// not list is [`Trigger::Unconfirmed`].
pub fn recompute_row(noun: u32, attr: u32) -> Trigger {
    let _ = (noun, attr);
    match CONFIRMED.recompute {
        Slot::Pending => Trigger::Unconfirmed,
        Slot::Confirmed(table) | Slot::Candidate(table) => match table {},
    }
}

/// Whether reordering an element of `noun` among its owner's members
/// recomputes its entries (`spamap.recompute.<NOUN>`).
pub fn member_order_row(noun: u32) -> Trigger {
    let _ = noun;
    match CONFIRMED.recompute {
        Slot::Pending => Trigger::Unconfirmed,
        Slot::Confirmed(table) | Slot::Candidate(table) => match table {},
    }
}

/// What happened to an element whose descendants may need new entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescendantKind {
    /// It changed owner.
    Move,
    /// Its POS changed.
    Pos,
    /// Its ORI changed.
    Ori,
}

/// Whether descendants `depth` levels below such an element (1 = its members)
/// have their entries recomputed (`spamap.descendants.{move,pos,ori}`).
///
/// Unconfirmed is `false`: Recompute_Set takes only the descendants confirmed
/// to be recomputed (definitions ④ and ⑤, D8). This is the one place to
/// change that.
pub fn descendants_included(kind: DescendantKind, depth: u32) -> bool {
    let _ = depth;
    let slot = match kind {
        DescendantKind::Move => CONFIRMED.descendants_move,
        DescendantKind::Pos => CONFIRMED.descendants_pos,
        DescendantKind::Ori => CONFIRMED.descendants_ori,
    };
    match slot {
        Slot::Pending => false,
        Slot::Confirmed(rule) | Slot::Candidate(rule) => match rule {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_id_has_exactly_one_row() {
        assert_eq!(RULES.len(), RuleId::ALL.len());
        for id in RuleId::ALL {
            assert_eq!(RULES.iter().filter(|rule| rule.id == id).count(), 1, "{id:?}");
        }
    }

    #[test]
    fn nothing_is_confirmed_before_its_evidence() {
        for id in RuleId::ALL {
            assert_eq!(RuleSet::confirmed().state(id), SlotState::Pending, "{}", id.name());
            assert!(!rule(id).status.is_confirmed(), "{}", id.name());
        }
        assert_eq!(recompute_row(0, 0), Trigger::Unconfirmed);
        assert_eq!(member_order_row(0), Trigger::Unconfirmed);
        for kind in [DescendantKind::Move, DescendantKind::Pos, DescendantKind::Ori] {
            assert!(!descendants_included(kind, 1));
        }
    }
}
