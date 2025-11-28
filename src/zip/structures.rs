//! ZIP file format data structures.
//!
//! This module defines the data structures that represent the various
//! components of a ZIP file according to the PKZIP APPNOTE specification.
//!
//! ## ZIP File Layout
//!
//! ```text
//! [Local File Header 1]
//! [File Data 1]
//! [Local File Header 2]
//! [File Data 2]
//! ...
//! [Central Directory File Header 1]
//! [Central Directory File Header 2]
//! ...
//! [ZIP64 End of Central Directory Record] (optional)
//! [ZIP64 End of Central Directory Locator] (optional)
//! [End of Central Directory Record]
//! ```

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

use anyhow::{Result, bail};

/// ZIP compression methods.
///
/// ZIP supports various compression methods, identified by a 16-bit integer.
/// This enum represents the methods supported by this implementation.
///
/// ## Supported Methods
///
/// - `Stored` (0): No compression, data is stored as-is
/// - `Deflate` (8): DEFLATE compression (RFC 1951)
///
/// ## Unsupported Methods
///
/// Other common methods that are NOT supported:
/// - Shrunk (1), Reduced (2-5), Imploded (6), Tokenized (7)
/// - BZIP2 (12), LZMA (14), IBM TERSE (18), LZ77 (19)
/// - Zstandard (93), MP3 (94), XZ (95), JPEG (96), WavPack (97), PPMd (98)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMethod {
    /// No compression (method 0)
    Stored,
    /// DEFLATE compression (method 8)
    Deflate,
    /// Unknown or unsupported compression method
    Unknown(u16),
}

impl CompressionMethod {
    /// Convert a raw method ID to a CompressionMethod variant.
    ///
    /// # Arguments
    ///
    /// * `value` - The 16-bit compression method identifier
    ///
    /// # Returns
    ///
    /// The corresponding CompressionMethod variant.
    pub fn from_u16(value: u16) -> Self {
        match value {
            0 => CompressionMethod::Stored,
            8 => CompressionMethod::Deflate,
            _ => CompressionMethod::Unknown(value),
        }
    }

    /// Convert a CompressionMethod variant to its raw method ID.
    ///
    /// # Returns
    ///
    /// The 16-bit compression method identifier.
    pub fn as_u16(&self) -> u16 {
        match self {
            CompressionMethod::Stored => 0,
            CompressionMethod::Deflate => 8,
            CompressionMethod::Unknown(v) => *v,
        }
    }
}

/// End of Central Directory (EOCD) record.
///
/// This structure appears at the very end of a ZIP file and contains
/// pointers to the Central Directory. Finding this structure is the
/// first step in reading a ZIP file.
///
/// ## Structure (22 bytes minimum)
///
/// | Offset | Size | Description |
/// |--------|------|-------------|
/// | 0 | 4 | Signature (0x06054b50) |
/// | 4 | 2 | Disk number |
/// | 6 | 2 | Disk with Central Directory |
/// | 8 | 2 | Entries on this disk |
/// | 10 | 2 | Total entries |
/// | 12 | 4 | Central Directory size |
/// | 16 | 4 | Central Directory offset |
/// | 20 | 2 | Comment length |
/// | 22 | n | Comment (variable) |
pub struct EndOfCentralDirectory {
    /// Number of this disk
    pub disk_number: u16,
    /// Disk where Central Directory starts
    pub disk_with_cd: u16,
    /// Number of Central Directory entries on this disk
    pub disk_entries: u16,
    /// Total number of Central Directory entries
    pub total_entries: u16,
    /// Size of the Central Directory in bytes
    pub cd_size: u32,
    /// Offset to start of Central Directory
    pub cd_offset: u32,
    /// Length of the ZIP file comment
    pub comment_len: u16,
}

impl EndOfCentralDirectory {
    /// EOCD signature bytes: "PK\x05\x06"
    pub const SIGNATURE: &'static [u8] = b"PK\x05\x06";
    /// Minimum size of EOCD record (without comment)
    pub const SIZE: usize = 22;

