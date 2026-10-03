//! Built_In_AABB (`.kiro/specs/write-path-phase-3/design.md`, Built_In_AABB;
//! requirements 6.1, 6.2, 6.8): the World_AABB e3d-io computes for an element
//! itself — a BOX from XLEN / YLEN / ZLEN, a CYLI from DIAM / HEIG (D1) — from
//! its own attributes and the poses up its owner chain, with the boxes of its
//! `IMAP` 3 members (the negative solids under it) taken in.
//!
//! As in the codec, every step that depends on E3D_Core is a rule of
//! [`crate::core_rules`]: how ORI is stored (`aabb.ori_form`), how poses
//! compose and which owners take part (`aabb.compose`), how each noun's box
//! follows from its sizes (`aabb.box`, `aabb.cyli`, and for members
//! `aabb.cone`, `aabb.pyramid`), which members' boxes count
//! (`aabb.members`, consulted only for an element with members), sizes below
//! 0.5 mm (`aabb.degenerate`, consulted only when a size is), and the
//! precision (`aabb.arithmetic`). A pending rule is noted and the walk goes
//! on; then no box comes out.
//!
//! Every attribute the computation needs, the owners' included, is read
//! before any rule is asked, so one that cannot be decoded is reported
//! whatever the rules: requirement 6.8 refuses the batch on it, under Skip
//! too. The box is in millimetres and unrounded; rounding to 0.1 mm is the
//! codec's.

use e3d_attlib::db1_hash;

use super::rules::soft_imap;
use super::{SpatialState, UndecodableAttr, Walk, WorldAabb};
use crate::core_rules::{candidate, RuleId, RuleSet};
use crate::record::descriptor::DescriptorValue;
use crate::refno::RefNo;

/// The nouns e3d-io computes a World_AABB for; for every other Geometric_Noun
/// the caller gives one. The one statement of D1: its alternative A empties
/// the list, alternative B adds to it.
pub const BUILT_IN_NOUNS: &[&str] = &["BOX", "CYLI"];

/// The members whose boxes the built-in AABB can take into their owner's:
/// `dbboxlib/BOXELE`'s box types 3, 2, 5 and 7. An element with a member
/// `aabb.members` takes in of any other noun has no built-in box.
pub const BUILT_IN_MEMBER_NOUNS: &[&str] = &["NBOX", "NCYL", "NCON", "NPYR"];

/// No rotation.
pub const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// An element's own axes in the world: the columns of `rotation` are its x, y
/// and z axes in world coordinates, and `origin_mm` is its origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldFrame {
    pub rotation: [[f64; 3]; 3],
    pub origin_mm: [f64; 3],
}

impl WorldFrame {
    /// The world's own frame.
    pub const WORLD: WorldFrame = WorldFrame { rotation: IDENTITY, origin_mm: [0.0; 3] };

    /// A point given in the frame's own coordinates, in the world's.
    pub fn point(&self, own_mm: [f64; 3]) -> [f64; 3] {
        let mut world = self.origin_mm;
        for (at, row) in world.iter_mut().zip(&self.rotation) {
            *at += row[0] * own_mm[0] + row[1] * own_mm[1] + row[2] * own_mm[2];
        }
        world
    }

    /// The frame of a member at `pos_mm` turned by `rotation`, both given in
    /// this frame.
    pub fn then(&self, pos_mm: [f64; 3], rotation: [[f64; 3]; 3]) -> WorldFrame {
        WorldFrame { rotation: multiply(&self.rotation, &rotation), origin_mm: self.point(pos_mm) }
    }
}

/// The matrix product `a · b`.
pub fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut product = [[0.0; 3]; 3];
    for (i, row) in product.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    product
}

/// One element of an owner chain: its pose in its owner's frame, `None`
/// where it has no POS or no ORI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub refno: RefNo,
    pub noun: u32,
    pub pos_mm: Option<[f64; 3]>,
    pub rotation: Option<[[f64; 3]; 3]>,
}

/// What a composer makes of one element.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameOutcome {
    Frame(WorldFrame),
    /// Every pending rule the frame needs, in [`RuleId`] order.
    NeedsUnconfirmed(Vec<RuleId>),
    /// The owner chain ends, or runs deeper than any real one, before a WORL.
    NoWorld,
}

/// Puts an element's pose and its owners' together into its world frame.
/// Replaceable, as the evidence on ORI and on composing may ask.
pub trait FrameComposer {
    fn world_frame(&self, state: &dyn SpatialState, refno: RefNo) -> Result<FrameOutcome, UndecodableAttr>;
}

