use std::path::{Path, PathBuf};

use crate::element_name;
use crate::error::E3dReaderError;
use crate::index::reference::ReferenceTable;
use crate::index::{IndexCursor, IndexEntry, SearchResult};
use crate::meta::{DbDescriptor, FileInfo};
use crate::page::cache::PageCache;
use crate::page::io::PageReader;
use crate::page::{PageHeader, PageId};
use crate::record::element::ParsedElement;
use crate::record::{self, ElementRecordView};
use crate::refno::RefNo;
use crate::session::{
    ChangedElements, SessionDiff, SessionInfo, SessionPage, SessionSelector, TableRoot,
};

/// How much of each element [`ReadOnlyEngine::scan_elements`] reads.
///
/// The tiers exist because the price of a whole-database scan is set by what
/// each element costs: a derived type index needs four header words, a name
/// index needs one explicit attribute, and only a reference index needs the
/// record parsed whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanTier {
    /// The eleven-word record header: noun and owner at one page read per
    /// element, no chain walking.
    Header,
    /// The whole record gathered and parsed, and its stored `NAME` kept.
    Named,
    /// The whole record gathered, parsed and handed over: members and every
    /// explicit attribute, reference-valued ones included.
    Full,
}

/// One element out of [`ReadOnlyEngine::scan_elements`].
#[derive(Debug, Clone)]
pub struct ScannedElement {
    /// The key the index enumerates — confirmed against the record's own
    /// header before this struct is handed out.
    pub refno: RefNo,
    /// Where the record lives: page number in the tree's extent, and the
    /// packed word offset an index leaf states.
    pub page_no: u32,
    pub offset_words: u32,
    pub noun_hash: u32,
    pub owner: RefNo,
    /// The stored `NAME`, from [`ScanTier::Named`] up. `None` on the many
    /// elements that store none — a positional display name is a rendering,
    /// not a stored fact, and a scan hands out stored facts.
    pub name: Option<String>,
    /// The parsed record, on [`ScanTier::Full`].
    pub parsed: Option<ParsedElement>,
}

/// A streaming pass over every element the selected session indexes.
///
/// Yields elements in key order, one `Result` each: a page or record that
/// cannot be read fails that element and the scan carries on, so the caller
/// decides whether one bad record poisons the whole pass.
pub struct ElementScan<'a> {
    cache: &'a mut PageCache,
    ext: u16,
    tier: ScanTier,
    entries: std::vec::IntoIter<IndexEntry>,
}

impl ElementScan<'_> {
    /// How many elements remain, the one about to be yielded included.
    pub fn remaining(&self) -> usize {
        self.entries.len()
    }

    fn read(&mut self, entry: &IndexEntry) -> Result<ScannedElement, E3dReaderError> {
        let confirm = |stated: RefNo| {
            if stated == entry.refno {
                Ok(())
            } else {
                Err(E3dReaderError::IndexRecordMismatch {
                    refno: entry.refno,
                    page_no: entry.page_no,
                    byte_offset: entry.offset_words * 2,
                    found: stated,
                })
            }
        };

        if self.tier == ScanTier::Header {
            let header = record::read_record_header(
                self.cache,
                PageId::new(self.ext, entry.page_no),
                entry.offset_words,
            )?;
            confirm(header.refno)?;
            return Ok(ScannedElement {
                refno: entry.refno,
                page_no: entry.page_no,
                offset_words: entry.offset_words,
                noun_hash: header.noun_hash,
                owner: header.owner,
                name: None,
                parsed: None,
            });
        }

        let view = record::read_element_record(self.cache, self.ext, entry)?;
        let parsed = ParsedElement::parse(&view.raw_bytes).map_err(|error| {
            E3dReaderError::Other(format!("{}: gathered record parses: {error}", entry.refno))
        })?;
        confirm(parsed.refno)?;
        Ok(ScannedElement {
            refno: entry.refno,
            page_no: entry.page_no,
            offset_words: entry.offset_words,
            noun_hash: parsed.noun_hash,
            owner: parsed.owner,
            name: element_name::stored_name(&parsed),
            parsed: (self.tier == ScanTier::Full).then_some(parsed),
        })
    }
}

