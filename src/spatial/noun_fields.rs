//! The noun dictionary of `attlib.dat`: the fields core.dll's `DB_Noun` loads
//! for each noun and `ATNINT` / `ATNLOG` read — among them `IMAP`, by which
//! Core3D's `mapboxlib` decides whether an element gets SPAMAP entries
//! (`docs/evidence/2026-10-03-spamap-coverage.md`).
//!
//! The layout is the attribute dictionary's (`e3d_attlib::dict`) on two other
//! directory slots: slot 4 lists the fields in order, a field's 1-based place
//! in that list being its ordinal; slot 6 indexes each noun's record as
//! `page × 512 + word`. A record starts at file word `(page − 1) × 512 + word − 2`
//! with one header word per ordinal: `−1` takes the field's default, `0` the
//! value of the noun its `SYNO` field names, anything else is the offset of
//! the value from the record's start.

use std::collections::HashMap;
use std::path::Path;

/// The field naming the noun a record inherits a `0` header word from.
pub const SYNO: u32 = 837_586;

const WORDS_PER_PAGE: usize = 512;
const FIELD_LIST_SLOT: usize = 4;
const NOUN_INDEX_SLOT: usize = 6;
const END: u32 = 0xFFFF_FFFF;
/// The base-27 name hashes: `[531442, 387951929]`.
const HASH_RANGE: std::ops::RangeInclusive<u32> = 531_442..=387_951_929;
/// Followed `SYNO` links per lookup, against a cycle.
const INHERITANCE_DEPTH: usize = 32;

/// A field value as the dictionary resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldValue {
    /// Stored in the noun's record, or in that of a noun it inherits from.
    Stored(u32),
    /// The field's default, for a header word of `−1`.
    Default(u32),
}

impl FieldValue {
    pub fn word(self) -> u32 {
        match self {
            FieldValue::Stored(word) | FieldValue::Default(word) => word,
        }
    }
}

/// The noun dictionary of one `attlib.dat`.
pub struct NounFields {
    words: Vec<u32>,
    ordinal: HashMap<u32, usize>,
    default: HashMap<u32, u32>,
    record: HashMap<u32, usize>,
}

impl NounFields {
    pub fn parse_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|&word| u32::from_be_bytes(word)).collect();
        if words.len() < 2 * WORDS_PER_PAGE {
            return Err(format!("attlib.dat of {} bytes has no directory page", bytes.len()));
        }
        let start_page = |slot: usize| words[WORDS_PER_PAGE + slot] as usize;
        let (ordinal, default) = field_list(&words, start_page(FIELD_LIST_SLOT));
        let record = noun_index(&words, start_page(NOUN_INDEX_SLOT));
        if ordinal.is_empty() || record.is_empty() {
            return Err(format!("attlib.dat lists {} noun fields and {} nouns", ordinal.len(), record.len()));
        }
        Ok(Self { words, ordinal, default, record })
    }

    /// Every noun the dictionary has a record for, in hash order.
    pub fn nouns(&self) -> Vec<u32> {
        let mut nouns: Vec<u32> = self.record.keys().copied().collect();
        nouns.sort_unstable();
        nouns
    }

    /// Every field, in ordinal order.
    pub fn fields(&self) -> Vec<u32> {
        let mut fields: Vec<(usize, u32)> = self.ordinal.iter().map(|(&field, &ordinal)| (ordinal, field)).collect();
        fields.sort_unstable();
        fields.into_iter().map(|(_, field)| field).collect()
    }

    pub fn has_noun(&self, noun: u32) -> bool {
        self.record.contains_key(&noun)
    }

    /// The field of the noun as `ATNINT` reads it, or `None` when the noun
    /// or the field is not in the dictionary, or the value cannot be found.
    pub fn field(&self, noun: u32, field: u32) -> Option<FieldValue> {
        let ordinal = *self.ordinal.get(&field)?;
        let mut noun = noun;
        for _ in 0..INHERITANCE_DEPTH {
            match self.header(noun, ordinal)? {
                END => return self.default.get(&field).map(|&word| FieldValue::Default(word)),
                0 => {
                    let syno = *self.ordinal.get(&SYNO)?;
                    let offset = self.header(noun, syno)?;
                    match offset {
                        0 | END => return None,
                        offset => {
                            let parent = self.word(noun, offset)?;
                            if parent == noun {
                                return None;
                            }
                            noun = parent;
                        }
                    }
                }
                offset => return self.word(noun, offset).map(FieldValue::Stored),
            }
        }
        None
    }

    fn header(&self, noun: u32, ordinal: usize) -> Option<u32> {
        self.word(noun, ordinal as u32)
    }

    fn word(&self, noun: u32, offset: u32) -> Option<u32> {
        let start = *self.record.get(&noun)?;
        self.words.get(start.checked_add(offset as usize)?).copied()
    }
}

/// Slot 4: records `[hash, data type, kind]`, kind 2 followed by a default —
/// one word, or for data type 4 a count and that many words. A page ends at a
/// 0 word, the list at `−1`.
fn field_list(words: &[u32], start_page: usize) -> (HashMap<u32, usize>, HashMap<u32, u32>) {
    let mut ordinal = HashMap::new();
    let mut default = HashMap::new();
    let mut count = 0;
    let Some(mut page) = start_page.checked_sub(1) else { return (ordinal, default) };
    'pages: while (page + 1) * WORDS_PER_PAGE <= words.len() {
        let base = page * WORDS_PER_PAGE;
        let mut at = 0;
        while at + 2 < WORDS_PER_PAGE {
            let hash = words[base + at];
            if hash == END {
                break 'pages;
            }
            let (data_type, kind) = (words[base + at + 1], words[base + at + 2]);
            if !HASH_RANGE.contains(&hash) || !(1..=12).contains(&data_type) || !(kind == 1 || kind == 2) {
                break;
            }
            at += 3;
            if kind == 2 && at < WORDS_PER_PAGE {
                if data_type == 4 {
                    at += 1 + words[base + at] as usize;
                } else {
                    default.entry(hash).or_insert(words[base + at]);
                    at += 1;
                }
            }
            count += 1;
            ordinal.entry(hash).or_insert(count);
        }
        page += 1;
    }
    (ordinal, default)
}

/// Slot 6: pairs `(noun hash, page × 512 + word)`. A page ends at a 0 word, the
/// index at `−1`.
fn noun_index(words: &[u32], start_page: usize) -> HashMap<u32, usize> {
    let mut record = HashMap::new();
    let Some(mut page) = start_page.checked_sub(1) else { return record };
    'pages: while (page + 1) * WORDS_PER_PAGE <= words.len() {
        let base = page * WORDS_PER_PAGE;
        let mut at = 0;
        while at + 1 < WORDS_PER_PAGE {
            let hash = words[base + at];
            if hash == END {
                break 'pages;
            }
            if !HASH_RANGE.contains(&hash) {
                break;
            }
            let packed = words[base + at + 1] as usize;
            let (record_page, word) = (packed / WORDS_PER_PAGE, packed % WORDS_PER_PAGE);
            if let Some(start) = (record_page * WORDS_PER_PAGE + word).checked_sub(WORDS_PER_PAGE + 2) {
                record.insert(hash, start);
            }
            at += 2;
        }
        page += 1;
    }
    record
}