/// The composer by the rules: up the owner chain to WORL, each link's POS and
/// ORI read (ORI as `aabb.ori_form` has it), put together as `aabb.compose`
/// says, each value held as `aabb.arithmetic` says.
#[derive(Debug, Clone, Copy)]
pub struct ChainComposer<'r> {
    rules: &'r RuleSet,
}

/// An owner chain longer than this is taken for a loop.
const MAX_DEPTH: usize = 64;

impl<'r> ChainComposer<'r> {
    pub fn new(rules: &'r RuleSet) -> Self {
        Self { rules }
    }
}

impl FrameComposer for ChainComposer<'_> {
    fn world_frame(&self, state: &dyn SpatialState, refno: RefNo) -> Result<FrameOutcome, UndecodableAttr> {
        let world = db1_hash("WORL");
        let mut read = Vec::new();
        let mut at = refno;
        loop {
            let Some(noun) = state.noun(at) else { return Ok(FrameOutcome::NoWorld) };
            if noun == world {
                break;
            }
            if read.len() == MAX_DEPTH {
                return Ok(FrameOutcome::NoWorld);
            }
            read.push((at, noun, position(state, at)?, orientation(state, at)?));
            let Some(owner) = state.owner(at) else { return Ok(FrameOutcome::NoWorld) };
            at = owner;
        }

        let rules = self.rules;
        let mut walk = Walk::default();
        let ori_form = if read.iter().any(|(.., ori)| ori.is_some()) {
            walk.take(RuleId::AabbOriForm, &rules.ori_form)
        } else {
            None
        };
        let compose = walk.take(RuleId::AabbCompose, &rules.compose);
        let held = walk.take(RuleId::AabbArithmetic, &rules.arithmetic);
        let mut chain = Vec::with_capacity(read.len());
        for (refno, noun, pos_mm, ori) in read {
            let rotation = match (ori, ori_form) {
                (Some(ori), Some(form)) => {
                    let rotation = form(&ori);
                    if rotation.is_none() {
                        walk.need(RuleId::AabbOriForm);
                    }
                    rotation
                }
                _ => None,
            };
            chain.push(Link { refno, noun, pos_mm, rotation });
        }
        let (Some(compose), Some(held), true) = (compose, held, walk.pending.is_empty()) else {
            return Ok(FrameOutcome::NeedsUnconfirmed(Walk::sorted(walk.pending)));
        };
        for link in &mut chain {
            link.pos_mm = link.pos_mm.map(|pos| pos.map(held));
            link.rotation = link.rotation.map(|rotation| rotation.map(|row| row.map(held)));
        }
        Ok(match compose(&chain) {
            Some(frame) => FrameOutcome::Frame(WorldFrame {
                rotation: frame.rotation.map(|row| row.map(held)),
                origin_mm: frame.origin_mm.map(held),
            }),
            None => FrameOutcome::NeedsUnconfirmed(vec![RuleId::AabbCompose]),
        })
    }
}

/// What the built-in AABB makes of one element.
#[derive(Debug, Clone, PartialEq)]
pub enum AabbOutcome {
    Aabb(WorldAabb),
    /// Every pending rule the box needs, its frame's among them, in
    /// [`RuleId`] order.
    NeedsUnconfirmed(Vec<RuleId>),
    /// An input e3d-io cannot have (requirement 6.15): the box of a member of
    /// a noun outside [`BUILT_IN_MEMBER_NOUNS`], the `IMAP` a UDET gives, or
    /// members the state cannot list.
    MissingInput(&'static str),
    /// Not of a noun in [`BUILT_IN_NOUNS`], or no such element.
    NotBuiltIn,
    /// The owner chain does not reach a WORL.
    NoWorld,
}

/// The World_AABB of a BOX or CYLI from its sizes, its world frame and the
/// members `aabb.members` takes in.
#[derive(Clone, Copy)]
pub struct BuiltInAabb<'r> {
    rules: &'r RuleSet,
    composer: &'r dyn FrameComposer,
}

/// A solid's sizes as stored, in mm.
#[derive(Clone, Copy)]
enum Solid {
    Box([f64; 3]),
    Cylinder { diam: f64, heig: f64 },
    Cone { heig: f64, dtop: f64, dbot: f64 },
    Pyramid { top: [f64; 2], bottom: [f64; 2], heig: f64, offset: [f64; 2] },
}

