//! Setting an attribute on a [`RecordDraft`] by its descriptor — the inverse
//! of [`crate::engine::extract_parsed_element_with_descriptors`].
//!
//! A noun's template says, for every attribute, its storage type, how many
//! elements it may hold and — when the value lives in the implicit region —
//! the word the slot starts at (`encoded_location & 0xFFFFF`, the wide
//! column) and how many words the slot has. [`set_attribute`] takes a
//! [`DescriptorValue`] the read side would produce and lays it down where the
//! descriptor says, in the shape the read side decodes:
//!
//! | storage type | shape | implicit slot | explicit field (`type << 10`) |
//! |---|---|---|---|
//! | 2 real, 6 real array | `f64`, two Fortran-ordered words | value; `[n][2n]` | same |
//! | 3 word, 7 word array, 18 | one word | value; `[n][n]` | same |
//! | 4 ref, 8 ref array, 16, 17 | two words | value; `[n][2n]` | same |
//! | 5 bool | one bit of the slot word, `location >> 20` | read-modify-write | `[0 \| 1]` |
//! | 9 bool array | 32 to a word | `[n][ceil(n/32)]` | same |
//! | 10, 15 text | four bytes to a word, NUL-padded | `[bytes][ceil(bytes/4)]` | same |
//! | 14 record | `stride` words per element | `[n][n·stride]` | verbatim |
//! | 19 string list | [`crate::record::hunk`] | — | verbatim |
//!
//! Only what the value needs is written: an implicit slot's words past the
//! count and elements keep whatever they held, the way `4SCEA` leaves them,
//! and a record read off the disk and written back through this comes out
//! word for word the same. [`DescriptorValue::Unset`] writes the unset marker
//! into an implicit slot and removes an explicit field, which is what makes
//! the read side fall back to the noun's default.
//!
//! What the read side collapses, this takes back the way it came: a one-word
//! `Word { raw }` is its raw word, a `Bool` on a one-word slot is 0 or 1, an
//! angle attribute's `Real` degrees are the signed centidegrees the word
//! stores, and a `RawWords` value is the elements of an implicit slot but the
//! whole data of an explicit field whose type the stream decoder hands over
//! raw (14, 17, 18, 19 …), because that is exactly what it handed over.
//!
//! Records are written in the **wide** layout only, the one every element of
//! the corpus uses; a compact record (format-word bit 29 clear) is refused.
//! Text is stored as the header's `FORMAT` says ([`crate::record::text`]):
//! its UTF-8 bytes under `FORMAT 1`, the project code page in `&~ … &` runs
//! under any other, the count word being the stored byte count either way.

use crate::record::descriptor::{descriptor_is_array, DescriptorValue, StorageShape};
use crate::record::hunk;
use crate::record::template::{NounTemplate, TemplateAttr, WIDE_LAYOUT_FLAG};
use crate::record::text::{self, TextEncodeError, TextFormat};
use crate::refno::RefNo;

use super::record::{RecordDraft, RecordEncodeError};

/// The word written where an implicit value is unset. `core.dll` treats both
/// [`crate::record::descriptor::UNSET_SENTINELS`] alike on read;
/// `sub_5AB4CF0` fills what it did not read with this one.
pub const UNSET_WORD: u32 = 0x8000_0001;

