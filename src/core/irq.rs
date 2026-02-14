#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqLine {
    Timer0 = 0,
    VBlank = 1,
    Dma0 = 2,
}

impl IrqLine {
    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

#[derive(Clone)]
pub struct IrqController {
    enabled: u32,
    pending: u32,
}

impl Default for IrqController {
    fn default() -> Self {
        Self::new()
    }
}

impl IrqController {
    pub fn new() -> Self {
        Self {
            enabled: u32::MAX,
            pending: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pending = 0;
    }

    pub fn set_enabled_mask(&mut self, mask: u32) {
        self.enabled = mask;
    }

    pub fn raise(&mut self, line: IrqLine) {
        self.pending |= line.bit();
    }

    pub fn clear(&mut self, line: IrqLine) {
        self.pending &= !line.bit();
    }

    pub fn next_pending(&self) -> Option<IrqLine> {
        let active = self.pending & self.enabled;
        if active & IrqLine::Timer0.bit() != 0 {
            return Some(IrqLine::Timer0);
        }
        if active & IrqLine::VBlank.bit() != 0 {
            return Some(IrqLine::VBlank);
        }
        if active & IrqLine::Dma0.bit() != 0 {
            return Some(IrqLine::Dma0);
        }
        None
    }
}
