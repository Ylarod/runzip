//! # runzip
//!
//! A Rust unzip utility with HTTP URL support using Range requests.
//!
//! This library provides functionality to extract ZIP files from both local filesystem
//! and remote HTTP servers. For remote files, it uses HTTP Range requests to efficiently
//! download only the necessary parts of the archive, making it suitable for extracting
//! specific files from large remote archives without downloading the entire file.
//!
//! ## Features
//!
//! - Extract ZIP files from local filesystem
//! - Extract ZIP files from HTTP/HTTPS URLs using Range requests
//! - Support for ZIP64 format (archives larger than 4GB)
//! - Support for STORED (uncompressed) and DEFLATE compression methods
//! - Selective file extraction with glob pattern matching
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use runzip::{HttpRangeReader, ZipExtractor};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create a reader for a remote ZIP file
//!     let reader = Arc::new(HttpRangeReader::new("https://example.com/archive.zip".to_string()).await?);
//!
//!     // Create an extractor
//!     let extractor = ZipExtractor::new(reader);
//!
//!     // List all files in the archive
//!     let files = extractor.list_files().await?;
//!     for file in &files {
//!         println!("{}", file.file_name);
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod cli;
pub mod io;
pub mod zip;

pub use cli::Cli;
pub use io::{HttpRangeReader, LocalFileReader, ReadAt};
pub use zip::{ZipExtractor, ZipFileEntry};
