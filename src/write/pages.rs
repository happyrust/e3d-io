//! Whole-page I/O on a dabacon file that is being appended to.
//!
//! `core.dll` addresses blocks 1-based (`FHDBWN`: `offset = (blk - 1) ×
//! page_bytes`, A9 of the write-path note); this crate's page numbers are the
//! 0-based ones the index and the session pages state, so a page lands at
//! `page_no × page_bytes` — the same arithmetic
//! [`PageId::byte_offset`](crate::page::PageId::byte_offset) uses on the read
//! side. Words are big-endian on disk, which is the caller's business: this
//! type moves bytes.
//!
//! It never truncates. A page written past the current end extends the file;
//! the pages between the old end and the new one, if any, are left to the
//! caller, who allocates page numbers contiguously and so never leaves a hole
//! (A7: "文件只在尾部增长、无洞").

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::page::PageSize;

use super::WriteError;

/// A read/write handle on one dabacon file.
#[derive(Debug)]
pub struct PageFile {
    path: PathBuf,
    file: File,
    size: PageSize,
    /// Whether this handle holds the write-mode sentinel lock (Windows only;
    /// always `false` elsewhere, where it is unread). Released on drop.
    #[cfg_attr(not(windows), allow(dead_code))]
    locked: bool,
}

impl PageFile {
    /// Open for reading and writing. The page size comes from page 0's
    /// `PGSIZE` word (+0x34), read here so that every later call agrees with
    /// the file about where a page starts.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WriteError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| WriteError::io(&path, source))?;
        let mut head = [0u8; 0x38];
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.read_exact(&mut head))
            .map_err(|source| WriteError::io(&path, source))?;
        let words = u32::from_be_bytes(head[0x34..0x38].try_into().expect("four bytes"));
        let size = PageSize::from_header_words(words).map_err(|source| WriteError::PageSize {
            path: path.clone(),
            words,
            detail: source.to_string(),
        })?;
        Ok(Self { path, file, size, locked: false })
    }

    /// Take `core.dll`'s write-mode sentinel lock before appending anything:
    /// an exclusive lock on one byte at [`sentinel::LOCK_OFFSET`]
    /// (`0x7FFFFFF6FFFFFFFF`, A9 of the write-path note), which `core.dll` holds
    /// while a database is open for writing. A file another writer — this crate
    /// or a real E3D in write mode — is mid-write on is refused at once with
    /// [`WriteError::Locked`], nothing having been written; the lock is released
    /// when this handle drops (on commit or rollback). Waiting for the lock is
    /// the caller's business, so this never blocks.
    ///
    /// On platforms without `LockFileEx` this is a no-op — the commit's page-0
    /// re-read still catches a writer that finished first (`WriteError::Moved`),
    /// as it did before the lock.
    pub fn lock_for_write(&mut self) -> Result<(), WriteError> {
        #[cfg(windows)]
        {
            if sentinel::try_lock_exclusive(&self.file) {
                self.locked = true;
                Ok(())
            } else {
                Err(WriteError::Locked { path: self.path.clone() })
            }
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn page_size(&self) -> PageSize {
        self.size
    }

    /// Whole pages the file holds right now, by its length.
    pub fn physical_page_count(&self) -> Result<u32, WriteError> {
        let len = self
            .file
            .metadata()
            .map_err(|source| WriteError::io(&self.path, source))?
            .len();
        Ok((len / self.size.bytes() as u64) as u32)
    }

    /// One page, exactly `page_size` bytes.
    pub fn read_page(&mut self, page_no: u32) -> Result<Vec<u8>, WriteError> {
        let mut page = vec![0u8; self.size.bytes()];
        let offset = u64::from(page_no) * self.size.bytes() as u64;
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut page))
            .map_err(|source| WriteError::io_page(&self.path, page_no, source))?;
        Ok(page)
    }

    /// Write one page at `page_no`, extending the file when it lies past the
    /// end. The buffer must be exactly one page.
    pub fn write_page(&mut self, page_no: u32, page: &[u8]) -> Result<(), WriteError> {
        if page.len() != self.size.bytes() {
            return Err(WriteError::PageLength {
                page_no,
                len: page.len(),
                want: self.size.bytes(),
            });
        }
        let offset = u64::from(page_no) * self.size.bytes() as u64;
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.write_all(page))
            .map_err(|source| WriteError::io_page(&self.path, page_no, source))
    }

    /// Push everything to the disk. `core.dll` does this when it leaves write
    /// mode (`FlushFileBuffers` in `FHSWIT`); the commit sequence calls it once
    /// after the new pages and once after page 0, so the two never reorder.
    pub fn sync(&mut self) -> Result<(), WriteError> {
        self.file
            .sync_all()
            .map_err(|source| WriteError::io(&self.path, source))
    }
}

