//! On-disk reader for noun-template `.dat` files (e.g. `desvir.dat`).
//!
//! Reverse-engineered from IDA `sub_5AFA740` (`2OTDB:Name is %s,Templ
//! type is %s,...`), `sub_5AF6AB0` (template lookup with binary search +
//! chunked load), and `sub_5AF0640` (raw FHDBRN chunk reader). See
//! `pdms-io-fork-engine-v2/docs/ida-3.1-structures.md §14.7` for the
//! full evidence trail.
//!
//! ## File format (Fortran `"DB,BL 512"` — 2048 byte pages, big-endian u32)
//!
//! **Page 0** (8-dword header read via `sub_5AF0640(handle, /*token=*/1, &buf, 8)`):
//!
//! | Offset (dword) | Meaning |
//! |---|---|
//! | 0 | magic = 6 (`if (v30 != 6) error 517`) |
//! | 1 | unused |
//! | 2 | template_type — caller must match (`if (v31 != a1) error 548`) |
//! | 3 | aux1 (descriptor[+4]) — purpose TBD |
//! | 4 | unused |
//! | 5 | child_count |
//! | 6 | unused |
//! | 7 | chunk1_token — Fortran 1-indexed page no of the first child-array chunk |
//!
//! **Child array** (28 bytes / 7 dwords each, sorted by hash for binary search):
//!
//! ```text
//! { hash, tok1, sz1, tok2, sz2, tok3, sz3 }
//! ```
//!
//! Loaded by chunked-read: each physical chunk holds 511 data dwords + 1 chain pointer
//! at position 511 (the Fortran token of the NEXT chunk). When `remaining > 511`,
//! read 512 dwords and advance the cursor by 511 (overwriting the chain pointer
//! on the next iteration); when `remaining <= 511`, read exactly that many.
//!
//! **Per-noun payload** (per §14.2 / §14.7.5):
//!
//! ```text
//! payload[9]     = count (slots in this noun template)
//! payload[14+v9] = attr_hash[i]   // v9 is cumulative dword cursor
//! payload[15+v9] = stride[i]      // payload-internal stride to next slot
//! payload[16+v9] = aux[i]
//! v9 += stride[i]
//! ```
//!
//! ## Pragmatic scope
//!
//! This module focuses on **reading**: open a `.dat` file, parse the header,
//! materialize the child array, look up by hash, and load the per-noun chunk.
//! Parsing the payload into a `NounTemplate` lives in [`super::template`].
//!
//! The implementation does NOT cache; each `read_payload` call re-reads the
//! file. Callers that need repeated access should keep the parsed `Vec<u32>`
//! around.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Fortran "DB,BL 512" page size: 512 dwords × 4 bytes = 2048 bytes.
pub const PAGE_BYTES: usize = 2048;
/// Number of u32 words per page.
pub const PAGE_WORDS: usize = PAGE_BYTES / 4;
/// Data dwords per chunk before the chain pointer; chunks larger than this
/// use the 512th dword as the next-chunk Fortran token.
pub const CHUNK_DATA_WORDS: usize = 511;
/// Magic byte at header word 0 — `sub_5AFA740` errors with 517 on mismatch.
pub const HEADER_MAGIC: u32 = 6;

/// Errors returned by the template-file reader.
#[derive(Debug, thiserror::Error)]
pub enum TemplateFileError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid header magic in {path}: expected 6, got {actual}")]
    BadMagic { path: PathBuf, actual: u32 },

    #[error(
        "template_type mismatch in {path}: expected 0x{expected:08X}, file declares 0x{actual:08X}"
    )]
    TypeMismatch {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },

    #[error("child count {0} exceeds sanity cap (1_000_000)")]
    ChildCountTooLarge(u32),

    #[error("token {token} out of file range (file has {pages} pages)")]
    TokenOutOfRange { token: u32, pages: u32 },

    #[error("noun hash 0x{0:08X} not present in template file")]
    NounNotFound(u32),
}

/// Parsed file header (8 dwords from page 0 / Fortran token=1).
#[derive(Debug, Clone, Copy)]
pub struct TemplateHeader {
    pub magic: u32,
    pub template_type: u32,
    pub aux1: u32,
    pub child_count: u32,
    pub chunk1_token: u32,
}