    /// Parse an EOCD record from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `data` - Byte slice containing the EOCD record
    ///
    /// # Returns
    ///
    /// The parsed EOCD structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too short or has an invalid signature.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            bail!("Invalid End of Central Directory");
        }

        // Verify signature: PK\x05\x06
        if &data[0..4] != Self::SIGNATURE {
            bail!("Invalid End of Central Directory");
        }

        let mut cursor = Cursor::new(&data[4..]);

        Ok(Self {
            disk_number: cursor.read_u16::<LittleEndian>()?,
            disk_with_cd: cursor.read_u16::<LittleEndian>()?,
            disk_entries: cursor.read_u16::<LittleEndian>()?,
            total_entries: cursor.read_u16::<LittleEndian>()?,
            cd_size: cursor.read_u32::<LittleEndian>()?,
            cd_offset: cursor.read_u32::<LittleEndian>()?,
            comment_len: cursor.read_u16::<LittleEndian>()?,
        })
    }

    /// Check if this archive requires ZIP64 extensions.
    ///
    /// ZIP64 is needed when any of the following fields have their
    /// maximum value (indicating the real value is in the ZIP64 EOCD):
    /// - Entry count is 0xFFFF
    /// - Central Directory size is 0xFFFFFFFF
    /// - Central Directory offset is 0xFFFFFFFF
    ///
    /// # Returns
    ///
    /// `true` if ZIP64 EOCD should be read for accurate values.
    pub fn is_zip64(&self) -> bool {
        self.disk_entries == 0xFFFF
            || self.total_entries == 0xFFFF
            || self.cd_size == 0xFFFFFFFF
            || self.cd_offset == 0xFFFFFFFF
    }
}

/// ZIP64 End of Central Directory Locator.
///
/// This structure helps locate the ZIP64 EOCD record. It appears
/// immediately before the regular EOCD in ZIP64 archives.
///
/// ## Structure (20 bytes)
///
/// | Offset | Size | Description |
/// |--------|------|-------------|
/// | 0 | 4 | Signature (0x07064b50) |
/// | 4 | 4 | Disk with ZIP64 EOCD |
/// | 8 | 8 | ZIP64 EOCD offset |
/// | 16 | 4 | Total number of disks |
pub struct Zip64EOCDLocator {
    /// Disk number containing ZIP64 EOCD
    pub disk_with_eocd64: u32,
    /// Absolute offset to ZIP64 EOCD
    pub eocd64_offset: u64,
    /// Total number of disks
    pub total_disks: u32,
}

impl Zip64EOCDLocator {
    /// ZIP64 EOCD Locator signature: "PK\x06\x07"
    pub const SIGNATURE: &'static [u8] = b"PK\x06\x07";
    /// Size of the locator record
    pub const SIZE: usize = 20;

    /// Parse a ZIP64 EOCD Locator from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `data` - Byte slice containing the locator record
    ///
    /// # Returns
    ///
    /// The parsed locator structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too short or has an invalid signature.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            bail!("Invalid ZIP64 format");
        }

        if &data[0..4] != Self::SIGNATURE {
            bail!("Invalid ZIP64 format");
        }

        let mut cursor = Cursor::new(&data[4..]);

        Ok(Self {
            disk_with_eocd64: cursor.read_u32::<LittleEndian>()?,
            eocd64_offset: cursor.read_u64::<LittleEndian>()?,
            total_disks: cursor.read_u32::<LittleEndian>()?,
        })
    }
}

/// ZIP64 End of Central Directory record.
///
/// This structure contains 64-bit versions of the fields that overflow
/// in the regular EOCD for large archives (>4GB or >65535 files).
///
/// ## Structure (56 bytes minimum)
///
/// | Offset | Size | Description |
/// |--------|------|-------------|
/// | 0 | 4 | Signature (0x06064b50) |
/// | 4 | 8 | Size of ZIP64 EOCD (excluding signature and this field) |
/// | 12 | 2 | Version made by |
/// | 14 | 2 | Version needed to extract |
/// | 16 | 4 | Disk number |
/// | 20 | 4 | Disk with Central Directory |
/// | 24 | 8 | Entries on this disk |
/// | 32 | 8 | Total entries |
/// | 40 | 8 | Central Directory size |
/// | 48 | 8 | Central Directory offset |
pub struct Zip64EOCD {
    /// Size of this record (excluding first 12 bytes)
    pub eocd64_size: u64,
    /// Version that created the archive
    pub version_made_by: u16,
    /// Minimum version needed to extract
    pub version_needed: u16,
    /// Number of this disk
    pub disk_number: u32,
    /// Disk with Central Directory start
    pub disk_with_cd: u32,
    /// Entries on this disk
    pub disk_entries: u64,
    /// Total number of entries
    pub total_entries: u64,
    /// Central Directory size in bytes
    pub cd_size: u64,
    /// Offset to Central Directory
    pub cd_offset: u64,
}

impl Zip64EOCD {
    /// ZIP64 EOCD signature: "PK\x06\x06"
    pub const SIGNATURE: &'static [u8] = b"PK\x06\x06";
    /// Minimum size of ZIP64 EOCD record
    pub const MIN_SIZE: usize = 56;