/// A member of the element, as `aabb.members` and its box need it: its
/// noun's `IMAP`, and for a noun in [`BUILT_IN_MEMBER_NOUNS`] its solid and
/// its pose in the element's frame.
struct Member {
    imap: u8,
    solid: Option<Solid>,
    pos_mm: Option<[f64; 3]>,
    ori: Option<DescriptorValue>,
}

/// How a solid's box follows from its sizes, measured as `BOXELE` measures
/// them.
enum Method {
    Box(candidate::BoxAabb, [f64; 3]),
    Cylinder(candidate::CyliAabb, f64, f64),
    Cone(candidate::ConeAabb, f64, f64, f64),
    Pyramid(candidate::PyramidAabb, [f64; 2], [f64; 2], f64, [f64; 2]),
}

impl Method {
    fn bound(&self, frame: &WorldFrame, held: candidate::Arithmetic) -> [f64; 6] {
        match *self {
            Method::Box(rule, size) => rule(frame, size.map(held)),
            Method::Cylinder(rule, diam, heig) => rule(frame, held(diam), held(heig)),
            Method::Cone(rule, heig, dtop, dbot) => rule(frame, held(heig), held(dtop), held(dbot)),
            Method::Pyramid(rule, top, bottom, heig, offset) => {
                rule(frame, top.map(held), bottom.map(held), held(heig), offset.map(held))
            }
        }
    }
}

const UNLISTED_MEMBERS: &str = "the element's members";
const UDET_IMAP: &str = "the IMAP a UDET gives the element or one of its members";
const UNBUILT_MEMBER: &str = "the box of a member of a noun the built-in AABB does not compute";

impl<'r> BuiltInAabb<'r> {
    pub fn new(rules: &'r RuleSet, composer: &'r dyn FrameComposer) -> Self {
        Self { rules, composer }
    }

    /// The World_AABB of `refno` as `state` has it and its owners.
    pub fn compute(&self, state: &dyn SpatialState, refno: RefNo) -> Result<AabbOutcome, UndecodableAttr> {
        let Some(noun) = state.noun(refno) else { return Ok(AabbOutcome::NotBuiltIn) };
        let built_in = |name: &str| noun == db1_hash(name) && BUILT_IN_NOUNS.contains(&name);
        let solid = if built_in("BOX") {
            Solid::Box([length(state, refno, "XLEN")?, length(state, refno, "YLEN")?, length(state, refno, "ZLEN")?])
        } else if built_in("CYLI") {
            Solid::Cylinder { diam: length(state, refno, "DIAM")?, heig: length(state, refno, "HEIG")? }
        } else {
            return Ok(AabbOutcome::NotBuiltIn);
        };
        let (imap, members) = match members(state, refno, noun)? {
            Ok(read) => read,
            Err(missing) => return Ok(AabbOutcome::MissingInput(missing)),
        };
        let mut walk = Walk::default();
        let frame = match self.composer.world_frame(state, refno)? {
            FrameOutcome::Frame(frame) => Some(frame),
            FrameOutcome::NeedsUnconfirmed(pending) => {
                pending.into_iter().for_each(|id| walk.need(id));
                None
            }
            FrameOutcome::NoWorld => return Ok(AabbOutcome::NoWorld),
        };

        let rules = self.rules;
        let held = walk.take(RuleId::AabbArithmetic, &rules.arithmetic);
        let own = self.method(solid, &mut walk);
        let mut taken_in = Vec::new();
        let takes_in = if members.is_empty() { None } else { walk.take(RuleId::AabbMembers, &rules.members) };
        if let Some(takes_in) = takes_in {
            for member in members.into_iter().filter(|member| takes_in(imap, member.imap)) {
                let Some(solid) = member.solid else { return Ok(AabbOutcome::MissingInput(UNBUILT_MEMBER)) };
                let rotation = match &member.ori {
                    None => Some(IDENTITY),
                    Some(ori) => walk.take(RuleId::AabbOriForm, &rules.ori_form).and_then(|form| {
                        let rotation = form(ori);
                        if rotation.is_none() {
                            walk.need(RuleId::AabbOriForm);
                        }
                        rotation
                    }),
                };
                if let (Some(method), Some(rotation)) = (self.method(solid, &mut walk), rotation) {
                    taken_in.push((method, member.pos_mm.unwrap_or([0.0; 3]), rotation));
                }
            }
        }
        let (Some(frame), Some(held), Some(own), true) = (frame, held, own, walk.pending.is_empty()) else {
            return Ok(AabbOutcome::NeedsUnconfirmed(Walk::sorted(walk.pending)));
        };

        let mut aabb_mm = own.bound(&frame, held);
        for (method, pos_mm, rotation) in taken_in {
            let member_frame = frame.then(pos_mm.map(held), rotation.map(|row| row.map(held)));
            let member_mm = method.bound(&member_frame, held);
            for axis in 0..3 {
                if member_mm[axis] < aabb_mm[axis] {
                    aabb_mm[axis] = member_mm[axis];
                }
                if member_mm[axis + 3] > aabb_mm[axis + 3] {
                    aabb_mm[axis + 3] = member_mm[axis + 3];
                }
            }
        }
        Ok(AabbOutcome::Aabb(WorldAabb::from_mm(aabb_mm.map(held)).expect("finite poses and sizes bound a box")))
    }

