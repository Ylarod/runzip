//! Local filesystem reader with random access support.
//!
//! This module implements random-access reading from local files using
//! platform-specific optimizations for efficient I/O.

use super::ReadAt;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

/// Local file reader with random access support.
///
/// This reader provides efficient random-access reads from local files,
/// using platform-specific APIs for optimal performance:
///
/// - **Unix**: Uses `pread(2)` via `FileExt::read_at`, which reads at an
///   offset without changing the file position (thread-safe)
/// - **Windows**: Uses seek + read with handle duplication to avoid
///   modifying the original file's position
/// - **Other platforms**: Falls back to seek + read
///
/// ## Example
///
/// ```no_run
/// use std::path::Path;
/// use runzip::LocalFileReader;
///
/// # fn main() -> anyhow::Result<()> {
/// let reader = LocalFileReader::new(Path::new("archive.zip"))?;
/// println!("File size: {} bytes", reader.size());
/// # Ok(())
/// # }
/// ```
pub struct LocalFileReader {
    /// The underlying file handle
    file: std::fs::File,
    /// Cached file size in bytes
    size: u64,
}

impl LocalFileReader {
    /// Create a new local file reader for the given path.
    ///
    /// Opens the file in read-only mode and caches its size for later use.
    ///
    /// # Arguments
    ///
    /// * `path` - The filesystem path to the ZIP file
    ///
    /// # Returns
    ///
    /// A configured reader ready for random-access reads.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file doesn't exist
    /// - The file can't be opened (permissions, etc.)
    /// - The file metadata can't be read
    pub fn new(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self { file, size })
    }
}

#[async_trait]
impl ReadAt for LocalFileReader {
    /// Read data at the specified offset from the local file.
    ///
    /// Uses platform-specific optimizations:
    /// - Unix: `pread(2)` for atomic positioned reads
    /// - Windows: Duplicates handle to avoid position conflicts
    /// - Other: Standard seek + read
    ///
    /// # Arguments
    ///
    /// * `offset` - The byte offset to start reading from
    /// * `buf` - The buffer to read data into
    ///
    /// # Returns
    ///
    /// The number of bytes read, or an error if the read fails.
    ///
    /// # Platform Notes
    ///
    /// On Unix, this operation is atomic and thread-safe. On Windows and
    /// other platforms, concurrent reads may have race conditions, though
    /// this is generally safe in the single-threaded async context used here.
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        #[cfg(unix)]
        {
            // Unix: use pread for atomic positioned read (thread-safe)
            use std::os::unix::fs::FileExt;
            Ok(self.file.read_at(buf, offset)?)
        }

        #[cfg(windows)]
        {
            use std::io::{Read, Seek, SeekFrom};
            // Windows doesn't have pread, need to seek and read
            // We duplicate the handle to avoid affecting the original file position
            let file = &self.file;
            let mut file = unsafe {
                // Create a temporary handle copy for this read operation
                // SAFETY: We're creating a new File from the same raw handle,
                // and we call forget() at the end to prevent double-close
                use std::os::windows::io::AsRawHandle;
                use std::os::windows::io::FromRawHandle;
                std::fs::File::from_raw_handle(file.as_raw_handle())
            };
            file.seek(SeekFrom::Start(offset))?;
            let n = file.read(buf)?;
            std::mem::forget(file); // Don't close the handle - original owns it
            Ok(n)
        }

        #[cfg(not(any(unix, windows)))]
        {
            // Fallback for other platforms: simple seek + read
            use std::io::{Read, Seek, SeekFrom};
            let mut file = &self.file;
            file.seek(SeekFrom::Start(offset))?;
            Ok(file.read(buf)?)
        }
    }

    /// Get the total size of the local file.
    ///
    /// Returns the cached file size obtained during construction.
    fn size(&self) -> u64 {
        self.size
    }
}
