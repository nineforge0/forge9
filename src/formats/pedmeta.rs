//! Ped metadata tables stored as PSIN data blocks beside the character packs.
//! Layout checked on real files in docs/drawable_format.md ("Ped metadata").
//!
//! - PSIN data: `PSIN`, u32 big-endian size, then a block table of 16-byte rows
//!   `{u32 struct hash, u32 size, 8 x 0xaa}` ended by 8 x 0x70; block data follows,
//!   each block starting on a 16-byte boundary.
//! - Hash maps are stored as nodes of 16 control bytes then 16 entries; a control
//!   byte with the top bit set marks an empty slot.
//! - Part table (block hash 0x101, 64-byte entries, one per drawable of the pack):
//!   `+0x00 u64 id, +0x08 properties map (24 bytes), +0x20 u64 id again,
//!   +0x28 u64, +0x30 u64, +0x38 u32, +0x3C u8 kind`.
//! - Texture variation records (struct 0x37e52277, 184 bytes): `+0x00 u32 variation
//!   name hash, +0x08 u64 part id, +0x10 u64 diffuse or region-map key, +0x18 u64
//!   normal key, +0x20 u64`; keys are `.ttpf` store keys.

use anyhow::{bail, ensure, Result};
use super::{u32_at, u64_at};

/// Struct hash of texture variation records.
pub const VARIATION_STRUCT: u32 = 0x37e5_2277;
pub const VARIATION_SIZE: usize = 184;
/// Struct hash of the hash-map node blocks that hold the part table.
pub const MAP_NODE_STRUCT: u32 = 0x101;
pub const PART_SIZE: usize = 64;
const NODE_SLOTS: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub hash: u32,
    pub offset: usize,
    pub size: usize,
}

/// Data blocks of a PSIN payload.
pub fn blocks(psin: &[u8]) -> Result<Vec<Block>> {
    ensure!(psin.starts_with(b"PSIN"), "not a PSIN payload");
    let mut rows = Vec::new();
    let mut at = 8;
    loop {
        let Some(row) = psin.get(at..at + 16) else { bail!("PSIN block table runs past the data") };
        if row[..8] == [0x70; 8] { break; }
        ensure!(row[8..] == [0xaa; 8], "PSIN block table row at {at:#x} has no 0xaa padding");
        rows.push((u32_at(row, 0).unwrap(), u32_at(row, 4).unwrap() as usize));
        at += 16;
    }
    let mut pos = at + 8;
    rows.into_iter().map(|(hash, size)| {
        pos = pos.next_multiple_of(16);
        let b = Block { hash, offset: pos, size };
        ensure!(pos + size <= psin.len(), "PSIN block {hash:#x} at {pos:#x}+{size:#x} past the data");
        pos += size;
        Ok(b)
    }).collect()
}

