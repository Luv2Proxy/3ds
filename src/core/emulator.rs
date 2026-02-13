use super::cpu::{Arm11Cpu, CpuException, CpuRunState, ExceptionKind};
use super::dsp::Dsp;
use super::error::{EmulatorError, Result};
use super::fs::TitlePackage;
use super::kernel::{Kernel, ServiceEvent};
use super::memory::Memory;
use super::pica::{GpuCommand, PicaGpu};
use super::rom::RomImage;
use super::scheduler::Scheduler;
use super::timing::{TimingModel, TimingSnapshot};

const ENTRYPOINT_OFFSET: usize = 0x200;
const ROM_LOAD_ADDR: usize = 0x0010_0000;

#[derive(Debug, Clone, Copy)]
pub struct EmulatorConfig {
    pub max_cycle_budget: u32,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            max_cycle_budget: 5_000_000,
        }
    }
}

#[derive(Clone)]
pub struct Emulator3ds {
    cpu: Arm11Cpu,
    memory: Memory,
    gpu: PicaGpu,
    dsp: Dsp,
    scheduler: Scheduler,
    kernel: Kernel,
    timing: TimingModel,
    rom_loaded: bool,
    config: EmulatorConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatorState {
    pub pc: u32,
    pub cpsr: u32,
    pub cycles: u64,
    pub cpu_state: CpuRunState,
    pub registers: [u32; 16],
    pub audio_samples: usize,
    pub last_exception: Option<CpuException>,
    pub service_calls: usize,
}

impl Default for Emulator3ds {
    fn default() -> Self {
        Self::new()
    }
}

impl Emulator3ds {
    pub fn new() -> Self {
        Self::with_config(EmulatorConfig::default())
    }

    pub fn with_config(config: EmulatorConfig) -> Self {
        Self {
            cpu: Arm11Cpu::new(),
            memory: Memory::new(),
            gpu: PicaGpu::new(),
            dsp: Dsp::new(),
            scheduler: Scheduler::new(),
            kernel: Kernel::new(),
            timing: TimingModel::new(),
            rom_loaded: false,
            config,
        }
    }

    pub fn reset(&mut self) {
        self.memory.clear();
        self.scheduler.reset();
        self.timing.reset();
        self.cpu.reset(0);
        self.rom_loaded = false;
    }

    pub fn load_rom(&mut self, rom: &[u8]) -> Result<()> {
        let parsed = RomImage::parse(rom, self.memory.len() - ROM_LOAD_ADDR)?;
        self.memory.clear();
        self.memory.load(ROM_LOAD_ADDR, parsed.bytes())?;
        self.cpu.reset((ROM_LOAD_ADDR + ENTRYPOINT_OFFSET) as u32);
        self.scheduler.reset();
        self.timing.reset();
        self.rom_loaded = true;
        Ok(())
    }

    pub fn load_title_package(&mut self, package: &[u8]) -> Result<()> {
        let title = TitlePackage::parse(package)?;
        let first = title
            .contents()
            .first()
            .ok_or(EmulatorError::InvalidTitlePackage)?;
        let rom = title
            .content_bytes(first.id)
            .ok_or(EmulatorError::InvalidTitlePackage)?;
        self.load_rom(rom)
    }

    pub fn enqueue_gpu_command(&mut self, cmd: GpuCommand) {
        self.gpu.enqueue_command(cmd);
    }

    pub fn set_gpu_shader_constant(&mut self, color: u32) {
        self.gpu.set_shader_constant(color);
    }

    pub fn run_cycles(&mut self, budget: u32) -> Result<u32> {
        if !self.rom_loaded {
            return Err(EmulatorError::RomNotLoaded);
        }

        let capped_budget = budget.min(self.config.max_cycle_budget);
        let mut executed = 0;

        for _ in 0..capped_budget {
            let consumed = self.cpu.step(&mut self.memory)?;
            self.scheduler.tick(consumed);
            self.kernel.tick(consumed);
            self.timing.tick(consumed);
            self.gpu.tick(self.scheduler.cycles());
            self.dsp.tick(self.scheduler.cycles());
            executed += consumed;

            if let Some(exception) = self.cpu.last_exception()
                && exception.kind == ExceptionKind::SoftwareInterrupt
            {
                self.kernel.handle_swi(exception.fault_opcode & 0x00FF_FFFF);
            }

            if self.cpu.run_state() == CpuRunState::Halted {
                break;
            }
        }

        Ok(executed)
    }

    pub fn frame_rgba(&self) -> Vec<u8> {
        self.gpu.frame_u8()
    }

    pub fn timing_snapshot(&self) -> TimingSnapshot {
        self.timing.snapshot()
    }

    pub fn last_service_call(&self) -> Option<ServiceEvent> {
        self.kernel.last_service_call()
    }