/// Why an attribute could not be set. Nothing is changed on the draft when
/// one of these comes back.
#[derive(Debug, thiserror::Error)]
pub enum AttributeWriteError {
    #[error("attribute {hash:#x} is not in the template of noun {noun:#x}")]
    NotInTemplate { noun: u32, hash: u32 },
    #[error("attribute {hash:#x}: the template carries no physical descriptor for it")]
    NotAuthoritative { hash: u32 },
    #[error("attribute {hash:#x}: storage type {storage_type} is not one the schema files use")]
    UnknownStorageType { hash: u32, storage_type: u32 },
    #[error("attribute {hash:#x} (storage type {storage_type}, {shape}) cannot take a {given}")]
    TypeMismatch {
        hash: u32,
        storage_type: u32,
        shape: &'static str,
        given: &'static str,
    },
    #[error("attribute {hash:#x}: {count} elements, the descriptor allows {max}")]
    TooManyElements { hash: u32, count: usize, max: u32 },
    #[error("attribute {hash:#x}: {words} words do not fit its {slot}-word implicit slot")]
    SlotTooSmall { hash: u32, words: usize, slot: u32 },
    #[error("attribute {hash:#x}: its implicit slot at word {offset} runs past the {len}-word implicit region")]
    SlotOutOfRecord { hash: u32, offset: usize, len: usize },
    #[error("attribute {hash:#x}: the record is in the compact layout, which this writer does not produce")]
    CompactLayout { hash: u32 },
    #[error("attribute {hash:#x}: {source}")]
    Text { hash: u32, source: TextEncodeError },
    #[error("attribute {hash:#x}: packed-bit index {bit} exceeds 31")]
    BitIndex { hash: u32, bit: u32 },
    #[error("attribute {hash:#x}: a type-14 record of {words} words is not a whole number of {stride}-word elements")]
    RecordStride { hash: u32, words: usize, stride: u32 },
    #[error("attribute {hash:#x}: a string list is set through the string-list setters, not a text value")]
    StringList { hash: u32 },
    #[error(transparent)]
    Record(#[from] RecordEncodeError),
}

/// Where the encoded value goes.
enum Encoded {
    /// Words to write from the slot's first word on.
    Implicit(Vec<u32>),
    /// One bit of the slot's word, set or cleared.
    Bit(bool),
    /// An explicit field: type code and data words.
    Explicit(u16, Vec<u32>),
}

/// Set attribute `hash` of `draft` to `value` where `template`'s descriptor
/// places it. `text` is the database header's `FORMAT` reading, for text
/// values. See the module documentation for the shapes.
pub fn set_attribute(
    draft: &mut RecordDraft,
    template: &NounTemplate,
    hash: u32,
    value: &DescriptorValue,
    text: TextFormat,
) -> Result<(), AttributeWriteError> {
    let attr = template
        .attrs
        .iter()
        .find(|attr| attr.hash == hash)
        .ok_or(AttributeWriteError::NotInTemplate { noun: template.noun_hash, hash })?;
    if !attr.authoritative {
        return Err(AttributeWriteError::NotAuthoritative { hash });
    }
    if draft.implicit().get(10).is_some_and(|format| format & WIDE_LAYOUT_FLAG == 0) {
        return Err(AttributeWriteError::CompactLayout { hash });
    }
    let implicit = attr.wide.location.word_offset() != 0;

    // An attribute the record keeps out of the record: its explicit field is
    // a hunk directory entry, which the read side hands over as the five raw
    // words it is. Handed back, it is re-emitted as it stands — address and
    // all, the way a rewritten record keeps its hunks where they are
    // (`sub_5AE9960`). A new value for such an attribute goes through the
    // hunk setters on [`RecordDraft`].
    if let (DescriptorValue::RawWords(words), Some((type_code, _))) = (value, draft.explicit_words(hash)) {
        if hunk::HunkRef::from_words(type_code, words).is_some() {
            draft.set_explicit_words(hash, type_code, words.clone());
            return Ok(());
        }
    }

    if matches!(value, DescriptorValue::Unset) {
        if implicit {
            let offset = attr.wide.location.word_offset() as usize;
            if attr.storage_type_code == 5 {
                return write_bit(draft, attr, false);
            }
            check_slot(draft, attr, 1)?;
            *draft.implicit_word_mut(offset).expect("checked") = UNSET_WORD;
        } else {
            draft.remove_explicit(hash);
        }
        return Ok(());
    }

    match encode(attr, value, implicit, text)? {
        Encoded::Implicit(words) => {
            check_slot(draft, attr, words.len())?;
            let offset = attr.wide.location.word_offset() as usize;
            for (index, word) in words.into_iter().enumerate() {
                *draft.implicit_word_mut(offset + index).expect("checked") = word;
            }
        }
        Encoded::Bit(set) => write_bit(draft, attr, set)?,
        Encoded::Explicit(type_code, data) => {
            draft.set_explicit_words_checked(hash, type_code, data)?;
        }
    }
    Ok(())
}

impl RecordDraft {
    /// [`set_attribute`] on this draft.
    pub fn set_attribute(
        &mut self,
        template: &NounTemplate,
        hash: u32,
        value: &DescriptorValue,
        text: TextFormat,
    ) -> Result<(), AttributeWriteError> {
        set_attribute(self, template, hash, value, text)
    }
}

/// The implicit slot has room for `words` words and lies inside the record.
fn check_slot(draft: &RecordDraft, attr: &TemplateAttr, words: usize) -> Result<(), AttributeWriteError> {
    let hash = attr.hash;
    let offset = attr.wide.location.word_offset() as usize;
    let slot = attr.wide.storage_words;
    if words > slot as usize {
        return Err(AttributeWriteError::SlotTooSmall { hash, words, slot });
    }
    let len = draft.implicit().len();
    if offset + words > len {
        return Err(AttributeWriteError::SlotOutOfRecord { hash, offset, len });
    }
    Ok(())
}

fn write_bit(draft: &mut RecordDraft, attr: &TemplateAttr, set: bool) -> Result<(), AttributeWriteError> {
    let hash = attr.hash;
    let bit = attr
        .wide
        .location
        .bool_bit_index()
        .ok_or(AttributeWriteError::BitIndex { hash, bit: attr.wide.location.upper_bits() })?;
    check_slot(draft, attr, 1)?;
    let offset = attr.wide.location.word_offset() as usize;
    let word = draft.implicit_word_mut(offset).expect("checked");
    if set {
        *word |= 1 << bit;
    } else {
        *word &= !(1 << bit);
    }
    Ok(())
}

/// Whether an explicit field of this storage type reaches the read side as
/// raw data words (`crate::record::explicit::decode_value`'s fall-through),
/// so that a [`DescriptorValue::RawWords`] for it is the whole field.
fn explicit_is_verbatim(storage_type: u32) -> bool {
    !matches!(storage_type, 2 | 3 | 4 | 5 | 6 | 7 | 8 | 10 | 15 | 16)
}

fn mismatch(attr: &TemplateAttr, shape: &'static str, value: &DescriptorValue) -> AttributeWriteError {
    AttributeWriteError::TypeMismatch {
        hash: attr.hash,
        storage_type: attr.storage_type_code,
        shape,
        given: value_name(value),
    }
}

fn value_name(value: &DescriptorValue) -> &'static str {
    match value {
        DescriptorValue::Unset => "Unset",
        DescriptorValue::Bool(_) => "Bool",
        DescriptorValue::Int(_) => "Int",
        DescriptorValue::Real(_) => "Real",
        DescriptorValue::Text(_) => "Text",
        DescriptorValue::RefNo(_) => "RefNo",
        DescriptorValue::Word { .. } => "Word",
        DescriptorValue::IntArray(_) => "IntArray",
        DescriptorValue::RealArray(_) => "RealArray",
        DescriptorValue::RefNoArray(_) => "RefNoArray",
        DescriptorValue::WordArray(_) => "WordArray",
        DescriptorValue::BoolArray(_) => "BoolArray",
        DescriptorValue::RawWords(_) => "RawWords",
    }
}

