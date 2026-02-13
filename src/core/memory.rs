use super::error::{EmulatorError, Result};

pub const FCRAM_SIZE: usize = 128 * 1024 * 1024;

#[derive(Clone)]
pub struct Memory {
    fcram: Vec<u8>,
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    pub fn new() -> Self {
        Self {
            fcram: vec![0; FCRAM_SIZE],
        }
    }

    pub fn clear(&mut self) {
        self.fcram.fill(0);
    }

    pub fn len(&self) -> usize {
        self.fcram.len()
    }

    pub fn read_u32(&self, addr: u32) -> Result<u32> {
        let base = addr as usize;
        if base + 4 > self.fcram.len() {
            return Err(EmulatorError::MemoryOutOfBounds { address: addr });
        }
        let bytes = &self.fcram[base..base + 4];
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn write_u32(&mut self, addr: u32, value: u32) -> Result<()> {
        let base = addr as usize;
        if base + 4 > self.fcram.len() {
            return Err(EmulatorError::MemoryOutOfBounds { address: addr });
        }
        self.fcram[base..base + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn load(&mut self, base: usize, data: &[u8]) -> Result<()> {
        let end = base + data.len();
        if end > self.fcram.len() {
            return Err(EmulatorError::MemoryOutOfBounds {
                address: end as u32,
            });
        }
        self.fcram[base..end].copy_from_slice(data);
        Ok(())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.fcram
    }
}
