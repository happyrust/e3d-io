//! Which keys an element record puts in the keyed tables — `TYPEX` and the
//! reverse-reference tables — and the edits a session's element writes imply
//! for them.
//!
//! `core.dll` keeps these tables as a side effect of setting attributes
//! (`sub_5AB91B0`, the type-14/15/16/18 table-attribute writer) and of
//! deleting an element (`sub_5AB3620`, which walks the noun template's
//! attribute list and removes every key the element's values made). Read
//! together with the corpus, the rules are (`docs/plans/2026-09-20-dabacon-write-plan.md`
//! §7, 2026-09-20 反向表):
//!
//! * **Type 16** (a single reference: `SPRE`, `CATR`, `ISPE`, `LSTU`, `HSTU`,
//!   `SPCREF`, …): a non-null value `t` on element `e` is the key
//!   `[t.0, t.1, e.0, e.1]` in the table named by the attribute's hash. Only
//!   storage type 16 does this — type 4 and 8 references (`CREF`, `HREF`,
//!   `TREF`, `PSPE`, `RREF`…) have no table. On `ams8000_0001` every one of
//!   the 975 set type-16 values sampled has its edge and every edge sampled
//!   has its value; the six type-4 attributes with 1 397 set values have no
//!   table at all.
//! * **Type 17** (a reference array: `HCREF`, `TCREF`, `MDSYSF`, `CONNRE`,
//!   `DESLNK`, `COCORE`, …): every non-null entry `t` of the array on element
//!   `e` is its own key `[t.0, t.1, e.0, e.1]` in the table named by the
//!   attribute's hash — `sub_5AB3620` withdraws types 16 and 17 alike, "every
//!   non-zero pair, one `3CTE` each". Over the 609 DESI / CATA databases
//!   installed here every one of the 3 191 non-null entries set has its edge
//!   and every edge maps back to an entry (`docs/plans/2026-09-21-write-path-next-steps.md`
//!   §7 S4b); the three databases holding `CONNRE` values but no `CONNRE`
//!   table hold only null entries, so no key was ever filed and no table ever
//!   built. The array is `[count][pair × count]` at the descriptor's wide
//!   word offset when implicit-resident (`DESLNK` on `CYMLNK`), else an
//!   explicit field of type code `17 << 10` whose data may keep more pairs
//!   than the count says.
//! * **`TYPEX`**: a noun whose template lists `TYPEX` (storage type 14, one
//!   element of one word) carries `[1, noun]` in its explicit stream and the
//!   key `[noun, e.0, e.1, 0]` in the `TYPEX` table; a noun whose template
//!   does not (`BOX`, `CYLI`, `EXTR`, `LOOP`, `VERT`, `TEXT`, `WORL`, …) has
//!   neither. 5 941 of the 6 639 elements of `ams8000_0001` carry it; the 698
//!   without are exactly those nouns.
//!
//! Knowing an attribute's storage type and, for an implicit-resident one, its
//! word offset takes the noun's template, so everything here is given a
//! [`NounTemplate`]; a noun the schema does not know contributes nothing.

use std::collections::{BTreeMap, BTreeSet};

use e3d_attlib::db1_hash;

use crate::record::template::{NounTemplate, TemplateAttr};
use crate::refno::RefNo;

use super::keyed_cow::{KeyedEdit, KeyedEditKind, KeyedKey};
use super::record::{RecordDraft, KEYED_TYPE_CODE, REFERENCE_ARRAY_TYPE_CODE};

/// Storage type of a single-reference attribute that files a reverse table.
pub const REFERENCE_STORAGE_TYPE: u32 = 16;
/// Storage type of a reference-array attribute that files a reverse table,
/// one edge per non-null entry.
pub const REFERENCE_ARRAY_STORAGE_TYPE: u32 = 17;
/// Storage type of a keyed table attribute (`TYPEX`).
pub const KEYED_STORAGE_TYPE: u32 = 14;

/// The words `core.dll` writes where a value is unset; a key is never made
/// from one.
const UNSET_WORDS: [u32; 2] = [0x8000_0000, 0x8000_0001];

/// The name hash of the `TYPEX` table — `db1_hash("TYPEX")`, 13 396 799, the
/// constant `DB_Element::checkForUnknownAtts` compares against.
pub fn typex_table_hash() -> u32 {
    db1_hash("TYPEX")
}

