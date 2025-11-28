//! I/O abstractions for reading ZIP files from various sources.
//!
//! This module provides a unified interface for random-access reading,
//! allowing the ZIP parser to work with both local files and remote HTTP sources.
//!
//! ## Architecture
//!
//! The core abstraction is the [`ReadAt`] trait, which provides:
//! - Random access reads at arbitrary offsets
//! - Total size information for the data source
//!
//! ## Implementations
//!
//! - [`LocalFileReader`]: Reads from local filesystem using platform-specific
//!   optimizations (pread on Unix, seek+read on Windows)
//! - [`HttpRangeReader`]: Reads from HTTP servers using Range requests,
//!   enabling efficient partial downloads of remote archives

mod http;
mod local;

pub use http::HttpRangeReader;
pub use local::LocalFileReader;

use anyhow::Result;
use async_trait::async_trait;

/// Trait for random access reading from a data source.
///
/// This trait abstracts over different data sources (local files, HTTP, etc.)
/// to provide a unified interface for the ZIP parser. Implementations must
/// be thread-safe (`Send + Sync`) to support concurrent access.
///
/// # Example
///
/// ```ignore
/// async fn read_header<R: ReadAt>(reader: &R) -> Result<[u8; 4]> {
///     let mut buf = [0u8; 4];
///     reader.read_at(0, &mut buf).await?;
///     Ok(buf)
/// }
/// ```
#[async_trait]
pub trait ReadAt: Send + Sync {
    /// Read data at the specified offset into the buffer.
    ///
    /// Reads up to `buf.len()` bytes starting at `offset` into `buf`.
    /// Returns the number of bytes actually read, which may be less than
    /// the buffer size if EOF is reached.
    ///
    /// # Arguments
    ///
    /// * `offset` - The byte offset to start reading from
    /// * `buf` - The buffer to read data into
    ///
    /// # Returns
    ///
    /// The number of bytes read, or an error if the read fails.
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Get the total size of the data source in bytes.
    ///
    /// For local files, this is the file size.
    /// For HTTP sources, this is the Content-Length from the server.
    fn size(&self) -> u64;
}