    /// The rule that boxes `solid` and its sizes as `BOXELE` measures them:
    /// every size of a BOX, NBOX, CYLI or NCYL and the HEIG of an NCON or
    /// NPYR through `aabb.degenerate` when below 0.5 mm. `None` with the
    /// pending rules noted in `walk`.
    fn method(&self, solid: Solid, walk: &mut Walk) -> Option<Method> {
        let rules = self.rules;
        match solid {
            Solid::Box(size) => {
                let size = size.map(|size| measured(size, rules, walk));
                let rule = walk.take(RuleId::AabbBox, &rules.box_aabb);
                Some(Method::Box(rule?, [size[0]?, size[1]?, size[2]?]))
            }
            Solid::Cylinder { diam, heig } => {
                let (diam, heig) = (measured(diam, rules, walk), measured(heig, rules, walk));
                let rule = walk.take(RuleId::AabbCyli, &rules.cyli_aabb);
                Some(Method::Cylinder(rule?, diam?, heig?))
            }
            Solid::Cone { heig, dtop, dbot } => {
                let heig = measured(heig, rules, walk);
                let rule = walk.take(RuleId::AabbCone, &rules.cone_aabb);
                Some(Method::Cone(rule?, heig?, dtop, dbot))
            }
            Solid::Pyramid { top, bottom, heig, offset } => {
                let heig = measured(heig, rules, walk);
                let rule = walk.take(RuleId::AabbPyramid, &rules.pyramid_aabb);
                Some(Method::Pyramid(rule?, top, bottom, heig?, offset))
            }
        }
    }
}

/// The size below which Core3D's `dbboxlib/BOXELE` measures a size by
/// `aabb.degenerate` rather than as stored.
const DEGENERATE_BELOW_MM: f64 = 0.5;

/// A size as `BOXELE` measures it: as stored from 0.5 mm up, by
/// `aabb.degenerate` below; `None` with the rule noted when it is pending.
fn measured(size_mm: f64, rules: &RuleSet, walk: &mut Walk) -> Option<f64> {
    if size_mm >= DEGENERATE_BELOW_MM {
        return Some(size_mm);
    }
    let measured = walk.take(RuleId::AabbDegenerate, &rules.degenerate).and_then(|degenerate| degenerate(size_mm));
    if measured.is_none() {
        walk.need(RuleId::AabbDegenerate);
    }
    measured
}

/// An element's `IMAP` and its members, or the input the state cannot give.
type Members = Result<(u8, Vec<Member>), &'static str>;

/// The element's `IMAP` and its members, each with what `aabb.members` and
/// its box need, every attribute read. An element without members needs no
/// `IMAP` (0).
fn members(state: &dyn SpatialState, refno: RefNo, noun: u32) -> Result<Members, UndecodableAttr> {
    let Some(listed) = state.members(refno) else { return Ok(Err(UNLISTED_MEMBERS)) };
    if listed.is_empty() {
        return Ok(Ok((0, Vec::new())));
    }
    let Some(imap) = soft_imap(refno, noun, state)? else { return Ok(Err(UDET_IMAP)) };
    let mut members = Vec::with_capacity(listed.len());
    for member in listed {
        let Some(member_noun) = state.noun(member) else { continue };
        let Some(member_imap) = soft_imap(member, member_noun, state)? else { return Ok(Err(UDET_IMAP)) };
        let solid = member_solid(state, member, member_noun)?;
        let (pos_mm, ori) = match solid {
            Some(_) => (position(state, member)?, orientation(state, member)?),
            None => (None, None),
        };
        members.push(Member { imap: member_imap, solid, pos_mm, ori });
    }
    Ok(Ok((imap, members)))
}

