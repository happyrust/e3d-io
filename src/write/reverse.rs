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

use crate::record::template::NounTemplate;
use crate::refno::RefNo;

use super::keyed_cow::{KeyedEdit, KeyedEditKind, KeyedKey};
use super::record::{RecordDraft, KEYED_TYPE_CODE};

/// Storage type of a single-reference attribute that files a reverse table.
pub const REFERENCE_STORAGE_TYPE: u32 = 16;
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
/// from the template's type-16 attributes: an implicit-resident one is read
/// at its descriptor's word offset in the record's layout, an explicit one by
/// its hash in the stream. A null reference (both words zero) and an unset
/// marker make no key.
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

    for attr in template
        .attrs
        .iter()
        .filter(|attr| attr.storage_type_code == REFERENCE_STORAGE_TYPE)
    {
        let Some(target) = reference_value(draft, template, attr.hash) else {
            continue;
        };
        keys.insert((attr.hash, [target.word0, target.word1, me.word0, me.word1]));
    }
    keys
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
/// `core.dll` does at creation: `[1, noun]` at the front of the explicit
/// stream. Returns whether the field was added; a template without `TYPEX`
/// (`BOX`, `WORL`, …) or a record that already carries it changes nothing.
pub fn ensure_typex(draft: &mut RecordDraft, template: &NounTemplate) -> bool {
    let typex = typex_table_hash();
    let listed = template
        .attrs
        .iter()
        .any(|attr| attr.hash == typex && attr.storage_type_code == KEYED_STORAGE_TYPE);
    if !listed || draft.explicit_words(typex).is_some() {
        return false;
    }
    draft.set_explicit_words_first(typex, KEYED_TYPE_CODE, vec![1, draft.noun_hash]);
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
    fn keys_come_from_typex_and_type_16_values_only() {
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
