use super::error::MemoryAccessKind;
use super::error::Result;
use super::memory::Memory;
use super::mmu::Mmu;

const REG_COUNT: usize = 16;
const PC_INDEX: usize = 15;
const LR_INDEX: usize = 14;

const FLAG_N: u32 = 1 << 31;
const FLAG_Z: u32 = 1 << 30;
const FLAG_C: u32 = 1 << 29;
const FLAG_V: u32 = 1 << 28;

const MODE_MASK: u32 = 0x1F;
const MODE_USR: u32 = 0b1_0000;
const MODE_UND: u32 = 0b1_1011;
const MODE_SVC: u32 = 0b1_0011;

const VECTOR_BASE: u32 = 0x0010_0000;
const VECTOR_UND: u32 = VECTOR_BASE + 0x0000_0004;
const VECTOR_SWI: u32 = VECTOR_BASE + 0x0000_0008;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuRunState {
    Running,
    Halted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    UndefinedInstruction,
    SoftwareInterrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuException {
    pub kind: ExceptionKind,
    pub vector: u32,
    pub return_address: u32,
    pub fault_opcode: u32,
}

#[derive(Clone)]
pub struct Arm11Cpu {
    regs: [u32; REG_COUNT],
    cpsr: u32,
    spsr_und: u32,
    spsr_svc: u32,
    state: CpuRunState,
    last_exception: Option<CpuException>,
    cp15_regs: [u32; 16],
    mmu: Mmu,
}

impl Default for Arm11Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Arm11Cpu {
    pub fn new() -> Self {
        Self {
            regs: [0; REG_COUNT],
            cpsr: MODE_USR,
            spsr_und: MODE_USR,
            spsr_svc: MODE_USR,
            state: CpuRunState::Running,
            last_exception: None,
            cp15_regs: [0; 16],
            mmu: Mmu::new(),
        }
    }

    pub fn reset(&mut self, pc: u32) {
        self.regs = [0; REG_COUNT];
        self.regs[PC_INDEX] = pc;
        self.cpsr = MODE_USR;
        self.spsr_und = MODE_USR;
        self.spsr_svc = MODE_USR;
        self.state = CpuRunState::Running;
        self.last_exception = None;
        self.cp15_regs = [0; 16];
        self.mmu.reset();
    }

    pub fn run_state(&self) -> CpuRunState {
        self.state
    }

    pub fn pc(&self) -> u32 {
        self.regs[PC_INDEX]
    }

    pub fn regs(&self) -> &[u32; REG_COUNT] {
        &self.regs
    }

    pub fn cpsr(&self) -> u32 {
        self.cpsr
    }

    pub fn last_exception(&self) -> Option<CpuException> {
        self.last_exception
    }

    pub fn step(&mut self, memory: &mut Memory) -> Result<u32> {
        if self.state == CpuRunState::Halted {
            return Ok(1);
        }

        let pc = self.pc();
        let opcode = self.fetch_instruction(memory, pc)?;
        self.regs[PC_INDEX] = pc.wrapping_add(4);

        if !self.condition_passed(opcode >> 28) {
            return Ok(1);
        }

        if opcode == 0xE320_F003 {
            self.state = CpuRunState::Halted;
            return Ok(1);
        }

        if (opcode >> 24) & 0xF == 0xF {
            self.take_exception(ExceptionKind::SoftwareInterrupt, pc, opcode);
            return Ok(3);
        }

        if self.exec_system(opcode) {
            return Ok(1);
        }

        if self.exec_coprocessor(opcode, memory) {
            return Ok(2);
        }

        if (opcode >> 25) & 0x7 == 0b101 {
            self.exec_branch(opcode, pc);
            return Ok(2);
        }

        if (opcode >> 26) & 0x3 == 0b01 {
            self.exec_single_data_transfer(opcode, memory)?;
            return Ok(3);
        }

        if (opcode >> 26) & 0x3 == 0b00 {
            if self.exec_data_processing(opcode) {
                return Ok(1);
            }
            if self.exec_bx(opcode) {
                return Ok(2);
            }
            if self.exec_multiply(opcode) {
                return Ok(2);
            }
        }

        self.take_exception(ExceptionKind::UndefinedInstruction, pc, opcode);
        Ok(3)
    }

    fn fetch_instruction(&mut self, memory: &Memory, va: u32) -> Result<u32> {
        let pa = self.translate_va(memory, va, MemoryAccessKind::Execute)?;
        memory.read_u32_checked(pa)
    }

    fn translate_va(&mut self, memory: &Memory, va: u32, access: MemoryAccessKind) -> Result<u32> {
        self.mmu.translate(memory, va, access, self.is_privileged())
    }

    fn is_privileged(&self) -> bool {
        self.mode() != MODE_USR
    }

    fn exec_system(&mut self, opcode: u32) -> bool {
        // MRS CPSR/SPSR
        if opcode & 0x0FBF_0FFF == 0x010F_0000 {
            let rd = ((opcode >> 12) & 0xF) as usize;
            let spsr = ((opcode >> 22) & 1) == 1;
            self.regs[rd] = if spsr {
                self.current_spsr().unwrap_or(self.cpsr)
            } else {
                self.cpsr
            };
            return true;
        }

        // MSR CPSR_f, Rm (subset)
        if opcode & 0x0FB0_FFF0 == 0x0120_F000 {
            let rm = (opcode & 0xF) as usize;
            self.cpsr = (self.cpsr & !0xF000_0000)
                | (self.regs[rm] & 0xF000_0000)
                | (self.cpsr & MODE_MASK);
            return true;
        }

        false
    }

    fn exec_coprocessor(&mut self, opcode: u32, _memory: &mut Memory) -> bool {
        // Very small MRC/MCR CP15 subset model.
        if (opcode & 0x0F00_0010) == 0x0E00_0010 {
            let cp_num = (opcode >> 8) & 0xF;
            let rd = ((opcode >> 12) & 0xF) as usize;
            let crn = (opcode >> 16) & 0xF;
            let crm = opcode & 0xF;
            let opc2 = (opcode >> 5) & 0x7;
            let is_mrc = ((opcode >> 20) & 1) == 1;

            if cp_num == 15 {
                let idx = usize::try_from(crm & 0xF).unwrap_or(0);
                if is_mrc {
                    self.regs[rd] = match (crn, crm, opc2) {
                        (1, 0, 0) => self.cp15_regs[1],
                        (2, 0, 0) => self.cp15_regs[2],
                        (3, 0, 0) => self.cp15_regs[3],
                        _ => self.cp15_regs[idx],
                    };
                } else {
                    let value = self.regs[rd];
                    self.cp15_regs[idx] = value;
                    match (crn, crm, opc2) {
                        (1, 0, 0) => {
                            self.cp15_regs[1] = value;
                            self.mmu.write_control(value);
                        }
                        (2, 0, 0) => {
                            self.cp15_regs[2] = value;
                            self.mmu.write_ttbr0(value);
                        }
                        (3, 0, 0) => {
                            self.cp15_regs[3] = value;
                            self.mmu.write_dacr(value);
                        }
                        (8, 7, 0) | (8, 5, 0) | (8, 6, 0) => {
                            self.mmu.invalidate_tlb();
                        }
                        _ => {}
                    }
                }
                return true;
            }
        }
        false
    }

    fn exec_multiply(&mut self, opcode: u32) -> bool {
        if opcode & 0x0FC0_00F0 == 0x0000_0090 {
            let accumulate = ((opcode >> 21) & 1) == 1;
            let set_flags = ((opcode >> 20) & 1) == 1;
            let rd = ((opcode >> 16) & 0xF) as usize;
            let rn = ((opcode >> 12) & 0xF) as usize;
            let rs = ((opcode >> 8) & 0xF) as usize;
            let rm = (opcode & 0xF) as usize;

            let mut result = self.regs[rm].wrapping_mul(self.regs[rs]);
            if accumulate {
                result = result.wrapping_add(self.regs[rn]);
            }
            self.regs[rd] = result;
            if set_flags {
                self.update_nzcv_logical(result, self.cpsr & FLAG_C != 0);
            }
            return true;
        }
        false
    }

    fn exec_branch(&mut self, opcode: u32, pc: u32) {
        let link = ((opcode >> 24) & 1) == 1;
        let mut offset = ((opcode & 0x00FF_FFFF) << 2) as i32;
        if (offset & 0x0200_0000) != 0 {
            offset |= !0x03FF_FFFF;
        }

        if link {
            self.regs[LR_INDEX] = pc.wrapping_add(4);
        }
        let target = pc.wrapping_add(8).wrapping_add(offset as u32);
        self.regs[PC_INDEX] = target;
    }

    fn exec_single_data_transfer(&mut self, opcode: u32, memory: &mut Memory) -> Result<()> {
        let immediate_offset = ((opcode >> 25) & 1) == 0;
        let pre_index = ((opcode >> 24) & 1) == 1;
        let add = ((opcode >> 23) & 1) == 1;
        let write_back = ((opcode >> 21) & 1) == 1;
        let load = ((opcode >> 20) & 1) == 1;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;

        if !immediate_offset {
            return Ok(());
        }

        let offset = opcode & 0xFFF;
        let base = self.regs[rn];
        let effective = if add {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };

        let address = if pre_index { effective } else { base };
        if load {
            let pa = self.translate_va(memory, address, MemoryAccessKind::Read)?;
            self.regs[rd] = memory.read_u32_checked(pa)?;
        } else {
            let pa = self.translate_va(memory, address, MemoryAccessKind::Write)?;
            memory.write_u32_checked(pa, self.regs[rd])?;
        }

        if write_back || !pre_index {
            self.regs[rn] = effective;
        }

        Ok(())
    }

    fn exec_data_processing(&mut self, opcode: u32) -> bool {
        let immediate = ((opcode >> 25) & 1) == 1;
        let op = (opcode >> 21) & 0xF;
        let set_flags = ((opcode >> 20) & 1) == 1;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;

        let (operand2, shifter_carry) = if immediate {
            let imm8 = opcode & 0xFF;
            let rotate = ((opcode >> 8) & 0xF) * 2;
            let value = imm8.rotate_right(rotate);
            let carry = if rotate == 0 {
                self.cpsr & FLAG_C != 0
            } else {
                value & FLAG_N != 0
            };
            (value, carry)
        } else {
            let rm = (opcode & 0xF) as usize;
            let shift_imm = (opcode >> 7) & 0x1F;
            let shift_type = (opcode >> 5) & 0x3;
            self.shift_value(self.regs[rm], shift_imm, shift_type)
        };

        let lhs = self.regs[rn];
        let mut write_result = None;

        match op {
            0x0 => write_result = Some(lhs & operand2), // AND
            0x1 => write_result = Some(lhs ^ operand2), // EOR
            0x2 => {
                let (res, borrow) = lhs.overflowing_sub(operand2);
                write_result = Some(res);
                if set_flags {
                    let overflow = ((lhs ^ operand2) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, !borrow, overflow);
                }
            }
            0x3 => {
                let (res, borrow) = operand2.overflowing_sub(lhs);
                write_result = Some(res);
                if set_flags {
                    let overflow = ((operand2 ^ lhs) & (operand2 ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, !borrow, overflow);
                }
            }
            0x4 => {
                let (res, carry) = lhs.overflowing_add(operand2);
                write_result = Some(res);
                if set_flags {
                    let overflow = ((!(lhs ^ operand2)) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, carry, overflow);
                }
            }
            0x5 => {
                let c = if self.cpsr & FLAG_C != 0 { 1 } else { 0 };
                let (tmp, c1) = lhs.overflowing_add(operand2);
                let (res, c2) = tmp.overflowing_add(c);
                write_result = Some(res);
                if set_flags {
                    let overflow = ((!(lhs ^ operand2)) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, c1 || c2, overflow);
                }
            }
            0x6 => {
                let c = if self.cpsr & FLAG_C != 0 { 1 } else { 0 };
                let (tmp, b1) = lhs.overflowing_sub(operand2);
                let (res, b2) = tmp.overflowing_sub(1 - c);
                write_result = Some(res);
                if set_flags {
                    let overflow = ((lhs ^ operand2) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, !(b1 || b2), overflow);
                }
            }
            0x8 => {
                let res = lhs & operand2;
                if set_flags {
                    self.update_nzcv_logical(res, shifter_carry);
                }
            }
            0x9 => {
                let res = lhs ^ operand2;
                if set_flags {
                    self.update_nzcv_logical(res, shifter_carry);
                }
            }
            0xA => {
                let (res, borrow) = lhs.overflowing_sub(operand2);
                if set_flags {
                    let overflow = ((lhs ^ operand2) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, !borrow, overflow);
                }
            }
            0xB => {
                let (res, carry) = lhs.overflowing_add(operand2);
                if set_flags {
                    let overflow = ((!(lhs ^ operand2)) & (lhs ^ res) & FLAG_N) != 0;
                    self.update_nzcv_arithmetic(res, carry, overflow);
                }
            }
            0xC => write_result = Some(lhs | operand2),
            0xD => write_result = Some(operand2),
            0xE => write_result = Some(lhs & !operand2),
            0xF => write_result = Some(!operand2),
            _ => return false,
        }

        if let Some(result) = write_result {
            self.regs[rd] = result;
            if matches!(op, 0x0 | 0x1 | 0xC | 0xD | 0xE | 0xF) && set_flags {
                self.update_nzcv_logical(result, shifter_carry);
            }
            if set_flags && rd == PC_INDEX {
                self.restore_cpsr_from_spsr();
            }
        }

        true
    }

    fn exec_bx(&mut self, opcode: u32) -> bool {
        if opcode & 0x0FFF_FFF0 == 0x012F_FF10 {
            let rm = (opcode & 0xF) as usize;
            self.regs[PC_INDEX] = self.regs[rm] & !1;
            true
        } else {
            false
        }
    }

    fn shift_value(&self, value: u32, shift_imm: u32, shift_type: u32) -> (u32, bool) {
        match shift_type {
            0b00 => {
                if shift_imm == 0 {
                    (value, self.cpsr & FLAG_C != 0)
                } else {
                    (value << shift_imm, ((value >> (32 - shift_imm)) & 1) == 1)
                }
            }
            0b01 => {
                let shift = if shift_imm == 0 { 32 } else { shift_imm };
                if shift >= 32 {
                    (0, ((value >> 31) & 1) == 1)
                } else {
                    (value >> shift, ((value >> (shift - 1)) & 1) == 1)
                }
            }
            0b10 => {
                let shift = if shift_imm == 0 { 32 } else { shift_imm };
                let sign = (value & FLAG_N) != 0;
                let res = if shift >= 32 {
                    if sign { u32::MAX } else { 0 }
                } else {
                    ((value as i32) >> shift) as u32
                };
                (res, ((value >> (shift.saturating_sub(1).min(31))) & 1) == 1)
            }
            0b11 => {
                let shift = if shift_imm == 0 { 1 } else { shift_imm };
                let res = value.rotate_right(shift);
                (res, (res & FLAG_N) != 0)
            }
            _ => (value, self.cpsr & FLAG_C != 0),
        }
    }

    fn update_nzcv_logical(&mut self, result: u32, carry: bool) {
        self.set_flag(FLAG_N, result & FLAG_N != 0);
        self.set_flag(FLAG_Z, result == 0);
        self.set_flag(FLAG_C, carry);
    }

    fn update_nzcv_arithmetic(&mut self, result: u32, carry: bool, overflow: bool) {
        self.set_flag(FLAG_N, result & FLAG_N != 0);
        self.set_flag(FLAG_Z, result == 0);
        self.set_flag(FLAG_C, carry);
        self.set_flag(FLAG_V, overflow);
    }

    fn set_flag(&mut self, mask: u32, set: bool) {
        if set {
            self.cpsr |= mask;
        } else {
            self.cpsr &= !mask;
        }
    }

    fn mode(&self) -> u32 {
        self.cpsr & MODE_MASK
    }

    fn current_spsr(&self) -> Option<u32> {
        match self.mode() {
            MODE_UND => Some(self.spsr_und),
            MODE_SVC => Some(self.spsr_svc),
            _ => None,
        }
    }

    fn restore_cpsr_from_spsr(&mut self) {
        if let Some(saved) = self.current_spsr() {
            self.cpsr = saved;
        }
    }

    fn take_exception(&mut self, kind: ExceptionKind, pc: u32, fault_opcode: u32) {
        let (vector, mode) = match kind {
            ExceptionKind::UndefinedInstruction => (VECTOR_UND, MODE_UND),
            ExceptionKind::SoftwareInterrupt => (VECTOR_SWI, MODE_SVC),
        };

        match mode {
            MODE_UND => self.spsr_und = self.cpsr,
            MODE_SVC => self.spsr_svc = self.cpsr,
            _ => {}
        }

        self.regs[LR_INDEX] = pc.wrapping_add(4);
        self.regs[PC_INDEX] = vector;
        self.cpsr = (self.cpsr & !MODE_MASK) | mode;
        self.last_exception = Some(CpuException {
            kind,
            vector,
            return_address: pc.wrapping_add(4),
            fault_opcode,
        });
    }

    fn condition_passed(&self, cond: u32) -> bool {
        let n = self.cpsr & FLAG_N != 0;
        let z = self.cpsr & FLAG_Z != 0;
        let c = self.cpsr & FLAG_C != 0;
        let v = self.cpsr & FLAG_V != 0;

        match cond {
            0x0 => z,
            0x1 => !z,
            0x2 => c,
            0x3 => !c,
            0x4 => n,
            0x5 => !n,
            0x6 => v,
            0x7 => !v,
            0x8 => c && !z,
            0x9 => !c || z,
            0xA => n == v,
            0xB => n != v,
            0xC => !z && (n == v),
            0xD => z || (n != v),
            0xE => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::EmulatorError;

    fn mcr_cp15(crn: u32, rd: usize, crm: u32, opc2: u32) -> u32 {
        0xEE00_0010 | (crn << 16) | ((rd as u32) << 12) | (15 << 8) | (opc2 << 5) | crm
    }

    #[test]
    fn instruction_fetch_faults_when_mmu_mapping_missing() {
        let mut cpu = Arm11Cpu::new();
        let mut memory = Memory::new();

        cpu.regs[0] = 0x0000_4000;
        assert!(cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory)); // TTBR0
        cpu.regs[0] = 0b01;
        assert!(cpu.exec_coprocessor(mcr_cp15(3, 0, 0, 0), &mut memory)); // DACR
        cpu.regs[0] = 1;
        assert!(cpu.exec_coprocessor(mcr_cp15(1, 0, 0, 0), &mut memory)); // SCTLR.M

        cpu.regs[PC_INDEX] = 0x0010_0000;
        let err = cpu
            .step(&mut memory)
            .err()
            .unwrap_or_else(|| panic!("expected fetch fault"));
        assert_eq!(
            err,
            EmulatorError::MmuTranslationFault {
                va: 0x0010_0000,
                access: MemoryAccessKind::Execute,
            }
        );
    }

    #[test]
    fn mcr_ttbr_write_invalidates_cached_translation() {
        let mut cpu = Arm11Cpu::new();
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, 0x0800_0000 | (0b11 << 10) | 0b10)
            .unwrap_or_else(|e| panic!("descriptor write: {e}"));

        cpu.regs[0] = 0x0000_4000;
        cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory);
        cpu.regs[0] = 0b01;
        cpu.exec_coprocessor(mcr_cp15(3, 0, 0, 0), &mut memory);
        cpu.regs[0] = 1;
        cpu.exec_coprocessor(mcr_cp15(1, 0, 0, 0), &mut memory);

        cpu.mmu
            .translate(&memory, 0x0000_1000, MemoryAccessKind::Read, true)
            .unwrap_or_else(|e| panic!("initial translation: {e}"));
        assert_eq!(cpu.mmu.tlb_len(), 1);

        cpu.regs[0] = 0x0000_8000;
        cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory);
        assert_eq!(cpu.mmu.tlb_len(), 0);
    }
}