/// A member's sizes as stored, for a noun in [`BUILT_IN_MEMBER_NOUNS`].
fn member_solid(state: &dyn SpatialState, member: RefNo, noun: u32) -> Result<Option<Solid>, UndecodableAttr> {
    let is = |name: &str| noun == db1_hash(name) && BUILT_IN_MEMBER_NOUNS.contains(&name);
    let size = |name: &str| length(state, member, name);
    Ok(Some(if is("NBOX") {
        Solid::Box([size("XLEN")?, size("YLEN")?, size("ZLEN")?])
    } else if is("NCYL") {
        Solid::Cylinder { diam: size("DIAM")?, heig: size("HEIG")? }
    } else if is("NCON") {
        Solid::Cone { heig: size("HEIG")?, dtop: size("DTOP")?, dbot: size("DBOT")? }
    } else if is("NPYR") {
        Solid::Pyramid {
            top: [size("XTOP")?, size("YTOP")?],
            bottom: [size("XBOT")?, size("YBOT")?],
            heig: size("HEIG")?,
            offset: [size("XOFF")?, size("YOFF")?],
        }
    } else {
        return Ok(None);
    }))
}

/// A length attribute in mm: present, a real, finite.
fn length(state: &dyn SpatialState, refno: RefNo, name: &str) -> Result<f64, UndecodableAttr> {
    let hash = db1_hash(name);
    match state.attr(refno, hash)? {
        Some(DescriptorValue::Real(mm)) if mm.is_finite() => Ok(mm),
        _ => Err(UndecodableAttr { refno, name: name.to_owned(), hash }),
    }
}

/// POS in mm, or `None` for an element without one.
fn position(state: &dyn SpatialState, refno: RefNo) -> Result<Option<[f64; 3]>, UndecodableAttr> {
    let hash = db1_hash("POS");
    match state.attr(refno, hash)? {
        None => Ok(None),
        Some(DescriptorValue::RealArray(mm)) if mm.len() == 3 && mm.iter().all(|value| value.is_finite()) => {
            Ok(Some([mm[0], mm[1], mm[2]]))
        }
        Some(_) => Err(UndecodableAttr { refno, name: "POS".to_owned(), hash }),
    }
}

