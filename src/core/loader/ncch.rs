use crate::core::error::{EmulatorError, Result};

use super::exheader::ExHeader;

const NCCH_HEADER_SIZE: usize = 0x200;
const NCCH_MAGIC: &[u8; 4] = b"NCCH";
const MEDIA_UNIT_SIZE: usize = 0x200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomRegion {
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcchProgram {
    pub entrypoint: u32,
    pub exheader: ExHeader,
    pub exefs_region: RomRegion,
    pub romfs_region: Option<RomRegion>,
}

pub fn parse_ncch_program(raw: &[u8], ncch_offset: usize, ncch_size: usize) -> Result<NcchProgram> {
    if ncch_size < NCCH_HEADER_SIZE {
        return Err(EmulatorError::InvalidNcchHeader);
    }
    let header = raw
        .get(ncch_offset..ncch_offset + NCCH_HEADER_SIZE)
        .ok_or(EmulatorError::InvalidNcchHeader)?;

    if &header[0x100..0x104] != NCCH_MAGIC {
        return Err(EmulatorError::InvalidNcchHeader);
    }

    let flags = header[0x188 + 7];
    if flags & 0x04 != 0 || flags & 0x20 != 0 {
        return Err(EmulatorError::UnsupportedNcchCrypto);
    }

    let exheader_size = read_u32(header, 0x180)? as usize;
    if exheader_size < 0x200 {
        return Err(EmulatorError::InvalidExHeader);
    }

    let exheader_offset = ncch_offset + NCCH_HEADER_SIZE;
    let exheader_end = exheader_offset
        .checked_add(exheader_size)
        .ok_or(EmulatorError::InvalidNcchHeader)?;
    let ncch_end = ncch_offset + ncch_size;
    if exheader_end > ncch_end || exheader_end > raw.len() {
        return Err(EmulatorError::InvalidNcchHeader);
    }

    let exheader = ExHeader::parse(&raw[exheader_offset..exheader_end])?;
    let entrypoint = read_u32(&raw[exheader_offset..exheader_end], 0)?;
    let exefs_region = parse_section_region(header, ncch_offset, ncch_end, 0x1A8)?;
    let romfs_region = parse_optional_section_region(header, ncch_offset, ncch_end, 0x1B0)?;

    Ok(NcchProgram {
        entrypoint,
        exheader,
        exefs_region,
        romfs_region,
    })
}

fn parse_optional_section_region(
    ncch_header: &[u8],
    ncch_offset: usize,
    ncch_end: usize,
    field_offset: usize,
) -> Result<Option<RomRegion>> {
    let offset_units = read_u32(ncch_header, field_offset)? as usize;
    let size_units = read_u32(ncch_header, field_offset + 4)? as usize;
    if offset_units == 0 || size_units == 0 {
        return Ok(None);
    }
    parse_section_region(ncch_header, ncch_offset, ncch_end, field_offset).map(Some)
}

fn parse_section_region(
    ncch_header: &[u8],
    ncch_offset: usize,
    ncch_end: usize,
    field_offset: usize,
) -> Result<RomRegion> {
    let offset_units = read_u32(ncch_header, field_offset)? as usize;
    let size_units = read_u32(ncch_header, field_offset + 4)? as usize;
    let offset = ncch_offset + offset_units * MEDIA_UNIT_SIZE;
    let size = size_units * MEDIA_UNIT_SIZE;
    let end = offset
        .checked_add(size)
        .ok_or(EmulatorError::InvalidNcchHeader)?;
    if size == 0 || end > ncch_end {
        return Err(EmulatorError::InvalidNcchHeader);
    }
    Ok(RomRegion { offset, size })
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or(EmulatorError::InvalidNcchHeader)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}
