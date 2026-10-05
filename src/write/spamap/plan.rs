use super::{EndState, SpatialStage};
use crate::core_rules::{self, DescendantKind, RuleSet, Trigger};
use crate::index::SpamapEntry;
use crate::refno::RefNo;
use crate::spatial::aabb::{AabbOutcome, BuiltInAabb, ChainComposer};
use crate::spatial::codec::{Encoded, SpamapCodec};
use crate::spatial::{
    AabbSource, ElementSpatial, MissingAabb, MissingReason, RecomputeCause, SpatialError,
    SpatialPolicy, SpatialReport, SpatialState,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub(crate) struct SpamapEditSet {
    pub(crate) deletes: BTreeSet<[u32; 4]>,
    pub(crate) inserts: BTreeMap<[u32; 4], [u32; 7]>,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct SpamapPlan {
    pub(crate) edits: SpamapEditSet,
    pub(crate) report: SpatialReport,
    pub(crate) record_items: Vec<RecordItemPatch>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordItemPatch {
    pub(crate) refno: RefNo,
    pub(crate) key0: u32,
    pub(crate) k: u32,
}

trait RecomputeView: SpatialState {
    fn previous_state(&self) -> &dyn SpatialState;
    fn drafts(&self) -> &BTreeMap<RefNo, Option<super::super::RecordDraft>>;
    fn inserted(&self, r: RefNo) -> bool;
    fn old_record(&self, r: RefNo) -> Option<super::super::RecordDraft>;
    fn final_record(&self, r: RefNo) -> Option<super::super::RecordDraft>;
    fn changed_attrs(
        &self,
        old: &super::super::RecordDraft,
        new: &super::super::RecordDraft,
    ) -> Vec<u32>;
    fn children(&self, r: RefNo, deleted: bool) -> Vec<RefNo>;
}
impl RecomputeView for EndState<'_, '_> {
    fn previous_state(&self) -> &dyn SpatialState {
        self.previous
    }
    fn drafts(&self) -> &BTreeMap<RefNo, Option<super::super::RecordDraft>> {
        &self.drafts
    }
    fn inserted(&self, r: RefNo) -> bool {
        self.inserted.contains(&r)
    }
    fn old_record(&self, r: RefNo) -> Option<super::super::RecordDraft> {
        self.previous.record(r).ok().flatten()
    }
    fn final_record(&self, r: RefNo) -> Option<super::super::RecordDraft> {
        self.record(r).ok().flatten()
    }
    fn changed_attrs(
        &self,
        old: &super::super::RecordDraft,
        new: &super::super::RecordDraft,
    ) -> Vec<u32> {
        self.previous.changed_attrs(old, new)
    }
    fn children(&self, r: RefNo, deleted: bool) -> Vec<RefNo> {
        self.children(r, deleted)
    }
}

fn geometric(noun: u32) -> bool {
    matches!(crate::spatial::rules::imap_of(noun), 1 | 5 | 7)
}
fn add(set: &mut BTreeMap<RefNo, Vec<RecomputeCause>>, r: RefNo, cause: RecomputeCause) {
    let causes = set.entry(r).or_default();
    if !causes.contains(&cause) {
        causes.push(cause);
    }
}
fn descendants(
    end: &impl RecomputeView,
    ancestor: RefNo,
    kind: DescendantKind,
    deleted: bool,
    cause: RecomputeCause,
    set: &mut BTreeMap<RefNo, Vec<RecomputeCause>>,
) {
    if !deleted
        && !end
            .noun(ancestor)
            .is_some_and(crate::spatial::rules::mpbox_walks_below)
    {
        return;
    }
    let mut stack: Vec<_> = end
        .children(ancestor, deleted)
        .into_iter()
        .map(|r| (r, 1))
        .collect();
    let mut seen = HashSet::from([ancestor]);
    while let Some((r, depth)) = stack.pop() {
        if !seen.insert(r) {
            continue;
        }
        if deleted
            || (end.noun(r).is_some_and(geometric) && core_rules::descendants_included(kind, depth))
        {
            add(set, r, cause.clone());
        }
        if deleted
            || end
                .noun(r)
                .is_some_and(crate::spatial::rules::mpbox_walks_below)
        {
            stack.extend(end.children(r, deleted).into_iter().map(|r| (r, depth + 1)));
        }
    }
}
fn first_non3(state: &dyn SpatialState, r: RefNo) -> Option<RefNo> {
    let mut at = state.owner(r);
    let mut seen = HashSet::new();
    while let Some(r) = at {
        if !seen.insert(r) {
            return None;
        }
        let noun = state.noun(r)?;
        if crate::spatial::rules::imap_of(noun) != 3 {
            return geometric(noun).then_some(r);
        }
        at = state.owner(r);
    }
    None
}

pub(crate) fn recompute_set(end: &EndState<'_, '_>) -> BTreeMap<RefNo, Vec<RecomputeCause>> {
    collect_recompute_set(end)
}
fn collect_recompute_set(end: &impl RecomputeView) -> BTreeMap<RefNo, Vec<RecomputeCause>> {
    let mut set = BTreeMap::new();
    // Reordered children need not have a draft themselves.
    let mut reordered = BTreeSet::new();
    for (&r, draft) in end.drafts() {
        if let (Some(draft), Some(old)) = (draft, end.old_record(r)) {
            let members: HashSet<_> = draft.members().iter().copied().collect();
            let shared: HashSet<_> = old
                .members()
                .iter()
                .copied()
                .filter(|m| members.contains(m))
                .collect();
            let before: Vec<_> = old
                .members()
                .iter()
                .filter(|m| shared.contains(m))
                .copied()
                .collect();
            let after: Vec<_> = draft
                .members()
                .iter()
                .filter(|m| shared.contains(m))
                .copied()
                .collect();
            for (a, b) in before.iter().zip(&after) {
                if a != b {
                    reordered.insert(*a);
                    reordered.insert(*b);
                }
            }
        }
    }
    let touched: BTreeSet<_> = end
        .drafts()
        .keys()
        .chain(reordered.iter())
        .copied()
        .collect();
    for r in touched {
        let old = end.old_record(r);
        let new = end.final_record(r);
        if end.drafts().get(&r).is_some_and(Option::is_none) {
            add(&mut set, r, RecomputeCause::Deleted);
            descendants(
                end,
                r,
                DescendantKind::Move,
                true,
                RecomputeCause::AncestorDeleted(r),
                &mut set,
            );
        }
        let inserted = end.inserted(r);
        let moved = old
            .as_ref()
            .zip(new.as_ref())
            .is_some_and(|(a, b)| a.owner != b.owner);
        let noun = new
            .as_ref()
            .or(old.as_ref())
            .map(|d| d.noun_hash)
            .unwrap_or(0);
        let changed: Vec<_> = old
            .as_ref()
            .zip(new.as_ref())
            .map(|(a, b)| end.changed_attrs(a, b))
            .unwrap_or_default()
            .into_iter()
            .filter(|h| core_rules::recompute_row(noun, *h) != Trigger::DoesNotTrigger)
            .collect();
        let order =
            reordered.contains(&r) && core_rules::member_order_row(noun) != Trigger::DoesNotTrigger;
        if geometric(noun) {
            if inserted {
                add(&mut set, r, RecomputeCause::Inserted);
            }
            if !changed.is_empty() {
                add(&mut set, r, RecomputeCause::OwnAttrs(changed.clone()));
            }
            if moved {
                add(&mut set, r, RecomputeCause::OwnerChanged);
            }
            if order {
                add(&mut set, r, RecomputeCause::MemberOrder);
            }
        }
        if moved {
            descendants(
                end,
                r,
                DescendantKind::Move,
                false,
                RecomputeCause::AncestorOwner(r),
                &mut set,
            );
        }
        if order {
            descendants(
                end,
                r,
                DescendantKind::Move,
                false,
                RecomputeCause::AncestorMemberOrder(r),
                &mut set,
            );
        }
        if !changed.is_empty() {
            let kind = if changed.contains(&e3d_attlib::db1_hash("ORI")) {
                DescendantKind::Ori
            } else {
                DescendantKind::Pos
            };
            descendants(
                end,
                r,
                kind,
                false,
                RecomputeCause::AncestorAttrs {
                    ancestor: r,
                    attrs: changed.clone(),
                },
                &mut set,
            );
        }
        if crate::spatial::rules::imap_of(noun) == 3
            && (inserted || moved || order || !changed.is_empty() || new.is_none())
        {
            if new.is_some() {
                if let Some(a) = first_non3(end, r) {
                    add(&mut set, a, RecomputeCause::Imap3Member(r));
                }
            }
            if moved || new.is_none() {
                if let Some(a) = first_non3(end.previous_state(), r) {
                    add(&mut set, a, RecomputeCause::Imap3Member(r));
                }
            }
        }
    }
    set
}

pub(crate) fn plan(
    stage: &SpatialStage,
    end: &EndState<'_, '_>,
    remint: &[(RefNo, RefNo)],
    previous_entries: Option<&[SpamapEntry]>,
    set: BTreeMap<RefNo, Vec<RecomputeCause>>,
) -> Result<SpamapPlan, SpatialError> {
    let mut result = SpamapPlan {
        report: stage.report(),
        ..SpamapPlan::default()
    };
    if stage.policy == SpatialPolicy::Off {
        return Ok(result);
    }
    let remint: HashMap<_, _> = remint.iter().copied().collect();
    for &given in stage.given.keys() {
        if !set.contains_key(remint.get(&given).unwrap_or(&given)) {
            result.report.unused_aabb.push(given);
        }
    }
    result.report.unused_aabb.sort();
    let codec = SpamapCodec::new(RuleSet::confirmed());
    let composer = ChainComposer::new(RuleSet::confirmed());
    let built_in = BuiltInAabb::new(RuleSet::confirmed(), &composer);
    let mut old_by_refno: HashMap<RefNo, Vec<SpamapEntry>> = HashMap::new();
    for entry in previous_entries.unwrap_or_default() {
        old_by_refno.entry(entry.refno()).or_default().push(*entry);
    }
    let mut missing = Vec::new();
    for (r, causes) in set {
        let given_as = remint
            .iter()
            .find_map(|(&given, &written)| (written == r).then_some(given));
        let given = stage.given.get(&given_as.unwrap_or(r));
        let old = old_by_refno.remove(&r).unwrap_or_default();
        let mut target = Vec::new();
        let mut source = AabbSource::None;
        let mut reasons = Vec::new();
        let exists = end.noun(r).is_some();
        let blocker = exists.then(|| end.blocking_ancestor(r)).flatten();
        if let Some(ancestor) = blocker {
            if end.base.is_some_and(|base| {
                base.noun(ancestor).is_some() && end.previous.noun(ancestor).is_none()
            }) {
                result.report.unreachable.push((r, ancestor));
            }
        } else if exists && end.noun(r).is_some_and(geometric) {
            let invalidated = given.is_some() && end.given_invalidated(r);
            let box_value = if let Some(aabb) = given.filter(|_| !invalidated) {
                source = AabbSource::Given;
                Some(*aabb)
            } else {
                match built_in.compute(end, r).map_err(spatial_attr)? {
                    AabbOutcome::Aabb(aabb) => {
                        source = AabbSource::BuiltIn;
                        Some(aabb)
                    }
                    AabbOutcome::NeedsUnconfirmed(rules) => {
                        reasons.push(MissingReason::Unconfirmed(
                            rules.iter().map(|r| r.name()).collect(),
                        ));
                        None
                    }
                    AabbOutcome::MissingInput(input) => {
                        reasons.push(MissingReason::MissingInput(input));
                        None
                    }
                    AabbOutcome::NoWorld => None,
                    AabbOutcome::NotBuiltIn => {
                        reasons.push(MissingReason::NoValue);
                        None
                    }
                }
            };
            if invalidated && box_value.is_none() {
                reasons.insert(0, MissingReason::Invalidated);
            }
            if let Some(aabb) = box_value {
                match codec.encode(r, end, &aabb).map_err(codec_error)? {
                    Encoded::Entries { entries, .. } => target = entries,
                    Encoded::NeedsUnconfirmed(rules) => reasons.push(MissingReason::Unconfirmed(
                        rules.iter().map(|r| r.name()).collect(),
                    )),
                    Encoded::MissingInput(input) => {
                        reasons.push(MissingReason::MissingInput(input))
                    }
                }
            }
        }
        if !reasons.is_empty() {
            missing.push(MissingAabb {
                refno: r,
                given_as,
                reasons,
            });
        }
        // A same-key changed value must be withdrawn before insertion (529 preserves an existing value).
        if old.len() != target.len() || old.iter().any(|e| !target.contains(e)) {
            for e in &old {
                result.edits.deletes.insert(e.key);
            }
            for e in &target {
                result.edits.inserts.insert(e.key, e.value);
            }
        }
        for e in &target {
            if !old
                .iter()
                .any(|o| o.key[3] == e.key[3] && o.key[0] == e.key[0])
            {
                result.record_items.push(RecordItemPatch {
                    refno: r,
                    key0: e.key[0],
                    k: e.key[3],
                });
            }
        }
        if !target.is_empty() {
            for e in &old {
                if !target.iter().any(|new| new.key[3] == e.key[3]) {
                    result.record_items.push(RecordItemPatch {
                        refno: r,
                        key0: 0x8000_0001,
                        k: e.key[3],
                    });
                }
            }
        }
        result.report.elements.insert(
            r,
            ElementSpatial {
                given_as,
                causes,
                aabb: source,
                registered: target
                    .iter()
                    .filter(|e| !old.contains(e))
                    .copied()
                    .collect(),
                removed: old
                    .iter()
                    .filter(|e| !target.contains(e))
                    .copied()
                    .collect(),
            },
        );
    }
    end.previous.check_reads()?;
    if let Some(base) = end.base {
        base.check_reads()?;
    }
    if !missing.is_empty() {
        if stage.policy == SpatialPolicy::Require {
            return Err(SpatialError::MissingAabb(missing));
        }
        result.report.skipped = missing;
    }
    Ok(result)
}
fn spatial_attr(e: crate::spatial::UndecodableAttr) -> SpatialError {
    SpatialError::UndecodableAttr {
        refno: e.refno,
        name: e.name,
        hash: e.hash,
    }
}
fn codec_error(error: crate::spatial::codec::CodecError) -> SpatialError {
    match error {
        crate::spatial::codec::CodecError::Undecodable(e) => spatial_attr(e),
        crate::spatial::codec::CodecError::OutOfRange {
            refno,
            axis,
            value_mm,
            range,
        } => SpatialError::OutOfRange {
            refno,
            axis,
            value_mm,
            range,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::RecordDraft;
    use super::*;
    use crate::record::descriptor::DescriptorValue;
    use crate::spatial::UndecodableAttr;
    use proptest::prelude::*;

    #[derive(Default)]
    struct Tree(BTreeMap<RefNo, RecordDraft>);
    impl SpatialState for Tree {
        fn noun(&self, r: RefNo) -> Option<u32> {
            self.0.get(&r).map(|d| d.noun_hash)
        }
        fn owner(&self, r: RefNo) -> Option<RefNo> {
            self.0.get(&r).map(|d| d.owner).filter(|r| r.is_valid())
        }
        fn members(&self, r: RefNo) -> Option<Vec<RefNo>> {
            self.0.get(&r).map(|d| d.members().to_vec())
        }
        fn attr(&self, _: RefNo, _: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
            Ok(None)
        }
    }
    struct Model {
        old: Tree,
        final_tree: Tree,
        edits: BTreeMap<RefNo, Option<RecordDraft>>,
        inserted: RefNo,
    }
    impl SpatialState for Model {
        fn noun(&self, r: RefNo) -> Option<u32> {
            self.final_tree.noun(r)
        }
        fn owner(&self, r: RefNo) -> Option<RefNo> {
            self.final_tree.owner(r)
        }
        fn members(&self, r: RefNo) -> Option<Vec<RefNo>> {
            self.final_tree.members(r)
        }
        fn attr(&self, _: RefNo, _: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
            Ok(None)
        }
    }
    impl RecomputeView for Model {
        fn previous_state(&self) -> &dyn SpatialState {
            &self.old
        }
        fn drafts(&self) -> &BTreeMap<RefNo, Option<RecordDraft>> {
            &self.edits
        }
        fn inserted(&self, r: RefNo) -> bool {
            r == self.inserted
        }
        fn old_record(&self, r: RefNo) -> Option<RecordDraft> {
            self.old.0.get(&r).cloned()
        }
        fn final_record(&self, r: RefNo) -> Option<RecordDraft> {
            self.final_tree.0.get(&r).cloned()
        }
        fn changed_attrs(&self, a: &RecordDraft, b: &RecordDraft) -> Vec<u32> {
            if a.implicit()[11] == b.implicit()[11] {
                vec![]
            } else {
                vec![e3d_attlib::db1_hash(
                    if geometric(b.noun_hash) || crate::spatial::rules::imap_of(b.noun_hash) == 3 {
                        "XLEN"
                    } else {
                        "POS"
                    },
                )]
            }
        }
        fn children(&self, r: RefNo, deleted: bool) -> Vec<RefNo> {
            let mut result = self
                .final_tree
                .0
                .iter()
                .filter(|(_, d)| d.owner == r)
                .map(|(r, _)| *r)
                .collect::<Vec<_>>();
            if deleted {
                result.extend(
                    self.old
                        .0
                        .iter()
                        .filter(|(child, d)| !self.final_tree.0.contains_key(child) && d.owner == r)
                        .map(|(r, _)| *r),
                );
            }
            result
        }
    }
    fn r(n: usize) -> RefNo {
        RefNo::new(1, n as u32 + 1)
    }
    fn draft(n: usize, noun: &str, owner: RefNo) -> RecordDraft {
        RecordDraft::new(
            r(n),
            e3d_attlib::db1_hash(noun),
            owner,
            vec![0; 12],
            vec![],
            vec![],
        )
        .unwrap()
    }
    fn thread_members(tree: &mut Tree, reverse: bool) {
        let children = tree
            .0
            .iter()
            .map(|(r, d)| (d.owner, *r))
            .collect::<Vec<_>>();
        for d in tree.0.values_mut() {
            let mut members = children
                .iter()
                .filter(|(owner, _)| *owner == d.refno)
                .map(|(_, r)| *r)
                .collect::<Vec<_>>();
            if reverse {
                members.reverse();
            }
            for member in members {
                d.push_member(member);
            }
        }
    }
    // Direct set definition: examine every possible target and each changed
    // ancestor, independently of the production traversal/queue implementation.
    fn reference(model: &Model) -> BTreeSet<RefNo> {
        let mut result = BTreeSet::new();
        let mut moved = BTreeSet::new();
        let mut attrs = BTreeSet::new();
        let mut order = BTreeSet::new();
        let deleted = model
            .old
            .0
            .keys()
            .filter(|r| !model.final_tree.0.contains_key(r))
            .copied()
            .collect::<BTreeSet<_>>();
        for (&id, new) in &model.final_tree.0 {
            if let Some(old) = model.old.0.get(&id) {
                if old.owner != new.owner {
                    moved.insert(id);
                }
                if old.implicit()[11] != new.implicit()[11] {
                    attrs.insert(id);
                }
            }
        }
        for (&id, new) in &model.final_tree.0 {
            if let Some(old) = model.old.0.get(&id) {
                let before = old
                    .members()
                    .iter()
                    .filter(|r| new.members().contains(r))
                    .collect::<Vec<_>>();
                let after = new
                    .members()
                    .iter()
                    .filter(|r| old.members().contains(r))
                    .collect::<Vec<_>>();
                for member in &before {
                    if before.iter().position(|r| r == member)
                        != after.iter().position(|r| r == member)
                    {
                        order.insert(**member);
                    }
                }
            }
        }
        for (&id, new) in &model.final_tree.0 {
            if geometric(new.noun_hash)
                && (id == model.inserted
                    || attrs.contains(&id)
                    || order.contains(&id)
                    || moved.contains(&id))
            {
                result.insert(id);
            }
            let mut at = Some(new.owner);
            let mut traversable = true;
            while let Some(ancestor) = at {
                if deleted.contains(&ancestor) {
                    result.insert(id);
                }
                let node = model
                    .final_tree
                    .0
                    .get(&ancestor)
                    .or_else(|| model.old.0.get(&ancestor));
                let Some(node) = node else { break };
                traversable &= crate::spatial::rules::mpbox_walks_below(node.noun_hash);
                if geometric(new.noun_hash)
                    && traversable
                    && (moved.contains(&ancestor)
                        || order.contains(&ancestor)
                        || attrs.contains(&ancestor))
                {
                    result.insert(id);
                }
                at = node.owner.is_valid().then_some(node.owner);
            }
        }
        result.extend(&deleted);
        // Deleted descendants still have a previous owner chain.
        for &id in &deleted {
            let mut at = model.old.owner(id);
            while let Some(parent) = at {
                if deleted.contains(&parent) {
                    result.insert(id);
                }
                at = model.old.owner(parent);
            }
        }
        for id in model
            .old
            .0
            .keys()
            .chain(model.final_tree.0.keys())
            .copied()
            .collect::<BTreeSet<_>>()
        {
            let noun = model.noun(id).or_else(|| model.old.noun(id)).unwrap();
            if crate::spatial::rules::imap_of(noun) != 3 {
                continue;
            }
            if id != model.inserted
                && !deleted.contains(&id)
                && !moved.contains(&id)
                && !attrs.contains(&id)
                && !order.contains(&id)
            {
                continue;
            }
            for state in [&model.final_tree as &dyn SpatialState, &model.old] {
                if std::ptr::eq(state, &model.old as &dyn SpatialState)
                    && !deleted.contains(&id)
                    && !moved.contains(&id)
                {
                    continue;
                }
                let mut at = state.owner(id);
                while let Some(parent) = at {
                    let Some(noun) = state.noun(parent) else {
                        break;
                    };
                    if crate::spatial::rules::imap_of(noun) != 3 {
                        if geometric(noun) {
                            result.insert(parent);
                        }
                        break;
                    }
                    at = state.owner(parent);
                }
            }
        }
        result
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn recompute_matches_all_seven_clauses(nodes in prop::collection::vec((any::<u8>(), 0u8..5, any::<bool>(), any::<bool>()), 3..18), reverse in any::<bool>()) {
            let mut old=Tree::default(); old.0.insert(r(0),draft(0,"WORL",RefNo::new(0,0)));
            for (i,(parent,noun,_,_)) in nodes.iter().enumerate() {
                let index=i+1;
                old.0.insert(r(index),draft(index,["ZONE","EQUI","BOX","NBOX","BRAN"][*noun as usize],r(*parent as usize%index)));
            }
            thread_members(&mut old,false);
            let mut final_tree=Tree::default();
            for (&id,d) in &old.0 {
                let index=id.word1 as usize-1;
                if index>0 && nodes[index-1].3 { continue; }
                let mut new=d.clone();
                for member in d.members() { new.remove_member(*member); }
                if index>0 && nodes[index-1].2 { *new.implicit_word_mut(11).unwrap()=1; }
                // Reparent selected elements without introducing cycles.
                if index>1 && nodes[index-1].0%3==0 { new.owner=r(0); }
                final_tree.0.insert(id,new);
            }
            let inserted=r(nodes.len()+1);
            final_tree.0.insert(inserted,draft(nodes.len()+1,"NBOX",r(1)));
            thread_members(&mut final_tree,reverse);
            let mut edits=BTreeMap::new();
            for (&id,d) in &old.0 {
                match final_tree.0.get(&id) {
                    Some(new) if !d.content_eq(new) => { edits.insert(id,Some(new.clone())); }
                    None => { edits.insert(id,None); }
                    _ => {},
                }
            }
            edits.insert(inserted,Some(final_tree.0[&inserted].clone()));
            let model=Model { old,final_tree,edits,inserted };
            prop_assert_eq!(collect_recompute_set(&model).into_keys().collect::<BTreeSet<_>>(),reference(&model));
        }
    }
}
