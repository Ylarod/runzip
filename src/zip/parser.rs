use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::sync::Arc;

use crate::io::ReadAt;
use anyhow::{bail, Result};

use super::structures::*;

/// Maximum ZIP comment size
const MAX_COMMENT_SIZE: u64 = 65535;

/// ZIP file parser
pub struct ZipParser<R: ReadAt> {
    reader: Arc<R>,
    size: u64,
}

impl<R: ReadAt> ZipParser<R> {
    pub fn new(reader: Arc<R>) -> Self {
        let size = reader.size();
        Self { reader, size }
    }

    /// Find and parse End of Central Directory
    pub async fn find_eocd(&self) -> Result<(EndOfCentralDirectory, u64)> {
        // First, try the simple case: no comment
        if self.size >= EndOfCentralDirectory::SIZE as u64 {
            let offset = self.size - EndOfCentralDirectory::SIZE as u64;
            let mut buf = vec![0u8; EndOfCentralDirectory::SIZE];
            self.reader.read_at(offset, &mut buf).await?;

            // Check signature and zero comment length
            if &buf[0..4] == EndOfCentralDirectory::SIGNATURE && &buf[20..22] == b"\x00\x00" {
                let eocd = EndOfCentralDirectory::from_bytes(&buf)?;
                return Ok((eocd, offset));
            }
        }

        // Search for EOCD with comment
        let search_size = (MAX_COMMENT_SIZE + EndOfCentralDirectory::SIZE as u64).min(self.size);
        let search_start = self.size - search_size;

        let mut buf = vec![0u8; search_size as usize];
        self.reader.read_at(search_start, &mut buf).await?;

        // Search backwards for EOCD signature
        for i in (0..buf.len().saturating_sub(EndOfCentralDirectory::SIZE)).rev() {
            if &buf[i..i + 4] == EndOfCentralDirectory::SIGNATURE {
                // Verify comment length matches remaining bytes
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

    /// Read ZIP64 End of Central Directory
    pub async fn read_zip64_eocd(&self, eocd_offset: u64) -> Result<Zip64EOCD> {
        // Read ZIP64 EOCD Locator (located right before EOCD)
        let locator_offset = eocd_offset - Zip64EOCDLocator::SIZE as u64;
        let mut locator_buf = vec![0u8; Zip64EOCDLocator::SIZE];
        self.reader
            .read_at(locator_offset, &mut locator_buf)
            .await?;

        let locator = Zip64EOCDLocator::from_bytes(&locator_buf)?;

        // Read ZIP64 EOCD
        let mut eocd64_buf = vec![0u8; Zip64EOCD::MIN_SIZE];
        self.reader
            .read_at(locator.eocd64_offset, &mut eocd64_buf)
            .await?;

        Zip64EOCD::from_bytes(&eocd64_buf)
    }

    /// List all files in the ZIP archive
    pub async fn list_files(&self) -> Result<Vec<ZipFileEntry>> {
        let (eocd, eocd_offset) = self.find_eocd().await?;

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

        // Read entire Central Directory
        let mut cd_data = vec![0u8; cd_size as usize];
        self.reader.read_at(cd_offset, &mut cd_data).await?;

        // Parse each entry
        let mut entries = Vec::with_capacity(total_entries as usize);
        let mut cursor = Cursor::new(&cd_data);

        for _ in 0..total_entries {
            let entry = self.parse_cdfh(&mut cursor)?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Parse a Central Directory File Header
    fn parse_cdfh(&self, cursor: &mut Cursor<&Vec<u8>>) -> Result<ZipFileEntry> {
        // Read and verify signature
        let mut sig = [0u8; 4];
        cursor.read_exact(&mut sig)?;
        if sig != CDFH_SIGNATURE {
            bail!("Invalid Central Directory File Header");
        }

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

        // Read file name
        let mut file_name_bytes = vec![0u8; file_name_length as usize];
        cursor.read_exact(&mut file_name_bytes)?;
        let file_name = String::from_utf8_lossy(&file_name_bytes).to_string();

        // Check if directory
        let is_directory = file_name.ends_with('/');

        // Parse extra field for ZIP64 information
        let extra_field_end = cursor.position() + extra_field_length as u64;

        while cursor.position() + 4 <= extra_field_end {
            let header_id = cursor.read_u16::<LittleEndian>()?;
            let field_size = cursor.read_u16::<LittleEndian>()?;

            if header_id == 0x0001 {
                // ZIP64 extended information
                if uncompressed_size == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    uncompressed_size = cursor.read_u64::<LittleEndian>()?;
                }
                if compressed_size == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    compressed_size = cursor.read_u64::<LittleEndian>()?;
                }
                if lfh_offset == 0xFFFFFFFF && cursor.position() + 8 <= extra_field_end {
                    lfh_offset = cursor.read_u64::<LittleEndian>()?;
                }
                // Skip remaining ZIP64 fields
                let remaining = extra_field_end.saturating_sub(cursor.position());
                cursor.set_position(cursor.position() + remaining);
            } else {
                // Skip unknown extra field
                cursor.set_position(cursor.position() + field_size as u64);
            }
        }

        // Ensure we're at the end of extra field
        cursor.set_position(extra_field_end);

        // Skip file comment
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

    /// Get the actual data offset for a file entry
    pub async fn get_data_offset(&self, entry: &ZipFileEntry) -> Result<u64> {
        let mut lfh_buf = vec![0u8; LFH_SIZE];
        self.reader.read_at(entry.lfh_offset, &mut lfh_buf).await?;

        // Verify signature
        if &lfh_buf[0..4] != LFH_SIGNATURE {
            bail!("Invalid Local File Header");
        }

        let mut cursor = Cursor::new(&lfh_buf);
        cursor.set_position(26); // Skip to filename length

        let file_name_length = cursor.read_u16::<LittleEndian>()? as u64;
        let extra_field_length = cursor.read_u16::<LittleEndian>()? as u64;

        let data_offset =
            entry.lfh_offset + LFH_SIZE as u64 + file_name_length + extra_field_length;

        Ok(data_offset)
    }

    /// Get the underlying reader
    pub fn reader(&self) -> &Arc<R> {
        &self.reader
    }
}
