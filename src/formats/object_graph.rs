//! Serialised Object Graph parser.
//!
//! Format reference: rage9-re docs/SCRIPT_FORMAT.md section 4.
//! Header (0x00..0x28): all zero except u32_le record_count at +0x14.
//! Offset table (0x28..): record_count x u64_le strictly ascending 16-byte-aligned offsets.
//! Records: each has +0x00 u32 root_id, +0x04 u32 zero, +0x08 u64 type_hash.
//! Tagged pointers: {u32 target_plus_8, u32 0x71717171} at 8-byte aligned offsets >= +0x10.

use super::{u32_at, u64_at};
use anyhow::{ensure, Result};
use std::collections::BTreeMap;

const POINTER_TAG: u32 = 0x71717171;
const ARRAY_TYPE: u64 = 0x14;

pub struct Record {
    pub index: usize,
    pub offset: usize,
    pub size: usize,
    pub root_id: u32,
    pub type_hash: u64,
    /// (count, stride) when type_hash == 0x14
    pub array_info: Option<(u32, u32)>,
    /// (field_offset_within_record, target_record_index)
    pub pointers: Vec<(usize, usize)>,
}

pub struct ObjectGraph {
    pub records: Vec<Record>,
    pub root_indices: Vec<usize>,
}

impl ObjectGraph {
    pub fn parse(buf: &[u8]) -> Result<Self> {
        ensure!(buf.len() >= 0x30, "too short for object graph header");
        // Header: 0x00..0x14 all zero, u32 record_count at +0x14, 0x18..0x28 all zero
        ensure!(buf[..0x14].iter().all(|&b| b == 0), "header bytes 0x00..0x14 not zero");
        let record_count = u32_at(buf, 0x14).unwrap() as usize;
        ensure!(record_count > 0, "record_count is zero");
        ensure!(buf[0x18..0x28].iter().all(|&b| b == 0), "header bytes 0x18..0x28 not zero");

        // Offset table
        let table_start = 0x28;
        let table_end = table_start + record_count * 8;
        ensure!(buf.len() >= table_end, "buffer too short for offset table");

        let mut offsets = Vec::with_capacity(record_count);
        for i in 0..record_count {
            let off = u64_at(buf, table_start + i * 8).unwrap() as usize;
            offsets.push(off);
        }
        // Verify ascending
        for w in offsets.windows(2) {
            ensure!(w[1] > w[0], "offsets not strictly ascending: {:#x} >= {:#x}", w[0], w[1]);
        }

        // Build offset -> index lookup
        let offset_to_index: BTreeMap<usize, usize> = offsets.iter().enumerate().map(|(i, &o)| (o, i)).collect();

        // Parse records
        let mut records = Vec::with_capacity(record_count);
        let mut root_indices = Vec::new();

        for i in 0..record_count {
            let off = offsets[i];
            let end = if i + 1 < record_count { offsets[i + 1] } else { buf.len() };
            let size = end - off;
            ensure!(off + 16 <= buf.len(), "record {i} header truncated at {off:#x}");

            let root_id = u32_at(buf, off).unwrap();
            let type_hash = u64_at(buf, off + 0x08).unwrap();

            let array_info = if type_hash == ARRAY_TYPE && off + 0x1C <= buf.len() {
                let count = u32_at(buf, off + 0x10).unwrap();
                let stride = u32_at(buf, off + 0x18).unwrap();
                Some((count, stride))
            } else {
                None
            };

            // Scan for tagged pointers at 8-byte aligned offsets >= +0x10
            let mut pointers = Vec::new();
            let scan_start = off + 0x10;
            let scan_end = off + size;
            let mut pos = scan_start;
            while pos + 8 <= scan_end {
                let lo = u32_at(buf, pos).unwrap();
                let hi = u32_at(buf, pos + 4).unwrap();
                if hi == POINTER_TAG && lo >= 8 {
                    let target_offset = (lo as usize) - 8;
                    if let Some(&target_idx) = offset_to_index.get(&target_offset) {
                        pointers.push((pos - off, target_idx));
                    }
                }
                pos += 8;
            }

            if root_id != 0 {
                root_indices.push(i);
            }

            records.push(Record { index: i, offset: off, size, root_id, type_hash, array_info, pointers });
        }

        Ok(ObjectGraph { records, root_indices })
    }

    /// Type distribution, tree count, pointer count, max depth.
    pub fn summary(&self) -> String {
        let pointer_count: usize = self.records.iter().map(|r| r.pointers.len()).sum();
        let max_depth = self.max_depth();
        let mut types: BTreeMap<u64, usize> = BTreeMap::new();
        for r in &self.records {
            *types.entry(r.type_hash).or_default() += 1;
        }
        let mut sorted: Vec<_> = types.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let mut out = format!(
            "{} records, {} roots ({} trees), {} pointers, max depth {}, {} distinct types\n",
            self.records.len(), self.root_indices.len(), self.root_indices.len(), pointer_count, max_depth, sorted.len()
        );
        out.push_str("  top types:\n");
        for (hash, count) in sorted.iter().take(10) {
            out.push_str(&format!("    {hash:#018x}: {count}\n"));
        }
        out
    }

    pub fn type_census(&self) -> BTreeMap<u64, usize> {
        let mut types: BTreeMap<u64, usize> = BTreeMap::new();
        for r in &self.records {
            *types.entry(r.type_hash).or_default() += 1;
        }
        types
    }

    /// Disassemble one tree rooted at the record whose root_id matches.
    pub fn dump_tree(&self, root_id: u32) -> Option<String> {
        let root_idx = self.records.iter().position(|r| r.root_id == root_id)?;
        let mut out = String::new();
        self.dump_record(&mut out, root_idx, 0);
        Some(out)
    }

    fn dump_record(&self, out: &mut String, idx: usize, depth: usize) {
        let r = &self.records[idx];
        let indent = "  ".repeat(depth);
        let array = r.array_info.map(|(c, s)| format!(" array[{c}] stride {s}")).unwrap_or_default();
        let root = if r.root_id != 0 { format!(" root_id={:#x}", r.root_id) } else { String::new() };
        out.push_str(&format!(
            "{indent}record #{} @{:#x} ({} bytes) type {:#018x}{root}{array} [{} ptrs]\n",
            r.index, r.offset, r.size, r.type_hash, r.pointers.len()
        ));
        for &(field_off, target) in &r.pointers {
            out.push_str(&format!("{indent}  +{field_off:#x} -> #{target}\n"));
            self.dump_record(out, target, depth + 1);
        }
    }

    fn max_depth(&self) -> usize {
        let mut max = 0;
        for &ri in &self.root_indices {
            max = max.max(self.tree_depth(ri));
        }
        max
    }

    fn tree_depth(&self, idx: usize) -> usize {
        let r = &self.records[idx];
        if r.pointers.is_empty() {
            1
        } else {
            1 + r.pointers.iter().map(|&(_, t)| self.tree_depth(t)).max().unwrap_or(0)
        }
    }
}

/// Quick heuristic: does this buffer look like a serialised object graph?
pub fn looks_like(buf: &[u8]) -> bool {
    if buf.len() < 0x30 { return false; }
    if !buf[..0x14].iter().all(|&b| b == 0) { return false; }
    let Some(rc) = u32_at(buf, 0x14) else { return false };
    if rc == 0 { return false; }
    if !buf[0x18..0x28].iter().all(|&b| b == 0) { return false; }
    // First offset should be a reasonable, 16-byte-aligned value
    let Some(first) = u64_at(buf, 0x28) else { return false };
    let expected_first = ((0x28 + rc as u64 * 8) + 15) & !15;
    first == expected_first
}
