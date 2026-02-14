use super::error::{EmulatorError, Result};
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
    load_process_from_image(memory, &image)
}

pub fn load_process_from_image(memory: &mut Memory, image: &RomImage) -> Result<ProcessMetadata> {
    memory.map_rom(image.bytes());
    let program = image
        .first_program()
        .ok_or(EmulatorError::InvalidRomLayout)?;
    map_program(memory, image.bytes(), program)?;

    Ok(ProcessMetadata {
        entrypoint: program.entrypoint,
        stack_size: program.exheader.stack_size,
        heap_size: program.exheader.heap_size,
        bss_size: program.exheader.bss_size,
        services: program.exheader.service_access.clone(),
    })
}

fn map_program(memory: &mut Memory, bytes: &[u8], program: &NcchProgram) -> Result<()> {
    map_segment(
        memory,
        bytes,
        program.text_region.offset,
        program.exheader.text_size as usize,
        program.exheader.text_address,
    )?;
    map_segment(
        memory,
        bytes,
        program.ro_region.offset,
        program.exheader.ro_size as usize,
        program.exheader.ro_address,
    )?;
    map_segment(
        memory,
        bytes,
        program.data_region.offset,
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
        let mut rom = vec![0u8; 0x5000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        rom[0x124..0x128].copy_from_slice(&0x20u32.to_le_bytes());

        let ncch = 0x200;
        rom[ncch + 0x100..ncch + 0x104].copy_from_slice(b"NCCH");
        rom[ncch + 0x180..ncch + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
        rom[ncch + 0x190..ncch + 0x194].copy_from_slice(&3u32.to_le_bytes());
        rom[ncch + 0x194..ncch + 0x198].copy_from_slice(&1u32.to_le_bytes());
        rom[ncch + 0x198..ncch + 0x19C].copy_from_slice(&4u32.to_le_bytes());
        rom[ncch + 0x19C..ncch + 0x1A0].copy_from_slice(&1u32.to_le_bytes());
        rom[ncch + 0x1A0..ncch + 0x1A4].copy_from_slice(&5u32.to_le_bytes());
        rom[ncch + 0x1A4..ncch + 0x1A8].copy_from_slice(&1u32.to_le_bytes());

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

        rom[0x800..0x820].copy_from_slice(&[0x11; 0x20]);
        rom[0xA00..0xA20].copy_from_slice(&[0x22; 0x20]);
        rom[0xC00..0xC20].copy_from_slice(&[0x33; 0x20]);
        rom
    }

    #[test]
    fn maps_sections_and_reports_entrypoint() {
        let mut memory = Memory::new();
        let rom = make_fixture_rom();
        let metadata = load_process_from_rom(&mut memory, &rom)
            .unwrap_or_else(|e| panic!("loader should parse fixture: {e}"));

        assert_eq!(metadata.entrypoint, 0x0010_0000);
        assert_eq!(metadata.stack_size, 0x2000);
        assert_eq!(metadata.heap_size, 0x8000);
        assert_eq!(memory.read_u8(0x0010_0000), 0x11);
        assert_eq!(memory.read_u8(0x0010_1000), 0x22);
        assert_eq!(memory.read_u8(0x0010_2000), 0x33);
        assert_eq!(memory.read_u8(0x0010_2020), 0x00);
    }
}