impl Iterator for ElementScan<'_> {
    type Item = Result<ScannedElement, E3dReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        crate::read_stats::note_scanned_element();
        Some(self.read(&entry))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

/// Version-adjacent facts persisted in the database header and selected
/// session.
///
/// These fields deliberately carry no inferred [`crate::E3dVersion`]:
/// `CRINFO` names the creator, `TMPLVN` protects template compatibility, and
/// the DAB tag names the engine that wrote one session. None is, by itself, a
/// durable selector between the `e3d210` and `e3d31` SchemaPacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseHeaderDiagnostics {
    pub database: PathBuf,
    pub template_type_hash: u32,
    pub template_type_name: Option<String>,
    pub template_version: i32,
    pub creation_info: Option<String>,
    pub header_format: Option<u32>,
    pub session_page: PageId,
    pub session_number: u32,
    pub dabacon_version_tag: Option<String>,
}

/// Read-only engine for supported E3D 2.1/3.1 database files, as of one session.
pub struct ReadOnlyEngine {
    cache: PageCache,
    descriptor: DbDescriptor,
    file_info: FileInfo,
    path: PathBuf,
    /// The newest session, read once at open. Its index root is the only tree
    /// this engine descends.
    session: SessionPage,
}

/// Open a database read-only and return its descriptor/latest-session
/// diagnostics without scanning any element tree.
pub fn database_header_diagnostics(
    path: impl AsRef<Path>,
) -> Result<DatabaseHeaderDiagnostics, E3dReaderError> {
    Ok(ReadOnlyEngine::open(path)?.header_diagnostics())
}

