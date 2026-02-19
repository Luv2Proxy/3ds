use std::collections::HashMap;

use crate::core::error::{EmulatorError, Result};

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
            return Err(EmulatorError::InvalidRomFs);
        }

        let file_count = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let mut cursor = 8usize;
        let mut files = HashMap::new();
        for _ in 0..file_count {
            if cursor + 10 > raw.len() {
                return Err(EmulatorError::InvalidRomFs);
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
                .ok_or(EmulatorError::InvalidRomFs)?;
            let path_bytes = raw
                .get(cursor..path_end)
                .ok_or(EmulatorError::InvalidRomFs)?;
            cursor = path_end;

            let path = normalize_path(&String::from_utf8_lossy(path_bytes));
            let end = offset
                .checked_add(size)
                .ok_or(EmulatorError::InvalidRomFs)?;
            if end > raw.len() {
                return Err(EmulatorError::InvalidRomFs);
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

pub fn normalize_path(path: &str) -> String {
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
