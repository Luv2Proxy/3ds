use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmulatorError {
    RomTooSmall,
    InvalidRomMagic,
    RomTooLarge { size: usize, capacity: usize },
    MemoryOutOfBounds { address: u32 },
    InvalidInstruction { pc: u32, opcode: u32 },
    RomNotLoaded,
    InvalidTitlePackage,
    InvalidRomLayout,
    InvalidExHeader,
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
        }
    }
}

impl std::error::Error for EmulatorError {}

pub type Result<T> = std::result::Result<T, EmulatorError>;
