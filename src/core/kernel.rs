use std::collections::{HashMap, VecDeque};

use super::diagnostics::StructuredError;
use super::fs::{ArchiveHandle, ArchiveId, FileHandle, VirtualFileSystem};
use super::ipc::{
    Handle, IpcEvent, IpcMessage, IpcPort, IpcSession, KernelObjectType, ProcessId,
    RESULT_INVALID_COMMAND, RESULT_INVALID_HANDLE, RESULT_NOT_FOUND, RESULT_OK,
    service_name_from_words,
};
use super::pica::PicaCommandBufferPacket;

const KERNEL_PROCESS_ID: ProcessId = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCall {
    Yield,
    GetTick,
    SendSyncRequest,
    CreateEvent,
    DuplicateHandle,
    CloseHandle,
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
    HidUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SrvCommand {
    RegisterService,
    GetServiceHandle,
}

impl SrvCommand {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x0001 => Some(Self::RegisterService),
            0x0005 => Some(Self::GetServiceHandle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FsCommand {
    OpenArchive,
    OpenFile,
    ReadFile,
}

impl FsCommand {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x0001 => Some(Self::OpenArchive),
            0x0002 => Some(Self::OpenFile),
            0x0003 => Some(Self::ReadFile),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AptCommand {
    GetAppState,
    SetAppState,
}

impl AptCommand {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x0001 => Some(Self::GetAppState),
            0x0002 => Some(Self::SetAppState),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GspCommand {
    SetFrameClearColor,
    QueuePixel,
}

impl GspCommand {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x0001 => Some(Self::SetFrameClearColor),
            0x0002 => Some(Self::QueuePixel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HidCommand {
    Initialize,
    GetInputState,
    SetInputState,
}

impl HidCommand {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x0001 => Some(Self::Initialize),
            0x000A => Some(Self::GetInputState),
            0x000B => Some(Self::SetInputState),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServiceDefinition {
    target: ServiceTarget,
    max_sessions: u32,
}

#[derive(Debug, Clone, Default)]
struct ServiceRegistry {
    definitions: HashMap<String, ServiceDefinition>,
}

impl ServiceRegistry {
    fn register(&mut self, name: &str, target: ServiceTarget, max_sessions: u32) {
        self.definitions.insert(
            name.to_string(),
            ServiceDefinition {
                target,
                max_sessions,
            },
        );
    }

    fn definition(&self, name: &str) -> Option<ServiceDefinition> {
        self.definitions.get(name).copied()
    }

    fn bootstrap() -> Self {
        let mut registry = Self::default();
        registry.register("srv:", ServiceTarget::Srv, 16);
        registry.register("fs:USER", ServiceTarget::Fs, 8);
        registry.register("apt:u", ServiceTarget::Apt, 4);
        registry.register("gsp::Gpu", ServiceTarget::GspGpu, 4);
        registry.register("hid:USER", ServiceTarget::HidUser, 4);
        registry
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HidInputState {
    buttons: u32,
    touch_x: u16,
    touch_y: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KernelObject {
    Port(ServiceTarget),
    Session(ServiceTarget),
    Event(IpcEvent),
    Archive(ArchiveHandle),
    File(FileHandle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcResponse {
    pub result_code: u32,
    pub words: Vec<u32>,
}

#[derive(Debug, Clone)]
struct IpcRequest {
    session_handle: Handle,
    message: IpcMessage,
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
    registry: ServiceRegistry,
    app_state: u32,
    gpu_handoff: VecDeque<Vec<u32>>,
    hid_state: HidInputState,
    vfs: VirtualFileSystem,
    last_ipc: Option<(u16, Handle, u32)>,
    last_service_imm24: Option<u32>,
    last_error: Option<StructuredError>,
}

impl Kernel {
    pub fn new() -> Self {
        let mut kernel = Self {
            next_handle: 0x20,
            app_state: 1,
            registry: ServiceRegistry::bootstrap(),
            vfs: VirtualFileSystem::default(),
            last_ipc: None,
            last_service_imm24: None,
            last_error: None,
            ..Self::default()
        };
        kernel.ensure_process(KERNEL_PROCESS_ID);
        kernel.bootstrap_services(KERNEL_PROCESS_ID);
        kernel
    }

    fn bootstrap_services(&mut self, pid: ProcessId) {
        let definitions: Vec<(String, u32)> = self
            .registry
            .definitions
            .iter()
            .map(|(name, def)| (name.clone(), def.max_sessions))
            .collect();
        for (name, max_sessions) in definitions {
            self.register_service_port(pid, &name, max_sessions);
        }
    }

    pub fn reset_runtime(&mut self) {
        self.svc_log.clear();
        self.ticks = 0;
        self.next_handle = 0x20;
        self.processes.clear();
        self.service_ports.clear();
        self.registry = ServiceRegistry::bootstrap();
        self.app_state = 1;
        self.gpu_handoff.clear();
        self.hid_state = HidInputState::default();
        self.vfs = VirtualFileSystem::default();
        self.last_ipc = None;
        self.last_service_imm24 = None;
        self.last_error = None;
        self.ensure_process(KERNEL_PROCESS_ID);
        self.bootstrap_services(KERNEL_PROCESS_ID);
    }

    pub fn tick(&mut self, cycles: u32) {
        self.ticks = self.ticks.saturating_add(u64::from(cycles));
    }

    pub fn handle_swi(&mut self, imm24: u32) {
        let call = self.dispatch_syscall(1, imm24, &[]);
        self.svc_log.push(ServiceEvent {
            call,
            argument: imm24,
        });
        self.last_service_imm24 = Some(imm24);
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

            let cmd_id = req.message.command_id;
            let handle_id = req.session_handle;
            let (result_code, words) = self.dispatch_request(pid, req);
            self.last_ipc = Some((cmd_id, handle_id, result_code));
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
        let message = IpcMessage::parse(&words).unwrap_or(IpcMessage {
            command_id: 0,
            normal_words: vec![],
            descriptors: vec![],
        });
        if let Some(proc_state) = self.processes.get_mut(&pid) {
            proc_state.pending_requests.push_back(IpcRequest {
                session_handle,
                message,
            });
        }
    }

    pub fn dispatch_syscall(&mut self, pid: ProcessId, imm24: u32, args: &[u32]) -> ServiceCall {
        self.ensure_process(pid);
        match imm24 {
            0x00 => ServiceCall::Yield,
            0x01 => ServiceCall::GetTick,
            0x23 => {
                let _ = self.create_event(pid, "svc:event");
                ServiceCall::CreateEvent
            }
            0x27 => {
                if let Some(&handle) = args.first() {
                    let _ = self.duplicate_handle(pid, handle);
                }
                ServiceCall::DuplicateHandle
            }
            0x29 => {
                if let Some(&handle) = args.first() {
                    let _ = self.close_handle(pid, handle);
                }
                ServiceCall::CloseHandle
            }
            0x32 => {
                self.pump_ipc_events(1);
                ServiceCall::SendSyncRequest
            }
            other => ServiceCall::Unknown(other),
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
        let target = self
            .registry
            .definition(name)
            .map(|def| def.target)
            .unwrap_or(ServiceTarget::Srv);
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

    pub fn take_last_ipc_dispatch(&mut self) -> Option<(u16, Handle, u32)> {
        self.last_ipc.take()
    }

    pub fn take_last_service_imm24(&mut self) -> Option<u32> {
        self.last_service_imm24.take()
    }

    pub fn report_error(&mut self, err: StructuredError) {
        self.last_error = Some(err);
    }

    pub fn take_last_error(&mut self) -> Option<StructuredError> {
        self.last_error.take()
    }

    pub fn mount_romfs(&mut self, romfs: super::fs::RomFs) {
        self.vfs.mount_romfs(romfs);
    }
    pub fn drain_gpu_handoff(&mut self) -> Vec<Vec<u32>> {
        self.gpu_handoff.drain(..).collect()
    }

    pub fn create_event(&mut self, pid: ProcessId, name: &str) -> Handle {
        self.allocate_handle(
            pid,
            KernelObject::Event(IpcEvent {
                name: name.to_string(),
                signaled: false,
            }),
        )
    }

    pub fn duplicate_handle(&mut self, pid: ProcessId, handle: Handle) -> Option<Handle> {
        let obj = self.lookup_object(pid, handle)?;
        Some(self.allocate_handle(pid, obj))
    }

    pub fn close_handle(&mut self, pid: ProcessId, handle: Handle) -> bool {
        self.processes
            .get_mut(&pid)
            .and_then(|p| p.handles.remove(&handle))
            .is_some()
    }

    pub fn handle_type(&self, pid: ProcessId, handle: Handle) -> Option<KernelObjectType> {
        let obj = self.lookup_object(pid, handle)?;
        let kind = match obj {
            KernelObject::Port(_) => KernelObjectType::Port,
            KernelObject::Session(_) => KernelObjectType::Session,
            KernelObject::Event(_) => KernelObjectType::Event,
            KernelObject::Archive(_) => KernelObjectType::Archive,
            KernelObject::File(_) => KernelObjectType::File,
        };
        Some(kind)
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
        self.processes.get(&pid)?.handles.get(&handle).cloned()
    }

    fn dispatch_request(&mut self, pid: ProcessId, req: IpcRequest) -> (u32, Vec<u32>) {
        let Some(object) = self.lookup_object(pid, req.session_handle) else {
            return (RESULT_INVALID_HANDLE, vec![]);
        };
        let KernelObject::Session(target) = object else {
            return (RESULT_INVALID_HANDLE, vec![]);
        };

        match target {
            ServiceTarget::Srv => self.dispatch_srv(pid, req.message),
            ServiceTarget::Fs => self.dispatch_fs(pid, req.message),
            ServiceTarget::Apt => self.dispatch_apt(req.message),
            ServiceTarget::GspGpu => self.dispatch_gsp(req.message),
            ServiceTarget::HidUser => self.dispatch_hid(req.message),
        }
    }

    fn dispatch_srv(&mut self, pid: ProcessId, msg: IpcMessage) -> (u32, Vec<u32>) {
        match SrvCommand::from_id(msg.command_id) {
            Some(SrvCommand::RegisterService) => {
                if msg.normal_words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let name = service_name_from_words(&msg.normal_words[..2]);
                let max_sessions = msg.normal_words[2];
                self.registry
                    .register(&name, ServiceTarget::Srv, max_sessions);
                let port = self.register_service_port(pid, &name, max_sessions);
                (RESULT_OK, vec![port])
            }
            Some(SrvCommand::GetServiceHandle) => {
                if msg.normal_words.len() < 2 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let name = service_name_from_words(&msg.normal_words[..2]);
                match self.connect_to_service(pid, &name) {
                    Some(handle) => (RESULT_OK, vec![handle]),
                    None => (RESULT_NOT_FOUND, vec![]),
                }
            }
            None => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_fs(&mut self, pid: ProcessId, msg: IpcMessage) -> (u32, Vec<u32>) {
        match FsCommand::from_id(msg.command_id) {
            Some(FsCommand::OpenArchive) => {
                let archive_id = msg
                    .normal_words
                    .first()
                    .copied()
                    .unwrap_or(ArchiveId::Sdmc as u32);
                let Some(archive_obj) = self.vfs.open_archive(archive_id) else {
                    return (RESULT_NOT_FOUND, vec![]);
                };
                let archive = self.allocate_handle(pid, KernelObject::Archive(archive_obj));
                (RESULT_OK, vec![archive])
            }
            Some(FsCommand::OpenFile) => {
                if msg.normal_words.len() < 2 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let archive_handle = msg.normal_words[0];
                let Some(KernelObject::Archive(archive_obj)) =
                    self.lookup_object(pid, archive_handle)
                else {
                    return (RESULT_INVALID_HANDLE, vec![]);
                };
                let translated = self.vfs.translate_path(
                    archive_obj.archive,
                    &format!("/{:08x}", msg.normal_words[1]),
                );
                let Some(file_obj) = self.vfs.open_file(archive_obj, &translated) else {
                    if archive_obj.archive == ArchiveId::RomFs {
                        return (RESULT_NOT_FOUND, vec![]);
                    }
                    let fallback_path = format!("/{:08x}", msg.normal_words[1]);
                    let file_obj = self
                        .vfs
                        .open_file(archive_obj, &fallback_path)
                        .expect("fallback open");
                    let file = self.allocate_handle(pid, KernelObject::File(file_obj));
                    return (RESULT_OK, vec![file]);
                };
                let file = self.allocate_handle(pid, KernelObject::File(file_obj));
                (RESULT_OK, vec![file])
            }
            Some(FsCommand::ReadFile) => {
                if msg.normal_words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                let file_handle = msg.normal_words[0];
                let offset = msg.normal_words[1] as usize;
                let size = msg.normal_words[2] as usize;
                let Some(KernelObject::File(file_obj)) = self.lookup_object(pid, file_handle)
                else {
                    return (RESULT_INVALID_HANDLE, vec![]);
                };
                let Some(data) = self.vfs.read_file(&file_obj, offset, size) else {
                    return (RESULT_NOT_FOUND, vec![]);
                };
                (RESULT_OK, vec![data.len() as u32])
            }
            None => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_apt(&mut self, msg: IpcMessage) -> (u32, Vec<u32>) {
        match AptCommand::from_id(msg.command_id) {
            Some(AptCommand::GetAppState) => (RESULT_OK, vec![self.app_state]),
            Some(AptCommand::SetAppState) => {
                self.app_state = msg.normal_words.first().copied().unwrap_or(self.app_state);
                (RESULT_OK, vec![self.app_state])
            }
            None => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_gsp(&mut self, msg: IpcMessage) -> (u32, Vec<u32>) {
        match GspCommand::from_id(msg.command_id) {
            Some(GspCommand::SetFrameClearColor) => {
                let color = msg.normal_words.first().copied().unwrap_or(0xFF00_0000);
                self.gpu_handoff.push_back(vec![
                    PicaCommandBufferPacket::encode(0x0200, 1, false),
                    color,
                ]);
                (RESULT_OK, vec![])
            }
            Some(GspCommand::QueuePixel) => {
                if msg.normal_words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                self.gpu_handoff.push_back(vec![
                    PicaCommandBufferPacket::encode(0x0201, 1, false),
                    (msg.normal_words[1] << 16) | (msg.normal_words[0] & 0xFFFF),
                    PicaCommandBufferPacket::encode(0x0202, 1, false),
                    msg.normal_words[2],
                ]);
                (RESULT_OK, vec![])
            }
            None => (RESULT_INVALID_COMMAND, vec![]),
        }
    }

    fn dispatch_hid(&mut self, msg: IpcMessage) -> (u32, Vec<u32>) {
        match HidCommand::from_id(msg.command_id) {
            Some(HidCommand::Initialize) => {
                self.hid_state = HidInputState::default();
                (RESULT_OK, vec![1])
            }
            Some(HidCommand::GetInputState) => (
                RESULT_OK,
                vec![
                    self.hid_state.buttons,
                    u32::from(self.hid_state.touch_x),
                    u32::from(self.hid_state.touch_y),
                ],
            ),
            Some(HidCommand::SetInputState) => {
                if msg.normal_words.len() < 3 {
                    return (RESULT_INVALID_COMMAND, vec![]);
                }
                self.hid_state.buttons = msg.normal_words[0];
                self.hid_state.touch_x = msg.normal_words[1] as u16;
                self.hid_state.touch_y = msg.normal_words[2] as u16;
                (RESULT_OK, vec![])
            }
            None => (RESULT_INVALID_COMMAND, vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ipc::{IpcMessage, KernelObjectType, service_name_words};

    fn mk_command(command_id: u16, payload: &[u32]) -> Vec<u32> {
        IpcMessage {
            command_id,
            normal_words: payload.to_vec(),
            descriptors: vec![],
        }
        .into_words()
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
        let fs_name = service_name_words("fs:USER");
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

        let apt_name = service_name_words("apt:u");
        kernel.queue_ipc_command(pid, srv, mk_command(0x0005, &apt_name));
        kernel.pump_ipc_events(1);
        let apt = kernel.pop_ipc_response(pid).expect("apt").words[0];

        kernel.queue_ipc_command(pid, apt, mk_command(0x0001, &[]));
        kernel.pump_ipc_events(1);
        let app_state = kernel.pop_ipc_response(pid).expect("app state");
        assert_eq!(app_state.result_code, RESULT_OK);
        assert_eq!(app_state.words[0], 1);
    }

    #[test]
    fn handle_validity_duplicate_and_close() {
        let mut kernel = Kernel::new();
        let pid_a = 100;
        let pid_b = 101;
        kernel.ensure_process(pid_a);
        kernel.ensure_process(pid_b);

        let event = kernel.create_event(pid_a, "unit:event");
        assert_eq!(
            kernel.handle_type(pid_a, event),
            Some(KernelObjectType::Event)
        );
        assert_eq!(kernel.handle_type(pid_b, event), None);

        let dup = kernel
            .duplicate_handle(pid_a, event)
            .expect("duplicate event");
        assert_ne!(dup, event);
        assert_eq!(
            kernel.handle_type(pid_a, dup),
            Some(KernelObjectType::Event)
        );

        assert!(kernel.close_handle(pid_a, event));
        assert_eq!(kernel.handle_type(pid_a, event), None);
        assert_eq!(
            kernel.handle_type(pid_a, dup),
            Some(KernelObjectType::Event)
        );
    }

    #[test]
    fn service_session_lifecycle() {
        let mut kernel = Kernel::new();
        let pid = 77;
        kernel.ensure_process(pid);

        let srv = kernel.connect_to_service(pid, "srv:").expect("srv session");
        assert_eq!(
            kernel.handle_type(pid, srv),
            Some(KernelObjectType::Session)
        );

        assert!(kernel.close_handle(pid, srv));
        assert_eq!(kernel.handle_type(pid, srv), None);

        kernel.queue_ipc_command(pid, srv, mk_command(0x0005, &service_name_words("fs:USER")));
        kernel.pump_ipc_events(1);
        let failed = kernel.pop_ipc_response(pid).expect("response after close");
        assert_eq!(failed.result_code, RESULT_INVALID_HANDLE);
    }

    #[test]
    fn hid_user_tracks_minimal_input_state() {
        let mut kernel = Kernel::new();
        let pid = 91;
        kernel.ensure_process(pid);

        let srv = kernel.connect_to_service(pid, "srv:").expect("srv session");
        kernel.queue_ipc_command(
            pid,
            srv,
            mk_command(0x0005, &service_name_words("hid:USER")),
        );
        kernel.pump_ipc_events(1);
        let hid = kernel.pop_ipc_response(pid).expect("hid handle").words[0];

        kernel.queue_ipc_command(pid, hid, mk_command(0x0001, &[]));
        kernel.queue_ipc_command(pid, hid, mk_command(0x000B, &[0x12, 320, 120]));
        kernel.queue_ipc_command(pid, hid, mk_command(0x000A, &[]));
        kernel.pump_ipc_events(3);
        assert_eq!(
            kernel.pop_ipc_response(pid).expect("init").result_code,
            RESULT_OK
        );
        assert_eq!(
            kernel.pop_ipc_response(pid).expect("set").result_code,
            RESULT_OK
        );
        let state = kernel.pop_ipc_response(pid).expect("get");
        assert_eq!(state.result_code, RESULT_OK);
        assert_eq!(state.words, vec![0x12, 320, 120]);
    }

    #[test]
    fn boot_sequence_replay_for_target_title_services() {
        let mut kernel = Kernel::new();
        let pid = 123;
        kernel.ensure_process(pid);

        let srv = kernel.connect_to_service(pid, "srv:").expect("srv session");
        let service_names = ["fs:USER", "apt:u", "gsp::Gpu", "hid:USER"];
        let mut handles = HashMap::new();

        for service in service_names {
            kernel.queue_ipc_command(pid, srv, mk_command(0x0005, &service_name_words(service)));
            kernel.pump_ipc_events(1);
            let response = kernel
                .pop_ipc_response(pid)
                .expect("service handle response");
            assert_eq!(response.result_code, RESULT_OK);
            handles.insert(service, response.words[0]);
        }

        let fs = handles["fs:USER"];
        kernel.queue_ipc_command(pid, fs, mk_command(0x0001, &[0]));
        kernel.pump_ipc_events(1);
        let archive = kernel.pop_ipc_response(pid).expect("archive response");
        assert_eq!(archive.result_code, RESULT_OK);

        let archive_handle = archive.words[0];
        kernel.queue_ipc_command(pid, fs, mk_command(0x0002, &[archive_handle, 0x2000]));
        kernel.pump_ipc_events(1);
        let file = kernel.pop_ipc_response(pid).expect("file response");
        assert_eq!(file.result_code, RESULT_OK);

        let file_handle = file.words[0];
        kernel.queue_ipc_command(pid, fs, mk_command(0x0003, &[file_handle, 0, 0x40]));

        let apt = handles["apt:u"];
        kernel.queue_ipc_command(pid, apt, mk_command(0x0001, &[]));

        let gsp = handles["gsp::Gpu"];
        kernel.queue_ipc_command(pid, gsp, mk_command(0x0001, &[0xFF00_FF00]));

        let hid = handles["hid:USER"];
        kernel.queue_ipc_command(pid, hid, mk_command(0x0001, &[]));
        kernel.queue_ipc_command(pid, hid, mk_command(0x000A, &[]));

        kernel.pump_ipc_events(5);
        for _ in 0..5 {
            let response = kernel.pop_ipc_response(pid).expect("ordered response");
            assert_eq!(response.result_code, RESULT_OK);
        }
    }
}
