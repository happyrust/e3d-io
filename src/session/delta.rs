//! What, inside a changed element, changed.
//!
//! [`crate::session::diff`] says *that* a record was rewritten and hands over
//! both versions. This module says *what* differs between them, in the four
//! places a record keeps its state, each compared on its own terms:
//!
//! * **owner** — the two header words. Reported on its own because it is the
//!   one change a tree-shaped consumer has to act on before anything else.
//! * **members** — the member list, as a set and as a sequence. Not an
//!   attribute row anywhere: `STRU 24384/26202` of `ams8000_0001` gained a
//!   member between sessions 250 and 264 and `WORL 13244/0` of `ams5052_0001`
//!   lost two between 163 and 189, and both compare as *zero* attribute rows
//!   changed. Reading the delta off descriptor rows alone misses every
//!   structural edit.
//! * **explicit attributes** — the tail stream, keyed by attribute hash. No
//!   template needed; this is what NAME, DESC and every UDA value live in.
//! * **implicit words** — the fixed region behind the eleven-word header,
//!   word by word. Also template-free, and the only view of an attribute whose
//!   noun has no template (a DICT database read with `desvir.dat`, say). The
//!   header's own words are left out: five of them are the addresses of the
//!   record's tail blocks, which move whenever the record is rewritten and
//!   would report every relocation as a change.
//!
//! On top of those, when a template is available, **attribute rows**: the
//! descriptor-decoded view of both sides through
//! [`crate::engine::extract_parsed_element_with_descriptors`], row by row. This
//! is the reading E3D's `q att` gives, and the one that says `FTUB 24384/23262`
//! moved from z=3400 to z=2900 rather than that words 14 and 27 changed. It
//! is optional because it needs the right `*vir.dat` for the database type,
//! and a missing template is a stated reason, not a failure of the delta.

use e3d_attlib::AttlibData;

use crate::engine::{extract_parsed_element_with_descriptors, ResolveError};
use crate::record::descriptor::{
    AttributeExtractionStatus, DescriptorValue, ElementExtraction, ExtractedAttribute,
};
use crate::record::element::ParsedElement;
use crate::record::explicit::ExplicitValue;
use crate::record::template::TemplateProvider;
use crate::refno::RefNo;
use crate::session::diff::{ChangeKind, ChangedElement};

/// Words of the record header, none of which is an attribute. See
/// `crate::record::HEADER_WORDS`.
const HEADER_WORDS: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerChange {
    pub before: RefNo,
    pub after: RefNo,
}

/// One explicit attribute whose value differs, or which is present on one
/// side only.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplicitChange {
    pub hash: u32,
    /// The attribute's name, when the hash dehashes.
    pub name: Option<String>,
    pub before: Option<ExplicitValue>,
    pub after: Option<ExplicitValue>,
}

/// One implicit word that differs. `None` on a side means that side's implicit
/// region ends before this word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImplicitWordChange {
    pub word: usize,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

/// One descriptor row whose value or status differs between the two sides.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeRowChange {
    pub name: String,
    pub before: Option<DescriptorValue>,
    pub after: Option<DescriptorValue>,
    pub before_status: Option<AttributeExtractionStatus>,
    pub after_status: Option<AttributeExtractionStatus>,
}

