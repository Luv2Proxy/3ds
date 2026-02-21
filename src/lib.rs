#![forbid(unsafe_code)]

mod core;

pub use crate::core::cpu::{CpuException, CpuRunState, ExceptionKind};
pub use crate::core::emulator::{Emulator3ds, EmulatorConfig, EmulatorState};
pub use crate::core::error::EmulatorError;
pub use crate::core::kernel::{ServiceCall, ServiceEvent};
pub use crate::core::timing::{DriftCorrectionPolicy, TimingSnapshot};
pub use crate::core::trace::{
    BootCheckpoint, BootCheckpointSnapshot, FaultSnapshot, StructuredError, TraceCategory,
    TracePayload, TraceRecord,
};

#[derive(Default)]
pub struct Wasm3ds {
    inner: Emulator3ds,
}

impl Wasm3ds {
    pub fn new() -> Self {
        Self {
            inner: Emulator3ds::new(),
        }
    }

    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), String> {
        self.inner.load_rom(rom).map_err(|e| e.to_string())
    }

    pub fn load_title_package(&mut self, package: &[u8]) -> Result<(), String> {
        self.inner
            .load_title_package(package)
            .map_err(|e| e.to_string())
    }

    pub fn run_cycles(&mut self, cycles: u32) -> Result<u32, String> {
        self.inner.run_cycles(cycles).map_err(|e| e.to_string())
    }

    pub fn set_drift_policy(&mut self, max_frame_lead_us: i64, max_frame_lag_us: i64) {
        self.inner.set_wasm_drift_policy(DriftCorrectionPolicy {
            max_frame_lead_us,
            max_frame_lag_us,
        });
    }

    pub fn set_wall_time_anchor_us(&mut self, host_wall_time_us: u64) {
        self.inner.set_wasm_wall_time_anchor_us(host_wall_time_us);
    }

    pub fn run_cycles_synced(
        &mut self,
        requested_cycles: u32,
        host_wall_time_us: u64,
    ) -> Result<u32, String> {
        self.inner
            .run_cycles_synced(requested_cycles, host_wall_time_us)
            .map_err(|e| e.to_string())
    }

    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        self.inner.take_audio_samples()
    }

    pub fn take_frame_present_count(&mut self) -> u64 {
        self.inner.take_frame_present_count()
    }

    pub fn take_audio_sample_count(&mut self) -> u64 {
        self.inner.take_audio_sample_count()
    }

    pub fn enqueue_gpu_fifo_words(&mut self, words: &[u32]) {
        self.inner.enqueue_gpu_fifo_words(words);
    }

    pub fn read_phys_u8(&self, addr: u32) -> u8 {
        self.inner.read_phys_u8(addr)
    }

    pub fn write_phys_u8(&mut self, addr: u32, value: u8) {
        self.inner.write_phys_u8(addr, value);
    }

    pub fn read_phys_u32(&self, addr: u32) -> u32 {
        self.inner.read_phys_u32(addr)
    }

    pub fn write_phys_u32(&mut self, addr: u32, value: u32) {
        self.inner.write_phys_u32(addr, value);
    }

    pub fn mapped_memory_bytes(&self) -> usize {
        self.inner.mapped_memory_bytes()
    }

    pub fn frame_rgba(&self) -> Vec<u8> {
        self.inner.frame_rgba()
    }

    pub fn state_json(&self) -> String {
        self.inner.state_json()
    }

    pub fn diagnostics_json(&self) -> String {
        self.inner.diagnostics_json()
    }

    pub fn recent_fault_snapshots(&self, limit: usize) -> Vec<FaultSnapshot> {
        self.inner.recent_fault_snapshots(limit)
    }

    pub fn boot_checkpoint_snapshot(&self) -> BootCheckpointSnapshot {
        self.inner.boot_checkpoint_snapshot()
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }
}
