//! L1: the session chain, and opening the database as of one session.
//!
//! The index is copy-on-write — a page, once written, is never modified — so
//! **every session page carries the index root as it stood at that session**.
//! Opening "at session N" is therefore not a filter applied after the fact: it
//! is the choice of which root to descend from. That is also what makes
//! two-root differencing possible later.
//!
//! Two rules hold this module together:
//!
//! * `open` and `open_at` are one implementation. `open` is `open_at` asking
//!   for the chain tail, so there is no separate "latest" path that could drift
//!   from the "historical" one.
//! * A session that is not there is an `Err` naming it. The reader this
//!   replaces answered `None` for a missing session, a malformed session page
//!   and an unreadable header alike — and one level up, `None` reads as "that
//!   refno does not exist".

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::{E3dReaderError, SessionError};
use crate::meta::DbDescriptor;
use crate::page::cache::PageCache;
use crate::page::io::PageReader;
use crate::page::{IoStats, PageId, PageType};

pub mod delta;
pub mod diff;

pub use delta::{
    AttributeRowChange, ElementDelta, ExplicitChange, ImplicitWordChange, OwnerChange,
};
pub use diff::{ChangeKind, ChangedElement, ChangedElements, SessionDiff};

/// Byte offsets inside a session page, all big-endian words.
mod at {
    pub const PAGE_TYPE: usize = 0x00;
    pub const LAST_SES_PGNO: usize = 0x04;
    pub const LAST_SES_EXTNO: usize = 0x08;
    pub const SESNO: usize = 0x0C;
    pub const RESERVED_10: usize = 0x10;
    pub const END_PGNO: usize = 0x14;
    pub const END_EXTNO: usize = 0x18;
    pub const INDEX_ROOT_PGNO: usize = 0x1C;
    pub const INDEX_ROOT_EXTNO: usize = 0x20;
    pub const CLAIM_PGNO: usize = 0x24;
    pub const CLAIM_EXTNO: usize = 0x28;
    /// First word of the timestamp region, whose encoding is not closed.
    pub const STAMP: usize = 0x34;
    pub const STAMP_WORDS: usize = 5;
    /// NUL-padded ASCII, runs to [`COMMENT_LEN`].
    pub const MACHINE: usize = 0x7C;
    /// Comment length, counted in 4-byte words.
    pub const COMMENT_LEN: usize = 0xA0;
    pub const COMMENT_TEXT: usize = 0xA4;
    /// Everything through the comment length word must be present.
    pub const MIN_BYTES: usize = COMMENT_TEXT;
}

/// A session page marks the chain head by pointing nowhere.
///
/// `ams8000_0001` page 4 is session 1 and states `0xFFFFFFFF`. Page 0 is the
/// descriptor and can never be a session page, so 0 ends the chain too.
const NO_PREVIOUS: [u32; 2] = [u32::MAX, 0];

/// One session page: what the database looked like when that session closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPage {
    /// Where this page lives.
    pub page: PageId,
    /// Session number, +0x0C.
    pub sesno: u32,
    /// The session before this one, or `None` at the head of the chain.
    pub previous: Option<PageId>,
    /// +0x10 verbatim. Reads `0xFFFFFFFF` on every sampled session page, so
    /// whatever it is, it is not a page number. Carried only because
    /// [`SessionInfo::start_page_no`] has been reporting it for years.
    pub reserved_10: u32,
    /// Last page the session wrote, +0x14 / +0x18.
    pub end: PageId,
    /// **Root of the index as of this session**, +0x1C / +0x20.
    pub index_root: PageId,
    /// Claim page, +0x24 / +0x28.
    pub claim: PageId,
    /// Machine that closed the session, +0x7C, NUL-padded ASCII.
    pub machine: String,
    /// Session comment, +0xA4, length at +0xA0 counted in 4-byte words.
    pub comment: String,
    /// Words +0x34..+0x48 verbatim.
    ///
    /// The first two decode as a year and a month (2026, 8 across the sampled
    /// pages) and +0x40 rises monotonically with session number, but the rest
    /// of the encoding is **not closed**, so the words are handed over as they
    /// are rather than turned into a date this library cannot justify.
    pub stamp_words: [u32; at::STAMP_WORDS],
}

