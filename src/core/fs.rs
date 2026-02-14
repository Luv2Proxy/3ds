use std::collections::HashMap;

use super::error::{EmulatorError, Result};

const EXEFS_HEADER_SIZE: usize = 0x200;
const EXEFS_ENTRY_SIZE: usize = 0x10;
const EXEFS_MAX_ENTRIES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRecord {
    pub id: u32,
    pub offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct TitlePackage {
    bytes: Vec<u8>,
    contents: Vec<ContentRecord>,
}

impl TitlePackage {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 8 || &raw[0..4] != b"3DST" {
            return Err(EmulatorError::InvalidTitlePackage);
        }
        let count = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let mut contents = Vec::with_capacity(count);
        let mut cursor = 8usize;
        for _ in 0..count {
            if cursor + 12 > raw.len() {
                return Err(EmulatorError::InvalidTitlePackage);
            }
            let id = u32::from_le_bytes([
                raw[cursor],
                raw[cursor + 1],
                raw[cursor + 2],
                raw[cursor + 3],
            ]);
            let offset = u32::from_le_bytes([
                raw[cursor + 4],
                raw[cursor + 5],
                raw[cursor + 6],
                raw[cursor + 7],
            ]);
            let size = u32::from_le_bytes([
                raw[cursor + 8],
                raw[cursor + 9],
                raw[cursor + 10],
                raw[cursor + 11],
            ]);
            let end = offset as usize + size as usize;
            if end > raw.len() {
                return Err(EmulatorError::InvalidTitlePackage);
            }
            contents.push(ContentRecord { id, offset, size });
            cursor += 12;
        }

        Ok(Self {
            bytes: raw.to_vec(),
            contents,
        })
    }

    pub fn contents(&self) -> &[ContentRecord] {
        &self.contents
    }

    pub fn content_bytes(&self, id: u32) -> Option<&[u8]> {
        let c = self.contents.iter().find(|c| c.id == id)?;
        let start = c.offset as usize;
        let end = start + c.size as usize;
        self.bytes.get(start..end)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExeFsFile {
    pub name: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct ExeFs {
    bytes: Vec<u8>,
    entries: Vec<ExeFsFile>,
}

impl ExeFs {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < EXEFS_HEADER_SIZE {
            return Err(EmulatorError::InvalidRomLayout);
        }

        let mut entries = Vec::new();
        for idx in 0..EXEFS_MAX_ENTRIES {
            let base = idx * EXEFS_ENTRY_SIZE;
            let name_bytes = &raw[base..base + 8];
            if name_bytes.iter().all(|b| *b == 0) {
                continue;
            }
            let name_end = name_bytes.iter().position(|b| *b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_string();
            let offset =
                u32::from_le_bytes([raw[base + 8], raw[base + 9], raw[base + 10], raw[base + 11]])
                    as usize;
            let size = u32::from_le_bytes([
                raw[base + 12],
                raw[base + 13],
                raw[base + 14],
                raw[base + 15],
            ]) as usize;
            let data_start = EXEFS_HEADER_SIZE + offset;
            let data_end = data_start
                .checked_add(size)
                .ok_or(EmulatorError::InvalidRomLayout)?;
            if size == 0 || data_end > raw.len() {
                return Err(EmulatorError::InvalidRomLayout);
            }
            entries.push(ExeFsFile {
                name,
                offset: data_start,
                size,
            });
        }

        Ok(Self {
            bytes: raw.to_vec(),
            entries,
        })
    }

    pub fn file(&self, name: &str) -> Option<&[u8]> {
        let entry = self.entries.iter().find(|entry| entry.name == name)?;
        self.bytes.get(entry.offset..entry.offset + entry.size)
    }

    pub fn entries(&self) -> &[ExeFsFile] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFsFile {
    pub path: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct RomFs {
    bytes: Vec<u8>,
    files: HashMap<String, RomFsFile>,
}

impl RomFs {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 8 || &raw[0..4] != b"ROMF" {
            return Err(EmulatorError::InvalidRomLayout);
        }

        let file_count = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let mut cursor = 8usize;
        let mut files = HashMap::new();
        for _ in 0..file_count {
            if cursor + 10 > raw.len() {
                return Err(EmulatorError::InvalidRomLayout);
            }
            let path_len = u16::from_le_bytes([raw[cursor], raw[cursor + 1]]) as usize;
            let offset = u32::from_le_bytes([
                raw[cursor + 2],
                raw[cursor + 3],
                raw[cursor + 4],
                raw[cursor + 5],
            ]) as usize;
            let size = u32::from_le_bytes([
                raw[cursor + 6],
                raw[cursor + 7],
                raw[cursor + 8],
                raw[cursor + 9],
            ]) as usize;
            cursor += 10;
            let path_end = cursor
                .checked_add(path_len)
                .ok_or(EmulatorError::InvalidRomLayout)?;
            let path_bytes = raw
                .get(cursor..path_end)
                .ok_or(EmulatorError::InvalidRomLayout)?;
            cursor = path_end;
            let path = normalize_path(&String::from_utf8_lossy(path_bytes));

            let end = offset
                .checked_add(size)
                .ok_or(EmulatorError::InvalidRomLayout)?;
            if end > raw.len() {
                return Err(EmulatorError::InvalidRomLayout);
            }
            files.insert(path.clone(), RomFsFile { path, offset, size });
        }

        Ok(Self {
            bytes: raw.to_vec(),
            files,
        })
    }

    pub fn lookup(&self, path: &str) -> Option<&RomFsFile> {
        self.files.get(&normalize_path(path))
    }

    pub fn read_file(&self, path: &str, offset: usize, size: usize) -> Option<Vec<u8>> {
        let file = self.lookup(path)?;
        if offset >= file.size {
            return Some(Vec::new());
        }
        let read_len = size.min(file.size - offset);
        let start = file.offset + offset;
        let end = start + read_len;
        self.bytes.get(start..end).map(|bytes| bytes.to_vec())
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
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

fn normalize_path(path: &str) -> String {
    let cleaned = path.replace('\\', "/");
    let mut out = Vec::new();
    for segment in cleaned.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            let _ = out.pop();
            continue;
        }
        out.push(segment);
    }
    format!("/{}", out.join("/"))
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

    #[test]
    fn exefs_section_extraction() {
        let mut exefs = vec![0u8; EXEFS_HEADER_SIZE + 16];
        exefs[0..5].copy_from_slice(b".code");
        exefs[8..12].copy_from_slice(&0u32.to_le_bytes());
        exefs[12..16].copy_from_slice(&16u32.to_le_bytes());
        exefs[EXEFS_HEADER_SIZE..EXEFS_HEADER_SIZE + 16].copy_from_slice(&[0xAA; 16]);

        let parsed = ExeFs::parse(&exefs).expect("valid exefs");
        let code = parsed.file(".code").expect(".code exists");
        assert_eq!(code, &[0xAA; 16]);
    }
}