/// One entry in the child array (28 bytes per IDA layout).
///
/// Each chunk is `(token, size_in_dwords)`. The three are not
/// interchangeable, and `sub_5AF6AB0` picks between them by its `mode`
/// argument:
///
/// | mode | chunk | what it holds |
/// |---|---|---|
/// | 0, 2 | 0 | the attribute descriptors |
/// | 1 | 1 | the noun's implicit default record |
/// | 1 | 2 | the noun's explicit default stream |
///
/// `sub_5B03B90` (GCEAD) asks for mode 1 and reads attribute defaults out of
/// chunks 1 and 2. They are populated for most nouns: across the 20 shipped
/// schema files 715 of 1113 `desvir.dat` children carry chunk 1 and 426 carry
/// chunk 2, together covering 10892 of the 35734 descriptors.
#[derive(Debug, Clone, Copy)]
pub struct TemplateChild {
    pub hash: u32,
    pub chunks: [(u32, u32); 3],
}

impl TemplateChild {
    /// Chunk index of the attribute descriptors (`sub_5AF6AB0` mode 0 and 2).
    pub const DESCRIPTORS: usize = 0;
    /// Chunk index of the implicit default record (mode 1, first output).
    pub const IMPLICIT_DEFAULTS: usize = 1;
    /// Chunk index of the explicit default stream (mode 1, third output).
    pub const EXPLICIT_DEFAULTS: usize = 2;

    /// True when chunk `i` has any data to read.
    pub fn chunk_present(&self, i: usize) -> bool {
        self.chunks.get(i).map_or(false, |(_, sz)| *sz > 0)
    }

    /// Total data dwords across all three chunks.
    pub fn total_dwords(&self) -> u32 {
        self.chunks.iter().map(|(_, sz)| *sz).sum()
    }
}

/// Reader for a template `.dat` file. Holds an open `File` handle.
pub struct TemplateFile {
    path: PathBuf,
    file: File,
    page_count: u32,
}

impl TemplateFile {
    /// Open a template `.dat` file for reading.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TemplateFileError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|e| TemplateFileError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let len = file
            .metadata()
            .map_err(|e| TemplateFileError::Io {
                path: path.to_path_buf(),
                source: e,
            })?
            .len();
        let page_count = (len / PAGE_BYTES as u64) as u32;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            page_count,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// Read page `page_no` (0-indexed) as 512 big-endian u32 words.
    fn read_page_words(&mut self, page_no: u32) -> Result<[u32; PAGE_WORDS], TemplateFileError> {
        if page_no >= self.page_count {
            return Err(TemplateFileError::TokenOutOfRange {
                token: page_no + 1,
                pages: self.page_count,
            });
        }
        let offset = (page_no as u64) * (PAGE_BYTES as u64);
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| TemplateFileError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        let mut buf = [0u8; PAGE_BYTES];
        self.file
            .read_exact(&mut buf)
            .map_err(|e| TemplateFileError::Io {
                path: self.path.clone(),
                source: e,
            })?;
        let mut words = [0u32; PAGE_WORDS];
        for (i, chunk) in buf.chunks_exact(4).enumerate() {
            words[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Ok(words)
    }

    /// Chunked read of `total_dwords` consecutive dwords starting at Fortran token
    /// `initial_token`. Mirrors IDA `sub_5AFA740` / `sub_5AF6AB0` chunked-read:
    /// each physical chunk holds up to 511 data dwords + 1 chain pointer at
    /// position 511 (the Fortran token of the next chunk). When `remaining <= 511`,
    /// the final chunk reads only the remaining dwords (no chain pointer needed).
    pub fn read_chunked(
        &mut self,
        initial_token: u32,
        total_dwords: u32,
    ) -> Result<Vec<u32>, TemplateFileError> {
        let mut out = Vec::with_capacity(total_dwords as usize);
        let mut cur_token = initial_token;
        let mut remaining = total_dwords as usize;
        while remaining > 0 {
            if cur_token == 0 {
                // Defensive: an unexpected null token would otherwise produce
                // a page-0 read that's almost certainly not the right data.
                return Err(TemplateFileError::TokenOutOfRange {
                    token: 0,
                    pages: self.page_count,
                });
            }
            let page_no = cur_token.saturating_sub(1);
            let page = self.read_page_words(page_no)?;
            if remaining > CHUNK_DATA_WORDS {
                out.extend_from_slice(&page[..CHUNK_DATA_WORDS]);
                cur_token = page[CHUNK_DATA_WORDS];
                remaining -= CHUNK_DATA_WORDS;
            } else {
                out.extend_from_slice(&page[..remaining]);
                remaining = 0;
            }
        }
        Ok(out)
    }

    /// Read the 8-dword file header at Fortran token=1 (= page 0).
    pub fn read_header(&mut self) -> Result<TemplateHeader, TemplateFileError> {
        let page0 = self.read_page_words(0)?;
        let header = TemplateHeader {
            magic: page0[0],
            template_type: page0[2],
            aux1: page0[3],
            child_count: page0[5],
            chunk1_token: page0[7],
        };
        if header.magic != HEADER_MAGIC {
            return Err(TemplateFileError::BadMagic {
                path: self.path.clone(),
                actual: header.magic,
            });
        }
        if header.child_count > 1_000_000 {
            return Err(TemplateFileError::ChildCountTooLarge(header.child_count));
        }
        Ok(header)
    }

    /// Read and parse the entire child array (`7 * header.child_count` dwords).
    /// Children come back sorted by hash (the on-disk invariant `sub_5AF6AB0`
    /// relies on for its binary search).
    pub fn read_children(
        &mut self,
        header: &TemplateHeader,
    ) -> Result<Vec<TemplateChild>, TemplateFileError> {
        let total_dwords = header.child_count.saturating_mul(7);
        let words = self.read_chunked(header.chunk1_token, total_dwords)?;
        let mut children = Vec::with_capacity(header.child_count as usize);
        for c in words.chunks_exact(7) {
            children.push(TemplateChild {
                hash: c[0],
                chunks: [(c[1], c[2]), (c[3], c[4]), (c[5], c[6])],
            });
        }
        Ok(children)
    }

    /// Find a child by hash using binary search. Returns `None` when not present.
    pub fn find_child<'a>(children: &'a [TemplateChild], hash: u32) -> Option<&'a TemplateChild> {
        children
            .binary_search_by(|c| c.hash.cmp(&hash))
            .ok()
            .map(|i| &children[i])
    }

