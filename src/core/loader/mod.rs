mod exefs;
mod exheader;
mod ncch;
mod ncsd;
mod process;
mod romfs;

pub use exefs::{ExeFs, ExeFsFile};
pub use exheader::ExHeader;
pub use ncch::{NcchProgram, RomRegion};
pub use ncsd::{NcsdPartition, RomImage};
pub use process::{
    LoadedProcessImage, ProcessImage, ProcessSegment, TitleImageLayout, install_process_image,
    parse_process_image, parse_process_image_from_rom,
};
pub use romfs::{RomFs, RomFsFile, normalize_path};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::Memory;

    fn valid_rom_fixture() -> Vec<u8> {
        let mut rom = vec![0u8; 0x6000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        rom[0x124..0x128].copy_from_slice(&0x2Fu32.to_le_bytes());

        let ncch = 0x200;
        rom[ncch + 0x100..ncch + 0x104].copy_from_slice(b"NCCH");
        rom[ncch + 0x180..ncch + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
        rom[ncch + 0x1A8..ncch + 0x1AC].copy_from_slice(&3u32.to_le_bytes());
        rom[ncch + 0x1AC..ncch + 0x1B0].copy_from_slice(&2u32.to_le_bytes());

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

        rom
    }

    #[test]
    fn parses_valid_decrypted_ncch_and_maps_sections() {
        let loaded = parse_process_image_from_rom(&valid_rom_fixture()).expect("valid fixture");
        assert_eq!(loaded.process.entrypoint, 0x0010_0004);
        assert_eq!(loaded.process.text.virtual_address, 0x0010_0000);
        assert_eq!(loaded.process.ro.virtual_address, 0x0010_1000);
        assert_eq!(loaded.process.data.virtual_address, 0x0010_2000);
        assert_eq!(loaded.process.service_access, vec!["ndm:u".to_string()]);

        let mut mem = Memory::new();
        install_process_image(&mut mem, &loaded.process).expect("map works");
        assert_eq!(mem.read_u8(0x0010_0000), 0x11);
        assert_eq!(mem.read_u8(0x0010_1000), 0x22);
        assert_eq!(mem.read_u8(0x0010_2000), 0x33);
        assert_eq!(mem.read_u8(0x0010_2020), 0x00);
    }

    #[test]
    fn rejects_invalid_ncch_and_bounds() {
        let mut bad_magic = valid_rom_fixture();
        bad_magic[0x200 + 0x100..0x200 + 0x104].copy_from_slice(b"BAD!");
        assert!(matches!(
            parse_process_image_from_rom(&bad_magic),
            Err(crate::core::error::EmulatorError::InvalidNcchHeader)
        ));

        let mut bad_entry = valid_rom_fixture();
        bad_entry[0x400..0x404].copy_from_slice(&0x0040_0000u32.to_le_bytes());
        assert!(matches!(
            parse_process_image_from_rom(&bad_entry),
            Err(crate::core::error::EmulatorError::EntrypointOutsideText)
        ));

        let mut bad_sections = valid_rom_fixture();
        bad_sections[0x400 + 0x28..0x400 + 0x2C].copy_from_slice(&0x9000u32.to_le_bytes());
        assert!(matches!(
            parse_process_image_from_rom(&bad_sections),
            Err(crate::core::error::EmulatorError::InvalidSectionLayout)
        ));
    }
}
