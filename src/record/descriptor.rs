//! Direct decoder for noun-specific descriptors loaded from `*vir.dat`.

use e3d_attlib::{db1_dehash, lookup_system_name, AttrDefEntry};

use crate::record::attrs;
use crate::record::element::ParsedElement;
use crate::record::explicit::ExplicitValue;
use crate::record::template::{
    EncodedLocation, NounDefaults, StorageDescriptor, StorageLayout, TemplateAttr,
    DEFAULT_RECORD_WORD_BIAS, ENCODED_OFFSET_MASK,
};
use crate::refno::RefNo;

/// The two words E3D writes where an attribute has no stored value.
///
/// `core.dll` checks both together everywhere it inspects a value it has just
/// read — `sub_5AB5E10`, `sub_5AB62C0`, `sub_5AB71B0`, `sub_5AB7B90` and
/// `sub_5AB91B0` all guard with `!= 0x80000000 && != 0x80000001` — and
/// `sub_5AB4CF0` fills the unread tail of a record array with `0x80000001`.
/// The check happens on the value the reader returned, never inside the read,
/// so it applies to a whole storage slot rather than to its first word.
pub const UNSET_SENTINELS: [u32; 2] = [0x8000_0000, 0x8000_0001];

#[deprecated(note = "E3D marks an absent value with UNSET_SENTINELS; 0xFFFFFFFF is a real value")]
pub const RAW_USE_DEFAULT: u32 = 0xFFFF_FFFF;

fn is_unset_word(word: u32) -> bool {
    UNSET_SENTINELS.contains(&word)
}

/// How one element of a storage type is laid out.
///
/// `core.dll` sizes every value through `dbl_6453DF8[storage_type_code]`, a
/// table of elements-per-dword: a scalar occupies `1 / ratio` words and an
/// array of `n` occupies `ceil(n / ratio)`. The table sits in `.bss` and is
/// filled at startup by `sub_5AE4A90` (called from `sub_5AE31E0`) — but from
/// constants: eight 16-byte `.rdata` literals at `0x5D8B9B0..=0x5D8BA20` plus
/// two `0.5` immediates, so it can be read out of the E3D 3.1 IDB after all.
/// Read that way (type: elements per dword) it is
///
/// ```text
///  0: 0     1: 0     2: 0.5   3: 1     4: 0.5   5: 32    6: 0.5   7: 1
///  8: 0.5   9: 32   10: 4    11: 0.25 12: 0.5  13: 0.5  14: 1    15: 4
/// 16: 0.5  17: 0.5  18: 1    19: 4    20: 1    21: 0.5  22: 4    23: 4
/// ```
///
/// The same ratio is recoverable from descriptor geometry, because
/// `storage_words` is exactly `1 + ceil(logical_count / ratio)` for an array.
/// Fitting it over all 35734 descriptors in the 20 shipped `*vir.dat` files
/// leaves no row unexplained (`tools/probe_type_widths.py`) and agrees with the
/// table above on every type a descriptor uses — 2..=10 and 14..=19. Types 11,
/// 12, 13 and 20..=23 never appear in a descriptor: 20..=23 are the
/// out-of-record array kinds whose payload `sub_5AE9960` copies by its
/// `(page_no, packed)` address, so nothing below needs a shape for them. Type
/// 14's `1` is only a fallback; its real width comes from the descriptor, in
/// core.dll too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageShape {
    /// 2, 6 — `f64` in a wide record, `f32` in a compact one.
    Real,
    /// 3, 7, 18 — one word per element.
    Word,
    /// 4, 8, 16, 17 — a two-word `RefNo` per element.
    Reference,
    /// 5 — one bit inside the storage word, indexed by `location >> 20`.
    /// The only type that ever sets those upper bits: 582 of 1839 do, and no
    /// descriptor of any other type does.
    PackedBit,
    /// 9 — 32 booleans per word. `TRINCO.DEPTYP` declares 500000 of them in
    /// 15626 words, `LOOPTS.INVI` 500 in 17.
    BitSet,
    /// 10, 15 — four characters per word; the count word counts
    /// characters, not words. `BRAN.NAME` declares 200 characters in 51 words.
    Characters,
    /// 19 — a list of strings: the on-disk form of the
    /// `std::vector<std::string>` that `db_cxx_put_str_vector` (routine 640,
    /// `sub_5AAA8B0` → `sub_5AD5060`) takes. That routine packs each string
    /// four characters to a word (after the project charset conversion when
    /// header attribute 18 is not UTF-8) and builds
    /// `[item_count] { [index][word_count][byte_length][packed text × word_count] }*`
    /// — a 0-based item index, the item's word count and its byte length
    /// ahead of every string — which `4SCEA` (`sub_5AB7B90`, its `type == 19`
    /// branch) writes verbatim: into an explicit entry headed
    /// `word_count | 19 << 26` (no separate count word, the length in words,
    /// never divided by the width table; the explicit reader `sub_5AB4CF0`,
    /// `case 19`, copies it back as it is) or into the implicit slot when the
    /// descriptor has one. Type 23 is the same value moved out of the record
    /// into a hunk (`sub_5AB5BC0` treats 19 and 23 as one type, as it does
    /// 6/21, 7/20 and 10/22). Both are taken apart by
    /// [`crate::record::hunk::decode_text_list`] into
    /// [`DescriptorValue::TextList`].
    ///
    /// Until 2026-09-22 type 19 shared `Characters`, which would have taken
    /// the item count for a character count and the item headers for text;
    /// it never happened, because every type-19 descriptor in the schema
    /// files (`ROKEYS`/`ROVALS` on FIXING, PFIT and the HR* nouns, `CLAURI`,
    /// `DIURIS`, `TADFLT`, `VLOCDE`, `CFGTAV`, …) is unset on every element of
    /// the 1 000+ E3D 3.1 databases installed here — 9 421 ROKEYS/ROVALS
    /// slots, 116 688 `TABITE.CLAURI`, 6 586 `UDA.TADFLT` — and most have no
    /// implicit slot at all (word offset 0). E3D 2.1's templates have no type
    /// 19. What sets type-19 values today is this crate's own writer
    /// (`RecordDraft::set_explicit_text_list`, `set_attribute`), and a real
    /// E3D 3.1 reads those back (`tools/e3d/write_gate.ps1`, the FIXING's
    /// `Rokeys` / `Rovals`).
    TextList,
    /// 14 — a fixed-width record whose width the descriptor states itself as
    /// `(compact.storage_words - 1) / logical_count`. That division is exact
    /// for all 5387 type-14 descriptors and yields 1, 2, 3 or 4 words.
    Record,
}

