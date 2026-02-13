#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuCommand {
    Clear(u32),
    DrawPoint { x: u16, y: u16, color: u32 },
}

#[derive(Clone)]
pub struct PicaGpu {
    frame_buffer: Vec<u32>,
    queue: Vec<GpuCommand>,
    shader_constant: u32,
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
            queue: Vec::new(),
            shader_constant: 0x0000_0000,
        }
    }

    pub fn enqueue_command(&mut self, command: GpuCommand) {
        self.queue.push(command);
    }

    pub fn set_shader_constant(&mut self, rgba: u32) {
        self.shader_constant = rgba;
    }

    fn run_shader_pipeline(&self, color: u32) -> u32 {
        color ^ self.shader_constant
    }

    pub fn tick(&mut self, cycle: u64) {
        if let Some(command) = self.queue.pop() {
            match command {
                GpuCommand::Clear(color) => {
                    let shaded = self.run_shader_pipeline(color);
                    self.frame_buffer.fill(shaded);
                }
                GpuCommand::DrawPoint { x, y, color } => {
                    let xi = usize::from(x).min(Self::WIDTH.saturating_sub(1));
                    let yi = usize::from(y).min(Self::HEIGHT.saturating_sub(1));
                    let idx = yi * Self::WIDTH + xi;
                    self.frame_buffer[idx] = self.run_shader_pipeline(color);
                }
            }
            return;
        }

        let idx = (cycle as usize) % self.frame_buffer.len();
        let color = if cycle & 1 == 0 {
            0xFF00_00FF
        } else {
            0xFF00_FF00
        };
        self.frame_buffer[idx] = color;
    }

    pub fn frame_u8(&self) -> Vec<u8> {
        self.frame_buffer
            .iter()
            .flat_map(|px| px.to_le_bytes())
            .collect()
    }
}