/// One key an element contributes: which table, and the key itself.
pub type TableKey = (u32, KeyedKey);

/// Every keyed-table key `draft` contributes under `template`.
///
/// `TYPEX` comes from the record itself — the explicit `TYPEX` field, one
/// key per element it lists (there is always one). The reference keys come
/// from the template's type-16 and type-17 attributes: an implicit-resident
/// one is read at its descriptor's word offset in the record's layout, an
/// explicit one by its hash in the stream; a type-17 array files one key per
/// entry. A null reference (both words zero) and an unset marker make no key.
pub fn table_keys(draft: &RecordDraft, template: &NounTemplate) -> BTreeSet<TableKey> {
    let mut keys = BTreeSet::new();
    let me = draft.refno;

    if let Some((type_code, data)) = draft.explicit_words(typex_table_hash()) {
        if type_code == KEYED_TYPE_CODE {
            let count = data.first().copied().unwrap_or(0) as usize;
            for (k, value) in data.iter().skip(1).take(count).enumerate() {
                if !UNSET_WORDS.contains(value) {
                    keys.insert((typex_table_hash(), [*value, me.word0, me.word1, k as u32]));
                }
            }
        }
    }

    for attr in &template.attrs {
        match attr.storage_type_code {
            REFERENCE_STORAGE_TYPE => {
                if let Some(target) = reference_value(draft, template, attr.hash) {
                    keys.insert((attr.hash, [target.word0, target.word1, me.word0, me.word1]));
                }
            }
            REFERENCE_ARRAY_STORAGE_TYPE => {
                for target in reference_array(draft, template, attr.hash).unwrap_or_default() {
                    keys.insert((attr.hash, [target.word0, target.word1, me.word0, me.word1]));
                }
            }
            _ => {}
        }
    }
    keys
}

fn array_attr(template: &NounTemplate, hash: u32) -> Option<&TemplateAttr> {
    template
        .attrs
        .iter()
        .find(|attr| attr.hash == hash && attr.storage_type_code == REFERENCE_ARRAY_STORAGE_TYPE)
}

/// How many `(word0, word1)` pairs an implicit-resident array slot holds
/// behind its count word.
fn implicit_array_capacity(attr: &TemplateAttr) -> usize {
    (attr.wide.storage_words.saturating_sub(1) / 2) as usize
}

/// The **non-null** entries of the type-17 attribute `hash` on `draft`, in
/// stored order — the targets its reverse table holds an edge for. `None`
/// when the attribute is not set (no explicit field, or an implicit count of
/// zero or an unset marker), and when `template` does not list `hash` as a
/// type-17 attribute. Null pairs and unset markers inside the array are
/// skipped, as `sub_5AB3620` skips them.
///
/// Read in the **wide** layout, like [`reference_value`]. Only `count`
/// pairs are read, however many the field has room for.
pub fn reference_array(draft: &RecordDraft, template: &NounTemplate, hash: u32) -> Option<Vec<RefNo>> {
    let attr = array_attr(template, hash)?;
    let offset = attr.wide.location.word_offset() as usize;
    let words: Vec<u32> = if offset != 0 {
        let implicit = draft.implicit();
        let count = *implicit.get(offset)?;
        if count == 0 || UNSET_WORDS.contains(&count) {
            return None;
        }
        let count = (count as usize).min(implicit_array_capacity(attr));
        implicit.get(offset..=offset + 2 * count)?.to_vec()
    } else {
        let (_, data) = draft.explicit_words(hash)?;
        data
    };
    let count = words.first().copied().unwrap_or(0) as usize;
    let pairs = words.get(1..).unwrap_or(&[]);
    let pairs = &pairs[..pairs.len().min(2 * count)];
    let (pairs, _odd) = pairs.as_chunks::<2>();
    let targets = pairs
        .iter()
        .filter(|&&[word0, word1]| !UNSET_WORDS.contains(&word0) && !(word0 == 0 && word1 == 0))
        .map(|&[word0, word1]| RefNo::new(word0, word1))
        .collect();
    Some(targets)
}