impl StorageShape {
    pub fn of(storage_type_code: u32) -> Option<Self> {
        Some(match storage_type_code {
            2 | 6 => Self::Real,
            3 | 7 | 18 => Self::Word,
            4 | 8 | 16 | 17 => Self::Reference,
            5 => Self::PackedBit,
            9 => Self::BitSet,
            10 | 15 => Self::Characters,
            19 => Self::TextList,
            14 => Self::Record,
            _ => return None,
        })
    }

    /// Words one element occupies. `None` for the shapes that pack several
    /// elements into a word or take their width from the descriptor.
    fn words_per_element(self, layout: StorageLayout) -> Option<u32> {
        Some(match self {
            Self::Real if layout == StorageLayout::Wide => 2,
            Self::Real => 1,
            Self::Word => 1,
            Self::Reference => 2,
            Self::PackedBit | Self::BitSet | Self::Characters | Self::TextList | Self::Record => return None,
        })
    }
}

/// Whether a descriptor addresses a count-prefixed run rather than a lone
/// value, per `sub_5AB5BC0`:
///
/// ```c
/// if (!logical_count || logical_count > 1 || type == 14 || type == 17 || type == 19)
///     is_array = 1;
/// ```
pub fn descriptor_is_array(descriptor: &TemplateAttr) -> bool {
    descriptor.logical_count == 0
        || descriptor.logical_count > 1
        || matches!(descriptor.storage_type_code, 14 | 17 | 19)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DescriptorValue {
    /// E3D's logical `unset` value. This is distinct from numeric zero,
    /// `false`, an empty string, and `Nulref`.
    Unset,
    Bool(bool),
    Int(i32),
    Real(f64),
    Text(String),
    RefNo(RefNo),
    Word {
        raw: u32,
        text: Option<String>,
    },
    IntArray(Vec<i32>),
    RealArray(Vec<f64>),
    RefNoArray(Vec<RefNo>),
    WordArray(Vec<u32>),
    /// Storage type 9: booleans packed 32 to a word.
    BoolArray(Vec<bool>),
    /// Storage type 19 inline, or a type-23 hunk: a list of strings, in order.
    TextList(Vec<String>),
    /// Words this decoder gives no shape to — a type-14 record of several
    /// words per element, a type-17 reference array or type-18 set handed
    /// over as stored, a one-word array attlib does not type — and a **hunk
    /// directory entry** (`[1][page][packed][count][words]`) when the record
    /// was decoded without its pages
    /// ([`crate::ReadOnlyEngine::extract_element_with_descriptors`] resolves
    /// those; the pure `extract_parsed_element_with_descriptors` cannot).
    RawWords(Vec<u32>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeExtractionStatus {
    Decoded,
    DecodedExplicit,
    /// No persisted value exists and the noun's own default record supplied
    /// one, the way `sub_5B03B90` does. This is the authoritative default:
    /// it comes out of the same `*vir.dat` child as the descriptor.
    TemplateDefault,
    /// No persisted value exists and the logical catalog supplied the
    /// type-aware E3D default.
    CatalogDefault,
    /// No persisted value exists; an E3D-compatible pseudo-attribute
    /// evaluator derived the value from other element state.
    RuntimeDerived {
        evaluator: String,
    },
    NonImplicit,
    DefaultRequired,
    /// No longer produced. The descriptor states the storage shape on its own,
    /// so a missing attlib definition only leaves integer-versus-word
    /// undetermined, which surfaces as [`DescriptorValue::Word`] or
    /// [`DescriptorValue::RawWords`] rather than as a failure.
    #[deprecated(note = "the descriptor is sufficient to decode; nothing produces this")]
    MetadataMissing,
    Unsupported {
        detail: String,
    },
    OutOfBounds {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedAttribute {
    pub hash: u32,
    pub name: String,
    pub storage_type_code: u32,
    pub semantic_type_code: Option<u32>,
    pub semantic_size: Option<u32>,
    pub layout: StorageLayout,
    pub storage: StorageDescriptor,
    pub status: AttributeExtractionStatus,
    pub value: Option<DescriptorValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementExtraction {
    pub refno: RefNo,
    pub owner: RefNo,
    pub noun_hash: u32,
    pub noun_name: Option<String>,
    pub implicit_word_count: u32,
    pub format_word_10: u32,
    pub layout: StorageLayout,
    pub attributes: Vec<ExtractedAttribute>,
}

impl ElementExtraction {
    /// Approximate heap footprint in bytes: the noun name plus every extracted
    /// attribute with its name and the payload behind its value (text, arrays).
    /// Lengths, not capacities — an estimate for cache accounting
    /// (`DbSet::element_cache_stats`); the inline part is
    /// `size_of::<ElementExtraction>()` on the caller's side.
    pub fn approx_heap_bytes(&self) -> usize {
        let attributes: usize = self
            .attributes
            .iter()
            .map(|attribute| {
                std::mem::size_of::<ExtractedAttribute>()
                    + attribute.name.len()
                    + match &attribute.value {
                        Some(DescriptorValue::Text(text)) => text.len(),
                        Some(DescriptorValue::Word { text, .. }) => {
                            text.as_ref().map_or(0, String::len)
                        }
                        Some(DescriptorValue::IntArray(values)) => values.len() * 4,
                        Some(DescriptorValue::RealArray(values)) => values.len() * 8,
                        Some(DescriptorValue::RefNoArray(values)) => values.len() * 8,
                        Some(DescriptorValue::WordArray(values)) => values.len() * 4,
                        Some(DescriptorValue::BoolArray(values)) => values.len(),
                        Some(DescriptorValue::TextList(items)) => {
                            items.iter().map(|item| std::mem::size_of::<String>() + item.len()).sum()
                        }
                        Some(DescriptorValue::RawWords(values)) => values.len() * 4,
                        Some(
                            DescriptorValue::Unset
                            | DescriptorValue::Bool(_)
                            | DescriptorValue::Int(_)
                            | DescriptorValue::Real(_)
                            | DescriptorValue::RefNo(_),
                        )
                        | None => 0,
                    }
            })
            .sum();
        self.noun_name.as_ref().map_or(0, String::len) + attributes
    }

    /// 按名字找属性，大小写不敏感，取**第一条**同名的。
    ///
    /// 就是顺序扫。2026-09-04 试过给它建一张「大小写无关指纹 → 下标」的排序索引，
    /// 实测**更慢**（ams8000 生成 280ms → 300ms）：名字表短，104 万次查询平均只比
    /// 9 个名字就命中，而建索引要为每一件抽取件排一次序，建表钱比省下的比较钱还多。
    /// 再想加索引之前，先把 `attr_lookups` 与平均扫描长度重新量一遍。
    pub fn find_attribute(&self, name: &str) -> Option<&ExtractedAttribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
    }
}

pub fn extract_attribute(
    parsed: &ParsedElement,
    descriptor: &TemplateAttr,
    def: Option<&AttrDefEntry>,
    layout: StorageLayout,
    name: String,
) -> ExtractedAttribute {
    let storage = descriptor.storage(layout);
    let mut result = ExtractedAttribute {
        hash: descriptor.hash,
        name,
        storage_type_code: descriptor.storage_type_code,
        semantic_type_code: def.map(|d| d.data_type),
        semantic_size: def.map(|d| d.size),
        layout,
        storage,
        status: AttributeExtractionStatus::Decoded,
        value: None,
    };

    if !descriptor.authoritative {
        result.status = AttributeExtractionStatus::Unsupported {
            detail: "template does not carry physical descriptor locations".into(),
        };
        return result;
    }
    // Catalogue geometry can hold a formula where a number would go, and the
    // formula outranks the slot: `BLTP.Bdiameter` reads 0.0 and prints `( 24 )`.
    // It has to be looked for before the slot is, because an attribute with no
    // implicit slot at all — `BLTP.Bthkness` — can carry one too.
    if let Some(expression) = stored_expression(parsed, descriptor.hash) {
        result.status = AttributeExtractionStatus::DecodedExplicit;
        result.value = Some(DescriptorValue::Text(expression));
        return result;
    }
    let offset = storage.location.word_offset();
    if offset == 0 {
        result.status = AttributeExtractionStatus::NonImplicit;
        return result;
    }
    let total_words = parsed.implicit_data.len() / 4;
    let end = offset as usize + storage.storage_words as usize;
    if storage.storage_words == 0 || end > total_words {
        result.status = AttributeExtractionStatus::OutOfBounds {
            detail: format!(
                "offset {offset} + storage {} exceeds implicit words {total_words}",
                storage.storage_words
            ),
        };
        return result;
    }
    if descriptor.storage_type_code == 5 {
        let Some(bit) = storage.location.bool_bit_index() else {
            result.status = AttributeExtractionStatus::OutOfBounds {
                detail: format!(
                    "BOOL bit index {} exceeds 31",
                    storage.location.upper_bits()
                ),
            };
            return result;
        };
        let Some(word) = read_u32(&parsed.implicit_data, offset) else {
            result.status = AttributeExtractionStatus::OutOfBounds {
                detail: "BOOL storage word is unavailable".into(),
            };
            return result;
        };
        result.value = Some(DescriptorValue::Bool(((word >> bit) & 1) != 0));
        return result;
    }

    // A packed bit is the one storage that has no word of its own to mark, so
    // the sentinel check starts here. For an array the marked word is the
    // count; for a scalar it is the value itself.
    if read_u32(&parsed.implicit_data, offset).is_some_and(is_unset_word) {
        result.status = AttributeExtractionStatus::DefaultRequired;
        return result;
    }

    match decode_value(parsed, descriptor, def, layout, storage) {
        Ok(value) => result.value = Some(value),
        Err(DecodeFailure::Unsupported(detail)) => {
            result.status = AttributeExtractionStatus::Unsupported { detail }
        }
        Err(DecodeFailure::OutOfBounds(detail)) => {
            result.status = AttributeExtractionStatus::OutOfBounds { detail }
        }
    }
    result
}

/// The PML expression held for an attribute, if the element carries one.
///
/// The explicit stream keys it by the two's-complement negation of the
/// attribute's hash, which puts it above the UDA floor and so out of reach of
/// every dictionary. Only a body that decodes whole is accepted, so an
/// unrelated UDA that happens to land on the negated hash cannot be mistaken
/// for a formula.
fn stored_expression(parsed: &ParsedElement, hash: u32) -> Option<String> {
    crate::record::catalogue_pml::render(stored_expression_words(parsed, hash)?)
}

/// The words of that expression, for a consumer that needs the number rather
/// than the text.
///
/// [`extract_attribute`] renders the body and keeps only the string, because a
/// descriptor value is what E3D would print. Evaluating it needs the program
/// itself: re-parsing `( ATTRIB PARA[5 ] + ATTRIB PARA[4 ] )` back out of the
/// display text would be a second, weaker reader of a format already read here.
///
/// Unlike [`stored_expression`] this does not require the body to decode — the
/// caller can tell an entry it cannot read from an entry that is not there.
pub fn stored_expression_words(parsed: &ParsedElement, hash: u32) -> Option<&[i32]> {
    let key = hash.wrapping_neg();
    parsed
        .explicit_attributes
        .iter()
        .find(|attribute| attribute.hash == key)
        .and_then(|attribute| match &attribute.value {
            ExplicitValue::IntArray(words) => Some(words.as_slice()),
            _ => None,
        })
}

/// The noun's own default for one attribute, out of `*vir.dat` chunks 1 and 2.
///
/// This is what `sub_5B03B90` (GCEAD) returns when an element stores nothing:
/// an implicit-resident attribute reads its default out of the chunk-1 record
/// at `wide_word_offset - 11`, and one that lives in the explicit stream finds
/// it by hash in chunk 2. Defaults are stored in the wide form regardless of
/// how the element itself is laid out — GCEAD indexes with `descriptor[8]`,
/// the wide column, in both branches.
pub fn extract_default(
    descriptor: &TemplateAttr,
    def: Option<&AttrDefEntry>,
    defaults: &NounDefaults,
    name: String,
) -> Option<ExtractedAttribute> {
    if !descriptor.authoritative {
        return None;
    }
    let offset = descriptor.wide.location.word_offset();
    let (record, descriptor) = if offset == 0 {
        let entry = defaults.explicit_for(descriptor.hash)?;
        // Re-anchor the payload at the bias word so the same decoder can read
        // it; the packed-bit index in the upper location bits is preserved.
        let mut relocated = descriptor.clone();
        let raw = (descriptor.wide.location.raw & !ENCODED_OFFSET_MASK) | DEFAULT_RECORD_WORD_BIAS;
        relocated.wide = StorageDescriptor {
            storage_words: entry.words.len() as u32,
            location: EncodedLocation { raw },
            flags: descriptor.wide.flags,
        };
        (entry.words.clone(), relocated)
    } else {
        if defaults.implicit.is_empty() {
            return None;
        }
        (defaults.implicit.clone(), descriptor.clone())
    };

    let mut bytes = vec![0u8; DEFAULT_RECORD_WORD_BIAS as usize * 4];
    for word in &record {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    let synthetic = ParsedElement {
        impl_len_words: (bytes.len() / 4) as i32,
        refno: RefNo::new(0, 0),
        noun_hash: 0,
        owner: RefNo::new(0, 0),
        prefix_offset: 0,
        implicit_data: bytes,
        members: Vec::new(),
        explicit_attributes: Vec::new(),
        record_end: 0,
        total_size: 0,
        // Template defaults come from the schema files, not from a database,
        // so no header states their format; the guess is the only reading.
        text_format: crate::record::text::TextFormat::Detect,
    };
    let extracted = extract_attribute(&synthetic, &descriptor, def, StorageLayout::Wide, name);
    extracted.value.is_some().then_some(extracted)
}

enum DecodeFailure {
    Unsupported(String),
    OutOfBounds(String),
}

fn decode_value(
    parsed: &ParsedElement,
    descriptor: &TemplateAttr,
    def: Option<&AttrDefEntry>,
    layout: StorageLayout,
    storage: StorageDescriptor,
) -> Result<DescriptorValue, DecodeFailure> {
    let offset = storage.location.word_offset();
    let Some(shape) = StorageShape::of(descriptor.storage_type_code) else {
        return Err(DecodeFailure::Unsupported(format!(
            "storage type {} is not one of the fifteen the schema files use",
            descriptor.storage_type_code
        )));
    };
    // `def.size` used to be able to promote a scalar to an array. It cannot:
    // the descriptor already states the shape, and the two disagree — 12
    // descriptors in `desvir.dat` alone declare `logical_count == 0`, which
    // `sub_5AB5BC0` reads as an array and a `size` of 1 would read as a scalar.
    let is_array = descriptor_is_array(descriptor);

    match shape {
        StorageShape::BitSet => {
            let count = read_count(parsed, descriptor, storage)?;
            let words = count.div_ceil(32);
            require_storage(storage, 1 + words)?;
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                let word =
                    read_u32(&parsed.implicit_data, offset + 1 + index / 32).ok_or_else(|| {
                        DecodeFailure::OutOfBounds(format!("boolean item {index} is truncated"))
                    })?;
                out.push((word >> (index % 32)) & 1 != 0);
            }
            Ok(DescriptorValue::BoolArray(out))
        }
        StorageShape::Characters => {
            let count = read_count(parsed, descriptor, storage)?;
            let words = count.div_ceil(4);
            require_storage(storage, 1 + words)?;
            let mut bytes = Vec::with_capacity(count as usize);
            for index in 0..words {
                let word =
                    read_u32(&parsed.implicit_data, offset + 1 + index).ok_or_else(|| {
                        DecodeFailure::OutOfBounds(format!("character word {index} is truncated"))
                    })?;
                bytes.extend_from_slice(&word.to_be_bytes());
            }
            bytes.truncate(count as usize);
            Ok(DescriptorValue::Text(crate::record::text::decode_as(
                &bytes,
                parsed.text_format,
            )))
        }
        // The slot holds the string-list buffer as `4SCEA` wrote it: its
        // first word is the item count (0 for an empty list), and the items
        // follow with their own headers, so the list says where it ends and
        // whatever the slot holds past it is not read.
        StorageShape::TextList => {
            let words = read_words(parsed, offset, storage.storage_words)?;
            crate::record::hunk::decode_text_list(&words, parsed.text_format)
                .map(DescriptorValue::TextList)
                .ok_or_else(|| {
                    DecodeFailure::Unsupported(format!(
                        "the {}-word slot does not hold a string list ({} items announced)",
                        storage.storage_words, words[0]
                    ))
                })
        }
        StorageShape::Record => {
            // `sub_5AB5600`: `v18 = v17 * ((a3[4] - 1) / a3[3])`. The stride
            // comes from the compact column in both layouts, and a type-14
            // element is the same size either way.
            let stride = descriptor
                .compact
                .storage_words
                .checked_sub(1)
                .and_then(|words| words.checked_div(descriptor.logical_count.max(1)))
                .filter(|stride| *stride > 0)
                .ok_or_else(|| {
                    DecodeFailure::Unsupported(format!(
                        "record stride is undefined: compact storage {} over {} elements",
                        descriptor.compact.storage_words, descriptor.logical_count
                    ))
                })?;
            let count = read_count(parsed, descriptor, storage)?;
            require_storage(storage, 1 + count * stride)?;
            let words = read_words(parsed, offset + 1, count * stride)?;
            Ok(if stride == 1 {
                DescriptorValue::IntArray(words.into_iter().map(|word| word as i32).collect())
            } else {
                DescriptorValue::RawWords(words)
            })
        }
        StorageShape::Real => {
            let words_per = shape.words_per_element(layout).expect("real has a width");
            if !is_array {
                return read_real(&parsed.implicit_data, offset, layout)
                    .map(DescriptorValue::Real)
                    .ok_or_else(|| {
                        DecodeFailure::OutOfBounds("numeric scalar is truncated".into())
                    });
            }
            let count = read_count(parsed, descriptor, storage)?;
            require_storage(storage, 1 + count * words_per)?;
            let mut values = Vec::with_capacity(count as usize);
            for index in 0..count {
                let word = offset + 1 + index * words_per;
                values.push(
                    read_real(&parsed.implicit_data, word, layout).ok_or_else(|| {
                        DecodeFailure::OutOfBounds(format!("numeric item {index} is truncated"))
                    })?,
                );
            }
            Ok(DescriptorValue::RealArray(values))
        }
        StorageShape::Reference => {
            if !is_array {
                return attrs::read_refno(&parsed.implicit_data, offset)
                    .map(DescriptorValue::RefNo)
                    .ok_or_else(|| {
                        DecodeFailure::OutOfBounds("reference scalar is truncated".into())
                    });
            }
            let count = read_count(parsed, descriptor, storage)?;
            require_storage(storage, 1 + count * 2)?;
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                out.push(
                    attrs::read_refno(&parsed.implicit_data, offset + 1 + index * 2).ok_or_else(
                        || {
                            DecodeFailure::OutOfBounds(format!(
                                "reference item {index} is truncated"
                            ))
                        },
                    )?,
                );
            }
            Ok(DescriptorValue::RefNoArray(out))
        }
        // One word per element. Which of integer, boolean and word it holds is
        // the one thing the descriptor does not say — `sub_5AB5BC0` accepts
        // whichever the caller asks for — so the attlib definition decides, and
        // without one the word is surfaced as a word.
        StorageShape::Word if !is_array => {
            let raw = read_u32(&parsed.implicit_data, offset)
                .ok_or_else(|| DecodeFailure::OutOfBounds("scalar is truncated".into()))?;
            Ok(match def.map(|def| def.data_type) {
                Some(1) => DescriptorValue::Int(raw as i32),
                // attlib's boolean is a word holding 0 or 1 (`4SCEA` writes a
                // one-word LOGICAL as exactly that, and the packed-bit form
                // above is 0 or 1 by construction). A word that is neither is
                // not a boolean whatever attlib says of it: `UDA.DTYP` is typed
                // 3 there and holds the words `TEXT`, `REAL`, `INTE` on disk —
                // 185 elements of `ams5100_0001`, every one printed by E3D as
                // `Dtype TEXT` and the like — so it is surfaced as the word it
                // is, name and all, rather than collapsed to `true`.
                Some(3) if raw <= 1 => DescriptorValue::Bool(raw == 1),
                Some(3) => DescriptorValue::Word {
                    raw,
                    text: word_name(raw),
                },
                // An angle attribute stored in one word holds hundredths of a
                // degree: `SCTN.Bangle` writes -9000 where `q att` prints -90,
                // and `SSLC.Pxbshear` writes 1150 where it prints 11.5. attlib
                // carries no `ATGTDF` definition for several of these (`BANG`
                // among them), so its `data_type` cannot decide — the catalog
                // `ANGL` unit, baked into e3d-attlib, is the only authority that
                // says to read the word as a signed, centidegree-scaled real
                // rather than an unsigned word. This mirrors the reading the TTY
                // reconciliation already accepts for a word-shaped angle.
                _ if e3d_attlib::is_angle_attr(descriptor.hash) => {
                    DescriptorValue::Real(f64::from(raw as i32) / 100.0)
                }
                _ => DescriptorValue::Word {
                    raw,
                    text: word_name(raw),
                },
            })
        }
        StorageShape::Word => {
            let count = read_count(parsed, descriptor, storage)?;
            require_storage(storage, 1 + count)?;
            let words = read_words(parsed, offset + 1, count)?;
            Ok(match def.map(|def| def.data_type) {
                Some(1) => {
                    DescriptorValue::IntArray(words.into_iter().map(|word| word as i32).collect())
                }
                Some(5 | 6) => DescriptorValue::WordArray(words),
                // Undetermined between the two. `RawWords` is what the implicit
                // path already means by that, and it keeps both readings open
                // for the TTY reconciliation instead of committing to one.
                _ => DescriptorValue::RawWords(words),
            })
        }
        StorageShape::PackedBit => Err(DecodeFailure::Unsupported(
            "packed bits are decoded before dispatch".into(),
        )),
    }
}

fn read_words(parsed: &ParsedElement, first: u32, count: u32) -> Result<Vec<u32>, DecodeFailure> {
    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count {
        out.push(
            read_u32(&parsed.implicit_data, first + index)
                .ok_or_else(|| DecodeFailure::OutOfBounds(format!("word {index} is truncated")))?,
        );
    }
    Ok(out)
}

/// Text behind a word code: a system name if E3D reserves one, otherwise the
/// four characters the hash was built from.
pub fn word_name(raw: u32) -> Option<String> {
    lookup_system_name(raw)
        .map(String::from)
        .or_else(|| db1_dehash(raw))
}

fn read_count(
    parsed: &ParsedElement,
    descriptor: &TemplateAttr,
    storage: StorageDescriptor,
) -> Result<u32, DecodeFailure> {
    let count = read_u32(&parsed.implicit_data, storage.location.word_offset())
        .ok_or_else(|| DecodeFailure::OutOfBounds("array count is unavailable".into()))?;
    // `logical_count == 0` is the descriptor declining to state an upper
    // bound, not a bound of zero: `sub_5AB5BC0` reads it as "array" and no
    // ceiling. The declared storage still bounds the read.
    let ceiling = if descriptor.logical_count == 0 {
        u32::MAX
    } else {
        descriptor.logical_count
    };
    if count > ceiling {
        return Err(DecodeFailure::OutOfBounds(format!(
            "array count {count} exceeds descriptor maximum {}",
            descriptor.logical_count
        )));
    }
    Ok(count)
}

fn require_storage(storage: StorageDescriptor, required: u32) -> Result<(), DecodeFailure> {
    if required > storage.storage_words {
        return Err(DecodeFailure::OutOfBounds(format!(
            "decoded value requires {required} words, descriptor declares {}",
            storage.storage_words
        )));
    }
    Ok(())
}

fn read_u32(data: &[u8], word: u32) -> Option<u32> {
    let offset = word as usize * 4;
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_real(data: &[u8], word: u32, layout: StorageLayout) -> Option<f64> {
    match layout {
        StorageLayout::Compact => attrs::read_f32(data, word).map(f64::from),
        StorageLayout::Wide => attrs::read_f64(data, word),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template::{EncodedLocation, StorageDescriptor};
    use e3d_attlib::AttrDefEntry;

    fn parsed(words: &[u32]) -> ParsedElement {
        let mut data = Vec::with_capacity(words.len() * 4);
        for word in words {
            data.extend_from_slice(&word.to_be_bytes());
        }
        ParsedElement {
            impl_len_words: words.len() as i32,
            refno: RefNo::new(1, 2),
            noun_hash: 3,
            owner: RefNo::new(0, 0),
            prefix_offset: 0,
            implicit_data: data,
            members: Vec::new(),
            explicit_attributes: Vec::new(),
            record_end: words.len() * 4,
            total_size: words.len() * 4,
            text_format: crate::record::text::TextFormat::Detect,
        }
    }

    fn descriptor(
        storage_type_code: u32,
        logical_count: u32,
        raw: u32,
        words: u32,
    ) -> TemplateAttr {
        TemplateAttr {
            hash: 0xAA,
            stride: 10,
            aux: storage_type_code,
            storage_type_code,
            logical_count,
            compact: StorageDescriptor {
                storage_words: words,
                location: EncodedLocation { raw },
                flags: 0,
            },
            wide: StorageDescriptor {
                storage_words: words,
                location: EncodedLocation { raw },
                flags: 0,
            },
            tail: vec![],
            authoritative: true,
        }
    }

    #[test]
    fn packed_bool_uses_high_location_bits() {
        let mut words = vec![0u32; 16];
        words[11] = 1 << 3;
        let d = descriptor(5, 1, (3 << 20) | 11, 1);
        let def = AttrDefEntry::new(d.hash, 3, 1, 1);
        let got = extract_attribute(
            &parsed(&words),
            &d,
            Some(&def),
            StorageLayout::Wide,
            "B".into(),
        );
        assert_eq!(got.value, Some(DescriptorValue::Bool(true)));
    }

    #[test]
    fn compact_numeric_array_promotes_f32() {
        let mut words = vec![0u32; 18];
        words[11] = 3;
        words[12] = 1.25f32.to_bits();
        words[13] = (-2.5f32).to_bits();
        words[14] = 3.75f32.to_bits();
        let d = descriptor(6, 3, 11, 4);
        let def = AttrDefEntry::new(d.hash, 8, 2, 3);
        let got = extract_attribute(
            &parsed(&words),
            &d,
            Some(&def),
            StorageLayout::Compact,
            "POS".into(),
        );
        assert_eq!(
            got.value,
            Some(DescriptorValue::RealArray(vec![1.25, -2.5, 3.75]))
        );
    }

    #[test]
    fn zero_offset_is_reported_not_skipped() {
        let d = descriptor(2, 1, 0, 2);
        let def = AttrDefEntry::new(d.hash, 2, 1, 1);
        let got = extract_attribute(
            &parsed(&[0; 16]),
            &d,
            Some(&def),
            StorageLayout::Wide,
            "R".into(),
        );
        assert_eq!(got.status, AttributeExtractionStatus::NonImplicit);
        assert_eq!(got.value, None);
    }

    /// An angle stored in one word is hundredths of a degree. `BANG` has no
    /// attlib definition, so before the catalog unit was consulted this read
    /// back as the unsigned word 4294958296; now it is the -90.0 that E3D
    /// prints. `-9000 as i32 as f64 / 100.0`.
    #[test]
    fn a_word_stored_angle_reads_as_scaled_degrees() {
        let mut words = vec![0u32; 16];
        words[11] = (-9000i32) as u32;
        let mut d = descriptor(3, 1, 11, 1);
        d.hash = e3d_attlib::db1_hash("BANG");
        let got = extract_attribute(
            &parsed(&words),
            &d,
            None,
            StorageLayout::Wide,
            "BANG".into(),
        );
        assert_eq!(got.status, AttributeExtractionStatus::Decoded);
        assert_eq!(got.value, Some(DescriptorValue::Real(-90.0)));
    }

    /// The scaling is scoped to attributes the catalog marks as angles: a word
    /// with the same bits but no angle unit stays an unsigned word, because the
    /// generation chain would otherwise scale a genuine word code.
    #[test]
    fn a_word_without_an_angle_unit_stays_a_word() {
        let mut words = vec![0u32; 16];
        words[11] = (-9000i32) as u32;
        let d = descriptor(3, 1, 11, 1);
        assert!(!e3d_attlib::is_angle_attr(d.hash));
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "X".into());
        assert_eq!(
            got.value,
            Some(DescriptorValue::Word {
                raw: 4_294_958_296,
                text: word_name(4_294_958_296),
            })
        );
    }

    /// A storage type outside the fifteen the schema files use is reported,
    /// not silently read as words.
    #[test]
    fn unknown_storage_type_is_reported_not_skipped() {
        let d = descriptor(99, 1, 11, 1);
        let got = extract_attribute(&parsed(&[0; 16]), &d, None, StorageLayout::Wide, "X".into());
        assert!(matches!(
            got.status,
            AttributeExtractionStatus::Unsupported { .. }
        ));
    }

    /// Type 9 packs its booleans 32 to a word behind a count, the same shape
    /// `LOOPTS.INVI` uses for 500 of them in 17 words.
    #[test]
    fn bitset_decodes_one_bool_per_bit() {
        let mut words = vec![0u32; 20];
        words[11] = 35;
        words[12] = 0b0101;
        words[13] = 0b0010;
        let mut d = descriptor(9, 40, 11, 3);
        d.compact.storage_words = 3;
        d.wide.storage_words = 3;
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "I".into());
        let DescriptorValue::BoolArray(bits) = got.value.expect("decoded") else {
            panic!("expected a bool array, got {:?}", got.status);
        };
        assert_eq!(bits.len(), 35);
        assert_eq!(&bits[..4], &[true, false, true, false]);
        assert_eq!(&bits[32..35], &[false, true, false]);
    }

    /// attlib's data type 3 is a boolean only while the word is 0 or 1. The
    /// word `TEXT` in a slot attlib calls boolean — `UDA.DTYP`, 185 elements
    /// of `ams5100_0001`, printed `Dtype TEXT` by E3D — is the word, with its
    /// name, not `true`; 0 and 1 are still `false` and `true`.
    #[test]
    fn a_boolean_typed_word_that_is_not_0_or_1_is_the_word_it_is() {
        let text_word = e3d_attlib::db1_hash("TEXT");
        let d = descriptor(3, 1, 11, 1);
        let def = AttrDefEntry::new(d.hash, 3, 1, 1);
        let read = |word: u32| {
            let mut words = vec![0u32; 16];
            words[11] = word;
            extract_attribute(&parsed(&words), &d, Some(&def), StorageLayout::Wide, "DTYP".into())
        };
        assert_eq!(
            read(text_word).value,
            Some(DescriptorValue::Word {
                raw: text_word,
                text: Some("TEXT".into())
            })
        );
        assert_eq!(read(1).value, Some(DescriptorValue::Bool(true)));
        assert_eq!(read(0).value, Some(DescriptorValue::Bool(false)));
        // The unset marker is not a boolean either: the default is asked for.
        assert_eq!(read(UNSET_SENTINELS[1]).status, AttributeExtractionStatus::DefaultRequired);
    }

    /// A type-19 slot holds the string-list buffer as `4SCEA` wrote it and
    /// decodes item by item; an empty list is a list; whatever the slot holds
    /// past the list is not read; a slot that does not hold the shape is
    /// reported rather than read as text.
    #[test]
    fn a_string_list_slot_decodes_item_by_item() {
        let mut words = vec![0u32; 24];
        // "AB" and "CDEFG", then two stale words the list does not reach.
        words[11..21].copy_from_slice(&[2, 0, 1, 2, 0x4142_0000, 1, 2, 5, 0x4344_4546, 0x4700_0000]);
        words[21] = 0xDEAD_BEEF;
        let mut d = descriptor(19, 0, 11, 12);
        d.compact.storage_words = 12;
        d.wide.storage_words = 12;
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "ROKEYS".into());
        assert_eq!(got.status, AttributeExtractionStatus::Decoded);
        assert_eq!(got.value, Some(DescriptorValue::TextList(vec!["AB".into(), "CDEFG".into()])));

        let mut empty = vec![0u32; 24];
        empty[11] = 0;
        let got = extract_attribute(&parsed(&empty), &d, None, StorageLayout::Wide, "ROKEYS".into());
        assert_eq!(got.value, Some(DescriptorValue::TextList(vec![])));

        let mut broken = vec![0u32; 24];
        broken[11..16].copy_from_slice(&[1, 0, 30, 120, 0x4142_4344]);
        let got = extract_attribute(&parsed(&broken), &d, None, StorageLayout::Wide, "ROKEYS".into());
        assert!(matches!(got.status, AttributeExtractionStatus::Unsupported { .. }), "{:?}", got.status);
        assert_eq!(got.value, None);
    }

    /// Type 15 counts characters, not words, and packs four per word — the
    /// shape `BRAN.NAME` uses for 200 characters in 51 words.
    #[test]
    fn character_storage_decodes_to_text() {
        let mut words = vec![0u32; 20];
        words[11] = 5;
        words[12] = u32::from_be_bytes(*b"NUTS");
        words[13] = u32::from_be_bytes(*b"X   ");
        let mut d = descriptor(15, 8, 11, 3);
        d.compact.storage_words = 3;
        d.wide.storage_words = 3;
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "N".into());
        assert_eq!(got.value, Some(DescriptorValue::Text("NUTSX".into())));
    }

    /// Type 14 takes its element width from the descriptor itself, so the
    /// value spans `count * stride` words rather than the whole reserved slot.
    #[test]
    fn record_storage_spans_count_times_declared_stride() {
        let mut words = vec![0u32; 24];
        words[11] = 2;
        words[12..16].copy_from_slice(&[1, 2, 3, 4]);
        let mut d = descriptor(14, 4, 11, 9);
        d.compact.storage_words = 9; // (9 - 1) / 4 = 2 words per element
        d.wide.storage_words = 9;
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "R".into());
        assert_eq!(got.value, Some(DescriptorValue::RawWords(vec![1, 2, 3, 4])));
    }

    /// `logical_count == 0` states no upper bound; `sub_5AB5BC0` still reads
    /// the slot as an array.
    #[test]
    fn unbounded_logical_count_is_an_array() {
        let mut words = vec![0u32; 16];
        words[11] = 3;
        words[12..15].copy_from_slice(&[7, 8, 9]);
        let mut d = descriptor(7, 0, 11, 4);
        d.compact.storage_words = 4;
        d.wide.storage_words = 4;
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "A".into());
        assert_eq!(got.value, Some(DescriptorValue::RawWords(vec![7, 8, 9])));
    }

    #[test]
    fn raw_word_decodes_without_semantic_metadata() {
        let mut words = [0u32; 16];
        words[11] = 726_491;
        let d = descriptor(3, 1, 11, 1);
        let got = extract_attribute(
            &parsed(&words),
            &d,
            None,
            StorageLayout::Wide,
            "HCON".into(),
        );
        assert_eq!(got.status, AttributeExtractionStatus::Decoded);
        assert_eq!(
            got.value,
            Some(DescriptorValue::Word {
                raw: 726_491,
                text: db1_dehash(726_491)
            })
        );
    }

    #[test]
    fn either_unset_sentinel_is_reported_without_metadata() {
        for sentinel in UNSET_SENTINELS {
            let mut words = [0u32; 16];
            words[11] = sentinel;
            let d = descriptor(3, 1, 11, 1);
            let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "D".into());
            assert_eq!(
                got.status,
                AttributeExtractionStatus::DefaultRequired,
                "0x{sentinel:08X}"
            );
            assert_eq!(got.value, None);
        }
    }

    /// `Rlock` stores 0xFFFFFFFF and E3D prints -1 for it, so the word that
    /// used to short-circuit to "use the default" is an ordinary value.
    #[test]
    fn all_ones_is_a_value_not_an_absent_one() {
        let mut words = [0u32; 16];
        words[11] = 0xFFFF_FFFF;
        let d = descriptor(3, 1, 11, 1);
        let got = extract_attribute(&parsed(&words), &d, None, StorageLayout::Wide, "R".into());
        assert_eq!(got.status, AttributeExtractionStatus::Decoded);
        assert_eq!(
            got.value,
            Some(DescriptorValue::Word {
                raw: 0xFFFF_FFFF,
                text: word_name(0xFFFF_FFFF),
            })
        );
    }
}