/// Everything that differs between the two versions of one element.
///
/// For an insertion `before` is absent and every member, explicit attribute
/// and word of the new record is reported as a change from nothing; a
/// deletion is the mirror image. For a modification the lists hold only what
/// differs, and all of them empty together with `kind` saying
/// `relocated_only` is a record that was copied unchanged.
#[derive(Debug, Clone)]
pub struct ElementDelta {
    pub refno: RefNo,
    pub kind: ChangeKind,
    pub noun_hash: u32,
    pub noun_name: Option<String>,
    pub owner: Option<OwnerChange>,
    /// Members in `after` and not in `before`, in `after`'s order.
    pub members_added: Vec<RefNo>,
    /// Members in `before` and not in `after`, in `before`'s order.
    pub members_removed: Vec<RefNo>,
    /// Same members, different order. Order is meaning in E3D — it is the
    /// position of a member under its owner — so it is a change.
    pub members_reordered: bool,
    pub explicit_changed: Vec<ExplicitChange>,
    pub implicit_words_changed: Vec<ImplicitWordChange>,
    /// Filled by [`ElementDelta::with_attribute_rows`]; `None` until then, or
    /// when no template could decode this noun — see
    /// [`ElementDelta::attribute_rows_unavailable`].
    pub attribute_rows: Option<Vec<AttributeRowChange>>,
    /// Why `attribute_rows` is `None` after [`ElementDelta::with_attribute_rows`]
    /// was asked for them.
    pub attribute_rows_unavailable: Option<String>,
    pub before: Option<ParsedElement>,
    pub after: Option<ParsedElement>,
}

/// A record that could not be parsed. Both sides of a change were read from
/// addresses the index stated, so this is a malformed record, not a wrong
/// address — which `read_element_record` would already have refused.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{refno}: the record at the {side} session does not parse: {detail}")]
pub struct DeltaError {
    pub refno: RefNo,
    /// `"from"` or `"to"`.
    pub side: &'static str,
    pub detail: String,
}

impl ElementDelta {
    /// Compare the two sides of a change. Template-free.
    pub fn of(changed: &ChangedElement) -> Result<Self, DeltaError> {
        let parse = |side: &'static str, view: &crate::record::ElementRecordView| {
            ParsedElement::parse(&view.raw_bytes).map_err(|detail| DeltaError {
                refno: changed.refno,
                side,
                detail,
            })
        };
        let before = changed
            .before
            .as_ref()
            .map(|v| parse("from", v))
            .transpose()?;
        let after = changed.after.as_ref().map(|v| parse("to", v)).transpose()?;

        let present = after
            .as_ref()
            .or(before.as_ref())
            .expect("a change has a record on at least one side");
        let noun_hash = present.noun_hash;

        let owner = match (&before, &after) {
            (Some(b), Some(a)) if b.owner != a.owner => Some(OwnerChange {
                before: b.owner,
                after: a.owner,
            }),
            _ => None,
        };

        let before_members: &[RefNo] = before.as_ref().map_or(&[], |p| &p.members);
        let after_members: &[RefNo] = after.as_ref().map_or(&[], |p| &p.members);
        let members_added: Vec<RefNo> = after_members
            .iter()
            .filter(|m| !before_members.contains(m))
            .copied()
            .collect();
        let members_removed: Vec<RefNo> = before_members
            .iter()
            .filter(|m| !after_members.contains(m))
            .copied()
            .collect();
        let members_reordered = members_added.is_empty()
            && members_removed.is_empty()
            && before_members != after_members;

        let explicit_changed = explicit_changes(before.as_ref(), after.as_ref());
        let implicit_words_changed = implicit_word_changes(before.as_ref(), after.as_ref());