/// Set the type-17 attribute `hash` on `draft` to `targets` — `[count][pair ×
/// count]` in the implicit slot when the descriptor places it there, else an
/// explicit field of type code `17 << 10` — so that the next write files one
/// reverse-table key per non-null entry. An empty `targets` clears an
/// explicit field and zeroes an implicit slot. Returns `false`, changing
/// nothing, when `template` does not list `hash` as a type-17 attribute or
/// an implicit slot has no room for that many entries.
pub fn set_reference_array(draft: &mut RecordDraft, template: &NounTemplate, hash: u32, targets: &[RefNo]) -> bool {
    let Some(attr) = array_attr(template, hash) else {
        return false;
    };
    let offset = attr.wide.location.word_offset() as usize;
    if offset != 0 {
        let capacity = implicit_array_capacity(attr);
        if targets.len() > capacity || draft.implicit().len() < offset + 1 + 2 * capacity {
            return false;
        }
        let mut words = vec![0u32; 1 + 2 * capacity];
        words[0] = targets.len() as u32;
        for (i, target) in targets.iter().enumerate() {
            words[1 + 2 * i] = target.word0;
            words[2 + 2 * i] = target.word1;
        }
        for (i, word) in words.into_iter().enumerate() {
            match draft.implicit_word_mut(offset + i) {
                Some(slot) => *slot = word,
                None => return false,
            }
        }
    } else if targets.is_empty() {
        draft.remove_explicit(hash);
    } else {
        let mut data = Vec::with_capacity(1 + 2 * targets.len());
        data.push(targets.len() as u32);
        for target in targets {
            data.push(target.word0);
            data.push(target.word1);
        }
        draft.set_explicit_words(hash, REFERENCE_ARRAY_TYPE_CODE, data);
    }
    true
}

/// The value of the type-16 attribute `hash` on `draft`, or `None` when it is
/// unset or null. `None` also when `template` does not list `hash` as a
/// type-16 attribute.
///
/// A draft is read in the **wide** layout — the one [`RecordDraft::encode`]
/// stamps on every record it writes (bit 29 of the format word, set on every
/// element of the corpus), so the descriptor's wide column is where its
/// implicit values are.
pub fn reference_value(draft: &RecordDraft, template: &NounTemplate, hash: u32) -> Option<RefNo> {
    let attr = template
        .attrs
        .iter()
        .find(|attr| attr.hash == hash && attr.storage_type_code == REFERENCE_STORAGE_TYPE)?;
    let offset = attr.wide.location.word_offset() as usize;
    let (word0, word1) = if offset != 0 {
        let implicit = draft.implicit();
        (*implicit.get(offset)?, *implicit.get(offset + 1)?)
    } else {
        let (_, data) = draft.explicit_words(hash)?;
        (*data.first()?, *data.get(1)?)
    };
    if UNSET_WORDS.contains(&word0) || (word0 == 0 && word1 == 0) {
        return None;
    }
    Some(RefNo::new(word0, word1))
}

/// Set the type-16 attribute `hash` on `draft` to `value` — in the implicit
/// region when the descriptor places it there, else in the explicit stream —
/// so that the next write files the matching reverse-table key. A null
/// `value` (both words zero) clears an explicit field and zeroes an implicit
/// one. Returns `false`, changing nothing, when `template` does not list
/// `hash` as a type-16 attribute.
pub fn set_reference(draft: &mut RecordDraft, template: &NounTemplate, hash: u32, value: RefNo) -> bool {
    let Some(attr) = template
        .attrs
        .iter()
        .find(|attr| attr.hash == hash && attr.storage_type_code == REFERENCE_STORAGE_TYPE)
    else {
        return false;
    };
    let offset = attr.wide.location.word_offset() as usize;
    if offset != 0 {
        let Some(word0) = draft.implicit_word_mut(offset) else {
            return false;
        };
        *word0 = value.word0;
        let Some(word1) = draft.implicit_word_mut(offset + 1) else {
            return false;
        };
        *word1 = value.word1;
    } else if value.word0 == 0 && value.word1 == 0 {
        draft.remove_explicit(hash);
    } else {
        draft.set_explicit_refno(hash, value);
    }
    true
}

