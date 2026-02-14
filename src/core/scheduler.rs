#[derive(Default, Clone)]
pub struct Scheduler {
    cycles: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { cycles: 0 }
    }

    pub fn reset(&mut self) {
        self.cycles = 0;
    }

    pub fn tick(&mut self, cycles: u32) {
        self.cycles = self.cycles.saturating_add(cycles as u64);
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}
