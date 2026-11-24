//! Paged resources (`0DRP` / PRD0 wrapper and plain virtual segments).
//!
//! A resource is a virtual segment of objects linked by 64-bit pointers
//! `0x5000_0000 + offset`, plus a physical segment of GPU buffers addressed by
//! `0x6000_0000 + offset`. PSO-backed resources carry `0DRP` at +0x10 after a
//! zero u64; some payloads start with `0DRP` directly.

use anyhow::{bail, Result};
use super::{u32_at, u64_at};

pub const VIRTUAL_BASE: u64 = 0x5000_0000;
pub const PHYSICAL_BASE: u64 = 0x6000_0000;
const SEGMENT_SPAN: u64 = 0x1000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `0DRP` magic: PSO-backed paged resource.
    Prd0,
    /// Zero u64 followed by a virtual pointer: other paged resource.
    Paged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Virtual,
    Physical,
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub kind: Kind,
    /// Offset of the `0DRP` magic, when present.
    pub magic_at: Option<usize>,
    /// The u32 after the magic (version / type word).
    pub version: Option<u32>,
    pub virtual_ptrs: usize,
    pub physical_ptrs: usize,
}

/// Splits a resource pointer into its segment and byte offset.
pub fn resolve(ptr: u64) -> Option<(Segment, u64)> {
    match ptr {
        p if (VIRTUAL_BASE..VIRTUAL_BASE + SEGMENT_SPAN).contains(&p) => Some((Segment::Virtual, p - VIRTUAL_BASE)),
        p if (PHYSICAL_BASE..PHYSICAL_BASE + SEGMENT_SPAN).contains(&p) => Some((Segment::Physical, p - PHYSICAL_BASE)),
        _ => None,
    }
}

pub fn detect(data: &[u8]) -> Option<(Kind, Option<usize>)> {
    if data.starts_with(b"0DRP") { return Some((Kind::Prd0, Some(0))); }
    if u64_at(data, 0)? != 0 { return None; }
    if data.get(0x10..0x14) == Some(b"0DRP") { return Some((Kind::Prd0, Some(0x10))); }
    matches!(resolve(u64_at(data, 8)?), Some((Segment::Virtual, _))).then_some((Kind::Paged, None))
}

pub fn parse(data: &[u8]) -> Result<Resource> {
    let Some((kind, magic_at)) = detect(data) else { bail!("not a paged resource"); };
    // ponytail: counts every aligned u64 in range, so payload bytes can false-positive; walk typed objects for exact counts.
    let (mut virtual_ptrs, mut physical_ptrs) = (0, 0);
    for chunk in data.chunks_exact(8) {
        match resolve(u64::from_le_bytes(chunk.try_into().unwrap())) {
            Some((Segment::Virtual, _)) => virtual_ptrs += 1,
            Some((Segment::Physical, _)) => physical_ptrs += 1,
            None => {}
        }
    }
    Ok(Resource { kind, magic_at, version: magic_at.and_then(|m| u32_at(data, m + 4)), virtual_ptrs, physical_ptrs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_counts_pointers() {
        let mut d = vec![0u8; 0x20];
        d[0x10..0x14].copy_from_slice(b"0DRP");
        d[0x14..0x18].copy_from_slice(&0x7Au32.to_le_bytes());
        d.extend_from_slice(&0x5000_0040u64.to_le_bytes());
        d.extend_from_slice(&0x6000_0000u64.to_le_bytes());
        let r = parse(&d).unwrap();
        assert_eq!((r.kind, r.magic_at, r.version), (Kind::Prd0, Some(0x10), Some(0x7A)));
        assert_eq!((r.virtual_ptrs, r.physical_ptrs), (1, 1));
        assert_eq!(resolve(0x5000_0040), Some((Segment::Virtual, 0x40)));
        assert!(parse(&[1u8; 16]).is_err());
    }
}
