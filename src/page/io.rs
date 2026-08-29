//! L0: turning a `(extent, page)` address into bytes.
//!
//! Three defects in the reader this replaces all lived in this file, and each
//! one is now answered by a type or an error rather than by care:
//!
//! | defect | answer |
//! |---|---|
//! | the +0x34 page size field counts words, was read as bytes | [`PageSize`] stores words; `bytes()` is the only conversion |
//! | an unusable page size fell back to `DEFAULT_PAGE_SIZE` | [`PageSize::from_header_words`] returns `Err`, with no fallback |
//! | `PageId.ext` was carried but never used to pick a file | [`ExtentSet::file`] resolves it, and an absent extent is [`PageIoError::MissingExtent`] |

use std::fs::File;
use std::path::{Path, PathBuf};

use super::{IoCounters, IoStats, PageId, PageSize, PageSource, ReadIntent};
use crate::error::{E3dReaderError, PageIoError};
use crate::meta::constants::MIN_FILE_SIZE;

/// Bytes of file header the opener needs; the page size field ends at 0x38.
const HEADER_PROBE_BYTES: usize = 0x40;

/// Offset of the page size field, counted in 4-byte words.
const PAGE_SIZE_WORD_OFFSET: usize = 0x34;

/// One opened extent file.
#[derive(Debug)]
struct Extent {
    ext: u16,
    path: PathBuf,
    file: File,
    len: u64,
}

/// The extent files this reader may address.
///
/// It holds exactly the files that were opened. Resolving an extent that is not
/// in here is an error, never a read of the primary file: `PageId { ext: 2, page: N }`
/// used to come back with extent 1's page N and an `Ok`.
#[derive(Debug)]
pub struct ExtentSet {
    extents: Vec<Extent>,
    primary: u16,
    page_size: PageSize,
    naming: Option<ExtentNaming>,
}

/// How sibling extent files are named, when the primary follows the convention.
///
/// Dabacon files are `<stem>_<extent:04>`, e.g. `ams8000_0001`. Only used to
/// name the file a [`PageIoError::MissingExtent`] was looking for.
#[derive(Debug, Clone)]
struct ExtentNaming {
    dir: PathBuf,
    stem: String,
    digits: usize,
}

impl ExtentNaming {
    fn detect(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        let (stem, suffix) = name.rsplit_once('_')?;
        if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(Self {
            dir: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
            stem: stem.to_string(),
            digits: suffix.len(),
        })
    }

    fn extent_of(path: &Path) -> Option<u16> {
        let name = path.file_name()?.to_str()?;
        let (_, suffix) = name.rsplit_once('_')?;
        suffix.parse().ok()
    }

    fn path_for(&self, ext: u16) -> PathBuf {
        self.dir.join(format!(
            "{}_{:0width$}",
            self.stem,
            ext,
            width = self.digits
        ))
    }
}

impl ExtentSet {
    /// Open `path` as the sole extent of a database.
    ///
    /// The extent number comes from the `_NNNN` filename suffix when there is
    /// one, and is [`PageId::PRIMARY`] otherwise — a temporary file with an
    /// arbitrary name still addresses as extent 1.
    fn open_primary(path: &Path) -> Result<Self, PageIoError> {
        let naming = ExtentNaming::detect(path);
        let primary = ExtentNaming::extent_of(path).unwrap_or(PageId::PRIMARY);
        let (extent, words) = open_extent(primary, path)?;
        let page_size = PageSize::from_header_words(words).map_err(|e| e.with_path(path))?;

        Ok(Self {
            extents: vec![extent],
            primary,
            page_size,
            naming,
        })
    }

    /// Register a further extent file.
    ///
    /// Cross-extent addressing is not exercised yet; this is the seam that lets
    /// it be wired in without the resolver ever having to guess. The extent must
    /// agree with the primary on page size, since a page address means nothing
    /// otherwise.
    pub fn attach(&mut self, ext: u16, path: impl AsRef<Path>) -> Result<(), PageIoError> {
        let path = path.as_ref();
        let (extent, words) = open_extent(ext, path)?;
        if words != self.page_size.words() {
            return Err(PageIoError::ExtentPageSizeMismatch {
                ext,
                path: path.to_path_buf(),
                words,
                primary_ext: self.primary,
                primary_words: self.page_size.words(),
            });
        }
        self.extents.retain(|e| e.ext != ext);
        self.extents.push(extent);
        Ok(())
    }

    fn get(&self, ext: u16) -> Option<&Extent> {
        self.extents.iter().find(|e| e.ext == ext)
    }

    fn resolve(&self, id: PageId) -> Result<&Extent, PageIoError> {
        self.get(id.ext).ok_or_else(|| PageIoError::MissingExtent {
            ext: id.ext,
            page: id.page,
            expected_path: self.naming.as_ref().map(|n| n.path_for(id.ext)),
        })
    }

