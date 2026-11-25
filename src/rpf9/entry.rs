//! TOC entry parsing from decrypted table regions.

pub const RECORD: usize = 24;

#[derive(Debug, Clone)]
pub struct Entry {
    pub index: usize,
    pub offset: u64,
    pub size: u64,
    pub size_hint: u32,
    pub aux: u32,
    pub flags: u32,
    /// joaat of the entry's name, from the TOC's name hash column (see `name_hashes`).
    pub name_hash: Option<u32>,
    pub raw: [u8; RECORD],
}

impl Entry {
    pub fn is_zlib(&self, data: &[u8]) -> bool {
        data.get(self.offset as usize..).is_some_and(zlib_head)
    }

    pub fn payload_magic(&self, data: &[u8]) -> Option<[u8; 4]> {
        let off = self.offset as usize;
        data.get(off..off + 4).map(|s| [s[0], s[1], s[2], s[3]])
    }
}

/// True if `data` starts with a zlib stream header.
pub fn zlib_head(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] & 0x0f == 8 && ((u16::from(data[0]) << 8) | u16::from(data[1])) % 31 == 0
}

const GRAPH_MAGIC: [u8; 16] = [
    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x89, 0x09, 0xc5, 0xf2, 0xfa, 0x0d, 0xf7, 0x33,
];

pub fn has_graph_magic(plain: &[u8]) -> bool {
    plain.windows(GRAPH_MAGIC.len()).any(|w| w == GRAPH_MAGIC)
}

