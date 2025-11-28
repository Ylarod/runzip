//! HTTP Range request reader for remote ZIP files.
//!
//! This module implements random-access reading from HTTP servers using
//! the Range request header (RFC 7233). This allows efficient partial
//! downloads of ZIP archives, fetching only the necessary data.

use async_trait::async_trait;
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::ReadAt;
use anyhow::{Result, anyhow, bail};

/// HTTP Range reader for remote ZIP files.
///
/// This reader uses HTTP Range requests to fetch specific byte ranges from
/// a remote server, enabling efficient extraction of individual files from
/// large remote archives without downloading the entire file.
///
/// ## Requirements
///
/// The remote server must:
/// - Support HTTP Range requests (indicated by `Accept-Ranges: bytes` header)
/// - Provide a `Content-Length` header in HEAD responses
///
/// ## Features
///
/// - Automatic retry with exponential backoff for transient network errors
/// - Transfer statistics tracking for monitoring bandwidth usage
/// - Connection pooling via reqwest for efficient HTTP requests
///
/// ## Example
///
/// ```no_run
/// use runzip::HttpRangeReader;
///
/// # async fn example() -> anyhow::Result<()> {
/// let reader = HttpRangeReader::new("https://example.com/large.zip".to_string()).await?;
/// println!("File size: {} bytes", reader.size());
/// # Ok(())
/// # }
/// ```
pub struct HttpRangeReader {
    /// HTTP client with connection pooling
    client: Client,
    /// The URL of the remote file
    url: String,
    /// Total size of the remote file in bytes
    size: u64,
    /// Cumulative bytes transferred from the network
    transferred_bytes: AtomicU64,
    /// Maximum number of retries for failed requests
    max_retry: u32,
}

impl HttpRangeReader {
    /// Create a new HTTP Range reader for the given URL.
    ///
    /// This constructor performs a HEAD request to:
    /// 1. Verify the server responds successfully
    /// 2. Check for Range request support via `Accept-Ranges` header
    /// 3. Obtain the file size from `Content-Length` header
    ///
    /// # Arguments
    ///
    /// * `url` - The HTTP or HTTPS URL of the ZIP file
    ///
    /// # Returns
    ///
    /// A configured reader ready for random-access reads.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The HTTP request fails
    /// - The server doesn't support Range requests
    /// - The server doesn't provide Content-Length
    pub async fn new(url: String) -> Result<Self> {
        // Create HTTP client with reasonable timeout
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

        // Send HEAD request to check server capabilities
        let resp = client.head(&url).send().await?;

        // Verify successful response
        if !resp.status().is_success() {
            bail!("HTTP request failed with status: {}", resp.status());
        }

        // Verify Range request support (required for partial downloads)
        let accept_ranges = resp
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none");

        if !accept_ranges.contains("bytes") {
            bail!("Remote server does not support Range requests");
        }

        // Get total file size (required for ZIP parsing from end)
        let size = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow!("Remote server did not return Content-Length"))?;

        Ok(Self {
            client,
            url,
            size,
            transferred_bytes: AtomicU64::new(0),
            max_retry: 10,
        })
    }

    /// Get the total bytes transferred from the network.
    ///
    /// This counter tracks all successful data transfers and can be used
    /// to display bandwidth usage statistics to the user.
    ///
    /// # Returns
    ///
    /// The cumulative number of bytes received from the server.
    pub fn transferred_bytes(&self) -> u64 {
        self.transferred_bytes.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ReadAt for HttpRangeReader {
    /// Read data at the specified offset using HTTP Range requests.
    ///
    /// Sends a GET request with `Range: bytes=start-end` header to fetch
    /// the requested data. Implements automatic retry with exponential
    /// backoff for transient network errors (timeouts, connection failures).
    ///
    /// # Arguments
    ///
    /// * `offset` - The byte offset to start reading from
    /// * `buf` - The buffer to read data into
    ///
    /// # Returns
    ///
    /// The number of bytes read, or an error if the request fails.
    ///
    /// # Retry Behavior
    ///
    /// - Retries on timeout and connection errors
    /// - Uses exponential backoff (500ms * retry_count)
    /// - Gives up after `max_retry` attempts (default: 10)
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        // Handle empty buffer case
        if buf.is_empty() {
            return Ok(0);
        }

        // Calculate the byte range to request
        // Clamp end to file size to avoid requesting beyond EOF
        let end = offset + buf.len() as u64 - 1;
        let end = end.min(self.size - 1);
        let expected_size = (end - offset + 1) as usize;

        let mut received = 0;
        let mut retry_count = 0;

        // Loop until we've received all expected data or exhausted retries
        while received < expected_size {
            let current_start = offset + received as u64;
            let range = format!("bytes={}-{}", current_start, end);

            // Send Range request
            let result = self
                .client
                .get(&self.url)
                .header("Range", &range)
                .send()
                .await;

            match result {
                Ok(resp) => {
                    // Verify we got a Partial Content response (206)
                    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        bail!("HTTP request failed with status: {}", resp.status());
                    }

                    // Read response body and copy to buffer
                    let bytes = resp.bytes().await?;
                    let chunk_len = bytes.len().min(expected_size - received);
                    buf[received..received + chunk_len].copy_from_slice(&bytes[..chunk_len]);
                    received += chunk_len;

                    // Update transfer statistics
                    self.transferred_bytes
                        .fetch_add(chunk_len as u64, Ordering::Relaxed);
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    // Retry on transient network errors with backoff
                    retry_count += 1;
                    if retry_count >= self.max_retry {
                        bail!("Max retries exceeded");
                    }
                    eprintln!(
                        "Connection error, retry {}/{}: {}",
                        retry_count, self.max_retry, e
                    );
                    // Exponential backoff: 500ms, 1000ms, 1500ms, ...
                    tokio::time::sleep(Duration::from_millis(500 * retry_count as u64)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(received)
    }

    /// Get the total size of the remote file.
    ///
    /// Returns the Content-Length value obtained during construction.
    fn size(&self) -> u64 {
        self.size
    }
}
