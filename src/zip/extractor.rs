use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::io::ReadAt;
use anyhow::{bail, Result};

use super::parser::ZipParser;
use super::structures::{CompressionMethod, ZipFileEntry};

/// ZIP file extractor
pub struct ZipExtractor<R: ReadAt> {
    parser: ZipParser<R>,
}

impl<R: ReadAt> ZipExtractor<R> {
    pub fn new(reader: Arc<R>) -> Self {
        Self {
            parser: ZipParser::new(reader),
        }
    }

    /// List all files in the archive
    pub async fn list_files(&self) -> Result<Vec<ZipFileEntry>> {
        self.parser.list_files().await
    }

    /// Extract file data to memory
    pub async fn extract_to_memory(&self, entry: &ZipFileEntry) -> Result<Vec<u8>> {
        // Check compression method
        if entry.compression_method != CompressionMethod::Stored {
            bail!(
                "Unsupported compression method: {} (only STORED/uncompressed is supported)",
                entry.compression_method.as_u16()
            );
        }

        // Get data offset
        let data_offset = self.parser.get_data_offset(entry).await?;

        // Read file data
        let mut buf = vec![0u8; entry.uncompressed_size as usize];
        self.parser.reader().read_at(data_offset, &mut buf).await?;

        Ok(buf)
    }

    /// Extract file to disk
    pub async fn extract_to_file(&self, entry: &ZipFileEntry, output_path: &Path) -> Result<()> {
        // Create parent directories if needed
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await?;
            }
        }

        // Extract data
        let data = self.extract_to_memory(entry).await?;

        // Write to file
        let mut file = fs::File::create(output_path).await?;
        file.write_all(&data).await?;

        Ok(())
    }

    /// Extract file to stdout
    pub async fn extract_to_stdout(&self, entry: &ZipFileEntry) -> Result<()> {
        let data = self.extract_to_memory(entry).await?;

        let mut stdout = tokio::io::stdout();
        stdout.write_all(&data).await?;

        Ok(())
    }
}
