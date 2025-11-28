use async_trait::async_trait;
use reqwest::Client;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::ReadAt;
use anyhow::{anyhow, bail, Result};

/// HTTP Range reader for remote ZIP files
pub struct HttpRangeReader {
    client: Client,
    url: String,
    size: u64,
    transferred_bytes: AtomicU64,
    max_retry: u32,
}

impl HttpRangeReader {
    /// Create a new HTTP Range reader
    ///
    /// This will send a HEAD request to verify Range support and get file size
    pub async fn new(url: String) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

        // Send HEAD request to check capabilities
        let resp = client.head(&url).send().await?;

        if !resp.status().is_success() {
            bail!("HTTP request failed with status: {}", resp.status());
        }

        // Check if server supports Range requests
        let accept_ranges = resp
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none");

        if !accept_ranges.contains("bytes") {
            bail!("Remote server does not support Range requests");
        }

        // Get file size from Content-Length
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

    /// Get total bytes transferred from network
    pub fn transferred_bytes(&self) -> u64 {
        self.transferred_bytes.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ReadAt for HttpRangeReader {
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let end = offset + buf.len() as u64 - 1;
        let end = end.min(self.size - 1);
        let expected_size = (end - offset + 1) as usize;

        let mut received = 0;
        let mut retry_count = 0;

        while received < expected_size {
            let current_start = offset + received as u64;
            let range = format!("bytes={}-{}", current_start, end);

            let result = self
                .client
                .get(&self.url)
                .header("Range", &range)
                .send()
                .await;

            match result {
                Ok(resp) => {
                    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                        bail!("HTTP request failed with status: {}", resp.status());
                    }

                    let bytes = resp.bytes().await?;
                    let chunk_len = bytes.len().min(expected_size - received);
                    buf[received..received + chunk_len].copy_from_slice(&bytes[..chunk_len]);
                    received += chunk_len;

                    self.transferred_bytes
                        .fetch_add(chunk_len as u64, Ordering::Relaxed);
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    retry_count += 1;
                    if retry_count >= self.max_retry {
                        bail!("Max retries exceeded");
                    }
                    eprintln!(
                        "Connection error, retry {}/{}: {}",
                        retry_count, self.max_retry, e
                    );
                    tokio::time::sleep(Duration::from_millis(500 * retry_count as u64)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(received)
    }

    fn size(&self) -> u64 {
        self.size
    }
}
