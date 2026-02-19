use crate::core::error::{EmulatorError, Result};
use crate::core::fs::VirtualFileSystem;
use crate::core::memory::Memory;

use super::exefs::ExeFs;
use super::ncsd::{NcsdPartition, RomImage};
use super::romfs::RomFs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSegment {
    pub virtual_address: u32,
    pub size: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessImage {
    pub entrypoint: u32,
    pub text: ProcessSegment,
    pub ro: ProcessSegment,
    pub data: ProcessSegment,
    pub bss_size: u32,
    pub stack_size: u32,
    pub heap_size: u32,
    pub service_access: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleImageLayout {
    pub partitions: Vec<NcsdPartition>,
}

#[derive(Debug, Clone)]
pub struct LoadedProcessImage {
    pub layout: TitleImageLayout,
    pub process: ProcessImage,
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
        .ok_or(EmulatorError::InvalidNcsdHeader)?;
    let program = &partition.program;

    let exefs_region = image
        .bytes()
        .get(program.exefs_region.offset..program.exefs_region.offset + program.exefs_region.size)
        .ok_or(EmulatorError::InvalidExeFs)?;
    let exefs = ExeFs::parse(exefs_region)?;
    let code = exefs
        .file(".code")
        .ok_or(EmulatorError::MissingCodeSection)?;

    program.exheader.validate_layout(code.len())?;

    let text = read_segment(
        code,
        0,
        program.exheader.text_size as usize,
        program.exheader.text_address,
    )?;
    let ro = read_segment(
        code,
        program.exheader.text_size as usize,
        program.exheader.ro_size as usize,
        program.exheader.ro_address,
    )?;
    let data = read_segment(
        code,
        (program.exheader.text_size + program.exheader.ro_size) as usize,
        program.exheader.data_size as usize,
        program.exheader.data_address,
    )?;

    if program.entrypoint < text.virtual_address
        || program.entrypoint >= text.virtual_address.saturating_add(text.size)
    {
        return Err(EmulatorError::EntrypointOutsideText);
    }

    let mut vfs = VirtualFileSystem::default();
    if let Some(romfs_region) = program.romfs_region {
        let romfs_bytes = image
            .bytes()
            .get(romfs_region.offset..romfs_region.offset + romfs_region.size)
            .ok_or(EmulatorError::InvalidRomFs)?;
        let romfs = RomFs::parse(romfs_bytes)?;
        vfs.mount_romfs(romfs);
    }

    Ok(LoadedProcessImage {
        layout: TitleImageLayout {
            partitions: image.partitions().to_vec(),
        },
        process: ProcessImage {
            entrypoint: program.entrypoint,
            text,
            ro,
            data,
            bss_size: program.exheader.bss_size,
            stack_size: program.exheader.stack_size,
            heap_size: program.exheader.heap_size,
            service_access: program.exheader.service_access.clone(),
        },
        vfs,
    })
}

pub fn install_process_image(memory: &mut Memory, image: &ProcessImage) -> Result<()> {
    map_segment(memory, &image.text)?;
    map_segment(memory, &image.ro)?;
    map_segment(memory, &image.data)?;

    if image.bss_size > 0 {
        let bss_start = image.data.virtual_address.wrapping_add(image.data.size);
        for offset in 0..image.bss_size {
            memory.write_u8_checked(bss_start.wrapping_add(offset), 0)?;
        }
    }
    Ok(())
}

fn map_segment(memory: &mut Memory, segment: &ProcessSegment) -> Result<()> {
    for (idx, b) in segment.bytes.iter().enumerate() {
        memory.write_u8_checked(segment.virtual_address.wrapping_add(idx as u32), *b)?;
    }
    Ok(())
}

fn read_segment(
    image: &[u8],
    file_offset: usize,
    size: usize,
    target_address: u32,
) -> Result<ProcessSegment> {
    let end = file_offset
        .checked_add(size)
        .ok_or(EmulatorError::InvalidSectionLayout)?;
    let section = image
        .get(file_offset..end)
        .ok_or(EmulatorError::InvalidSectionLayout)?;

    Ok(ProcessSegment {
        virtual_address: target_address,
        size: size as u32,
        bytes: section.to_vec(),
    })
}
