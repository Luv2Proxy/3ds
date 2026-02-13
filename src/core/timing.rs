const CPU_HZ: u64 = 268_000_000;
const AUDIO_HZ: u64 = 48_000;
const VIDEO_HZ: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSnapshot {
    pub cpu_cycles: u64,
    pub audio_samples_due: u64,
    pub video_frames_due: u64,
    pub av_desync_samples: i64,
}

#[derive(Clone, Default)]
pub struct TimingModel {
    cpu_cycles: u64,
    audio_samples_due: u64,
    video_frames_due: u64,
}

impl TimingModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.cpu_cycles = 0;
        self.audio_samples_due = 0;
        self.video_frames_due = 0;
    }

    pub fn tick(&mut self, cycles: u32) {
        self.cpu_cycles = self.cpu_cycles.saturating_add(u64::from(cycles));
        self.audio_samples_due = self.cpu_cycles.saturating_mul(AUDIO_HZ) / CPU_HZ;
        self.video_frames_due = self.cpu_cycles.saturating_mul(VIDEO_HZ) / CPU_HZ;
    }

    pub fn snapshot(&self) -> TimingSnapshot {
        let nominal_samples_per_frame = (AUDIO_HZ / VIDEO_HZ) as i64;
        let expected_samples = self.video_frames_due as i64 * nominal_samples_per_frame;
        TimingSnapshot {
            cpu_cycles: self.cpu_cycles,
            audio_samples_due: self.audio_samples_due,
            video_frames_due: self.video_frames_due,
            av_desync_samples: self.audio_samples_due as i64 - expected_samples,
        }
    }
}
