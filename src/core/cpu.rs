use super::error::EmulatorError;
use super::error::MemoryAccessKind;
use super::error::Result;
use super::irq::IrqLine;
use super::memory::Memory;
use super::mmu::Mmu;

const REG_COUNT: usize = 16;
const PC_INDEX: usize = 15;
const LR_INDEX: usize = 14;
const SP_INDEX: usize = 13;

const FLAG_N: u32 = 1 << 31;
const FLAG_Z: u32 = 1 << 30;
const FLAG_C: u32 = 1 << 29;
const FLAG_V: u32 = 1 << 28;
const FLAG_I: u32 = 1 << 7;
const FLAG_T: u32 = 1 << 5;

const MODE_MASK: u32 = 0x1F;
const MODE_USR: u32 = 0b1_0000;
const MODE_IRQ: u32 = 0b1_0010;
const MODE_SVC: u32 = 0b1_0011;
const MODE_ABT: u32 = 0b1_0111;
const MODE_UND: u32 = 0b1_1011;

const VECTOR_BASE: u32 = 0x0010_0000;
const VECTOR_UND: u32 = VECTOR_BASE + 0x0000_0004;
const VECTOR_SWI: u32 = VECTOR_BASE + 0x0000_0008;
const VECTOR_PABT: u32 = VECTOR_BASE + 0x0000_000C;
const VECTOR_DABT: u32 = VECTOR_BASE + 0x0000_0010;
const VECTOR_IRQ: u32 = VECTOR_BASE + 0x0000_0018;