fn check_count(attr: &TemplateAttr, count: usize) -> Result<(), AttributeWriteError> {
    if attr.logical_count != 0 && count > attr.logical_count as usize {
        return Err(AttributeWriteError::TooManyElements {
            hash: attr.hash,
            count,
            max: attr.logical_count,
        });
    }
    Ok(())
}

fn real_words(value: f64) -> [u32; 2] {
    let bits = value.to_bits();
    [bits as u32, (bits >> 32) as u32]
}

fn refno_words(value: RefNo) -> [u32; 2] {
    [value.word0, value.word1]
}

/// Signed centidegrees, the word an angle attribute stores (`SCTN.Bangle`
/// holds -9000 for -90°). Rounded, so a value read as `raw / 100.0` goes
/// back to `raw`.
fn centidegrees(degrees: f64) -> u32 {
    (degrees * 100.0).round() as i32 as u32
}

/// `[stored bytes][packed text]`: the text as the header's `FORMAT` stores
/// it ([`text::encode_as`]), the count the stored length — escapes included
/// under a non-UTF-8 header, which is what the read side's count says too.
fn text_words(attr: &TemplateAttr, text_value: &str, format: TextFormat) -> Result<Vec<u32>, AttributeWriteError> {
    let bytes = text::encode_as(text_value, format).map_err(|source| AttributeWriteError::Text { hash: attr.hash, source })?;
    let mut words = Vec::with_capacity(1 + bytes.len().div_ceil(4));
    words.push(bytes.len() as u32);
    words.extend(text::pack_words(&bytes));
    Ok(words)
}

