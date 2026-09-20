use crate::record::explicit::{parse_tail_as, ExplicitAttribute};
use crate::record::text::TextFormat;
use crate::refno::RefNo;

const HEADER_SIZE: usize = 24;

/// Parsed element record header.
///
/// Layout from pdmsdb_engine_v2/db4/page_layout.rs:
/// - [prefix padding: 0x00000000 or 0x00000007 words]
/// - impl_len_words (i32 BE): length of implicit region in 4-byte words
/// - refno (2 x u32 BE): element RefNo
/// - noun_hash (u32 BE): element type hash/identifier
/// - owner (2 x u32 BE): owner element RefNo
/// - [implicit data bytes]
/// - [member data]
/// - [explicit attribute data]
#[derive(Debug, Clone)]
pub struct ParsedElement {
    pub impl_len_words: i32,
    pub refno: RefNo,
    pub noun_hash: u32,
    pub owner: RefNo,
    pub prefix_offset: usize,
    pub implicit_data: Vec<u8>,
    pub members: Vec<RefNo>,
    pub explicit_attributes: Vec<ExplicitAttribute>,
    pub record_end: usize,
    pub total_size: usize,
    /// How this record's text was decoded — the database header's `FORMAT`
    /// word when the caller had it ([`ParsedElement::parse_as`]), else the
    /// guess ([`TextFormat::Detect`]). The descriptor decoder reads implicit
    /// text under the same format later, so a record carries its own answer.
    pub text_format: TextFormat,
}

impl ParsedElement {
    /// Approximate heap footprint of this record in bytes: the implicit bytes,
    /// the member list and every explicit attribute with the payload behind
    /// its value (text, arrays). Lengths, not capacities — a size estimate
    /// for cache accounting (`DbSet::element_cache_stats`), not an allocator
    /// audit; the inline part is `size_of::<ParsedElement>()` on the caller's
    /// side.
    pub fn approx_heap_bytes(&self) -> usize {
        use crate::record::explicit::ExplicitValue;
        let attributes: usize = self
            .explicit_attributes
            .iter()
            .map(|attribute| {
                std::mem::size_of::<ExplicitAttribute>()
                    + match &attribute.value {
                        ExplicitValue::Text(text) => text.len(),
                        ExplicitValue::IntArray(values) => values.len() * 4,
                        ExplicitValue::RealArray(values) => values.len() * 8,
                        ExplicitValue::RefNoArray(values) => values.len() * 8,
                        ExplicitValue::RawWords(values) => values.len() * 4,
                        ExplicitValue::Integer(_)
                        | ExplicitValue::Real(_)
                        | ExplicitValue::Bool(_)
                        | ExplicitValue::RefNo(_) => 0,
                    }
            })
            .sum();
        self.implicit_data.len() + self.members.len() * 8 + attributes
    }

    /// [`ParsedElement::parse_as`] without the header's `FORMAT` word: text
    /// is guessed at from its bytes. Prefer `parse_as` with
    /// [`crate::ReadOnlyEngine::text_format`] when an engine is at hand.
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        Self::parse_as(raw, TextFormat::Detect)
    }

    /// Parse one gathered record, decoding its text the way the database's
    /// header says it is stored.
    pub fn parse_as(raw: &[u8], text_format: TextFormat) -> Result<Self, String> {
        let prefix = skip_padding(raw);
        if prefix + HEADER_SIZE > raw.len() {
            return Err(format!(
                "record too short after padding: {} bytes at prefix {}",
                raw.len(),
                prefix
            ));
        }

        let r = |off: usize| u32::from_be_bytes(raw[off..off + 4].try_into().unwrap());

        let impl_len_words = r(prefix) as i32;
        if impl_len_words <= 0 {
            return Err(format!("invalid impl_len_words: {}", impl_len_words));
        }

        let refno = RefNo::new(r(prefix + 4), r(prefix + 8));
        let noun_hash = r(prefix + 12);
        let owner = RefNo::new(r(prefix + 16), r(prefix + 20));

        let declared_impl_len = impl_len_words as usize * 4;
        let impl_end = (prefix + declared_impl_len).min(raw.len());
        let implicit_data = raw[prefix..impl_end].to_vec();
        let tail = parse_tail_as(raw, impl_end, refno, text_format);
        if tail.continues_past_end {
            return Err(format!(
                "record for {refno} ends at {} with a block that says it continues elsewhere: \
                 these bytes are part of the record, not all of it",
                tail.record_end
            ));
        }

        Ok(Self {
            impl_len_words,
            refno,
            noun_hash,
            owner,
            prefix_offset: prefix,
            implicit_data,
            members: tail.members,
            explicit_attributes: tail.attributes,
            record_end: tail.record_end,
            total_size: raw.len(),
            text_format,
        })
    }
}

fn skip_padding(input: &[u8]) -> usize {
    let mut pos = 0;
    while pos + 4 <= input.len() {
        let w = &input[pos..pos + 4];
        if w == [0, 0, 0, 0] || w == [0, 0, 0, 7] {
            pos += 4;
        } else {
            break;
        }
    }
    pos
}
