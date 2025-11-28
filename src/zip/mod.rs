//! ZIP archive parsing and extraction.
//!
//! This module provides functionality for reading and extracting ZIP archives,
//! supporting both standard ZIP format and ZIP64 extensions for large archives.
//!
//! ## Architecture
//!
//! The module is organized into three main components:
//!
//! - [`structures`]: Data structures representing ZIP format elements (EOCD, file headers, etc.)
//! - [`parser`]: Low-level parsing of ZIP structures from raw bytes
//! - [`extractor`]: High-level extraction API for end users
//!
//! ## ZIP Format Overview
//!
//! A ZIP file consists of:
//! 1. Local file headers and compressed data for each file
//! 2. Central Directory with metadata for all files
//! 3. End of Central Directory (EOCD) record at the end
//!
//! This implementation reads the EOCD first (from the end of the file),
//! then the Central Directory, which allows listing files without reading
//! the entire archive - perfect for HTTP Range requests.
//!
//! ## Supported Features
//!
//! - Standard ZIP format (PKZIP APPNOTE 6.3.x compatible)
//! - ZIP64 extensions for files > 4GB
//! - STORED (no compression) method
//! - DEFLATE compression method
//!
//! ## Limitations
//!
//! - No encryption support
//! - No multi-disk archive support
//! - No BZIP2, LZMA, or other compression methods

mod extractor;
mod parser;
mod structures;

pub use extractor::ZipExtractor;
pub use parser::ZipParser;
pub use structures::*;
