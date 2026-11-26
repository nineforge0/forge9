//! PSIN (PSO v122) parser: compiled game data (binary-XML DOM).
//!
//! PSIN containers hold game configuration: vehicle handling, weapons,
//! AI behavior, spawn rules, etc. The format is an IFF-chunk chain:
//! PSIN → LSCH → LENM → LSNS → LGID → LRSC → CHKS
//!
//! Each section has a 4-byte ASCII tag + u32 BE length (includes header).
//! Instance blocks carry the binary-XML DOM (tree of nodes + attributes).

use anyhow::{bail, Result};
use std::fmt;

#[derive(Debug, Clone)]
pub struct PsinContainer {
    pub sections: Vec<Section>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub dom: Option<Dom>,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub tag: [u8; 4],
    pub offset: usize,
    pub length: usize,
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = std::str::from_utf8(&self.tag).unwrap_or("????");
        write!(f, "{} at {:#x} ({} bytes)", tag, self.offset, self.length)
    }
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name_hash: u32,
    pub signature: u32,
    pub size: u32,
    pub members: Vec<MemberDef>,
}

#[derive(Debug, Clone)]
pub struct MemberDef {
    pub name_hash: u32,
    pub offset: u32,
    pub type_code: u8,
    pub sub_type: u8,
    pub reference: u16,
    pub extra: u32,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name_hash: u32,
    pub values: Vec<(i64, u64)>,
}

#[derive(Debug, Clone)]
pub struct Dom {
    pub nodes: Vec<DomNode>,
    pub attributes: Vec<DomAttr>,
    pub strings: Vec<String>,
    pub vectors: Vec<[f32; 3]>,
    pub u64_table: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct DomNode {
    pub name_hash: u32,
    pub parent: u16,
    pub child: u16,
    pub sibling: u16,
    pub prev_sibling: u16,
    pub attr_start: u16,
    pub attr_count: u8,
}

#[derive(Debug, Clone)]
pub struct DomAttr {
    pub name_hash: u32,
    pub value_hash: u32,
    pub raw_value: u32,
    pub attr_type: u8,
}

impl DomAttr {
    pub fn type_name(&self) -> &'static str {
        match self.attr_type {
            0 => "string",
            1 => "vec3",
            2 => "float",
            3 => "int",
            4 => "bool",
            5 => "hash64",
            _ => "unknown",
        }
    }
}

impl Dom {
    /// Render as XML using the hash dictionary for name resolution.
    pub fn to_xml(&self, dict: &crate::rpf9::hashes::Dict) -> String {
        let mut out = String::from("<?xml version=\"1.0\"?>\n");
        if self.nodes.is_empty() { return out; }
        self.write_node(0, 0, dict, &mut out);
        out
    }

    fn write_node(&self, idx: usize, depth: usize, dict: &crate::rpf9::hashes::Dict, out: &mut String) {
        let node = &self.nodes[idx];
        let indent: String = "  ".repeat(depth);
        let name = dict.fmt(node.name_hash);

        let attrs: Vec<String> = (0..node.attr_count as usize).filter_map(|i| {
            let a = self.attributes.get(node.attr_start as usize + i)?;
            let aname = dict.fmt(a.name_hash);
            let val = self.format_attr(a, dict);
            Some(format!("{aname}=\"{val}\""))
        }).collect();

        let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(" ")) };

        if node.child == 0xFFFF {
            out.push_str(&format!("{indent}<{name}{attr_str} />\n"));
        } else {
            out.push_str(&format!("{indent}<{name}{attr_str}>\n"));
            let mut ci = node.child as usize;
            // depth cap guards against cyclic child/sibling links in malformed data
            while ci < self.nodes.len() && depth < 64 {
                self.write_node(ci, depth + 1, dict, out);
                let sib = self.nodes[ci].sibling;
                if sib == 0xFFFF { break; }
                ci = sib as usize;
            }
            out.push_str(&format!("{indent}</{name}>\n"));
        }
    }

    fn format_attr(&self, a: &DomAttr, dict: &crate::rpf9::hashes::Dict) -> String {
        match a.attr_type {
            0 => dict.fmt(a.raw_value),
            1 => {
                let vi = a.raw_value as usize;
                if let Some(v) = self.vectors.get(vi) {
                    format!("{:.6}, {:.6}, {:.6}", v[0], v[1], v[2])
                } else {
                    format!("vec3[{vi}]")
                }
            }
            2 => format!("{:.6}", f32::from_bits(a.raw_value)),
            3 => format!("{}", a.raw_value as i32),
            4 => if a.raw_value != 0 { "true".into() } else { "false".into() },
            5 => {
                let ui = a.raw_value as usize;
                if let Some(&h) = self.u64_table.get(ui) {
                    format!("{h:#018x}")
                } else {
                    format!("u64[{ui}]")
                }
            }
            _ => format!("{:#010x}", a.raw_value),
        }
    }
}

