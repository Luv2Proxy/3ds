use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAccessKind {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmulatorError {
    RomTooSmall,
    InvalidRomMagic,
    RomTooLarge {
        size: usize,
        capacity: usize,
    },
    MemoryOutOfBounds {
        address: u32,
    },
    InvalidInstruction {
        pc: u32,
        opcode: u32,
    },
    RomNotLoaded,
    InvalidTitlePackage,
    InvalidRomLayout,
    InvalidExHeader,
    MmuTranslationFault {
        pc: u32,
        va: u32,
        pa: Option<u32>,
        access: MemoryAccessKind,
    },
    MmuDomainFault {
        pc: u32,
        va: u32,
        pa: Option<u32>,
        domain: u8,
        access: MemoryAccessKind,
    },
    MmuPermissionFault {
        pc: u32,
        va: u32,
        pa: Option<u32>,
        access: MemoryAccessKind,
    },
    AlignmentFault {
        pc: u32,
        va: u32,
        pa: Option<u32>,
        access: MemoryAccessKind,
    },
    ServiceCallError {
        pc: u32,
        service_command_id: u16,
        handle_id: u32,
        result_code: u32,
    },
}

impl Display for EmulatorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RomTooSmall => write!(f, "ROM is too small to contain an NCSD header"),
            Self::InvalidRomMagic => write!(f, "invalid ROM magic; expected NCSD at 0x100"),
            Self::RomTooLarge { size, capacity } => {
                write!(f, "ROM size {size} exceeds memory capacity {capacity}")
            }
            Self::MemoryOutOfBounds { address } => {
                write!(f, "memory access out of bounds at 0x{address:08x}")
            }
            Self::InvalidInstruction { pc, opcode } => {
                write!(f, "invalid instruction 0x{opcode:08x} at PC=0x{pc:08x}")
            }
            Self::RomNotLoaded => write!(f, "cannot execute because no ROM is loaded"),
            Self::InvalidTitlePackage => write!(f, "invalid title package format"),
            Self::InvalidRomLayout => write!(f, "invalid NCSD/NCCH ROM layout"),
            Self::InvalidExHeader => write!(f, "invalid NCCH exheader format"),
            Self::MmuTranslationFault { pc, va, pa, access } => {
                write!(
                    f,
                    "MMU translation fault at PC=0x{pc:08x}, VA=0x{va:08x}, PA={pa:?} ({access:?})"
                )
            }
            Self::MmuDomainFault {
                pc,
                va,
                pa,
                domain,
                access,
            } => write!(
                f,
                "MMU domain fault at PC=0x{pc:08x}, VA=0x{va:08x}, PA={pa:?}, domain={domain} ({access:?})"
            ),
            Self::MmuPermissionFault { pc, va, pa, access } => {
                write!(
                    f,
                    "MMU permission fault at PC=0x{pc:08x}, VA=0x{va:08x}, PA={pa:?} ({access:?})"
                )
            }
            Self::AlignmentFault { pc, va, pa, access } => {
                write!(
                    f,
                    "alignment fault at PC=0x{pc:08x}, VA=0x{va:08x}, PA={pa:?} ({access:?})"
                )
            }
            Self::ServiceCallError {
                pc,
                service_command_id,
                handle_id,
                result_code,
            } => {
                write!(
                    f,
                    "service call failure at PC=0x{pc:08x}, cmd=0x{service_command_id:04x}, handle={handle_id}, result=0x{result_code:08x}"
                )
            }
        }
    }
}

impl std::error::Error for EmulatorError {}

pub type Result<T> = std::result::Result<T, EmulatorError>;