impl SessionPage {
    /// Parse a session page, rejecting anything that is not one.
    pub fn parse(page: PageId, data: &[u8]) -> Result<Self, SessionError> {
        if data.len() < at::MIN_BYTES {
            return Err(SessionError::PageTooSmall {
                page,
                len: data.len(),
                need: at::MIN_BYTES,
            });
        }
        let r = |off: usize| u32::from_be_bytes(data[off..off + 4].try_into().unwrap());

        let page_type = r(at::PAGE_TYPE);
        if PageType::from_raw(page_type) != PageType::Session {
            return Err(SessionError::NotASessionPage { page, page_type });
        }

        let last_pgno = r(at::LAST_SES_PGNO);
        let previous = if NO_PREVIOUS.contains(&last_pgno) {
            None
        } else {
            Some(address(
                page,
                "previous session",
                r(at::LAST_SES_EXTNO),
                last_pgno,
            )?)
        };

        let mut stamp_words = [0u32; at::STAMP_WORDS];
        for (i, word) in stamp_words.iter_mut().enumerate() {
            *word = r(at::STAMP + i * 4);
        }

        Ok(Self {
            page,
            sesno: r(at::SESNO),
            previous,
            reserved_10: r(at::RESERVED_10),
            end: address(page, "end", r(at::END_EXTNO), r(at::END_PGNO))?,
            index_root: address(
                page,
                "index root",
                r(at::INDEX_ROOT_EXTNO),
                r(at::INDEX_ROOT_PGNO),
            )?,
            claim: address(page, "claim", r(at::CLAIM_EXTNO), r(at::CLAIM_PGNO))?,
            machine: ascii_field(&data[at::MACHINE..at::COMMENT_LEN]),
            comment: comment(data, r(at::COMMENT_LEN)),
            stamp_words,
        })
    }
}

/// Which session a caller wants.
///
/// `open` passes [`SessionSelector::Latest`] into the same `open_at` that a
/// caller naming a session number reaches, so there is exactly one resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSelector {
    /// The newest session, i.e. the tail of the chain.
    Latest,
    /// One named session. Absent means `Err`, never a fallback to latest.
    Exact(u32),
    /// The newest retained session whose number is `<= n`.
    ///
    /// `MERGE CHANGES` drops sessions from the chain — `ams7000_0001` keeps
    /// only 1, 237, 238, 262 and 263 — so a downstream that recorded "I have
    /// everything up to session 200" cannot ask for 200 exactly. It wants the
    /// diff from the newest session that is not past its watermark, which on
    /// that file is session 1: everything since then is re-reported rather
    /// than the changes in 201..236, which were folded into 237, being lost.
    /// The session actually chosen is always reported back, never silent.
    ///
    /// Resolved as the first session the newest-first walk reaches whose
    /// number is `<= n`, which is the newest such session on a chain whose
    /// numbers descend — every chain of the 453-file corpus does.
    AtOrBefore(u32),
}

impl SessionSelector {
    /// The number the selector names, if it names one.
    pub const fn sesno(self) -> Option<u32> {
        match self {
            Self::Latest => None,
            Self::Exact(sesno) | Self::AtOrBefore(sesno) => Some(sesno),
        }
    }
}

impl From<u32> for SessionSelector {
    fn from(sesno: u32) -> Self {
        Self::Exact(sesno)
    }
}

impl From<SessionSelector> for Option<u32> {
    fn from(selector: SessionSelector) -> Self {
        selector.sesno()
    }
}

/// A database opened as of one session.
///
/// Holds the page source, the file header and the chosen session page. What
/// makes it a *view* rather than a handle is [`DbView::index_root`]: two views
/// of the same file at different sessions descend different trees, and neither
/// is more current than the file says it is.
pub struct DbView {
    pages: PageCache,
    descriptor: DbDescriptor,
    session: SessionPage,
    sessions_read: u32,
    /// Kept for error messages: a session that is not there is named together
    /// with the file it is not in.
    path: PathBuf,
}

/// Names the session and the root, which is what tells two views apart.
impl std::fmt::Debug for DbView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbView")
            .field("db_num", &self.descriptor.db_mark)
            .field("sesno", &self.session.sesno)
            .field("index_root", &self.session.index_root)
            .field("sessions_read", &self.sessions_read)
            .finish_non_exhaustive()
    }
}

impl DbView {
    /// Pages held while reading. Matches `ReadOnlyEngine`'s default.
    pub const DEFAULT_CACHE_PAGES: usize = 256;

