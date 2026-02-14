#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledDeviceEvent {
    TimerExpiry,
    VBlank,
    DmaCompletion { channel: u8 },
}

impl ScheduledDeviceEvent {
    fn priority(self) -> u8 {
        match self {
            ScheduledDeviceEvent::TimerExpiry => 0,
            ScheduledDeviceEvent::VBlank => 1,
            ScheduledDeviceEvent::DmaCompletion { .. } => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScheduledEvent {
    at_cycle: u64,
    order: u64,
    event: ScheduledDeviceEvent,
}

#[derive(Default, Clone)]
pub struct Scheduler {
    cycles: u64,
    next_order: u64,
    pending: Vec<ScheduledEvent>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            cycles: 0,
            next_order: 0,
            pending: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.cycles = 0;
        self.next_order = 0;
        self.pending.clear();
    }

    pub fn tick(&mut self, cycles: u32) {
        self.cycles = self.cycles.saturating_add(cycles as u64);
    }

    pub fn schedule_in(&mut self, cycles_from_now: u64, event: ScheduledDeviceEvent) {
        let at_cycle = self.cycles.saturating_add(cycles_from_now);
        self.schedule_at(at_cycle, event);
    }

    pub fn schedule_at(&mut self, at_cycle: u64, event: ScheduledDeviceEvent) {
        let entry = ScheduledEvent {
            at_cycle,
            order: self.next_order,
            event,
        };
        self.next_order = self.next_order.saturating_add(1);
        self.pending.push(entry);
    }

    pub fn drain_due_events(&mut self) -> Vec<ScheduledDeviceEvent> {
        let mut due = Vec::new();
        let mut remain = Vec::with_capacity(self.pending.len());
        for event in self.pending.drain(..) {
            if event.at_cycle <= self.cycles {
                due.push(event);
            } else {
                remain.push(event);
            }
        }
        self.pending = remain;
        due.sort_by_key(|e| (e.at_cycle, e.event.priority(), e.order));
        due.into_iter().map(|entry| entry.event).collect()
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}
