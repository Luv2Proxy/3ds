use std::collections::{HashMap, VecDeque};

use super::ipc::{
    CommandBuffer, CommandHeader, Handle, IpcPort, IpcSession, ProcessId, RESULT_INVALID_COMMAND,
    RESULT_INVALID_HANDLE, RESULT_NOT_FOUND, RESULT_OK, service_name_from_words,
};
use super::pica::PicaCommandBufferPacket;

const KERNEL_PROCESS_ID: ProcessId = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCall {
    Yield,
    GetTick,
    SendSyncRequest,
    Unknown(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceEvent {
    pub call: ServiceCall,
    pub argument: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceTarget {
    Srv,
    Fs,
    Apt,
    GspGpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelObject {
    Port(ServiceTarget),
    Session(ServiceTarget),
    Archive(u32),
    File(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcResponse {
    pub result_code: u32,
    pub words: Vec<u32>,
}

#[derive(Debug, Clone)]
struct IpcRequest {
    session_handle: Handle,
    command: CommandBuffer,
}

#[derive(Clone, Default)]
struct ProcessState {
    handles: HashMap<Handle, KernelObject>,
    pending_requests: VecDeque<IpcRequest>,
    pending_responses: VecDeque<IpcResponse>,
    last_result_code: u32,
}

#[derive(Clone, Default)]
pub struct Kernel {
    svc_log: Vec<ServiceEvent>,
    ticks: u64,
    next_handle: Handle,
    processes: HashMap<ProcessId, ProcessState>,
    service_ports: HashMap<String, (ProcessId, Handle)>,
    app_state: u32,
    gpu_handoff: VecDeque<Vec<u32>>,
}

impl Kernel {
    pub fn new() -> Self {
        let mut kernel = Self {
            next_handle: 0x20,
            app_state: 1,
            ..Self::default()
        };
        kernel.ensure_process(KERNEL_PROCESS_ID);
        kernel.bootstrap_services(KERNEL_PROCESS_ID);
        kernel
    }

    fn bootstrap_services(&mut self, pid: ProcessId) {
        self.register_service_port(pid, "srv:", 16);
        self.register_service_port(pid, "fs:", 8);
        self.register_service_port(pid, "apt:", 4);
        self.register_service_port(pid, "gsp::Gpu", 4);
    }

    pub fn reset_runtime(&mut self) {
        self.svc_log.clear();
        self.ticks = 0;
        self.next_handle = 0x20;
        self.processes.clear();
        self.service_ports.clear();
        self.app_state = 1;
        self.gpu_handoff.clear();
        self.ensure_process(KERNEL_PROCESS_ID);
        self.bootstrap_services(KERNEL_PROCESS_ID);
    }

    pub fn tick(&mut self, cycles: u32) {
        self.ticks = self.ticks.saturating_add(u64::from(cycles));
    }

    pub fn handle_swi(&mut self, imm24: u32) {
        let call = match imm24 {
            0x00 => ServiceCall::Yield,
            0x01 => ServiceCall::GetTick,
            0x32 => {
                self.pump_ipc_events(1);
                ServiceCall::SendSyncRequest
            }
            other => ServiceCall::Unknown(other),
        };
        self.svc_log.push(ServiceEvent {
            call,
            argument: imm24,
        });
    }

    pub fn pump_ipc_events(&mut self, budget: usize) {
        for _ in 0..budget {
            let mut selected: Option<(ProcessId, IpcRequest)> = None;
            for (pid, proc_state) in &mut self.processes {
                if let Some(req) = proc_state.pending_requests.pop_front() {
                    selected = Some((*pid, req));
                    break;
                }
            }

            let Some((pid, req)) = selected else {
                break;
            };

            let (result_code, words) = self.dispatch_request(pid, req);
            if let Some(proc_state) = self.processes.get_mut(&pid) {
                proc_state.last_result_code = result_code;
                proc_state
                    .pending_responses
                    .push_back(IpcResponse { result_code, words });
            }
        }
    }

    pub fn queue_ipc_command(&mut self, pid: ProcessId, session_handle: Handle, words: Vec<u32>) {
        self.ensure_process(pid);
        let command = CommandBuffer::parse(&words).unwrap_or(CommandBuffer {
            header: CommandHeader {
                command_id: 0,
                normal_words: 0,
                translate_words: 0,
            },
            words: vec![],
        });
        if let Some(proc_state) = self.processes.get_mut(&pid) {
            proc_state.pending_requests.push_back(IpcRequest {
                session_handle,
                command,
            });
        }
    }

    pub fn pop_ipc_response(&mut self, pid: ProcessId) -> Option<IpcResponse> {
        self.processes.get_mut(&pid)?.pending_responses.pop_front()
    }

    pub fn ensure_process(&mut self, pid: ProcessId) {
        self.processes.entry(pid).or_default();
    }

    pub fn register_service_port(
        &mut self,
        pid: ProcessId,
        name: &str,
        max_sessions: u32,
    ) -> Handle {
        self.ensure_process(pid);
        let target = match name {
            "srv:" => ServiceTarget::Srv,
            "fs:" => ServiceTarget::Fs,
            "apt:" => ServiceTarget::Apt,
            "gsp::Gpu" => ServiceTarget::GspGpu,
            _ => ServiceTarget::Srv,
        };
        let _port = IpcPort {
            name: name.to_string(),
            max_sessions,
        };
        let handle = self.allocate_handle(pid, KernelObject::Port(target));
        self.service_ports.insert(name.to_string(), (pid, handle));
        handle
    }

    pub fn connect_to_service(&mut self, pid: ProcessId, name: &str) -> Option<Handle> {
        self.ensure_process(pid);
        let (owner_pid, port_handle) = *self.service_ports.get(name)?;
        let target = match self.lookup_object(owner_pid, port_handle)? {
            KernelObject::Port(target) => target,
            _ => return None,
        };
        let _session = IpcSession {
            service: name.to_string(),
            server_port: port_handle,
        };
        Some(self.allocate_handle(pid, KernelObject::Session(target)))
    }

    pub fn last_result_code(&self, pid: ProcessId) -> Option<u32> {
        self.processes.get(&pid).map(|p| p.last_result_code)
    }

    pub fn last_service_call(&self) -> Option<ServiceEvent> {
        self.svc_log.last().copied()
    }

    pub fn service_call_count(&self) -> usize {
        self.svc_log.len()
    }

    pub fn drain_gpu_handoff(&mut self) -> Vec<Vec<u32>> {
        self.gpu_handoff.drain(..).collect()
    }

    fn allocate_handle(&mut self, pid: ProcessId, object: KernelObject) -> Handle {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        if let Some(proc_state) = self.processes.get_mut(&pid) {
            proc_state.handles.insert(handle, object);
        }
        handle
    }

    fn lookup_object(&self, pid: ProcessId, handle: Handle) -> Option<KernelObject> {
        self.processes.get(&pid)?.handles.get(&handle).copied()
    }

    fn dispatch_request(&mut self, pid: ProcessId, req: IpcRequest) -> (u32, Vec<u32>) {
        let Some(object) = self.lookup_object(pid, req.session_handle) else {
            return (RESULT_INVALID_HANDLE, vec![]);
        };
        let KernelObject::Session(target) = object else {
            return (RESULT_INVALID_HANDLE, vec![]);
        };

        match target {
            ServiceTarget::Srv => self.dispatch_srv(pid, req.command),
            ServiceTarget::Fs => self.dispatch_fs(pid, req.command),
            ServiceTarget::Apt => self.dispatch_apt(req.command),
            ServiceTarget::GspGpu => self.dispatch_gsp(req.command),
        }
    }

    fn dispatch_srv(&mut self, pid: ProcessId, cmd: CommandBuffer) -> (u32, Vec<u32>) {
        match cmd.header.command_id {
            0x0001 => {
                if cmd.words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let name = service_name_from_words(&cmd.words[..2]);
                let max_sessions = cmd.words[2];
                let port = self.register_service_port(pid, &name, max_sessions);
                (RESULT_OK, vec![port])
            }
            0x0005 => {
                if cmd.words.len() < 2 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let name = service_name_from_words(&cmd.words[..2]);
                match self.connect_to_service(pid, &name) {
                    Some(handle) => (RESULT_OK, vec![handle]),
                    None => (RESULT_NOT_FOUND, vec![]),
                }
            }
            _ => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_fs(&mut self, pid: ProcessId, cmd: CommandBuffer) -> (u32, Vec<u32>) {
        match cmd.header.command_id {
            0x0001 => {
                let archive_id = cmd.words.first().copied().unwrap_or(0);
                let archive = self.allocate_handle(pid, KernelObject::Archive(archive_id));
                (RESULT_OK, vec![archive])
            }
            0x0002 => {
                if cmd.words.len() < 2 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let archive_handle = cmd.words[0];
                if !matches!(
                    self.lookup_object(pid, archive_handle),
                    Some(KernelObject::Archive(_))
                ) {
                    return (RESULT_INVALID_HANDLE, vec![]);
                }
                let file_id = cmd.words[1];
                let file = self.allocate_handle(pid, KernelObject::File(file_id));
                (RESULT_OK, vec![file])
            }
            _ => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_apt(&mut self, cmd: CommandBuffer) -> (u32, Vec<u32>) {
        match cmd.header.command_id {
            0x0001 => (RESULT_OK, vec![self.app_state]),
            0x0002 => {
                self.app_state = cmd.words.first().copied().unwrap_or(self.app_state);
                (RESULT_OK, vec![self.app_state])
            }
            _ => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_gsp(&mut self, cmd: CommandBuffer) -> (u32, Vec<u32>) {
        match cmd.header.command_id {
            0x0001 => {
                let color = cmd.words.first().copied().unwrap_or(0xFF00_0000);
                self.gpu_handoff.push_back(vec![
                    PicaCommandBufferPacket::encode(0x0200, 1, false),
                    color,
                ]);
                (RESULT_OK, vec![])
            }
            0x0002 => {
                if cmd.words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                self.gpu_handoff.push_back(vec![
                    PicaCommandBufferPacket::encode(0x0201, 1, false),
                    (cmd.words[1] << 16) | (cmd.words[0] & 0xFFFF),
                    PicaCommandBufferPacket::encode(0x0202, 1, false),
                    cmd.words[2],
                ]);
                (RESULT_OK, vec![])
            }
            _ => (RESULT_INVALID_COMMAND, vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ipc::{CommandHeader, service_name_words};

    fn mk_command(command_id: u16, payload: &[u32]) -> Vec<u32> {
        let header = CommandHeader {
            command_id,
            normal_words: payload.len() as u16,
            translate_words: 0,
        };
        let mut words = vec![header.encode()];
        words.extend_from_slice(payload);
        words
    }

    #[test]
    fn srv_registration_and_handle_acquisition() {
        let mut kernel = Kernel::new();
        let pid = 42;
        kernel.ensure_process(pid);

        let srv_handle = kernel.connect_to_service(pid, "srv:").expect("srv handle");
        let name_words = service_name_words("hid:USER");
        kernel.queue_ipc_command(
            pid,
            srv_handle,
            mk_command(0x0001, &[name_words[0], name_words[1], 2]),
        );
        kernel.pump_ipc_events(1);
        let reg = kernel.pop_ipc_response(pid).expect("registration response");
        assert_eq!(reg.result_code, RESULT_OK);
        assert_eq!(reg.words.len(), 1);

        kernel.queue_ipc_command(pid, srv_handle, mk_command(0x0005, &name_words));
        kernel.pump_ipc_events(1);
        let get = kernel.pop_ipc_response(pid).expect("get service response");
        assert_eq!(get.result_code, RESULT_OK);
        assert_eq!(get.words.len(), 1);
    }

    #[test]
    fn minimal_service_init_sequence() {
        let mut kernel = Kernel::new();
        let pid = 7;
        kernel.ensure_process(pid);

        let srv = kernel.connect_to_service(pid, "srv:").expect("srv session");
        let fs_name = service_name_words("fs:");
        kernel.queue_ipc_command(pid, srv, mk_command(0x0005, &fs_name));
        kernel.pump_ipc_events(1);
        let fs = kernel.pop_ipc_response(pid).expect("fs response").words[0];

        kernel.queue_ipc_command(pid, fs, mk_command(0x0001, &[1]));
        kernel.pump_ipc_events(1);
        let archive = kernel.pop_ipc_response(pid).expect("archive").words[0];

        kernel.queue_ipc_command(pid, fs, mk_command(0x0002, &[archive, 0x1234]));
        kernel.pump_ipc_events(1);
        let file = kernel.pop_ipc_response(pid).expect("file");
        assert_eq!(file.result_code, RESULT_OK);
        assert_eq!(file.words.len(), 1);

        let apt_name = service_name_words("apt:");
        kernel.queue_ipc_command(pid, srv, mk_command(0x0005, &apt_name));
        kernel.pump_ipc_events(1);
        let apt = kernel.pop_ipc_response(pid).expect("apt").words[0];

        kernel.queue_ipc_command(pid, apt, mk_command(0x0001, &[]));
        kernel.pump_ipc_events(1);
        let app_state = kernel.pop_ipc_response(pid).expect("app state");
        assert_eq!(app_state.result_code, RESULT_OK);
        assert_eq!(app_state.words[0], 1);
    }
}