impl ReadOnlyEngine {
    /// Open a database file for read-only access.
    ///
    /// Reads page 0 (descriptor), page 1 (file info) and the newest session
    /// page, which is where the index root comes from.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        Self::open_with_cache(path, 256)
    }

    pub fn open_with_cache(
        path: impl AsRef<Path>,
        cache_capacity: usize,
    ) -> Result<Self, E3dReaderError> {
        Self::open_selected(path, SessionSelector::Latest, cache_capacity)
    }

    /// Open as of one named session rather than the newest.
    ///
    /// The index is copy-on-write, so a session page carries the root as it
    /// stood then and opening "at session N" is the choice of which root to
    /// descend — see [`crate::session`]. A caller pinning a watermark wants
    /// exactly this: the tree as of the session whose data is known to have
    /// landed, not whatever the file has grown since.
    ///
    /// A session that is not in the chain is an `Err` naming it. Falling back
    /// to the newest would answer a different question than the one asked, and
    /// one level up that reads as "the element changed".
    ///
    /// Takes a bare session number or any [`SessionSelector`]:
    /// `open_at(path, 264)` and `open_at(path, SessionSelector::AtOrBefore(200))`
    /// both work.
    pub fn open_at(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
    ) -> Result<Self, E3dReaderError> {
        Self::open_selected(path, at.into(), 256)
    }

    /// The one open path: `open` is this with [`SessionSelector::Latest`].
    ///
    /// Session selection goes through [`crate::session::resolve`], the crate's
    /// only resolver, rather than a second one written here. It costs one page
    /// per session step, once per engine.
    fn open_selected(
        path: impl AsRef<Path>,
        at: SessionSelector,
        cache_capacity: usize,
    ) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        let reader = PageReader::open(path)?;
        let mut cache = PageCache::new(reader, cache_capacity);

        let page0 = cache.get_page(0)?;
        let descriptor = DbDescriptor::parse(page0)?;

        let page1 = cache.get_page(FileInfo::PAGE_INDEX)?;
        let file_info = FileInfo::parse_page1(page1)?;

        let (session, _sessions_read) = crate::session::resolve(&mut cache, &descriptor, at, path)?;
        crate::read_stats::note_database_open();

        Ok(Self {
            path: path.to_path_buf(),
            cache,
            descriptor,
            file_info,
            session,
        })
    }

    pub fn descriptor(&self) -> &DbDescriptor {
        &self.descriptor
    }

    /// Header diagnostics for the session this engine was opened at.
    ///
    /// [`Self::open`] reports the newest session. [`Self::open_at`] reports the
    /// selected historical session, including the DAB engine tag stored on
    /// that exact session page.
    pub fn header_diagnostics(&self) -> DatabaseHeaderDiagnostics {
        DatabaseHeaderDiagnostics {
            database: self.path.clone(),
            template_type_hash: self.descriptor.db_type_hash,
            template_type_name: self.descriptor.db_type_name(),
            template_version: self.descriptor.template_version,
            creation_info: self.descriptor.creation_info.clone(),
            header_format: self.descriptor.header_format,
            session_page: self.session.page,
            session_number: self.session.sesno,
            dabacon_version_tag: self.session.dabacon_version_tag.clone(),
        }
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

    /// Every session, newest first.
    ///
    /// Delegates to [`crate::session::walk_chain`] so there is one session
    /// reader in the crate. The hand-written walk this replaced started from
    /// the descriptor's `session_page_no` (+0x30) — the *oldest* session — read
    /// exactly one page, and never followed `last_ses_pgno`: on `ams8000_0001`
    /// it returned a one-element vector describing session 1 of 264, with no
    /// error to say so.
    pub fn sessions(&mut self) -> Result<Vec<SessionInfo>, E3dReaderError> {
        let chain = crate::session::walk_chain(&mut self.cache, &self.descriptor)?;
        Ok(chain.iter().map(SessionInfo::from).collect())
    }

    /// The session this engine answers as of, and the root it descends from.
    ///
    /// One root, taken from the session page, which is where the index root is
    /// written. The rule this replaces read every page in the file and kept
    /// the index pages no other index page named — so it found the current
    /// tree along with every tree left behind by an earlier session, because
    /// the index is copy-on-write and nothing points at the old ones.
    pub fn session(&self) -> &SessionPage {
        &self.session
    }

    pub fn index_root(&self) -> PageId {
        self.session.index_root
    }

    /// What changed between two sessions of this file.
    ///
    /// Independent of the session this engine was opened at: the two roots
    /// come off the chain, not off `self.session`. See
    /// [`crate::session::diff::diff_between`] for the semantics — a range is
    /// the net difference of its ends, and `from` must be the older one.
    pub fn diff_sessions(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<SessionDiff, E3dReaderError> {
        crate::session::diff::diff_between(
            &mut self.cache,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The same range one retained session at a time. See
    /// [`crate::session::diff::diff_steps`].
    pub fn diff_steps(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<Vec<SessionDiff>, E3dReaderError> {
        crate::session::diff::diff_steps(
            &mut self.cache,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The records behind a diff, read through this engine's cache. See
    /// [`SessionDiff::elements`].
    pub fn changed_elements<'a>(&'a mut self, diff: &'a SessionDiff) -> ChangedElements<'a> {
        diff.elements(&mut self.cache)
    }

    /// The named trees this session points at, beyond the refno index.
    ///
    /// See [`crate::index::reference`]: most of them are reverse-reference
    /// tables, one per reference attribute.
    pub fn tables(&self) -> &[TableRoot] {
        &self.session.tables
    }

    /// One reverse-reference table of the selected session, or `None` when the
    /// session names no table by that hash.
    ///
    /// `name_hash` is `db1_hash("SPRE")` and the like — the reference
    /// attribute, not the `SPBREF`-style pseudo-attribute that reads it back.
    pub fn reference_table(&mut self, name_hash: u32) -> Option<ReferenceTable<'_>> {
        let root = self.session.table_root(name_hash)?;
        Some(ReferenceTable::at_root(&mut self.cache, root, name_hash))
    }

    /// Everything **in this database** that references `target` through
    /// `name_hash`.
    ///
    /// An absent table answers empty: a database that never wrote through this
    /// attribute has no referrers to report, which is the same fact. A table
    /// that is there and cannot be read is an `Err`, never an empty answer.
    ///
    /// This is one database's half of the answer. `DB_MDB::findElements` runs
    /// the same search over every database of a type and unions the results,
    /// because a table is stored in the *referring* database — see the module
    /// note on [`crate::index::reference`].
    pub fn referrers_of(
        &mut self,
        name_hash: u32,
        target: RefNo,
    ) -> Result<Vec<RefNo>, E3dReaderError> {
        let Some(mut table) = self.reference_table(name_hash) else {
            return Ok(Vec::new());
        };
        table
            .referrers_of(target)
            .map_err(|source| E3dReaderError::Other(source.to_string()))
    }

    /// Find one RefNo in the index of the selected session.
    ///
    /// Delegates to [`IndexCursor`], which is the crate's only descent.
    pub fn search_index(&mut self, target: RefNo) -> Result<Option<SearchResult>, E3dReaderError> {
        crate::read_stats::note_index_point_lookup();
        let root = self.session.index_root;
        let found = IndexCursor::at_root(&mut self.cache, root)
            .seek(target)
            .map_err(|source| E3dReaderError::IndexError {
                page_no: root.page,
                detail: source.to_string(),
            })?
            .found;

        Ok(found.map(|record| SearchResult {
            entry: record.entry(),
            leaf_page_no: record.leaf.page,
        }))
    }

    /// Every RefNo the selected session indexes, in key order.
    ///
    /// Walks the one live tree. The scan this replaces read every page in the
    /// file and took the leaf entries of every index page it found, live or
    /// not: on the 429-database corpus that is 940 694 entries against the
    /// 789 831 the live trees hold, so 16% of what it returned was an address
    /// out of a tree no session can reach.
    pub fn indexed_refnos(&mut self) -> Result<Vec<(RefNo, u32, u32)>, E3dReaderError> {
        let root = self.session.index_root;
        let mut out = Vec::new();
        for found in IndexCursor::at_root(&mut self.cache, root).walk() {
            let found = found.map_err(|source| E3dReaderError::IndexError {
                page_no: root.page,
                detail: source.to_string(),
            })?;
            out.push((found.refno, found.loc.page.page, found.loc.offset_words));
        }
        Ok(out)
    }

    /// Every element the selected session indexes, read at the chosen depth.
    ///
    /// One walk of the live tree names the elements — the same walk
    /// [`Self::indexed_refnos`] makes, so the two cannot disagree about what
    /// exists — then each element's pages are read as the iterator is driven.
    /// Elements arrive in key order. This is the primitive a derived index is
    /// built from: the file states no index but the RefNo tree, so type, name
    /// and back-reference lookups all start as one pass over everything.
    ///
    /// The entry list is collected up front (24 bytes an element, ~19 MiB for
    /// the largest corpus file), so the tree is not descended per element and
    /// the scan's page reads run in tree order.
    pub fn scan_elements(&mut self, tier: ScanTier) -> Result<ElementScan<'_>, E3dReaderError> {
        crate::read_stats::note_scan_call();
        let root = self.session.index_root;
        let mut entries = Vec::new();
        for found in IndexCursor::at_root(&mut self.cache, root).walk() {
            let found = found.map_err(|source| E3dReaderError::IndexError {
                page_no: root.page,
                detail: source.to_string(),
            })?;
            entries.push(found.entry());
        }
        Ok(ElementScan {
            cache: &mut self.cache,
            ext: root.ext,
            tier,
            entries: entries.into_iter(),
        })
    }

    /// Find an element by RefNo: one descent, then the record it addresses.
    ///
    /// `Ok(None)` means the session does not index that RefNo. A record that
    /// answers to a different name is an error, not a miss — across the 429
    /// databases the index was settled on, none of the 789 831 keys does.
    ///
    /// Three fallbacks used to sit under this call, and all three were paying
    /// for the wrong root. It searched every tree in the file, kept the copy
    /// furthest into it, and if nothing confirmed, read the whole file
    /// backwards looking for the RefNo as a byte pattern. The eleven leaves
    /// `ams1112_0001` has for 17496/950, ten of them addressing an unrelated
    /// element, are ten dead trees and one live one.
    pub fn find_element(
        &mut self,
        refno: RefNo,
    ) -> Result<Option<ElementRecordView>, E3dReaderError> {
        let Some(found) = self.search_index(refno)? else {
            return Ok(None);
        };
        // The entry names a page but not a file. Every page of the descended
        // tree inherits the root's extent — the rule `RecordLoc` states — so
        // the root's extent is the entry's.
        let ext = self.session.index_root.ext;
        let view = record::read_element_record(&mut self.cache, ext, &found.entry)?;
        let stated = RefNo::new(
            u32::from_be_bytes(view.raw_bytes[4..8].try_into().expect("record header")),
            u32::from_be_bytes(view.raw_bytes[8..12].try_into().expect("record header")),
        );
        if stated != refno {
            return Err(E3dReaderError::IndexRecordMismatch {
                refno,
                page_no: found.entry.page_no,
                byte_offset: found.entry.offset_words * 2,
                found: stated,
            });
        }
        Ok(Some(view))
    }

    /// The database's WORLD element, by point lookup — no element scan.
    ///
    /// Looks up [`DbDescriptor::world_refno`] (`primary_extent_ref / 0`) in
    /// this session's index and confirms the record really is a `WORL` with a
    /// null owner. `Ok(None)` means the index has no element at that address
    /// (the three project-administration database types keep their WORLD
    /// elsewhere; see [`DbDescriptor::world_refno`]). An element *is* there
    /// but is not a `WORL` is data this library does not understand and is
    /// reported as [`E3dReaderError::NotAWorld`] rather than guessed around.
    ///
    /// This is the tree-root entry point that replaces "scan the database for
    /// the `WORL` noun": one B+tree descent and one record read per database.
    pub fn world(&mut self) -> Result<Option<ElementRecordView>, E3dReaderError> {
        let refno = self.descriptor.world_refno();
        let Some(view) = self.find_element(refno)? else {
            return Ok(None);
        };
        let header = view.header();
        if header.noun_hash != e3d_attlib::db1_hash("WORL") || header.owner.is_valid() {
            return Err(E3dReaderError::NotAWorld {
                refno,
                noun_hash: header.noun_hash,
                owner: header.owner,
            });
        }
        Ok(Some(view))
    }

    /// Read the record at an address the caller already has.
    ///
    /// A bare page number names a page of the primary extent; a caller who
    /// has an address in another extent has a `PageId` saying so and
    /// [`record::read_element_record`] takes it directly.
    ///
    /// The RefNo is carried into the view unchecked, so a caller probing a
    /// location can still see which element is really there.
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
        record::read_element_record(&mut self.cache, PageId::PRIMARY, &entry)
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

    /// Decode a record the caller already holds. Delegates to the free
    /// function [`extract_parsed_element_with_descriptors`]; nothing here needs
    /// the engine.
    pub fn extract_parsed_element_with_descriptors(
        &self,
        refno: RefNo,
        parsed: &record::element::ParsedElement,
        attlib: &e3d_attlib::AttlibData,
        template: &record::template::NounTemplate,
    ) -> Result<record::descriptor::ElementExtraction, ResolveError> {
        extract_parsed_element_with_descriptors(refno, parsed, attlib, template)
    }

    fn summarize_parsed_element_with_template(
        &self,
        refno: RefNo,
        parsed: &record::element::ParsedElement,
        attlib: &e3d_attlib::AttlibData,
        template: &record::template::NounTemplate,
    ) -> Result<ElementSummary, ResolveError> {
        let extraction = extract_parsed_element_with_descriptors(refno, parsed, attlib, template)?;
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

/// Decode every descriptor-backed attribute of a parsed record.
///
/// Pure: the record, the schema and the template are all in hand, so no page
/// is read and no engine is needed. [`ReadOnlyEngine::extract_element_with_descriptors`]
/// finds and parses the record first and then comes here;
/// [`crate::session::delta`] comes here with the two records of a changed
/// element, which it read through a diff rather than through a lookup.
pub fn extract_parsed_element_with_descriptors(
    refno: RefNo,
    parsed: &record::element::ParsedElement,
    attlib: &e3d_attlib::AttlibData,
    template: &record::template::NounTemplate,
) -> Result<record::descriptor::ElementExtraction, ResolveError> {
    {
        if template.noun_hash != parsed.noun_hash {
            return Err(ResolveError::TemplateNounMismatch {
                expected: parsed.noun_hash,
                actual: template.noun_hash,
            });
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
                            descriptor.hash,
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
    attribute_hash: u32,
) -> record::descriptor::DescriptorValue {
    use record::descriptor::DescriptorValue;
    use record::explicit::ExplicitValue;
    match value {
        // Explicit one-word angles carry the same signed centidegrees as
        // implicit words (descriptor::decode_value). A missing ATGTDF entry
        // must not erase the catalog's ANGL unit, as it did for RAIL.BANG.
        ExplicitValue::Integer(value)
            if storage_type_code == 3
                && e3d_attlib::is_angle_attr(attribute_hash)
                && !def.is_some_and(|def| matches!(def.data_type, 1 | 3)) =>
        {
            DescriptorValue::Real(f64::from(*value) / 100.0)
        }
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
    fn authoritative_zero_attribute_noun_extracts_as_empty() {
        let noun_hash = 0x000B_FA34;
        let parsed = record::element::ParsedElement {
            impl_len_words: 6,
            refno: RefNo::new(24_575, 13),
            noun_hash,
            owner: RefNo::new(24_575, 1),
            prefix_offset: 0,
            implicit_data: vec![0; 24],
            members: Vec::new(),
            explicit_attributes: Vec::new(),
            record_end: 24,
            total_size: 24,
        };

        let extraction = extract_parsed_element_with_descriptors(
            parsed.refno,
            &parsed,
            &e3d_attlib::AttlibData::default(),
            &record::template::NounTemplate::new(noun_hash),
        )
        .expect("an authoritative zero-attribute template is a complete description");

        assert_eq!(extraction.noun_hash, noun_hash);
        assert_eq!(extraction.noun_name.as_deref(), Some("DTWL"));
        assert_eq!(extraction.implicit_word_count, 6);
        assert_eq!(extraction.format_word_10, 0);
        assert!(extraction.attributes.is_empty());
    }

    #[test]
    fn explicit_integer_uses_word_storage_semantics_without_attlib_definition() {
        let value = explicit_value_to_descriptor_value(
            &record::explicit::ExplicitValue::Integer(914_199),
            None,
            3,
            0,
        );

        assert_eq!(
            value,
            record::descriptor::DescriptorValue::Word {
                raw: 914_199,
                text: Some("FALS".into()),
            }
        );
    }

    #[test]
    fn explicit_word_angle_uses_the_same_units_as_implicit_storage() {
        use record::descriptor::DescriptorValue;
        use record::explicit::ExplicitValue;
        for (raw, expected) in [(0, 0.0), (-9000, -90.0), (1150, 11.5)] {
            let value = explicit_value_to_descriptor_value(
                &ExplicitValue::Integer(raw),
                None,
                3,
                e3d_attlib::db1_hash("BANG"),
            );
            assert_eq!(value, DescriptorValue::Real(expected));
        }
        assert_eq!(
            explicit_value_to_descriptor_value(
                &ExplicitValue::Real(11.5),
                None,
                3,
                e3d_attlib::db1_hash("BANG")
            ),
            DescriptorValue::Real(11.5)
        );
        assert!(matches!(
            explicit_value_to_descriptor_value(
                &ExplicitValue::Integer(-9000),
                None,
                3,
                e3d_attlib::db1_hash("JUSL")
            ),
            DescriptorValue::Word { .. }
        ));
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