/// ORI as stored, for `aabb.ori_form` to read, or `None` for an element
/// without one. A stored number that is not finite is undecodable whatever
/// the form.
fn orientation(state: &dyn SpatialState, refno: RefNo) -> Result<Option<DescriptorValue>, UndecodableAttr> {
    let hash = db1_hash("ORI");
    let ori = state.attr(refno, hash)?;
    if let Some(DescriptorValue::RealArray(values)) = &ori {
        if values.iter().any(|value| !value.is_finite()) {
            return Err(UndecodableAttr { refno, name: "ORI".to_owned(), hash });
        }
    }
    Ok(ori)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::core_rules::CandidatePick;
    use crate::index::SpamapEntry;
    use crate::spatial::candidates;
    use crate::spatial::codec::{Encoded, SpamapCodec};

    struct Element {
        noun: u32,
        owner: Option<RefNo>,
        attrs: HashMap<u32, DescriptorValue>,
    }

    /// Elements in memory.
    #[derive(Default)]
    struct Tree {
        elements: HashMap<RefNo, Element>,
    }

    impl Tree {
        fn with(mut self, refno: RefNo, noun: &str, owner: Option<RefNo>, attrs: &[(&str, DescriptorValue)]) -> Self {
            let attrs = attrs.iter().map(|(name, value)| (db1_hash(name), value.clone())).collect();
            self.elements.insert(refno, Element { noun: db1_hash(noun), owner, attrs });
            self
        }
    }

    impl SpatialState for Tree {
        fn noun(&self, refno: RefNo) -> Option<u32> {
            self.elements.get(&refno).map(|element| element.noun)
        }
        fn owner(&self, refno: RefNo) -> Option<RefNo> {
            self.elements.get(&refno).and_then(|element| element.owner)
        }
        fn attr(&self, refno: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
            Ok(self.elements.get(&refno).and_then(|element| element.attrs.get(&hash).cloned()))
        }
        fn members(&self, refno: RefNo) -> Option<Vec<RefNo>> {
            let mut members: Vec<RefNo> =
                self.elements.iter().filter(|(_, element)| element.owner == Some(refno)).map(|(member, _)| *member).collect();
            members.sort_by_key(|member| (member.word0, member.word1));
            Some(members)
        }
    }

    const WORLD: RefNo = RefNo { word0: 16_192, word1: 0 };
    const SITE: RefNo = RefNo { word0: 24_384, word1: 22_399 };
    const ZONE: RefNo = RefNo { word0: 24_384, word1: 24_775 };
    const EQUI: RefNo = RefNo { word0: 24_384, word1: 24_776 };
    const NATIVE_BOX: RefNo = RefNo { word0: 24_384, word1: 24_777 };

    fn reals(values: [f64; 3]) -> DescriptorValue {
        DescriptorValue::RealArray(values.to_vec())
    }

    /// WORL, a SITE and a ZONE at the origin, and an EQUI posed as given.
    fn owners(equi_pos: [f64; 3], equi_ori: [f64; 3]) -> Tree {
        let at_origin = [("POS", reals([0.0; 3])), ("ORI", reals([0.0; 3]))];
        Tree::default()
            .with(WORLD, "WORL", None, &[])
            .with(SITE, "SITE", Some(WORLD), &at_origin)
            .with(ZONE, "ZONE", Some(SITE), &at_origin)
            .with(EQUI, "EQUI", Some(ZONE), &[("POS", reals(equi_pos)), ("ORI", reals(equi_ori))])
    }

    /// A BOX at its owner's origin, of these sizes, flags 2.
    fn a_box(tree: Tree, refno: RefNo, pos: [f64; 3], size: [f64; 3]) -> Tree {
        let [x, y, z] = size;
        tree.with(
            refno,
            "BOX",
            Some(EQUI),
            &[
                ("POS", reals(pos)),
                ("ORI", reals([0.0; 3])),
                ("XLEN", DescriptorValue::Real(x)),
                ("YLEN", DescriptorValue::Real(y)),
                ("ZLEN", DescriptorValue::Real(z)),
                ("OBST", DescriptorValue::Word { raw: 2, text: None }),
            ],
        )
    }

    /// The native BOX `24384/24777` and its owners as session 640 stores them.
    fn native_box() -> Tree {
        a_box(owners([-2821.27, 8003.96, 1400.0], [180.0, 0.0, -175.0]), NATIVE_BOX, [0.0; 3], [310.0, 590.0, 550.0])
    }

    fn compute(rules: &RuleSet, tree: &Tree, refno: RefNo) -> Result<AabbOutcome, UndecodableAttr> {
        BuiltInAabb::new(rules, &ChainComposer::new(rules)).compute(tree, refno)
    }

    #[test]
    fn the_candidates_give_the_native_boxs_stored_entry_word_for_word() {
        let tree = native_box();
        let rules = RuleSet::candidates(&candidates::pick());
        let AabbOutcome::Aabb(aabb) = compute(&rules, &tree, NATIVE_BOX).unwrap() else { panic!("a BOX") };
        let Encoded::Entries { entries, .. } = SpamapCodec::new(&rules).encode(NATIVE_BOX, &tree, &aabb).unwrap() else {
            panic!("the candidates encode a BOX")
        };
        let words = [-30_014i32, 76_966, 11_250, -26_411, 83_113, 16_750].map(|word| word as u32);
        let value = [words[0], words[1], words[2], words[3], words[4], words[5], 2];
        assert_eq!(entries, [SpamapEntry { key: [299_969_986, 24_384, 24_777, 0], value }]);
    }

    /// Requirement 6.17: 301 × 400 × 500 at E 1000 N 2000 U 3000, Y is N and Z is U.
    #[test]
    fn requirement_6_17s_box_is_bounded_exactly() {
        let box_26484 = RefNo::new(24_384, 26_484);
        let tree = a_box(owners([0.0; 3], [0.0; 3]), box_26484, [1000.0, 2000.0, 3000.0], [301.0, 400.0, 500.0]);
        let rules = RuleSet::candidates(&candidates::pick());
        assert_eq!(
            compute(&rules, &tree, box_26484).unwrap(),
            AabbOutcome::Aabb(WorldAabb::new([849.5, 1800.0, 2750.0], [1150.5, 2200.0, 3250.0]).unwrap())
        );
    }

    #[test]
    fn with_nothing_confirmed_every_rule_on_the_way_is_named() {
        assert_eq!(
            compute(&RuleSet::candidates_alone(&CandidatePick::default()), &native_box(), NATIVE_BOX).unwrap(),
            AabbOutcome::NeedsUnconfirmed(vec![
                RuleId::AabbOriForm,
                RuleId::AabbCompose,
                RuleId::AabbBox,
                RuleId::AabbArithmetic
            ])
        );
    }

    #[test]
    fn only_a_size_below_half_a_millimetre_consults_the_degenerate_rule() {
        let flat_refno = RefNo::new(24_384, 30_000);
        let flat = a_box(owners([0.0; 3], [0.0; 3]), flat_refno, [0.0; 3], [100.0, -200.0, 0.0]);
        let pick = CandidatePick { degenerate: None, ..candidates::pick() };
        assert_eq!(
            compute(&RuleSet::candidates_alone(&pick), &flat, flat_refno).unwrap(),
            AabbOutcome::NeedsUnconfirmed(vec![RuleId::AabbDegenerate])
        );
        assert!(matches!(compute(&RuleSet::candidates_alone(&pick), &native_box(), NATIVE_BOX).unwrap(), AabbOutcome::Aabb(_)));
        assert_eq!(
            compute(&RuleSet::candidates(&candidates::pick()), &flat, flat_refno).unwrap(),
            AabbOutcome::Aabb(WorldAabb::new([-50.0, -0.25, -0.25], [50.0, 0.25, 0.25]).unwrap()),
            "BOXELE measures −200 and 0 alike as 0.5"
        );
    }

    #[test]
    fn an_owners_undecodable_pos_is_reported_whatever_the_rules() {
        let tree = native_box().with(EQUI, "EQUI", Some(ZONE), &[("POS", DescriptorValue::Text("x".into()))]);
        for rules in [*RuleSet::confirmed(), RuleSet::candidates(&candidates::pick())] {
            assert_eq!(
                compute(&rules, &tree, NATIVE_BOX),
                Err(UndecodableAttr { refno: EQUI, name: "POS".into(), hash: db1_hash("POS") })
            );
        }
    }

    #[test]
    fn a_chain_that_misses_the_world_and_other_nouns_have_no_built_in_box() {
        let rules = RuleSet::candidates(&candidates::pick());
        let orphan = native_box().with(ZONE, "ZONE", None, &[]);
        assert_eq!(compute(&rules, &orphan, NATIVE_BOX).unwrap(), AabbOutcome::NoWorld);
        assert_eq!(compute(&rules, &native_box(), EQUI).unwrap(), AabbOutcome::NotBuiltIn);
        assert_eq!(compute(&rules, &native_box(), RefNo::new(24_384, 1)).unwrap(), AabbOutcome::NotBuiltIn);
    }

    #[test]
    fn a_leaning_cylinder_is_bounded_tighter_than_its_own_box() {
        let cyli = RefNo::new(24_384, 40_000);
        let cylinder = |ori: [f64; 3]| {
            owners([0.0; 3], ori).with(
                cyli,
                "CYLI",
                Some(EQUI),
                &[("DIAM", DescriptorValue::Real(200.0)), ("HEIG", DescriptorValue::Real(1000.0))],
            )
        };
        let bound = |tree: &Tree, cyli_aabb: crate::core_rules::candidate::CyliAabb| {
            let pick = CandidatePick { cyli_aabb: Some(cyli_aabb), ..candidates::pick() };
            match compute(&RuleSet::candidates_alone(&pick), tree, cyli).unwrap() {
                AabbOutcome::Aabb(aabb) => aabb.max_mm(),
                other => panic!("{other:?}"),
            }
        };
        let upright = cylinder([0.0; 3]);
        assert_eq!(bound(&upright, candidates::cylinder_about_its_centre), [100.0, 100.0, 500.0]);
        assert_eq!(bound(&upright, candidates::cylinder_by_its_own_box), [100.0, 100.0, 500.0]);

        // Turned 45° about x, then 30° about z: its axis leans into all three world axes.
        let leaning = cylinder([45.0, 0.0, 30.0]);
        let exact = bound(&leaning, candidates::cylinder_about_its_centre);
        let own_box = bound(&leaning, candidates::cylinder_by_its_own_box);
        assert!((exact[0] - 270.32).abs() < 0.01 && (own_box[0] - 298.73).abs() < 0.01, "{exact:?} {own_box:?}");
        assert!((exact[2] - own_box[2]).abs() < 1e-9, "z: one own axis does not lean into it");
    }

    /// A member of `noun` under `owner`, at `pos` on its owner's axes.
    fn member(tree: Tree, refno: RefNo, noun: &str, owner: RefNo, pos: [f64; 3], sizes: &[(&str, f64)]) -> Tree {
        let mut attrs = vec![("POS", reals(pos)), ("ORI", reals([0.0; 3]))];
        attrs.extend(sizes.iter().map(|&(name, mm)| (name, DescriptorValue::Real(mm))));
        tree.with(refno, noun, Some(owner), &attrs)
    }

    /// `24383/78369` of ams7999_0001 in small: a 105 × 35 × 108 BOX whose
    /// NBOX, 3 mm down and 105 high, reaches 1.5 mm below it.
    #[test]
    fn an_imap_3_member_reaches_out_of_its_owners_box() {
        let bx = RefNo::new(24_384, 50_000);
        let tree = a_box(owners([0.0; 3], [0.0; 3]), bx, [0.0; 3], [105.0, 35.0, 108.0]);
        let tree = member(tree, RefNo::new(24_384, 50_001), "NBOX", bx, [0.0, 0.0, -3.0], &[("XLEN", 101.0), ("YLEN", 35.0), ("ZLEN", 105.0)]);
        assert_eq!(
            compute(RuleSet::confirmed(), &tree, bx).unwrap(),
            AabbOutcome::Aabb(WorldAabb::new([-52.5, -17.5, -55.5], [52.5, 17.5, 54.0]).unwrap())
        );
    }

    #[test]
    fn cylinders_cones_and_pyramids_count_and_other_members_leave_no_box() {
        let bx = RefNo::new(24_384, 50_000);
        let with = |noun: &str, sizes: &[(&str, f64)]| {
            let tree = a_box(owners([0.0; 3], [0.0; 3]), bx, [0.0; 3], [10.0, 10.0, 10.0]);
            member(tree, RefNo::new(24_384, 50_001), noun, bx, [0.0, 0.0, 10.0], sizes)
        };
        let boxed = |min: [f64; 3], max: [f64; 3]| AabbOutcome::Aabb(WorldAabb::new(min, max).unwrap());
        let rules = RuleSet::confirmed();
        // DIAM 20, HEIG 4, centred 10 up.
        let ncyl = with("NCYL", &[("DIAM", 20.0), ("HEIG", 4.0)]);
        assert_eq!(compute(rules, &ncyl, bx).unwrap(), boxed([-10.0, -10.0, -5.0], [10.0, 10.0, 12.0]));
        // DTOP 30 at the top end, DBOT 2 at the bottom.
        let ncon = with("NCON", &[("HEIG", 4.0), ("DTOP", 30.0), ("DBOT", 2.0)]);
        assert_eq!(compute(rules, &ncon, bx).unwrap(), boxed([-15.0, -15.0, -5.0], [15.0, 15.0, 12.0]));
        // The top face 2 × 2 half an XOFF of 8 one way, the bottom 4 × 6 the other.
        let sizes = [("XTOP", 2.0), ("YTOP", 2.0), ("XBOT", 4.0), ("YBOT", 6.0), ("HEIG", 4.0), ("XOFF", 8.0), ("YOFF", 0.0)];
        assert_eq!(compute(rules, &with("NPYR", &sizes), bx).unwrap(), boxed([-6.0, -5.0, -5.0], [5.0, 5.0, 12.0]));
        assert_eq!(compute(rules, &with("NSNO", &[("HEIG", 4.0)]), bx).unwrap(), AabbOutcome::MissingInput(UNBUILT_MEMBER));
    }

    #[test]
    fn the_members_rule_is_asked_only_for_an_element_with_members() {
        let rules = RuleSet::candidates_alone(&CandidatePick { members: None, ..candidates::pick() });
        assert!(matches!(compute(&rules, &native_box(), NATIVE_BOX).unwrap(), AabbOutcome::Aabb(_)));
        let bx = RefNo::new(24_384, 50_000);
        let tree = a_box(owners([0.0; 3], [0.0; 3]), bx, [0.0; 3], [10.0; 3]);
        let tree = member(tree, RefNo::new(24_384, 50_001), "NBOX", bx, [0.0; 3], &[("XLEN", 1.0), ("YLEN", 1.0), ("ZLEN", 1.0)]);
        assert_eq!(compute(&rules, &tree, bx).unwrap(), AabbOutcome::NeedsUnconfirmed(vec![RuleId::AabbMembers]));
    }
}