    fn primary_extent(&self) -> &Extent {
        self.get(self.primary)
            .expect("the primary extent is registered at construction")
    }
}

fn open_extent(ext: u16, path: &Path) -> Result<(Extent, u32), PageIoError> {
    let file = File::open(path).map_err(|e| PageIoError::io(path, None, e))?;
    let len = file
        .metadata()
        .map_err(|e| PageIoError::io(path, None, e))?
        .len();
    if len < MIN_FILE_SIZE {
        return Err(PageIoError::FileTooSmall {
            path: path.to_path_buf(),
            expected: MIN_FILE_SIZE,
            actual: len,
        });
    }

    let mut header = [0u8; HEADER_PROBE_BYTES];
    read_exact_at(&file, path, PageId::new(ext, 0), &mut header, 0)?;
    let words = u32::from_be_bytes(
        header[PAGE_SIZE_WORD_OFFSET..PAGE_SIZE_WORD_OFFSET + 4]
            .try_into()
            .expect("slice of four bytes"),
    );

    Ok((
        Extent {
            ext,
            path: path.to_path_buf(),
            file,
            len,
        },
        words,
    ))
}

/// Reads pages out of a database's extent files.
///
/// Implements [`PageSource`]; there is no page-returning method, so nothing
/// here allocates per read.
#[derive(Debug)]
pub struct PageReader {
    extents: ExtentSet,
    counters: IoCounters,
}

impl PageReader {
    /// Open a database file and take its page size from the header.
    ///
    /// Fails when the header's page size field is unusable. The reader this
    /// replaces fell back to 4096 in that case and went on to address every
    /// page in the file at the wrong offset.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        Ok(Self {
            extents: ExtentSet::open_primary(path)?,
            counters: IoCounters::default(),
        })
    }

    /// Register another extent file so `PageId { ext, .. }` can reach it.
    pub fn attach_extent(&mut self, ext: u16, path: impl AsRef<Path>) -> Result<(), PageIoError> {
        self.extents.attach(ext, path)
    }

    /// Path of the primary extent.
    pub fn path(&self) -> &Path {
        &self.extents.primary_extent().path
    }

    /// Complete pages present in the primary extent when it was opened.
    ///
    /// A length sampled at open time, so it does not grow with a file that E3D
    /// is still appending to. Reads are not range-checked against it — a page
    /// past the end surfaces as an `UnexpectedEof` read failure — so a stale
    /// value bounds enumeration without ever blocking a valid read.
    pub fn physical_page_count(&self) -> u32 {
        let extent = self.extents.primary_extent();
        (extent.len / self.extents.page_size.bytes() as u64).min(u64::from(u32::MAX)) as u32
    }
}

impl PageSource for PageReader {
    fn page_size(&self) -> PageSize {
        self.extents.page_size
    }

    fn read_with_intent(
        &self,
        id: PageId,
        buf: &mut [u8],
        intent: ReadIntent,
    ) -> Result<(), PageIoError> {
        let need = self.extents.page_size.bytes();
        let got = buf.len();
        let dst = buf
            .get_mut(..need)
            .ok_or(PageIoError::BufferTooSmall { need, got })?;

        let extent = self.extents.resolve(id)?;
        read_exact_at(
            &extent.file,
            &extent.path,
            id,
            dst,
            id.byte_offset(self.extents.page_size),
        )?;

        match intent {
            ReadIntent::Demand => self.counters.record_demand(need),
            ReadIntent::Prefetch => self.counters.record_prefetch(),
        }
        Ok(())
    }

    fn stats(&self) -> IoStats {
        self.counters.snapshot()
    }
}

