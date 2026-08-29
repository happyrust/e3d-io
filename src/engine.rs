use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::E3dReaderError;
use crate::index::{IndexPageView, SearchResult};
use crate::meta::constants::DATA_PAGE_INDEX;
use crate::meta::{DbDescriptor, FileInfo};
use crate::page::cache::PageCache;
use crate::page::io::PageReader;
use crate::page::PageHeader;
use crate::record::{self, ElementRecordView};
use crate::refno::RefNo;
use crate::session::SessionInfo;

/// Read-only engine for E3D 3.1 database files.
pub struct ReadOnlyEngine {
    cache: PageCache,
    descriptor: DbDescriptor,
    file_info: FileInfo,
    path: PathBuf,
    index_roots: Option<Vec<(u32, u32)>>,
}

impl ReadOnlyEngine {
    /// Open a database file for read-only access.
    ///
    /// Reads page 0 (descriptor) and page 1 (file info) to initialize metadata.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        Self::open_with_cache(path, 256)
    }

    pub fn open_with_cache(
        path: impl AsRef<Path>,
        cache_capacity: usize,
    ) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        let reader = PageReader::open(path)?;
        let mut cache = PageCache::new(reader, cache_capacity);

        let page0 = cache.get_page(0)?;
        let descriptor = DbDescriptor::parse(page0)?;

        let page1 = cache.get_page(FileInfo::PAGE_INDEX)?;
        let file_info = FileInfo::parse_page1(page1)?;

        Ok(Self {
            path: path.to_path_buf(),
            cache,
            descriptor,
            file_info,
            index_roots: None,
        })
    }

    pub fn descriptor(&self) -> &DbDescriptor {
        &self.descriptor
    }

    pub fn file_info(&self) -> &FileInfo {
        &self.file_info
    }

    pub fn page_size(&self) -> u32 {
        self.cache.page_size()
    }

    /// Complete pages addressable in this file.
    ///
    /// Nothing in the header records a page count, so the file length is the
    /// only bound. The word previously used here was page 1's extent-table
    /// cursor target — a refno prefix — which capped `ams7000_0001` at 15192
    /// of its 53406 pages.
    pub fn readable_page_count(&self) -> u32 {
        self.cache.physical_page_count()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read raw page bytes.
    pub fn read_page(&mut self, page_no: u32) -> Result<&[u8], E3dReaderError> {
        if page_no >= self.readable_page_count() {
            return Err(E3dReaderError::PageOutOfRange {
                page_no,
                total_pages: self.readable_page_count(),
            });
        }
        self.cache.get_page(page_no)
    }

    /// Parse the header of a page.
    pub fn read_page_header(&mut self, page_no: u32) -> Result<PageHeader, E3dReaderError> {
        let data = self.read_page(page_no)?;
        PageHeader::parse(data).ok_or_else(|| E3dReaderError::InvalidPage {
            page_no,
            detail: "page too small for header".to_string(),
        })
    }

    /// Walk session pages starting from the descriptor's session_page_no.
    pub fn sessions(&mut self) -> Result<Vec<SessionInfo>, E3dReaderError> {
        let session_page_no = self.descriptor.session_page_no;
        if session_page_no == 0 || session_page_no >= self.readable_page_count() {
            return Ok(vec![]);
        }

        let page_data = self.read_page(session_page_no)?;
        if page_data.len() < 24 {
            return Ok(vec![]);
        }

        let r = |off: usize| u32::from_be_bytes(page_data[off..off + 4].try_into().unwrap());
        let info = SessionInfo {
            session_mark: r(4),
            db_handle: r(8),
            session_id: r(12),
            start_page_no: r(16),
            end_page_no: r(20),
        };

        Ok(vec![info])
    }

    /// Find the root index page by scanning for index pages with highest level.
    pub fn find_index_root(&mut self) -> Result<Option<u32>, E3dReaderError> {
        let roots = self.find_all_index_roots()?;
        Ok(roots.first().map(|&(pn, _)| pn))
    }

    /// Find all B+tree root pages (highest level in each independent tree).
    /// Returns vec of (page_no, level) sorted by level descending.
    pub fn find_all_index_roots(&mut self) -> Result<Vec<(u32, u32)>, E3dReaderError> {
        if let Some(roots) = &self.index_roots {
            return Ok(roots.clone());
        }
        let roots = self.discover_index_roots()?;
        self.index_roots = Some(roots.clone());
        Ok(roots)
    }

    /// B+tree roots: index pages that no other index page names as a child.
    ///
    /// Taking only the pages sitting at the file's highest level, as this used
    /// to, throws away every independent tree whose own root is shallower. In
    /// `ams1112_0001` that left 33 of the 211 sampled elements with no index
    /// path at all, so each of their lookups fell back to reading the whole
    /// file backwards.
    ///
    /// Roots are ordered deepest first and then by page number, which is the
    /// order the previous rule produced for the pages it did return.
    fn discover_index_roots(&mut self) -> Result<Vec<(u32, u32)>, E3dReaderError> {
        let total = self.readable_page_count();
        let mut index_pages: Vec<(u32, u32)> = Vec::new();
        let mut children: HashSet<u32> = HashSet::new();

        for page_no in 0..total {
            let data = self.cache.get_page(page_no)?;
            let Some(hdr) = PageHeader::parse(data) else {
                continue;
            };
            if hdr.type_id != DATA_PAGE_INDEX {
                continue;
            }
            let Ok(idx_page) = IndexPageView::parse(data) else {
                continue;
            };
            if !idx_page.is_leaf() {
                children.extend(idx_page.entries.iter().map(|entry| entry.page_no));
            }
            index_pages.push((page_no, idx_page.level));
        }

        let mut roots: Vec<(u32, u32)> = index_pages
            .into_iter()
            .filter(|(page_no, _)| !children.contains(page_no))
            .collect();
        roots.sort_by_key(|&(page_no, level)| (std::cmp::Reverse(level), page_no));
        Ok(roots)
    }

    /// Search the B+tree index for a RefNo. Returns the index entry if found.
    pub fn search_index(
        &mut self,
        root_page_no: u32,
        target: RefNo,
    ) -> Result<Option<SearchResult>, E3dReaderError> {
        let mut current_page = root_page_no;
        let max_depth = 20;

        for _ in 0..max_depth {
            let data = self.cache.get_page(current_page)?;
            let idx_page = IndexPageView::parse(data).map_err(|_| E3dReaderError::IndexError {
                page_no: current_page,
                detail: "failed to parse index page".into(),
            })?;

            if idx_page.is_leaf() {
                if let Some(entry) = idx_page.find_in_leaf(target) {
                    return Ok(Some(SearchResult {
                        entry: *entry,
                        leaf_page_no: current_page,
                    }));
                }
                return Ok(None);
            }

            match idx_page.select_child(target) {
                Some(child) => current_page = child,
                None => return Ok(None),
            }
        }

        Err(E3dReaderError::IndexError {
            page_no: current_page,
            detail: "B+tree depth exceeded maximum".into(),
        })
    }

    /// Scan all index pages and collect all indexed RefNos.
    pub fn scan_all_indexed_refnos(&mut self) -> Result<Vec<(RefNo, u32, u32)>, E3dReaderError> {
        let total = self.readable_page_count();
        let mut results = Vec::new();

        for page_no in 0..total {
            let data = self.cache.get_page(page_no)?;
            let hdr = match PageHeader::parse(data) {
                Some(h) => h,
                None => continue,
            };
            if hdr.type_id != DATA_PAGE_INDEX {
                continue;
            }
            if let Ok(idx_page) = IndexPageView::parse(data) {
                if idx_page.is_leaf() {
                    for entry in &idx_page.entries {
                        if !entry.is_start_marker() {
                            results.push((entry.refno, entry.page_no, entry.offset_words));
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Find an element by RefNo: search all B+tree roots, then read the record.
    ///
    /// A leaf entry is believed only once the record it addresses says the same
    /// RefNo back. Stale entries outlive the record they were written for:
    /// `ams1112_0001` indexes 17496/950 from eleven leaves, and ten of them
    /// address a location that now holds an unrelated element. Returning what
    /// the index pointed at would answer a lookup with somebody else's data.
    pub fn find_element(
        &mut self,
        refno: RefNo,
    ) -> Result<Option<ElementRecordView>, E3dReaderError> {
        let roots = self.find_all_index_roots()?;

        // A Dabacon session appends a new version of a record instead of
        // overwriting the old one, and the index goes on addressing every
        // version: `ams1112_0001` holds twelve readable copies of SITE 9304/2.
        // Answering with the first root that confirmed took whichever copy was
        // indexed first, which is the oldest — the copy it returned for that
        // SITE was missing the UDA value E3D reports. The record scan below
        // already reads backwards for exactly this reason; this now agrees
        // with it and takes the copy furthest into the file.
        let mut newest: Option<(u32, u32, ElementRecordView)> = None;
        for &(root_page, _) in &roots {
            let Ok(Some(result)) = self.search_index(root_page, refno) else {
                continue;
            };
            let at = (result.entry.page_no, result.entry.offset_words);
            if newest
                .as_ref()
                .is_some_and(|(page, offset, _)| (*page, *offset) >= at)
            {
                continue;
            }
            let Ok(view) = record::read_element_record(&mut self.cache, &result.entry) else {
                continue;
            };
            let Ok(parsed) = record::element::ParsedElement::parse(&view.raw_bytes) else {
                continue;
            };
            if parsed.refno == refno {
                newest = Some((at.0, at.1, view));
            }
        }
        if let Some((_, _, view)) = newest {
            return Ok(Some(view));
        }

        self.find_element_by_record_scan(refno)
    }

    /// Last-resort lookup for live/session records that are present in data pages but absent
    /// from the B+tree leaves exposed by the current file snapshot.
    ///
    /// Pages and candidates are visited backwards because Dabacon sessions append newer record
    /// versions later in the file.  A byte-pattern hit is accepted only when it also has a
    /// plausible element header and the normal record parser confirms the requested RefNo.
    pub fn find_element_by_record_scan(
        &mut self,
        refno: RefNo,
    ) -> Result<Option<ElementRecordView>, E3dReaderError> {
        for page_no in (0..self.readable_page_count()).rev() {
            let candidates = {
                let page = self.cache.get_page(page_no)?;
                record_candidate_offsets(page, refno)
            };
            for offset_words in candidates.into_iter().rev() {
                let Ok(view) = self.read_element_direct(refno, page_no, offset_words) else {
                    continue;
                };
                let Ok(parsed) = record::element::ParsedElement::parse(&view.raw_bytes) else {
                    continue;
                };
                if parsed.refno == refno {
                    return Ok(Some(view));
                }
            }
        }
        Ok(None)
    }

    /// Read an element directly given page_no and offset_words (bypassing B+tree search).
    /// Use when you already have the location from scan_all_indexed_refnos.
    pub fn read_element_direct(
        &mut self,
        refno: RefNo,
        page_no: u32,
        offset_words: u32,
    ) -> Result<ElementRecordView, E3dReaderError> {
        use crate::index::IndexEntry;
        let entry = IndexEntry {
            refno,
            page_no,
            offset_words,
            flag: 0,
        };
        record::read_element_record(&mut self.cache, &entry)
    }

    /// Resolve an attribute value for an element by RefNo, walking the
    /// SYNO chain when the direct slot reads as zero.
    ///
    /// Algorithm:
    /// 1. Load the element and parse its record header.
    /// 2. Decode the attribute via [`record::atnlog::decode_attribute_by_hash`]
    ///    with `follow_syno_chain = false`.
    /// 3. If the decoded value is "zero-like" (per the high-level API's
    ///    own predicate), look up SYNO. If SYNO yields a non-trivial
    ///    `DbRef`, recurse into that element. Recursion is bounded by
    ///    `max_chain_depth` to avoid pathological cycles in corrupt data.
    ///
    /// Returns `Ok(Some(value))` when a value is found (direct or via
    /// chain), `Ok(None)` when the chain ends without resolving, and
    /// `Err(ResolveError)` when the attribute is unknown or I/O fails.
    ///
    /// # This addresses attributes the wrong way
    ///
    /// It reads slot `6 + atgtdf_position - 1`, as though attributes sat in
    /// ATGTDF order right behind a six-word header. `core.dll` locates them by
    /// the descriptor's `encoded_location & 0xFFFFF`, an absolute word offset
    /// that is neither sequential nor based at six: `BRAN` puts its first
    /// eleven at words 11, 12, 12, 12, 12, 13, 20, 27, 34, 41, 43. The values
    /// this returns are read from the wrong words except by coincidence.
    #[deprecated(
        note = "addresses attributes by ATGTDF position; use extract_element_with_descriptors"
    )]
    pub fn resolve_attribute(
        &mut self,
        refno: RefNo,
        attr_hash: u32,
        attlib: &e3d_attlib::AttlibData,
        max_chain_depth: u32,
    ) -> Result<Option<record::atnlog::AttrValue>, ResolveError> {
        use crate::record::atnlog::{
            decode_attribute_by_hash, AttrValue, LookupError, DEFAULT_HEADER_WORDS, SYNO_CHAIN_HASH,
        };
        use crate::record::element::ParsedElement;

        let mut current = refno;
        let mut visited: Vec<RefNo> = Vec::with_capacity(max_chain_depth as usize + 1);

        for hop in 0..=max_chain_depth {
            if visited.iter().any(|r| *r == current) {
                return Err(ResolveError::ChainCycle { at: current });
            }
            visited.push(current);

            let view = self
                .find_element(current)
                .map_err(ResolveError::Io)?
                .ok_or(ResolveError::ElementNotFound(current))?;
            let parsed = ParsedElement::parse(&view.raw_bytes)
                .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;

            let value = match decode_attribute_by_hash(
                &parsed,
                attlib,
                attr_hash,
                false,
                DEFAULT_HEADER_WORDS,
            ) {
                Ok(v) => v,
                Err(LookupError::NotInAttlib(h)) => return Err(ResolveError::AttrNotInAttlib(h)),
                Err(e) => return Err(ResolveError::Lookup(e)),
            };

            // Direct hit (non-zero): return immediately.
            if let Some(v) = &value {
                if !is_zero_like(v) {
                    return Ok(value);
                }
            }
            // Out of hops -> stop here with whatever direct returned.
            if hop == max_chain_depth {
                return Ok(value);
            }

            // Try SYNO chain on this element.
            let syno = decode_attribute_by_hash(
                &parsed,
                attlib,
                SYNO_CHAIN_HASH,
                false,
                DEFAULT_HEADER_WORDS,
            );
            let next_ref = match syno {
                Ok(Some(AttrValue::DbRef(r))) if r.is_valid() => r,
                _ => return Ok(value),
            };
            current = next_ref;
        }
        unreachable!("loop returns on every iteration")
    }

    pub fn summarize_element_diagnostics(
        &mut self,
        refno: RefNo,
        attlib: &e3d_attlib::AttlibData,
    ) -> Result<ElementSummaryDiagnostics, ResolveError> {
        use crate::record::atnlog::{
            decode_attribute_by_hash, LookupError, DEFAULT_HEADER_WORDS, SYNO_CHAIN_HASH,
        };
        use crate::record::element::ParsedElement;

        let view = self
            .find_element(refno)
            .map_err(ResolveError::Io)?
            .ok_or(ResolveError::ElementNotFound(refno))?;
        let parsed = ParsedElement::parse(&view.raw_bytes)
            .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;

        let mut attributes = Vec::new();
        let mut skipped = Vec::new();
        for def in attlib.attr_defs_unique() {
            let hash = def.hash;
            if hash == SYNO_CHAIN_HASH {
                skipped.push(AttributeSkip {
                    hash,
                    name: attr_name(hash),
                    reason: AttributeSkipReason::Structural,
                });
                continue;
            }
            match decode_attribute_by_hash(&parsed, attlib, hash, false, DEFAULT_HEADER_WORDS) {
                Ok(Some(value)) if !is_zero_like(&value) => {
                    attributes.push((attr_name(hash), value));
                }
                Ok(Some(_)) => skipped.push(AttributeSkip {
                    hash,
                    name: attr_name(hash),
                    reason: AttributeSkipReason::ZeroLike,
                }),
                Ok(None) => skipped.push(AttributeSkip {
                    hash,
                    name: attr_name(hash),
                    reason: AttributeSkipReason::SlotOutOfRange,
                }),
                Err(LookupError::NotInAttlib(_)) => skipped.push(AttributeSkip {
                    hash,
                    name: attr_name(hash),
                    reason: AttributeSkipReason::NotInAttlib,
                }),
                Err(err) => skipped.push(AttributeSkip {
                    hash,
                    name: attr_name(hash),
                    reason: AttributeSkipReason::DecodeFailed(err.to_string()),
                }),
            }
        }

        Ok(ElementSummaryDiagnostics {
            summary: ElementSummary {
                refno,
                noun_hash: parsed.noun_hash,
                noun_name: e3d_attlib::db1_dehash(parsed.noun_hash),
                implicit_word_count: parsed.impl_len_words.max(0) as u32,
                attributes,
            },
            skipped,
        })
    }
}

fn record_candidate_offsets(page: &[u8], target: RefNo) -> Vec<u32> {
    const MIN_ELEMENT_HEADER_WORDS: u32 = 6;
    const MAX_IMPLICIT_WORDS: u32 = 16 * 1024;
    // Public and internal E3D noun identifiers occupy the system-name range;
    // tiny integers in later extents are page/array fields, not noun hashes.
    const MIN_NOUN_HASH: u32 = 0x0008_0000;

    if page.len() < 24 {
        return Vec::new();
    }
    let target = target.to_be_bytes();
    let mut offsets = Vec::new();
    for start in (0..=page.len() - 24).step_by(4) {
        if page[start + 4..start + 12] != target {
            continue;
        }
        let implicit_words = u32::from_be_bytes(page[start..start + 4].try_into().unwrap());
        if !(MIN_ELEMENT_HEADER_WORDS..=MAX_IMPLICIT_WORDS).contains(&implicit_words) {
            continue;
        }
        let noun_hash = u32::from_be_bytes(page[start + 12..start + 16].try_into().unwrap());
        if noun_hash < MIN_NOUN_HASH {
            continue;
        }
        offsets.push((start / 2) as u32);
    }
    offsets
}

#[cfg(test)]
mod record_scan_tests {
    use super::*;

    #[test]
    fn candidate_scan_rejects_owner_hits_and_keeps_element_headers() {
        let target = RefNo::new(24383, 73930);
        let mut page = vec![0u8; 128];

        // An owner/member reference has no plausible implicit-length word immediately before it.
        page[12..20].copy_from_slice(&target.to_be_bytes());

        // A real element header: impl_len, self RefNo, noun hash, owner RefNo.
        page[64..68].copy_from_slice(&86u32.to_be_bytes());
        page[68..76].copy_from_slice(&target.to_be_bytes());
        page[76..80].copy_from_slice(&0x000C_551Cu32.to_be_bytes());
        page[80..88].copy_from_slice(&RefNo::new(24383, 73929).to_be_bytes());

        assert_eq!(record_candidate_offsets(&page, target), vec![32]);
    }

    #[test]
    fn candidate_scan_rejects_small_page_fields_as_noun_hashes() {
        let target = RefNo::new(24383, 73930);
        let mut page = vec![0u8; 64];
        page[0..4].copy_from_slice(&25u32.to_be_bytes());
        page[4..12].copy_from_slice(&target.to_be_bytes());
        page[12..16].copy_from_slice(&5u32.to_be_bytes());
        assert!(record_candidate_offsets(&page, target).is_empty());
    }
}

fn attr_name(hash: u32) -> String {
    e3d_attlib::lookup_system_name(hash)
        .map(String::from)
        .or_else(|| e3d_attlib::db1_dehash(hash))
        .unwrap_or_else(|| format!("0x{:08X}", hash))
}

/// "Zero-like" predicate used by [`ReadOnlyEngine::resolve_attribute`]
/// to decide whether to walk the SYNO chain. Mirrors the predicate
/// internal to [`record::atnlog::decode_attribute_by_hash`].
fn is_zero_like(v: &record::atnlog::AttrValue) -> bool {
    use record::atnlog::{AttrValue, RAW_USE_DEFAULT};
    match v {
        AttrValue::Bool(false) => true,
        AttrValue::Int(0) => true,
        AttrValue::Real(r) => *r == 0.0,
        AttrValue::Text(0) => true,
        AttrValue::Raw(0) => true,
        AttrValue::Raw(w) if *w == RAW_USE_DEFAULT => true,
        AttrValue::IntArray(a) => a.is_empty(),
        AttrValue::RealArray(a) => a.is_empty(),
        AttrValue::BoolArray(a) => a.is_empty(),
        AttrValue::DbRefArray(a) => a.is_empty(),
        AttrValue::TextArray(a) => a.is_empty(),
        _ => false,
    }
}

impl ReadOnlyEngine {
    /// One-call element summary: load element, parse, and decode every
    /// attribute known to the supplied attlib that yields a non-trivial
    /// value. Returns an [`ElementSummary`] suitable for printing,
    /// serialisation, or further analysis.
    ///
    /// This is the fast path: the element is loaded once, then each
    /// attribute is decoded directly via the record-level API.
    /// SYNO chain recursion is performed only for attributes whose
    /// direct slot resolves to zero AND whose chain pointer points to
    /// another element on this same DB. `max_chain_depth` caps the
    /// recursion as in [`resolve_attribute`]; pass `0` to disable the
    /// chain walk entirely (the fastest mode).
    ///
    /// Attributes not present in the attlib's ATGTDF table are silently
    /// skipped; transient decode failures are also skipped (the caller
    /// can re-run `resolve_attribute` individually to introspect).
    ///
    /// # This addresses attributes the wrong way
    ///
    /// See [`resolve_attribute`], which it shares the addressing with. On the
    /// `NXTR` element this crate ships as a sample it produces
    /// `VISI = Int(1079410688)` — 0x40580000, the high half of the double
    /// 96.0 — and `UNIT = RealArray([2.12e-313, 1e-323])`. Use
    /// [`extract_element_with_descriptors`], which reads the descriptor's own
    /// word offsets.
    ///
    /// [`resolve_attribute`]: ReadOnlyEngine::resolve_attribute
    /// [`extract_element_with_descriptors`]: ReadOnlyEngine::extract_element_with_descriptors
    #[deprecated(
        note = "addresses attributes by ATGTDF position; use extract_element_with_descriptors"
    )]
    pub fn summarize_element(
        &mut self,
        refno: RefNo,
        attlib: &e3d_attlib::AttlibData,
        max_chain_depth: u32,
    ) -> Result<ElementSummary, ResolveError> {
        use crate::record::atnlog::{
            decode_attribute_by_hash, AttrValue, DEFAULT_HEADER_WORDS, SYNO_CHAIN_HASH,
        };
        use crate::record::element::ParsedElement;

        // Single load of the target element.
        let view = self
            .find_element(refno)
            .map_err(ResolveError::Io)?
            .ok_or(ResolveError::ElementNotFound(refno))?;
        let parsed = ParsedElement::parse(&view.raw_bytes)
            .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;
        let noun_hash = parsed.noun_hash;
        let noun_name = e3d_attlib::db1_dehash(noun_hash);

        let mut attributes = Vec::new();
        let mut chain_targets: Vec<(String, u32, RefNo)> = Vec::new();

        // Dedup by hash: duplicate ATGTDF entries point to the same
        // resolved slot via atgtdf_position, so re-iterating duplicates
        // would just duplicate work and output.
        for def in attlib.attr_defs_unique() {
            let hash = def.hash;
            // SYNO is structural; skip from user-facing output.
            if hash == SYNO_CHAIN_HASH {
                continue;
            }
            let direct = match decode_attribute_by_hash(
                &parsed,
                attlib,
                hash,
                /* follow_syno = */ false,
                DEFAULT_HEADER_WORDS,
            ) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name = e3d_attlib::lookup_system_name(hash)
                .map(String::from)
                .or_else(|| e3d_attlib::db1_dehash(hash))
                .unwrap_or_else(|| format!("0x{:08X}", hash));

            if let Some(v) = direct {
                if !is_zero_like(&v) {
                    attributes.push((name, v));
                    continue;
                }
                // Direct was zero — queue the SYNO walk for later if
                // chain depth permits.
                if max_chain_depth > 0 {
                    // Peek SYNO ref on the SAME element without
                    // recursing yet; we collect chain targets and
                    // resolve them in a second pass to keep the inner
                    // loop hot in the local element.
                    if let Ok(Some(AttrValue::DbRef(target))) = decode_attribute_by_hash(
                        &parsed,
                        attlib,
                        SYNO_CHAIN_HASH,
                        false,
                        DEFAULT_HEADER_WORDS,
                    ) {
                        if target.is_valid() && target != refno {
                            chain_targets.push((name, hash, target));
                        }
                    }
                }
            }
        }

        // Second pass: walk each chain target individually using the
        // bounded resolve_attribute helper. This is the slow path; it
        // only fires when an attribute genuinely needs chain resolution.
        for (name, hash, _target) in chain_targets {
            #[allow(deprecated)] // both halves of the same deprecated path
            if let Ok(Some(v)) = self.resolve_attribute(refno, hash, attlib, max_chain_depth) {
                if !is_zero_like(&v) {
                    attributes.push((name, v));
                }
            }
        }

        Ok(ElementSummary {
            refno,
            noun_hash,
            noun_name,
            implicit_word_count: parsed.impl_len_words.max(0) as u32,
            attributes,
        })
    }
}

/// Output of [`ReadOnlyEngine::summarize_element`]. Holds the element's
/// identity plus every attribute that the attlib knows about and that
/// resolved (direct or via SYNO chain) to a non-zero value.
#[derive(Debug, Clone)]
pub struct ElementSummary {
    pub refno: RefNo,
    pub noun_hash: u32,
    pub noun_name: Option<String>,
    pub implicit_word_count: u32,
    pub attributes: Vec<(String, record::atnlog::AttrValue)>,
}

#[derive(Debug, Clone)]
pub struct ElementSummaryDiagnostics {
    pub summary: ElementSummary,
    pub skipped: Vec<AttributeSkip>,
}

#[derive(Debug, Clone)]
pub struct AttributeSkip {
    pub hash: u32,
    pub name: String,
    pub reason: AttributeSkipReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeSkipReason {
    Structural,
    NotInAttlib,
    SlotOutOfRange,
    DecodeFailed(String),
    ZeroLike,
}

impl ReadOnlyEngine {
    /// Template-aware variant of [`summarize_element`]: iterates the
    /// supplied [`NounTemplate`]'s attribute list and decodes each one
    /// via its **global ATGTDF position** (per IDA `ATNLOG`
    /// `sub_55BC98B`: `value_word = page_cache[base + atgtdf_index]`).
    /// Template provides the *which* (the noun's authoritative attr
    /// set, per `sub_5B03900` / GALFE); attlib provides the *where*
    /// (the global ATGTDF-position used as the slot index inside the
    /// element record).
    ///
    /// **Step 16 clarification**: the previous cut of this function
    /// used `tattr.stride` cumulatively to position element slots,
    /// which only happened to work for ATGTSX-synthesized templates
    /// where `stride = 1` uniformly. For real DB-file-loaded templates
    /// (e.g. NXTR has 21 attrs with stride 10/11), the stride field is
    /// payload-internal (used to walk the template byte stream) and
    /// must NOT be used as an element-data slot stride.
    ///
    /// When the template is absent or empty, callers should fall back
    /// to [`summarize_element`] (which iterates all of ATGTDF).
    ///
    /// Attributes whose hash is not present in `attlib.attr_def_map`
    /// (the ATGTDF table) are silently skipped — the template can list
    /// attributes the attlib doesn't carry (UDA, project-extension,
    /// etc.); decoding those requires #2b's ATGTIX-payload decoder.
    ///
    /// SYNO chain resolution is NOT performed here in this first cut —
    /// the template path is intentionally pure-fast so it can be
    /// benchmarked against the legacy path without chain-walk noise.
    /// Callers needing chain resolution can post-process: see
    /// [`resolve_attribute`] for a per-hash chain walker.
    ///
    /// [`summarize_element`]: ReadOnlyEngine::summarize_element
    /// [`resolve_attribute`]: ReadOnlyEngine::resolve_attribute
    /// [`NounTemplate`]: record::template::NounTemplate
    #[deprecated(note = "use extract_element_with_descriptors for physical noun descriptors")]
    pub fn summarize_element_with_template(
        &mut self,
        refno: RefNo,
        attlib: &e3d_attlib::AttlibData,
        template: &record::template::NounTemplate,
    ) -> Result<ElementSummary, ResolveError> {
        use crate::record::element::ParsedElement;

        let view = self
            .find_element(refno)
            .map_err(ResolveError::Io)?
            .ok_or(ResolveError::ElementNotFound(refno))?;
        let parsed = ParsedElement::parse(&view.raw_bytes)
            .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;
        self.summarize_parsed_element_with_template(refno, &parsed, attlib, template)
    }

    #[deprecated(note = "use extract_element_with_descriptors for physical noun descriptors")]
    pub fn summarize_element_with_template_provider(
        &mut self,
        refno: RefNo,
        attlib: &e3d_attlib::AttlibData,
        provider: &mut record::template::TemplateProvider,
    ) -> Result<ElementSummary, ResolveError> {
        let view = self
            .find_element(refno)
            .map_err(ResolveError::Io)?
            .ok_or(ResolveError::ElementNotFound(refno))?;
        let parsed = record::element::ParsedElement::parse(&view.raw_bytes)
            .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;
        let template = provider
            .template_for(parsed.noun_hash)
            .map_err(ResolveError::TemplateLoad)?
            .ok_or(ResolveError::TemplateMissing(parsed.noun_hash))?;

        self.summarize_parsed_element_with_template(refno, &parsed, attlib, template)
    }

    /// Decode every descriptor-backed implicit attribute for one persisted element.
    ///
    /// The returned vector preserves descriptor order and includes an explicit status
    /// for non-implicit, metadata-missing, unsupported, and out-of-bounds attributes.
    pub fn extract_element_with_descriptors(
        &mut self,
        refno: RefNo,
        attlib: &e3d_attlib::AttlibData,
        provider: &mut record::template::TemplateProvider,
    ) -> Result<record::descriptor::ElementExtraction, ResolveError> {
        let view = self
            .find_element(refno)
            .map_err(ResolveError::Io)?
            .ok_or(ResolveError::ElementNotFound(refno))?;
        let parsed = record::element::ParsedElement::parse(&view.raw_bytes)
            .map_err(|e| ResolveError::ParseFailed(format!("{e}")))?;
        let template = provider
            .template_for(parsed.noun_hash)
            .map_err(ResolveError::TemplateLoad)?
            .ok_or(ResolveError::TemplateMissing(parsed.noun_hash))?;

        self.extract_parsed_element_with_descriptors(refno, &parsed, attlib, template)
    }

    pub fn extract_parsed_element_with_descriptors(
        &self,
        refno: RefNo,
        parsed: &record::element::ParsedElement,
        attlib: &e3d_attlib::AttlibData,
        template: &record::template::NounTemplate,
    ) -> Result<record::descriptor::ElementExtraction, ResolveError> {
        if template.noun_hash != parsed.noun_hash {
            return Err(ResolveError::TemplateNounMismatch {
                expected: parsed.noun_hash,
                actual: template.noun_hash,
            });
        }
        if template.attrs.is_empty() {
            return Err(ResolveError::TemplateEmpty(parsed.noun_hash));
        }
        if !template.is_authoritative() {
            return Err(ResolveError::TemplateNotAuthoritative(parsed.noun_hash));
        }
        let (format_word_10, layout) = match read_implicit_u32(parsed, 10) {
            Some(format_word) => (
                format_word,
                record::template::NounTemplate::layout_from_format_word(format_word),
            ),
            None if template.attrs.iter().all(|attribute| {
                attribute.compact.location.word_offset() == 0
                    && attribute.wide.location.word_offset() == 0
            }) =>
            {
                (0, record::template::StorageLayout::Compact)
            }
            None => return Err(ResolveError::FormatWordMissing(parsed.noun_hash)),
        };
        let attributes = template
            .attrs
            .iter()
            .map(|descriptor| {
                let name = e3d_attlib::lookup_system_name(descriptor.hash)
                    .map(String::from)
                    .or_else(|| e3d_attlib::db1_dehash(descriptor.hash))
                    .unwrap_or_else(|| format!("0x{:08X}", descriptor.hash));
                let mut extracted = record::descriptor::extract_attribute(
                    parsed,
                    descriptor,
                    attlib.attr_def(descriptor.hash),
                    layout,
                    name,
                );
                if extracted.status == record::descriptor::AttributeExtractionStatus::NonImplicit {
                    if let Some(explicit) = parsed
                        .explicit_attributes
                        .iter()
                        .rev()
                        .find(|attribute| attribute.hash == descriptor.hash)
                    {
                        extracted.value = Some(explicit_value_to_descriptor_value(
                            &explicit.value,
                            attlib.attr_def(descriptor.hash),
                            descriptor.storage_type_code,
                        ));
                        extracted.status =
                            record::descriptor::AttributeExtractionStatus::DecodedExplicit;
                    }
                }
                // Nothing persisted: the noun's own default record answers,
                // exactly as `sub_5B03B90` answers a `GCEA` that found no value.
                if matches!(
                    extracted.status,
                    record::descriptor::AttributeExtractionStatus::NonImplicit
                        | record::descriptor::AttributeExtractionStatus::DefaultRequired
                ) {
                    if let Some(default) = record::descriptor::extract_default(
                        descriptor,
                        attlib.attr_def(descriptor.hash),
                        &template.defaults,
                        extracted.name.clone(),
                    ) {
                        extracted.value = default.value;
                        extracted.status =
                            record::descriptor::AttributeExtractionStatus::TemplateDefault;
                    }
                }
                extracted
            })
            .collect();

        Ok(record::descriptor::ElementExtraction {
            refno,
            owner: parsed.owner,
            noun_hash: parsed.noun_hash,
            noun_name: e3d_attlib::db1_dehash(parsed.noun_hash),
            implicit_word_count: parsed.impl_len_words.max(0) as u32,
            format_word_10,
            layout,
            attributes,
        })
    }

    fn summarize_parsed_element_with_template(
        &self,
        refno: RefNo,
        parsed: &record::element::ParsedElement,
        attlib: &e3d_attlib::AttlibData,
        template: &record::template::NounTemplate,
    ) -> Result<ElementSummary, ResolveError> {
        let extraction =
            self.extract_parsed_element_with_descriptors(refno, parsed, attlib, template)?;
        let noun_hash = extraction.noun_hash;
        let attributes = extraction
            .attributes
            .into_iter()
            .filter_map(|attr| {
                let value = descriptor_value_to_attr_value(attr.value?)?;
                (!is_zero_like(&value)).then_some((attr.name, value))
            })
            .collect();

        Ok(ElementSummary {
            refno,
            noun_hash,
            noun_name: e3d_attlib::db1_dehash(noun_hash),
            implicit_word_count: parsed.impl_len_words.max(0) as u32,
            attributes,
        })
    }
}

fn read_implicit_u32(parsed: &record::element::ParsedElement, word: u32) -> Option<u32> {
    let start = word as usize * 4;
    Some(u32::from_be_bytes(
        parsed
            .implicit_data
            .get(start..start + 4)?
            .try_into()
            .ok()?,
    ))
}

fn descriptor_value_to_attr_value(
    value: record::descriptor::DescriptorValue,
) -> Option<record::atnlog::AttrValue> {
    use record::atnlog::AttrValue;
    use record::descriptor::DescriptorValue;
    Some(match value {
        DescriptorValue::Unset => return None,
        DescriptorValue::Bool(value) => AttrValue::Bool(value),
        DescriptorValue::Int(value) => AttrValue::Int(value),
        DescriptorValue::Real(value) => AttrValue::Real(value),
        DescriptorValue::Text(_) => return None,
        DescriptorValue::RefNo(value) => AttrValue::DbRef(value),
        DescriptorValue::Word { raw, .. } => AttrValue::Text(raw),
        DescriptorValue::IntArray(value) => AttrValue::IntArray(value),
        DescriptorValue::RealArray(value) => AttrValue::RealArray(value),
        DescriptorValue::RefNoArray(value) => AttrValue::DbRefArray(value),
        DescriptorValue::WordArray(value) => AttrValue::TextArray(value),
        DescriptorValue::BoolArray(value) => AttrValue::BoolArray(value),
        DescriptorValue::RawWords(value) => AttrValue::Raw(*value.first()?),
    })
}

fn explicit_value_to_descriptor_value(
    value: &record::explicit::ExplicitValue,
    def: Option<&e3d_attlib::AttrDefEntry>,
    storage_type_code: u32,
) -> record::descriptor::DescriptorValue {
    use record::descriptor::DescriptorValue;
    use record::explicit::ExplicitValue;
    match value {
        ExplicitValue::Integer(value)
            if def.is_some_and(|def| matches!(def.data_type, 5 | 6)) || storage_type_code == 3 =>
        {
            let raw = *value as u32;
            DescriptorValue::Word {
                raw,
                text: e3d_attlib::lookup_system_name(raw)
                    .map(String::from)
                    .or_else(|| e3d_attlib::db1_dehash(raw)),
            }
        }
        ExplicitValue::Integer(value) => DescriptorValue::Int(*value),
        ExplicitValue::Real(value) => DescriptorValue::Real(*value),
        ExplicitValue::Bool(value) => DescriptorValue::Bool(*value),
        ExplicitValue::Text(value) => DescriptorValue::Text(value.clone()),
        ExplicitValue::RefNo(value) => DescriptorValue::RefNo(*value),
        // A word array is stored exactly like an integer array, so it arrives
        // here as one: SBOL's `Bitems` reported [546602, 702806, 702806,
        // 546602] where E3D reports NUT WASH WASH NUT — the same four words
        // undehashed.
        ExplicitValue::IntArray(value)
            if def.is_some_and(|def| matches!(def.data_type, 5 | 6)) || storage_type_code == 3 =>
        {
            DescriptorValue::WordArray(value.iter().map(|item| *item as u32).collect())
        }
        // BITE has no attlib definition and its template declares no word
        // storage, so which of the two it is stays undetermined. That is what
        // `RawWords` already means on the implicit path for unknown multi-word
        // storage, and it lets the reconciliation accept either reading rather
        // than commit to a guess here.
        ExplicitValue::IntArray(value) if def.is_none() => {
            DescriptorValue::RawWords(value.iter().map(|item| *item as u32).collect())
        }
        ExplicitValue::IntArray(value) => DescriptorValue::IntArray(value.clone()),
        ExplicitValue::RealArray(value) => DescriptorValue::RealArray(value.clone()),
        ExplicitValue::RefNoArray(value) => DescriptorValue::RefNoArray(value.clone()),
        ExplicitValue::RawWords(value) => DescriptorValue::RawWords(value.clone()),
    }
}

#[cfg(test)]
mod explicit_semantic_tests {
    use super::*;
    #[test]
    fn explicit_integer_uses_word_storage_semantics_without_attlib_definition() {
        let value = explicit_value_to_descriptor_value(
            &record::explicit::ExplicitValue::Integer(914_199),
            None,
            3,
        );

        assert_eq!(
            value,
            record::descriptor::DescriptorValue::Word {
                raw: 914_199,
                text: Some("FALS".into()),
            }
        );
    }
}

/// Error returned by [`ReadOnlyEngine::resolve_attribute`].
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("I/O error during element lookup: {0}")]
    Io(E3dReaderError),
    #[error("element not found: RefNo({0})")]
    ElementNotFound(RefNo),
    #[error("element record parse failed: {0}")]
    ParseFailed(String),
    #[error("attribute hash 0x{0:08X} not present in attlib ATGTDF table")]
    AttrNotInAttlib(u32),
    #[error(transparent)]
    Lookup(#[from] record::atnlog::LookupError),
    #[error(transparent)]
    TemplateLoad(#[from] record::template::TemplateLoadError),
    #[error("noun descriptor template is missing for 0x{0:08X}")]
    TemplateMissing(u32),
    #[error("noun descriptor template is empty for 0x{0:08X}")]
    TemplateEmpty(u32),
    #[error("noun template 0x{0:08X} has names but no physical descriptor locations")]
    TemplateNotAuthoritative(u32),
    #[error("element noun 0x{0:08X} has no implicit format word 10")]
    FormatWordMissing(u32),
    #[error("template noun mismatch: element noun 0x{expected:08X}, template noun 0x{actual:08X}")]
    TemplateNounMismatch { expected: u32, actual: u32 },
    #[error("SYNO chain contains cycle at RefNo({at})")]
    ChainCycle { at: RefNo },
}
