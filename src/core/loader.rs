use super::error::{EmulatorError, Result};
use super::fs::{ExeFs, RomFs, VirtualFileSystem};
use super::memory::Memory;
use super::rom::{NcchProgram, RomImage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessMetadata {
    pub entrypoint: u32,
    pub stack_size: u32,
    pub heap_size: u32,
    pub bss_size: u32,
    pub services: Vec<String>,
}

pub fn load_process_from_rom(memory: &mut Memory, rom: &[u8]) -> Result<ProcessMetadata> {
    let image = RomImage::parse(rom, usize::MAX)?;
    load_process_from_image(memory, &image).map(|(meta, _)| meta)
}

pub fn load_process_with_fs_from_rom(
    memory: &mut Memory,
    rom: &[u8],
) -> Result<(ProcessMetadata, VirtualFileSystem)> {
    let image = RomImage::parse(rom, usize::MAX)?;
    load_process_from_image(memory, &image)
}

pub fn load_process_from_image(
    memory: &mut Memory,
    image: &RomImage,
) -> Result<(ProcessMetadata, VirtualFileSystem)> {
    memory.map_rom(image.bytes());
    let program = image
        .first_program()
        .ok_or(EmulatorError::InvalidRomLayout)?;

    let exefs_region = image
        .bytes()
        .get(program.exefs_region.offset..program.exefs_region.offset + program.exefs_region.size)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let exefs = ExeFs::parse(exefs_region)?;

    map_program(memory, &exefs, program)?;

    let mut vfs = VirtualFileSystem::default();
    if let Some(romfs_region) = program.romfs_region {
        let romfs_bytes = image
            .bytes()
            .get(romfs_region.offset..romfs_region.offset + romfs_region.size)
            .ok_or(EmulatorError::InvalidRomLayout)?;
        let romfs = RomFs::parse(romfs_bytes)?;
        vfs.mount_romfs(romfs);
    }

    Ok((
        ProcessMetadata {
            entrypoint: program.entrypoint,
            stack_size: program.exheader.stack_size,
            heap_size: program.exheader.heap_size,
            bss_size: program.exheader.bss_size,
            services: program.exheader.service_access.clone(),
        },
        vfs,
    ))
}

fn map_program(memory: &mut Memory, exefs: &ExeFs, program: &NcchProgram) -> Result<()> {
    let code = exefs.file(".code").ok_or(EmulatorError::InvalidRomLayout)?;

    map_segment(
        memory,
        code,
        0,
        program.exheader.text_size as usize,
        program.exheader.text_address,
    )?;
    map_segment(
        memory,
        code,
        program.exheader.text_size as usize,
        program.exheader.ro_size as usize,
        program.exheader.ro_address,
    )?;
    map_segment(
        memory,
        code,
        (program.exheader.text_size + program.exheader.ro_size) as usize,
        program.exheader.data_size as usize,
        program.exheader.data_address,
    )?;

    if program.exheader.bss_size > 0 {
        let bss_start = program
            .exheader
            .data_address
            .wrapping_add(program.exheader.data_size);
        for offset in 0..program.exheader.bss_size {
            memory.write_u8(bss_start.wrapping_add(offset), 0);
        }
    }

    Ok(())
}

fn map_segment(
    memory: &mut Memory,
    image: &[u8],
    file_offset: usize,
    size: usize,
    target_address: u32,
) -> Result<()> {
    if size == 0 {
        return Ok(());
    }
    let end = file_offset
        .checked_add(size)
        .ok_or(EmulatorError::InvalidRomLayout)?;
    let section = image
        .get(file_offset..end)
        .ok_or(EmulatorError::InvalidRomLayout)?;

    for (index, byte) in section.iter().enumerate() {
        memory.write_u8(target_address.wrapping_add(index as u32), *byte);
    }

    if size >= 4 {
        let first_word = u32::from_le_bytes([section[0], section[1], section[2], section[3]]);
        memory.write_u32(target_address, first_word);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::Memory;

    fn make_fixture_rom() -> Vec<u8> {
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
        rom[ex..ex + 4].copy_from_slice(&0x0010_0000u32.to_le_bytes());
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
    fn maps_sections_and_reports_entrypoint() {
        let mut memory = Memory::new();
        let rom = make_fixture_rom();
        let (metadata, vfs) = load_process_with_fs_from_rom(&mut memory, &rom)
            .unwrap_or_else(|e| panic!("loader should parse fixture: {e}"));

        assert_eq!(metadata.entrypoint, 0x0010_0000);
        assert_eq!(metadata.stack_size, 0x2000);
        assert_eq!(metadata.heap_size, 0x8000);
        assert_eq!(memory.read_u8(0x0010_0000), 0x11);
        assert_eq!(memory.read_u8(0x0010_1000), 0x22);
        assert_eq!(memory.read_u8(0x0010_2000), 0x33);
        assert_eq!(memory.read_u8(0x0010_2020), 0x00);

        let archive = vfs.open_archive(2).expect("romfs archive mounted");
        let file = vfs
            .open_file(archive, "/boot.bin")
            .expect("boot file exists");
        let bytes = vfs.read_file(&file, 1, 8).expect("read works");
        assert_eq!(bytes, vec![8, 7, 6]);
    }
}