/// Give a new element the `TYPEX` its noun's template lists, the way
/// `core.dll` does at creation: `[1, noun]` **appended** to the explicit
/// stream through the ordinary `4SCEA` path — after the chunk-2 prefix
/// `RecordDraft::from_template` seeds and after whatever the caller set
/// before staging, which is where E3D 3.1 itself puts it (`TES023`: `EQUI
/// [NAME TYPEX]`, `ZONE [DUNIO NAME FUNC TYPEX]`; 3 251 of `ams8000_0001`'s
/// 5 941 `TYPEX` records open with the seeded `ISOH`, not `TYPEX` —
/// `docs/evidence/2026-09-22-core-dll-record-draft-data-flow.md` ①). Returns
/// whether the field was added; a template without `TYPEX` (`BOX`, `WORL`, …)
/// or a record that already carries it changes nothing.
pub fn ensure_typex(draft: &mut RecordDraft, template: &NounTemplate) -> bool {
    let typex = typex_table_hash();
    let listed = template
        .attrs
        .iter()
        .any(|attr| attr.hash == typex && attr.storage_type_code == KEYED_STORAGE_TYPE);
    if !listed || draft.explicit_words(typex).is_some() {
        return false;
    }
    draft.set_explicit_words(typex, KEYED_TYPE_CODE, vec![1, draft.noun_hash]);
    true
}

/// The keyed-table edits a session implies, per table.
#[derive(Debug, Default)]
pub struct TableEdits {
    by_table: BTreeMap<u32, Vec<KeyedEdit>>,
}

