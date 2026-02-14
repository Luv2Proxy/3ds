use super::memory::Memory;
use super::pica::PicaGpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaTransferKind {
    MemoryToMemory,
    GpuQueueFeed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaTransfer {
    pub channel: u8,
    pub source: u32,
    pub destination: u32,
    pub words: u32,
    pub kind: DmaTransferKind,
}

#[derive(Clone, Default)]
pub struct DmaEngine {
    in_flight: Vec<DmaTransfer>,
}

impl DmaEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.in_flight.clear();
    }

    pub fn queue_transfer(&mut self, transfer: DmaTransfer) -> u64 {
        self.in_flight.push(transfer);
        u64::from(transfer.words.max(1))
    }

    pub fn complete_transfer(
        &mut self,
        channel: u8,
        memory: &mut Memory,
        gpu: &mut PicaGpu,
    ) -> bool {
        let Some(idx) = self.in_flight.iter().position(|t| t.channel == channel) else {
            return false;
        };
        let transfer = self.in_flight.remove(idx);
        match transfer.kind {
            DmaTransferKind::MemoryToMemory => {
                for word in 0..transfer.words {
                    let src = transfer.source.wrapping_add(word.wrapping_mul(4));
                    let dst = transfer.destination.wrapping_add(word.wrapping_mul(4));
                    let value = memory.read_u32(src);
                    memory.write_u32(dst, value);
                }
            }
            DmaTransferKind::GpuQueueFeed => {
                let mut words = Vec::with_capacity(transfer.words as usize);
                for word in 0..transfer.words {
                    let src = transfer.source.wrapping_add(word.wrapping_mul(4));
                    words.push(memory.read_u32(src));
                }
                gpu.enqueue_gsp_fifo_words(&words);
            }
        }
        true
    }
}