/// Offsets of the occupied entries of a hash map stored as nodes in `block`.
pub fn occupied(psin: &[u8], block: &Block, entry_size: usize) -> Vec<usize> {
    let node = NODE_SLOTS + NODE_SLOTS * entry_size;
    let mut out = Vec::new();
    for n in 0..block.size / node {
        let base = block.offset + n * node;
        for k in 0..NODE_SLOTS {
            if psin.get(base + k).is_some_and(|c| c & 0x80 == 0) { out.push(base + NODE_SLOTS + k * entry_size); }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub id: u64,
    pub kind: u8,
    pub extra: [u64; 2],
}

/// The part table: the first map-node block whose entries repeat their key at +0x20.
pub fn parts(psin: &[u8]) -> Result<Vec<Part>> {
    let node = NODE_SLOTS + NODE_SLOTS * PART_SIZE;
    for b in blocks(psin)?.iter().filter(|b| b.hash == MAP_NODE_STRUCT && b.size > 0 && b.size % node == 0) {
        let entries = occupied(psin, b, PART_SIZE);
        let read = |e: usize, o: usize| u64_at(psin, e + o).unwrap_or(0);
        if entries.is_empty() || !entries.iter().all(|&e| read(e, 0) == read(e, 0x20)) { continue; }
        return Ok(entries.iter().map(|&e| Part { id: read(e, 0), kind: psin[e + 0x3C], extra: [read(e, 0x28), read(e, 0x30)] }).collect());
    }
    bail!("no part table (map nodes of {PART_SIZE}-byte entries keyed at +0x00 and +0x20)")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variation {
    pub variation: u32,
    pub part: u64,
    pub diffuse: u64,
    pub normal: u64,
    pub extra: u64,
}

/// Every texture variation record (blocks of struct 0x37e52277); records with a zero part id are skipped.
pub fn variations(psin: &[u8]) -> Result<Vec<Variation>> {
    let mut out = Vec::new();
    for b in blocks(psin)?.iter().filter(|b| b.hash == VARIATION_STRUCT) {
        for at in (b.offset..b.offset + b.size - b.size % VARIATION_SIZE).step_by(VARIATION_SIZE) {
            let q = |o| u64_at(psin, at + o).unwrap_or(0);
            if q(8) == 0 { continue; }
            out.push(Variation { variation: u32_at(psin, at).unwrap_or(0), part: q(8), diffuse: q(16), normal: q(24), extra: q(32) });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(buf: &mut Vec<u8>, at: usize, bytes: &[u8]) {
        if buf.len() < at + bytes.len() { buf.resize(at + bytes.len(), 0); }
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// PSIN payload with the given (hash, data) blocks.
    fn psin(blocks: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut d = b"PSIN\0\0\0\0".to_vec();
        for (h, data) in blocks {
            d.extend(h.to_le_bytes());
            d.extend((data.len() as u32).to_le_bytes());
            d.extend([0xaa; 8]);
        }
        d.extend([0x70; 8]);
        for (_, data) in blocks {
            d.resize(d.len().next_multiple_of(16), 0x80);
            d.extend(data);
        }
        d
    }

    #[test]
    fn reads_aligned_blocks() {
        let d = psin(&[(7, vec![1; 5]), (9, vec![2; 3])]);
        let b = blocks(&d).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].hash, b[0].size, b[0].offset % 16), (7, 5, 0));
        assert_eq!(b[1].offset, (b[0].offset + 5).next_multiple_of(16));
        assert!(d[b[1].offset..b[1].offset + 3].iter().all(|&x| x == 2));
        assert!(blocks(b"PSIX").is_err());
    }

    #[test]
    fn reads_part_table_from_map_nodes() {
        let mut node = vec![0x80u8; 16];
        node[0] = 0x12;
        node[3] = 0x05;
        node.resize(16 + 16 * PART_SIZE, 0);
        for (slot, id, kind) in [(0usize, 0x1111_2222_3333_4444u64, 62u8), (3, 0x5555_6666_7777_8888, 23)] {
            let e = 16 + slot * PART_SIZE;
            put(&mut node, e, &id.to_le_bytes());
            put(&mut node, e + 0x20, &id.to_le_bytes());
            put(&mut node, e + 0x30, &0xabcdu64.to_le_bytes());
            node[e + 0x3C] = kind;
        }
        // A slot whose control byte says empty is ignored even if it holds data.
        put(&mut node, 16 + 5 * PART_SIZE, &9u64.to_le_bytes());
        let d = psin(&[(0x4a, vec![0; 8]), (MAP_NODE_STRUCT, node)]);
        let p = parts(&d).unwrap();
        assert_eq!(p, vec![
            Part { id: 0x1111_2222_3333_4444, kind: 62, extra: [0, 0xabcd] },
            Part { id: 0x5555_6666_7777_8888, kind: 23, extra: [0, 0xabcd] },
        ]);
    }

    #[test]
    fn rejects_map_without_repeated_keys() {
        let mut node = vec![0x80u8; 16];
        node[0] = 1;
        node.resize(16 + 16 * PART_SIZE, 0);
        put(&mut node, 16, &5u64.to_le_bytes());
        assert!(parts(&psin(&[(MAP_NODE_STRUCT, node)])).is_err());
    }

    #[test]
    fn reads_variation_records() {
        let mut data = vec![0u8; 3 * VARIATION_SIZE];
        for (i, part) in [(0usize, 0xaau64), (2, 0xbb)] {
            let at = i * VARIATION_SIZE;
            put(&mut data, at, &0x44e2_1c90u32.to_le_bytes());
            put(&mut data, at + 8, &part.to_le_bytes());
            put(&mut data, at + 16, &(part + 1).to_le_bytes());
            put(&mut data, at + 24, &(part + 2).to_le_bytes());
        }
        let d = psin(&[(1, vec![0; 4]), (VARIATION_STRUCT, data)]);
        let v = variations(&d).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1], Variation { variation: 0x44e2_1c90, part: 0xbb, diffuse: 0xbc, normal: 0xbd, extra: 0 });
    }
}