impl TableEdits {
    /// Record an element going from the keys `before` made to the keys
    /// `after` makes: a key only in `before` is deleted, one only in `after`
    /// inserted. An insert has an empty `before`, a delete an empty `after`.
    pub fn transition(&mut self, before: &BTreeSet<TableKey>, after: &BTreeSet<TableKey>) {
        for (table, key) in before.difference(after) {
            self.by_table.entry(*table).or_default().push(KeyedEdit {
                key: *key,
                kind: KeyedEditKind::Delete,
            });
        }
        for (table, key) in after.difference(before) {
            self.by_table.entry(*table).or_default().push(KeyedEdit {
                key: *key,
                kind: KeyedEditKind::Insert,
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_table.is_empty()
    }

    /// The tables touched, each with its edits, in name-hash order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &[KeyedEdit])> + '_ {
        self.by_table.iter().map(|(table, edits)| (*table, edits.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::{EncodedLocation, StorageDescriptor, TemplateAttr};
    use crate::write::record::REFERENCE_TYPE_CODE;

    const SPRE: u32 = 643_429;
    const ISPE: u32 = 642_042;
    const ZONE: u32 = 640_493;

    fn attr(hash: u32, storage_type_code: u32, wide_offset: u32) -> TemplateAttr {
        let storage = StorageDescriptor {
            storage_words: 2,
            location: EncodedLocation { raw: wide_offset },
            flags: 0,
        };
        TemplateAttr {
            hash,
            stride: 10,
            aux: storage_type_code,
            storage_type_code,
            logical_count: 1,
            compact: storage,
            wide: storage,
            tail: Vec::new(),
            authoritative: true,
        }
    }

    /// A ZONE-like template: `TYPEX` listed, `ISPE` implicit at wide word 30,
    /// `SPRE` explicit, and a type-4 `PSPE` that must make no key.
    fn template() -> NounTemplate {
        let mut template = NounTemplate::new(ZONE);
        template.attrs = vec![
            attr(typex_table_hash(), KEYED_STORAGE_TYPE, 0),
            attr(ISPE, REFERENCE_STORAGE_TYPE, 30),
            attr(SPRE, REFERENCE_STORAGE_TYPE, 0),
            attr(db1_hash("PSPE"), 4, 28),
        ];
        template
    }

    fn draft() -> RecordDraft {
        let mut implicit = vec![0u32; 34];
        implicit[10] = 0x2000_0000;
        RecordDraft::new(RefNo::new(24_384, 47), ZONE, RefNo::new(24_384, 46), implicit, vec![], vec![]).unwrap()
    }

    #[test]
    fn typex_hash_is_the_constant_core_compares_against() {
        assert_eq!(typex_table_hash(), 13_396_799);
    }

    #[test]
    fn keys_come_from_typex_and_table_reference_values_only() {
        let template = template();
        let mut draft = draft();
        assert!(table_keys(&draft, &template).is_empty(), "nothing set, nothing filed");

        assert!(ensure_typex(&mut draft, &template));
        assert!(!ensure_typex(&mut draft, &template), "a second call changes nothing");
        assert_eq!(draft.explicit_words(typex_table_hash()), Some((KEYED_TYPE_CODE, vec![1, ZONE])));

        assert!(set_reference(&mut draft, &template, ISPE, RefNo::new(13_245, 918_008)));
        assert!(set_reference(&mut draft, &template, SPRE, RefNo::new(13_244, 108_564)));
        assert!(
            !set_reference(&mut draft, &template, db1_hash("PSPE"), RefNo::new(1, 1)),
            "a type-4 reference is not a table attribute"
        );
        assert_eq!(draft.implicit()[30..32], [13_245, 918_008], "ISPE went to its implicit slot");
        assert_eq!(draft.explicit_words(SPRE), Some((REFERENCE_TYPE_CODE, vec![13_244, 108_564])));

        let keys = table_keys(&draft, &template);
        assert_eq!(
            keys,
            BTreeSet::from([
                (typex_table_hash(), [ZONE, 24_384, 47, 0]),
                (ISPE, [13_245, 918_008, 24_384, 47]),
                (SPRE, [13_244, 108_564, 24_384, 47]),
            ])
        );

        // Nulling a reference withdraws its key; unset markers never made one.
        assert!(set_reference(&mut draft, &template, SPRE, RefNo::new(0, 0)));
        assert!(draft.explicit_words(SPRE).is_none());
        *draft.implicit_word_mut(30).unwrap() = 0x8000_0001;
        let keys = table_keys(&draft, &template);
        assert_eq!(keys, BTreeSet::from([(typex_table_hash(), [ZONE, 24_384, 47, 0])]));
    }

    /// A type-17 descriptor: `storage_words` is the slot's size (count word
    /// plus two per pair), `wide_offset` 0 for an explicit-resident one.
    fn array_attr(hash: u32, wide_offset: u32, storage_words: u32) -> TemplateAttr {
        let mut attr = attr(hash, REFERENCE_ARRAY_STORAGE_TYPE, wide_offset);
        attr.compact.storage_words = storage_words;
        attr.wide.storage_words = storage_words;
        attr.logical_count = (storage_words - 1) / 2;
        attr
    }

    #[test]
    fn a_reference_array_files_one_key_per_non_null_entry() {
        use crate::write::record::REFERENCE_ARRAY_TYPE_CODE;
        let hcref = db1_hash("HCREF");
        let deslnk = db1_hash("DESLNK");
        let mut template = NounTemplate::new(db1_hash("CWBRAN"));
        template.attrs = vec![
            attr(typex_table_hash(), KEYED_STORAGE_TYPE, 0),
            array_attr(hcref, 0, 201),  // explicit-resident, room for 100 pairs
            array_attr(deslnk, 30, 7),  // implicit-resident at wide word 30, room for 3
            attr(SPRE, REFERENCE_STORAGE_TYPE, 0),
        ];
        let mut implicit = vec![0u32; 40];
        implicit[10] = 0x2000_0000;
        let me = RefNo::new(2_013_287_686, 36);
        let mut draft = RecordDraft::new(me, db1_hash("CWBRAN"), RefNo::new(2_013_287_686, 3), implicit, vec![], vec![]).unwrap();
        assert_eq!(reference_array(&draft, &template, hcref), None, "no field, not set");
        assert_eq!(reference_array(&draft, &template, deslnk), None, "count 0, not set");
        assert!(table_keys(&draft, &template).is_empty());

        // Explicit: `[count][pairs]` under `17 << 10`, one key per entry.
        let (t1, t2) = (RefNo::new(2_013_287_686, 75), RefNo::new(2_013_287_686, 87));
        assert!(set_reference_array(&mut draft, &template, hcref, &[t1, t2]));
        assert_eq!(
            draft.explicit_words(hcref),
            Some((REFERENCE_ARRAY_TYPE_CODE, vec![2, t1.word0, t1.word1, t2.word0, t2.word1]))
        );
        assert_eq!(reference_array(&draft, &template, hcref), Some(vec![t1, t2]));

        // Implicit: the count word then the pairs, the rest of the slot zeroed.
        let t3 = RefNo::new(2_013_286_765, 5);
        assert!(set_reference_array(&mut draft, &template, deslnk, &[t3]));
        assert_eq!(draft.implicit()[30..37], [1, t3.word0, t3.word1, 0, 0, 0, 0]);
        assert_eq!(reference_array(&draft, &template, deslnk), Some(vec![t3]));
        assert!(
            !set_reference_array(&mut draft, &template, deslnk, &[t3, t3, t3, t3]),
            "four pairs do not fit a slot sized for three"
        );
        assert_eq!(draft.implicit()[30..37], [1, t3.word0, t3.word1, 0, 0, 0, 0], "a refused set changes nothing");
        assert!(!set_reference_array(&mut draft, &template, SPRE, &[t1]), "a type-16 attribute is not an array");

        let keys = table_keys(&draft, &template);
        assert_eq!(
            keys,
            BTreeSet::from([
                (hcref, [t1.word0, t1.word1, me.word0, me.word1]),
                (hcref, [t2.word0, t2.word1, me.word0, me.word1]),
                (deslnk, [t3.word0, t3.word1, me.word0, me.word1]),
            ])
        );

        // Null and unset entries inside the array make no key, and a field
        // that keeps more pairs than its count says (four of the corpus's
        // 2 754) is read by the count.
        draft.set_explicit_words(
            hcref,
            REFERENCE_ARRAY_TYPE_CODE,
            vec![3, t1.word0, t1.word1, 0, 0, 0x8000_0001, 7, t2.word0, t2.word1, 9, 9],
        );
        assert_eq!(reference_array(&draft, &template, hcref), Some(vec![t1]), "count 3: t1, a null, an unset marker");
        let keys = table_keys(&draft, &template);
        assert!(keys.contains(&(hcref, [t1.word0, t1.word1, me.word0, me.word1])));
        assert!(!keys.contains(&(hcref, [t2.word0, t2.word1, me.word0, me.word1])), "past the count");

        // Clearing: an empty array removes the explicit field and zeroes the
        // implicit slot; neither files a key.
        assert!(set_reference_array(&mut draft, &template, hcref, &[]));
        assert!(draft.explicit_words(hcref).is_none());
        assert!(set_reference_array(&mut draft, &template, deslnk, &[]));
        assert_eq!(draft.implicit()[30..37], [0; 7]);
        assert!(table_keys(&draft, &template).is_empty());
    }

    #[test]
    fn a_template_without_typex_files_no_typex_key() {
        let mut template = NounTemplate::new(db1_hash("BOX"));
        template.attrs = vec![attr(db1_hash("TMRREF"), REFERENCE_STORAGE_TYPE, 0)];
        let mut draft = RecordDraft::new(
            RefNo::new(24_384, 31_483),
            db1_hash("BOX"),
            RefNo::new(24_384, 47),
            vec![0x2000_0000; 11],
            vec![],
            vec![],
        )
        .unwrap();
        assert!(!ensure_typex(&mut draft, &template));
        assert!(draft.explicit_words(typex_table_hash()).is_none());
        assert!(table_keys(&draft, &template).is_empty());
    }

    #[test]
    fn a_transition_deletes_what_went_and_inserts_what_came() {
        let before = BTreeSet::from([(SPRE, [1, 1, 9, 9]), (ISPE, [2, 2, 9, 9])]);
        let after = BTreeSet::from([(SPRE, [1, 1, 9, 9]), (ISPE, [3, 3, 9, 9])]);
        let mut edits = TableEdits::default();
        edits.transition(&before, &after);
        let tables: Vec<(u32, Vec<KeyedEdit>)> = edits.iter().map(|(t, e)| (t, e.to_vec())).collect();
        assert_eq!(
            tables,
            vec![(
                ISPE,
                vec![
                    KeyedEdit { key: [2, 2, 9, 9], kind: KeyedEditKind::Delete },
                    KeyedEdit { key: [3, 3, 9, 9], kind: KeyedEditKind::Insert },
                ]
            )],
            "SPRE did not change, so it is not touched"
        );
    }
}
