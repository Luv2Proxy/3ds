pub use super::diagnostics::{
    BootCheckpoint, BootCheckpointProfiler, BootCheckpointSnapshot, FaultSnapshot, RingBuffer,
    StructuredError,
};

use super::error::MemoryAccessKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceCategory {
    CpuFetchDecode,
    Ipc,
    ServiceCall,
    MmuFault,
    GpuCommand,
    Irq,
    Timer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TracePayload {
    CpuFetchDecode {
        pc: u32,
        opcode: u32,
        thumb: bool,
    },
    Ipc {
        command_id: u16,
        handle_id: u32,
        result_code: u32,
    },
    ServiceCall {
        imm24: u32,
    },
    MmuFault {
        va: u32,
        pa: Option<u32>,
        access: MemoryAccessKind,
    },
    GpuCommand {
        reg: u16,
        value: u32,
    },
    IrqRaised {
        line: u8,
    },
    TimerScheduled {
        period_cycles: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRecord {
    pub cycle: u64,
    pub payload: TracePayload,
}