impl PsinContainer {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 8 || &data[..4] != b"PSIN" {
            bail!("not a PSIN container (magic mismatch)");
        }

        let mut sections = Vec::new();
        let mut pos = 0;
        while pos + 8 <= data.len() {
            let tag: [u8; 4] = data[pos..pos + 4].try_into().unwrap();
            let length = u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if length < 8 || pos + length > data.len() { break; }
            sections.push(Section { tag, offset: pos, length });
            pos += length;
        }

        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut dom = None;

        for section in &sections {
            let body = &data[section.offset + 8..section.offset + section.length];
            match &section.tag {
                b"LSCH" => structs = parse_lsch(body)?,
                b"LENM" => enums = parse_lenm(body)?,
                b"PSIN" if section.offset == 0 => {
                    dom = parse_psin_instances(body, &structs).ok().flatten();
                }
                _ => {}
            }
        }

        Ok(PsinContainer { sections, structs, enums, dom })
    }
}

fn le32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn le16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}

fn le64(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
}

fn f32_le(data: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn parse_lsch(body: &[u8]) -> Result<Vec<StructDef>> {
    // All 32-byte struct headers come first, then every struct's 16-byte
    // members back to back in header order.
    let mut structs = Vec::new();
    let mut counts = Vec::new();
    let mut pos = 0;
    while pos + 32 <= body.len() && body[pos + 16..pos + 24] == [0xAA; 8] {
        structs.push(StructDef {
            name_hash: le32(body, pos),
            signature: le32(body, pos + 4),
            size: le32(body, pos + 24),
            members: Vec::new(),
        });
        counts.push(u16::from_le_bytes(body[pos + 30..pos + 32].try_into().unwrap()) as usize);
        pos += 32;
    }
    for (s, count) in structs.iter_mut().zip(counts) {
        for _ in 0..count {
            if pos + 16 > body.len() { bail!("LSCH members truncated at {pos:#x}"); }
            s.members.push(MemberDef {
                name_hash: le32(body, pos),
                offset: le32(body, pos + 4),
                type_code: body[pos + 8],
                sub_type: body[pos + 9],
                reference: u16::from_le_bytes(body[pos + 10..pos + 12].try_into().unwrap()),
                extra: le32(body, pos + 12),
            });
            pos += 16;
        }
    }
    Ok(structs)
}

fn parse_lenm(body: &[u8]) -> Result<Vec<EnumDef>> {
    // Headers first, then values
    let mut enums = Vec::new();
    let mut pos = 0;
    let mut headers = Vec::new();
    // Scan for headers (sentinel 0xEE×8)
    while pos + 24 <= body.len() {
        let sentinel = &body[pos + 8..pos + 16];
        if sentinel == [0xEE; 8] {
            let name_hash = le32(body, pos);
            let count = le32(body, pos + 16) as usize;
            headers.push((name_hash, count));
            pos += 24;
        } else {
            break;
        }
    }
    // Now read values
    for (name_hash, count) in headers {
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            if pos + 16 > body.len() { break; }
            let val = i64::from_le_bytes(body[pos..pos + 8].try_into().unwrap());
            let name = u64::from_le_bytes(body[pos + 8..pos + 16].try_into().unwrap());
            values.push((val, name));
            pos += 16;
        }
        enums.push(EnumDef { name_hash, values });
    }
    Ok(enums)
}

