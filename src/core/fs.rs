pub use crate::core::loader::RomFs;
use crate::core::loader::normalize_path;

use super::error::{EmulatorError, Result};

#[derive(Debug, Clone)]
pub struct TitlePackage {
    rom: Vec<u8>,
}

impl TitlePackage {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 0x104 {
            return Err(EmulatorError::InvalidTitlePackage);
        }
        if &raw[0x100..0x104] != b"NCSD" {
            return Err(EmulatorError::TitlePackageFormatDeprecated);
        }
        Ok(Self { rom: raw.to_vec() })
    }

    pub fn primary_rom(&self) -> &[u8] {
        &self.rom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveId {
    Sdmc = 0,
    Save = 1,
    RomFs = 2,
    ExtData = 3,
}

impl ArchiveId {
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::Sdmc),
            1 => Some(Self::Save),
            2 => Some(Self::RomFs),
            3 => Some(Self::ExtData),
            _ => None,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Sdmc => "/sdmc",
            Self::Save => "/save",
            Self::RomFs => "/romfs",
            Self::ExtData => "/extdata",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveHandle {
    pub archive: ArchiveId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHandle {
    archive: ArchiveId,
    path: String,
}

#[derive(Debug, Clone, Default)]
pub struct VirtualFileSystem {
    romfs: Option<RomFs>,
}

impl VirtualFileSystem {
    pub fn mount_romfs(&mut self, romfs: RomFs) {
        self.romfs = Some(romfs);
    }

    pub fn open_archive(&self, raw_id: u32) -> Option<ArchiveHandle> {
        let archive = ArchiveId::from_raw(raw_id)?;
        if archive == ArchiveId::RomFs && self.romfs.is_none() {
            return None;
        }
        Some(ArchiveHandle { archive })
    }

    pub fn translate_path(&self, archive: ArchiveId, path: &str) -> String {
        format!(
            "{}/{}",
            archive.prefix(),
            normalize_path(path).trim_start_matches('/')
        )
    }

    pub fn open_file(&self, archive: ArchiveHandle, path: &str) -> Option<FileHandle> {
        let normalized = normalize_path(path);
        match archive.archive {
            ArchiveId::RomFs => self
                .romfs
                .as_ref()?
                .lookup(&normalized)
                .map(|_| FileHandle {
                    archive: ArchiveId::RomFs,
                    path: normalized,
                }),
            _ => Some(FileHandle {
                archive: archive.archive,
                path: normalized,
            }),
        }
    }

    pub fn read_file(&self, file: &FileHandle, offset: usize, size: usize) -> Option<Vec<u8>> {
        match file.archive {
            ArchiveId::RomFs => self.romfs.as_ref()?.read_file(&file.path, offset, size),
            _ => Some(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn romfs_fixture() -> Vec<u8> {
        let mut romfs = Vec::new();
        romfs.extend_from_slice(b"ROMF");
        romfs.extend_from_slice(&1u32.to_le_bytes());
        let path = b"/data/config.bin";
        let data_offset = 64u32;
        let data = [1u8, 2, 3, 4, 5, 6];
        romfs.extend_from_slice(&(path.len() as u16).to_le_bytes());
        romfs.extend_from_slice(&data_offset.to_le_bytes());
        romfs.extend_from_slice(&(data.len() as u32).to_le_bytes());
        romfs.extend_from_slice(path);
        romfs.resize(data_offset as usize, 0);
        romfs.extend_from_slice(&data);
        romfs
    }

    #[test]
    fn romfs_path_lookup_and_read_boundaries() {
        let romfs = RomFs::parse(&romfs_fixture()).expect("valid romfs");
        let file = romfs.lookup("data/./config.bin").expect("path lookup");
        assert_eq!(file.size, 6);

        let slice = romfs
            .read_file("/data/config.bin", 2, 8)
            .expect("read works");
        assert_eq!(slice, vec![3, 4, 5, 6]);

        let empty = romfs
            .read_file("/data/config.bin", 100, 4)
            .expect("out of range returns empty");
        assert!(empty.is_empty());
    }
}