/// Big-endian word helpers shared by the page builders.
pub(crate) fn be_word(page: &[u8], byte_offset: usize) -> u32 {
    u32::from_be_bytes(
        page[byte_offset..byte_offset + 4]
            .try_into()
            .expect("slice is exactly four bytes"),
    )
}

pub(crate) fn put_be_word(page: &mut [u8], byte_offset: usize, word: u32) {
    page[byte_offset..byte_offset + 4].copy_from_slice(&word.to_be_bytes());
}

#[cfg(windows)]
impl Drop for PageFile {
    fn drop(&mut self) {
        if self.locked {
            sentinel::unlock(&self.file);
        }
    }
}

/// `core.dll`'s write-mode sentinel lock, by hand — two `extern "system"`
/// declarations, no crate (Q5). `core.dll` (`FHSWIT`, A9) takes an exclusive
/// `LockFileEx` on one byte at `0x7FFFFFF6FFFFFFFF` when a database enters write
/// mode and drops it with `UnlockFileEx` on the way out; locking the same byte
/// is how this crate and E3D exclude each other.
#[cfg(windows)]
mod sentinel {
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, RawHandle};

    /// The byte `core.dll` locks. Beyond any real file length — Windows locks a
    /// range past end-of-file fine, and the point is the marker, not the byte.
    pub const LOCK_OFFSET: u64 = 0x7FFF_FFF6_FFFF_FFFF;
    const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
    const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        h_event: RawHandle,
    }

    extern "system" {
        fn LockFileEx(
            file: RawHandle,
            flags: u32,
            reserved: u32,
            bytes_low: u32,
            bytes_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
        fn UnlockFileEx(
            file: RawHandle,
            reserved: u32,
            bytes_low: u32,
            bytes_high: u32,
            overlapped: *mut Overlapped,
        ) -> i32;
    }

    fn overlapped() -> Overlapped {
        Overlapped {
            internal: 0,
            internal_high: 0,
            offset: LOCK_OFFSET as u32,
            offset_high: (LOCK_OFFSET >> 32) as u32,
            h_event: std::ptr::null_mut(),
        }
    }

    /// Try to take the exclusive one-byte lock without waiting. `true` if it was
    /// taken, `false` if another handle already holds it.
    pub fn try_lock_exclusive(file: &File) -> bool {
        let mut ov = overlapped();
        // SAFETY: `file`'s handle outlives the call; `ov` is a valid, owned
        // OVERLAPPED for the duration; one byte is locked, none read or written.
        let ok = unsafe {
            LockFileEx(
                file.as_raw_handle(),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut ov,
            )
        };
        ok != 0
    }

    /// Release the one-byte lock. Errors are ignored: the handle is closing, and
    /// a lock on a handle is dropped when the handle is anyway.
    pub fn unlock(file: &File) {
        let mut ov = overlapped();
        // SAFETY: as `try_lock_exclusive`.
        unsafe {
            UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut ov);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal file whose page-0 `PGSIZE` word (+0x34) is 512, enough for
    /// [`PageFile::open`].
    fn one_page_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("lock_probe");
        let mut page = vec![0u8; 2048];
        page[0x34..0x38].copy_from_slice(&512u32.to_be_bytes());
        let mut file = File::create(&path).expect("create");
        file.write_all(&page).expect("write");
        file.sync_all().expect("sync");
        (dir, path)
    }

    #[test]
    fn the_write_lock_is_exclusive_and_released_on_drop() {
        let (_dir, path) = one_page_file();

        let mut first = PageFile::open(&path).expect("open first");
        first.lock_for_write().expect("first takes the lock");

        // A second handle cannot take it while the first holds it.
        let mut second = PageFile::open(&path).expect("open second");
        assert!(
            matches!(second.lock_for_write(), Err(WriteError::Locked { .. })),
            "the byte core.dll locks admits one writer at a time"
        );

        // Dropping the first releases it; the second can then take it.
        drop(first);
        second.lock_for_write().expect("the lock is free once the first handle drops");
    }
}
