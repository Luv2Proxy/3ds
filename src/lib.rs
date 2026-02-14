#![forbid(unsafe_code)]

mod core;

pub use crate::core::cpu::{CpuException, CpuRunState, ExceptionKind};
pub use crate::core::emulator::{Emulator3ds, EmulatorConfig, EmulatorState};
pub use crate::core::error::EmulatorError;
pub use crate::core::kernel::{ServiceCall, ServiceEvent};
pub use crate::core::pica::GpuCommand;
pub use crate::core::timing::TimingSnapshot;

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

    pub fn enqueue_gpu_draw_point(&mut self, x: u16, y: u16, color: u32) {
        self.inner
            .enqueue_gpu_command(GpuCommand::DrawPoint { x, y, color });
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

    pub fn reset(&mut self) {
        self.inner.reset();
    }
}
