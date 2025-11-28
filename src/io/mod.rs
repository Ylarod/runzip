mod http;
mod local;

pub use http::HttpRangeReader;
pub use local::LocalFileReader;

use anyhow::Result;
use async_trait::async_trait;

/// Trait for random access reading from a data source
#[async_trait]
pub trait ReadAt: Send + Sync {
    /// Read data at the specified offset into the buffer
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Get the total size of the data source
    fn size(&self) -> u64;
}
