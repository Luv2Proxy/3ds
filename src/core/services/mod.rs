use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceTarget {
    Srv,
    FsUser,
    AptU,
    GspGpu,
    HidUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDefinition {
    pub target: ServiceTarget,
    pub max_sessions: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceRegistry {
    definitions: HashMap<String, ServiceDefinition>,
}

impl ServiceRegistry {
    pub fn register(&mut self, name: &str, target: ServiceTarget, max_sessions: u32) {
        self.definitions.insert(
            name.to_string(),
            ServiceDefinition {
                target,
                max_sessions,
            },
        );
    }

    pub fn definition(&self, name: &str) -> Option<ServiceDefinition> {
        self.definitions.get(name).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &ServiceDefinition)> {
        self.definitions.iter()
    }

    pub fn bootstrap() -> Self {
        let mut registry = Self::default();
        registry.register("srv:", ServiceTarget::Srv, 32);
        registry.register("fs:USER", ServiceTarget::FsUser, 8);
        registry.register("apt:u", ServiceTarget::AptU, 8);
        registry.register("gsp::Gpu", ServiceTarget::GspGpu, 4);
        registry.register("hid:USER", ServiceTarget::HidUser, 4);
        registry
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HidInputState {
    pub buttons: u32,
    pub touch_x: u16,
    pub touch_y: u16,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceRuntime {
    pub app_state: u32,
    pub gpu_handoff: VecDeque<Vec<u32>>,
    pub hid_state: HidInputState,
}
