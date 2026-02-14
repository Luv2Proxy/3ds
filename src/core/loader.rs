use super::error::{EmulatorError, Result};
use super::fs::{ExeFs, ExeFsFile, RomFs, VirtualFileSystem};
use super::memory::Memory;
use super::rom::{ExHeader, NcchProgram, NcsdPartition, RomImage, RomRegion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedSegment {
    pub virtual_address: u32,
    pub size: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootImage {
    pub entrypoint: u32,
    pub code_segment: MappedSegment,
    pub ro_segment: MappedSegment,
    pub data_segment: MappedSegment,
    pub bss_size: u32,
    pub stack_size_hint: u32,
    pub heap_size_hint: u32,
    pub required_service_access: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcchMetadata {
    pub entrypoint: u32,
    pub exheader: ExHeader,
    pub exefs_region: RomRegion,
    pub romfs_region: Option<RomRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExeFsMetadata {
    pub entries: Vec<ExeFsFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFsMetadata {
    pub file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleImageLayout {
    pub partitions: Vec<NcsdPartition>,
    pub ncch: NcchMetadata,
    pub exefs: ExeFsMetadata,
    pub romfs: Option<RomFsMetadata>,
}

#[derive(Debug, Clone)]
pub struct LoadedProcessImage {
    pub layout: TitleImageLayout,
    pub boot: BootImage,
    pub vfs: VirtualFileSystem,
}

pub fn parse_process_image_from_rom(rom: &[u8]) -> Result<LoadedProcessImage> {
    let image = RomImage::parse(rom, usize::MAX)?;
    parse_process_image(&image)
}

pub fn parse_process_image(image: &RomImage) -> Result<LoadedProcessImage> {
    let partition = image
        .partitions()
        .first()
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let program = &partition.program;

    let exefs_region = image
        .bytes()
        .get(program.exefs_region.offset..program.exefs_region.offset + program.exefs_region.size)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let exefs = ExeFs::parse(exefs_region)?;

    let code = exefs.file(".code").ok_or(EmulatorError::InvalidRomLayout)?;

    let code_segment = read_segment(
        code,
        0,
        program.exheader.text_size as usize,
        program.exheader.text_address,
    )?;
    let ro_segment = read_segment(
        code,
        program.exheader.text_size as usize,
        program.exheader.ro_size as usize,
        program.exheader.ro_address,
    )?;
    let data_segment = read_segment(
        code,
        (program.exheader.text_size + program.exheader.ro_size) as usize,
        program.exheader.data_size as usize,
        program.exheader.data_address,
    )?;

    let mut vfs = VirtualFileSystem::default();
    let mut romfs_meta = None;
    if let Some(romfs_region) = program.romfs_region {
        let romfs_bytes = image
            .bytes()
            .get(romfs_region.offset..romfs_region.offset + romfs_region.size)
            .ok_or(EmulatorError::InvalidRomLayout)?;
        let romfs = RomFs::parse(romfs_bytes)?;
        romfs_meta = Some(RomFsMetadata {
            file_count: romfs.file_count(),
        });
        vfs.mount_romfs(romfs);
    }

    Ok(LoadedProcessImage {
        layout: TitleImageLayout {
            partitions: image.partitions().to_vec(),
            ncch: NcchMetadata {
                entrypoint: program.entrypoint,
                exheader: program.exheader.clone(),
                exefs_region: program.exefs_region,
                romfs_region: program.romfs_region,
            },
            exefs: ExeFsMetadata {
                entries: exefs.entries().to_vec(),
            },
            romfs: romfs_meta,
        },
        boot: BootImage {
            entrypoint: select_entrypoint(program, &code_segment),
            code_segment,
            ro_segment,
            data_segment,
            bss_size: program.exheader.bss_size,
            stack_size_hint: program.exheader.stack_size,
            heap_size_hint: program.exheader.heap_size,
            required_service_access: program.exheader.service_access.clone(),
        },
        vfs,
    })
}

pub fn install_process_image(memory: &mut Memory, boot: &BootImage) {
    map_segment(memory, &boot.code_segment);
    map_segment(memory, &boot.ro_segment);
    map_segment(memory, &boot.data_segment);

    if boot.bss_size > 0 {
        let bss_start = boot
            .data_segment
            .virtual_address
            .wrapping_add(boot.data_segment.size);
        for offset in 0..boot.bss_size {
            memory.write_u8(bss_start.wrapping_add(offset), 0);
        }
    }
}

fn map_segment(memory: &mut Memory, segment: &MappedSegment) {
    for (index, byte) in segment.bytes.iter().enumerate() {
        memory.write_u8(segment.virtual_address.wrapping_add(index as u32), *byte);
    }
    if segment.bytes.len() >= 4 {
        let first_word = u32::from_le_bytes([
            segment.bytes[0],
            segment.bytes[1],
            segment.bytes[2],
            segment.bytes[3],
        ]);
        memory.write_u32(segment.virtual_address, first_word);
    }
}

fn read_segment(
    image: &[u8],
    file_offset: usize,
    size: usize,
    target_address: u32,
) -> Result<MappedSegment> {
    if size == 0 {
        return Ok(MappedSegment {
            virtual_address: target_address,
            size: 0,
            bytes: Vec::new(),
        });
    }
    let end = file_offset
        .checked_add(size)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let section = image
        .get(file_offset..end)
        .ok_or(EmulatorError::InvalidRomLayout)?;

    Ok(MappedSegment {
        virtual_address: target_address,
        size: size as u32,
        bytes: section.to_vec(),
    })
}

fn select_entrypoint(program: &NcchProgram, code_segment: &MappedSegment) -> u32 {
    if program.entrypoint >= code_segment.virtual_address
        && program.entrypoint < code_segment.virtual_address.wrapping_add(code_segment.size)
    {
        program.entrypoint
    } else {
        code_segment.virtual_address
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::Memory;

    fn make_valid_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x6000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        rom[0x124..0x128].copy_from_slice(&0x2Fu32.to_le_bytes());

        let ncch = 0x200;
        rom[ncch + 0x100..ncch + 0x104].copy_from_slice(b"NCCH");
        rom[ncch + 0x180..ncch + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
        rom[ncch + 0x1A8..ncch + 0x1AC].copy_from_slice(&3u32.to_le_bytes());
        rom[ncch + 0x1AC..ncch + 0x1B0].copy_from_slice(&2u32.to_le_bytes());
        rom[ncch + 0x1B0..ncch + 0x1B4].copy_from_slice(&6u32.to_le_bytes());
        rom[ncch + 0x1B4..ncch + 0x1B8].copy_from_slice(&2u32.to_le_bytes());

        let ex = ncch + 0x200;
        rom[ex..ex + 4].copy_from_slice(&0x0010_0004u32.to_le_bytes());
        rom[ex + 0x10..ex + 0x14].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        rom[ex + 0x18..ex + 0x1C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x20..ex + 0x24].copy_from_slice(&0x0010_1000u32.to_le_bytes());
        rom[ex + 0x28..ex + 0x2C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x30..ex + 0x34].copy_from_slice(&0x0010_2000u32.to_le_bytes());
        rom[ex + 0x38..ex + 0x3C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x3C..ex + 0x40].copy_from_slice(&0x10u32.to_le_bytes());
        rom[ex + 0x1C..ex + 0x20].copy_from_slice(&0x2000u32.to_le_bytes());
        rom[ex + 0x40..ex + 0x44].copy_from_slice(&0x8000u32.to_le_bytes());
        rom[ex + 0x100..ex + 0x105].copy_from_slice(b"ndm:u");

        let exefs = 0x800;
        rom[exefs..exefs + 5].copy_from_slice(b".code");
        rom[exefs + 8..exefs + 12].copy_from_slice(&0u32.to_le_bytes());
        rom[exefs + 12..exefs + 16].copy_from_slice(&0x60u32.to_le_bytes());
        rom[exefs + 0x200..exefs + 0x220].copy_from_slice(&[0x11; 0x20]);
        rom[exefs + 0x220..exefs + 0x240].copy_from_slice(&[0x22; 0x20]);
        rom[exefs + 0x240..exefs + 0x260].copy_from_slice(&[0x33; 0x20]);

        let romfs = 0xE00;
        let mut romfs_blob = Vec::new();
        romfs_blob.extend_from_slice(b"ROMF");
        romfs_blob.extend_from_slice(&1u32.to_le_bytes());
        let path = b"/boot.bin";
        romfs_blob.extend_from_slice(&(path.len() as u16).to_le_bytes());
        romfs_blob.extend_from_slice(&48u32.to_le_bytes());
        romfs_blob.extend_from_slice(&4u32.to_le_bytes());
        romfs_blob.extend_from_slice(path);
        romfs_blob.resize(48, 0);
        romfs_blob.extend_from_slice(&[9, 8, 7, 6]);
        rom[romfs..romfs + romfs_blob.len()].copy_from_slice(&romfs_blob);

        rom
    }

    #[test]
    fn parses_title_metadata_and_boot_image() {
        let rom = make_valid_rom();
        let loaded = parse_process_image_from_rom(&rom)
            .unwrap_or_else(|e| panic!("fixture should parse: {e}"));

        assert_eq!(loaded.layout.partitions.len(), 1);
        assert_eq!(loaded.layout.ncch.entrypoint, 0x0010_0004);
        assert_eq!(loaded.layout.exefs.entries.len(), 1);
        assert_eq!(loaded.layout.romfs.as_ref().map(|r| r.file_count), Some(1));

        assert_eq!(loaded.boot.entrypoint, 0x0010_0004);
        assert_eq!(loaded.boot.code_segment.virtual_address, 0x0010_0000);
        assert_eq!(loaded.boot.code_segment.size, 0x20);
        assert_eq!(loaded.boot.ro_segment.virtual_address, 0x0010_1000);
        assert_eq!(loaded.boot.ro_segment.size, 0x20);
        assert_eq!(loaded.boot.data_segment.virtual_address, 0x0010_2000);
        assert_eq!(loaded.boot.data_segment.size, 0x20);
    }

    #[test]
    fn installs_segments_and_zeroes_bss() {
        let rom = make_valid_rom();
        let loaded = parse_process_image_from_rom(&rom)
            .unwrap_or_else(|e| panic!("fixture should parse: {e}"));
        let mut memory = Memory::new();

        install_process_image(&mut memory, &loaded.boot);

        assert_eq!(memory.read_u8(0x0010_0000), 0x11);
        assert_eq!(memory.read_u8(0x0010_1000), 0x22);
        assert_eq!(memory.read_u8(0x0010_2000), 0x33);
        assert_eq!(memory.read_u8(0x0010_2020), 0x00);
    }

    #[test]
    fn rejects_malformed_layouts() {
        let mut bad_magic = make_valid_rom();
        bad_magic[0x200 + 0x100..0x200 + 0x104].copy_from_slice(b"BAD!");

        let mut bad_exefs = make_valid_rom();
        bad_exefs[0x800 + 12..0x800 + 16].copy_from_slice(&0xFFFFu32.to_le_bytes());

        assert!(parse_process_image_from_rom(&bad_magic).is_err());
        assert!(parse_process_image_from_rom(&bad_exefs).is_err());
    }

    #[test]
    fn falls_back_to_text_base_when_entrypoint_is_outside_code() {
        let mut rom = make_valid_rom();
        let exheader = 0x400;
        rom[exheader..exheader + 4].copy_from_slice(&0x0040_0000u32.to_le_bytes());

        let loaded = parse_process_image_from_rom(&rom)
            .unwrap_or_else(|e| panic!("fixture should parse with fallback: {e}"));
        assert_eq!(loaded.boot.entrypoint, 0x0010_0000);
    }
}
