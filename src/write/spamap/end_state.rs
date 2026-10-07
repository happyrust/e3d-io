use super::super::{ElementWrite, RecordDraft, WriteError};
use crate::record::descriptor::{AttributeExtractionStatus, DescriptorValue, ElementExtraction};
use crate::record::element::ParsedElement;
use crate::record::template::TemplateProvider;
use crate::refno::RefNo;
use crate::spatial::{SpatialState, UndecodableAttr};
use crate::ReadOnlyEngine;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

pub(crate) type Templates<'a> = Rc<RefCell<&'a mut TemplateProvider>>;
type Records = HashMap<RefNo, Result<Option<Rc<RecordDraft>>, String>>;
type Values = RefCell<HashMap<(RefNo, u32), Result<Option<DescriptorValue>, UndecodableAttr>>>;

/// Lazy exact-session view; merge views share the caller's schema.
pub(crate) struct Snapshot<'a> {
    engine: RefCell<ReadOnlyEngine>,
    templates: Templates<'a>,
    records: RefCell<Records>,
    values: Values,
}
impl<'a> Snapshot<'a> {
    pub(crate) fn open(
        path: &Path,
        sesno: u32,
        templates: Templates<'a>,
    ) -> Result<Self, WriteError> {
        Ok(Self {
            engine: RefCell::new(ReadOnlyEngine::open_at(path, sesno)?),
            templates,
            records: RefCell::new(HashMap::new()),
            values: RefCell::default(),
        })
    }
    pub(crate) fn record(&self, r: RefNo) -> Result<Option<RecordDraft>, String> {
        self.shared_record(r).map(|d| d.map(|d| (*d).clone()))
    }
    fn shared_record(&self, r: RefNo) -> Result<Option<Rc<RecordDraft>>, String> {
        if let Some(record) = self.records.borrow().get(&r) {
            return record.clone();
        }
        let record = self
            .engine
            .borrow_mut()
            .find_element(r)
            .map_err(|e| e.to_string())
            .and_then(|v| {
                v.map(|v| {
                    RecordDraft::from_record_bytes(&v.raw_bytes)
                        .map(Rc::new)
                        .map_err(|e| e.to_string())
                }).transpose()
            });
        self.records.borrow_mut().insert(r, record.clone());
        record
    }
    pub(crate) fn check_reads(&self) -> Result<(), crate::spatial::SpatialError> {
        let records = self.records.borrow();
        if let Some((&refno, Err(detail))) = records
            .iter()
            .filter(|(_, value)| value.is_err())
            .min_by_key(|(r, _)| **r)
        {
            return Err(crate::spatial::SpatialError::UndecodableAttr {
                refno,
                name: detail.clone(),
                hash: 0,
            });
        }
        Ok(())
    }
    fn value(
        &self,
        draft: &RecordDraft,
        hash: u32,
    ) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        let error = || UndecodableAttr {
            refno: draft.refno,
            hash,
            name: e3d_attlib::lookup_system_name(hash)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{hash:#x}")),
        };
        let extraction = self.extract(draft).map_err(|_| error())?;
        Self::extracted_value(&extraction, hash).map_err(|_| error())
    }
    fn extracted_value(extraction: &ElementExtraction, hash: u32) -> Result<Option<DescriptorValue>, ()> {
        let Some(row) = extraction.attributes.iter().find(|a| a.hash == hash) else {
            return Ok(None);
        };
        match (&row.value, &row.status) {
            (Some(v), _) => Ok(Some(v.clone())),
            (None, AttributeExtractionStatus::NonImplicit) => Ok(None),
            _ => Err(()),
        }
    }
    fn extract(&self, draft: &RecordDraft) -> Result<ElementExtraction, ()> {
        let error = || ();
        let text = self.engine.borrow().text_format();
        let mut parsed =
            ParsedElement::parse_as(&draft.encode(0, 0, 0).bytes, text).map_err(|_| error())?;
        // Pending hunks have no disk address; decode before following retained entries.
        for pending in draft.pending_hunks() {
            if let Some(a) = parsed
                .explicit_attributes
                .iter_mut()
                .find(|a| a.hash == pending.hash)
            {
                let words = [1, 0, 0, pending.count, pending.payload.len() as u32];
                let hunk =
                    crate::record::hunk::HunkRef::from_words(pending.hunk_type << 10, &words)
                        .ok_or_else(error)?;
                a.value = crate::record::hunk::decode_hunk_value(&hunk, &pending.payload, text);
            }
        }
        self.engine
            .borrow_mut()
            .resolve_hunks(&mut parsed)
            .map_err(|_| error())?;
        let mut templates = self.templates.borrow_mut();
        let template = templates
            .template_for(draft.noun_hash)
            .map_err(|_| error())?
            .ok_or_else(error)?;
        crate::engine::extract_parsed_with_optional_dictionary(
            draft.refno,
            &parsed,
            None,
            template,
        )
        .map_err(|_| error())
    }
    pub(crate) fn changed_attrs(&self, old: &RecordDraft, new: &RecordDraft) -> Vec<u32> {
        let hashes: Vec<u32> = self
            .templates
            .borrow_mut()
            .template_for(new.noun_hash)
            .ok()
            .flatten()
            .map(|t| t.attrs.iter().map(|a| a.hash).collect())
            .unwrap_or_default();
        // These are two immutable versions. Extract each once; comparing each
        // descriptor used to re-encode and re-extract the entire record.
        let old_values = self.extract(old);
        let new_values = self.extract(new);
        hashes
            .into_iter()
            .filter(|h| match (
                old_values.as_ref().map_err(|_| ()).and_then(|e| Self::extracted_value(e, *h)),
                new_values.as_ref().map_err(|_| ()).and_then(|e| Self::extracted_value(e, *h)),
            ) {
                (Ok(a), Ok(b)) => a != b,
                _ => {
                    let mut templates = self.templates.borrow_mut();
                    let attr = templates
                        .template_for(new.noun_hash)
                        .ok()
                        .flatten()
                        .and_then(|t| t.attrs.iter().find(|a| a.hash == *h));
                    match attr {
                        Some(a) if a.wide.location.word_offset() != 0 => {
                            let at = a.wide.location.word_offset() as usize;
                            let to = at + a.wide.storage_words as usize;
                            old.implicit().get(at..to) != new.implicit().get(at..to)
                        }
                        _ => {
                            old.explicit_words(*h) != new.explicit_words(*h)
                                || old
                                    .pending_hunks()
                                    .iter()
                                    .find(|a| a.hash == *h)
                                    .map(|a| &a.payload)
                                    != new
                                        .pending_hunks()
                                        .iter()
                                        .find(|a| a.hash == *h)
                                        .map(|a| &a.payload)
                        }
                    }
                }
            })
            .collect()
    }
}
impl SpatialState for Snapshot<'_> {
    fn noun(&self, r: RefNo) -> Option<u32> {
        self.shared_record(r).ok().flatten().map(|d| d.noun_hash)
    }
    fn owner(&self, r: RefNo) -> Option<RefNo> {
        self.shared_record(r)
            .ok()
            .flatten()
            .map(|d| d.owner)
            .filter(|r| r.is_valid())
    }
    fn members(&self, r: RefNo) -> Option<Vec<RefNo>> {
        self.shared_record(r).ok().flatten().map(|d| d.members().to_vec())
    }
    fn attr(&self, r: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        if let Some(value) = self.values.borrow().get(&(r, hash)) {
            return value.clone();
        }
        let value = match self.shared_record(r) {
            Ok(Some(d)) => self.value(&d, hash),
            Ok(None) => Ok(None),
            Err(_) => Err(UndecodableAttr {
                refno: r,
                hash,
                name: format!("{hash:#x}"),
            }),
        };
        self.values.borrow_mut().insert((r, hash), value.clone());
        value
    }
}