/// Decodes a record into `(byte offset, explicit size)`.
///
/// Two layouts share the 24-byte stride:
/// - indexed (most archives): `{u32 size_hint, 0, u32 offset/16, u32 flags, u32 aux, 0}`
/// - stored (e.g. script.rpf, whose entries are nested RPF9s):
///   `{u64 offset/16, u64 ceil(size/16), u64 size}`: the middle field is a
///   built-in check that the indexed layout never satisfies once flags are set.
fn decode(bytes: &[u8]) -> Option<(u64, Option<u64>)> {
    if bytes.len() < RECORD { return None; }
    let quad = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let (units, blocks, size) = (quad(0), quad(8), quad(16));
    if units != 0 && size != 0 && blocks == size.div_ceil(16) {
        return Some((units.saturating_mul(16), Some(size)));
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    if word(4) != 0 || word(20) != 0 || word(8) == 0 { return None; }
    Some((u64::from(word(8)) * 16, None))
}

/// Rewrites a record's location in place, preserving its layout.
pub fn encode_location(record: &mut [u8], offset: u64, size: u64) {
    if matches!(decode(record), Some((_, Some(_)))) {
        record[0..8].copy_from_slice(&(offset / 16).to_le_bytes());
        record[8..16].copy_from_slice(&size.div_ceil(16).to_le_bytes());
        record[16..24].copy_from_slice(&size.to_le_bytes());
    } else {
        record[8..12].copy_from_slice(&((offset / 16) as u32).to_le_bytes());
    }
}

pub fn detect(plain: &[u8], file_len: u64, declared: usize) -> (usize, usize) {
    let limit = declared.saturating_add(2);
    let mut best = (0usize, 0usize);
    let mut start = 0usize;
    while start + RECORD <= plain.len() {
        let mut run = 0usize;
        let mut at = start;
        while at + RECORD <= plain.len() && run < limit {
            let Some((offset, _)) = decode(&plain[at..]) else { break; };
            if offset / 16 >= file_len { break; }
            run += 1;
            at += RECORD;
        }
        if run > best.1 { best = (start, run); }
        start += 4;
    }
    best
}

/// Directory hash of the 20-byte entry section that content archives (e.g. the
/// nested RPF9s of story/level packs) carry instead of the 24-byte one:
/// `{u32 size_hint, u32 flags, u32 size/16, u32 offset/16, u32 aux}`.
/// Each payload starts with 16 zero bytes, which are not part of the entry.
const COMPACT_SECTION: [u8; 8] = 0xba18_0afb_232d_1b4bu64.to_le_bytes();
const COMPACT_RECORD: usize = 20;
const COMPACT_PREFIX: u64 = 16;

/// Finds the compact entry table: `(start, count)` of the first 8-aligned run
/// of `count` records after the section's directory descriptor whose payloads
/// lie in `data` past `data_start` behind the zero prefix.
pub fn detect_compact(plain: &[u8], data: &[u8], data_start: u64) -> Option<(usize, usize)> {
    let dir = plain.windows(8).position(|w| w == COMPACT_SECTION)?;
    let count = u32::from_le_bytes(plain.get(dir + 8..dir + 12)?.try_into().unwrap()) as usize;
    let valid = |at: usize| {
        let (_, blocks, units) = compact_fields(&plain[at..at + COMPACT_RECORD]);
        let (start, end) = (units * 16, (units + blocks) * 16);
        blocks > 1 && start >= data_start && end <= data.len() as u64
            && data[start as usize..(start + COMPACT_PREFIX) as usize].iter().all(|&b| b == 0)
    };
    ((dir + 24).next_multiple_of(8)..)
        .step_by(8)
        .take_while(|&s| s + count * COMPACT_RECORD <= plain.len())
        .find(|&s| (0..count).all(|i| valid(s + i * COMPACT_RECORD)))
        .map(|s| (s, count))
}

/// Column ids in the TOC's column directory, whose descriptors are `{u64 id, u32 count, u32 offset,
/// u64 stride}` with offsets from one shared base: the 24-byte records, the 20-byte records
/// (`COMPACT_SECTION`) and the u32 name hashes, one per record index. A u64 column (id
/// 0x73bcb72dce74f11f) holds a further per-record value whose hash function is not identified.
const RECORDS: u64 = 0x33f7_0dfa_f2c5_0989;
const NAME_HASHES: u64 = 0xb707_6b62_45d3_5fb5;

/// `(count, offset, stride)` of column `id`.
fn column(plain: &[u8], id: u64) -> Option<(usize, usize, u64)> {
    let at = plain.windows(8).position(|w| w == id.to_le_bytes())?;
    let d = plain.get(at + 8..at + 24)?;
    let word = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as usize;
    Some((word(0), word(4), u64::from_le_bytes(d[8..16].try_into().unwrap())))
}

/// The name hash column, indexed like the records that start at `table_at`. Some archives
/// declare fewer hashes than records; the records past the column have none.
pub fn name_hashes(plain: &[u8], table_at: usize, compact: bool) -> Vec<u32> {
    let records = if compact { u64::from_le_bytes(COMPACT_SECTION) } else { RECORDS };
    let (Some((_, rec_off, _)), Some((count, off, 4))) = (column(plain, records), column(plain, NAME_HASHES)) else { return Vec::new() };
    let Some(at) = (table_at + off).checked_sub(rec_off) else { return Vec::new() };
    plain.get(at..at + count * 4).map_or(Vec::new(), |b| b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
}

fn compact_fields(bytes: &[u8]) -> ([u32; 5], u64, u64) {
    let w: [u32; 5] = std::array::from_fn(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()));
    (w, u64::from(w[2]), u64::from(w[3]))
}

pub fn parse_compact(plain: &[u8], table_at: usize, count: usize) -> Vec<Entry> {
    (0..count)
        .map(|index| {
            let at = table_at + index * COMPACT_RECORD;
            let (w, blocks, units) = compact_fields(&plain[at..at + COMPACT_RECORD]);
            let mut raw = [0u8; RECORD];
            raw[..COMPACT_RECORD].copy_from_slice(&plain[at..at + COMPACT_RECORD]);
            Entry {
                index,
                offset: units * 16 + COMPACT_PREFIX,
                size: blocks * 16 - COMPACT_PREFIX,
                size_hint: w[0],
                aux: w[4],
                flags: w[1],
                name_hash: None,
                raw,
            }
        })
        .collect()
}

pub fn parse(plain: &[u8], table_at: usize, run: usize, count: usize, end: u64) -> Vec<Entry> {
    let count = count.min(run);
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = table_at + index * RECORD;
        let Some(bytes) = plain.get(at..at + RECORD) else { break; };
        let raw: [u8; RECORD] = bytes.try_into().unwrap();
        let word = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
        let Some((offset, size)) = decode(&raw) else { continue; };
        if offset >= end { continue; }
        entries.push(Entry {
            index,
            offset,
            size: size.unwrap_or(0),
            size_hint: word(0),
            aux: word(16),
            flags: word(12),
            name_hash: None,
            raw,
        });
    }
    assign_sizes(&mut entries, end);
    entries
}

