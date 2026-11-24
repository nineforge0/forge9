use anyhow::{bail, Result};

pub const MAGIC: u32 = 0x52504639; // "9FPR" LE
pub const HEADER_SIZE: usize = 0x40;

#[derive(Debug, Clone)]
pub struct Header {
    pub magic: u32,
    pub payload_start: u32,
    pub split: u32,
    pub trailer_size: u32,
    pub archive_id: u64,
    pub gen_word: u32,
    pub gen_const: u32,
    pub version_low: u16,
    pub version_high: u16,
    pub entry_count: u32,
    pub key_id: u32,
    pub flags: u32,
    pub alignment: u32,
}

impl Header {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            bail!("data too short for header ({} < {HEADER_SIZE})", data.len());
        }
        let r = |off: usize| u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        let magic = r(0x00);
        if magic != MAGIC {
            bail!("bad magic: {magic:#x} (expected {MAGIC:#x})");
        }
        Ok(Self {
            magic,
            payload_start: r(0x04),
            split: r(0x08),
            trailer_size: r(0x0C),
            archive_id: u64::from_le_bytes(data[0x10..0x18].try_into().unwrap()),
            gen_word: r(0x18),
            gen_const: r(0x1C),
            version_low: u16::from_le_bytes(data[0x20..0x22].try_into().unwrap()),
            version_high: u16::from_le_bytes(data[0x22..0x24].try_into().unwrap()),
            entry_count: r(0x24),
            key_id: r(0x28),
            flags: r(0x2C),
            alignment: r(0x30),
        })
    }

    pub fn generation(&self) -> u32 {
        self.gen_word >> 16
    }

    pub fn entry_stride(&self) -> usize {
        8 + (self.flags & 0xFF) as usize
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0x00..0x04].copy_from_slice(&self.magic.to_le_bytes());
        buf[0x04..0x08].copy_from_slice(&self.payload_start.to_le_bytes());
        buf[0x08..0x0C].copy_from_slice(&self.split.to_le_bytes());
        buf[0x0C..0x10].copy_from_slice(&self.trailer_size.to_le_bytes());
        buf[0x10..0x18].copy_from_slice(&self.archive_id.to_le_bytes());
        buf[0x18..0x1C].copy_from_slice(&self.gen_word.to_le_bytes());
        buf[0x1C..0x20].copy_from_slice(&self.gen_const.to_le_bytes());
        buf[0x20..0x22].copy_from_slice(&self.version_low.to_le_bytes());
        buf[0x22..0x24].copy_from_slice(&self.version_high.to_le_bytes());
        buf[0x24..0x28].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[0x28..0x2C].copy_from_slice(&self.key_id.to_le_bytes());
        buf[0x2C..0x30].copy_from_slice(&self.flags.to_le_bytes());
        buf[0x30..0x34].copy_from_slice(&self.alignment.to_le_bytes());
        buf
    }
}