fn parse_psin_instances(body: &[u8], structs: &[StructDef]) -> Result<Option<Dom>> {
    let root_hash: u32 = 0xde396fe2;
    if !structs.iter().any(|s| s.name_hash == root_hash) {
        return Ok(None);
    }

    // Block header table: contiguous N x 16-byte {type_hash, size, 0xAA×8} at body[0].
    // Data region follows after padding to 16-byte file alignment.
    let mut headers: Vec<(u32, usize)> = Vec::new();
    let mut pos = 0;
    while pos + 16 <= body.len() && body[pos + 8..pos + 16] == [0xAA; 8] {
        headers.push((le32(body, pos), le32(body, pos + 4) as usize));
        pos += 16;
    }
    if headers.is_empty() { return Ok(None); }

    // PSIN section starts at file offset 0, so body is at file offset 8.
    // Data starts at align16(8 + N*16) in the file = align16(8 + N*16) - 8 in body.
    let n = headers.len();
    let mut file_off = (8 + n * 16 + 15) & !15;
    let mut blocks: Vec<(u32, &[u8])> = Vec::new();
    for &(type_hash, size) in &headers {
        let body_off = file_off - 8;
        let end = (body_off + size).min(body.len());
        if body_off > body.len() { break; }
        blocks.push((type_hash, &body[body_off..end]));
        file_off = (file_off + size + 15) & !15;
    }

    // Find the root document block
    let root_data = blocks.iter().find(|(h, _)| *h == root_hash).map(|(_, d)| *d);
    if root_data.is_none() {
        return Ok(None);
    }
    let root_data = root_data.unwrap();
    if root_data.len() < 0x50 {
        return Ok(None);
    }

    // DOM layout derived from rage9-re format documentation.

    // Helper: read an atArray ref from the root block and resolve its pool.
    let read_pool = |arr_off: usize| -> Option<(usize, &[u8])> {
        let count = le32(root_data, arr_off) as usize;
        if count == 0 { return Some((0, &[])); }
        let pool_ref = le64(root_data, arr_off + 8);
        let blk_idx = (pool_ref & 0xFFF) as usize;
        let byte_off = (pool_ref >> 20) as usize;
        if blk_idx == 0 { return None; }
        let pool = blocks.get(blk_idx - 1)?.1;
        if byte_off <= pool.len() { Some((count, &pool[byte_off..])) } else { None }
    };

    let mut attributes = Vec::new();
    if let Some((n, pool)) = read_pool(0x00) {
        for i in 0..n {
            let o = i * 16;
            if o + 16 > pool.len() { break; }
            attributes.push(DomAttr {
                name_hash: le32(pool, o),
                value_hash: le32(pool, o + 4),
                raw_value: le32(pool, o + 8),
                attr_type: pool[o + 12],
            });
        }
    }

    // Strings (+0x10): skipped for now
    let strings = Vec::new();

    let mut vectors = Vec::new();
    if let Some((n, pool)) = read_pool(0x20) {
        for i in 0..n {
            let o = i * 12;
            if o + 12 > pool.len() { break; }
            vectors.push([f32_le(pool, o), f32_le(pool, o + 4), f32_le(pool, o + 8)]);
        }
    }

    let mut u64_table = Vec::new();
    if let Some((n, pool)) = read_pool(0x30) {
        for i in 0..n {
            let o = i * 8;
            if o + 8 > pool.len() { break; }
            u64_table.push(le64(pool, o));
        }
    }

    let mut nodes = Vec::new();
    if let Some((n, pool)) = read_pool(0x40) {
        for i in 0..n {
            let o = i * 16;
            if o + 16 > pool.len() { break; }
            nodes.push(DomNode {
                name_hash: le32(pool, o),
                parent: le16(pool, o + 4),
                child: le16(pool, o + 6),
                sibling: le16(pool, o + 8),
                prev_sibling: le16(pool, o + 10),
                attr_start: le16(pool, o + 12),
                attr_count: pool[o + 14],
            });
        }
    }

    Ok(Some(Dom { nodes, attributes, strings, vectors, u64_table }))
}

