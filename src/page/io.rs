use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::E3dReaderError;
use crate::meta::constants::DEFAULT_PAGE_SIZE;

/// Low-level page reader backed by a file handle.
///
/// All pages are `page_size` bytes on disk, addressed as `offset = page_no * page_size`.
/// This matches core.dll's DirectAccessToken virtual method behavior.
pub struct PageReader {
    file: File,
    path: PathBuf,
    page_size: u32,
    file_len: u64,
}

impl PageReader {
    /// Open a database file and detect page size.
    ///
    /// Bootstrap: reads first 24 bytes to extract page_size from descriptor (page 0, offset +20).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, E3dReaderError> {
        let path = path.as_ref();
        let mut file = File::open(path).map_err(|e| E3dReaderError::io(path, e))?;

        let file_len = file
            .metadata()
            .map_err(|e| E3dReaderError::io(path, e))?
            .len();
        if file_len < crate::meta::constants::MIN_FILE_SIZE {
            return Err(E3dReaderError::FileTooSmall {
                expected: crate::meta::constants::MIN_FILE_SIZE,
                actual: file_len,
            });
        }

        let mut header = [0u8; 60];
        file.read_exact(&mut header)
            .map_err(|e| E3dReaderError::io(path, e))?;

        let raw_page_size_words = u32::from_be_bytes(header[0x34..0x38].try_into().unwrap());
        // The header stores page size in 4-byte words (Fortran convention).
        // Actual byte size = words * 4.
        // Evidence: file_size / (words * 4) gives clean page count matching latest_ses_pgno.
        let page_size_bytes = raw_page_size_words.saturating_mul(4);
        let page_size = if page_size_bytes >= 128
            && page_size_bytes <= 65536
            && page_size_bytes.is_power_of_two()
        {
            page_size_bytes
        } else {
            DEFAULT_PAGE_SIZE
        };

        Ok(Self {
            file,
            path: path.to_path_buf(),
            page_size,
            file_len,
        })
    }

    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of complete physical pages currently present in the file.
    pub fn physical_page_count(&self) -> u32 {
        (self.file_len / u64::from(self.page_size)).min(u64::from(u32::MAX)) as u32
    }

    /// Read a full page into a newly allocated buffer.
    pub fn read_page(&mut self, page_no: u32) -> Result<Vec<u8>, E3dReaderError> {
        let offset = page_no as u64 * self.page_size as u64;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| E3dReaderError::io(&self.path, e))?;

        let mut buf = vec![0u8; self.page_size as usize];
        self.file
            .read_exact(&mut buf)
            .map_err(|e| E3dReaderError::io(&self.path, e))?;
        Ok(buf)
    }

    /// Read a page into an existing buffer (must be at least page_size bytes).
    pub fn read_page_into(&mut self, page_no: u32, buf: &mut [u8]) -> Result<(), E3dReaderError> {
        assert!(buf.len() >= self.page_size as usize);
        let offset = page_no as u64 * self.page_size as u64;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| E3dReaderError::io(&self.path, e))?;
        self.file
            .read_exact(&mut buf[..self.page_size as usize])
            .map_err(|e| E3dReaderError::io(&self.path, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn physical_page_count_uses_complete_file_pages() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = vec![0u8; 3 * 4096 + 17];
        bytes[0x34..0x38].copy_from_slice(&1024u32.to_be_bytes());
        file.write_all(&bytes).unwrap();
        let reader = PageReader::open(file.path()).unwrap();
        assert_eq!(reader.page_size(), 4096);
        assert_eq!(reader.physical_page_count(), 3);
    }
}
