//! Low-level ZIP archive parser.
//!
//! This module handles the binary parsing of ZIP file structures,
//! reading from any source that implements the [`ReadAt`] trait.
//!
//! ## Parsing Strategy
//!
//! ZIP files are designed to be read from the end:
//! 1. Find the End of Central Directory (EOCD) at the file's end
//! 2. If ZIP64, read the ZIP64 EOCD for large file support
//! 3. Read the Central Directory to get metadata for all files
//! 4. For extraction, read each file's Local File Header and data
//!
//! This approach is efficient for HTTP Range requests, as we only
//! need to fetch the file's tail to list contents.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::io::ReadAt;
use anyhow::{Result, bail};

use super::structures::*;

/// Maximum ZIP comment size allowed by the format (65535 bytes).
///
/// This limits the search area when looking for EOCD with a comment.
const MAX_COMMENT_SIZE: u64 = 65535;

/// Low-level ZIP file parser.
///
/// This struct handles reading and parsing ZIP structures from
/// a data source. It's generic over the reader type to support
/// both local files and HTTP sources.
///
/// ## Usage
///
/// Typically used through [`ZipExtractor`](super::ZipExtractor)
/// rather than directly.
///
/// ## Example
///
/// ```ignore
/// let parser = ZipParser::new(reader);
/// let entries = parser.list_files().await?;
/// for entry in entries {
///     let offset = parser.get_data_offset(&entry).await?;
///     // Read file data from offset...
/// }
/// ```
pub struct ZipParser<R: ReadAt> {
    /// The underlying data source
    reader: Arc<R>,
    /// Total size of the archive in bytes
    size: u64,
}

impl<R: ReadAt> ZipParser<R> {
    /// Create a new parser for the given reader.
    ///
    /// # Arguments
    ///
    /// * `reader` - A shared reference to a reader implementing [`ReadAt`]
    ///
    /// # Returns
    ///
    /// A new parser instance ready to read the archive.
    pub fn new(reader: Arc<R>) -> Self {
        let size = reader.size();
        Self { reader, size }
    }

    /// Find and parse the End of Central Directory record.
    ///
    /// The EOCD is located at the end of the ZIP file. This method
    /// handles both the simple case (no comment) and archives with
    /// comments by searching backwards for the signature.
    ///
    /// # Returns
    ///
    /// A tuple of (EOCD record, offset of EOCD in file).
    ///
    /// # Errors
    ///
    /// Returns an error if no valid EOCD can be found, indicating
    /// the file is not a valid ZIP archive.
    pub async fn find_eocd(&self) -> Result<(EndOfCentralDirectory, u64)> {
        // Optimization: First try the simple case where there's no comment.
        // This avoids reading extra data in the common case.
        if self.size >= EndOfCentralDirectory::SIZE as u64 {
            let offset = self.size - EndOfCentralDirectory::SIZE as u64;
            let mut buf = vec![0u8; EndOfCentralDirectory::SIZE];
            self.reader.read_at(offset, &mut buf).await?;

            // Check for signature and zero-length comment
            if &buf[0..4] == EndOfCentralDirectory::SIGNATURE && &buf[20..22] == b"\x00\x00" {
                let eocd = EndOfCentralDirectory::from_bytes(&buf)?;
                return Ok((eocd, offset));
            }
        }

        // EOCD not at expected location - search for it.
        // The EOCD could be earlier if there's a ZIP comment.
        // We need to search backwards from the end of the file.
        let search_size = (MAX_COMMENT_SIZE + EndOfCentralDirectory::SIZE as u64).min(self.size);
        let search_start = self.size - search_size;

        let mut buf = vec![0u8; search_size as usize];
        self.reader.read_at(search_start, &mut buf).await?;

        // Search backwards for EOCD signature (PK\x05\x06)
        for i in (0..buf.len().saturating_sub(EndOfCentralDirectory::SIZE)).rev() {
            if &buf[i..i + 4] == EndOfCentralDirectory::SIGNATURE {
                // Found a potential EOCD - verify the comment length is correct.
                // The comment length field should match the remaining bytes.
                let comment_len = u16::from_le_bytes([buf[i + 20], buf[i + 21]]) as usize;

                if comment_len == buf.len() - i - EndOfCentralDirectory::SIZE {
                    let eocd = EndOfCentralDirectory::from_bytes(
                        &buf[i..i + EndOfCentralDirectory::SIZE],
                    )?;
                    return Ok((eocd, search_start + i as u64));
                }
            }
        }

        bail!("Not a valid ZIP file")
    }

