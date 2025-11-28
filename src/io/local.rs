use super::ReadAt;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

/// Local file reader with random access support
pub struct LocalFileReader {
    file: std::fs::File,
    size: u64,
}

impl LocalFileReader {
    pub fn new(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self { file, size })
    }
}

#[async_trait]
impl ReadAt for LocalFileReader {
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            Ok(self.file.read_at(buf, offset)?)
        }

        #[cfg(windows)]
        {
            use std::io::{Read, Seek, SeekFrom};
            // Windows doesn't have pread, need to seek and read
            // This is not thread-safe, but we're using it in async context
            let file = &self.file;
            let mut file = unsafe {
                // Create a new handle for this read operation
                use std::os::windows::io::AsRawHandle;
                use std::os::windows::io::FromRawHandle;
                std::fs::File::from_raw_handle(file.as_raw_handle())
            };
            file.seek(SeekFrom::Start(offset))?;
            let n = file.read(buf)?;
            std::mem::forget(file); // Don't close the handle
            Ok(n)
        }

        #[cfg(not(any(unix, windows)))]
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = &self.file;
            file.seek(SeekFrom::Start(offset))?;
            Ok(file.read(buf)?)
        }
    }

    fn size(&self) -> u64 {
        self.size
    }
}