    /// Read the descriptor payload (chunk 0). The other two chunks hold the
    /// noun's defaults and are read through [`read_chunk`].
    ///
    /// [`read_chunk`]: TemplateFile::read_chunk
    pub fn read_payload(&mut self, child: &TemplateChild) -> Result<Vec<u32>, TemplateFileError> {
        self.read_chunk(child, TemplateChild::DESCRIPTORS)
    }

    /// Read one of a child's three chunks; empty when that chunk is absent.
    pub fn read_chunk(
        &mut self,
        child: &TemplateChild,
        index: usize,
    ) -> Result<Vec<u32>, TemplateFileError> {
        let Some(&(token, size)) = child.chunks.get(index) else {
            return Ok(Vec::new());
        };
        if size == 0 {
            return Ok(Vec::new());
        }
        self.read_chunked(token, size)
    }

    /// Read all three chunks of a child concatenated. Useful when the caller
    /// expects multi-chunk payloads; most nouns have only chunk 0 populated.
    pub fn read_all_payload(
        &mut self,
        child: &TemplateChild,
    ) -> Result<Vec<u32>, TemplateFileError> {
        let mut out = Vec::with_capacity(child.total_dwords() as usize);
        for (token, size) in child.chunks {
            if size == 0 {
                continue;
            }
            let chunk = self.read_chunked(token, size)?;
            out.extend(chunk);
        }
        Ok(out)
    }

