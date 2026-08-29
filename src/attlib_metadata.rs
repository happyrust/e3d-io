//! Human-facing attribute names recovered directly from `attlib.dat`.
//!
//! ATGTIX stores a `(hash, packed page/word location)` pair. The location is
//! one-based on disk and points 62 words before the attribute metadata record.
//! The record contains the short command name followed by descriptive strings;
//! the first following token without whitespace is the TTY query/report name.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use e3d_attlib::{db1_dehash, lookup_system_name, AttlibData};

const WORDS_PER_PAGE: usize = 512;
const RECORD_DELTA_WORDS: usize = 62;
const RECORD_SCAN_WORDS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeMetadata {
    pub hash: u32,
    pub short_name: String,
    pub report_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct AttributeMetadataCatalog {
    by_hash: HashMap<u32, AttributeMetadata>,
}

impl AttributeMetadataCatalog {
    pub fn parse_file(path: impl AsRef<Path>, attlib: &AttlibData) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut by_hash = HashMap::new();
        for entry in &attlib.atgtix_entries {
            if entry.page_no == 0 || entry.hash > 0x171F_AD39 || by_hash.contains_key(&entry.hash) {
                continue;
            }
            if let Some(metadata) = parse_metadata_record(
                &bytes,
                entry.hash,
                entry.page_no as usize,
                entry.word_offset as usize,
            ) {
                by_hash.insert(entry.hash, metadata);
            }
        }
        Ok(Self { by_hash })
    }

    pub fn get(&self, hash: u32) -> Option<&AttributeMetadata> {
        self.by_hash.get(&hash)
    }

    pub fn report_name(&self, hash: u32) -> Option<&str> {
        self.get(hash).map(|item| item.report_name.as_str())
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

fn parse_metadata_record(
    bytes: &[u8],
    hash: u32,
    one_based_page: usize,
    word_offset: usize,
) -> Option<AttributeMetadata> {
    let start_word = (one_based_page - 1)
        .checked_mul(WORDS_PER_PAGE)?
        .checked_add(word_offset)?
        .checked_add(RECORD_DELTA_WORDS)?;
    let total_words = bytes.len() / 4;
    if start_word >= total_words {
        return None;
    }
    let end_word = total_words.min(start_word + RECORD_SCAN_WORDS);
    let words: Vec<u32> = (start_word..end_word)
        .map(|word| {
            let offset = word * 4;
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
        })
        .collect();

    let strings = length_prefixed_strings(&words);
    let short_name = lookup_system_name(hash)
        .map(String::from)
        .or_else(|| db1_dehash(hash))?;
    let category_index = strings
        .iter()
        .find(|(_, value)| is_category(value))
        .map(|(index, _)| *index)
        .unwrap_or(usize::MAX);
    let report_name = strings
        .iter()
        .filter(|(_, value)| is_query_token(value) && !is_category(value))
        .max_by_key(|(index, value)| {
            (
                value.chars().any(char::is_lowercase),
                *index < category_index,
                common_prefix_len(&short_name, value),
                *index,
            )
        })
        .filter(|(_, value)| common_prefix_len(&short_name, value) >= 1)
        .map(|(_, value)| value.clone())?;
    Some(AttributeMetadata {
        hash,
        short_name,
        report_name,
    })
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.bytes()
        .map(|value| value.to_ascii_lowercase())
        .zip(right.bytes().map(|value| value.to_ascii_lowercase()))
        .take_while(|(left, right)| left == right)
        .count()
}

fn length_prefixed_strings(words: &[u32]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for index in 0..words.len() {
        let len = words[index] as usize;
        if len == 0 || len > 128 || index + 1 + len > words.len() {
            continue;
        }
        let chars = &words[index + 1..index + 1 + len];
        if chars.iter().all(|value| matches!(*value, 0x20..=0x7E)) {
            out.push((
                index,
                chars.iter().map(|value| *value as u8 as char).collect(),
            ));
        }
    }
    out
}

fn is_query_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.chars().any(char::is_whitespace)
        && !matches!(
            value,
            "Ignore" | "Internal" | "General" | "Design" | "Geometry"
        )
}

fn is_category(value: &str) -> bool {
    matches!(
        value,
        "Ignore"
            | "Internal"
            | "General"
            | "Design"
            | "Geometry"
            | "Focus"
            | "Router"
            | "Aveva Only"
            | "Spooler"
            | "Purchasing"
            | "Specification"
            | "Manufacturing"
            | "Template"
            | "Operating Parameters"
    ) || value.starts_with("Catalog values")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_tokens_reject_descriptions_and_categories() {
        assert!(is_query_token("Hposition"));
        assert!(is_query_token("HeadWidth"));
        assert!(!is_query_token("Head position"));
        assert!(!is_query_token("Ignore"));
    }
}