/// Committed records overlaid with the final, possibly merged batch.
pub(crate) struct EndState<'a, 't> {
    pub(crate) previous: &'a Snapshot<'t>,
    pub(crate) base: Option<&'a Snapshot<'t>>,
    pub(crate) drafts: BTreeMap<RefNo, Option<RecordDraft>>,
    pub(crate) inserted: HashSet<RefNo>,
    values: Values,
    children_by_owner: BTreeMap<RefNo, Vec<RefNo>>,
}
impl<'a, 't> EndState<'a, 't> {
    pub(crate) fn new(
        previous: &'a Snapshot<'t>,
        base: Option<&'a Snapshot<'t>>,
        elements: &[ElementWrite],
    ) -> Self {
        let mut drafts = BTreeMap::new();
        let mut inserted = HashSet::new();
        for e in elements {
            match e {
                ElementWrite::Insert(d) => {
                    inserted.insert(d.refno);
                    drafts.insert(d.refno, Some(d.clone()));
                }
                ElementWrite::Update(d) => {
                    drafts.insert(d.refno, Some(d.clone()));
                }
                ElementWrite::Delete(r) => {
                    drafts.insert(*r, None);
                }
            }
        }
        let mut children_by_owner: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (&child, draft) in &drafts {
            if let Some(draft) = draft {
                children_by_owner.entry(draft.owner).or_default().push(child);
            }
        }
        Self {
            previous,
            base,
            drafts,
            inserted,
            values: RefCell::default(),
            children_by_owner,
        }
    }
    pub(crate) fn record(&self, r: RefNo) -> Result<Option<RecordDraft>, String> {
        self.drafts
            .get(&r)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| self.previous.record(r))
    }
    pub(crate) fn children(&self, r: RefNo, deleted: bool) -> Vec<RefNo> {
        let mut members = if deleted {
            self.previous.members(r).unwrap_or_default()
        } else {
            self.members(r).unwrap_or_default()
        };
        members.retain(|m| {
            self.owner(*m) == Some(r) || (deleted && matches!(self.drafts.get(m), Some(None)))
        });
        let mut present: HashSet<_> = members.iter().copied().collect();
        for &child in self.children_by_owner.get(&r).into_iter().flatten() {
            if present.insert(child) {
                members.push(child);
            }
        }
        members
    }
    pub(crate) fn blocking_ancestor(&self, r: RefNo) -> Option<RefNo> {
        let mut seen = HashSet::new();
        let mut at = r;
        loop {
            if !seen.insert(at) {
                return Some(at);
            }
            let Some(noun) = self.noun(at) else {
                return Some(at);
            };
            if noun == e3d_attlib::db1_hash("WORL") {
                return None;
            }
            let Some(owner) = self.owner(at) else {
                return Some(at);
            };
            at = owner;
        }
    }
    pub(crate) fn given_invalidated(&self, r: RefNo) -> bool {
        let Some(base) = self.base else { return false };
        if !self.inserted.contains(&r) {
            match (base.record(r), self.previous.record(r)) {
                (Ok(Some(old)), Ok(Some(new))) => {
                    if old.owner != new.owner
                        || self.previous.changed_attrs(&old, &new).iter().any(|h| {
                            crate::core_rules::recompute_row(new.noun_hash, *h)
                                != crate::core_rules::Trigger::DoesNotTrigger
                        })
                    {
                        return true;
                    }
                }
                _ => return true,
            }
        }
        let mut at = self.owner(r);
        let mut seen = HashSet::new();
        while let Some(owner) = at {
            if !seen.insert(owner) {
                return true;
            }
            match (base.record(owner), self.previous.record(owner)) {
                (Ok(Some(old)), Ok(Some(new))) => {
                    if old.owner != new.owner
                        || self.previous.changed_attrs(&old, &new).iter().any(|h| {
                            *h == e3d_attlib::db1_hash("POS") || *h == e3d_attlib::db1_hash("ORI")
                        })
                    {
                        return true;
                    }
                }
                (Ok(None), Ok(None)) if self.inserted.contains(&owner) => {}
                _ => return true,
            }
            at = self.owner(owner);
        }
        false
    }
}
impl SpatialState for EndState<'_, '_> {
    fn noun(&self, r: RefNo) -> Option<u32> {
        match self.drafts.get(&r) {
            Some(draft) => draft.as_ref().map(|d| d.noun_hash),
            None => self.previous.noun(r),
        }
    }
    fn owner(&self, r: RefNo) -> Option<RefNo> {
        match self.drafts.get(&r) {
            Some(draft) => draft.as_ref().map(|d| d.owner).filter(|r| r.is_valid()),
            None => self.previous.owner(r),
        }
    }
    fn members(&self, r: RefNo) -> Option<Vec<RefNo>> {
        match self.drafts.get(&r) {
            Some(draft) => draft.as_ref().map(|d| d.members().to_vec()),
            None => self.previous.members(r),
        }
    }
    fn attr(&self, r: RefNo, hash: u32) -> Result<Option<DescriptorValue>, UndecodableAttr> {
        let Some(draft) = self.drafts.get(&r) else {
            return self.previous.attr(r, hash);
        };
        if let Some(value) = self.values.borrow().get(&(r, hash)) {
            return value.clone();
        }
        let value = match draft {
            Some(d) => self.previous.value(d, hash),
            None => Ok(None),
        };
        self.values.borrow_mut().insert((r, hash), value.clone());
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_and_committed_hunks_share_the_descriptor_decoder() {
        let source = r"D:\e3dio-fixtures\phase3\ams8000_0001";
        if !Path::new(source).exists() {
            // The pinned fixture is an optional corpus artifact; a missing
            // copy must skip this decoder check rather than panic and turn an
            // otherwise healthy build into a false regression.
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        std::fs::copy(source, &path).expect("required pinned fixture");
        let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/e3d31/binaries/desvir.dat");
        let mut provider = TemplateProvider::open(schema, Some(0x000B_0692)).unwrap();
        let mut engine = ReadOnlyEngine::open(&path).unwrap();
        let sesno = engine.session().sesno;
        let r = engine
            .scan_elements(crate::engine::ScanTier::Header)
            .unwrap()
            .map(Result::unwrap)
            .find(|e| e.noun_hash == e3d_attlib::db1_hash("EQUI"))
            .unwrap()
            .refno;
        let desc = e3d_attlib::db1_hash("DESC");
        let text = engine.text_format();
        {
            let shared = Rc::new(RefCell::new(&mut provider));
            let snapshot = Snapshot::open(&path, sesno, shared).unwrap();
            let mut draft = snapshot.record(r).unwrap().unwrap();
            let original = snapshot.attr(r, desc).unwrap();
            assert_eq!(snapshot.attr(r, desc).unwrap(), original);
            draft.set_text_hunk(desc, "a pending hunk", text).unwrap();
            let end = EndState::new(&snapshot, None, &[ElementWrite::Update(draft)]);
            assert_eq!(
                end.attr(r, desc).unwrap(),
                Some(DescriptorValue::Text("a pending hunk".into()))
            );
            assert_eq!(end.attr(r, desc).unwrap(), Some(DescriptorValue::Text("a pending hunk".into())));
            assert_eq!(snapshot.attr(r, desc).unwrap(), original, "draft cache must not replace the committed snapshot value");
        }
        let mut tx = super::super::super::Transaction::open(&path, &mut provider)
            .unwrap()
            .with_spatial(crate::spatial::SpatialPolicy::Off);
        tx.set_hunk(r, "DESC", "a committed hunk").unwrap();
        tx.commit(&super::super::super::SessionRequest {
            user: "e3dio",
            comment: "hunk",
            clock: None,
        })
        .unwrap();
        let latest = ReadOnlyEngine::open(&path).unwrap().session().sesno;
        let snapshot = Snapshot::open(&path, latest, Rc::new(RefCell::new(&mut provider))).unwrap();
        assert_eq!(
            snapshot.attr(r, desc).unwrap(),
            Some(DescriptorValue::Text("a committed hunk".into()))
        );
    }
}