/// Print a PSIN container summary.
pub fn print_summary(container: &PsinContainer) {
    println!("  sections:");
    for s in &container.sections {
        println!("    {s}");
    }
    println!("  {} struct definitions, {} enum definitions",
        container.structs.len(), container.enums.len());
    if let Some(dom) = &container.dom {
        println!("  DOM: {} nodes, {} attributes, {} vec3, {} u64",
            dom.nodes.len(), dom.attributes.len(), dom.vectors.len(), dom.u64_table.len());
    }
}

/// Print struct definitions with member details.
pub fn print_structs(container: &PsinContainer, limit: usize, dict: &crate::rpf9::hashes::Dict) {
    for (i, s) in container.structs.iter().take(limit).enumerate() {
        println!("  struct #{i} hash={} sig={:#010x} size={} members={}",
            dict.fmt(s.name_hash), s.signature, s.size, s.members.len());
        for m in &s.members {
            let type_name = match m.type_code {
                0x01 => "bool", 0x05 => "struct", 0x07 => "ptr",
                0x10 | 0x11 => "u8", 0x12 | 0x13 => "u16",
                0x14 => "i32/f32", 0x15 => "u32", 0x16 | 0x17 | 0x18 => "u64",
                0x20..=0x22 => "vec3", 0x23 | 0x24 => "vec4",
                0x44 => "string", 0x48 => "hash_string", 0x4A => "joaat",
                0x50 => "array_fixed", 0x52 => "array_ptr", 0x5C => "array_counted",
                0x60 => "enum_u8", 0x65 => "bitset",
                _ => "?",
            };
            // name 0x100 marks an anonymous array-element descriptor
            let name = if m.name_hash == 0x100 { "<item>".to_string() } else { dict.fmt(m.name_hash) };
            let target = if m.extra != 0 { format!(" -> {}", dict.fmt(m.extra)) } else { String::new() };
            println!("    +{:#06x} {type_name:<13} {name} sub={:#04x} ref={:#06x}{target}",
                m.offset, m.sub_type, m.reference);
        }
    }
    if container.structs.len() > limit {
        println!("  … {} more structs", container.structs.len() - limit);
    }
    for e in &container.enums {
        let values: Vec<String> = e.values.iter().map(|(v, h)| format!("{v}={h:#010x}")).collect();
        println!("  enum {} [{}]", dict.fmt(e.name_hash), values.join(", "));
    }
}

/// Print the DOM as XML.
pub fn print_dom(container: &PsinContainer, dict: &crate::rpf9::hashes::Dict) {
    if let Some(dom) = &container.dom {
        print!("{}", dom.to_xml(dict));
    } else {
        println!("  (no DOM)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: u32, size: u32, members: u16) -> Vec<u8> {
        let mut h = vec![0u8; 32];
        h[0..4].copy_from_slice(&name.to_le_bytes());
        h[16..24].copy_from_slice(&[0xAA; 8]);
        h[24..28].copy_from_slice(&size.to_le_bytes());
        h[30..32].copy_from_slice(&members.to_le_bytes());
        h
    }

    fn member(name: u32, offset: u32, ty: u8, extra: u32) -> Vec<u8> {
        [name.to_le_bytes(), offset.to_le_bytes(), [ty, 0, 0, 0], extra.to_le_bytes()].concat()
    }

    #[test]
    fn lsch_headers_precede_members() {
        let body = [
            header(0xde396fe2, 0x50, 2),
            header(0x85b4c631, 0x10, 1),
            member(0x100, 0, 0x05, 0x85b4c631),
            member(0x749dbb9e, 0, 0x5c, 0),
            member(0x2a8e416f, 0xc, 0x60, 0x649ecf8e),
        ].concat();
        let structs = parse_lsch(&body).unwrap();
        assert_eq!(structs.len(), 2);
        assert_eq!(structs[0].members.len(), 2);
        assert_eq!(structs[0].members[0].extra, 0x85b4c631);
        assert_eq!(structs[1].members[0].type_code, 0x60);
        assert_eq!(structs[1].members[0].offset, 0xc);
        assert!(parse_lsch(&body[..body.len() - 1]).is_err());
    }
}
