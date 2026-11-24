//! AWC audio containers (`ADAT` magic): stream table, chunk table and stream formats.
//!
//! Header, little-endian, offsets from the magic:
//! `ADAT`, u16 version, u16 flags, u32 stream count, u32 header size, then the
//! optional tables chosen by flags (0x40: one u32; 0x01: u16 per stream;
//! 0x20: u32 per stream plus one), one u32 per stream (id in bits 0..28, chunk
//! count in bits 29..31) and one u64 per chunk (offset bits 0..27, size bits
//! 28..55, tag bits 56..63).

use anyhow::{bail, Result};
use super::{u16_at, u32_at, u64_at};

pub const TAG_DATA: u8 = 0x55;
pub const TAG_FORMAT: u8 = 0x76;
pub const TAG_MULTI_FORMAT: u8 = 0x48;
pub const TAG_SEEK: u8 = 0xA3;
pub const TAG_PEAK: u8 = 0x36;

#[derive(Debug, Clone)]
pub struct Awc {
    pub version: u16,
    pub flags: u16,
    pub header_size: u32,
    pub streams: Vec<Stream>,
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub id: u32,
    pub chunks: Vec<Chunk>,
    pub format: Option<Format>,
}

#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub tag: u8,
    pub offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Format {
    /// Sample count, all channels together.
    pub samples: u32,
    /// The stored rate field; Opus data always runs at 48 kHz regardless.
    pub sample_rate: u16,
    pub codec: u8,
    pub channels: u8,
}

pub fn codec_name(codec: u8) -> &'static str {
    match codec {
        0x00 => "pcm16",
        0x02 => "pcm-f32",
        0x05 => "xma2",
        0x0B => "opus",
        _ => "unknown",
    }
}

pub fn tag_name(tag: u8) -> &'static str {
    match tag {
        TAG_DATA => "data",
        TAG_FORMAT => "format",
        TAG_MULTI_FORMAT => "multichannel-format",
        TAG_SEEK => "seek",
        TAG_PEAK => "peak",
        _ => "other",
    }
}

impl Stream {
    pub fn chunk(&self, tag: u8) -> Option<&Chunk> {
        self.chunks.iter().find(|c| c.tag == tag)
    }
}

pub fn parse(data: &[u8]) -> Result<Awc> {
    if !data.starts_with(b"ADAT") { bail!("not an AWC container (no ADAT magic)"); }
    let (Some(version), Some(flags), Some(count), Some(header_size)) =
        (u16_at(data, 4), u16_at(data, 6), u32_at(data, 8), u32_at(data, 12))
    else { bail!("truncated AWC header"); };
    let n = count as usize;
    if n > data.len() / 4 { bail!("implausible stream count {n}"); }

    let mut at = 0x10;
    if flags & 0x40 != 0 { at += 4; }
    if flags & 0x01 != 0 { at += 2 * n; }
    if flags & 0x20 != 0 { at += 4 * n + 4; }

    let mut streams = Vec::with_capacity(n);
    let mut chunk_at = at + 4 * n;
    for i in 0..n {
        let Some(info) = u32_at(data, at + 4 * i) else { bail!("truncated stream table"); };
        let mut chunks = Vec::new();
        for _ in 0..info >> 29 {
            let Some(word) = u64_at(data, chunk_at) else { bail!("truncated chunk table"); };
            chunks.push(Chunk {
                tag: (word >> 56) as u8,
                offset: (word & 0x0FFF_FFFF) as u32,
                size: ((word >> 28) & 0x0FFF_FFFF) as u32,
            });
            chunk_at += 8;
        }
        streams.push(Stream { id: info & 0x1FFF_FFFF, chunks, format: None });
    }
    if chunk_at != header_size as usize {
        bail!("AWC tables end at {chunk_at:#x}, header says {header_size:#x}");
    }

    // Single-stream formats live in the stream's own 0x76 chunk; a multichannel
    // stream's 0x48 chunk describes its chunkless sibling streams by id.
    let mut multi = Vec::new();
    for s in &mut streams {
        if let Some(c) = s.chunk(TAG_FORMAT) {
            let o = c.offset as usize;
            if let (Some(samples), Some(rate), Some(&[codec, channels])) =
                (u32_at(data, o), u16_at(data, o + 8), data.get(o + 0x1C..o + 0x1E))
            {
                s.format = Some(Format { samples, sample_rate: rate, codec, channels });
            }
        }
        if let Some(c) = s.chunk(TAG_MULTI_FORMAT) {
            let o = c.offset as usize;
            let channels = u32_at(data, o + 8).unwrap_or(0) as usize;
            for ch in 0..channels.min(64) {
                let e = o + 12 + 16 * ch;
                if let (Some(id), Some(samples), Some(rate), Some(&[codec, chans])) =
                    (u32_at(data, e), u32_at(data, e + 4), u16_at(data, e + 10), data.get(e + 12..e + 14))
                {
                    multi.push((id & 0x1FFF_FFFF, Format { samples, sample_rate: rate, codec, channels: chans }));
                }
            }
        }
    }
    for (id, format) in multi {
        if let Some(s) = streams.iter_mut().find(|s| s.id == id && s.format.is_none()) {
            s.format = Some(format);
        }
    }

    Ok(Awc { version, flags, header_size, streams })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: u8, offset: u64, size: u64) -> [u8; 8] {
        ((u64::from(tag) << 56) | (size << 28) | offset).to_le_bytes()
    }

    #[test]
    fn parses_single_opus_stream() {
        let mut d = Vec::new();
        d.extend_from_slice(b"ADAT");
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&0xFF00u16.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&36u32.to_le_bytes());
        d.extend_from_slice(&(0x0ABCDEFu32 | (2 << 29)).to_le_bytes());
        d.extend_from_slice(&chunk(TAG_FORMAT, 36, 0x20));
        d.extend_from_slice(&chunk(TAG_DATA, 0x800, 0x1000));
        let mut fmt = [0u8; 0x20];
        fmt[0..4].copy_from_slice(&2000u32.to_le_bytes());
        fmt[8..10].copy_from_slice(&48000u16.to_le_bytes());
        fmt[0x1C] = 0x0B;
        fmt[0x1D] = 1;
        d.extend_from_slice(&fmt);

        let awc = parse(&d).unwrap();
        assert_eq!(awc.streams.len(), 1);
        let s = &awc.streams[0];
        assert_eq!(s.id, 0x0ABCDEF);
        assert_eq!(s.chunk(TAG_DATA).unwrap().size, 0x1000);
        let f = s.format.unwrap();
        assert_eq!((f.samples, f.sample_rate, codec_name(f.codec), f.channels), (2000, 48000, "opus", 1));

        d[12] = 44; // header size no longer matches the tables
        assert!(parse(&d).is_err());
    }
}
