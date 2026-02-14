const MAX_BUFFERED_SAMPLES: usize = 4096;

#[derive(Clone)]
pub struct Dsp {
    samples: Vec<i16>,
    phase: u64,
}

impl Default for Dsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Dsp {
    pub fn new() -> Self {
        Self {
            samples: Vec::with_capacity(MAX_BUFFERED_SAMPLES),
            phase: 0,
        }
    }

    pub fn produce_samples(&mut self, count: u64) {
        for _ in 0..count {
            let value = ((self.phase % 64) as i16 - 32) * 128;
            self.samples.push(value);
            self.phase = self.phase.wrapping_add(1);
        }

        if self.samples.len() > MAX_BUFFERED_SAMPLES {
            let drain = self.samples.len() - MAX_BUFFERED_SAMPLES;
            self.samples.drain(0..drain);
        }
    }

    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    pub fn take_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.samples)
    }
}
