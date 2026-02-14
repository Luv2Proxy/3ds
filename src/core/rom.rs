use super::error::{EmulatorError, Result};

const ROM_MAGIC: &[u8; 4] = b"NCSD";
const MAGIC_OFFSET: usize = 0x100;
const HEADER_SIZE: usize = 0x104;

#[derive(Debug, Clone)]
pub struct RomImage {
    bytes: Vec<u8>,
}

impl RomImage {
    pub fn parse(raw: &[u8], max_size: usize) -> Result<Self> {
        if raw.len() < HEADER_SIZE {
            return Err(EmulatorError::RomTooSmall);
        }
        if &raw[MAGIC_OFFSET..MAGIC_OFFSET + 4] != ROM_MAGIC {
            return Err(EmulatorError::InvalidRomMagic);
        }
        if raw.len() > max_size {
            return Err(EmulatorError::RomTooLarge {
                size: raw.len(),
                capacity: max_size,
            });
        }
        Ok(Self {
            bytes: raw.to_vec(),
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_rom() {
        let mut rom = vec![0_u8; 0x200];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        let parsed =
            RomImage::parse(&rom, 0x1000).unwrap_or_else(|e| panic!("valid ROM should parse: {e}"));
        assert_eq!(parsed.bytes().len(), rom.len());
    }
}