    /// Parse a ZIP64 EOCD from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `data` - Byte slice containing the ZIP64 EOCD record
    ///
    /// # Returns
    ///
    /// The parsed ZIP64 EOCD structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too short or has an invalid signature.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::MIN_SIZE {
            bail!("Invalid ZIP64 format");
        }

        if &data[0..4] != Self::SIGNATURE {
            bail!("Invalid ZIP64 format");
        }

        let mut cursor = Cursor::new(&data[4..]);

        Ok(Self {
            eocd64_size: cursor.read_u64::<LittleEndian>()?,
            version_made_by: cursor.read_u16::<LittleEndian>()?,
            version_needed: cursor.read_u16::<LittleEndian>()?,
            disk_number: cursor.read_u32::<LittleEndian>()?,
            disk_with_cd: cursor.read_u32::<LittleEndian>()?,
            disk_entries: cursor.read_u64::<LittleEndian>()?,
            total_entries: cursor.read_u64::<LittleEndian>()?,
            cd_size: cursor.read_u64::<LittleEndian>()?,
            cd_offset: cursor.read_u64::<LittleEndian>()?,
        })
    }
}

/// Central Directory File Header signature: "PK\x01\x02"
pub const CDFH_SIGNATURE: &[u8] = b"PK\x01\x02";

/// Minimum size of Central Directory File Header (46 bytes)
pub const CDFH_MIN_SIZE: usize = 46;

/// Local File Header signature: "PK\x03\x04"
pub const LFH_SIGNATURE: &[u8] = b"PK\x03\x04";

/// Size of Local File Header (30 bytes, fixed portion)
pub const LFH_SIZE: usize = 30;

/// Parsed ZIP file entry information.
///
/// This structure contains all the metadata needed to extract a file
/// from a ZIP archive, parsed from the Central Directory.
///
/// ## Example
///
/// ```ignore
/// for entry in extractor.list_files().await? {
///     println!("{}: {} bytes (compressed: {})",
///         entry.file_name,
///         entry.uncompressed_size,
///         entry.compressed_size
///     );
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ZipFileEntry {
    /// The file name (may include path components)
    pub file_name: String,
    /// Compression method used for this entry
    pub compression_method: CompressionMethod,
    /// Size of compressed data in bytes
    pub compressed_size: u64,
    /// Size of uncompressed data in bytes
    pub uncompressed_size: u64,
    /// CRC-32 checksum of uncompressed data
    pub crc32: u32,
    /// Offset to Local File Header from start of archive
    pub lfh_offset: u64,
    /// Last modification time in DOS format
    pub last_mod_time: u16,
    /// Last modification date in DOS format
    pub last_mod_date: u16,
    /// True if this entry represents a directory
    pub is_directory: bool,
}

impl ZipFileEntry {
    /// Parse the modification date from DOS format.
    ///
    /// DOS date format packs year, month, and day into 16 bits:
    /// - Bits 0-4: Day (1-31)
    /// - Bits 5-8: Month (1-12)
    /// - Bits 9-15: Year offset from 1980
    ///
    /// # Returns
    ///
    /// A tuple of (year, month, day).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (year, month, day) = entry.mod_date();
    /// println!("Modified: {}-{:02}-{:02}", year, month, day);
    /// ```
    pub fn mod_date(&self) -> (u16, u8, u8) {
        let day = (self.last_mod_date & 0x1F) as u8;
        let month = ((self.last_mod_date >> 5) & 0x0F) as u8;
        let year = ((self.last_mod_date >> 9) & 0x7F) + 1980;
        (year, month, day)
    }

    /// Parse the modification time from DOS format.
    ///
    /// DOS time format packs hour, minute, and second into 16 bits:
    /// - Bits 0-4: Second / 2 (0-29, representing 0-58 seconds)
    /// - Bits 5-10: Minute (0-59)
    /// - Bits 11-15: Hour (0-23)
    ///
    /// # Returns
    ///
    /// A tuple of (hour, minute, second).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (hour, minute, second) = entry.mod_time();
    /// println!("Time: {:02}:{:02}:{:02}", hour, minute, second);
    /// ```
    pub fn mod_time(&self) -> (u8, u8, u8) {
        let second = ((self.last_mod_time & 0x1F) * 2) as u8;
        let minute = ((self.last_mod_time >> 5) & 0x3F) as u8;
        let hour = ((self.last_mod_time >> 11) & 0x1F) as u8;
        (hour, minute, second)
    }
}
