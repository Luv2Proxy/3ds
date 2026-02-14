const MAX_BUFFERED_SAMPLES: usize = 4096;

#[derive(Clone)]
pub struct Dsp {
    samples: Vec<i16>,
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
        }
    }

    pub fn tick(&mut self, cycle: u64) {
        let value = ((cycle % 64) as i16 - 32) * 128;
        self.samples.push(value);
        if self.samples.len() > MAX_BUFFERED_SAMPLES {
            let drain = self.samples.len() - MAX_BUFFERED_SAMPLES;
            self.samples.drain(0..drain);
        }
    }

    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
}
