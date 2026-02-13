#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCall {
    Yield,
    GetTick,
    Unknown(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceEvent {
    pub call: ServiceCall,
    pub argument: u32,
}

#[derive(Clone, Default)]
pub struct Kernel {
    svc_log: Vec<ServiceEvent>,
    ticks: u64,
}

impl Kernel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, cycles: u32) {
        self.ticks = self.ticks.saturating_add(u64::from(cycles));
    }

    pub fn handle_swi(&mut self, imm24: u32) {
        let call = match imm24 {
            0x00 => ServiceCall::Yield,
            0x01 => ServiceCall::GetTick,
            other => ServiceCall::Unknown(other),
        };
        self.svc_log.push(ServiceEvent {
            call,
            argument: imm24,
        });
    }

    pub fn last_service_call(&self) -> Option<ServiceEvent> {
        self.svc_log.last().copied()
    }

    pub fn service_call_count(&self) -> usize {
        self.svc_log.len()
    }
}
