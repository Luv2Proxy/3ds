const CPU_HZ: u64 = 268_000_000;
const AUDIO_HZ: u64 = 48_000;
const VIDEO_HZ: u64 = 60;

const SCANLINES_TOTAL: u16 = 262;
const SCANLINES_ACTIVE: u16 = 240;
const BOTTOM_SCANLINE_OFFSET: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenTimingSnapshot {
    pub scanline: u16,
    pub in_vblank: bool,
    pub frame_count: u64,
    pub vblank_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingTick {
    pub audio_samples: u64,
    pub video_frames: u64,
    pub top_vblank_edges: u64,
    pub bottom_vblank_edges: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriftCorrectionPolicy {
    pub max_frame_lead_us: i64,
    pub max_frame_lag_us: i64,
}

impl Default for DriftCorrectionPolicy {
    fn default() -> Self {
        Self {
            max_frame_lead_us: 2_000,
            max_frame_lag_us: 8_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSnapshot {
    pub cpu_cycles: u64,
    pub audio_samples_due: u64,
    pub video_frames_due: u64,
    pub av_desync_samples: i64,
    pub top: ScreenTimingSnapshot,
    pub bottom: ScreenTimingSnapshot,
    pub emu_time_us: u64,
}

#[derive(Clone)]
pub struct TimingModel {
    cpu_cycles: u64,
    audio_samples_due: u64,
    video_frames_due: u64,
    audio_phase: u64,
    video_phase: u64,
    top_vblank_count: u64,
    bottom_vblank_count: u64,
    wall_time_anchor_us: u64,
    drift_policy: DriftCorrectionPolicy,
}

impl Default for TimingModel {
    fn default() -> Self {
        Self {
            cpu_cycles: 0,
            audio_samples_due: 0,
            video_frames_due: 0,
            audio_phase: 0,
            video_phase: 0,
            top_vblank_count: 0,
            bottom_vblank_count: 0,
            wall_time_anchor_us: 0,
            drift_policy: DriftCorrectionPolicy::default(),
        }
    }
}

impl TimingModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_drift_policy(&mut self, policy: DriftCorrectionPolicy) {
        self.drift_policy = policy;
    }

    pub fn set_wall_time_anchor_us(&mut self, host_wall_time_us: u64) {
        self.wall_time_anchor_us = host_wall_time_us;
    }

    pub fn recommended_cycle_budget(&self, host_wall_time_us: u64, requested_cycles: u32) -> u32 {
        let clamped_host_us = host_wall_time_us.saturating_sub(self.wall_time_anchor_us);
        let emu_us = self.emulated_time_us() as i64;
        let host_us = clamped_host_us as i64;
        let drift = emu_us - host_us;

        if drift > self.drift_policy.max_frame_lead_us {
            (requested_cycles / 2).max(1)
        } else if drift < -self.drift_policy.max_frame_lag_us {
            requested_cycles.saturating_add(requested_cycles / 2)
        } else {
            requested_cycles
        }
    }

    pub fn tick(&mut self, cycles: u32) -> TimingTick {
        let cycles = u64::from(cycles);
        let prev_cycles = self.cpu_cycles;
        let prev_top = self.screen_snapshot_at(prev_cycles, Screen::Top, self.top_vblank_count);
        let prev_bottom =
            self.screen_snapshot_at(prev_cycles, Screen::Bottom, self.bottom_vblank_count);

        self.cpu_cycles = self.cpu_cycles.saturating_add(cycles);

        self.audio_phase = self
            .audio_phase
            .saturating_add(cycles.saturating_mul(AUDIO_HZ));
        let produced_samples = self.audio_phase / CPU_HZ;
        self.audio_phase %= CPU_HZ;
        self.audio_samples_due = self.audio_samples_due.saturating_add(produced_samples);

        self.video_phase = self
            .video_phase
            .saturating_add(cycles.saturating_mul(VIDEO_HZ));
        let produced_frames = self.video_phase / CPU_HZ;
        self.video_phase %= CPU_HZ;
        self.video_frames_due = self.video_frames_due.saturating_add(produced_frames);

        let next_top = self.screen_snapshot_at(self.cpu_cycles, Screen::Top, self.top_vblank_count);
        let next_bottom =
            self.screen_snapshot_at(self.cpu_cycles, Screen::Bottom, self.bottom_vblank_count);

        let top_edges = u64::from(!prev_top.in_vblank && next_top.in_vblank);
        let bottom_edges = u64::from(!prev_bottom.in_vblank && next_bottom.in_vblank);
        self.top_vblank_count = self.top_vblank_count.saturating_add(top_edges);
        self.bottom_vblank_count = self.bottom_vblank_count.saturating_add(bottom_edges);

        TimingTick {
            audio_samples: produced_samples,
            video_frames: produced_frames,
            top_vblank_edges: top_edges,
            bottom_vblank_edges: bottom_edges,
        }
    }

    pub fn snapshot(&self) -> TimingSnapshot {
        let nominal_samples_per_frame = (AUDIO_HZ / VIDEO_HZ) as i64;
        let expected_samples = self.video_frames_due as i64 * nominal_samples_per_frame;
        TimingSnapshot {
            cpu_cycles: self.cpu_cycles,
            audio_samples_due: self.audio_samples_due,
            video_frames_due: self.video_frames_due,
            av_desync_samples: self.audio_samples_due as i64 - expected_samples,
            top: self.screen_snapshot_at(self.cpu_cycles, Screen::Top, self.top_vblank_count),
            bottom: self.screen_snapshot_at(
                self.cpu_cycles,
                Screen::Bottom,
                self.bottom_vblank_count,
            ),
            emu_time_us: self.emulated_time_us(),
        }
    }

    fn screen_snapshot_at(
        &self,
        cpu_cycles: u64,
        screen: Screen,
        vblank_count: u64,
    ) -> ScreenTimingSnapshot {
        let frame_cycles = CPU_HZ / VIDEO_HZ;
        let mut cycle_in_frame = cpu_cycles % frame_cycles;
        let line_cycles = (frame_cycles / u64::from(SCANLINES_TOTAL)).max(1);

        if matches!(screen, Screen::Bottom) {
            cycle_in_frame = (cycle_in_frame
                + u64::from(BOTTOM_SCANLINE_OFFSET).saturating_mul(line_cycles))
                % frame_cycles;
        }

        let scanline = (cycle_in_frame / line_cycles) as u16;
        let in_vblank = scanline >= SCANLINES_ACTIVE;

        ScreenTimingSnapshot {
            scanline,
            in_vblank,
            frame_count: self.video_frames_due,
            vblank_count,
        }
    }

    pub fn emulated_time_us(&self) -> u64 {
        self.cpu_cycles.saturating_mul(1_000_000) / CPU_HZ
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_equal_cycle_input() {
        let mut a = TimingModel::new();
        let mut b = TimingModel::new();

        for _ in 0..64 {
            a.tick(1024);
            b.tick(1024);
        }

        assert_eq!(a.snapshot(), b.snapshot());
    }

    #[test]
    fn stable_frame_and_audio_counts() {
        let mut timing = TimingModel::new();
        for _ in 0..1_000 {
            timing.tick(4_096);
        }
        let first = timing.snapshot();

        timing.reset();

        for _ in 0..1_000 {
            timing.tick(4_096);
        }
        let second = timing.snapshot();

        assert_eq!(first.video_frames_due, second.video_frames_due);
        assert_eq!(first.audio_samples_due, second.audio_samples_due);
    }

    #[test]
    fn drift_policy_scales_budget() {
        let mut timing = TimingModel::new();
        timing.set_wall_time_anchor_us(0);
        timing.tick((CPU_HZ / VIDEO_HZ) as u32);

        let slow_down = timing.recommended_cycle_budget(1_000, 10_000);
        assert!(slow_down < 10_000);

        timing.reset();
        let speed_up = timing.recommended_cycle_budget(50_000, 10_000);
        assert!(speed_up > 10_000);
    }
}