    /// Read the ZIP64 End of Central Directory record.
    ///
    /// Called when the regular EOCD indicates ZIP64 extensions are needed
    /// (fields set to 0xFFFF or 0xFFFFFFFF).
    ///
    /// # Arguments
    ///
    /// * `eocd_offset` - Offset of the regular EOCD in the file
    ///
    /// # Returns
    ///
    /// The parsed ZIP64 EOCD with 64-bit field values.
    ///
    /// # Errors
    ///
    /// Returns an error if the ZIP64 structures are missing or invalid.
    pub async fn read_zip64_eocd(&self, eocd_offset: u64) -> Result<Zip64EOCD> {
        // The ZIP64 EOCD Locator is located immediately before the regular EOCD
        let locator_offset = eocd_offset - Zip64EOCDLocator::SIZE as u64;
        let mut locator_buf = vec![0u8; Zip64EOCDLocator::SIZE];
        self.reader
            .read_at(locator_offset, &mut locator_buf)
            .await?;

        let locator = Zip64EOCDLocator::from_bytes(&locator_buf)?;

        // Read the actual ZIP64 EOCD from the offset specified in the locator
        let mut eocd64_buf = vec![0u8; Zip64EOCD::MIN_SIZE];
        self.reader
            .read_at(locator.eocd64_offset, &mut eocd64_buf)
            .await?;

        Zip64EOCD::from_bytes(&eocd64_buf)
    }

    /// List all files in the ZIP archive.
    ///
    /// Reads the Central Directory to get metadata for all entries.
    /// This method reads the EOCD first, then fetches and parses the
    /// entire Central Directory.
    ///
    /// # Returns
    ///
    /// A vector of [`ZipFileEntry`] structures, one for each file/directory
    /// in the archive.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive is invalid or cannot be read.
    pub async fn list_files(&self) -> Result<Vec<ZipFileEntry>> {
        // Find and parse the EOCD to get Central Directory location
        let (eocd, eocd_offset) = self.find_eocd().await?;

        // Get Central Directory info, using ZIP64 if needed
        let (cd_offset, cd_size, total_entries) = if eocd.is_zip64() {
            let eocd64 = self.read_zip64_eocd(eocd_offset).await?;
            (eocd64.cd_offset, eocd64.cd_size, eocd64.total_entries)
        } else {
            (
                eocd.cd_offset as u64,
                eocd.cd_size as u64,
                eocd.total_entries as u64,
            )
        };

        // Read the entire Central Directory in one request
        // (efficient for HTTP as it's a single Range request)
        let mut cd_data = vec![0u8; cd_size as usize];
        self.reader.read_at(cd_offset, &mut cd_data).await?;

        // Parse each Central Directory File Header entry
        let mut entries = Vec::with_capacity(total_entries as usize);
        let mut cursor = Cursor::new(&cd_data);

        for _ in 0..total_entries {
            let entry = self.parse_cdfh(&mut cursor)?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Parse a Central Directory File Header from a cursor.
    ///
    /// The CDFH contains metadata about a file in the archive, including
    /// its name, sizes, and location of the actual file data.
    ///
    /// # Arguments
    ///
    /// * `cursor` - A cursor positioned at the start of a CDFH
    ///
    /// # Returns
    ///
    /// A parsed [`ZipFileEntry`] with all file metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the header is invalid.
    fn parse_cdfh(&self, cursor: &mut Cursor<&Vec<u8>>) -> Result<ZipFileEntry> {
        // Read and verify the signature (PK\x01\x02)
        let mut sig = [0u8; 4];
        cursor.read_exact(&mut sig)?;
        if sig != CDFH_SIGNATURE {
            bail!("Invalid Central Directory File Header");
        }

        // Read fixed-size header fields
        let _version_made_by = cursor.read_u16::<LittleEndian>()?;
        let _version_needed = cursor.read_u16::<LittleEndian>()?;
        let _flags = cursor.read_u16::<LittleEndian>()?;
        let compression_method = cursor.read_u16::<LittleEndian>()?;
        let last_mod_time = cursor.read_u16::<LittleEndian>()?;
        let last_mod_date = cursor.read_u16::<LittleEndian>()?;
        let crc32 = cursor.read_u32::<LittleEndian>()?;
        let mut compressed_size = cursor.read_u32::<LittleEndian>()? as u64;
        let mut uncompressed_size = cursor.read_u32::<LittleEndian>()? as u64;
        let file_name_length = cursor.read_u16::<LittleEndian>()?;
        let extra_field_length = cursor.read_u16::<LittleEndian>()?;
        let file_comment_length = cursor.read_u16::<LittleEndian>()?;
        let _disk_number_start = cursor.read_u16::<LittleEndian>()?;
        let _internal_attrs = cursor.read_u16::<LittleEndian>()?;
        let _external_attrs = cursor.read_u32::<LittleEndian>()?;
        let mut lfh_offset = cursor.read_u32::<LittleEndian>()? as u64;

        // Read the variable-length file name
        let mut file_name_bytes = vec![0u8; file_name_length as usize];
        cursor.read_exact(&mut file_name_bytes)?;
        // Use lossy conversion to handle non-UTF8 filenames gracefully
        let file_name = String::from_utf8_lossy(&file_name_bytes).to_string();

        // Directory entries end with '/'
        let is_directory = file_name.ends_with('/');

        // Parse extra field for ZIP64 extended information
        // ZIP64 uses extra field ID 0x0001
        let extra_field_end = cursor.position() + extra_field_length as u64;

        while cursor.position() + 4 <= extra_field_end {
            let header_id = cursor.read_u16::<LittleEndian>()?;
            let field_size = cursor.read_u16::<LittleEndian>()?;

            if header_id == 0x0001 {
                // ZIP64 extended information extra field
                // Fields are present only if corresponding header field is 0xFFFFFFFF
                if uncompressed_size == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    uncompressed_size = cursor.read_u64::<LittleEndian>()?;
                }
                if compressed_size == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    compressed_size = cursor.read_u64::<LittleEndian>()?;
                }
                if lfh_offset == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    lfh_offset = cursor.read_u64::<LittleEndian>()?;
                }
                // Skip any remaining ZIP64 fields (disk number start)
                let remaining = extra_field_end.saturating_sub(cursor.position());
                cursor.set_position(cursor.position() + remaining);
            } else {
                // Skip unknown extra fields
                cursor.set_position(cursor.position() + field_size as u64);
            }
        }

