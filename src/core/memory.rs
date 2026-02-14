use super::error::{EmulatorError, Result};

/// 3DS physical memory map segments represented in WASM-safe host buffers.
///
/// Instead of trying to index a single `Vec<u8>` by raw 32-bit physical address
/// (which fails for sparse/high addresses in WebAssembly), we maintain a list
/// of mapped ranges and translate addresses to segment-local offsets.
///
/// This keeps allocations proportional to actual mapped regions rather than
/// the full 4 GiB physical address space.
pub const FCRAM_START: u32 = 0x0000_0000;
pub const FCRAM_SIZE: usize = 128 * 1024 * 1024;

pub const VRAM_START: u32 = 0x1F00_0000;
pub const VRAM_SIZE: usize = 1024 * 1024;

pub const IO_START: u32 = 0x1010_0000;
pub const IO_SIZE: usize = 1024 * 1024;

pub const BIOS_START: u32 = 0x1FFF_0000;
pub const BIOS_SIZE: usize = 64 * 1024;

pub const ROM_START: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentKind {
    Fcram,
    Vram,
    Io,
    Bios,
    Rom,
}

#[derive(Clone)]
struct Segment {
    kind: SegmentKind,
    start: u32,
    end_exclusive: u32,
    writable: bool,
    data: Vec<u8>,
}

impl Segment {
    fn contains(&self, addr: u32) -> bool {
        self.start <= addr && addr < self.end_exclusive
    }

    fn offset_of(&self, addr: u32) -> usize {
        (addr - self.start) as usize
    }
}

#[derive(Clone)]
pub struct Memory {
    segments: Vec<Segment>,
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    pub fn new() -> Self {
        let mut memory = Self { segments: vec![] };
        memory.map_fixed_segment(SegmentKind::Fcram, FCRAM_START, FCRAM_SIZE, true);
        memory.map_fixed_segment(SegmentKind::Vram, VRAM_START, VRAM_SIZE, true);
        memory.map_fixed_segment(SegmentKind::Io, IO_START, IO_SIZE, true);
        memory.map_fixed_segment(SegmentKind::Bios, BIOS_START, BIOS_SIZE, false);
        memory
    }

    fn map_fixed_segment(&mut self, kind: SegmentKind, start: u32, size: usize, writable: bool) {
        let end_exclusive = start.saturating_add(size as u32);
        self.segments.push(Segment {
            kind,
            start,
            end_exclusive,
            writable,
            data: vec![0; size],
        });
        self.segments.sort_by_key(|s| s.start);
    }

    pub fn map_rom(&mut self, rom_bytes: &[u8]) {
        self.segments.retain(|s| s.kind != SegmentKind::Rom);
        let end_exclusive = ROM_START.saturating_add(rom_bytes.len() as u32);
        self.segments.push(Segment {
            kind: SegmentKind::Rom,
            start: ROM_START,
            end_exclusive,
            writable: false,
            data: rom_bytes.to_vec(),
        });
        self.segments.sort_by_key(|s| s.start);
    }

    fn find_segment_index(&self, addr: u32) -> Option<usize> {
        self.segments
            .iter()
            .position(|segment| segment.contains(addr))
    }

    fn read_u8_checked(&self, addr: u32) -> Result<u8> {
        let idx = self
            .find_segment_index(addr)
            .ok_or(EmulatorError::MemoryOutOfBounds { address: addr })?;
        let segment = &self.segments[idx];
        let offset = segment.offset_of(addr);
        segment
            .data
            .get(offset)
            .copied()
            .ok_or(EmulatorError::MemoryOutOfBounds { address: addr })
    }

    fn write_u8_checked(&mut self, addr: u32, value: u8) -> Result<()> {
        let idx = self
            .find_segment_index(addr)
            .ok_or(EmulatorError::MemoryOutOfBounds { address: addr })?;
        let segment = &mut self.segments[idx];
        if !segment.writable {
            return Ok(());
        }
        let offset = segment.offset_of(addr);
        let slot = segment
            .data
            .get_mut(offset)
            .ok_or(EmulatorError::MemoryOutOfBounds { address: addr })?;
        *slot = value;
        Ok(())
    }

    /// Read one byte via address-mapper translation.
    ///
    /// Returns `0` for unmapped addresses, which is useful for tolerant host
    /// probing in WASM environments.
    pub fn read_u8(&self, addr: u32) -> u8 {
        self.read_u8_checked(addr).unwrap_or(0)
    }

    /// Write one byte via address-mapper translation.
    ///
    /// Writes to unmapped/read-only regions are ignored to preserve safety.
    pub fn write_u8(&mut self, addr: u32, value: u8) {
        let _ = self.write_u8_checked(addr, value);
    }

    /// Read a little-endian 32-bit value through mapped segments.
    pub fn read_u32_checked(&self, addr: u32) -> Result<u32> {
        let b0 = self.read_u8_checked(addr)?;
        let b1 = self.read_u8_checked(addr.wrapping_add(1))?;
        let b2 = self.read_u8_checked(addr.wrapping_add(2))?;
        let b3 = self.read_u8_checked(addr.wrapping_add(3))?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    /// Write a little-endian 32-bit value through mapped segments.
    pub fn write_u32_checked(&mut self, addr: u32, value: u32) -> Result<()> {
        let bytes = value.to_le_bytes();
        self.write_u8_checked(addr, bytes[0])?;
        self.write_u8_checked(addr.wrapping_add(1), bytes[1])?;
        self.write_u8_checked(addr.wrapping_add(2), bytes[2])?;
        self.write_u8_checked(addr.wrapping_add(3), bytes[3])?;
        Ok(())
    }

    /// Read a little-endian 32-bit value through mapped segments.
    /// Returns `0` for unmapped addresses.
    pub fn read_u32(&self, addr: u32) -> u32 {
        self.read_u32_checked(addr).unwrap_or(0)
    }

    /// Write a little-endian 32-bit value through mapped segments.
    /// Writes to unmapped/read-only regions are ignored.
    pub fn write_u32(&mut self, addr: u32, value: u32) {
        let _ = self.write_u32_checked(addr, value);
    }

    pub fn clear_writable(&mut self) {
        for segment in &mut self.segments {
            if segment.writable {
                segment.data.fill(0);
            }
        }
    }

    pub fn len_mapped_bytes(&self) -> usize {
        self.segments.iter().map(|s| s.data.len()).sum()
    }
}
