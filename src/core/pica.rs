use std::collections::{HashMap, VecDeque};

const REG_VIEWPORT_XY: u16 = 0x0041;
const REG_VIEWPORT_WH: u16 = 0x0042;
const REG_SCISSOR_XY: u16 = 0x0043;
const REG_SCISSOR_WH: u16 = 0x0044;
const REG_ATTRIB_BASE: u16 = 0x0100;
const REG_INDEX_FORMAT: u16 = 0x0110;
const REG_VERTEX_FORMAT: u16 = 0x0111;
const REG_DEPTH_STENCIL: u16 = 0x0120;
const REG_BLEND_EQ: u16 = 0x0130;
const REG_TEXTURE_ADDR: u16 = 0x0140;
const REG_TEXTURE_SIZE: u16 = 0x0141;
const REG_TEXTURE_FORMAT: u16 = 0x0142;
const REG_TEV_STAGE0: u16 = 0x0150;
const REG_FRAMEBUFFER_FORMAT: u16 = 0x0160;
const REG_COLOR_CLEAR: u16 = 0x0200;
const REG_DRAW_BASE_VERTEX: u16 = 0x0204;
const REG_DRAW_VERTEX_COUNT: u16 = 0x0205;
const REG_DRAW_TRIGGER: u16 = 0x0206;
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
pub enum TextureFormat {
    Rgba8,
    Rgb565,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramebufferFormat {
    Rgba8,
    Rgb565,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VertexFetch {
    stride_words: u8,
    color_offset_words: u8,
    texcoord_offset_words: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShaderOp {
    XorConst,
    RotateChannels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuEvent {
    FrameComplete,
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
    depth_buffer: Vec<u16>,
    register_file: HashMap<u16, u32>,
    fifo: VecDeque<u32>,
    cmd: PicaCommandProcessor,
    attribute_mask: u32,
    index_format: IndexFormat,
    vertex_fetch: VertexFetch,
    viewport: Viewport,
    scissor: ScissorRect,
    depth_stencil: DepthStencilState,
    blend: BlendState,
    texture_format: TextureFormat,
    framebuffer_format: FramebufferFormat,
    texture_size: (u16, u16),
    texture_data: Vec<u8>,
    shader_microcode: Vec<u32>,
    shader_constant: u32,
    translated_shader: Vec<ShaderOp>,
    tev_stage0: u32,
    vertex_stream: Vec<u32>,
    index_stream: Vec<u16>,
    draw_base_vertex: u16,
    draw_vertex_count: u16,
    trace: Vec<GpuTraceEntry>,
    presents: u64,
    events: VecDeque<GpuEvent>,
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
            depth_buffer: vec![u16::MAX; Self::WIDTH * Self::HEIGHT],
            register_file: HashMap::new(),
            fifo: VecDeque::new(),
            cmd: PicaCommandProcessor,
            attribute_mask: 0,
            index_format: IndexFormat::U16,
            vertex_fetch: VertexFetch {
                stride_words: 3,
                color_offset_words: 1,
                texcoord_offset_words: 2,
            },
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
            texture_format: TextureFormat::Rgba8,
            framebuffer_format: FramebufferFormat::Rgba8,
            texture_size: (1, 1),
            texture_data: vec![0xFF, 0xFF, 0xFF, 0xFF],
            shader_microcode: Vec::new(),
            shader_constant: 0,
            translated_shader: Vec::new(),
            tev_stage0: 0,
            vertex_stream: Vec::new(),
            index_stream: Vec::new(),
            draw_base_vertex: 0,
            draw_vertex_count: 0,
            trace: Vec::new(),
            presents: 0,
            events: VecDeque::new(),
        }
    }

    pub fn load_vertex_stream(&mut self, words: &[u32]) {
        self.vertex_stream.clear();
        self.vertex_stream.extend_from_slice(words);
    }

    pub fn load_index_stream_u16(&mut self, indices: &[u16]) {
        self.index_stream.clear();
        self.index_stream.extend_from_slice(indices);
    }

    pub fn load_texture_rgba8(&mut self, width: u16, height: u16, texels: &[u8]) {
        self.texture_size = (width.max(1), height.max(1));
        self.texture_data.clear();
        self.texture_data.extend_from_slice(texels);
        self.texture_format = TextureFormat::Rgba8;
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

    pub fn take_events(&mut self) -> Vec<GpuEvent> {
        self.events.drain(..).collect()
    }

    pub fn present(&mut self, frame_count: u64) {
        self.presents = self.presents.saturating_add(frame_count);
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
            REG_VERTEX_FORMAT => {
                self.vertex_fetch.stride_words = (write.value & 0xFF) as u8;
                self.vertex_fetch.color_offset_words = ((write.value >> 8) & 0xFF) as u8;
                self.vertex_fetch.texcoord_offset_words = ((write.value >> 16) & 0xFF) as u8;
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
            REG_TEXTURE_SIZE => {
                self.texture_size = ((write.value & 0xFFFF) as u16, (write.value >> 16) as u16)
            }
            REG_TEXTURE_FORMAT => {
                self.texture_format = if write.value & 1 == 0 {
                    TextureFormat::Rgba8
                } else {
                    TextureFormat::Rgb565
                }
            }
            REG_TEV_STAGE0 => self.tev_stage0 = write.value,
            REG_FRAMEBUFFER_FORMAT => {
                self.framebuffer_format = if write.value & 1 == 0 {
                    FramebufferFormat::Rgba8
                } else {
                    FramebufferFormat::Rgb565
                }
            }
            REG_SHADER_CODE => self.shader_microcode.push(write.value),
            REG_SHADER_CONST => {
                self.shader_constant = write.value;
                self.translated_shader = self.translate_shader_microcode();
            }
            REG_COLOR_CLEAR => self.clear_color(write.value),
            REG_DRAW_BASE_VERTEX => self.draw_base_vertex = write.value as u16,
            REG_DRAW_VERTEX_COUNT => self.draw_vertex_count = write.value as u16,
            REG_DRAW_TRIGGER => {
                if write.value & 1 != 0 {
                    self.execute_draw_call();
                }
            }
            REG_TEXTURE_ADDR => {}
            _ => {}
        }
    }

    fn translate_shader_microcode(&self) -> Vec<ShaderOp> {
        self.shader_microcode
            .iter()
            .map(|word| {
                if word & 1 == 0 {
                    ShaderOp::XorConst
                } else {
                    ShaderOp::RotateChannels
                }
            })
            .collect()
    }

    fn run_shader_pipeline(&self, color: u32) -> u32 {
        self.translated_shader
            .iter()
            .fold(color, |acc, op| match op {
                ShaderOp::XorConst => acc ^ self.shader_constant,
                ShaderOp::RotateChannels => {
                    let [a, r, g, b] = acc.to_be_bytes();
                    u32::from_be_bytes([a, b, r, g])
                }
            })
    }

    fn clear_color(&mut self, color: u32) {
        let resolved = self.convert_framebuffer_format(self.run_shader_pipeline(color));
        self.frame_buffer.fill(resolved);
        self.depth_buffer.fill(u16::MAX);
    }

    fn execute_draw_call(&mut self) {
        let base = usize::from(self.draw_base_vertex);
        let count =
            usize::from(self.draw_vertex_count).min(self.index_stream.len().saturating_sub(base));
        let mut tri = [0usize; 3];
        for i in (0..count).step_by(3) {
            if i + 2 >= count {
                break;
            }
            tri[0] = self.index_stream[base + i] as usize;
            tri[1] = self.index_stream[base + i + 1] as usize;
            tri[2] = self.index_stream[base + i + 2] as usize;
            self.rasterize_triangle(tri);
        }
        self.events.push_back(GpuEvent::FrameComplete);
    }

    fn rasterize_triangle(&mut self, tri: [usize; 3]) {
        for &v in &tri {
            let stride = usize::from(self.vertex_fetch.stride_words.max(1));
            let pos = v.saturating_mul(stride);
            if pos >= self.vertex_stream.len() {
                continue;
            }
            let xy = self.vertex_stream[pos];
            let x = (xy & 0xFFFF) as u16;
            let y = (xy >> 16) as u16;
            if !self.point_in_rect(x, y, self.viewport.into())
                || !self.point_in_rect(x, y, self.scissor)
            {
                continue;
            }
            let color_word = self
                .vertex_stream
                .get(pos + usize::from(self.vertex_fetch.color_offset_words))
                .copied()
                .unwrap_or(0xFFFF_FFFF);
            let tex_word = self
                .vertex_stream
                .get(pos + usize::from(self.vertex_fetch.texcoord_offset_words))
                .copied()
                .unwrap_or(0);
            let tex_color = self.sample_texture(tex_word);
            let combiner_color = self.combine_color(color_word, tex_color);
            let shaded = self.convert_framebuffer_format(self.run_shader_pipeline(combiner_color));
            let xi = usize::from(x).min(Self::WIDTH - 1);
            let yi = usize::from(y).min(Self::HEIGHT - 1);
            let idx = yi * Self::WIDTH + xi;
            let depth = ((self.vertex_stream.get(pos).copied().unwrap_or(0) >> 8) & 0xFFFF) as u16;
            if self.depth_stencil.depth_test_enable && depth > self.depth_buffer[idx] {
                continue;
            }
            let dst = self.frame_buffer[idx];
            self.frame_buffer[idx] = self.blend_color(dst, shaded);
            if self.depth_stencil.depth_write_enable {
                self.depth_buffer[idx] = depth;
            }
        }
    }

    fn sample_texture(&self, texcoord_word: u32) -> u32 {
        let tx = (texcoord_word & 0xFFFF) as usize;
        let ty = ((texcoord_word >> 16) & 0xFFFF) as usize;
        let w = usize::from(self.texture_size.0.max(1));
        let h = usize::from(self.texture_size.1.max(1));
        let x = tx % w;
        let y = ty % h;
        match self.texture_format {
            TextureFormat::Rgba8 => {
                let idx = (y * w + x) * 4;
                if idx + 3 >= self.texture_data.len() {
                    return 0xFFFF_FFFF;
                }
                u32::from_le_bytes([
                    self.texture_data[idx],
                    self.texture_data[idx + 1],
                    self.texture_data[idx + 2],
                    self.texture_data[idx + 3],
                ])
            }
            TextureFormat::Rgb565 => {
                let idx = (y * w + x) * 2;
                if idx + 1 >= self.texture_data.len() {
                    return 0xFFFF_FFFF;
                }
                let raw = u16::from_le_bytes([self.texture_data[idx], self.texture_data[idx + 1]]);
                let r = ((raw >> 11) & 0x1F) as u8;
                let g = ((raw >> 5) & 0x3F) as u8;
                let b = (raw & 0x1F) as u8;
                u32::from_le_bytes([(r << 3), (g << 2), (b << 3), 0xFF])
            }
        }
    }

    fn combine_color(&self, a: u32, b: u32) -> u32 {
        if self.tev_stage0 & 1 == 0 {
            return a;
        }
        let ac = a.to_le_bytes();
        let bc = b.to_le_bytes();
        u32::from_le_bytes([
            (u16::from(ac[0]) + u16::from(bc[0])).min(255) as u8,
            (u16::from(ac[1]) + u16::from(bc[1])).min(255) as u8,
            (u16::from(ac[2]) + u16::from(bc[2])).min(255) as u8,
            ac[3],
        ])
    }

    fn blend_color(&self, dst: u32, src: u32) -> u32 {
        if self.blend.equation_rgb == 0 {
            return src;
        }
        let d = dst.to_le_bytes();
        let s = src.to_le_bytes();
        u32::from_le_bytes([
            ((u16::from(d[0]) + u16::from(s[0])) / 2) as u8,
            ((u16::from(d[1]) + u16::from(s[1])) / 2) as u8,
            ((u16::from(d[2]) + u16::from(s[2])) / 2) as u8,
            s[3],
        ])
    }

    fn convert_framebuffer_format(&self, rgba: u32) -> u32 {
        match self.framebuffer_format {
            FramebufferFormat::Rgba8 => rgba,
            FramebufferFormat::Rgb565 => {
                let [r, g, b, _a] = rgba.to_le_bytes();
                let raw =
                    ((u16::from(r) >> 3) << 11) | ((u16::from(g) >> 2) << 5) | (u16::from(b) >> 3);
                let r8 = ((raw >> 11) & 0x1F) as u8;
                let g8 = ((raw >> 5) & 0x3F) as u8;
                let b8 = (raw & 0x1F) as u8;
                u32::from_le_bytes([r8 << 3, g8 << 2, b8 << 3, 0xFF])
            }
        }
    }

    fn point_in_rect(&self, x: u16, y: u16, rect: ScissorRect) -> bool {
        x >= rect.x
            && y >= rect.y
            && x < rect.x.saturating_add(rect.w)
            && y < rect.y.saturating_add(rect.h)
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

    fn current_shader_hash(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for word in &self.shader_microcode {
            hash ^= u64::from(*word);
            hash = hash.wrapping_mul(0x1000_0000_01B3);
        }
        hash ^= u64::from(self.shader_constant);
        hash
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

    fn packet(words: &mut Vec<u32>, reg: u16, val: u32) {
        words.push(PicaCommandBufferPacket::encode(reg, 1, false));
        words.push(val);
    }

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
    fn replays_gpu_trace_and_matches_hash_regression() {
        let mut gpu = PicaGpu::new();
        gpu.load_vertex_stream(&[
            (10 << 16) | 10,
            0xFF00_00FF,
            0,
            (20 << 16) | 20,
            0xFF00_FF00,
            0,
            (30 << 16) | 30,
            0xFFFF_0000,
            0,
        ]);
        gpu.load_index_stream_u16(&[0, 1, 2]);
        let mut words = vec![];
        packet(&mut words, REG_VIEWPORT_XY, 0);
        packet(
            &mut words,
            REG_VIEWPORT_WH,
            ((PicaGpu::HEIGHT as u32) << 16) | PicaGpu::WIDTH as u32,
        );
        packet(&mut words, REG_SCISSOR_XY, 0);
        packet(
            &mut words,
            REG_SCISSOR_WH,
            ((PicaGpu::HEIGHT as u32) << 16) | PicaGpu::WIDTH as u32,
        );
        packet(&mut words, REG_VERTEX_FORMAT, 0x0002_0103);
        packet(&mut words, REG_DEPTH_STENCIL, 0x3);
        packet(&mut words, REG_BLEND_EQ, 0x0001);
        packet(&mut words, REG_TEV_STAGE0, 1);
        packet(&mut words, REG_COLOR_CLEAR, 0xFF11_2233);
        packet(&mut words, REG_DRAW_BASE_VERTEX, 0);
        packet(&mut words, REG_DRAW_VERTEX_COUNT, 3);
        packet(&mut words, REG_SHADER_CODE, 0x1);
        packet(&mut words, REG_SHADER_CONST, 0x0010_0020);
        packet(&mut words, REG_DRAW_TRIGGER, 1);
        gpu.enqueue_gsp_fifo_words(&words);
        gpu.tick(0);

        let trace = gpu.trace().to_vec();
        let expected = gpu.deterministic_render_hash();

        let mut replay = PicaGpu::new();
        replay.load_vertex_stream(&[
            (10 << 16) | 10,
            0xFF00_00FF,
            0,
            (20 << 16) | 20,
            0xFF00_FF00,
            0,
            (30 << 16) | 30,
            0xFFFF_0000,
            0,
        ]);
        replay.load_index_stream_u16(&[0, 1, 2]);
        for t in trace {
            replay.enqueue_register_write(t.reg, t.value);
        }
        replay.tick(0);

        assert_eq!(replay.deterministic_render_hash(), expected);
        assert_eq!(gpu.take_events(), vec![GpuEvent::FrameComplete]);
    }
}