const CP15_DFSR: usize = 5;
const CP15_IFSR: usize = 6;
const CP15_DFAR: usize = 7;
const CP15_IFAR: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuRunState {
    Running,
    Halted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    Translation,
    Domain,
    Permission,
    Alignment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionKind {
    UndefinedInstruction,
    SoftwareInterrupt,
    PrefetchAbort(FaultKind),
    DataAbort(FaultKind),
    Interrupt(IrqLine),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuException {
    pub kind: ExceptionKind,
    pub vector: u32,
    pub return_address: u32,
    pub fault_opcode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionTraceEntry {
    pub pc: u32,
    pub opcode: u32,
    pub thumb: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmuFaultDetail {
    pub va: u32,
    pub pa: Option<u32>,
    pub access: MemoryAccessKind,
    pub kind: FaultKind,
}

#[derive(Clone)]
pub struct Arm11Cpu {
    regs: [u32; REG_COUNT],
    cpsr: u32,
    spsr_und: u32,
    spsr_svc: u32,
    spsr_irq: u32,
    spsr_abt: u32,
    bank_usr_sp: u32,
    bank_usr_lr: u32,
    bank_svc_sp: u32,
    bank_svc_lr: u32,
    bank_irq_sp: u32,
    bank_irq_lr: u32,
    bank_und_sp: u32,
    bank_und_lr: u32,
    bank_abt_sp: u32,
    bank_abt_lr: u32,
    state: CpuRunState,
    last_exception: Option<CpuException>,
    cp15_regs: [u32; 16],
    mmu: Mmu,
    trace_enabled: bool,
    trace_limit: usize,
    trace_log: Vec<InstructionTraceEntry>,
    last_trace_entry: Option<InstructionTraceEntry>,
    last_mmu_fault: Option<MmuFaultDetail>,
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
            spsr_irq: MODE_USR,
            spsr_abt: MODE_USR,
            bank_usr_sp: 0,
            bank_usr_lr: 0,
            bank_svc_sp: 0,
            bank_svc_lr: 0,
            bank_irq_sp: 0,
            bank_irq_lr: 0,
            bank_und_sp: 0,
            bank_und_lr: 0,
            bank_abt_sp: 0,
            bank_abt_lr: 0,
            state: CpuRunState::Running,
            last_exception: None,
            cp15_regs: [0; 16],
            mmu: Mmu::new(),
            trace_enabled: false,
            trace_limit: 0,
            trace_log: Vec::new(),
            last_trace_entry: None,
            last_mmu_fault: None,
        }
    }

    pub fn reset(&mut self, pc: u32) {
        self.regs = [0; REG_COUNT];
        self.regs[PC_INDEX] = pc;
        self.cpsr = MODE_USR;
        self.spsr_und = MODE_USR;
        self.spsr_svc = MODE_USR;
        self.spsr_irq = MODE_USR;
        self.spsr_abt = MODE_USR;
        self.bank_usr_sp = 0;
        self.bank_usr_lr = 0;
        self.bank_svc_sp = 0;
        self.bank_svc_lr = 0;
        self.bank_irq_sp = 0;
        self.bank_irq_lr = 0;
        self.bank_und_sp = 0;
        self.bank_und_lr = 0;
        self.bank_abt_sp = 0;
        self.bank_abt_lr = 0;
        self.state = CpuRunState::Running;
        self.last_exception = None;
        self.cp15_regs = [0; 16];
        self.mmu.reset();
        self.trace_log.clear();
        self.last_trace_entry = None;
        self.last_mmu_fault = None;
    }

    pub fn enable_instruction_trace(&mut self, limit: usize) {
        self.trace_enabled = limit > 0;
        self.trace_limit = limit;
        self.trace_log.clear();
    }

    pub fn instruction_trace(&self) -> &[InstructionTraceEntry] {
        &self.trace_log
    }

    pub fn take_last_instruction_trace(&mut self) -> Option<InstructionTraceEntry> {
        self.last_trace_entry.take()
    }

    pub fn take_last_mmu_fault(&mut self) -> Option<MmuFaultDetail> {
        self.last_mmu_fault.take()
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

    pub fn interrupts_enabled(&self) -> bool {
        self.cpsr & FLAG_I == 0
    }

    pub fn enter_irq(&mut self, line: IrqLine) {
        if self.state == CpuRunState::Halted {
            self.state = CpuRunState::Running;
        }
        let pc = self.pc();
        self.take_exception(ExceptionKind::Interrupt(line), pc, line as u32, true);
    }

    pub fn step(&mut self, memory: &mut Memory) -> Result<u32> {
        self.last_trace_entry = None;
        self.last_mmu_fault = None;
        if self.state == CpuRunState::Halted {
            return Ok(1);
        }

        if self.is_thumb() {
            self.step_thumb(memory)
        } else {
            self.step_arm(memory)
        }
    }

    fn step_arm(&mut self, memory: &mut Memory) -> Result<u32> {
        let pc = self.pc();
        let opcode = match self.fetch_instruction(memory, pc) {
            Ok(op) => op,
            Err(kind) => {
                self.take_prefetch_abort(kind, pc, 0);
                return Ok(3);
            }
        };

        self.record_trace(pc, opcode, false);
        self.regs[PC_INDEX] = pc.wrapping_add(4);

        if !self.condition_passed(opcode >> 28) {
            return Ok(1);
        }

        if opcode == 0xE320_F003 {
            self.state = CpuRunState::Halted;
            return Ok(1);
        }

        if (opcode >> 24) & 0xF == 0xF {
            self.take_exception(ExceptionKind::SoftwareInterrupt, pc, opcode, true);
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
            return match self.exec_single_data_transfer(opcode, memory) {
                Ok(_) => Ok(3),
                Err(kind) => {
                    self.take_data_abort(kind, pc, opcode);
                    Ok(3)
                }
            };
        }

        if (opcode >> 25) & 0x7 == 0b000 && (opcode & 0x90) == 0x90 {
            return match self.exec_halfword_data_transfer(opcode, memory) {
                Ok(true) => Ok(3),
                Ok(false) => Ok(1),
                Err(kind) => {
                    self.take_data_abort(kind, pc, opcode);
                    Ok(3)
                }
            };
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

        self.take_exception(ExceptionKind::UndefinedInstruction, pc, opcode, true);
        Ok(3)
    }

    fn step_thumb(&mut self, memory: &mut Memory) -> Result<u32> {
        let pc = self.pc();
        let opcode = match self.fetch_thumb_instruction(memory, pc) {
            Ok(op) => op,
            Err(kind) => {
                self.take_prefetch_abort(kind, pc, 0);
                return Ok(3);
            }
        };

        self.record_trace(pc, u32::from(opcode), true);
        self.regs[PC_INDEX] = pc.wrapping_add(2);

        if self.exec_thumb_shift_imm(opcode)
            || self.exec_thumb_add_sub(opcode)
            || self.exec_thumb_mov_cmp_add_sub_imm(opcode)
            || self.exec_thumb_hi_reg_bx(opcode)
            || self.exec_thumb_cond_branch(opcode)
            || self.exec_thumb_uncond_branch(opcode)
        {
            return Ok(1);
        }

        if self.exec_thumb_ldr_literal(opcode, memory, pc)
            || self.exec_thumb_load_store_imm(opcode, memory, pc)
        {
            return Ok(1);
        }

        self.take_exception(
            ExceptionKind::UndefinedInstruction,
            pc,
            u32::from(opcode),
            true,
        );
        Ok(3)
    }

    fn record_trace(&mut self, pc: u32, opcode: u32, thumb: bool) {
        if !self.trace_enabled {
            return;
        }
        self.trace_log
            .push(InstructionTraceEntry { pc, opcode, thumb });
        self.last_trace_entry = Some(InstructionTraceEntry { pc, opcode, thumb });
        if self.trace_log.len() > self.trace_limit {
            let excess = self.trace_log.len() - self.trace_limit;
            self.trace_log.drain(0..excess);
        }
    }

    fn fetch_instruction(
        &mut self,
        memory: &Memory,
        va: u32,
    ) -> std::result::Result<u32, FaultKind> {
        if va & 3 != 0 {
            return Err(FaultKind::Alignment);
        }
        let pa = self.translate_instruction_va(memory, va)?;
        memory
            .read_u32_checked(pa)
            .map_err(|_| FaultKind::Translation)
    }

    fn fetch_thumb_instruction(
        &mut self,
        memory: &Memory,
        va: u32,
    ) -> std::result::Result<u16, FaultKind> {
        if va & 1 != 0 {
            return Err(FaultKind::Alignment);
        }
        let pa = self.translate_instruction_va(memory, va)?;
        let lo = memory
            .read_u8_checked(pa)
            .map_err(|_| FaultKind::Translation)?;
        let hi = memory
            .read_u8_checked(pa.wrapping_add(1))
            .map_err(|_| FaultKind::Translation)?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    fn translate_instruction_va(
        &mut self,
        memory: &Memory,
        va: u32,
    ) -> std::result::Result<u32, FaultKind> {
        self.mmu
            .translate_instruction(memory, va, self.is_privileged())
            .map_err(|err| {
                let kind = Self::fault_kind_from_error(err);
                self.last_mmu_fault = Some(MmuFaultDetail {
                    va,
                    pa: None,
                    access: MemoryAccessKind::Execute,
                    kind,
                });
                kind
            })
    }

    fn translate_va(
        &mut self,
        memory: &Memory,
        va: u32,
        access: MemoryAccessKind,
    ) -> std::result::Result<u32, FaultKind> {
        let translated = match access {
            MemoryAccessKind::Read => self.mmu.translate_read(memory, va, self.is_privileged()),
            MemoryAccessKind::Write => self.mmu.translate_write(memory, va, self.is_privileged()),
            MemoryAccessKind::Execute => {
                self.mmu
                    .translate_instruction(memory, va, self.is_privileged())
            }
        };
        translated.map_err(|err| {
            let kind = Self::fault_kind_from_error(err);
            self.last_mmu_fault = Some(MmuFaultDetail {
                va,
                pa: None,
                access,
                kind,
            });
            kind
        })
    }

    fn fault_kind_from_error(err: EmulatorError) -> FaultKind {
        match err {
            EmulatorError::MmuTranslationFault { .. } => FaultKind::Translation,
            EmulatorError::MmuDomainFault { .. } => FaultKind::Domain,
            EmulatorError::MmuPermissionFault { .. } => FaultKind::Permission,
            EmulatorError::AlignmentFault { .. } => FaultKind::Alignment,
            _ => FaultKind::Translation,
        }
    }

    fn is_privileged(&self) -> bool {
        self.mode() != MODE_USR
    }

    fn is_thumb(&self) -> bool {
        self.cpsr & FLAG_T != 0
    }

    fn exec_system(&mut self, opcode: u32) -> bool {
        if opcode == 0xF57F_F05F || opcode == 0xF57F_F04F {
            return true;
        }

        if opcode & 0x0FF0_00F0 == 0x0160_0010 {
            return true;
        }

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

        if opcode & 0x0FB0_FFF0 == 0x0120_F000 {
            let rm = (opcode & 0xF) as usize;
            let value = self.regs[rm];
            self.cpsr =
                (self.cpsr & !0xF000_0000) | (value & 0xF000_0000) | (self.cpsr & MODE_MASK);
            if self.is_privileged() {
                let new_mode = value & MODE_MASK;
                if matches!(
                    new_mode,
                    MODE_USR | MODE_SVC | MODE_IRQ | MODE_UND | MODE_ABT
                ) {
                    self.switch_mode(new_mode);
                }
                self.set_thumb_state((value & FLAG_T) != 0);
                self.set_flag(FLAG_I, (value & FLAG_I) != 0);
            }
            return true;
        }

        false
    }

    fn exec_coprocessor(&mut self, opcode: u32, _memory: &mut Memory) -> bool {
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
                        (8, 7, 0) | (8, 5, 0) | (8, 6, 0) => self.mmu.invalidate_tlb(),
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

    fn exec_single_data_transfer(
        &mut self,
        opcode: u32,
        memory: &mut Memory,
    ) -> std::result::Result<(), FaultKind> {
        let immediate_offset = ((opcode >> 25) & 1) == 0;
        let pre_index = ((opcode >> 24) & 1) == 1;
        let add = ((opcode >> 23) & 1) == 1;
        let write_back = ((opcode >> 21) & 1) == 1;
        let load = ((opcode >> 20) & 1) == 1;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;

        let offset = if immediate_offset {
            opcode & 0xFFF
        } else {
            let rm = (opcode & 0xF) as usize;
            let shift_imm = (opcode >> 7) & 0x1F;
            let shift_type = (opcode >> 5) & 0x3;
            self.shift_value(self.regs[rm], shift_imm, shift_type).0
        };
        let base = self.regs[rn];
        let effective = if add {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };

        let address = if pre_index { effective } else { base };
        let is_byte = ((opcode >> 22) & 1) == 1;
        if !is_byte && address & 3 != 0 {
            return Err(FaultKind::Alignment);
        }

        if load {
            let pa = self.translate_va(memory, address, MemoryAccessKind::Read)?;
            self.regs[rd] = if is_byte {
                u32::from(
                    memory
                        .read_u8_checked(pa)
                        .map_err(|_| FaultKind::Translation)?,
                )
            } else {
                memory
                    .read_u32_checked(pa)
                    .map_err(|_| FaultKind::Translation)?
            };
        } else {
            let pa = self.translate_va(memory, address, MemoryAccessKind::Write)?;
            if is_byte {
                memory
                    .write_u8_checked(pa, (self.regs[rd] & 0xFF) as u8)
                    .map_err(|_| FaultKind::Translation)?;
            } else {
                memory
                    .write_u32_checked(pa, self.regs[rd])
                    .map_err(|_| FaultKind::Translation)?;
            }
        }

        if write_back || !pre_index {
            self.regs[rn] = effective;
        }

        Ok(())
    }

    fn exec_halfword_data_transfer(
        &mut self,
        opcode: u32,
        memory: &mut Memory,
    ) -> std::result::Result<bool, FaultKind> {
        if (opcode & 0x0E00_0090) != 0x0000_0090 {
            return Ok(false);
        }
        let pre_index = ((opcode >> 24) & 1) == 1;
        let add = ((opcode >> 23) & 1) == 1;
        let write_back = ((opcode >> 21) & 1) == 1;
        let load = ((opcode >> 20) & 1) == 1;
        let rn = ((opcode >> 16) & 0xF) as usize;
        let rd = ((opcode >> 12) & 0xF) as usize;
        let immediate = ((opcode >> 22) & 1) == 1;
        let offset = if immediate {
            ((opcode >> 4) & 0xF0) | (opcode & 0xF)
        } else {
            self.regs[(opcode & 0xF) as usize]
        };
        let base = self.regs[rn];
        let effective = if add {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let address = if pre_index { effective } else { base };
        let signed = ((opcode >> 6) & 1) == 1;
        let halfword = ((opcode >> 5) & 1) == 1;

        if halfword && (address & 1 != 0) {
            return Err(FaultKind::Alignment);
        }

        if load {
            let pa = self.translate_va(memory, address, MemoryAccessKind::Read)?;
            self.regs[rd] = if halfword {
                let lo = memory
                    .read_u8_checked(pa)
                    .map_err(|_| FaultKind::Translation)?;
                let hi = memory
                    .read_u8_checked(pa.wrapping_add(1))
                    .map_err(|_| FaultKind::Translation)?;
                let v = u16::from_le_bytes([lo, hi]);
                if signed {
                    i16::from_le_bytes(v.to_le_bytes()) as i32 as u32
                } else {
                    u32::from(v)
                }
            } else {
                let v = memory
                    .read_u8_checked(pa)
                    .map_err(|_| FaultKind::Translation)?;
                if signed {
                    i8::from_le_bytes([v]) as i32 as u32
                } else {
                    u32::from(v)
                }
            };
        } else {
            if signed || !halfword {
                return Ok(false);
            }
            let pa = self.translate_va(memory, address, MemoryAccessKind::Write)?;
            let bytes = (self.regs[rd] as u16).to_le_bytes();
            memory
                .write_u8_checked(pa, bytes[0])
                .map_err(|_| FaultKind::Translation)?;
            memory
                .write_u8_checked(pa.wrapping_add(1), bytes[1])
                .map_err(|_| FaultKind::Translation)?;
        }

        if write_back || !pre_index {
            self.regs[rn] = effective;
        }
        Ok(true)
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
            0x0 => write_result = Some(lhs & operand2),
            0x1 => write_result = Some(lhs ^ operand2),
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
                if set_flags {
                    self.update_nzcv_logical(lhs & operand2, shifter_carry);
                }
            }
            0x9 => {
                if set_flags {
                    self.update_nzcv_logical(lhs ^ operand2, shifter_carry);
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
            let target = self.regs[rm];
            self.set_thumb_state((target & 1) != 0);
            self.regs[PC_INDEX] = target & !1;
            true
        } else {
            false
        }
    }

    fn exec_thumb_shift_imm(&mut self, opcode: u16) -> bool {
        if (opcode >> 13) != 0 {
            return false;
        }
        let op = (opcode >> 11) & 0x3;
        let offset = u32::from((opcode >> 6) & 0x1F);
        let rs = usize::from((opcode >> 3) & 0x7);
        let rd = usize::from(opcode & 0x7);
        let value = self.regs[rs];
        let (result, carry) = match op {
            0b00 => {
                if offset == 0 {
                    (value, self.cpsr & FLAG_C != 0)
                } else {
                    (value << offset, ((value >> (32 - offset)) & 1) != 0)
                }
            }
            0b01 => {
                let s = if offset == 0 { 32 } else { offset };
                if s == 32 {
                    (0, (value >> 31) != 0)
                } else {
                    (value >> s, ((value >> (s - 1)) & 1) != 0)
                }
            }
            0b10 => {
                let s = if offset == 0 { 32 } else { offset };
                let result = if s == 32 {
                    if (value >> 31) != 0 { u32::MAX } else { 0 }
                } else {
                    ((value as i32) >> s) as u32
                };
                (result, ((value >> (s.saturating_sub(1).min(31))) & 1) != 0)
            }
            _ => return false,
        };

        self.regs[rd] = result;
        self.update_nzcv_logical(result, carry);
        true
    }

    fn exec_thumb_add_sub(&mut self, opcode: u16) -> bool {
        if (opcode & 0xF800) != 0x1800 {
            return false;
        }
        let immediate = ((opcode >> 10) & 1) != 0;
        let sub = ((opcode >> 9) & 1) != 0;
        let rn_or_imm = u32::from((opcode >> 6) & 0x7);
        let rs = usize::from((opcode >> 3) & 0x7);
        let rd = usize::from(opcode & 0x7);
        let lhs = self.regs[rs];
        let rhs = if immediate {
            rn_or_imm
        } else {
            self.regs[rn_or_imm as usize]
        };

        if sub {
            let (res, borrow) = lhs.overflowing_sub(rhs);
            let overflow = ((lhs ^ rhs) & (lhs ^ res) & FLAG_N) != 0;
            self.regs[rd] = res;
            self.update_nzcv_arithmetic(res, !borrow, overflow);
        } else {
            let (res, carry) = lhs.overflowing_add(rhs);
            let overflow = ((!(lhs ^ rhs)) & (lhs ^ res) & FLAG_N) != 0;
            self.regs[rd] = res;
            self.update_nzcv_arithmetic(res, carry, overflow);
        }
        true
    }

    fn exec_thumb_mov_cmp_add_sub_imm(&mut self, opcode: u16) -> bool {
        if (opcode & 0xE000) != 0x2000 {
            return false;
        }
        let op = (opcode >> 11) & 0x3;
        let rd = usize::from((opcode >> 8) & 0x7);
        let imm = u32::from(opcode & 0xFF);

        match op {
            0b00 => {
                self.regs[rd] = imm;
                self.update_nzcv_logical(imm, self.cpsr & FLAG_C != 0);
            }
            0b01 => {
                let lhs = self.regs[rd];
                let (res, borrow) = lhs.overflowing_sub(imm);
                let overflow = ((lhs ^ imm) & (lhs ^ res) & FLAG_N) != 0;
                self.update_nzcv_arithmetic(res, !borrow, overflow);
            }
            0b10 => {
                let lhs = self.regs[rd];
                let (res, carry) = lhs.overflowing_add(imm);
                let overflow = ((!(lhs ^ imm)) & (lhs ^ res) & FLAG_N) != 0;
                self.regs[rd] = res;
                self.update_nzcv_arithmetic(res, carry, overflow);
            }
            0b11 => {
                let lhs = self.regs[rd];
                let (res, borrow) = lhs.overflowing_sub(imm);
                let overflow = ((lhs ^ imm) & (lhs ^ res) & FLAG_N) != 0;
                self.regs[rd] = res;
                self.update_nzcv_arithmetic(res, !borrow, overflow);
            }
            _ => return false,
        }
        true
    }

    fn exec_thumb_hi_reg_bx(&mut self, opcode: u16) -> bool {
        if (opcode & 0xFC00) != 0x4400 {
            return false;
        }
        let op = (opcode >> 8) & 0x3;
        let h1 = ((opcode >> 7) & 1) as usize;
        let h2 = ((opcode >> 6) & 1) as usize;
        let rs = usize::from((opcode >> 3) & 0x7) | (h2 << 3);
        let rd = usize::from(opcode & 0x7) | (h1 << 3);

        match op {
            0b00 => self.regs[rd] = self.regs[rd].wrapping_add(self.regs[rs]),
            0b01 => {
                let lhs = self.regs[rd];
                let rhs = self.regs[rs];
                let (res, borrow) = lhs.overflowing_sub(rhs);
                let overflow = ((lhs ^ rhs) & (lhs ^ res) & FLAG_N) != 0;
                self.update_nzcv_arithmetic(res, !borrow, overflow);
            }
            0b10 => self.regs[rd] = self.regs[rs],
            0b11 => {
                let target = self.regs[rs];
                self.set_thumb_state((target & 1) != 0);
                self.regs[PC_INDEX] = target & !1;
            }
            _ => return false,
        }
        true
    }

    fn exec_thumb_ldr_literal(&mut self, opcode: u16, memory: &mut Memory, pc: u32) -> bool {
        if (opcode & 0xF800) != 0x4800 {
            return false;
        }
        let rd = usize::from((opcode >> 8) & 0x7);
        let imm = u32::from(opcode & 0xFF) << 2;
        let address = (self.regs[PC_INDEX] & !3).wrapping_add(imm);
        if address & 3 != 0 {
            self.take_data_abort(FaultKind::Alignment, pc, u32::from(opcode));
            return true;
        }
        let pa = match self.translate_va(memory, address, MemoryAccessKind::Read) {
            Ok(pa) => pa,
            Err(kind) => {
                self.take_data_abort(kind, pc, u32::from(opcode));
                return true;
            }
        };
        match memory.read_u32_checked(pa) {
            Ok(v) => self.regs[rd] = v,
            Err(_) => self.take_data_abort(FaultKind::Translation, pc, u32::from(opcode)),
        }
        true
    }

    fn exec_thumb_load_store_imm(&mut self, opcode: u16, memory: &mut Memory, pc: u32) -> bool {
        if (opcode & 0xE000) != 0x6000 {
            return false;
        }
        let load = ((opcode >> 11) & 1) != 0;
        let imm5 = u32::from((opcode >> 6) & 0x1F) << 2;
        let rb = usize::from((opcode >> 3) & 0x7);
        let rd = usize::from(opcode & 0x7);
        let address = self.regs[rb].wrapping_add(imm5);

        if address & 3 != 0 {
            self.take_data_abort(FaultKind::Alignment, pc, u32::from(opcode));
            return true;
        }

        if load {
            let pa = match self.translate_va(memory, address, MemoryAccessKind::Read) {
                Ok(pa) => pa,
                Err(kind) => {
                    self.take_data_abort(kind, pc, u32::from(opcode));
                    return true;
                }
            };
            self.regs[rd] = match memory.read_u32_checked(pa) {
                Ok(v) => v,
                Err(_) => {
                    self.take_data_abort(FaultKind::Translation, pc, u32::from(opcode));
                    return true;
                }
            };
        } else {
            let pa = match self.translate_va(memory, address, MemoryAccessKind::Write) {
                Ok(pa) => pa,
                Err(kind) => {
                    self.take_data_abort(kind, pc, u32::from(opcode));
                    return true;
                }
            };
            if memory.write_u32_checked(pa, self.regs[rd]).is_err() {
                self.take_data_abort(FaultKind::Translation, pc, u32::from(opcode));
                return true;
            }
        }

        true
    }

    fn exec_thumb_cond_branch(&mut self, opcode: u16) -> bool {
        if (opcode & 0xF000) != 0xD000 || (opcode & 0x0F00) == 0x0F00 {
            return false;
        }
        let cond = u32::from((opcode >> 8) & 0xF);
        if !self.condition_passed(cond) {
            return true;
        }
        let offset = i32::from((opcode & 0xFF) as i8) << 1;
        self.regs[PC_INDEX] = self.regs[PC_INDEX].wrapping_add(offset as u32);
        true
    }

    fn exec_thumb_uncond_branch(&mut self, opcode: u16) -> bool {
        if (opcode & 0xF800) != 0xE000 {
            return false;
        }
        let mut offset = i32::from((opcode & 0x07FF) as i16) << 1;
        if (offset & 0x0800) != 0 {
            offset |= !0x0FFF;
        }
        self.regs[PC_INDEX] = self.regs[PC_INDEX].wrapping_add(offset as u32);
        true
    }

    fn set_thumb_state(&mut self, thumb: bool) {
        self.set_flag(FLAG_T, thumb);
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
            MODE_IRQ => Some(self.spsr_irq),
            MODE_ABT => Some(self.spsr_abt),
            _ => None,
        }
    }

    fn set_current_spsr(&mut self, value: u32) {
        match self.mode() {
            MODE_UND => self.spsr_und = value,
            MODE_SVC => self.spsr_svc = value,
            MODE_IRQ => self.spsr_irq = value,
            MODE_ABT => self.spsr_abt = value,
            _ => {}
        }
    }

    fn banked_sp_lr_mut(&mut self, mode: u32) -> (&mut u32, &mut u32) {
        match mode {
            MODE_USR => (&mut self.bank_usr_sp, &mut self.bank_usr_lr),
            MODE_SVC => (&mut self.bank_svc_sp, &mut self.bank_svc_lr),
            MODE_IRQ => (&mut self.bank_irq_sp, &mut self.bank_irq_lr),
            MODE_UND => (&mut self.bank_und_sp, &mut self.bank_und_lr),
            MODE_ABT => (&mut self.bank_abt_sp, &mut self.bank_abt_lr),
            _ => (&mut self.bank_usr_sp, &mut self.bank_usr_lr),
        }
    }

    fn switch_mode(&mut self, new_mode: u32) {
        let old_mode = self.mode();
        if old_mode == new_mode {
            return;
        }

        let old_sp = self.regs[SP_INDEX];
        let old_lr = self.regs[LR_INDEX];
        {
            let (sp, lr) = self.banked_sp_lr_mut(old_mode);
            *sp = old_sp;
            *lr = old_lr;
        }

        let (new_sp, new_lr) = {
            let (sp, lr) = self.banked_sp_lr_mut(new_mode);
            (*sp, *lr)
        };
        self.regs[SP_INDEX] = new_sp;
        self.regs[LR_INDEX] = new_lr;
        self.cpsr = (self.cpsr & !MODE_MASK) | new_mode;
    }

    fn restore_cpsr_from_spsr(&mut self) {
        if let Some(saved) = self.current_spsr() {
            self.switch_mode(saved & MODE_MASK);
            self.cpsr = saved;
        }
    }

    fn take_prefetch_abort(&mut self, fault: FaultKind, pc: u32, opcode: u32) {
        self.cp15_regs[CP15_IFAR] = pc;
        self.cp15_regs[CP15_IFSR] = self.encode_fault_status(fault);
        self.take_exception(ExceptionKind::PrefetchAbort(fault), pc, opcode, true);
    }

    fn take_data_abort(&mut self, fault: FaultKind, pc: u32, opcode: u32) {
        self.cp15_regs[CP15_DFAR] = self.last_mmu_fault.map(|f| f.va).unwrap_or(pc);
        self.cp15_regs[CP15_DFSR] = self.encode_fault_status(fault);
        self.take_exception(ExceptionKind::DataAbort(fault), pc, opcode, false);
    }

    fn encode_fault_status(&self, fault: FaultKind) -> u32 {
        match fault {
            FaultKind::Translation => 0b00101,
            FaultKind::Domain => 0b01001,
            FaultKind::Permission => 0b01101,
            FaultKind::Alignment => 0b00001,
        }
    }

    fn take_exception(&mut self, kind: ExceptionKind, pc: u32, fault_opcode: u32, lr_plus_4: bool) {
        let (vector, mode) = match kind {
            ExceptionKind::UndefinedInstruction => (VECTOR_UND, MODE_UND),
            ExceptionKind::SoftwareInterrupt => (VECTOR_SWI, MODE_SVC),
            ExceptionKind::PrefetchAbort(_) => (VECTOR_PABT, MODE_ABT),
            ExceptionKind::DataAbort(_) => (VECTOR_DABT, MODE_ABT),
            ExceptionKind::Interrupt(_) => (VECTOR_IRQ, MODE_IRQ),
        };

        let return_addr = if matches!(kind, ExceptionKind::PrefetchAbort(_)) {
            pc.wrapping_add(4)
        } else if matches!(kind, ExceptionKind::DataAbort(_)) {
            pc.wrapping_add(8)
        } else if lr_plus_4 {
            pc.wrapping_add(4)
        } else {
            pc
        };

        self.set_current_spsr(self.cpsr);
        self.switch_mode(mode);
        self.regs[LR_INDEX] = return_addr;
        self.regs[PC_INDEX] = vector;
        self.cpsr |= FLAG_I;
        self.cpsr &= !FLAG_T;
        self.last_exception = Some(CpuException {
            kind,
            vector,
            return_address: return_addr,
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

    fn mcr_cp15(crn: u32, rd: usize, crm: u32, opc2: u32) -> u32 {
        0xEE00_0010 | (crn << 16) | ((rd as u32) << 12) | (15 << 8) | (opc2 << 5) | crm
    }

    #[test]
    fn instruction_fetch_fault_routes_to_prefetch_abort() {
        let mut cpu = Arm11Cpu::new();
        let mut memory = Memory::new();

        cpu.regs[0] = 0x0000_4000;
        assert!(cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory));
        cpu.regs[0] = 0b01;
        assert!(cpu.exec_coprocessor(mcr_cp15(3, 0, 0, 0), &mut memory));
        cpu.regs[0] = 1;
        assert!(cpu.exec_coprocessor(mcr_cp15(1, 0, 0, 0), &mut memory));

        cpu.regs[PC_INDEX] = 0x0010_0000;
        cpu.step(&mut memory).expect("step must handle abort");
        let ex = cpu.last_exception().expect("prefetch abort raised");
        assert_eq!(
            ex.kind,
            ExceptionKind::PrefetchAbort(FaultKind::Translation)
        );
        assert_eq!(ex.vector, VECTOR_PABT);
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

    #[test]
    fn write_to_apx_read_only_section_routes_to_data_abort() {
        let mut cpu = Arm11Cpu::new();
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, (1 << 15) | (0b11 << 10) | 0b10)
            .unwrap_or_else(|e| panic!("descriptor write: {e}"));
        memory
            .write_u32_checked(0x0000_4004, 0x0010_0000 | (0b11 << 10) | 0b10)
            .unwrap_or_else(|e| panic!("descriptor write: {e}"));
        memory.write_u32(0x0010_0000, 0xE5801000); // str r1, [r0]

        cpu.regs[0] = 0x0000_4000;
        cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory);
        cpu.regs[0] = 0b01;
        cpu.exec_coprocessor(mcr_cp15(3, 0, 0, 0), &mut memory);
        cpu.regs[0] = 1;
        cpu.exec_coprocessor(mcr_cp15(1, 0, 0, 0), &mut memory);

        cpu.regs[PC_INDEX] = 0x0010_0000;
        cpu.regs[0] = 0;
        cpu.regs[1] = 0x1122_3344;
        cpu.step(&mut memory).expect("step must handle abort");

        let ex = cpu.last_exception().expect("data abort raised");
        assert_eq!(ex.kind, ExceptionKind::DataAbort(FaultKind::Permission));
        assert_eq!(ex.vector, VECTOR_DABT);
    }

    #[test]
    fn execute_never_section_routes_to_prefetch_abort_permission() {
        let mut cpu = Arm11Cpu::new();
        let mut memory = Memory::new();
        memory
            .write_u32_checked(0x0000_4000, 0x0800_0000 | (1 << 4) | (0b11 << 10) | 0b10)
            .unwrap_or_else(|e| panic!("descriptor write: {e}"));
        memory.write_u32(0x0800_0000, 0xE1A0_0000); // nop

        cpu.regs[0] = 0x0000_4000;
        cpu.exec_coprocessor(mcr_cp15(2, 0, 0, 0), &mut memory);
        cpu.regs[0] = 0b01;
        cpu.exec_coprocessor(mcr_cp15(3, 0, 0, 0), &mut memory);
        cpu.regs[0] = 1;
        cpu.exec_coprocessor(mcr_cp15(1, 0, 0, 0), &mut memory);

        cpu.regs[PC_INDEX] = 0;
        cpu.step(&mut memory).expect("step must handle abort");

        let ex = cpu.last_exception().expect("prefetch abort raised");
        assert_eq!(ex.kind, ExceptionKind::PrefetchAbort(FaultKind::Permission));
        assert_eq!(ex.vector, VECTOR_PABT);
    }

    #[test]
    fn arm_byte_transfer_and_register_offset_work() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();
        cpu.regs[PC_INDEX] = 0;
        cpu.regs[0] = 0x100;
        cpu.regs[1] = 0xAB;
        cpu.regs[2] = 0x10;

        mem.write_u32(0, 0xE7C0_1002); // strb r1, [r0, r2]
        mem.write_u32(4, 0xE7D0_3002); // ldrb r3, [r0, r2]

        cpu.step(&mut mem).expect("strb executes");
        cpu.step(&mut mem).expect("ldrb executes");

        assert_eq!(mem.read_u8(0x110), 0xAB);
        assert_eq!(cpu.regs[3], 0xAB);
    }

    #[test]
    fn arm_signed_halfword_load_sign_extends() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();
        cpu.regs[PC_INDEX] = 0;
        cpu.regs[0] = 0x200;
        mem.write_u8(0x200, 0x80);
        mem.write_u8(0x201, 0xFF);
        mem.write_u32(0, 0xE1D0_10F0); // ldrsh r1, [r0]

        cpu.step(&mut mem).expect("ldrsh executes");
        assert_eq!(cpu.regs[1], 0xFFFF_FF80);
    }

    #[test]
    fn thumb_data_abort_sets_abort_lr_offset() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();
        cpu.cpsr |= FLAG_T;
        cpu.regs[PC_INDEX] = 0;

        mem.write_u8(0, 0x00);
        mem.write_u8(1, 0x68); // ldr r0, [r0]
        cpu.regs[0] = 1; // unaligned

        cpu.step(&mut mem).expect("thumb step");
        let ex = cpu.last_exception().expect("data abort");
        assert_eq!(ex.kind, ExceptionKind::DataAbort(FaultKind::Alignment));
        assert_eq!(ex.return_address, 8);
        assert_eq!(cpu.regs[LR_INDEX], 8);
    }

    #[test]
    fn movs_pc_lr_restores_mode_and_banked_registers() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();

        cpu.regs[SP_INDEX] = 0x1000;
        cpu.regs[LR_INDEX] = 0x2000;
        cpu.regs[PC_INDEX] = 0;

        mem.write_u32(0, 0xEF00_0000); // swi
        cpu.step(&mut mem).expect("swi");

        cpu.regs[SP_INDEX] = 0x7777;
        cpu.regs[PC_INDEX] = VECTOR_SWI;
        mem.write_u32(VECTOR_SWI, 0xE1B0_F00E); // movs pc, lr
        cpu.step(&mut mem).expect("exception return");

        assert_eq!(cpu.mode(), MODE_USR);
        assert_eq!(cpu.regs[SP_INDEX], 0x1000);
        assert_eq!(cpu.regs[LR_INDEX], 0x2000);
        assert_eq!(cpu.pc(), 4);
    }
    #[test]
    fn banked_sp_lr_switch_between_usr_and_irq() {
        let mut cpu = Arm11Cpu::new();
        cpu.regs[SP_INDEX] = 0x1000;
        cpu.regs[LR_INDEX] = 0x2000;
        cpu.switch_mode(MODE_IRQ);
        cpu.regs[SP_INDEX] = 0x3000;
        cpu.regs[LR_INDEX] = 0x4000;
        cpu.switch_mode(MODE_USR);
        assert_eq!(cpu.regs[SP_INDEX], 0x1000);
        assert_eq!(cpu.regs[LR_INDEX], 0x2000);
    }

    #[test]
    fn thumb_fixture_conformance_sequence() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();
        cpu.cpsr |= FLAG_T;
        cpu.regs[PC_INDEX] = 0x0000_0000;

        // mov r0, #5 ; add r0,#3 ; sub r0,#1 ; b +0
        mem.write_u8(0, 0x05);
        mem.write_u8(1, 0x20);
        mem.write_u8(2, 0x03);
        mem.write_u8(3, 0x30);
        mem.write_u8(4, 0x01);
        mem.write_u8(5, 0x38);
        mem.write_u8(6, 0x00);
        mem.write_u8(7, 0xE0);

        for _ in 0..3 {
            cpu.step(&mut mem).expect("thumb step");
        }

        assert_eq!(cpu.regs[0], 7);
    }

    #[test]
    fn boot_trace_replay_records_first_failure_context() {
        let mut cpu = Arm11Cpu::new();
        let mut mem = Memory::new();
        cpu.enable_instruction_trace(8);
        cpu.regs[PC_INDEX] = 0;

        mem.write_u32(0, 0xE3A00001); // mov r0,#1
        mem.write_u32(4, 0xEF000011); // swi

        cpu.step(&mut mem).expect("mov executes");
        cpu.step(&mut mem).expect("swi executes");

        assert_eq!(cpu.instruction_trace().len(), 2);
        assert_eq!(cpu.instruction_trace()[0].pc, 0);
        assert!(matches!(
            cpu.last_exception().expect("swi exception").kind,
            ExceptionKind::SoftwareInterrupt
        ));
    }
}