    /// Open at the newest session.
    ///
    /// Deliberately nothing but a call to [`DbView::open_at`]: a second
    /// "latest" path is how a reader ends up with two answers to the same
    /// question.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        Self::open_at(path, SessionSelector::Latest)
    }

    /// Open as of a named session, or of the newest one.
    ///
    /// `open_at(path, 264)` and `open_at(path, SessionSelector::Latest)` both
    /// work; a `u32` converts into [`SessionSelector::Exact`].
    pub fn open_at(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
    ) -> Result<Self, E3dReaderError> {
        Self::open_at_with_cache(path, at, Self::DEFAULT_CACHE_PAGES)
    }

    pub fn open_at_with_cache(
        path: impl AsRef<Path>,
        at: impl Into<SessionSelector>,
        cache_pages: usize,
    ) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        let mut pages = PageCache::new(PageReader::open(path)?, cache_pages);

        let descriptor = DbDescriptor::parse(pages.get_page(DbDescriptor::PAGE_INDEX)?)?;
        let (session, sessions_read) = resolve(&mut pages, &descriptor, at.into(), path)?;

        Ok(Self {
            pages,
            descriptor,
            session,
            sessions_read,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What changed between two sessions of this file: see
    /// [`diff::diff_between`], which this delegates to without adding anything.
    ///
    /// Both selectors are resolved on one walk of the chain; `from` older than
    /// `to` is the only accepted order, and either being absent is an error
    /// naming the retained sessions either side of it.
    pub fn diff_sessions(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<SessionDiff, E3dReaderError> {
        diff::diff_between(
            &mut self.pages,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The same range, one retained session at a time: see [`diff::diff_steps`].
    pub fn diff_steps(
        &mut self,
        from: impl Into<SessionSelector>,
        to: impl Into<SessionSelector>,
    ) -> Result<Vec<SessionDiff>, E3dReaderError> {
        diff::diff_steps(
            &mut self.pages,
            &self.descriptor,
            &self.path,
            from.into(),
            to.into(),
        )
    }

    /// The records behind a diff, read through this view's page cache. See
    /// [`SessionDiff::elements`].
    pub fn changed_elements<'a>(&'a mut self, diff: &'a SessionDiff) -> ChangedElements<'a> {
        diff.elements(&mut self.pages)
    }

    /// The session actually in use — never a guess, never a fallback.
    pub fn selected_session(&self) -> u32 {
        self.session.sesno
    }

    /// Root of the index as of [`DbView::selected_session`].
    pub fn index_root(&self) -> PageId {
        self.session.index_root
    }

    pub fn session(&self) -> &SessionPage {
        &self.session
    }

    pub fn descriptor(&self) -> &DbDescriptor {
        &self.descriptor
    }

    /// Session pages read while opening: 1 for the newest, one more per step
    /// back. Also visible as demand reads in [`DbView::stats`].
    pub fn sessions_read(&self) -> u32 {
        self.sessions_read
    }

    pub fn stats(&self) -> IoStats {
        self.pages.stats()
    }

    pub fn pages(&self) -> &PageCache {
        &self.pages
    }

    pub fn pages_mut(&mut self) -> &mut PageCache {
        &mut self.pages
    }

    /// Every session, newest first. See [`walk_chain`].
    pub fn walk_sessions(&mut self) -> Result<Vec<SessionPage>, E3dReaderError> {
        walk_chain(&mut self.pages, &self.descriptor).map_err(Into::into)
    }
}

/// The chain, one session at a time, newest first.
///
/// The only chain traversal in the crate: [`walk_chain`], [`resolve`] and
/// [`resolve_pair`] all drive one of these, so "which pointer does the chain
/// start from" and "when is a chain broken" each have one answer. It starts
/// from `latest_ses_pgno` (+0x28). The hand-written walk this replaced started
/// from +0x30, which is the *oldest* session, and never followed
/// `last_ses_pgno` — on a 264-session database it returned one entry, for
/// session 1, and reported success.
///
/// A chain cannot be longer than the file has pages, and cannot revisit a
/// page: `last_ses_pgno` pointing at itself used to spin forever. Both are
/// errors here, not silent stops.
///
/// Costs one page read per session, all of them demand reads, so the price
/// shows up in `IoStats::pages_read`.
pub struct ChainWalk<'a> {
    pages: &'a mut PageCache,
    next: Option<PageId>,
    seen: HashSet<PageId>,
    steps: u32,
    limit: u32,
}

impl<'a> ChainWalk<'a> {
    pub fn new(pages: &'a mut PageCache, descriptor: &DbDescriptor) -> Self {
        let limit = pages.physical_page_count().max(1);
        Self {
            pages,
            next: Some(descriptor.latest_session_page()),
            seen: HashSet::new(),
            steps: 0,
            limit,
        }
    }

    /// The next session back, or `None` once the chain head has been passed.
    pub fn next_session(&mut self) -> Result<Option<SessionPage>, SessionError> {
        let Some(page) = self.next else {
            return Ok(None);
        };
        if !self.seen.insert(page) {
            return Err(SessionError::ChainCycle {
                revisited: page,
                after: self.steps,
            });
        }
        if self.steps >= self.limit {
            return Err(SessionError::ChainTooLong {
                steps: self.steps,
                limit: self.limit,
            });
        }
        let parsed = read_session(self.pages, page)?;
        self.next = parsed.previous;
        self.steps += 1;
        Ok(Some(parsed))
    }

    /// Session pages read so far.
    pub fn steps(&self) -> u32 {
        self.steps
    }
}

/// Walk the whole chain from the newest session back to the oldest.
///
/// [`DbView::walk_sessions`] and `ReadOnlyEngine::sessions` both come here.
pub fn walk_chain(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
) -> Result<Vec<SessionPage>, SessionError> {
    let mut walk = ChainWalk::new(pages, descriptor);
    let mut out = Vec::new();
    while let Some(session) = walk.next_session()? {
        out.push(session);
    }
    Ok(out)
}

/// The newest session, in one page read.
///
/// The chain head is where the descriptor says it is; walking to find it, as
/// [`walk_chain`] does, costs a page per session and answers the same.
pub fn latest(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
) -> Result<SessionPage, SessionError> {
    read_session(pages, descriptor.latest_session_page())
}

/// The one place a selector becomes a session page.
///
/// Returns the session and how many session pages had to be read to reach it.
pub fn resolve(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    want: SessionSelector,
    path: &Path,
) -> Result<(SessionPage, u32), SessionError> {
    let ([session], read) = resolve_many(pages, descriptor, [want], path)?;
    Ok((session, read))
}

/// Two selectors, one walk.
///
/// A range diff names two sessions; resolving them one after the other would
/// walk the chain twice and, worse, would be a second resolver. Returns the
/// pages in the order the selectors were given, and the session pages read.
pub fn resolve_pair(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    first: SessionSelector,
    second: SessionSelector,
    path: &Path,
) -> Result<([SessionPage; 2], u32), SessionError> {
    resolve_many(pages, descriptor, [first, second], path)
}

/// One selector part-way through a walk.
struct Pending {
    want: SessionSelector,
    found: Option<SessionPage>,
    /// Smallest sesno seen that is above the wanted number.
    nearest_newer: Option<u32>,
    /// Largest sesno seen that is below the wanted number.
    nearest_older: Option<u32>,
}

impl Pending {
    fn new(want: SessionSelector) -> Self {
        Self {
            want,
            found: None,
            nearest_newer: None,
            nearest_older: None,
        }
    }

    /// Offer one session of the walk, newest first.
    fn offer(&mut self, session: &SessionPage) {
        if self.found.is_some() {
            return;
        }
        match self.want {
            SessionSelector::Latest => self.found = Some(session.clone()),
            SessionSelector::Exact(wanted) => {
                if session.sesno == wanted {
                    self.found = Some(session.clone());
                } else {
                    self.note_neighbour(session.sesno, wanted);
                }
            }
            SessionSelector::AtOrBefore(wanted) => {
                if session.sesno <= wanted {
                    self.found = Some(session.clone());
                } else {
                    self.note_neighbour(session.sesno, wanted);
                }
            }
        }
    }

    fn note_neighbour(&mut self, seen: u32, wanted: u32) {
        if seen > wanted {
            self.nearest_newer = Some(self.nearest_newer.map_or(seen, |n| n.min(seen)));
        } else if seen < wanted {
            self.nearest_older = Some(self.nearest_older.map_or(seen, |n| n.max(seen)));
        }
    }
}

/// Resolve every selector on one newest-first walk, stopping as soon as all of
/// them have an answer.
///
/// `Exact(n)` with `n` above the newest session is refused after the first
/// page, without walking; that is the only shortcut. Anything else that is not
/// on the chain is reported after the whole chain has been read, with the
/// oldest session reached and the retained sessions either side of the
/// request.
fn resolve_many<const N: usize>(
    pages: &mut PageCache,
    descriptor: &DbDescriptor,
    wants: [SessionSelector; N],
    path: &Path,
) -> Result<([SessionPage; N], u32), SessionError> {
    let mut pending: Vec<Pending> = wants.into_iter().map(Pending::new).collect();
    let mut walk = ChainWalk::new(pages, descriptor);

    let newest = walk
        .next_session()?
        .expect("a chain walk always starts at the descriptor's newest session");
    let newest_sesno = newest.sesno;

    for want in &pending {
        if let SessionSelector::Exact(wanted) = want.want {
            if wanted > newest_sesno {
                return Err(SessionError::NoSuchSession {
                    path: path.to_path_buf(),
                    requested: wanted,
                    newest: newest_sesno,
                    oldest: None,
                    nearest_older: Some(newest_sesno),
                    nearest_newer: None,
                });
            }
        }
    }

    let mut current = Some(newest);
    let mut oldest_reached = newest_sesno;
    while let Some(session) = current {
        oldest_reached = session.sesno;
        for want in pending.iter_mut() {
            want.offer(&session);
        }
        if pending.iter().all(|want| want.found.is_some()) {
            break;
        }
        current = walk.next_session()?;
    }

    let read = walk.steps();
    let mut resolved = Vec::with_capacity(N);
    for want in pending {
        match want.found {
            Some(session) => resolved.push(session),
            None => {
                return Err(SessionError::NoSuchSession {
                    path: path.to_path_buf(),
                    requested: want
                        .want
                        .sesno()
                        .expect("Latest always resolves to the newest session"),
                    newest: newest_sesno,
                    oldest: Some(oldest_reached),
                    nearest_older: want.nearest_older,
                    nearest_newer: want.nearest_newer,
                })
            }
        }
    }
    let resolved: [SessionPage; N] = resolved
        .try_into()
        .unwrap_or_else(|_| unreachable!("one resolved session per selector"));
    Ok((resolved, read))
}

fn read_session(pages: &mut PageCache, page: PageId) -> Result<SessionPage, SessionError> {
    let data = pages.get(page).map_err(|source| SessionError::Unreadable {
        page,
        detail: source.to_string(),
    })?;
    SessionPage::parse(page, data)
}

/// A `(pageno, extno)` pair from a session page.
///
/// An extent of 0 means "the same file", which is how every sampled database
/// writes an address inside its own extent. An extent that does not fit in a
/// u16 is refused: falling back to the page's own extent, as this used to,
/// silently redirects the pointer into a different file, and no caller can
/// tell a redirected address from a stated one.
fn address(
    page: PageId,
    field: &'static str,
    extno: u32,
    pageno: u32,
) -> Result<PageId, SessionError> {
    let ext = match extno {
        0 => page.ext,
        stated => u16::try_from(stated).map_err(|_| SessionError::ExtentOutOfRange {
            page,
            field,
            extno,
        })?,
    };
    Ok(PageId::new(ext, pageno))
}

/// NUL-padded printable ASCII, as far as it goes.
fn ascii_field(bytes: &[u8]) -> String {
    bytes
        .iter()
        .copied()
        .take_while(|&b| (0x20..0x7F).contains(&b))
        .map(char::from)
        .collect()
}

/// The comment, whose length word counts 4-byte words rather than bytes.
///
/// `ams8000_0001` states 16 there and holds a 61-character comment NUL-padded
/// to 64 bytes; the byte just past `4 * 16` is NUL on every sampled page.
fn comment(data: &[u8], len_words: u32) -> String {
    let end = at::COMMENT_TEXT
        .saturating_add(len_words as usize * 4)
        .min(data.len());
    match data.get(at::COMMENT_TEXT..end) {
        Some(text) => ascii_field(text),
        None => String::new(),
    }
}

/// Session page fields under their pre-identification names.
///
/// Kept only because callers outside this module read them; three of the five
/// names are wrong and each says so on its own line. New code should take
/// [`SessionPage`], which names the fields after what they hold.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// **Actually +0x04 `last_ses_pgno`** — the *previous* session's page, not
    /// a mark. Renaming waits for L4.
    ///
    /// Normalised: a chain head reports `0xFFFFFFFF`, which is what every
    /// sampled head writes.
    pub session_mark: u32,
    /// **Actually +0x08 `last_ses_extno`** — the previous session's extent,
    /// not a handle. Renaming waits for L4.
    pub db_handle: u32,
    /// +0x0C `sesno`. This one is right.
    pub session_id: u32,
    /// **Actually +0x10, which reads `0xFFFFFFFF` on every sampled session
    /// page** — it has never held a start page, and nobody had looked at the
    /// value. Renaming waits for L4.
    pub start_page_no: u32,
    /// +0x14 `end_pgno`. This one is right.
    pub end_page_no: u32,
}

impl From<&SessionPage> for SessionInfo {
    fn from(page: &SessionPage) -> Self {
        Self {
            session_mark: page.previous.map_or(u32::MAX, |p| p.page),
            db_handle: page.previous.map_or(0, |p| u32::from(p.ext)),
            session_id: page.sesno,
            start_page_no: page.reserved_10,
            end_page_no: page.end.page,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session page 8897 of `ams8000_0001`, word for word up to the comment.
    fn ams8000_latest() -> Vec<u8> {
        let mut page = vec![0u8; 2048];
        let words: &[(usize, u32)] = &[
            (0x00, 3),
            (0x04, 8884),
            (0x08, 1),
            (0x0C, 264),
            (0x10, u32::MAX),
            (0x14, 8899),
            (0x18, 1),
            (0x1C, 8890),
            (0x20, 1),
            (0x24, 8898),
            (0x28, 1),
            (0x34, 2026),
            (0x38, 8),
            (0xA0, 3),
        ];
        for (off, value) in words {
            page[*off..*off + 4].copy_from_slice(&value.to_be_bytes());
        }
        page[0x7C..0x7F].copy_from_slice(b"dpc");
        page[0xA4..0xAD].copy_from_slice(b"CODEX db8");
        page
    }

    #[test]
    fn a_session_page_carries_its_own_index_root() {
        let parsed = SessionPage::parse(PageId::new(1, 8897), &ams8000_latest()).unwrap();
        assert_eq!(parsed.sesno, 264);
        assert_eq!(parsed.index_root, PageId::new(1, 8890));
        assert_eq!(parsed.previous, Some(PageId::new(1, 8884)));
        assert_eq!(parsed.end, PageId::new(1, 8899));
        assert_eq!(parsed.claim, PageId::new(1, 8898));
        assert_eq!(parsed.machine, "dpc");
        assert_eq!(parsed.comment, "CODEX db8");
        assert_eq!(parsed.stamp_words[0], 2026);
    }

    #[test]
    fn the_oldest_session_ends_the_chain() {
        let mut page = ams8000_latest();
        page[0x04..0x08].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            SessionPage::parse(PageId::new(1, 4), &page)
                .unwrap()
                .previous,
            None
        );

        page[0x04..0x08].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(
            SessionPage::parse(PageId::new(1, 4), &page)
                .unwrap()
                .previous,
            None
        );
    }

    #[test]
    fn a_page_that_is_not_a_session_is_refused_not_returned_empty() {
        let mut page = ams8000_latest();
        page[0x00..0x04].copy_from_slice(&5u32.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(err, SessionError::NotASessionPage { page_type: 5, .. }),
            "{err}"
        );

        let err = SessionPage::parse(PageId::new(1, 8897), &[0u8; 32]).unwrap_err();
        assert!(matches!(err, SessionError::PageTooSmall { .. }), "{err}");
    }

    /// **Reverting to the fallback turns this red.** An extent past `u16::MAX`
    /// used to become the page's own extent, so a corrupt pointer read as a
    /// valid address in the wrong file.
    #[test]
    fn an_extent_past_u16_is_refused_not_redirected() {
        let mut page = ams8000_latest();
        page[0x08..0x0C].copy_from_slice(&0x1_0000u32.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::ExtentOutOfRange {
                    field: "previous session",
                    extno: 0x1_0000,
                    ..
                }
            ),
            "{err}"
        );

        let mut page = ams8000_latest();
        page[0x20..0x24].copy_from_slice(&u32::MAX.to_be_bytes());
        let err = SessionPage::parse(PageId::new(1, 8897), &page).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::ExtentOutOfRange {
                    field: "index root",
                    extno: u32::MAX,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_u32_reads_as_an_exact_selector() {
        assert_eq!(SessionSelector::from(264u32), SessionSelector::Exact(264));
        assert_eq!(Option::<u32>::from(SessionSelector::Latest), None);
        assert_eq!(Option::<u32>::from(SessionSelector::Exact(7)), Some(7));
    }
}
