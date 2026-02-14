use std::collections::{HashMap, VecDeque};

const REG_VIEWPORT_XY: u16 = 0x0041;
const REG_VIEWPORT_WH: u16 = 0x0042;
const REG_SCISSOR_XY: u16 = 0x0043;
const REG_SCISSOR_WH: u16 = 0x0044;
const REG_ATTRIB_BASE: u16 = 0x0100;
const REG_INDEX_FORMAT: u16 = 0x0110;
const REG_DEPTH_STENCIL: u16 = 0x0120;
const REG_BLEND_EQ: u16 = 0x0130;
const REG_COLOR_CLEAR: u16 = 0x0200;
const REG_DRAW_POINT_XY: u16 = 0x0201;
const REG_DRAW_POINT_COLOR: u16 = 0x0202;
const REG_SHADER_CODE: u16 = 0x0300;
const REG_SHADER_CONST: u16 = 0x0301;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PicaRegisterWrite {
    pub reg: u16,
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PicaCommandBufferPacket {
    pub reg: u16,
    pub count: u8,
    pub sequential: bool,
}

impl PicaCommandBufferPacket {
    pub fn decode(word: u32) -> Self {
        Self {
            reg: (word & 0xFFFF) as u16,
            count: ((word >> 16) & 0x7F) as u8,
            sequential: ((word >> 23) & 1) != 0,
        }
    }

    pub fn encode(reg: u16, count: u8, sequential: bool) -> u32 {
        u32::from(reg) | (u32::from(count) << 16) | ((sequential as u32) << 23)
    }
}

#[derive(Debug, Default, Clone)]
pub struct PicaCommandProcessor;

impl PicaCommandProcessor {
    pub fn decode_command_words(&self, words: &[u32]) -> Vec<PicaRegisterWrite> {
        let mut out = Vec::new();
        let mut cursor = 0usize;
        while cursor < words.len() {
            let packet = PicaCommandBufferPacket::decode(words[cursor]);
            cursor += 1;
            let payload_words = usize::from(packet.count.max(1));
            for i in 0..payload_words {
                if cursor >= words.len() {
                    return out;
                }
                let reg = if packet.sequential {
                    packet.reg.wrapping_add(i as u16)
                } else {
                    packet.reg
                };
                out.push(PicaRegisterWrite {
                    reg,
                    value: words[cursor],
                });
                cursor += 1;
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFormat {
    U8,
    U16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 400,
            h: 240,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScissorRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Default for ScissorRect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 400,
            h: 240,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthStencilState {
    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub stencil_enable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendState {
    pub equation_rgb: u8,
    pub equation_alpha: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTraceEntry {
    pub reg: u16,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PicaStateSnapshot {
    pub attribute_mask: u32,
    pub index_format: IndexFormat,
    pub viewport: Viewport,
    pub scissor: ScissorRect,
    pub depth_stencil: DepthStencilState,
    pub blend: BlendState,
    pub shader_program_hash: u64,
    pub trace_len: usize,
}

#[derive(Clone)]
pub struct PicaGpu {
    frame_buffer: Vec<u32>,
    register_file: HashMap<u16, u32>,
    fifo: VecDeque<u32>,
    cmd: PicaCommandProcessor,
    attribute_mask: u32,
    index_format: IndexFormat,
    viewport: Viewport,
    scissor: ScissorRect,
    depth_stencil: DepthStencilState,
    blend: BlendState,
    shader_microcode: Vec<u32>,
    shader_constant: u32,
    shader_cache: HashMap<u64, String>,
    trace: Vec<GpuTraceEntry>,
    staged_point_xy: u32,
    staged_point_color: u32,
    presents: u64,
}

impl Default for PicaGpu {
    fn default() -> Self {
        Self::new()
    }
}

impl PicaGpu {
    pub const WIDTH: usize = 400;
    pub const HEIGHT: usize = 240;

    pub fn new() -> Self {
        Self {
            frame_buffer: vec![0xFF00_0000; Self::WIDTH * Self::HEIGHT],
            register_file: HashMap::new(),
            fifo: VecDeque::new(),
            cmd: PicaCommandProcessor,
            attribute_mask: 0,
            index_format: IndexFormat::U16,
            viewport: Viewport::default(),
            scissor: ScissorRect::default(),
            depth_stencil: DepthStencilState {
                depth_test_enable: false,
                depth_write_enable: false,
                stencil_enable: false,
            },
            blend: BlendState {
                equation_rgb: 0,
                equation_alpha: 0,
            },
            shader_microcode: Vec::new(),
            shader_constant: 0,
            shader_cache: HashMap::new(),
            trace: Vec::new(),
            staged_point_xy: 0,
            staged_point_color: 0xFFFF_FFFF,
            presents: 0,
        }
    }

    pub fn enqueue_gsp_fifo_words(&mut self, words: &[u32]) {
        self.fifo.extend(words.iter().copied());
    }

    pub fn enqueue_register_write(&mut self, reg: u16, value: u32) {
        let header = PicaCommandBufferPacket::encode(reg, 1, false);
        self.enqueue_gsp_fifo_words(&[header, value]);
    }

    pub fn tick(&mut self, _cycle: u64) {
        if self.fifo.is_empty() {
            return;
        }

        let words: Vec<u32> = self.fifo.drain(..).collect();
        for write in self.cmd.decode_command_words(&words) {
            self.apply_register_write(write);
        }
    }

    pub fn present(&mut self, frame_count: u64) {
        self.presents = self.presents.saturating_add(frame_count);
    }

    pub fn presents(&self) -> u64 {
        self.presents
    }

    fn apply_register_write(&mut self, write: PicaRegisterWrite) {
        self.register_file.insert(write.reg, write.value);
        self.trace.push(GpuTraceEntry {
            reg: write.reg,
            value: write.value,
        });
        match write.reg {
            REG_VIEWPORT_XY => {
                self.viewport.x = (write.value & 0xFFFF) as u16;
                self.viewport.y = ((write.value >> 16) & 0xFFFF) as u16;
            }
            REG_VIEWPORT_WH => {
                self.viewport.w = (write.value & 0xFFFF) as u16;
                self.viewport.h = ((write.value >> 16) & 0xFFFF) as u16;
            }
            REG_SCISSOR_XY => {
                self.scissor.x = (write.value & 0xFFFF) as u16;
                self.scissor.y = ((write.value >> 16) & 0xFFFF) as u16;
            }
            REG_SCISSOR_WH => {
                self.scissor.w = (write.value & 0xFFFF) as u16;
                self.scissor.h = ((write.value >> 16) & 0xFFFF) as u16;
            }
            REG_ATTRIB_BASE..=0x0107 => {
                let bit = 1u32 << u32::from(write.reg - REG_ATTRIB_BASE);
                if write.value & 1 == 1 {
                    self.attribute_mask |= bit;
                } else {
                    self.attribute_mask &= !bit;
                }
            }
            REG_INDEX_FORMAT => {
                self.index_format = if write.value & 1 == 0 {
                    IndexFormat::U8
                } else {
                    IndexFormat::U16
                };
            }
            REG_DEPTH_STENCIL => {
                self.depth_stencil.depth_test_enable = write.value & 1 != 0;
                self.depth_stencil.depth_write_enable = write.value & 2 != 0;
                self.depth_stencil.stencil_enable = write.value & 4 != 0;
            }
            REG_BLEND_EQ => {
                self.blend.equation_rgb = (write.value & 0xFF) as u8;
                self.blend.equation_alpha = ((write.value >> 8) & 0xFF) as u8;
            }
            REG_SHADER_CODE => self.shader_microcode.push(write.value),
            REG_SHADER_CONST => self.shader_constant = write.value,
            REG_COLOR_CLEAR => self.clear_color(write.value),
            REG_DRAW_POINT_XY => self.staged_point_xy = write.value,
            REG_DRAW_POINT_COLOR => {
                self.staged_point_color = write.value;
                self.draw_staged_point();
            }
            _ => {}
        }
    }

    fn clear_color(&mut self, color: u32) {
        let shaded = self.run_shader_pipeline(color);
        let resolved = self.resolve_framebuffer_color(shaded);
        self.frame_buffer.fill(resolved);
    }

    fn draw_staged_point(&mut self) {
        let x = (self.staged_point_xy & 0xFFFF) as u16;
        let y = ((self.staged_point_xy >> 16) & 0xFFFF) as u16;
        if !self.point_in_rect(x, y, self.viewport) || !self.point_in_rect(x, y, self.scissor) {
            return;
        }
        let xi = usize::from(x).min(Self::WIDTH.saturating_sub(1));
        let yi = usize::from(y).min(Self::HEIGHT.saturating_sub(1));
        let idx = yi * Self::WIDTH + xi;
        let shaded = self.run_shader_pipeline(self.staged_point_color);
        self.frame_buffer[idx] = self.resolve_framebuffer_color(shaded);
    }

    fn point_in_rect(&self, x: u16, y: u16, rect: impl Into<ScissorRect>) -> bool {
        let rect = rect.into();
        x >= rect.x
            && y >= rect.y
            && x < rect.x.saturating_add(rect.w)
            && y < rect.y.saturating_add(rect.h)
    }

    fn run_shader_pipeline(&mut self, color: u32) -> u32 {
        let hash = self.current_shader_hash();
        self.shader_cache.entry(hash).or_insert_with(|| {
            format!(
                "// pseudo translated GLSL\\nvec4 main() {{ return unpackUnorm4x8(0x{hash:016X}u); }}"
            )
        });
        color ^ self.shader_constant
    }

    fn current_shader_hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for word in &self.shader_microcode {
            hash ^= u64::from(*word);
            hash = hash.wrapping_mul(0x1000_0000_01B3);
        }
        hash ^= u64::from(self.shader_constant);
        hash
    }

    fn resolve_framebuffer_color(&self, rgba: u32) -> u32 {
        let [r, g, b, a] = rgba.to_be_bytes();
        let tiled_idx = ((usize::from(r) >> 3) ^ (usize::from(g) >> 3)) & 1;
        let (r, b) = if tiled_idx == 0 { (r, b) } else { (b, r) };
        u32::from_be_bytes([r, g, b, a])
    }

    pub fn frame_u8(&self) -> Vec<u8> {
        self.frame_buffer
            .iter()
            .flat_map(|px| px.to_le_bytes())
            .collect()
    }

    pub fn state_snapshot(&self) -> PicaStateSnapshot {
        PicaStateSnapshot {
            attribute_mask: self.attribute_mask,
            index_format: self.index_format,
            viewport: self.viewport,
            scissor: self.scissor,
            depth_stencil: self.depth_stencil,
            blend: self.blend,
            shader_program_hash: self.current_shader_hash(),
            trace_len: self.trace.len(),
        }
    }

    pub fn trace(&self) -> &[GpuTraceEntry] {
        &self.trace
    }

    pub fn deterministic_render_hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for px in &self.frame_buffer {
            hash ^= u64::from(*px);
            hash = hash.wrapping_mul(0x1000_0000_01B3);
        }
        hash
    }
}

impl From<Viewport> for ScissorRect {
    fn from(value: Viewport) -> Self {
        ScissorRect {
            x: value.x,
            y: value.y,
            w: value.w,
            h: value.h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_decodes_pica_packets() {
        let cmd = PicaCommandProcessor;
        let words = vec![
            PicaCommandBufferPacket::encode(REG_ATTRIB_BASE, 3, true),
            1,
            0,
            1,
            PicaCommandBufferPacket::encode(REG_BLEND_EQ, 1, false),
            0x0302,
        ];
        let writes = cmd.decode_command_words(&words);
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[0].reg, REG_ATTRIB_BASE);
        assert_eq!(writes[2].reg, REG_ATTRIB_BASE + 2);
        assert_eq!(writes[3].reg, REG_BLEND_EQ);
    }

    #[test]
    fn gpu_trace_and_render_hash_are_deterministic() {
        let mut gpu = PicaGpu::new();
        gpu.enqueue_gsp_fifo_words(&[
            PicaCommandBufferPacket::encode(REG_VIEWPORT_XY, 1, false),
            0,
            PicaCommandBufferPacket::encode(REG_VIEWPORT_WH, 1, false),
            ((PicaGpu::HEIGHT as u32) << 16) | PicaGpu::WIDTH as u32,
            PicaCommandBufferPacket::encode(REG_SCISSOR_XY, 1, false),
            0,
            PicaCommandBufferPacket::encode(REG_SCISSOR_WH, 1, false),
            ((PicaGpu::HEIGHT as u32) << 16) | PicaGpu::WIDTH as u32,
            PicaCommandBufferPacket::encode(REG_DEPTH_STENCIL, 1, false),
            0x3,
            PicaCommandBufferPacket::encode(REG_BLEND_EQ, 1, false),
            0x0201,
            PicaCommandBufferPacket::encode(REG_SHADER_CONST, 1, false),
            0x00FF_0000,
            PicaCommandBufferPacket::encode(REG_COLOR_CLEAR, 1, false),
            0xFF22_3344,
            PicaCommandBufferPacket::encode(REG_DRAW_POINT_XY, 1, false),
            (10 << 16) | 20,
            PicaCommandBufferPacket::encode(REG_DRAW_POINT_COLOR, 1, false),
            0xFFAB_CDEF,
        ]);
        gpu.tick(0);

        let snapshot = gpu.state_snapshot();
        assert_eq!(snapshot.depth_stencil.depth_test_enable, true);
        assert_eq!(snapshot.blend.equation_rgb, 0x01);
        assert!(snapshot.trace_len >= 10);
        assert_eq!(gpu.trace().first().expect("trace").reg, REG_VIEWPORT_XY);

        let hash = gpu.deterministic_render_hash();
        assert_eq!(hash, gpu.deterministic_render_hash());
    }
}
