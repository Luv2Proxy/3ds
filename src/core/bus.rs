use std::collections::BTreeMap;

use super::error::Result;
use super::memory::Memory;

pub trait BusDevice {
    fn read_u32(&mut self, _addr: u32) -> u32 {
        0
    }

    fn write_u32(&mut self, _addr: u32, _value: u32) {}
}

pub trait Bus {
    fn read_u8_checked(&mut self, addr: u32) -> Result<u8>;
    fn write_u8_checked(&mut self, addr: u32, value: u8) -> Result<()>;

    fn read_u8(&mut self, addr: u32) -> u8 {
        self.read_u8_checked(addr).unwrap_or(0)
    }

    fn write_u8(&mut self, addr: u32, value: u8) {
        let _ = self.write_u8_checked(addr, value);
    }

    fn read_u32_checked(&mut self, addr: u32) -> Result<u32> {
        let b0 = self.read_u8_checked(addr)?;
        let b1 = self.read_u8_checked(addr.wrapping_add(1))?;
        let b2 = self.read_u8_checked(addr.wrapping_add(2))?;
        let b3 = self.read_u8_checked(addr.wrapping_add(3))?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    fn write_u32_checked(&mut self, addr: u32, value: u32) -> Result<()> {
        let bytes = value.to_le_bytes();
        self.write_u8_checked(addr, bytes[0])?;
        self.write_u8_checked(addr.wrapping_add(1), bytes[1])?;
        self.write_u8_checked(addr.wrapping_add(2), bytes[2])?;
        self.write_u8_checked(addr.wrapping_add(3), bytes[3])?;
        Ok(())
    }

    fn read_u32(&mut self, addr: u32) -> u32 {
        self.read_u32_checked(addr).unwrap_or(0)
    }

    fn write_u32(&mut self, addr: u32, value: u32) {
        let _ = self.write_u32_checked(addr, value);
    }
}

impl Bus for Memory {
    fn read_u8_checked(&mut self, addr: u32) -> Result<u8> {
        Memory::read_u8_checked(self, addr)
    }

    fn write_u8_checked(&mut self, addr: u32, value: u8) -> Result<()> {
        Memory::write_u8_checked(self, addr, value)
    }
}

#[derive(Default)]
pub struct SystemBus {
    memory: Memory,
    mmio: BTreeMap<u32, Box<dyn BusDevice>>,
}

impl SystemBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut Memory {
        &mut self.memory
    }

    pub fn map_mmio_device(&mut self, base: u32, device: Box<dyn BusDevice>) {
        self.mmio.insert(base, device);
    }

    fn mmio_base(addr: u32) -> u32 {
        addr & !0xFFF
    }
}

impl Bus for SystemBus {
    fn read_u8_checked(&mut self, addr: u32) -> Result<u8> {
        if let Some(device) = self.mmio.get_mut(&Self::mmio_base(addr)) {
            let lane = (addr & 3) * 8;
            return Ok(((device.read_u32(addr & !3) >> lane) & 0xFF) as u8);
        }
        self.memory.read_u8_checked(addr)
    }

    fn write_u8_checked(&mut self, addr: u32, value: u8) -> Result<()> {
        if let Some(device) = self.mmio.get_mut(&Self::mmio_base(addr)) {
            let aligned = addr & !3;
            let shift = (addr & 3) * 8;
            let mut word = device.read_u32(aligned);
            word &= !(0xFF << shift);
            word |= u32::from(value) << shift;
            device.write_u32(aligned, word);
            return Ok(());
        }
        self.memory.write_u8_checked(addr, value)
    }
}