        Ok(Self {
            refno: changed.refno,
            kind: changed.kind,
            noun_hash,
            noun_name: e3d_attlib::db1_dehash(noun_hash),
            owner,
            members_added,
            members_removed,
            members_reordered,
            explicit_changed,
            implicit_words_changed,
            attribute_rows: None,
            attribute_rows_unavailable: None,
            before,
            after,
        })
    }

    /// Add the descriptor-decoded comparison.
    ///
    /// `provider` must be the template file for this database's type
    /// (`DbDescriptor::db_type_name` → `db_element::template_file_for`). A
    /// template that is missing, empty or not authoritative for this noun
    /// leaves `attribute_rows` at `None` and states why in
    /// `attribute_rows_unavailable`; the template-free fields stand regardless.
    pub fn with_attribute_rows(
        &mut self,
        attlib: &AttlibData,
        provider: &mut TemplateProvider,
    ) -> &mut Self {
        let template = match provider.template_for(self.noun_hash) {
            Ok(Some(template)) => template,
            Ok(None) => {
                self.attribute_rows_unavailable =
                    Some(ResolveError::TemplateMissing(self.noun_hash).to_string());
                return self;
            }
            Err(error) => {
                self.attribute_rows_unavailable = Some(error.to_string());
                return self;
            }
        };

        let extract =
            |parsed: &Option<ParsedElement>| -> Result<Option<ElementExtraction>, ResolveError> {
                parsed
                    .as_ref()
                    .map(|p| {
                        extract_parsed_element_with_descriptors(self.refno, p, attlib, template)
                    })
                    .transpose()
            };
        let (before, after) = match (extract(&self.before), extract(&self.after)) {
            (Ok(before), Ok(after)) => (before, after),
            (Err(error), _) | (_, Err(error)) => {
                self.attribute_rows_unavailable = Some(error.to_string());
                return self;
            }
        };

        self.attribute_rows = Some(attribute_row_changes(before.as_ref(), after.as_ref()));
        self.attribute_rows_unavailable = None;
        self
    }

    /// True when nothing this delta can see differs — which for a Modified
    /// key is exactly `relocated_only`, and for anything else is impossible.
    pub fn is_empty(&self) -> bool {
        self.owner.is_none()
            && self.members_added.is_empty()
            && self.members_removed.is_empty()
            && !self.members_reordered
            && self.explicit_changed.is_empty()
            && self.implicit_words_changed.is_empty()
            && self.attribute_rows.as_ref().is_none_or(Vec::is_empty)
    }
}

/// Explicit attributes keyed by hash, the last occurrence winning — the same
/// choice `extract_parsed_element_with_descriptors` makes when it reads an
/// explicit value behind a NonImplicit descriptor.
fn explicit_by_hash(parsed: Option<&ParsedElement>) -> Vec<(u32, &ExplicitValue)> {
    let mut out: Vec<(u32, &ExplicitValue)> = Vec::new();
    if let Some(parsed) = parsed {
        for attribute in &parsed.explicit_attributes {
            match out.iter_mut().find(|(hash, _)| *hash == attribute.hash) {
                Some(slot) => slot.1 = &attribute.value,
                None => out.push((attribute.hash, &attribute.value)),
            }
        }
    }
    out
}

fn explicit_changes(
    before: Option<&ParsedElement>,
    after: Option<&ParsedElement>,
) -> Vec<ExplicitChange> {
    let before = explicit_by_hash(before);
    let after = explicit_by_hash(after);
    let mut out = Vec::new();

    for (hash, value) in &before {
        let newer = after.iter().find(|(h, _)| h == hash).map(|(_, v)| *v);
        if newer != Some(*value) {
            out.push(ExplicitChange {
                hash: *hash,
                name: attribute_name(*hash),
                before: Some((*value).clone()),
                after: newer.cloned(),
            });
        }
    }
    for (hash, value) in &after {
        if !before.iter().any(|(h, _)| h == hash) {
            out.push(ExplicitChange {
                hash: *hash,
                name: attribute_name(*hash),
                before: None,
                after: Some((*value).clone()),
            });
        }
    }
    out
}

fn attribute_name(hash: u32) -> Option<String> {
    e3d_attlib::lookup_system_name(hash)
        .map(String::from)
        .or_else(|| e3d_attlib::db1_dehash(hash))
}

fn implicit_words(parsed: Option<&ParsedElement>) -> Vec<u32> {
    parsed
        .map(|p| {
            let (words, _) = p.implicit_data.as_chunks::<4>();
            words.iter().map(|w| u32::from_be_bytes(*w)).collect()
        })
        .unwrap_or_default()
}

fn implicit_word_changes(
    before: Option<&ParsedElement>,
    after: Option<&ParsedElement>,
) -> Vec<ImplicitWordChange> {
    let before = implicit_words(before);
    let after = implicit_words(after);
    let longest = before.len().max(after.len());
    (HEADER_WORDS..longest)
        .filter_map(|word| {
            let b = before.get(word).copied();
            let a = after.get(word).copied();
            (b != a).then_some(ImplicitWordChange {
                word,
                before: b,
                after: a,
            })
        })
        .collect()
}

