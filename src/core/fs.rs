use super::error::{EmulatorError, Result};

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
