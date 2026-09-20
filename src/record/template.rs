//! Noun-template-aware attribute layout.
//!
//! Reverse-engineered from IDA `sub_5AF6AB0` / `sub_5B03900` /
//! `sub_5AF0640` and friends (see
//! `pdms-io-fork-engine-v2/docs/ida-3.1-structures.md §14`). Each
//! "noun template" is a per-noun attribute schema with variable-length
//! attribute records chained by a `stride` field; this is the
//! authoritative layout the IDA runtime uses, replacing the global
//! ATGTDF-position heuristic used by the legacy `summarize_element`
//! path.
//!
//! `TemplateProvider` loads the authoritative descriptor payload from
//! `*vir.dat`. The ATGTSX constructor remains only as a non-authoritative
//! compatibility placeholder and is rejected by physical extraction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use e3d_attlib::AttlibData;

use crate::record::template_file::{TemplateFile, TemplateFileError};

/// A single attribute slot inside a noun template.
///
/// Fields mirror the IDA layout at `template_payload + 56 + 4·i`
/// (per `docs/ida-3.1-structures.md §14.2 / §14.7.5`):
///
/// * `hash` — the attribute hash this slot decodes
/// * `stride` — dwords to advance the payload cursor to the next record;
///   the confirmed prefix is ten dwords.
/// * `storage_type_code` — runtime storage/getter family.
/// * `compact` / `wide` — representation-specific storage size, encoded
///   word location, and flags. The low 20 location bits are the physical
///   word offset; upper bits carry the packed BOOL bit index.
pub const ENCODED_OFFSET_MASK: u32 = 0x000F_FFFF;
pub const WIDE_LAYOUT_FLAG: u32 = 0x2000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLayout {
    Compact,
    Wide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedLocation {
    pub raw: u32,
}

impl EncodedLocation {
    pub fn word_offset(self) -> u32 {
        self.raw & ENCODED_OFFSET_MASK
    }

    pub fn upper_bits(self) -> u32 {
        self.raw >> 20
    }

    pub fn bool_bit_index(self) -> Option<u32> {
        let bit = self.upper_bits();
        (bit < 32).then_some(bit)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageDescriptor {
    pub storage_words: u32,
    pub location: EncodedLocation,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateAttr {
    pub hash: u32,
    pub stride: u32,
    /// Deprecated compatibility alias for [`storage_type_code`].
    pub aux: u32,
    pub storage_type_code: u32,
    pub logical_count: u32,
    pub compact: StorageDescriptor,
    pub wide: StorageDescriptor,
    /// Descriptor words after the confirmed ten-dword prefix.
    pub tail: Vec<u32>,
    /// False only for the deprecated ATGTSX structural placeholder.
    pub authoritative: bool,
}

impl TemplateAttr {
    /// Construct a single-word slot attribute (`stride = 1`, `aux = 0`).
    /// This is the conservative default that matches the ATGTDF-position
    /// path's implicit assumption.
    pub fn single_word(hash: u32) -> Self {
        Self {
            hash,
            stride: 1,
            aux: 0,
            storage_type_code: 0,
            logical_count: 0,
            compact: StorageDescriptor {
                storage_words: 0,
                location: EncodedLocation { raw: 0 },
                flags: 0,
            },
            wide: StorageDescriptor {
                storage_words: 0,
                location: EncodedLocation { raw: 0 },
                flags: 0,
            },
            tail: Vec::new(),
            authoritative: false,
        }
    }

    pub fn storage(&self, layout: StorageLayout) -> StorageDescriptor {
        match layout {
            StorageLayout::Compact => self.compact,
            StorageLayout::Wide => self.wide,
        }
    }
}

/// The eleven words an element record carries before its first attribute and
/// the noun's default record does not.
///
/// `sub_5B03B90` reads an implicit default at `default_record + 4 * offset -
/// 44`, where `offset` is the descriptor's **wide** word offset. Defaults are
/// stored once, in the wide form, whatever layout the element uses.
pub const DEFAULT_RECORD_WORD_BIAS: u32 = 11;

/// One entry of a noun's explicit default stream (`*vir.dat` chunk 2).
///
/// `sub_5B03B90` walks it as `[hash][header][payload…]`, stepping by
/// `(header & 0x3FFFFFF) + 2`; `header >> 26` is the storage type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NounDefaultEntry {
    pub hash: u32,
    pub storage_type_code: u32,
    pub words: Vec<u32>,
}

/// The values a noun falls back to when an element stores none, held in
/// chunks 1 and 2 of its `*vir.dat` child.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NounDefaults {
    /// Chunk 1, shaped like an element's implicit region minus its header.
    pub implicit: Vec<u32>,
    /// Chunk 2, in stream order.
    pub explicit: Vec<NounDefaultEntry>,
}

impl NounDefaults {
    /// Split chunk 2 into entries. A trailing partial entry ends the walk
    /// rather than being guessed at.
    pub fn parse_explicit(stream: &[u32]) -> Vec<NounDefaultEntry> {
        let mut out = Vec::new();
        let mut cursor = 0usize;
        while cursor + 2 <= stream.len() {
            let hash = stream[cursor];
            let header = stream[cursor + 1];
            let length = (header & 0x03FF_FFFF) as usize;
            if hash == 0 {
                break;
            }
            let Some(words) = stream.get(cursor + 2..cursor + 2 + length) else {
                break;
            };
            out.push(NounDefaultEntry {
                hash,
                storage_type_code: header >> 26,
                words: words.to_vec(),
            });
            cursor += length + 2;
        }
        out
    }

    pub fn explicit_for(&self, hash: u32) -> Option<&NounDefaultEntry> {
        self.explicit.iter().find(|entry| entry.hash == hash)
    }

    pub fn is_empty(&self) -> bool {
        self.implicit.is_empty() && self.explicit.is_empty()
    }
}

/// What a noun's member list is, from word 3 of its template payload header
/// (`*vir.dat` chunk 0).
///
/// `core.dll`'s merge reads this word as the member-list `kind` and treats
/// `2` alone as "the list is one attribute: overwrite it whole" (`sub_5B224B0`
/// → `sub_5B1F660`, A3b.4). A survey of the 20 shipped E3D 3.1 schema files
/// bears the reading out: the word is 0 on nouns that own nothing (`TP`,
/// `WELD`, `NBOX`, `TEXT`… — 347 of `desvir.dat`'s 994 nouns), 1 on nouns
/// whose members are their own children (`ZONE`, `PIPE`, `BRAN`… — 646), and
/// 2 only on the nouns whose members are references to elements owned
/// elsewhere: `GROU` in `desvir.dat` and `catvir.dat`, `MDB` and `USLI` in
/// `sysvir.dat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberListKind {
    /// The template did not come from a `*vir.dat` payload (or the word was
    /// not read), so nothing is known; merge treats it as [`Owned`].
    ///
    /// [`Owned`]: MemberListKind::Owned
    #[default]
    Unknown,
    /// 0: the noun has no member list.
    None,
    /// 1: an owned member list — the members are this element's children.
    Owned,
    /// 2: a reference list — the members are elements owned elsewhere, and a
    /// merge overwrites the list whole rather than member by member.
    Referenced,
    /// Any other value, kept verbatim.
    Other(u32),
}

impl MemberListKind {
    /// The kind header word 3 states.
    pub fn from_word(word: u32) -> Self {
        match word {
            0 => Self::None,
            1 => Self::Owned,
            2 => Self::Referenced,
            other => Self::Other(other),
        }
    }
}

/// Noun-correct ordered attribute schema loaded from a `*vir.dat` payload.
#[derive(Debug, Clone, Default)]
pub struct NounTemplate {
    pub noun_hash: u32,
    pub attrs: Vec<TemplateAttr>,
    pub defaults: NounDefaults,
    /// Header word 3 of the payload, typed; [`MemberListKind::Unknown`] on a
    /// template not parsed from a payload.
    pub member_list_kind: MemberListKind,
}

impl NounTemplate {
    /// Build an empty template for a noun.
    pub fn new(noun_hash: u32) -> Self {
        Self {
            noun_hash,
            attrs: Vec::new(),
            defaults: NounDefaults::default(),
            member_list_kind: MemberListKind::Unknown,
        }
    }

    /// Build a NounTemplate from the attlib's ATGTSX-derived noun map
    /// for the given `noun_hash`. Each ATGTSX entry becomes a
    /// single-word slot (`stride = 1`, `aux = 0`).
    ///
    /// This is the "structural fallback" path: when the DB-internal
    /// template (the IDA `dword_6A54024 + 60·idx + 16` payload) is not
    /// yet loaded, ATGTSX is the best available noun→attribute mapping
    /// from `attlib.dat`. Returns `None` when the attlib has no ATGTSX
    /// entries for this noun.
    ///
    /// **Limitation**: ATGTSX carries no physical locations. The result is
    /// deliberately marked non-authoritative and extraction rejects it.
    pub fn from_attlib_noun_map(attlib: &AttlibData, noun_hash: u32) -> Option<Self> {
        let attr_indices = attlib.noun_attr_map.get(&noun_hash)?;
        let attrs: Vec<TemplateAttr> = attr_indices
            .iter()
            .filter_map(|&idx| attlib.attributes.get(idx as usize))
            .map(|attr| TemplateAttr::single_word(attr.id))
            .collect();
        if attrs.is_empty() {
            return None;
        }
        Some(Self {
            noun_hash,
            attrs,
            defaults: NounDefaults::default(),
            member_list_kind: MemberListKind::Unknown,
        })
    }

    /// Total cumulative stride across all attribute records (sum of
    /// `attrs[i].stride`). For ATGTSX-synthesized templates this equals
    /// the attribute count. For DB-file-loaded templates this is the
    /// payload size in dwords from the first attr-record to the end of
    /// the last attr-record — a useful sanity check against the actual
    /// chunk size returned by [`TemplateFile::read_payload`].
    pub fn total_stride(&self) -> u32 {
        self.attrs.iter().map(|a| a.stride).sum()
    }

    pub fn is_authoritative(&self) -> bool {
        self.attrs.iter().all(|a| a.authoritative)
    }

    pub fn layout_from_format_word(format_word: u32) -> StorageLayout {
        if format_word & WIDE_LAYOUT_FLAG != 0 {
            StorageLayout::Wide
        } else {
            StorageLayout::Compact
        }
    }

    /// Strict parser for the confirmed ten-dword descriptor prefix.
    pub fn parse_template_payload(
        payload: &[u32],
        noun_hash: u32,
    ) -> Result<Self, TemplateParseError> {
        if payload.len() < 14 {
            return Err(TemplateParseError::HeaderTooShort {
                actual: payload.len(),
            });
        }
        let count = payload[9] as usize;
        let mut attrs = Vec::with_capacity(count);
        let mut cursor = 14usize;
        for index in 0..count {
            if cursor + 10 > payload.len() {
                return Err(TemplateParseError::RecordTruncated {
                    index,
                    cursor,
                    needed: 10,
                    available: payload.len().saturating_sub(cursor),
                });
            }
            let stride = payload[cursor + 1] as usize;
            if stride < 10 {
                return Err(TemplateParseError::StrideTooSmall {
                    index,
                    stride: stride as u32,
                });
            }
            if cursor + stride > payload.len() {
                return Err(TemplateParseError::RecordTruncated {
                    index,
                    cursor,
                    needed: stride,
                    available: payload.len().saturating_sub(cursor),
                });
            }
            let storage_type_code = payload[cursor + 2];
            attrs.push(TemplateAttr {
                hash: payload[cursor],
                stride: stride as u32,
                aux: storage_type_code,
                storage_type_code,
                logical_count: payload[cursor + 3],
                compact: StorageDescriptor {
                    storage_words: payload[cursor + 4],
                    location: EncodedLocation {
                        raw: payload[cursor + 5],
                    },
                    flags: payload[cursor + 6],
                },
                wide: StorageDescriptor {
                    storage_words: payload[cursor + 7],
                    location: EncodedLocation {
                        raw: payload[cursor + 8],
                    },
                    flags: payload[cursor + 9],
                },
                tail: payload[cursor + 10..cursor + stride].to_vec(),
                authoritative: true,
            });
            cursor += stride;
        }
        Ok(Self {
            noun_hash,
            attrs,
            defaults: NounDefaults::default(),
            member_list_kind: MemberListKind::from_word(payload[3]),
        })
    }

    /// Parse a noun-template payload (raw u32 words, as returned by
    /// [`TemplateFile::read_payload`]) per IDA `sub_5B03900` (GALFE)
    /// walk semantics:
    ///
    /// ```text
    /// payload[9]      = count
    /// payload[14+v9]  = attr_hash[i]   (v9 = cumulative dword cursor)
    /// payload[15+v9]  = stride[i]
    /// payload[16+v9]  = storage_type_code[i]
    /// payload[17..23] = logical count plus compact/wide storage descriptors
    /// v9 += stride[i]
    /// ```
    ///
    /// Returns `None` when:
    /// * the payload is too short to carry the header (< 14 dwords), or
    /// * a record has stride below the confirmed ten-dword prefix, or
    /// * a record walk goes out of bounds (corrupt payload or wrong
    ///   chunk passed in)
    ///
    /// `noun_hash` is recorded verbatim on the result; the parser does
    /// not cross-check it against any field in the payload itself
    /// (payload byte 1/2 do typically echo the noun hash, but that's
    /// not load-bearing here — the caller already keyed the binary
    /// search by `noun_hash`).
    pub fn from_template_payload(payload: &[u32], noun_hash: u32) -> Option<Self> {
        Self::parse_template_payload(payload, noun_hash).ok()
    }

    /// End-to-end loader: open the template `.dat` file at `path`,
    /// validate its declared `template_type` (when `expected_type` is
    /// `Some`), look up `noun_hash` via binary search, and parse the
    /// resulting payload into a `NounTemplate`.
    ///
    /// This is the **authoritative** noun template path (per IDA
    /// `sub_5AFA740` / `sub_5AF6AB0`), in contrast to the
    /// [`from_attlib_noun_map`] structural fallback which only knows
    /// about the ATGTSX-covered nouns and uses `stride = 1`.
    ///
    /// [`from_attlib_noun_map`]: NounTemplate::from_attlib_noun_map
    pub fn load_from_template_file(
        path: impl AsRef<Path>,
        expected_type: Option<u32>,
        noun_hash: u32,
    ) -> Result<Self, TemplateLoadError> {
        let mut tf = TemplateFile::open(path)?;
        let header = tf.read_header()?;
        if let Some(expected) = expected_type {
            if header.template_type != expected {
                return Err(TemplateFileError::TypeMismatch {
                    path: tf.path().to_path_buf(),
                    expected,
                    actual: header.template_type,
                }
                .into());
            }
        }
        let children = tf.read_children(&header)?;
        let child = *TemplateFile::find_child(&children, noun_hash)
            .ok_or(TemplateFileError::NounNotFound(noun_hash))?;
        Self::load_child(&mut tf, &child, noun_hash)
    }

    /// Parse a child's descriptors and pick up the two default chunks beside
    /// them, the way `sub_5AF6AB0` mode 1 does.
    pub fn load_child(
        file: &mut TemplateFile,
        child: &crate::record::template_file::TemplateChild,
        noun_hash: u32,
    ) -> Result<Self, TemplateLoadError> {
        use crate::record::template_file::TemplateChild;
        let payload = file.read_chunk(child, TemplateChild::DESCRIPTORS)?;
        let mut template = Self::parse_template_payload(&payload, noun_hash)?;
        template.defaults = NounDefaults {
            implicit: file.read_chunk(child, TemplateChild::IMPLICIT_DEFAULTS)?,
            explicit: NounDefaults::parse_explicit(
                &file.read_chunk(child, TemplateChild::EXPLICIT_DEFAULTS)?,
            ),
        };
        Ok(template)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateParseError {
    #[error("template payload header requires 14 dwords, got {actual}")]
    HeaderTooShort { actual: usize },
    #[error("descriptor {index} stride must be at least 10 dwords, got {stride}")]
    StrideTooSmall { index: usize, stride: u32 },
    #[error(
        "descriptor {index} at dword {cursor} is truncated: need {needed} dwords, have {available}"
    )]
    RecordTruncated {
        index: usize,
        cursor: usize,
        needed: usize,
        available: usize,
    },
}

/// Errors returned by [`NounTemplate::load_from_template_file`].
#[derive(Debug, thiserror::Error)]
pub enum TemplateLoadError {
    #[error(transparent)]
    File(#[from] TemplateFileError),
    #[error(transparent)]
    Parse(#[from] TemplateParseError),
}

/// Cached reader for noun template files such as `desvir.dat`.
pub struct TemplateProvider {
    path: PathBuf,
    expected_type: Option<u32>,
    children: Vec<crate::record::template_file::TemplateChild>,
    cache: HashMap<u32, Option<NounTemplate>>,
}

impl TemplateProvider {
    pub fn open(
        path: impl AsRef<Path>,
        expected_type: Option<u32>,
    ) -> Result<Self, TemplateLoadError> {
        let path = path.as_ref().to_path_buf();
        let mut file = TemplateFile::open(&path)?;
        let header = file.read_header()?;
        if let Some(expected) = expected_type {
            if header.template_type != expected {
                return Err(TemplateFileError::TypeMismatch {
                    path,
                    expected,
                    actual: header.template_type,
                }
                .into());
            }
        }
        let children = file.read_children(&header)?;

        Ok(Self {
            path,
            expected_type,
            children,
            cache: HashMap::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn expected_type(&self) -> Option<u32> {
        self.expected_type
    }

    pub fn template_for(
        &mut self,
        noun_hash: u32,
    ) -> Result<Option<&NounTemplate>, TemplateLoadError> {
        if !self.cache.contains_key(&noun_hash) {
            let template = self.load_template(noun_hash)?;
            self.cache.insert(noun_hash, template);
        }
        Ok(self.cache.get(&noun_hash).and_then(Option::as_ref))
    }

    pub fn cached_count(&self) -> usize {
        self.cache.values().filter(|t| t.is_some()).count()
    }

    fn load_template(&self, noun_hash: u32) -> Result<Option<NounTemplate>, TemplateLoadError> {
        let Some(child) = TemplateFile::find_child(&self.children, noun_hash) else {
            return Ok(None);
        };
        let mut file = TemplateFile::open(&self.path)?;
        Ok(Some(NounTemplate::load_child(&mut file, child, noun_hash)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::template_file::PAGE_BYTES;
    use std::io::Write;

    #[test]
    fn template_attr_single_word_defaults() {
        let t = TemplateAttr::single_word(0xDEADBEEF);
        assert_eq!(t.hash, 0xDEADBEEF);
        assert_eq!(t.stride, 1);
        assert_eq!(t.aux, 0);
    }

    #[test]
    fn noun_template_total_stride_sums() {
        let mut t = NounTemplate::new(0x111);
        let mut a = TemplateAttr::single_word(0x10);
        a.stride = 1;
        t.attrs.push(a);
        let mut a = TemplateAttr::single_word(0x20);
        a.stride = 2;
        t.attrs.push(a);
        let mut a = TemplateAttr::single_word(0x30);
        a.stride = 1;
        t.attrs.push(a);
        assert_eq!(t.total_stride(), 4);
    }

    #[test]
    fn from_attlib_noun_map_returns_none_when_no_entries() {
        let attlib = AttlibData::default();
        assert!(NounTemplate::from_attlib_noun_map(&attlib, 0x123).is_none());
    }

    /// Header word 3 is the member-list kind; a template not parsed from a
    /// payload does not know it.
    #[test]
    fn member_list_kind_comes_from_header_word_3() {
        let mut payload = vec![0u32; 24];
        payload[9] = 1;
        payload[14..24].copy_from_slice(&[0xAA, 10, 2, 1, 1, 11, 0, 2, 11, 0]);
        for (word, kind) in [
            (0, MemberListKind::None),
            (1, MemberListKind::Owned),
            (2, MemberListKind::Referenced),
            (7, MemberListKind::Other(7)),
        ] {
            payload[3] = word;
            let t = NounTemplate::from_template_payload(&payload, 0x123).unwrap();
            assert_eq!(t.member_list_kind, kind, "word 3 = {word}");
        }
        assert_eq!(NounTemplate::new(0x123).member_list_kind, MemberListKind::Unknown);
        assert_eq!(NounTemplate::default().member_list_kind, MemberListKind::Unknown);
    }

    #[test]
    fn from_template_payload_parses_synthetic_payload() {
        let mut payload = vec![0u32; 45];
        payload[9] = 3;
        payload[14..24].copy_from_slice(&[0xAA, 10, 2, 1, 1, 11, 0, 2, 11, 0]);
        payload[24..35].copy_from_slice(&[
            0xBB,
            11,
            5,
            1,
            1,
            0x0030_000C,
            1,
            1,
            0x0030_000D,
            2,
            99,
        ]);
        payload[35..45].copy_from_slice(&[0xCC, 10, 6, 3, 4, 14, 0, 7, 20, 0]);

        let t = NounTemplate::from_template_payload(&payload, 0x123).unwrap();
        assert_eq!(t.noun_hash, 0x123);
        assert_eq!(t.attrs.len(), 3);
        assert_eq!(
            t.attrs[0],
            TemplateAttr {
                hash: 0xAA,
                stride: 10,
                aux: 2,
                storage_type_code: 2,
                logical_count: 1,
                compact: StorageDescriptor {
                    storage_words: 1,
                    location: EncodedLocation { raw: 11 },
                    flags: 0
                },
                wide: StorageDescriptor {
                    storage_words: 2,
                    location: EncodedLocation { raw: 11 },
                    flags: 0
                },
                tail: vec![],
                authoritative: true,
            }
        );
        assert_eq!(
            t.attrs[1],
            TemplateAttr {
                hash: 0xBB,
                stride: 11,
                aux: 5,
                storage_type_code: 5,
                logical_count: 1,
                compact: StorageDescriptor {
                    storage_words: 1,
                    location: EncodedLocation { raw: 0x0030_000C },
                    flags: 1
                },
                wide: StorageDescriptor {
                    storage_words: 1,
                    location: EncodedLocation { raw: 0x0030_000D },
                    flags: 2
                },
                tail: vec![99],
                authoritative: true,
            }
        );
        assert_eq!(
            t.attrs[2],
            TemplateAttr {
                hash: 0xCC,
                stride: 10,
                aux: 6,
                storage_type_code: 6,
                logical_count: 3,
                compact: StorageDescriptor {
                    storage_words: 4,
                    location: EncodedLocation { raw: 14 },
                    flags: 0
                },
                wide: StorageDescriptor {
                    storage_words: 7,
                    location: EncodedLocation { raw: 20 },
                    flags: 0
                },
                tail: vec![],
                authoritative: true,
            }
        );
    }

    #[test]
    fn from_template_payload_rejects_too_short() {
        assert!(NounTemplate::from_template_payload(&[1u32, 2, 3], 0x123).is_none());
        // Header is present, but count=1 cannot fit the confirmed ten-word record.
        let mut short = vec![0u32; 16];
        short[9] = 1;
        assert!(NounTemplate::from_template_payload(&short, 0x123).is_none());
    }

    #[test]
    fn from_template_payload_preserves_zero_count_pseudo_template() {
        let mut payload = vec![0u32; 30];
        payload[9] = 0;
        let parsed = NounTemplate::parse_template_payload(&payload, 0x123).unwrap();
        assert!(parsed.attrs.is_empty());
    }

    #[test]
    fn from_template_payload_rejects_truncated_walk() {
        let mut payload = vec![0u32; 30];
        payload[9] = 2;
        payload[14] = 0xAA;
        payload[15] = 10;
        payload[24] = 0xBB;
        payload[25] = 10;
        assert!(matches!(
            NounTemplate::parse_template_payload(&payload, 0x123),
            Err(TemplateParseError::RecordTruncated { index: 1, .. })
        ));
    }

    #[test]
    fn from_template_payload_rejects_small_stride() {
        let mut payload = vec![0u32; 30];
        payload[9] = 1;
        payload[14] = 0xAA;
        payload[15] = 9;
        assert!(matches!(
            NounTemplate::parse_template_payload(&payload, 0x123),
            Err(TemplateParseError::StrideTooSmall {
                index: 0,
                stride: 9
            })
        ));
    }

    fn synth_template_file(pages: &[Vec<u32>]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for words in pages {
            let mut bytes = vec![0u8; PAGE_BYTES];
            for (i, word) in words.iter().enumerate() {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
            }
            file.write_all(&bytes).unwrap();
        }
        file.flush().unwrap();
        file
    }

    #[test]
    fn template_provider_caches_hit_and_miss() {
        let page0 = vec![6u32, 0, 0x1234, 0, 0, 1, 0, 2];
        let page1 = vec![0xAAu32, 3, 34, 0, 0, 0, 0];
        let mut payload = vec![0u32; 34];
        payload[9] = 2;
        payload[14..24].copy_from_slice(&[0x10, 10, 2, 1, 1, 11, 0, 2, 11, 0]);
        payload[24..34].copy_from_slice(&[0x20, 10, 5, 1, 1, 12, 0, 1, 12, 0]);
        let file = synth_template_file(&[page0, page1, payload]);

        let mut provider = TemplateProvider::open(file.path(), Some(0x1234)).unwrap();
        assert_eq!(provider.cached_count(), 0);

        let first = provider.template_for(0xAA).unwrap().unwrap();
        assert_eq!(first.attrs.len(), 2);
        assert_eq!(provider.cached_count(), 1);

        assert!(provider.template_for(0xBB).unwrap().is_none());
        assert_eq!(provider.cached_count(), 1);
        assert_eq!(
            provider.template_for(0xAA).unwrap().unwrap().attrs[1].hash,
            0x20
        );
    }

    #[test]
    fn from_attlib_noun_map_collects_attr_hashes_in_index_order() {
        use e3d_attlib::parser::{AttrSource, AttributeRecord};
        let mut attlib = AttlibData::default();
        attlib.attributes.push(AttributeRecord {
            id: 0xAA,
            name: "AA".into(),
            type_code: 0,
            source: AttrSource::System,
        });
        attlib.attributes.push(AttributeRecord {
            id: 0xBB,
            name: "BB".into(),
            type_code: 0,
            source: AttrSource::System,
        });
        attlib.attributes.push(AttributeRecord {
            id: 0xCC,
            name: "CC".into(),
            type_code: 0,
            source: AttrSource::System,
        });
        attlib.noun_attr_map.insert(0x999, vec![0, 2, 1]);
        let t = NounTemplate::from_attlib_noun_map(&attlib, 0x999).unwrap();
        assert_eq!(t.noun_hash, 0x999);
        let hashes: Vec<u32> = t.attrs.iter().map(|a| a.hash).collect();
        assert_eq!(hashes, vec![0xAA, 0xCC, 0xBB]);
        // All single-word, no aux.
        for a in &t.attrs {
            assert_eq!(a.stride, 1);
            assert_eq!(a.aux, 0);
        }
    }
}