fn rows_of(extraction: Option<&ElementExtraction>) -> &[ExtractedAttribute] {
    extraction.map_or(&[], |e| &e.attributes)
}

/// The `n`th row called `name`, in descriptor order.
///
/// Rows come out in descriptor order on both sides, and a name can repeat —
/// the TTY reconciliation keeps duplicates for that reason — so rows are
/// matched by position within their name rather than by name alone.
fn nth_named<'a>(
    rows: &'a [ExtractedAttribute],
    name: &str,
    n: usize,
) -> Option<&'a ExtractedAttribute> {
    rows.iter().filter(|r| r.name == name).nth(n)
}

/// Counts how many times each name has been seen, handing back the ordinal.
#[derive(Default)]
struct Ordinals(Vec<(String, usize)>);

impl Ordinals {
    fn next(&mut self, name: &str) -> usize {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => {
                slot.1 += 1;
                slot.1
            }
            None => {
                self.0.push((name.to_owned(), 0));
                0
            }
        }
    }
}

fn attribute_row_changes(
    before: Option<&ElementExtraction>,
    after: Option<&ElementExtraction>,
) -> Vec<AttributeRowChange> {
    let before = rows_of(before);
    let after = rows_of(after);
    let mut out = Vec::new();

    let mut seen = Ordinals::default();
    for row in after {
        let older = nth_named(before, &row.name, seen.next(&row.name));
        let same = older.is_some_and(|o| o.value == row.value && o.status == row.status);
        if !same {
            out.push(AttributeRowChange {
                name: row.name.clone(),
                before: older.and_then(|o| o.value.clone()),
                after: row.value.clone(),
                before_status: older.map(|o| o.status.clone()),
                after_status: Some(row.status.clone()),
            });
        }
    }
    let mut seen = Ordinals::default();
    for row in before {
        if nth_named(after, &row.name, seen.next(&row.name)).is_none() {
            out.push(AttributeRowChange {
                name: row.name.clone(),
                before: row.value.clone(),
                after: None,
                before_status: Some(row.status.clone()),
                after_status: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ElementRecordView;
    use crate::refno::PageAddress;

    /// A record: 11-word header, `implicit` words behind it, then one member
    /// block and one explicit block holding a single integer attribute, in the
    /// block layout `crate::record::block` documents:
    /// `[kind:u16][words:u16][refno.w0][refno.w1][cont_pgno][cont_packed] payload…`.
    fn record(
        refno: RefNo,
        owner: RefNo,
        implicit: &[u32],
        members: &[RefNo],
        attr: Option<(u32, i32)>,
    ) -> Vec<u8> {
        let mut words: Vec<u32> = vec![0; HEADER_WORDS];
        words.extend_from_slice(implicit);
        words[0] = words.len() as u32;
        words[1] = refno.word0;
        words[2] = refno.word1;
        words[3] = 0x000B_0692;
        words[4] = owner.word0;
        words[5] = owner.word1;

        let block_header = |kind: u32, payload_words: usize, words: &mut Vec<u32>| {
            words.push((kind << 16) | (crate::record::block::MIN_WORDS + payload_words) as u32);
            words.extend_from_slice(&[refno.word0, refno.word1, 0, 0]);
        };
        if !members.is_empty() {
            let body: Vec<u32> = members.iter().flat_map(|m| [m.word0, m.word1]).collect();
            block_header(2, body.len(), &mut words);
            words.extend_from_slice(&body);
        }
        if let Some((hash, value)) = attr {
            block_header(1, 3, &mut words);
            words.push(hash);
            words.push((0x0C00 << 16) | 1); // integer, one data word
            words.push(value as u32);
        }
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn view(refno: RefNo, bytes: Vec<u8>) -> ElementRecordView {
        ElementRecordView {
            refno,
            address: PageAddress {
                page_number: 9,
                slot_offset: 0,
                slot_index: 0,
            },
            raw_bytes: bytes,
            page_no: 9,
            byte_offset: 0,
        }
    }

    #[test]
    fn a_relocated_record_has_an_empty_delta() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let bytes = record(refno, owner, &[5, 6, 7], &[RefNo::new(1, 11)], None);
        let changed = ChangedElement {
            refno,
            kind: ChangeKind::Modified {
                relocated_only: true,
            },
            before: Some(view(refno, bytes.clone())),
            after: Some(view(refno, bytes)),
        };
        let delta = ElementDelta::of(&changed).unwrap();
        assert!(delta.is_empty(), "{delta:?}");
    }

    #[test]
    fn members_owner_and_words_are_each_reported_where_they_change() {
        let refno = RefNo::new(1, 10);
        let before = record(
            refno,
            RefNo::new(1, 1),
            &[5, 6, 7],
            &[RefNo::new(1, 11), RefNo::new(1, 12)],
            None,
        );
        let after = record(
            refno,
            RefNo::new(1, 2),
            &[5, 60, 7],
            &[RefNo::new(1, 12), RefNo::new(1, 13)],
            None,
        );
        let changed = ChangedElement {
            refno,
            kind: ChangeKind::Modified {
                relocated_only: false,
            },
            before: Some(view(refno, before)),
            after: Some(view(refno, after)),
        };
        let delta = ElementDelta::of(&changed).unwrap();
        assert_eq!(
            delta.owner,
            Some(OwnerChange {
                before: RefNo::new(1, 1),
                after: RefNo::new(1, 2)
            })
        );
        assert_eq!(delta.members_added, vec![RefNo::new(1, 13)]);
        assert_eq!(delta.members_removed, vec![RefNo::new(1, 11)]);
        assert!(!delta.members_reordered);
        assert_eq!(
            delta.implicit_words_changed,
            vec![ImplicitWordChange {
                word: HEADER_WORDS + 1,
                before: Some(6),
                after: Some(60),
            }],
            "the header words are not attributes and must not be listed"
        );
        assert!(delta.explicit_changed.is_empty());
    }

    #[test]
    fn a_reordered_member_list_is_a_change_with_nothing_added_or_removed() {
        let refno = RefNo::new(1, 10);
        let owner = RefNo::new(1, 1);
        let a = RefNo::new(1, 11);
        let b = RefNo::new(1, 12);
        let changed = ChangedElement {
            refno,
            kind: ChangeKind::Modified {
                relocated_only: false,
            },
            before: Some(view(refno, record(refno, owner, &[], &[a, b], None))),
            after: Some(view(refno, record(refno, owner, &[], &[b, a], None))),
        };
        let delta = ElementDelta::of(&changed).unwrap();
        assert!(delta.members_added.is_empty() && delta.members_removed.is_empty());
        assert!(delta.members_reordered);
        assert!(!delta.is_empty());
    }

    #[test]
    fn an_insertion_reports_everything_as_new() {
        let refno = RefNo::new(1, 10);
        let bytes = record(
            refno,
            RefNo::new(1, 1),
            &[5],
            &[RefNo::new(1, 11)],
            Some((0xABCD, 42)),
        );
        let changed = ChangedElement {
            refno,
            kind: ChangeKind::Inserted,
            before: None,
            after: Some(view(refno, bytes)),
        };
        let delta = ElementDelta::of(&changed).unwrap();
        assert!(delta.before.is_none() && delta.after.is_some());
        assert_eq!(delta.owner, None, "no earlier owner to differ from");
        assert_eq!(delta.members_added, vec![RefNo::new(1, 11)]);
        assert_eq!(delta.explicit_changed.len(), 1);
        assert_eq!(delta.explicit_changed[0].hash, 0xABCD);
        assert_eq!(delta.explicit_changed[0].before, None);
        assert_eq!(
            delta.explicit_changed[0].after,
            Some(ExplicitValue::Integer(42))
        );
        assert_eq!(
            delta.implicit_words_changed,
            vec![ImplicitWordChange {
                word: HEADER_WORDS,
                before: None,
                after: Some(5),
            }]
        );
    }
}