/// Positioned read of a whole buffer.
///
/// Positioned rather than `seek` + `read_exact` so the call takes `&self`:
/// there is no shared file cursor for a second reader to move underneath us.
fn read_exact_at(
    file: &File,
    path: &Path,
    id: PageId,
    buf: &mut [u8],
    offset: u64,
) -> Result<(), PageIoError> {
    let want = buf.len();
    let mut filled = 0usize;
    while filled < want {
        match read_at(file, &mut buf[filled..], offset + filled as u64) {
            Ok(0) => return Err(PageIoError::short_read(path, id, want, filled)),
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(PageIoError::io(path, Some(id), e)),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A file of `pages` pages whose header states `words` words per page.
    fn db_file(dir: &Path, name: &str, words: u32, pages: usize) -> PathBuf {
        let page_bytes = words as usize * 4;
        let mut bytes = vec![0u8; page_bytes * pages];
        bytes[PAGE_SIZE_WORD_OFFSET..PAGE_SIZE_WORD_OFFSET + 4]
            .copy_from_slice(&words.to_be_bytes());
        for page in 0..pages {
            bytes[page * page_bytes] = page as u8;
        }
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
        path
    }

    #[test]
    fn page_size_comes_from_words_not_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_file(dir.path(), "ams1000_0001", 512, 3);
        let reader = PageReader::open(&path).unwrap();
        assert_eq!(reader.page_size().bytes(), 2048);
        assert_eq!(reader.physical_page_count(), 3);
    }

    #[test]
    fn unusable_page_size_is_an_error_not_a_default() {
        let dir = tempfile::tempdir().unwrap();
        // 700 words = 2800 bytes: in range, but not a power of two, so the old
        // rule fell through to DEFAULT_PAGE_SIZE and read the file at 4096.
        let path = db_file(dir.path(), "ams1000_0001", 700, 2);
        let err = PageReader::open(&path).unwrap_err();
        assert!(
            matches!(
                err,
                E3dReaderError::Page(PageIoError::InvalidPageSize { words: 700, .. })
            ),
            "expected InvalidPageSize, got {err}"
        );
    }

    #[test]
    fn absent_extent_reports_instead_of_reading_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_file(dir.path(), "ams1000_0001", 32, 4);
        let reader = PageReader::open(&path).unwrap();

        let mut buf = vec![0u8; reader.page_size().bytes()];
        reader.read_into(PageId::new(1, 2), &mut buf).unwrap();
        assert_eq!(buf[0], 2, "extent 1 page 2 should be readable");

        let err = reader.read_into(PageId::new(2, 2), &mut buf).unwrap_err();
        match err {
            PageIoError::MissingExtent {
                ext: 2,
                page: 2,
                expected_path,
            } => assert_eq!(
                expected_path.as_deref(),
                Some(dir.path().join("ams1000_0002").as_path())
            ),
            other => panic!("expected MissingExtent, got {other}"),
        }
    }

    #[test]
    fn attached_extent_is_read_from_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let primary = db_file(dir.path(), "ams1000_0001", 32, 4);
        let second = db_file(dir.path(), "ams1000_0002", 32, 4);
        // Mark the second extent so reading the wrong file is visible.
        let mut bytes = std::fs::read(&second).unwrap();
        bytes[2 * 128] = 0xAB;
        std::fs::write(&second, &bytes).unwrap();

        let mut reader = PageReader::open(&primary).unwrap();
        reader.attach_extent(2, &second).unwrap();

        let mut buf = vec![0u8; reader.page_size().bytes()];
        reader.read_into(PageId::new(2, 2), &mut buf).unwrap();
        assert_eq!(buf[0], 0xAB);
    }

    #[test]
    fn attaching_an_extent_with_another_page_size_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let primary = db_file(dir.path(), "ams1000_0001", 32, 4);
        let second = db_file(dir.path(), "ams1000_0002", 64, 4);
        let mut reader = PageReader::open(&primary).unwrap();
        let err = reader.attach_extent(2, &second).unwrap_err();
        assert!(
            matches!(err, PageIoError::ExtentPageSizeMismatch { ext: 2, .. }),
            "expected ExtentPageSizeMismatch, got {err}"
        );
    }

    #[test]
    fn prefetch_is_counted_apart_from_demand_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_file(dir.path(), "ams1000_0001", 32, 4);
        let reader = PageReader::open(&path).unwrap();
        let mut buf = vec![0u8; reader.page_size().bytes()];

        reader.read_into(PageId::primary(0), &mut buf).unwrap();
        reader.prefetch_into(PageId::primary(1), &mut buf).unwrap();
        reader.prefetch_into(PageId::primary(2), &mut buf).unwrap();

        let stats = reader.stats();
        assert_eq!(stats.pages_read, 1);
        assert_eq!(stats.bytes_read, 128);
        assert_eq!(stats.prefetched_pages, 2);
    }

    #[test]
    fn reading_past_the_end_reports_unexpected_eof() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_file(dir.path(), "ams1000_0001", 32, 2);
        let reader = PageReader::open(&path).unwrap();
        let mut buf = vec![0u8; reader.page_size().bytes()];

        let err = reader.read_into(PageId::primary(9), &mut buf).unwrap_err();
        match err {
            PageIoError::Io { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::UnexpectedEof)
            }
            other => panic!("expected UnexpectedEof, got {other}"),
        }
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_partly_filled() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_file(dir.path(), "ams1000_0001", 32, 2);
        let reader = PageReader::open(&path).unwrap();
        let mut buf = [0u8; 64];
        let err = reader.read_into(PageId::primary(0), &mut buf).unwrap_err();
        assert!(
            matches!(err, PageIoError::BufferTooSmall { need: 128, got: 64 }),
            "expected BufferTooSmall, got {err}"
        );
    }
}
