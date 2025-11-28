use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

use anyhow::{bail, Result};

/// ZIP compression methods
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMethod {
    Stored,
    Deflate,
    Unknown(u16),
}

impl CompressionMethod {
    pub fn from_u16(value: u16) -> Self {
        match value {
            0 => CompressionMethod::Stored,
            8 => CompressionMethod::Deflate,
            _ => CompressionMethod::Unknown(value),
        }
    }

    pub fn as_u16(&self) -> u16 {
        match self {
            CompressionMethod::Stored => 0,
            CompressionMethod::Deflate => 8,
            CompressionMethod::Unknown(v) => *v,
        }
    }
}

/// End of Central Directory (EOCD) - 22 bytes minimum
pub struct EndOfCentralDirectory {
    pub disk_number: u16,
    pub disk_with_cd: u16,
    pub disk_entries: u16,
    pub total_entries: u16,
    pub cd_size: u32,
    pub cd_offset: u32,
    pub comment_len: u16,
}

impl EndOfCentralDirectory {
    pub const SIGNATURE: &'static [u8] = b"PK\x05\x06";
    pub const SIZE: usize = 22;

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            bail!("Invalid End of Central Directory");
        }

        // Verify signature
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

    pub fn is_zip64(&self) -> bool {
        self.disk_entries == 0xFFFF
            || self.total_entries == 0xFFFF
            || self.cd_size == 0xFFFFFFFF
            || self.cd_offset == 0xFFFFFFFF
    }
}

/// ZIP64 End of Central Directory Locator - 20 bytes
pub struct Zip64EOCDLocator {
    pub disk_with_eocd64: u32,
    pub eocd64_offset: u64,
    pub total_disks: u32,
}

impl Zip64EOCDLocator {
    pub const SIGNATURE: &'static [u8] = b"PK\x06\x07";
    pub const SIZE: usize = 20;

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

/// ZIP64 End of Central Directory - 56 bytes minimum
pub struct Zip64EOCD {
    pub eocd64_size: u64,
    pub version_made_by: u16,
    pub version_needed: u16,
    pub disk_number: u32,
    pub disk_with_cd: u32,
    pub disk_entries: u64,
    pub total_entries: u64,
    pub cd_size: u64,
    pub cd_offset: u64,
}

impl Zip64EOCD {
    pub const SIGNATURE: &'static [u8] = b"PK\x06\x06";
    pub const MIN_SIZE: usize = 56;

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

/// Central Directory File Header (CDFH) - 46 bytes minimum
pub const CDFH_SIGNATURE: &[u8] = b"PK\x01\x02";
pub const CDFH_MIN_SIZE: usize = 46;

/// Local File Header (LFH) - 30 bytes
pub const LFH_SIGNATURE: &[u8] = b"PK\x03\x04";
pub const LFH_SIZE: usize = 30;

/// Parsed ZIP file entry information
#[derive(Debug, Clone)]
pub struct ZipFileEntry {
    pub file_name: String,
    pub compression_method: CompressionMethod,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub lfh_offset: u64,
    pub last_mod_time: u16,
    pub last_mod_date: u16,
    pub is_directory: bool,
}

impl ZipFileEntry {
    /// Parse modification date to (year, month, day)
    pub fn mod_date(&self) -> (u16, u8, u8) {
        let day = (self.last_mod_date & 0x1F) as u8;
        let month = ((self.last_mod_date >> 5) & 0x0F) as u8;
        let year = ((self.last_mod_date >> 9) & 0x7F) + 1980;
        (year, month, day)
    }

    /// Parse modification time to (hour, minute, second)
    pub fn mod_time(&self) -> (u8, u8, u8) {
        let second = ((self.last_mod_time & 0x1F) * 2) as u8;
        let minute = ((self.last_mod_time >> 5) & 0x3F) as u8;
        let hour = ((self.last_mod_time >> 11) & 0x1F) as u8;
        (hour, minute, second)
    }
}
