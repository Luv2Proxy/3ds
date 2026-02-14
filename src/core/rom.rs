use super::error::{EmulatorError, Result};

const NCSD_MAGIC: &[u8; 4] = b"NCSD";
const NCCH_MAGIC: &[u8; 4] = b"NCCH";
const NCSD_MAGIC_OFFSET: usize = 0x100;
const NCSD_HEADER_SIZE: usize = 0x200;
const MEDIA_UNIT_SIZE: usize = 0x200;
const NCCH_HEADER_SIZE: usize = 0x200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomRegion {
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExHeader {
    pub text_address: u32,
    pub text_size: u32,
    pub text_pages: u32,
    pub ro_address: u32,
    pub ro_size: u32,
    pub ro_pages: u32,
    pub data_address: u32,
    pub data_size: u32,
    pub data_pages: u32,
    pub bss_size: u32,
    pub stack_size: u32,
    pub heap_size: u32,
    pub service_access: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcchProgram {
    pub entrypoint: u32,
    pub exheader: ExHeader,
    pub text_region: RomRegion,
    pub ro_region: RomRegion,
    pub data_region: RomRegion,
}

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
                .ok_or(EmulatorError::InvalidRomLayout)?;
            if end > raw.len() {
                return Err(EmulatorError::InvalidRomLayout);
            }
            let program = parse_ncch_program(raw, offset, size)?;
            partitions.push(NcsdPartition {
                index: idx,
                region: RomRegion { offset, size },
                program,
            });
        }

        if partitions.is_empty() {
            return Err(EmulatorError::InvalidRomLayout);
        }

        Ok(Self {
            bytes: raw.to_vec(),
            partitions,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }


    pub fn first_program(&self) -> Option<&NcchProgram> {
        self.partitions.first().map(|partition| &partition.program)
    }
}

fn parse_ncch_program(raw: &[u8], ncch_offset: usize, ncch_size: usize) -> Result<NcchProgram> {
    if ncch_size < NCCH_HEADER_SIZE {
        return Err(EmulatorError::InvalidRomLayout);
    }
    let header = raw
        .get(ncch_offset..ncch_offset + NCCH_HEADER_SIZE)
        .ok_or(EmulatorError::InvalidRomLayout)?;

    if &header[0x100..0x104] != NCCH_MAGIC {
        return Err(EmulatorError::InvalidRomLayout);
    }

    let exheader_size = read_u32(header, 0x180)? as usize;
    if exheader_size == 0 {
        return Err(EmulatorError::InvalidRomLayout);
    }
    let exheader_offset = ncch_offset + NCCH_HEADER_SIZE;
    let exheader_end = exheader_offset
        .checked_add(exheader_size)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let ncch_end = ncch_offset + ncch_size;
    if exheader_end > ncch_end || exheader_end > raw.len() {
        return Err(EmulatorError::InvalidRomLayout);
    }

    let exheader_bytes = &raw[exheader_offset..exheader_end];
    let exheader = parse_exheader(exheader_bytes)?;

    let entrypoint = read_u32(exheader_bytes, 0x00)?;
    let text_region = parse_section_region(header, ncch_offset, ncch_end, 0x190)?;
    let ro_region = parse_section_region(header, ncch_offset, ncch_end, 0x198)?;
    let data_region = parse_section_region(header, ncch_offset, ncch_end, 0x1A0)?;

    Ok(NcchProgram {
        entrypoint,
        exheader,
        text_region,
        ro_region,
        data_region,
    })
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
        .ok_or(EmulatorError::InvalidRomLayout)?;
    if size == 0 || end > ncch_end {
        return Err(EmulatorError::InvalidRomLayout);
    }
    Ok(RomRegion { offset, size })
}

fn parse_exheader(exheader: &[u8]) -> Result<ExHeader> {
    if exheader.len() < 0x200 {
        return Err(EmulatorError::InvalidExHeader);
    }

    let mut service_access = Vec::new();
    for idx in 0..32 {
        let start = 0x100 + idx * 8;
        let Some(bytes) = exheader.get(start..start + 8) else {
            break;
        };
        if bytes.iter().all(|b| *b == 0) {
            continue;
        }
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let name = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
        if !name.is_empty() {
            service_access.push(name);
        }
    }

    Ok(ExHeader {
        text_address: read_u32(exheader, 0x10)?,
        text_pages: read_u32(exheader, 0x14)?,
        text_size: read_u32(exheader, 0x18)?,
        stack_size: read_u32(exheader, 0x1C)?,
        ro_address: read_u32(exheader, 0x20)?,
        ro_pages: read_u32(exheader, 0x24)?,
        ro_size: read_u32(exheader, 0x28)?,
        data_address: read_u32(exheader, 0x30)?,
        data_pages: read_u32(exheader, 0x34)?,
        data_size: read_u32(exheader, 0x38)?,
        bss_size: read_u32(exheader, 0x3C)?,
        heap_size: read_u32(exheader, 0x40)?,
        service_access,
    })
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let field = bytes
        .get(offset..end)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x4000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");

        rom[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        rom[0x124..0x128].copy_from_slice(&0x18u32.to_le_bytes());

        let ncch_base = 0x200;
        rom[ncch_base + 0x100..ncch_base + 0x104].copy_from_slice(b"NCCH");
        rom[ncch_base + 0x180..ncch_base + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
        rom[ncch_base + 0x190..ncch_base + 0x194].copy_from_slice(&3u32.to_le_bytes());
        rom[ncch_base + 0x194..ncch_base + 0x198].copy_from_slice(&1u32.to_le_bytes());
        rom[ncch_base + 0x198..ncch_base + 0x19C].copy_from_slice(&4u32.to_le_bytes());
        rom[ncch_base + 0x19C..ncch_base + 0x1A0].copy_from_slice(&1u32.to_le_bytes());
        rom[ncch_base + 0x1A0..ncch_base + 0x1A4].copy_from_slice(&5u32.to_le_bytes());
        rom[ncch_base + 0x1A4..ncch_base + 0x1A8].copy_from_slice(&1u32.to_le_bytes());

        let exheader = ncch_base + 0x200;
        rom[exheader..exheader + 4].copy_from_slice(&0x0010_1000u32.to_le_bytes());
        rom[exheader + 0x10..exheader + 0x14].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        rom[exheader + 0x14..exheader + 0x18].copy_from_slice(&1u32.to_le_bytes());
        rom[exheader + 0x18..exheader + 0x1C].copy_from_slice(&0x200u32.to_le_bytes());
        rom[exheader + 0x1C..exheader + 0x20].copy_from_slice(&0x8000u32.to_le_bytes());
        rom[exheader + 0x20..exheader + 0x24].copy_from_slice(&0x0010_2000u32.to_le_bytes());
        rom[exheader + 0x24..exheader + 0x28].copy_from_slice(&1u32.to_le_bytes());
        rom[exheader + 0x28..exheader + 0x2C].copy_from_slice(&0x100u32.to_le_bytes());
        rom[exheader + 0x30..exheader + 0x34].copy_from_slice(&0x0010_3000u32.to_le_bytes());
        rom[exheader + 0x34..exheader + 0x38].copy_from_slice(&1u32.to_le_bytes());
        rom[exheader + 0x38..exheader + 0x3C].copy_from_slice(&0x180u32.to_le_bytes());
        rom[exheader + 0x3C..exheader + 0x40].copy_from_slice(&0x200u32.to_le_bytes());
        rom[exheader + 0x40..exheader + 0x44].copy_from_slice(&0x20000u32.to_le_bytes());
        rom[exheader + 0x100..exheader + 0x107].copy_from_slice(b"fs:USER");

        rom
    }

    #[test]
    fn parse_valid_rom_with_ncch_metadata() {
        let rom = build_test_rom();
        let parsed = RomImage::parse(&rom, usize::MAX)
            .unwrap_or_else(|e| panic!("valid ROM should parse: {e}"));

        assert_eq!(parsed.partitions.len(), 1);
        let program = parsed
            .first_program()
            .unwrap_or_else(|| panic!("program should exist"));
        assert_eq!(program.entrypoint, 0x0010_1000);
        assert_eq!(program.exheader.text_address, 0x0010_0000);
        assert_eq!(program.exheader.service_access, vec!["fs:USER".to_string()]);
        assert_eq!(program.text_region.offset, 0x800);
        assert_eq!(program.ro_region.offset, 0xA00);
        assert_eq!(program.data_region.offset, 0xC00);
    }
}
