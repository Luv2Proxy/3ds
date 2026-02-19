use super::cpu::{Arm11Cpu, CpuException, CpuRunState, ExceptionKind};
use super::diagnostics::{
    BootCheckpoint, BootCheckpointProfiler, BootCheckpointSnapshot, FaultSnapshot, RingBuffer,
    StructuredError, TraceCategory, TracePayload, TraceRecord,
};
use super::dma::{DmaEngine, DmaTransfer, DmaTransferKind};
use super::dsp::Dsp;
use super::error::{EmulatorError, Result};
use super::fs::TitlePackage;
use super::fs::VirtualFileSystem;
use super::irq::{IrqController, IrqLine};
use super::kernel::{Kernel, ServiceEvent};
use super::loader::{install_process_image, parse_process_image_from_rom};
use super::memory::Memory;
use super::pica::PicaGpu;
use super::scheduler::{ScheduledDeviceEvent, Scheduler};
use super::timing::{DriftCorrectionPolicy, TimingModel, TimingSnapshot};

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
    irq: IrqController,
    dma: DmaEngine,
    kernel: Kernel,
    timing: TimingModel,
    rom_loaded: bool,
    vfs: VirtualFileSystem,
    config: EmulatorConfig,
    frame_callbacks: u64,
    audio_callbacks: u64,
    cpu_trace: RingBuffer<TraceRecord>,
    ipc_trace: RingBuffer<TraceRecord>,
    service_trace: RingBuffer<TraceRecord>,
    mmu_fault_trace: RingBuffer<TraceRecord>,
    gpu_trace: RingBuffer<TraceRecord>,
    fault_snapshots: RingBuffer<FaultSnapshot>,
    boot_profiler: BootCheckpointProfiler,
    last_gpu_trace_len: usize,
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
            irq: IrqController::new(),
            dma: DmaEngine::new(),
            kernel: Kernel::new(),
            timing: TimingModel::new(),
            rom_loaded: false,
            vfs: VirtualFileSystem::default(),
            config,
            frame_callbacks: 0,
            audio_callbacks: 0,
            cpu_trace: RingBuffer::new(512),
            ipc_trace: RingBuffer::new(256),
            service_trace: RingBuffer::new(256),
            mmu_fault_trace: RingBuffer::new(128),
            gpu_trace: RingBuffer::new(512),
            fault_snapshots: RingBuffer::new(128),
            boot_profiler: BootCheckpointProfiler::new(),
            last_gpu_trace_len: 0,
        }
    }

    pub fn reset(&mut self) {
        self.memory.clear_writable();
        self.scheduler.reset();
        self.irq.reset();
        self.dma.reset();
        self.timing.reset();
        self.kernel.reset_runtime();
        self.cpu.reset(0);
        self.rom_loaded = false;
        self.vfs = VirtualFileSystem::default();
        self.frame_callbacks = 0;
        self.audio_callbacks = 0;
        self.clear_diagnostics();
    }

    pub fn load_rom(&mut self, rom: &[u8]) -> Result<()> {
        self.memory.clear_writable();
        let loaded = parse_process_image_from_rom(rom)?;
        install_process_image(&mut self.memory, &loaded.process)?;
        self.vfs = loaded.vfs;
        self.cpu.reset(loaded.process.entrypoint);
        self.scheduler.reset();
        self.irq.reset();
        self.dma.reset();
        self.timing.reset();
        self.frame_callbacks = 0;
        self.audio_callbacks = 0;
        self.clear_diagnostics();
        self.rom_loaded = true;
        self.schedule_boot_events();
        self.boot_profiler
            .mark(BootCheckpoint::RomLoaded, self.scheduler.cycles());
        Ok(())
    }

    pub fn load_title_package(&mut self, package: &[u8]) -> Result<()> {
        let title = TitlePackage::parse(package)?;
        self.load_rom(title.primary_rom())
    }

    pub fn enqueue_gpu_fifo_words(&mut self, words: &[u32]) {
        self.gpu.enqueue_gsp_fifo_words(words);
    }

    pub fn queue_dma_memcpy(&mut self, channel: u8, source: u32, destination: u32, words: u32) {
        let latency = self.dma.queue_transfer(DmaTransfer {
            channel,
            source,
            destination,
            words,
            kind: DmaTransferKind::MemoryToMemory,
        });
        self.scheduler
            .schedule_in(latency, ScheduledDeviceEvent::DmaCompletion { channel });
    }

    pub fn queue_dma_gpu_feed(&mut self, channel: u8, source: u32, words: u32) {
        let latency = self.dma.queue_transfer(DmaTransfer {
            channel,
            source,
            destination: 0,
            words,
            kind: DmaTransferKind::GpuQueueFeed,
        });
        self.scheduler
            .schedule_in(latency, ScheduledDeviceEvent::DmaCompletion { channel });
    }

    fn clear_diagnostics(&mut self) {
        self.cpu_trace.clear();
        self.ipc_trace.clear();
        self.service_trace.clear();
        self.mmu_fault_trace.clear();
        self.gpu_trace.clear();
        self.fault_snapshots.clear();
        self.boot_profiler.reset();
        self.last_gpu_trace_len = 0;
    }

    fn record_trace(&mut self, category: TraceCategory, payload: TracePayload) {
        let cycle = self.scheduler.cycles();
        let rec = TraceRecord { cycle, payload };
        match category {
            TraceCategory::CpuFetchDecode => self.cpu_trace.push(rec),
            TraceCategory::Ipc => self.ipc_trace.push(rec),
            TraceCategory::ServiceCall => self.service_trace.push(rec),
            TraceCategory::MmuFault => self.mmu_fault_trace.push(rec),
            TraceCategory::GpuCommand => self.gpu_trace.push(rec),
        }
    }

    fn record_fault(&mut self, error: StructuredError) {
        self.fault_snapshots.push(FaultSnapshot {
            cycle: self.scheduler.cycles(),
            error,
        });
    }

    fn schedule_boot_events(&mut self) {
        self.scheduler
            .schedule_in(16, ScheduledDeviceEvent::TimerExpiry);
        self.scheduler
            .schedule_in(4_000_000, ScheduledDeviceEvent::VBlank);
    }

    fn handle_scheduled_event(&mut self, event: ScheduledDeviceEvent) {
        match event {
            ScheduledDeviceEvent::TimerExpiry => {
                self.irq.raise(IrqLine::Timer0);
                self.scheduler
                    .schedule_in(16, ScheduledDeviceEvent::TimerExpiry);
            }
            ScheduledDeviceEvent::VBlank => {
                self.irq.raise(IrqLine::VBlank);
                self.scheduler
                    .schedule_in(4_000_000, ScheduledDeviceEvent::VBlank);
            }
            ScheduledDeviceEvent::DmaCompletion { channel } => {
                if self
                    .dma
                    .complete_transfer(channel, &mut self.memory, &mut self.gpu)
                {
                    self.irq.raise(IrqLine::Dma0);
                }
            }
            ScheduledDeviceEvent::ServiceWake { pid } => {
                self.kernel.on_scheduler_wake(pid);
            }
        }
    }

    pub fn run_cycles(&mut self, budget: u32) -> Result<u32> {
        if !self.rom_loaded {
            return Err(EmulatorError::RomNotLoaded);
        }

        let capped_budget = budget.min(self.config.max_cycle_budget);
        let mut executed = 0;

        for _ in 0..capped_budget {
            if self.cpu.interrupts_enabled()
                && let Some(line) = self.irq.next_pending()
            {
                self.irq.clear(line);
                self.cpu.enter_irq(line);
            }

            let consumed = self.cpu.step(&mut self.memory)?;
            if let Some(entry) = self.cpu.take_last_instruction_trace() {
                self.record_trace(
                    TraceCategory::CpuFetchDecode,
                    TracePayload::CpuFetchDecode {
                        pc: entry.pc,
                        opcode: entry.opcode,
                        thumb: entry.thumb,
                    },
                );
                self.boot_profiler
                    .mark(BootCheckpoint::FirstInstruction, self.scheduler.cycles());
            }
            self.scheduler.tick(consumed);
            self.kernel.tick(consumed);
            let timing_tick = self.timing.tick(consumed);
            self.kernel.pump_ipc_events(1);
            if let Some((command_id, handle_id, result_code)) = self.kernel.take_last_ipc_dispatch()
            {
                self.record_trace(
                    TraceCategory::Ipc,
                    TracePayload::Ipc {
                        command_id,
                        handle_id,
                        result_code,
                    },
                );
                self.boot_profiler
                    .mark(BootCheckpoint::FirstIpcDispatch, self.scheduler.cycles());
                if result_code != 0 {
                    let err = StructuredError::ServiceCallFailure {
                        pc: self.cpu.pc(),
                        service_command_id: command_id,
                        handle_id,
                        result_code,
                    };
                    self.record_fault(err.clone());
                    self.kernel.report_error(err);
                    return Err(EmulatorError::ServiceCallError {
                        pc: self.cpu.pc(),
                        service_command_id: command_id,
                        handle_id,
                        result_code,
                    });
                }
            }
            if let Some(imm24) = self.kernel.take_last_service_imm24() {
                self.record_trace(
                    TraceCategory::ServiceCall,
                    TracePayload::ServiceCall { imm24 },
                );
                self.boot_profiler
                    .mark(BootCheckpoint::FirstServiceCall, self.scheduler.cycles());
            }

            for event in self.scheduler.drain_due_events() {
                self.handle_scheduled_event(event);
            }

            for fifo_words in self.kernel.drain_gpu_handoff() {
                self.gpu.enqueue_gsp_fifo_words(&fifo_words);
            }
            self.gpu.tick(self.scheduler.cycles());
            let new_gpu_writes: Vec<_> = self
                .gpu
                .trace()
                .iter()
                .skip(self.last_gpu_trace_len)
                .map(|w| (w.reg, w.value))
                .collect();
            for (reg, value) in new_gpu_writes {
                self.record_trace(
                    TraceCategory::GpuCommand,
                    TracePayload::GpuCommand { reg, value },
                );
                self.boot_profiler
                    .mark(BootCheckpoint::FirstGpuCommand, self.scheduler.cycles());
            }
            self.last_gpu_trace_len = self.gpu.trace().len();
            if timing_tick.video_frames > 0 {
                self.gpu.present(timing_tick.video_frames);
                self.frame_callbacks = self
                    .frame_callbacks
                    .saturating_add(timing_tick.video_frames);
                self.boot_profiler
                    .mark(BootCheckpoint::FirstFramePresent, self.scheduler.cycles());
            }
            if timing_tick.audio_samples > 0 {
                self.dsp.produce_samples(timing_tick.audio_samples);
                self.audio_callbacks = self
                    .audio_callbacks
                    .saturating_add(timing_tick.audio_samples);
            }
            executed += consumed;

            if let Some(exception) = self.cpu.last_exception()
                && exception.kind == ExceptionKind::SoftwareInterrupt
            {
                self.kernel.handle_swi(exception.fault_opcode & 0x00FF_FFFF);
                for event in self.kernel.take_pending_schedule_events() {
                    self.scheduler.schedule_in(
                        event.delay_cycles,
                        ScheduledDeviceEvent::ServiceWake { pid: event.pid },
                    );
                }
            }

            if let Some(fault) = self.cpu.take_last_mmu_fault() {
                self.record_trace(
                    TraceCategory::MmuFault,
                    TracePayload::MmuFault {
                        va: fault.va,
                        pa: fault.pa,
                        access: fault.access,
                    },
                );
                let err = StructuredError::MmuFault {
                    pc: self.cpu.pc(),
                    va: fault.va,
                    pa: fault.pa,
                    access: fault.access,
                };
                self.record_fault(err);
                return Err(match fault.kind {
                    super::cpu::FaultKind::Translation => EmulatorError::MmuTranslationFault {
                        pc: self.cpu.pc(),
                        va: fault.va,
                        pa: fault.pa,
                        access: fault.access,
                    },
                    super::cpu::FaultKind::Domain => EmulatorError::MmuDomainFault {
                        pc: self.cpu.pc(),
                        va: fault.va,
                        pa: fault.pa,
                        domain: 0,
                        access: fault.access,
                    },
                    super::cpu::FaultKind::Permission => EmulatorError::MmuPermissionFault {
                        pc: self.cpu.pc(),
                        va: fault.va,
                        pa: fault.pa,
                        access: fault.access,
                    },
                    super::cpu::FaultKind::Alignment => EmulatorError::AlignmentFault {
                        pc: self.cpu.pc(),
                        va: fault.va,
                        pa: fault.pa,
                        access: fault.access,
                    },
                });
            }

            if self.cpu.run_state() == CpuRunState::Halted {
                self.boot_profiler
                    .mark(BootCheckpoint::CpuHalted, self.scheduler.cycles());
                break;
            }
        }

        Ok(executed)
    }

    pub fn set_wasm_drift_policy(&mut self, policy: DriftCorrectionPolicy) {
        self.timing.set_drift_policy(policy);
    }

    pub fn set_wasm_wall_time_anchor_us(&mut self, host_wall_time_us: u64) {
        self.timing.set_wall_time_anchor_us(host_wall_time_us);
    }

    pub fn run_cycles_synced(
        &mut self,
        requested_cycles: u32,
        host_wall_time_us: u64,
    ) -> Result<u32> {
        let adjusted = self
            .timing
            .recommended_cycle_budget(host_wall_time_us, requested_cycles)
            .min(self.config.max_cycle_budget);
        self.run_cycles(adjusted)
    }

    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        self.dsp.take_samples()
    }

    pub fn take_frame_present_count(&mut self) -> u64 {
        let count = self.frame_callbacks;
        self.frame_callbacks = 0;
        count
    }

    pub fn take_audio_sample_count(&mut self) -> u64 {
        let count = self.audio_callbacks;
        self.audio_callbacks = 0;
        count
    }

    pub fn read_phys_u8(&self, addr: u32) -> u8 {
        self.memory.read_u8(addr)
    }

    pub fn write_phys_u8(&mut self, addr: u32, value: u8) {
        self.memory.write_u8(addr, value);
    }

    pub fn read_phys_u32(&self, addr: u32) -> u32 {
        self.memory.read_u32(addr)
    }

    pub fn write_phys_u32(&mut self, addr: u32, value: u32) {
        self.memory.write_u32(addr, value);
    }

    pub fn mapped_memory_bytes(&self) -> usize {
        self.memory.len_mapped_bytes()
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

    pub fn recent_trace_slice(&self, category: TraceCategory, limit: usize) -> Vec<TraceRecord> {
        match category {
            TraceCategory::CpuFetchDecode => self.cpu_trace.recent(limit),
            TraceCategory::Ipc => self.ipc_trace.recent(limit),
            TraceCategory::ServiceCall => self.service_trace.recent(limit),
            TraceCategory::MmuFault => self.mmu_fault_trace.recent(limit),
            TraceCategory::GpuCommand => self.gpu_trace.recent(limit),
        }
    }

    pub fn recent_fault_snapshots(&self, limit: usize) -> Vec<FaultSnapshot> {
        self.fault_snapshots.recent(limit)
    }

    pub fn boot_checkpoint_snapshot(&self) -> BootCheckpointSnapshot {
        self.boot_profiler.snapshot()
    }

    pub fn diagnostics_json(&self) -> String {
        let checkpoints = self.boot_checkpoint_snapshot();
        format!(
            r#"{{"cpu_trace":{},"ipc_trace":{},"service_trace":{},"mmu_fault_trace":{},"gpu_trace":{},"fault_snapshots":{},"boot_events":{},"boot_divergence_at":{}}}"#,
            self.recent_trace_slice(TraceCategory::CpuFetchDecode, 32)
                .len(),
            self.recent_trace_slice(TraceCategory::Ipc, 32).len(),
            self.recent_trace_slice(TraceCategory::ServiceCall, 32)
                .len(),
            self.recent_trace_slice(TraceCategory::MmuFault, 32).len(),
            self.recent_trace_slice(TraceCategory::GpuCommand, 32).len(),
            self.recent_fault_snapshots(16).len(),
            checkpoints.events.len(),
            checkpoints
                .divergence_at
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".to_string())
        )
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
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
            exc,
        )
    }

    pub fn memory_checksum(&self, start: usize, len: usize) -> Option<u64> {
        let mut checksum = 0_u64;
        for off in 0..len {
            let addr = (start as u32).checked_add(off as u32)?;
            checksum = checksum.wrapping_add(u64::from(self.memory.read_u8(addr)));
        }
        Some(checksum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pica::PicaCommandBufferPacket;

    fn valid_rom() -> Vec<u8> {
        let mut rom = vec![0_u8; 0x5000];
        rom[0x100..0x104].copy_from_slice(b"NCSD");
        rom[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        rom[0x124..0x128].copy_from_slice(&0x20u32.to_le_bytes());

        let ncch = 0x200;
        rom[ncch + 0x100..ncch + 0x104].copy_from_slice(b"NCCH");
        rom[ncch + 0x180..ncch + 0x184].copy_from_slice(&0x400u32.to_le_bytes());
        rom[ncch + 0x1A8..ncch + 0x1AC].copy_from_slice(&3u32.to_le_bytes());
        rom[ncch + 0x1AC..ncch + 0x1B0].copy_from_slice(&2u32.to_le_bytes());

        let ex = ncch + 0x200;
        rom[ex..ex + 4].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        rom[ex + 0x10..ex + 0x14].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        rom[ex + 0x18..ex + 0x1C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x20..ex + 0x24].copy_from_slice(&0x0010_1000u32.to_le_bytes());
        rom[ex + 0x28..ex + 0x2C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x30..ex + 0x34].copy_from_slice(&0x0010_2000u32.to_le_bytes());
        rom[ex + 0x38..ex + 0x3C].copy_from_slice(&0x20u32.to_le_bytes());
        rom[ex + 0x3C..ex + 0x40].copy_from_slice(&0x10u32.to_le_bytes());
        rom[ex + 0x1C..ex + 0x20].copy_from_slice(&0x2000u32.to_le_bytes());
        rom[ex + 0x40..ex + 0x44].copy_from_slice(&0x8000u32.to_le_bytes());

        let exefs = 0x800;
        rom[exefs..exefs + 5].copy_from_slice(b".code");
        rom[exefs + 8..exefs + 12].copy_from_slice(&0u32.to_le_bytes());
        rom[exefs + 12..exefs + 16].copy_from_slice(&0x60u32.to_le_bytes());
        rom[exefs + 0x200..exefs + 0x260].fill(0);

        rom
    }

    fn write_insn(rom: &mut [u8], offset: usize, opcode: u32) {
        rom[offset..offset + 4].copy_from_slice(&opcode.to_le_bytes());
    }

    #[test]
    fn wasm_memory_mapping_handles_high_rom_addresses() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0xA00, 0xE320_F003);
        emu.load_rom(&rom)
            .unwrap_or_else(|e| panic!("load works: {e}"));
        emu.run_cycles(1)
            .unwrap_or_else(|e| panic!("run works: {e}"));
        let state = emu.state();
        assert_eq!(state.pc, 0x0010_0004);
    }

    #[test]
    fn timer_event_triggers_irq_entry() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0xA00, 0xE1A0_0000); // NOP
        write_insn(&mut rom, 0xA18, 0xE320_F003); // HALT in IRQ vector
        emu.load_rom(&rom)
            .unwrap_or_else(|e| panic!("load works: {e}"));

        emu.scheduler
            .schedule_in(1, ScheduledDeviceEvent::TimerExpiry);

        emu.run_cycles(8)
            .unwrap_or_else(|e| panic!("run works: {e}"));

        let state = emu.state();
        let exception = state
            .last_exception
            .unwrap_or_else(|| panic!("expected IRQ exception"));
        assert!(matches!(
            exception.kind,
            ExceptionKind::Interrupt(IrqLine::Timer0)
        ));
        assert_eq!(exception.vector, 0x0010_0018);
        assert_eq!(state.pc, 0x0010_001C);
    }

    #[test]
    fn dma_completion_signals_irq_and_copies_memory() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0xA00, 0xE1A0_0000); // NOP
        write_insn(&mut rom, 0xA18, 0xE320_F003); // HALT in IRQ vector
        emu.load_rom(&rom)
            .unwrap_or_else(|e| panic!("load works: {e}"));

        emu.write_phys_u32(0x0000_0080, 0x1122_3344);
        emu.write_phys_u32(0x0000_0084, 0x5566_7788);
        emu.queue_dma_memcpy(0, 0x0000_0080, 0x0000_0100, 2);

        emu.run_cycles(8)
            .unwrap_or_else(|e| panic!("run works: {e}"));

        assert_eq!(emu.read_phys_u32(0x0000_0100), 0x1122_3344);
        assert_eq!(emu.read_phys_u32(0x0000_0104), 0x5566_7788);
        let exception = emu
            .state()
            .last_exception
            .unwrap_or_else(|| panic!("expected DMA IRQ"));
        assert!(matches!(
            exception.kind,
            ExceptionKind::Interrupt(IrqLine::Dma0)
        ));
    }

    #[test]
    fn gpu_kernel_timing_and_fs_pipeline_work() {
        let mut emu = Emulator3ds::new();
        let mut rom = valid_rom();
        write_insn(&mut rom, 0xA00, 0xEF00_0000);
        write_insn(&mut rom, 0xA04, 0xE320_F003);

        emu.load_title_package(&rom)
            .unwrap_or_else(|e| panic!("load title works: {e}"));
        emu.enqueue_gpu_fifo_words(&[
            PicaCommandBufferPacket::encode(0x0301, 1, false),
            0x0000_00FF,
            PicaCommandBufferPacket::encode(0x0200, 1, false),
            0xFF11_2233,
            PicaCommandBufferPacket::encode(0x0201, 1, false),
            (1 << 16) | 1,
            PicaCommandBufferPacket::encode(0x0202, 1, false),
            0xFF44_5566,
        ]);
        emu.run_cycles(8)
            .unwrap_or_else(|e| panic!("run works: {e}"));

        let timing = emu.timing_snapshot();
        assert!(timing.cpu_cycles > 0);
        assert!(emu.last_service_call().is_some());
        let frame = emu.frame_rgba();
        assert_eq!(frame.len(), PicaGpu::WIDTH * PicaGpu::HEIGHT * 4);
    }
}
