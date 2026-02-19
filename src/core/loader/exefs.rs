use crate::core::error::{EmulatorError, Result};

const EXEFS_HEADER_SIZE: usize = 0x200;
const EXEFS_ENTRY_SIZE: usize = 0x10;
const EXEFS_MAX_ENTRIES: usize = 10;

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
            return Err(EmulatorError::InvalidExeFs);
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
                .ok_or(EmulatorError::InvalidExeFs)?;
            if size == 0 || data_end > raw.len() {
                return Err(EmulatorError::InvalidExeFs);
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
