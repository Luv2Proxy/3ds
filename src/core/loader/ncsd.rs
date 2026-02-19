use crate::core::error::{EmulatorError, Result};

use super::ncch::{NcchProgram, RomRegion, parse_ncch_program};

const NCSD_MAGIC: &[u8; 4] = b"NCSD";
const NCSD_MAGIC_OFFSET: usize = 0x100;
const NCSD_HEADER_SIZE: usize = 0x200;
const MEDIA_UNIT_SIZE: usize = 0x200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcsdPartition {
    pub index: usize,
    pub region: RomRegion,
    pub program: NcchProgram,
}

#[derive(Debug, Clone)]
pub struct RomImage {
    bytes: Vec<u8>,
    partitions: Vec<NcsdPartition>,
}

impl RomImage {
    pub fn parse(raw: &[u8], max_size: usize) -> Result<Self> {
        if raw.len() < NCSD_HEADER_SIZE {
            return Err(EmulatorError::RomTooSmall);
        }
        if raw.len() > max_size {
            return Err(EmulatorError::RomTooLarge {
                size: raw.len(),
                capacity: max_size,
            });
        }
        if &raw[NCSD_MAGIC_OFFSET..NCSD_MAGIC_OFFSET + 4] != NCSD_MAGIC {
            return Err(EmulatorError::InvalidRomMagic);
        }

        let mut partitions = Vec::new();
        for idx in 0..8 {
            let entry = 0x120 + idx * 8;
            let offset_units = read_u32(raw, entry)? as usize;
            let size_units = read_u32(raw, entry + 4)? as usize;
            if offset_units == 0 || size_units == 0 {
                continue;
            }

            let offset = offset_units * MEDIA_UNIT_SIZE;
            let size = size_units * MEDIA_UNIT_SIZE;
            let end = offset
                .checked_add(size)
                .ok_or(EmulatorError::InvalidNcsdHeader)?;
            if end > raw.len() {
                return Err(EmulatorError::InvalidNcsdHeader);
            }

            let program = parse_ncch_program(raw, offset, size)?;
            partitions.push(NcsdPartition {
                index: idx,
                region: RomRegion { offset, size },
                program,
            });
        }

        if partitions.is_empty() {
            return Err(EmulatorError::InvalidNcsdHeader);
        }

        Ok(Self {
            bytes: raw.to_vec(),
            partitions,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn partitions(&self) -> &[NcsdPartition] {
        &self.partitions
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or(EmulatorError::InvalidNcsdHeader)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}