    /// Load a noun's template payload in one call: parse header, parse children,
    /// binary-search for `noun_hash`, return the chunk-0 payload. Optionally
    /// validates that the file's `template_type` matches `expected_type`.
    pub fn load_noun_payload(
        &mut self,
        expected_type: Option<u32>,
        noun_hash: u32,
    ) -> Result<Vec<u32>, TemplateFileError> {
        let header = self.read_header()?;
        if let Some(t) = expected_type {
            if header.template_type != t {
                return Err(TemplateFileError::TypeMismatch {
                    path: self.path.clone(),
                    expected: t,
                    actual: header.template_type,
                });
            }
        }
        let children = self.read_children(&header)?;
        let child = Self::find_child(&children, noun_hash)
            .ok_or(TemplateFileError::NounNotFound(noun_hash))?;
        self.read_payload(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic "DB,BL 512" page filled with the supplied u32 words
    /// (big-endian), padded to 512 dwords with `0xFFFFFFFF`.
    fn synth_page(words: &[u32]) -> Vec<u8> {
        let mut page = vec![0u8; PAGE_BYTES];
        for (i, w) in words.iter().enumerate().take(PAGE_WORDS) {
            let bytes = w.to_be_bytes();
            page[i * 4..i * 4 + 4].copy_from_slice(&bytes);
        }
        // Padding: 0xFF in unused bytes (clearly distinct from valid data).
        for i in (words.len() * 4)..PAGE_BYTES {
            page[i] = 0xFF;
        }
        page
    }

    /// Write `pages` to a temporary file and return the path.
    fn synth_file(pages: &[&[u32]]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        use std::io::Write;
        for words in pages {
            let bytes = synth_page(words);
            f.write_all(&bytes).unwrap();
        }
        f.flush().unwrap();
        f
    }

    #[test]
    fn header_parsing_happy_path() {
        // Page 0: magic=6, template_type=0x1234, aux1=7, child_count=2, chunk1_token=2
        // Words: [6, ?, 0x1234, 7, ?, 2, ?, 2]
        let page0 = vec![6u32, 0, 0x1234, 7, 0, 2, 0, 2];
        // Page 1: child 0 + child 1 = 14 dwords (no chain pointer needed)
        // child 0: hash=0xAA, tok1=3, sz1=4, tok2=0, sz2=0, tok3=0, sz3=0
        // child 1: hash=0xBB, tok1=4, sz1=4, tok2=0, sz2=0, tok3=0, sz3=0
        let page1 = vec![0xAAu32, 3, 4, 0, 0, 0, 0, 0xBBu32, 4, 4, 0, 0, 0, 0];
        // Page 2: payload of child 0: just 4 dwords (count=0 to keep simple)
        let page2 = vec![0u32, 0, 0, 0];
        // Page 3: payload of child 1
        let page3 = vec![0u32, 0, 0, 0];
        let f = synth_file(&[&page0, &page1, &page2, &page3]);

        let mut tf = TemplateFile::open(f.path()).unwrap();
        let hdr = tf.read_header().unwrap();
        assert_eq!(hdr.magic, 6);
        assert_eq!(hdr.template_type, 0x1234);
        assert_eq!(hdr.aux1, 7);
        assert_eq!(hdr.child_count, 2);
        assert_eq!(hdr.chunk1_token, 2);
    }

    #[test]
    fn bad_magic_rejected() {
        let page0 = vec![99u32, 0, 0x1234, 7, 0, 0, 0, 2];
        let f = synth_file(&[&page0]);
        let mut tf = TemplateFile::open(f.path()).unwrap();
        let err = tf.read_header().unwrap_err();
        assert!(matches!(
            err,
            TemplateFileError::BadMagic { actual: 99, .. }
        ));
    }

    #[test]
    fn child_count_sanity_cap() {
        let page0 = vec![6u32, 0, 0x1234, 7, 0, 2_000_000, 0, 2];
        let f = synth_file(&[&page0]);
        let mut tf = TemplateFile::open(f.path()).unwrap();
        let err = tf.read_header().unwrap_err();
        assert!(matches!(
            err,
            TemplateFileError::ChildCountTooLarge(2_000_000)
        ));
    }

    #[test]
    fn child_array_parsing_and_binary_search() {
        let page0 = vec![6u32, 0, 0x1234, 7, 0, 2, 0, 2];
        let page1 = vec![0xAAu32, 10, 20, 11, 5, 12, 3, 0xBBu32, 13, 30, 14, 6, 15, 2];
        let f = synth_file(&[&page0, &page1]);

        let mut tf = TemplateFile::open(f.path()).unwrap();
        let hdr = tf.read_header().unwrap();
        let children = tf.read_children(&hdr).unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].hash, 0xAA);
        assert_eq!(children[0].chunks, [(10, 20), (11, 5), (12, 3)]);
        assert_eq!(children[0].total_dwords(), 28);
        assert!(children[0].chunk_present(0));
        assert!(children[0].chunk_present(2));
        assert_eq!(children[1].hash, 0xBB);

        let found = TemplateFile::find_child(&children, 0xBB).unwrap();
        assert_eq!(found.hash, 0xBB);
        assert!(TemplateFile::find_child(&children, 0xCC).is_none());
    }

