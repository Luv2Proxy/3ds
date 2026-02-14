use std::collections::{HashMap, VecDeque};

use super::error::MemoryAccessKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceCategory {
    CpuFetchDecode,
    Ipc,
    ServiceCall,
    MmuFault,
    GpuCommand,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceRecord {
    pub cycle: u64,
    pub payload: TracePayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredError {
    CpuInvalidInstruction {
        pc: u32,
        opcode: u32,
    },
    MmuFault {
        pc: u32,
        va: u32,
        pa: Option<u32>,
        access: MemoryAccessKind,
    },
    ServiceCallFailure {
        pc: u32,
        service_command_id: u16,
        handle_id: u32,
        result_code: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultSnapshot {
    pub cycle: u64,
    pub error: StructuredError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BootCheckpoint {
    RomLoaded,
    FirstInstruction,
    FirstServiceCall,
    FirstIpcDispatch,
    FirstGpuCommand,
    FirstFramePresent,
    CpuHalted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootCheckpointEvent {
    pub checkpoint: BootCheckpoint,
    pub cycle: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BootCheckpointSnapshot {
    pub events: Vec<BootCheckpointEvent>,
    pub divergence_at: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    cap: usize,
    values: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            values: VecDeque::with_capacity(cap),
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn push(&mut self, value: T) {
        if self.cap == 0 {
            return;
        }
        if self.values.len() == self.cap {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    pub fn recent(&self, limit: usize) -> Vec<T>
    where
        T: Clone,
    {
        let keep = limit.min(self.values.len());
        self.values
            .iter()
            .skip(self.values.len().saturating_sub(keep))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct BootCheckpointProfiler {
    expected: Vec<BootCheckpoint>,
    observed: Vec<BootCheckpointEvent>,
    seen: HashMap<BootCheckpoint, usize>,
    divergence_at: Option<usize>,
}

impl Default for BootCheckpointProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl BootCheckpointProfiler {
    pub fn new() -> Self {
        Self {
            expected: vec![
                BootCheckpoint::RomLoaded,
                BootCheckpoint::FirstInstruction,
                BootCheckpoint::FirstServiceCall,
                BootCheckpoint::FirstIpcDispatch,
                BootCheckpoint::FirstGpuCommand,
                BootCheckpoint::FirstFramePresent,
            ],
            observed: Vec::new(),
            seen: HashMap::new(),
            divergence_at: None,
        }
    }

    pub fn reset(&mut self) {
        self.observed.clear();
        self.seen.clear();
        self.divergence_at = None;
    }

    pub fn mark(&mut self, checkpoint: BootCheckpoint, cycle: u64) {
        if self.seen.contains_key(&checkpoint) {
            return;
        }
        let idx = self.observed.len();
        self.observed
            .push(BootCheckpointEvent { checkpoint, cycle });
        self.seen.insert(checkpoint, idx);
        if self.divergence_at.is_none() {
            if let Some(expected) = self.expected.get(idx) {
                if *expected != checkpoint {
                    self.divergence_at = Some(idx);
                }
            } else {
                self.divergence_at = Some(idx);
            }
        }
    }

    pub fn snapshot(&self) -> BootCheckpointSnapshot {
        BootCheckpointSnapshot {
            events: self.observed.clone(),
            divergence_at: self.divergence_at,
        }
    }
}
