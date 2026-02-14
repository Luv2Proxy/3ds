use std::fmt;

pub type Handle = u32;
pub type ProcessId = u32;

pub const RESULT_OK: u32 = 0;
pub const RESULT_NOT_FOUND: u32 = 0xD8A1_83F8;
pub const RESULT_INVALID_HANDLE: u32 = 0xD8A1_83FA;
pub const RESULT_INVALID_COMMAND: u32 = 0xD8A1_8404;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcPort {
    pub name: String,
    pub max_sessions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcSession {
    pub service: String,
    pub server_port: Handle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandHeader {
    pub command_id: u16,
    pub normal_words: u16,
    pub translate_words: u8,
}

impl CommandHeader {
    pub fn parse(raw: u32) -> Self {
        Self {
            command_id: (raw & 0xFFFF) as u16,
            normal_words: ((raw >> 16) & 0x3FF) as u16,
            translate_words: ((raw >> 26) & 0x3F) as u8,
        }
    }

    pub fn encode(self) -> u32 {
        u32::from(self.command_id)
            | (u32::from(self.normal_words) << 16)
            | (u32::from(self.translate_words) << 26)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBuffer {
    pub header: CommandHeader,
    pub words: Vec<u32>,
}

impl CommandBuffer {
    pub fn parse(raw_words: &[u32]) -> Option<Self> {
        let (&first, tail) = raw_words.split_first()?;
        let header = CommandHeader::parse(first);
        if tail.len() < usize::from(header.normal_words) {
            return None;
        }
        let count = usize::from(header.normal_words);
        Some(Self {
            header,
            words: tail[..count].to_vec(),
        })
    }

    pub fn into_words(self) -> Vec<u32> {
        let mut out = Vec::with_capacity(self.words.len() + 1);
        out.push(
            CommandHeader {
                normal_words: self.words.len() as u16,
                ..self.header
            }
            .encode(),
        );
        out.extend(self.words);
        out
    }
}

pub fn service_name_from_words(words: &[u32]) -> String {
    let mut bytes = [0_u8; 8];
    if let Some(w0) = words.first().copied() {
        bytes[..4].copy_from_slice(&w0.to_le_bytes());
    }
    if let Some(w1) = words.get(1).copied() {
        bytes[4..8].copy_from_slice(&w1.to_le_bytes());
    }
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).to_string()
}

pub fn service_name_words(name: &str) -> [u32; 2] {
    let mut bytes = [0_u8; 8];
    let src = name.as_bytes();
    let len = src.len().min(8);
    bytes[..len].copy_from_slice(&src[..len]);
    [
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    ]
}

impl fmt::Display for CommandHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cmd=0x{:04X} normal={} translate={}",
            self.command_id, self.normal_words, self.translate_words
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_header_roundtrip() {
        let hdr = CommandHeader {
            command_id: 0x501,
            normal_words: 3,
            translate_words: 1,
        };
        let raw = hdr.encode();
        assert_eq!(CommandHeader::parse(raw), hdr);
    }

    #[test]
    fn command_buffer_parse_enforces_normal_count() {
        assert!(CommandBuffer::parse(&[0x0002_0001, 1]).is_none());
        let cmd = CommandBuffer::parse(&[0x0002_0001, 1, 2]).expect("valid parse");
        assert_eq!(cmd.header.command_id, 1);
        assert_eq!(cmd.words, vec![1, 2]);
    }
}