    #[test]
    fn chunked_read_with_chain_pointer() {
        // Build a scenario where children = 100 children = 700 dwords, which
        // exceeds 511 — so two physical chunks are needed and the chain
        // pointer at position 511 must be honored.
        //
        // Page 0: header (chunk1_token = 2)
        let mut page0 = vec![6u32, 0, 0x1234, 7, 0, 100, 0, 2];
        page0.resize(PAGE_WORDS, 0);

        // Page 1 (Fortran token=2): first 511 data dwords + chain pointer to
        // page 2 (Fortran token=3).
        let mut page1 = Vec::with_capacity(PAGE_WORDS);
        for i in 0..CHUNK_DATA_WORDS {
            page1.push((1000 + i) as u32);
        }
        page1.push(3); // chain pointer (Fortran token=3 → page 2)
                       // Pad to 512 (already exactly 512).
        assert_eq!(page1.len(), PAGE_WORDS);

        // Page 2 (Fortran token=3): remaining 700 - 511 = 189 dwords.
        let mut page2 = Vec::with_capacity(PAGE_WORDS);
        for i in CHUNK_DATA_WORDS..700 {
            page2.push((1000 + i) as u32);
        }
        // No chain pointer needed (remaining was <= 511).

        let f = synth_file(&[&page0, &page1, &page2]);
        let mut tf = TemplateFile::open(f.path()).unwrap();
        let data = tf.read_chunked(2, 700).unwrap();
        assert_eq!(data.len(), 700);
        for (i, w) in data.iter().enumerate() {
            assert_eq!(*w, (1000 + i) as u32, "mismatch at index {i}");
        }
    }

    #[test]
    fn null_chain_pointer_rejected() {
        // Build a chunked-read where the chain pointer is 0 (which would
        // otherwise read page -1).
        let mut page0 = vec![6u32, 0, 0x1234, 7, 0, 0, 0, 2];
        page0.resize(PAGE_WORDS, 0);

        let mut page1 = Vec::with_capacity(PAGE_WORDS);
        for i in 0..CHUNK_DATA_WORDS {
            page1.push((1000 + i) as u32);
        }
        page1.push(0); // bad chain pointer
        assert_eq!(page1.len(), PAGE_WORDS);

        let f = synth_file(&[&page0, &page1]);
        let mut tf = TemplateFile::open(f.path()).unwrap();
        let err = tf.read_chunked(2, 700).unwrap_err();
        assert!(matches!(
            err,
            TemplateFileError::TokenOutOfRange { token: 0, .. }
        ));
    }

    #[test]
    fn load_noun_payload_end_to_end_validates_type() {
        let page0 = vec![6u32, 0, 0x1234, 7, 0, 2, 0, 2];
        let page1 = vec![0xAAu32, 3, 4, 0, 0, 0, 0, 0xBBu32, 4, 4, 0, 0, 0, 0];
        let page2 = vec![100u32, 200, 300, 400];
        let page3 = vec![500u32, 600, 700, 800];
        let f = synth_file(&[&page0, &page1, &page2, &page3]);

        let mut tf = TemplateFile::open(f.path()).unwrap();
        // Right type, valid noun.
        let payload = tf.load_noun_payload(Some(0x1234), 0xAA).unwrap();
        assert_eq!(payload, vec![100, 200, 300, 400]);

        // Same file, different noun.
        let payload = tf.load_noun_payload(Some(0x1234), 0xBB).unwrap();
        assert_eq!(payload, vec![500, 600, 700, 800]);

        // Wrong type.
        let err = tf.load_noun_payload(Some(0xDEAD), 0xAA).unwrap_err();
        assert!(matches!(err, TemplateFileError::TypeMismatch { .. }));

        // Unknown noun.
        let err = tf.load_noun_payload(None, 0xCC).unwrap_err();
        assert!(matches!(err, TemplateFileError::NounNotFound(0xCC)));
    }

    #[test]
    fn read_payload_zero_size_returns_empty() {
        let child = TemplateChild {
            hash: 0xAA,
            chunks: [(0, 0), (0, 0), (0, 0)],
        };
        // Need any file just to construct TemplateFile.
        let page0 = vec![6u32, 0, 0, 0, 0, 0, 0, 0];
        let f = synth_file(&[&page0]);
        let mut tf = TemplateFile::open(f.path()).unwrap();
        let payload = tf.read_payload(&child).unwrap();
        assert!(payload.is_empty());
    }
}
