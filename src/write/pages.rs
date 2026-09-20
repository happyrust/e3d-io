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
        Ok(Self { path, file, size })
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