fn encode(
    attr: &TemplateAttr,
    value: &DescriptorValue,
    implicit: bool,
    text: TextFormat,
) -> Result<Encoded, AttributeWriteError> {
    let hash = attr.hash;
    let storage_type = attr.storage_type_code;
    let type_code = (storage_type as u16) << 10;
    let shape = StorageShape::of(storage_type)
        .ok_or(AttributeWriteError::UnknownStorageType { hash, storage_type })?;
    let is_array = descriptor_is_array(attr);

    // An explicit field the stream decoder hands over raw goes back raw.
    if let (false, true, DescriptorValue::RawWords(words)) = (implicit, explicit_is_verbatim(storage_type), value) {
        return Ok(Encoded::Explicit(type_code, words.clone()));
    }

    let words: Vec<u32> = match shape {
        StorageShape::PackedBit => {
            let DescriptorValue::Bool(set) = value else {
                return Err(mismatch(attr, "a packed bit", value));
            };
            if implicit {
                return Ok(Encoded::Bit(*set));
            }
            vec![u32::from(*set)]
        }
        StorageShape::Real if !is_array => {
            let real = match value {
                DescriptorValue::Real(real) => *real,
                DescriptorValue::Int(int) => f64::from(*int),
                _ => return Err(mismatch(attr, "a real", value)),
            };
            real_words(real).to_vec()
        }
        StorageShape::Real => {
            let DescriptorValue::RealArray(reals) = value else {
                return Err(mismatch(attr, "a real array", value));
            };
            check_count(attr, reals.len())?;
            let mut words = Vec::with_capacity(1 + 2 * reals.len());
            words.push(reals.len() as u32);
            for real in reals {
                words.extend_from_slice(&real_words(*real));
            }
            words
        }
        StorageShape::Word if !is_array => {
            let word = match value {
                DescriptorValue::Int(int) => *int as u32,
                DescriptorValue::Bool(set) => u32::from(*set),
                DescriptorValue::Word { raw, .. } => *raw,
                DescriptorValue::Real(degrees) if e3d_attlib::is_angle_attr(hash) => centidegrees(*degrees),
                DescriptorValue::RawWords(words) if words.len() == 1 => words[0],
                _ => return Err(mismatch(attr, "a word", value)),
            };
            vec![word]
        }
        StorageShape::Word => {
            let elements: Vec<u32> = match value {
                DescriptorValue::IntArray(ints) => ints.iter().map(|int| *int as u32).collect(),
                DescriptorValue::WordArray(words) => words.clone(),
                DescriptorValue::RawWords(words) => words.clone(),
                _ => return Err(mismatch(attr, "a word array", value)),
            };
            check_count(attr, elements.len())?;
            let mut words = Vec::with_capacity(1 + elements.len());
            words.push(elements.len() as u32);
            words.extend(elements);
            words
        }
        StorageShape::Reference if !is_array => {
            let DescriptorValue::RefNo(refno) = value else {
                return Err(mismatch(attr, "a reference", value));
            };
            refno_words(*refno).to_vec()
        }
        StorageShape::Reference => {
            let DescriptorValue::RefNoArray(refnos) = value else {
                return Err(mismatch(attr, "a reference array", value));
            };
            check_count(attr, refnos.len())?;
            let mut words = Vec::with_capacity(1 + 2 * refnos.len());
            words.push(refnos.len() as u32);
            for refno in refnos {
                words.extend_from_slice(&refno_words(*refno));
            }
            words
        }
        StorageShape::BitSet => {
            let DescriptorValue::BoolArray(bits) = value else {
                return Err(mismatch(attr, "a boolean array", value));
            };
            check_count(attr, bits.len())?;
            let mut words = vec![0u32; 1 + bits.len().div_ceil(32)];
            words[0] = bits.len() as u32;
            for (index, set) in bits.iter().enumerate() {
                if *set {
                    words[1 + index / 32] |= 1 << (index % 32);
                }
            }
            words
        }
        StorageShape::Characters => {
            if storage_type == 19 {
                return Err(AttributeWriteError::StringList { hash });
            }
            let DescriptorValue::Text(text_value) = value else {
                return Err(mismatch(attr, "a text", value));
            };
            let words = text_words(attr, text_value, text)?;
            check_count(attr, words[0] as usize)?;
            words
        }
        StorageShape::Record => {
            let elements: Vec<u32> = match value {
                DescriptorValue::IntArray(ints) => ints.iter().map(|int| *int as u32).collect(),
                DescriptorValue::RawWords(words) => words.clone(),
                _ => return Err(mismatch(attr, "a record array", value)),
            };
            // `sub_5AB5600`: the stride comes from the compact column.
            let stride = attr
                .compact
                .storage_words
                .saturating_sub(1)
                .checked_div(attr.logical_count.max(1))
                .filter(|stride| *stride > 0)
                .ok_or(AttributeWriteError::UnknownStorageType { hash, storage_type })?;
            if !elements.len().is_multiple_of(stride as usize) {
                return Err(AttributeWriteError::RecordStride { hash, words: elements.len(), stride });
            }
            let count = elements.len() / stride as usize;
            check_count(attr, count)?;
            let mut words = Vec::with_capacity(1 + elements.len());
            words.push(count as u32);
            words.extend(elements);
            words
        }
    };

    Ok(if implicit {
        Encoded::Implicit(words)
    } else {
        Encoded::Explicit(type_code, words)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::{EncodedLocation, StorageDescriptor};
    use e3d_attlib::db1_hash;

    fn attr(hash: u32, storage_type_code: u32, logical_count: u32, wide_offset: u32, storage_words: u32) -> TemplateAttr {
        let storage = StorageDescriptor {
            storage_words,
            location: EncodedLocation { raw: wide_offset },
            flags: 0,
        };
        TemplateAttr {
            hash,
            stride: 10,
            aux: storage_type_code,
            storage_type_code,
            logical_count,
            compact: storage,
            wide: storage,
            tail: Vec::new(),
            authoritative: true,
        }
    }

    fn bit_attr(hash: u32, wide_offset: u32, bit: u32) -> TemplateAttr {
        attr(hash, 5, 1, wide_offset | (bit << 20), 1)
    }

    const NOUN: u32 = 0x9C5ED;

    fn template() -> NounTemplate {
        let mut template = NounTemplate::new(NOUN);
        template.attrs = vec![
            attr(db1_hash("XLEN"), 2, 1, 12, 2),     // real scalar
            attr(db1_hash("POS"), 2, 3, 14, 7),      // real array of three
            attr(db1_hash("LOCK"), 3, 1, 21, 1),     // word (bool by attlib)
            attr(db1_hash("BANG"), 3, 1, 22, 1),     // angle word
            attr(db1_hash("PARA"), 7, 4, 23, 5),     // word array
            attr(db1_hash("CREF"), 4, 1, 28, 2),     // reference
            attr(db1_hash("HREF"), 8, 2, 30, 5),     // reference array
            bit_attr(db1_hash("INVI"), 35, 3),       // packed bit 3 of word 35
            attr(db1_hash("DEPTYP"), 9, 40, 36, 3),  // bool array of up to 40
            attr(db1_hash("FUNC"), 10, 12, 39, 4),   // text of up to 12 bytes
            attr(db1_hash("NAME"), 15, 200, 0, 51),  // explicit text
            attr(db1_hash("DESC"), 10, 120, 0, 31),  // explicit text
            attr(db1_hash("SPRE"), 16, 1, 0, 2),     // explicit reference
            attr(db1_hash("HCREF"), 17, 100, 0, 201), // explicit reference array
            attr(db1_hash("TYPEX"), 14, 1, 0, 2),    // explicit record, verbatim
            attr(db1_hash("BITE"), 7, 4, 0, 5),      // explicit word array
            attr(db1_hash("ROKEYS"), 19, 20, 0, 21), // string list
        ];
        template
    }

    fn draft() -> RecordDraft {
        let mut implicit = vec![0u32; 44];
        implicit[10] = WIDE_LAYOUT_FLAG;
        RecordDraft::new(RefNo::new(24_384, 47), NOUN, RefNo::new(24_384, 46), implicit, vec![], vec![]).unwrap()
    }

    fn set(draft: &mut RecordDraft, name: &str, value: DescriptorValue) -> Result<(), AttributeWriteError> {
        draft.set_attribute(&template(), db1_hash(name), &value, TextFormat::Utf8)
    }

    #[test]
    fn implicit_values_land_in_their_slots_in_the_shape_the_reader_decodes() {
        let mut draft = draft();
        set(&mut draft, "XLEN", DescriptorValue::Real(300.0)).unwrap();
        let bits = 300.0f64.to_bits();
        assert_eq!(draft.implicit()[12..14], [bits as u32, (bits >> 32) as u32], "low half first");

        set(&mut draft, "POS", DescriptorValue::RealArray(vec![1000.0, 2000.0, 3000.0])).unwrap();
        assert_eq!(draft.implicit()[14], 3);
        assert_eq!(draft.implicit()[15..17], real_words(1000.0));
        assert_eq!(draft.implicit()[19..21], real_words(3000.0));

        set(&mut draft, "LOCK", DescriptorValue::Bool(true)).unwrap();
        assert_eq!(draft.implicit()[21], 1);
        set(&mut draft, "LOCK", DescriptorValue::Word { raw: 0x1234, text: None }).unwrap();
        assert_eq!(draft.implicit()[21], 0x1234, "a word goes back as its raw word");
        set(&mut draft, "LOCK", DescriptorValue::Int(-2)).unwrap();
        assert_eq!(draft.implicit()[21], (-2i32) as u32);

        assert!(e3d_attlib::is_angle_attr(db1_hash("BANG")), "BANG is a catalogue angle");
        set(&mut draft, "BANG", DescriptorValue::Real(-90.0)).unwrap();
        assert_eq!(draft.implicit()[22] as i32, -9000, "centidegrees, as SCTN.Bangle stores -90");
        set(&mut draft, "BANG", DescriptorValue::Real(11.5)).unwrap();
        assert_eq!(draft.implicit()[22] as i32, 1150);

        set(&mut draft, "PARA", DescriptorValue::IntArray(vec![50, -1, 7])).unwrap();
        assert_eq!(draft.implicit()[23..27], [3, 50, (-1i32) as u32, 7]);
        assert_eq!(draft.implicit()[27], 0, "the slot's fifth word was not needed and not touched");
        set(&mut draft, "PARA", DescriptorValue::WordArray(vec![546_602, 702_806])).unwrap();
        assert_eq!(draft.implicit()[23..27], [2, 546_602, 702_806, 7], "the old third element stays behind the count");

        let target = RefNo::new(13_245, 918_008);
        set(&mut draft, "CREF", DescriptorValue::RefNo(target)).unwrap();
        assert_eq!(draft.implicit()[28..30], [13_245, 918_008]);
        set(&mut draft, "HREF", DescriptorValue::RefNoArray(vec![target, RefNo::new(1, 2)])).unwrap();
        assert_eq!(draft.implicit()[30..35], [2, 13_245, 918_008, 1, 2]);

        *draft.implicit_word_mut(35).unwrap() = 0xF0;
        set(&mut draft, "INVI", DescriptorValue::Bool(true)).unwrap();
        assert_eq!(draft.implicit()[35], 0xF8, "bit 3 set, the other bits of the word kept");
        set(&mut draft, "INVI", DescriptorValue::Bool(false)).unwrap();
        assert_eq!(draft.implicit()[35], 0xF0);

        let mut bools = vec![false; 34];
        bools[0] = true;
        bools[31] = true;
        bools[33] = true;
        set(&mut draft, "DEPTYP", DescriptorValue::BoolArray(bools)).unwrap();
        assert_eq!(draft.implicit()[36..39], [34, 0x8000_0001, 0b10]);

        set(&mut draft, "FUNC", DescriptorValue::Text("PUMP-1".into())).unwrap();
        assert_eq!(
            draft.implicit()[39..42],
            [6, u32::from_be_bytes(*b"PUMP"), u32::from_be_bytes(*b"-1\0\0")],
            "byte count, four bytes to a word, NUL-padded"
        );
        assert_eq!(draft.implicit()[42], 0, "the slot's last word was not needed");
    }

    #[test]
    fn explicit_values_become_fields_of_their_storage_type_code() {
        let mut draft = draft();
        set(&mut draft, "NAME", DescriptorValue::Text("/E3DIO-1".into())).unwrap();
        assert_eq!(
            draft.explicit_words(db1_hash("NAME")),
            Some((15 << 10, vec![8, u32::from_be_bytes(*b"/E3D"), u32::from_be_bytes(*b"IO-1")]))
        );
        set(&mut draft, "DESC", DescriptorValue::Text("abc".into())).unwrap();
        assert_eq!(
            draft.explicit_words(db1_hash("DESC")),
            Some((10 << 10, vec![3, u32::from_be_bytes(*b"abc\0")]))
        );
        let target = RefNo::new(13_244, 108_564);
        set(&mut draft, "SPRE", DescriptorValue::RefNo(target)).unwrap();
        assert_eq!(draft.explicit_words(db1_hash("SPRE")), Some((16 << 10, vec![13_244, 108_564])));
        set(&mut draft, "HCREF", DescriptorValue::RefNoArray(vec![target, RefNo::new(9, 9)])).unwrap();
        assert_eq!(
            draft.explicit_words(db1_hash("HCREF")),
            Some((17 << 10, vec![2, 13_244, 108_564, 9, 9]))
        );
        set(&mut draft, "TYPEX", DescriptorValue::RawWords(vec![1, NOUN])).unwrap();
        assert_eq!(
            draft.explicit_words(db1_hash("TYPEX")),
            Some((14 << 10, vec![1, NOUN])),
            "a raw explicit record is the field's data verbatim, count included"
        );
        set(&mut draft, "BITE", DescriptorValue::RawWords(vec![546_602, 702_806])).unwrap();
        assert_eq!(
            draft.explicit_words(db1_hash("BITE")),
            Some((7 << 10, vec![2, 546_602, 702_806])),
            "a raw word array is its elements, behind a count, because the stream decoder strips the count"
        );

        // Unset removes an explicit field and marks an implicit slot.
        set(&mut draft, "SPRE", DescriptorValue::Unset).unwrap();
        assert!(draft.explicit_words(db1_hash("SPRE")).is_none());
        set(&mut draft, "POS", DescriptorValue::RealArray(vec![1.0])).unwrap();
        set(&mut draft, "POS", DescriptorValue::Unset).unwrap();
        assert_eq!(draft.implicit()[14], UNSET_WORD);
        assert_eq!(draft.implicit()[15..17], real_words(1.0), "only the count word is marked");
    }

    #[test]
    fn what_does_not_fit_or_match_is_refused_and_changes_nothing() {
        let mut draft = draft();
        let before = draft.clone();
        let template = template();
        let text = TextFormat::Utf8;

        assert!(matches!(
            draft.set_attribute(&template, db1_hash("NOPE"), &DescriptorValue::Int(1), text),
            Err(AttributeWriteError::NotInTemplate { .. })
        ));
        assert!(matches!(
            set(&mut draft, "XLEN", DescriptorValue::Text("x".into())),
            Err(AttributeWriteError::TypeMismatch { shape: "a real", given: "Text", .. })
        ));
        assert!(matches!(
            set(&mut draft, "POS", DescriptorValue::RealArray(vec![1.0; 4])),
            Err(AttributeWriteError::TooManyElements { count: 4, max: 3, .. })
        ));
        assert!(matches!(
            set(&mut draft, "FUNC", DescriptorValue::Text("thirteen chars".into())),
            Err(AttributeWriteError::TooManyElements { count: 14, max: 12, .. })
        ));
        assert!(matches!(
            set(&mut draft, "ROKEYS", DescriptorValue::Text("K1".into())),
            Err(AttributeWriteError::StringList { .. })
        ));
        // `–` is GBK A843, a trail byte below 0xA0: no escaped form.
        assert!(matches!(
            draft.set_attribute(&template, db1_hash("DESC"), &DescriptorValue::Text("a–b".into()), TextFormat::Escaped),
            Err(AttributeWriteError::Text { source: TextEncodeError::Unmappable { ch: '–', .. }, .. })
        ));
        assert!(matches!(
            draft.set_attribute(&template, db1_hash("DESC"), &DescriptorValue::Text("反应堆".into()), TextFormat::Detect),
            Err(AttributeWriteError::Text { source: TextEncodeError::FormatUnknown { .. }, .. })
        ));
        assert!(draft.content_eq(&before), "every refusal left the draft as it was");

        // A slot the record is too short for, and a compact record.
        let mut short = RecordDraft::new(
            RefNo::new(1, 1),
            NOUN,
            RefNo::new(1, 0),
            {
                let mut implicit = vec![0u32; 20];
                implicit[10] = WIDE_LAYOUT_FLAG;
                implicit
            },
            vec![],
            vec![],
        )
        .unwrap();
        assert!(matches!(
            short.set_attribute(&template, db1_hash("CREF"), &DescriptorValue::RefNo(RefNo::new(1, 2)), text),
            Err(AttributeWriteError::SlotOutOfRecord { offset: 28, len: 20, .. })
        ));
        let mut compact = RecordDraft::new(RefNo::new(1, 1), NOUN, RefNo::new(1, 0), vec![0u32; 44], vec![], vec![]).unwrap();
        assert!(matches!(
            compact.set_attribute(&template, db1_hash("XLEN"), &DescriptorValue::Real(1.0), text),
            Err(AttributeWriteError::CompactLayout { .. })
        ));

        // UTF-8 text is stored as its bytes under FORMAT 1.
        let mut utf8 = draft.clone();
        utf8.set_attribute(&template, db1_hash("DESC"), &DescriptorValue::Text("反应堆".into()), TextFormat::Utf8).unwrap();
        let (_, data) = utf8.explicit_words(db1_hash("DESC")).unwrap();
        assert_eq!(data[0], 9, "three CJK characters are nine UTF-8 bytes");
        assert_eq!(data.len(), 4);

        // Under any other FORMAT the same text is the escaped run
        // `&~74S&6Q &` (`ams1112_0001` stores 反应堆厂房 as `&~74S&6Q3'7? &`),
        // and the count word is the ten stored bytes, escapes included.
        let mut escaped = draft.clone();
        escaped
            .set_attribute(&template, db1_hash("DESC"), &DescriptorValue::Text("反应堆".into()), TextFormat::Escaped)
            .unwrap();
        let (_, data) = escaped.explicit_words(db1_hash("DESC")).unwrap();
        assert_eq!(data, vec![10, u32::from_be_bytes(*b"&~74"), u32::from_be_bytes(*b"S&6Q"), u32::from_be_bytes(*b" &\0\0")]);
    }
}
