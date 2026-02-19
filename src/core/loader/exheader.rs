use crate::core::error::{EmulatorError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExHeader {
    pub text_address: u32,
    pub text_size: u32,
    pub ro_address: u32,
    pub ro_size: u32,
    pub data_address: u32,
    pub data_size: u32,
    pub bss_size: u32,
    pub stack_size: u32,
    pub heap_size: u32,
    pub service_access: Vec<String>,
}

impl ExHeader {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        if raw.len() < 0x200 {
            return Err(EmulatorError::InvalidExHeader);
        }

        let mut service_access = Vec::new();
        for idx in 0..32 {
            let start = 0x100 + idx * 8;
            let Some(bytes) = raw.get(start..start + 8) else {
                break;
            };
            if bytes.iter().all(|b| *b == 0) {
                continue;
            }
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            let svc = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
            if !svc.is_empty() {
                service_access.push(svc);
            }
        }

        Ok(Self {
            text_address: read_u32(raw, 0x10)?,
            text_size: read_u32(raw, 0x18)?,
            stack_size: read_u32(raw, 0x1C)?,
            ro_address: read_u32(raw, 0x20)?,
            ro_size: read_u32(raw, 0x28)?,
            data_address: read_u32(raw, 0x30)?,
            data_size: read_u32(raw, 0x38)?,
            bss_size: read_u32(raw, 0x3C)?,
            heap_size: read_u32(raw, 0x40)?,
            service_access,
        })
    }

    pub fn validate_layout(&self, code_file_size: usize) -> Result<()> {
        let text_end = self
            .text_size
            .checked_add(self.ro_size)
            .and_then(|v| v.checked_add(self.data_size))
            .ok_or(EmulatorError::InvalidSectionLayout)? as usize;
        if text_end > code_file_size {
            return Err(EmulatorError::InvalidSectionLayout);
        }

        let text_end_addr = self
            .text_address
            .checked_add(self.text_size)
            .ok_or(EmulatorError::InvalidSectionLayout)?;
        if text_end_addr > self.ro_address {
            return Err(EmulatorError::InvalidSectionLayout);
        }
        let ro_end_addr = self
            .ro_address
            .checked_add(self.ro_size)
            .ok_or(EmulatorError::InvalidSectionLayout)?;
        if ro_end_addr > self.data_address {
            return Err(EmulatorError::InvalidSectionLayout);
        }
        Ok(())
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or(EmulatorError::InvalidExHeader)?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}