/// Fills unknown (zero) sizes with the span up to the next entry or `end`.
pub fn assign_sizes(entries: &mut [Entry], end: u64) {
    if entries.is_empty() { return; }
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| entries[i].offset);
    order.dedup_by_key(|&mut i| entries[i].offset);
    for pair in order.windows(2) {
        if entries[pair[0]].size == 0 {
            entries[pair[0]].size = entries[pair[1]].offset.saturating_sub(entries[pair[0]].offset);
        }
    }
    let last = *order.last().unwrap();
    if entries[last].size == 0 {
        entries[last].size = end.saturating_sub(entries[last].offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quads(q: [u64; 3]) -> Vec<u8> {
        q.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn stored_layout_uses_explicit_offset_and_size() {
        // script.rpf record #0: nested RPF9 at 0x1000, 0xc2b90 bytes
        let plain = [quads([0x100, 0xc2b9, 0xc2b90]), quads([0xc400, 0x3642b, 0x3642b0])].concat();
        let (start, run) = detect(&plain, 0x428330, 2);
        assert_eq!((start, run), (0, 2));
        let entries = parse(&plain, start, run, 2, 0x4282b0);
        assert_eq!((entries[0].offset, entries[0].size), (0x1000, 0xc2b90));
        assert_eq!((entries[1].offset, entries[1].size), (0xc4000, 0x3642b0));
    }

    #[test]
    fn indexed_layout_spans_to_next_entry() {
        let mut plain = vec![0u8; 2 * RECORD];
        for (i, units) in [0x36u32, 0x100].iter().enumerate() {
            let at = i * RECORD;
            plain[at..at + 4].copy_from_slice(&4u32.to_le_bytes());
            plain[at + 8..at + 12].copy_from_slice(&units.to_le_bytes());
            plain[at + 12..at + 16].copy_from_slice(&0xff00u32.to_le_bytes());
        }
        let entries = parse(&plain, 0, 2, 2, 0x2270);
        assert_eq!((entries[0].offset, entries[0].size), (0x360, 0xca0));
        assert_eq!((entries[1].offset, entries[1].size), (0x1000, 0x1270));
    }

    #[test]
    fn compact_layout_from_directory() {
        // story000_restofworld.rpf nested #19: directory descriptor, padding, one record
        let mut plain = vec![0u8; 0x60];
        plain[0..8].copy_from_slice(&COMPACT_SECTION);
        plain[8..12].copy_from_slice(&1u32.to_le_bytes());
        for (i, w) in [0x201bu32, 0xe601, 0xb5e, 0x100, 4].iter().enumerate() {
            plain[0x30 + i * 4..0x34 + i * 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut data = vec![0u8; 0xd000];
        assert_eq!(detect_compact(&plain, &data, 0x2b0), Some((0x30, 1)));
        data[0x1008] = 1; // zero prefix broken
        assert_eq!(detect_compact(&plain, &data, 0x2b0), None);
        assert_eq!(detect_compact(&plain, &data[..0xc000], 0x2b0), None);
        let entries = parse_compact(&plain, 0x30, 1);
        assert_eq!((entries[0].offset, entries[0].size, entries[0].aux), (0x1010, 0xb5d0, 4));
    }

    #[test]
    fn name_hashes_follow_the_column_directory() {
        // story000_restofworld.rpf nested #1 shape: directory, then records and hashes at offsets
        // from a shared base (here 0x100, records at base + 0x40, hashes at base + 0x20).
        let mut plain = vec![0u8; 0x200];
        for (i, (id, count, off, stride)) in [(NAME_HASHES, 2u32, 0x20u32, 4u64), (u64::from_le_bytes(COMPACT_SECTION), 3, 0x40, 20)].into_iter().enumerate() {
            let d = 0x10 + i * 24;
            plain[d..d + 8].copy_from_slice(&id.to_le_bytes());
            plain[d + 8..d + 12].copy_from_slice(&count.to_le_bytes());
            plain[d + 12..d + 16].copy_from_slice(&off.to_le_bytes());
            plain[d + 16..d + 24].copy_from_slice(&stride.to_le_bytes());
        }
        plain[0x120..0x128].copy_from_slice(&[0x8b, 0x28, 0xf2, 0xd1, 0xcc, 0x4d, 0x93, 0x24]);
        assert_eq!(name_hashes(&plain, 0x140, true), [0xd1f2_288b, 0x2493_4dcc]);
        // No 24-byte records directory, so nothing for the indexed layout.
        assert!(name_hashes(&plain, 0x140, false).is_empty());
        plain[0x10..0x18].fill(0);
        assert!(name_hashes(&plain, 0x140, true).is_empty());
    }

    #[test]
    fn encode_location_roundtrips_both_layouts() {
        let mut stored = quads([0x100, 0xc2b9, 0xc2b90]);
        encode_location(&mut stored, 0x2000, 0x31);
        assert_eq!(decode(&stored), Some((0x2000, Some(0x31))));
        let mut indexed = vec![0u8; RECORD];
        indexed[8] = 1;
        indexed[12..16].copy_from_slice(&0xff00u32.to_le_bytes());
        encode_location(&mut indexed, 0x3000, 0x99);
        assert_eq!(decode(&indexed), Some((0x3000, None)));
    }
}
