//! High-level ZIP file extraction API.
//!
//! This module provides a user-friendly interface for extracting files
//! from ZIP archives, handling decompression and file I/O automatically.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::path::Path;
//! use runzip::{HttpRangeReader, ZipExtractor};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create an extractor for a remote ZIP file
//! let reader = Arc::new(HttpRangeReader::new("https://example.com/archive.zip".to_string()).await?);
//! let extractor = ZipExtractor::new(reader);
//!
//! // List and extract files
//! for entry in extractor.list_files().await? {
//!     if !entry.is_directory {
//!         extractor.extract_to_file(&entry, Path::new(&entry.file_name)).await?;
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::io::ReadAt;
use anyhow::{Result, bail};
use flate2::read::DeflateDecoder;

use super::parser::ZipParser;
use super::structures::{CompressionMethod, ZipFileEntry};

/// High-level ZIP file extractor.
///
/// This struct provides convenient methods for listing and extracting
/// files from ZIP archives. It wraps the lower-level [`ZipParser`] and
/// handles decompression automatically.
///
/// ## Supported Compression Methods
///
/// - `STORED` (0): No compression, data is copied directly
/// - `DEFLATE` (8): Standard ZIP compression using flate2
///
/// ## Generic Parameter
///
/// The extractor is generic over the reader type `R`, allowing it to
/// work with both local files ([`LocalFileReader`](crate::LocalFileReader))
/// and remote sources ([`HttpRangeReader`](crate::HttpRangeReader)).
pub struct ZipExtractor<R: ReadAt> {
    /// The underlying parser for reading ZIP structures
    parser: ZipParser<R>,
}

impl<R: ReadAt> ZipExtractor<R> {
    /// Create a new extractor for the given reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - A shared reference to a reader implementing [`ReadAt`]
    ///
    /// # Returns
    ///
    /// A new extractor instance ready to list and extract files.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let reader = Arc::new(LocalFileReader::new(Path::new("archive.zip"))?);
    /// let extractor = ZipExtractor::new(reader);
    /// ```
    pub fn new(reader: Arc<R>) -> Self {
        Self {
            parser: ZipParser::new(reader),
        }
    }

    /// List all files in the archive.
    ///
    /// Returns metadata for all entries in the ZIP file, including
    /// both files and directories.
    ///
    /// # Returns
    ///
    /// A vector of [`ZipFileEntry`] with metadata for each entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive is invalid or cannot be read.
    ///
    /// # Example
    ///
    /// ```ignore
    /// for entry in extractor.list_files().await? {
    ///     println!("{}: {} bytes", entry.file_name, entry.uncompressed_size);
    /// }
    /// ```
    pub async fn list_files(&self) -> Result<Vec<ZipFileEntry>> {
        self.parser.list_files().await
    }

    /// Extract a file's contents to memory.
    ///
    /// Reads and decompresses the file data, returning it as a byte vector.
    /// This method handles both STORED and DEFLATE compression methods.
    ///
    /// # Arguments
    ///
    /// * `entry` - The file entry to extract (from [`list_files()`])
    ///
    /// # Returns
    ///
    /// The decompressed file contents as a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file uses an unsupported compression method
    /// - The data cannot be read or decompressed
    ///
    /// # Memory Usage
    ///
    /// This method loads the entire file into memory. For large files,
    /// consider using [`extract_to_file()`] instead.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let data = extractor.extract_to_memory(&entry).await?;
    /// let text = String::from_utf8_lossy(&data);
    /// println!("{}", text);
    /// ```
    pub async fn extract_to_memory(&self, entry: &ZipFileEntry) -> Result<Vec<u8>> {
        // Calculate where the actual file data begins
        let data_offset = self.parser.get_data_offset(entry).await?;

        match entry.compression_method {
            CompressionMethod::Stored => {
                // No compression - read data directly
                let mut buf = vec![0u8; entry.uncompressed_size as usize];
                self.parser.reader().read_at(data_offset, &mut buf).await?;
                Ok(buf)
            }
            CompressionMethod::Deflate => {
                // DEFLATE compression - read compressed data first
                let mut compressed = vec![0u8; entry.compressed_size as usize];
                self.parser
                    .reader()
                    .read_at(data_offset, &mut compressed)
                    .await?;

                // Decompress using flate2's DeflateDecoder
                // Note: ZIP uses raw DEFLATE, not zlib or gzip wrapped
                let mut decoder = DeflateDecoder::new(&compressed[..]);
                let mut decompressed = Vec::with_capacity(entry.uncompressed_size as usize);
                decoder.read_to_end(&mut decompressed)?;

                Ok(decompressed)
            }
            CompressionMethod::Unknown(method) => {
                bail!("Unsupported compression method: {}", method);
            }
        }
    }

    /// Extract a file to the filesystem.
    ///
    /// Reads, decompresses, and writes the file to the specified path.
    /// Parent directories are created automatically if they don't exist.
    ///
    /// # Arguments
    ///
    /// * `entry` - The file entry to extract
    /// * `output_path` - The filesystem path to write the file to
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read or decompressed
    /// - Parent directories cannot be created
    /// - The file cannot be written
    ///
    /// # Example
    ///
    /// ```ignore
    /// extractor.extract_to_file(&entry, Path::new("output/file.txt")).await?;
    /// ```
    pub async fn extract_to_file(&self, entry: &ZipFileEntry, output_path: &Path) -> Result<()> {
        // Ensure parent directories exist
        if let Some(parent) = output_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).await?;
        }

        // Extract file contents to memory
        let data = self.extract_to_memory(entry).await?;

        // Write to the output file
        let mut file = fs::File::create(output_path).await?;
        file.write_all(&data).await?;

        Ok(())
    }

    /// Extract a file's contents to stdout.
    ///
    /// Reads, decompresses, and writes the file directly to standard output.
    /// Useful for piping archive contents to other commands.
    ///
    /// # Arguments
    ///
    /// * `entry` - The file entry to extract
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, decompressed, or written.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Equivalent to: runzip -p archive.zip file.txt | cat
    /// extractor.extract_to_stdout(&entry).await?;
    /// ```
    pub async fn extract_to_stdout(&self, entry: &ZipFileEntry) -> Result<()> {
        let data = self.extract_to_memory(entry).await?;

        let mut stdout = tokio::io::stdout();
        stdout.write_all(&data).await?;

        Ok(())
    }
}