        // Ensure cursor is positioned after extra field
        cursor.set_position(extra_field_end);

        // Skip over the file comment (we don't use it)
        cursor.set_position(cursor.position() + file_comment_length as u64);

        Ok(ZipFileEntry {
            file_name,
            compression_method: CompressionMethod::from_u16(compression_method),
            compressed_size,
            uncompressed_size,
            crc32,
            lfh_offset,
            last_mod_time,
            last_mod_date,
            is_directory,
        })
    }

    /// Get the actual data offset for a file entry.
    ///
    /// The Local File Header (LFH) has variable-length fields (filename,
    /// extra field) that may differ from the Central Directory entry.
    /// This method reads the LFH to calculate where the actual file
    /// data begins.
    ///
    /// # Arguments
    ///
    /// * `entry` - The file entry from [`list_files()`]
    ///
    /// # Returns
    ///
    /// The byte offset where the compressed file data begins.
    ///
    /// # Errors
    ///
    /// Returns an error if the LFH is invalid.
    pub async fn get_data_offset(&self, entry: &ZipFileEntry) -> Result<u64> {
        // Read the Local File Header
        let mut lfh_buf = vec![0u8; LFH_SIZE];
        self.reader.read_at(entry.lfh_offset, &mut lfh_buf).await?;

        // Verify LFH signature (PK\x03\x04)
        if &lfh_buf[0..4] != LFH_SIGNATURE {
            bail!("Invalid Local File Header");
        }

        // Read the variable field lengths from fixed positions in LFH
        let mut cursor = Cursor::new(&lfh_buf);
        cursor.set_position(26); // Offset to filename length field

        let file_name_length = cursor.read_u16::<LittleEndian>()? as u64;
        let extra_field_length = cursor.read_u16::<LittleEndian>()? as u64;

        // Data starts after: LFH (30 bytes) + filename + extra field
        let data_offset =
            entry.lfh_offset + LFH_SIZE as u64 + file_name_length + extra_field_length;

        Ok(data_offset)
    }

    /// Get a reference to the underlying reader.
    ///
    /// Useful for reading file data after getting the offset
    /// from [`get_data_offset()`].
    ///
    /// # Returns
    ///
    /// A shared reference to the reader.
    pub fn reader(&self) -> &Arc<R> {
        &self.reader
    }
}
