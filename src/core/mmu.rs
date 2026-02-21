use std::collections::HashMap;

use super::bus::Bus;
use super::error::{EmulatorError, MemoryAccessKind, Result};

const SECTION_DESCRIPTOR_MASK: u32 = 0b11;
const SECTION_DESCRIPTOR_VALUE: u32 = 0b10;
const SECTION_BASE_MASK: u32 = 0xFFF0_0000;
const SECTION_VA_MASK: u32 = 0xFFF0_0000;
const SECTION_OFFSET_MASK: u32 = 0x000F_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TlbEntry {
    pa_section: u32,
    domain: u8,
    ap: u8,
    apx: bool,
    execute_never: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccessFlags {
    ap: u8,
    apx: bool,
    execute_never: bool,
}

#[derive(Clone)]
pub struct Mmu {
    control: u32,
    ttbr0: u32,
    dacr: u32,
    icache_enabled: bool,
    dcache_enabled: bool,
    tlb: HashMap<u32, TlbEntry>,
}

impl Default for Mmu {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmu {
    pub fn new() -> Self {
        Self {
            control: 0,
            ttbr0: 0,
            dacr: 0,
            icache_enabled: false,
            dcache_enabled: false,
            tlb: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        self.control = 0;
        self.ttbr0 = 0;
        self.dacr = 0;
        self.icache_enabled = false;
        self.dcache_enabled = false;
        self.tlb.clear();
    }

    pub fn write_control(&mut self, value: u32) {
        let old_enabled = self.mmu_enabled();
        let old_icache = self.icache_enabled;
        let old_dcache = self.dcache_enabled;
        self.control = value;
        self.icache_enabled = (value & (1 << 12)) != 0;
        self.dcache_enabled = (value & (1 << 2)) != 0;
        if old_enabled != self.mmu_enabled()
            || old_icache != self.icache_enabled
            || old_dcache != self.dcache_enabled
        {
            self.invalidate_tlb();
        }
    }

    pub fn write_ttbr0(&mut self, value: u32) {
        self.ttbr0 = value & 0xFFFF_C000;
        self.invalidate_tlb();
    }

    pub fn write_dacr(&mut self, value: u32) {
        self.dacr = value;
        self.invalidate_tlb();
    }

    pub fn invalidate_tlb(&mut self) {
        self.tlb.clear();
    }

    #[cfg(test)]
    pub fn tlb_len(&self) -> usize {
        self.tlb.len()
    }

    pub fn mmu_enabled(&self) -> bool {
        (self.control & 1) != 0
    }

    pub fn icache_enabled(&self) -> bool {
        self.icache_enabled
    }

    pub fn dcache_enabled(&self) -> bool {
        self.dcache_enabled
    }

    pub fn translate_instruction(
        &mut self,
        memory: &mut dyn Bus,
        va: u32,
        privileged: bool,
    ) -> Result<u32> {
        self.translate(memory, va, MemoryAccessKind::Execute, privileged)
    }

    pub fn translate_read(
        &mut self,
        memory: &mut dyn Bus,
        va: u32,
        privileged: bool,
    ) -> Result<u32> {
        self.translate(memory, va, MemoryAccessKind::Read, privileged)
    }

    pub fn translate_write(
        &mut self,
        memory: &mut dyn Bus,
        va: u32,
        privileged: bool,
    ) -> Result<u32> {
        self.translate(memory, va, MemoryAccessKind::Write, privileged)
    }

    pub fn translate(
        &mut self,
        memory: &mut dyn Bus,
        va: u32,
        access: MemoryAccessKind,
        privileged: bool,
    ) -> Result<u32> {
        if !self.mmu_enabled() {
            return Ok(va);
        }

        let section_key = va & SECTION_VA_MASK;
        let entry = if let Some(entry) = self.tlb.get(&section_key).copied() {
            entry
        } else {
            let table_index = (va >> 20) * 4;
            let desc_addr = self.ttbr0.wrapping_add(table_index);
            let descriptor = memory.read_u32_checked(desc_addr)?;

            if descriptor & SECTION_DESCRIPTOR_MASK != SECTION_DESCRIPTOR_VALUE {
                return Err(EmulatorError::MmuTranslationFault {
                    pc: 0,
                    va,
                    pa: None,
                    access,
                });
            }

            let entry = TlbEntry {
                pa_section: descriptor & SECTION_BASE_MASK,
                domain: ((descriptor >> 5) & 0xF) as u8,
                ap: ((descriptor >> 10) & 0x3) as u8,
                apx: ((descriptor >> 15) & 1) != 0,
                execute_never: ((descriptor >> 4) & 1) != 0,
            };
            self.tlb.insert(section_key, entry);
            entry
        };

        self.check_domain_and_permissions(va, entry, access, privileged)?;
        Ok(entry.pa_section | (va & SECTION_OFFSET_MASK))
    }

    fn check_domain_and_permissions(
        &self,
        va: u32,
        entry: TlbEntry,
        access: MemoryAccessKind,
        privileged: bool,
    ) -> Result<()> {
        let domain_mode = (self.dacr >> (u32::from(entry.domain) * 2)) & 0b11;
        match domain_mode {
            0b00 | 0b10 => {
                return Err(EmulatorError::MmuDomainFault {
                    pc: 0,
                    va,
                    pa: None,
                    domain: entry.domain,
                    access,
                });
            }
            0b11 => return Ok(()),
            0b01 => {}
            _ => unreachable!(),
        }

        if entry.execute_never && matches!(access, MemoryAccessKind::Execute) {
            return Err(EmulatorError::MmuPermissionFault {
                pc: 0,
                va,
                pa: None,
                access,
            });
        }

        let flags = AccessFlags {
            ap: entry.ap,
            apx: entry.apx,
            execute_never: entry.execute_never,
        };
        let allowed = match flags.ap {
            0b00 => false,
            0b01 => privileged,
            0b10 => {
                privileged || matches!(access, MemoryAccessKind::Read | MemoryAccessKind::Execute)
            }
            0b11 => {
                if flags.apx {
                    !matches!(access, MemoryAccessKind::Write)
                } else {
                    true
                }
            }
            _ => false,
        };

        if !allowed {
            return Err(EmulatorError::MmuPermissionFault {
                pc: 0,
                va,
                pa: None,
                access,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::Memory;

    fn section_desc(pa_base: u32, domain: u32, ap: u32) -> u32 {
        (pa_base & SECTION_BASE_MASK) | (domain << 5) | (ap << 10) | SECTION_DESCRIPTOR_VALUE
    }

    #[test]
    fn translates_mapped_section_and_caches_tlb() {
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, section_desc(0x0800_0000, 0, 0b11))
            .unwrap_or_else(|e| panic!("write descriptor: {e}"));

        let mut mmu = Mmu::new();
        mmu.write_ttbr0(0x0000_4000);
        mmu.write_dacr(0b01);
        mmu.write_control(1);

        let pa = mmu
            .translate(&mut memory, 0x0000_1234, MemoryAccessKind::Read, false)
            .unwrap_or_else(|e| panic!("translation should succeed: {e}"));
        assert_eq!(pa, 0x0800_1234);
        assert_eq!(mmu.tlb_len(), 1);
    }

    #[test]
    fn faults_on_unmapped_section() {
        let mut memory = Memory::new();
        let mut mmu = Mmu::new();
        mmu.write_ttbr0(0x0000_4000);
        mmu.write_dacr(0b01);
        mmu.write_control(1);

        let err = mmu
            .translate(&mut memory, 0x0020_0000, MemoryAccessKind::Read, false)
            .err()
            .unwrap_or_else(|| panic!("expected translation fault"));
        assert_eq!(
            err,
            EmulatorError::MmuTranslationFault {
                pc: 0,
                va: 0x0020_0000,
                pa: None,
                access: MemoryAccessKind::Read,
            }
        );
    }

    #[test]
    fn enforces_privileged_permission_for_ap01() {
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, section_desc(0x0010_0000, 0, 0b01))
            .unwrap_or_else(|e| panic!("write descriptor: {e}"));

        let mut mmu = Mmu::new();
        mmu.write_ttbr0(0x0000_4000);
        mmu.write_dacr(0b01);
        mmu.write_control(1);

        let user_err = mmu
            .translate(&mut memory, 0x0000_1000, MemoryAccessKind::Read, false)
            .err()
            .unwrap_or_else(|| panic!("expected user permission fault"));
        assert_eq!(
            user_err,
            EmulatorError::MmuPermissionFault {
                pc: 0,
                va: 0x0000_1000,
                pa: None,
                access: MemoryAccessKind::Read,
            }
        );

        let pa = mmu
            .translate(&mut memory, 0x0000_1000, MemoryAccessKind::Read, true)
            .unwrap_or_else(|e| panic!("privileged read should succeed: {e}"));
        assert_eq!(pa, 0x0010_1000);
    }

    #[test]
    fn execute_never_section_faults_on_instruction_fetch() {
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, section_desc(0x0010_0000, 0, 0b11) | (1 << 4))
            .unwrap_or_else(|e| panic!("write descriptor: {e}"));

        let mut mmu = Mmu::new();
        mmu.write_ttbr0(0x0000_4000);
        mmu.write_dacr(0b01);
        mmu.write_control(1);

        let err = mmu
            .translate_instruction(&mut memory, 0x0000_1000, true)
            .err()
            .unwrap_or_else(|| panic!("expected XN permission fault"));
        assert_eq!(
            err,
            EmulatorError::MmuPermissionFault {
                pc: 0,
                va: 0x0000_1000,
                pa: None,
                access: MemoryAccessKind::Execute,
            }
        );
    }

    #[test]
    fn control_write_updates_cache_state() {
        let mut mmu = Mmu::new();
        mmu.write_control((1 << 12) | (1 << 2) | 1);
        assert!(mmu.mmu_enabled());
        assert!(mmu.icache_enabled());
        assert!(mmu.dcache_enabled());

        mmu.write_control(0);
        assert!(!mmu.mmu_enabled());
        assert!(!mmu.icache_enabled());
        assert!(!mmu.dcache_enabled());
    }

    #[test]
    fn apx_read_only_section_rejects_writes() {
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, section_desc(0x0010_0000, 0, 0b11) | (1 << 15))
            .unwrap_or_else(|e| panic!("write descriptor: {e}"));

        let mut mmu = Mmu::new();
        mmu.write_ttbr0(0x0000_4000);
        mmu.write_dacr(0b01);
        mmu.write_control(1);

        let read_pa = mmu
            .translate_read(&mut memory, 0x0000_1000, false)
            .unwrap_or_else(|e| panic!("read should succeed: {e}"));
        assert_eq!(read_pa, 0x0010_1000);

        let err = mmu
            .translate_write(&mut memory, 0x0000_1000, false)
            .err()
            .unwrap_or_else(|| panic!("expected permission fault"));
        assert_eq!(
            err,
            EmulatorError::MmuPermissionFault {
                pc: 0,
                va: 0x0000_1000,
                pa: None,
                access: MemoryAccessKind::Write,
            }
        );
    }
}