    pub fn state(&self) -> EmulatorState {
        EmulatorState {
            pc: self.cpu.pc(),
            cpsr: self.cpu.cpsr(),
            cycles: self.scheduler.cycles(),
            cpu_state: self.cpu.run_state(),
            registers: *self.cpu.regs(),
            audio_samples: self.dsp.samples().len(),
            last_exception: self.cpu.last_exception(),
            service_calls: self.kernel.service_call_count(),
        }
    }

    pub fn state_json(&self) -> String {
        let s = self.state();
        let exc = if let Some(exception) = s.last_exception {
            format!(
                "{{\"kind\":\"{:?}\",\"vector\":{},\"return_address\":{},\"fault_opcode\":{}}}",
                exception.kind, exception.vector, exception.return_address, exception.fault_opcode
            )
        } else {
            "null".to_string()
        };

        format!(
            "{{\"pc\":{},\"cpsr\":{},\"cycles\":{},\"cpu_state\":\"{:?}\",\"audio_samples\":{},\"service_calls\":{},\"registers\":[{}],\"last_exception\":{}}}",
            s.pc,
            s.cpsr,
            s.cycles,
            s.cpu_state,
            s.audio_samples,
            s.service_calls,
            s.registers
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(","),
            exc,
        )
    }

    pub fn memory_checksum(&self, start: usize, len: usize) -> Option<u64> {
        let end = start.checked_add(len)?;
        let bytes = self.memory.as_slice().get(start..end)?;
        Some(
            bytes
                .iter()
                .fold(0_u64, |acc, b| acc.wrapping_add(u64::from(*b))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_rom() -> Vec<u8> {
        let mut rom = vec![0_u8; 0x1000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom
    }

    fn write_insn(rom: &mut [u8], offset: usize, opcode: u32) {
        rom[offset..offset + 4].copy_from_slice(&opcode.to_le_bytes());
    }

    #[test]
    fn executes_data_processing_and_mul() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0x200, 0xE3A0_0005); // MOV r0,#5
        write_insn(&mut rom, 0x204, 0xE3A0_1003); // MOV r1,#3
        write_insn(&mut rom, 0x208, 0xE280_2007); // ADD r2,r0,#7
        write_insn(&mut rom, 0x20C, 0xE242_3002); // SUB r3,r2,#2
        write_insn(&mut rom, 0x210, 0xE320_F003); // WFI
        emu.load_rom(&rom)
            .unwrap_or_else(|e| panic!("load works: {e}"));
        emu.run_cycles(16)
            .unwrap_or_else(|e| panic!("run works: {e}"));
        let state = emu.state();
        assert_eq!(state.registers[2], 12);
        assert_eq!(state.registers[3], 10);
    }

    #[test]
    fn exception_return_restores_mode() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0x200, 0xEF00_0001); // SWI
        write_insn(&mut rom, 0x008, 0xE1B0_F00E); // MOVS pc, lr
        write_insn(&mut rom, 0x204, 0xE320_F003); // WFI after return
        emu.load_rom(&rom)
            .unwrap_or_else(|e| panic!("load works: {e}"));
        emu.run_cycles(8)
            .unwrap_or_else(|e| panic!("run works: {e}"));
        let state = emu.state();
        assert_eq!(state.cpsr & 0x1F, 0b1_0000);
        assert!(state.service_calls >= 1);
    }

    #[test]
    fn gpu_kernel_timing_and_fs_pipeline_work() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0x200, 0xEF00_0000);
        write_insn(&mut rom, 0x204, 0xE320_F003);

        let mut pkg = vec![];
        pkg.extend_from_slice(b"3DST");
        pkg.extend_from_slice(&1u32.to_le_bytes());
        let offset = 20u32;
        pkg.extend_from_slice(&7u32.to_le_bytes());
        pkg.extend_from_slice(&offset.to_le_bytes());
        pkg.extend_from_slice(&(rom.len() as u32).to_le_bytes());
        pkg.resize(offset as usize, 0);
        pkg.extend_from_slice(&rom);

        emu.load_title_package(&pkg)
            .unwrap_or_else(|e| panic!("load title works: {e}"));
        emu.set_gpu_shader_constant(0x0000_00FF);
        emu.enqueue_gpu_command(GpuCommand::Clear(0xFF11_2233));
        emu.enqueue_gpu_command(GpuCommand::DrawPoint {
            x: 1,
            y: 1,
            color: 0xFF44_5566,
        });
        emu.run_cycles(8)
            .unwrap_or_else(|e| panic!("run works: {e}"));

        let timing = emu.timing_snapshot();
        assert!(timing.cpu_cycles > 0);
        assert!(emu.last_service_call().is_some());
        let frame = emu.frame_rgba();
        assert_eq!(frame.len(), PicaGpu::WIDTH * PicaGpu::HEIGHT * 4);
    }
}
