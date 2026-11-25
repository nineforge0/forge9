//! Drawables (models): LOD geometry decode and glTF / OBJ export.
//!
//! A drawable is a paged resource whose system segment starts with the
//! drawable header and whose graphics segment holds vertex and index data.
//! A drawable dictionary instead starts with `+0x08 page map, +0x18 u32 1,
//! +0x20 {ptr u32 name hashes, u16 count, u16 capacity}, +0x30 {ptr drawable
//! pointers, u16 count, u16 capacity}`; its drawables have a zero page map.
//! Layout checked on real files in docs/drawable_format.md.
//!
//! - header: +0x08 page map, +0x10 shader group, +0x18 skeleton, +0x20 center,
//!   +0x2C radius, +0x30/+0x40 bounds, +0x50..+0x68 four LOD group pointers, +0xA8 name
//! - LOD group `{ptr models, u16 count, u16 capacity}`; model: +0x08 geometry
//!   pointers, +0x10 u16 count; geometry: +0x08 vertex buffer, +0x40 index descriptor
//! - vertex buffer: +0x08 u32 elements, +0x0C u16 stride, +0x18 data, +0x40 layout;
//!   layout: u32 offset per semantic, sizes at +0xD0, formats at +0x104,
//!   flags at +0x138 (`vertex count << 10 | 1`)
//! - vertex data starts with 16 u32 attribute descriptors (`format | offset << 7`);
//!   position format 66 is 8-byte quantized, 99 is float3
//! - semantic 4 (normal): one u32 per vertex, bits 0..9 / 10..19 an octahedral
//!   unit vector (unorm10 each); semantics 28 / 29: texcoords, snorm16x2 x `+0x7C f32`
//! - semantic 24: RGBA8 per vertex (exported as `_COLOR_0`, it is not an albedo tint)
//! - index data is clustered: per-cluster 7-bit triangle corners remapped
//!   through a u16/u32 global vertex table
//! - shader group (header +0x10): `+0x10 {ptr shaders, u16 count, u16 capacity}`;
//!   shader: +0x00 u32 hash, +0x10 texture slot pointers, +0x20 parameter table
//!   (`+0x01 u8 textures`, 8-byte `{u32 joaat name, u32 slot << 2}` entries from +0x08);
//!   texture reference +0x30 points to `{tagged ptr to {u64, u32 name hash}, u64 store key}`;
//!   model +0x20 points to one u16 shader index per geometry

use anyhow::{bail, ensure, Context, Result};
use std::fmt::Write as _;
use super::resource::{PHYSICAL_BASE, VIRTUAL_BASE};
use super::{u16_at, u32_at, u64_at};

pub const LOD_COUNT: usize = 4;
const HEADER: usize = 176;
const MAX_SYSTEM: u64 = 32 << 20;
const NORMAL_SEMANTIC: usize = 4;
const UV_SEMANTICS: [usize; 2] = [28, 29];
const COLOR_SEMANTIC: usize = 24;
/// joaat of the texture parameter names `diffusetex`, `bumptex` and `emissivetex`.
pub const DIFFUSE_PARAM: u32 = 0xc8e8_c282;
pub const BUMP_PARAM: u32 = 0x1e77_bf4c;
pub const EMISSIVE_PARAM: u32 = 0x8e0d_63ee;

#[derive(Debug)]
pub struct Mesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    /// Unit length.
    pub normals: Option<Vec<[f32; 3]>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    /// Second texcoord set (glTF TEXCOORD_1 only).
    pub uvs1: Option<Vec<[f32; 2]>>,
    /// Semantic 24, RGBA8.
    pub colors: Option<Vec<[u8; 4]>>,
    pub indices: Vec<u32>,
    /// Index into `Drawable::materials`.
    pub material: Option<usize>,
    /// The stored buffers and tables the mesh was decoded from.
    pub buffers: Buffers,
}

/// A geometry's stored vertex and index buffers, byte for byte, with the tables that describe them.
#[derive(Debug, Clone, Default)]
pub struct Buffers {
    /// Vertex buffer graphics data (`elements * element_stride` bytes): 16 u32 descriptors, the
    /// quantization header at +0x60, then one stream per semantic (structure of arrays).
    pub vertex: Vec<u8>,
    pub elements: u32,
    pub element_stride: u16,
    /// Semantics with a nonzero layout size, in semantic order.
    pub attributes: Vec<Attribute>,
    /// Position format (66 quantized, 99 float3) and stream offset.
    pub position_format: u32,
    pub position_offset: usize,
    pub quantization: Option<Quantization>,
    /// Texcoord snorm16 scale at vertex data +0x7C.
    pub texcoord_scale: Option<f32>,
    /// Index buffer graphics data: header words, meshlet records, triangle words, vertex refs.
    pub index: Vec<u8>,
    pub meshlets: Vec<Meshlet>,
    /// Bytes per vertex ref (2 or 4) and section offsets in `index`.
    pub ref_size: usize,
    pub triangles_offset: usize,
    pub refs_offset: usize,
    /// Meshlet-local vertex index to mesh vertex, concatenated over meshlets.
    pub refs: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub semantic: usize,
    pub offset: u32,
    pub size: u8,
    /// DXGI number declared by the layout (storage width).
    pub format: u8,
    /// The vertex descriptor at the same offset, if any.
    pub descriptor: Option<u32>,
}

/// Format-66 position decode: `bias + scale * lane`, lanes reordered by `axis`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantization {
    pub bias: [f32; 3],
    pub axis: u32,
    pub scale: [f32; 3],
}

/// One 16-byte meshlet (cluster) record of the index buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Meshlet {
    pub record: [u32; 4],
    pub vertices: usize,
    pub triangles: usize,
    /// Start in `Buffers::refs` and in the triangle words.
    pub first_ref: usize,
    pub first_triangle: usize,
}

/// Quantization header of format-66 vertex data (+0x60 f32x3 bias, +0x6C u32 axis order, +0x70 f32x3 scale).
pub fn quantization(vertex: &[u8]) -> Option<Quantization> {
    Some(Quantization { bias: vec3(vertex, 96)?, axis: u32_at(vertex, 108)?, scale: vec3(vertex, 112)? })
}

/// `count` positions stored at `at` of vertex data in `format` 66 (quantization header read from the same
/// data) or 99 (float3). Values are returned as stored, NaN and Inf included.
pub fn decode_positions(vertex: &[u8], format: u32, at: usize, count: usize) -> Result<Vec<[f32; 3]>> {
    let stride = match format { 99 => 12, 66 => 8, f => bail!("unsupported position format {f} (handled: 66 quantized, 99 float3)") };
    ensure!(at + count * stride <= vertex.len(), "positions {at:#x}+{count}x{stride} overflow the {:#x}-byte vertex data", vertex.len());
    if stride == 12 { return Ok((0..count).map(|i| vec3(vertex, at + i * 12).unwrap()).collect()); }
    ensure!(at >= 128, "quantized positions overlap their origin/scale at +0x60");
    let Quantization { bias, axis, scale } = quantization(vertex).unwrap();
    Ok((0..count).map(|i| {
        let q = u64_at(vertex, at + i * 8).unwrap();
        let lanes = [(q >> 8) & 0xFF_FFFF, ((q >> 32) & 0xFFFF) << 8 | (q & 0xFF), q >> 48];
        let [x, y, z] = [0, 1, 2].map(|k| (f64::from(bias[k]) + f64::from(scale[k]) * lanes[k] as f64) as f32);
        match axis { 0 => [z, y, x], 1 => [y, z, x], _ => [x, y, z] }
    }).collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextureRef {
    /// joaat of the shader parameter name (e.g. `DIFFUSE_PARAM`).
    pub param: u32,
    pub name_hash: u32,
    /// Looked up in the `.ttpf` texture index.
    pub key: u64,
}

#[derive(Debug, Default)]
pub struct Material {
    pub name: String,
    pub shader: u32,
    pub textures: Vec<TextureRef>,
    /// PNG images, filled in by the caller after resolving `textures`.
    pub base_color: Option<Vec<u8>>,
    pub normal: Option<Vec<u8>>,
    pub emissive: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct Drawable {
    pub name: String,
    pub lod: usize,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    /// Geometries left out, with the reason (e.g. an unhandled vertex format).
    pub skipped: Vec<String>,
}

fn system_ptr(p: u64) -> bool { (VIRTUAL_BASE..VIRTUAL_BASE + MAX_SYSTEM).contains(&p) }
fn finite(v: [f32; 3]) -> bool { v.iter().all(|c| c.is_finite() && c.abs() < 1e7) }

fn vec3(d: &[u8], at: usize) -> Option<[f32; 3]> {
    Some([f32::from_bits(u32_at(d, at)?), f32::from_bits(u32_at(d, at + 4)?), f32::from_bits(u32_at(d, at + 8)?)])
}

/// Drawable header check at system offset `at`. Standalone drawables carry
/// the page map pointer at +0x08; drawables inside a dictionary have zero there.
fn header_at(sys: &[u8], at: usize, standalone: bool) -> bool {
    let ptr = |off| u64_at(sys, at + off).is_some_and(system_ptr);
    if sys.len() < at + HEADER || u64_at(sys, at) != Some(0) || ![16, 80, 168].into_iter().all(ptr) { return false; }
    if standalone != ptr(8) || (!standalone && u64_at(sys, at + 8) != Some(0)) { return false; }
    // LOD 1..3 slots are null or pointers; effect objects with a `{ptr, u16 count, u16 capacity}` list at +0x50 fail here.
    if ![88, 96, 104].into_iter().all(|off| u64_at(sys, at + off) == Some(0) || ptr(off)) { return false; }
    let (Some(min), Some(max)) = (vec3(sys, at + 48), vec3(sys, at + 64)) else { return false; };
    let radius = f32::from_bits(u32_at(sys, at + 44).unwrap());
    finite(min) && finite(max) && (0..3).all(|i| min[i] <= max[i]) && (0.0..1e6).contains(&radius)
}

/// Standalone drawable header check.
pub fn looks_like(sys: &[u8]) -> bool { header_at(sys, 0, true) }

/// System offsets of the drawables of a drawable dictionary, in table order.
pub fn dictionary(sys: &[u8]) -> Option<Vec<usize>> {
    let list = |at| -> Option<(usize, usize)> {
        let (p, n) = (u64_at(sys, at)?, u16_at(sys, at + 8)? as usize);
        (system_ptr(p) && (1..=4096).contains(&n) && n == u16_at(sys, at + 10)? as usize).then(|| ((p - VIRTUAL_BASE) as usize, n))
    };
    if u64_at(sys, 0)? != 0 || !system_ptr(u64_at(sys, 8)?) || u64_at(sys, 16)? != 0 { return None; }
    let ((hashes, n), (values, m)) = (list(32)?, list(48)?);
    if n != m || hashes + n * 4 > sys.len() { return None; }
    (0..n).map(|i| {
        let p = u64_at(sys, values + i * 8).filter(|&p| system_ptr(p))?;
        let at = (p - VIRTUAL_BASE) as usize;
        header_at(sys, at, false).then_some(at)
    }).collect()
}

/// System and graphics page counts from the page map that +0x08 points to.
pub fn page_counts(sys: &[u8]) -> Option<(u8, u8)> {
    let at = u64_at(sys, 8).filter(|&p| system_ptr(p))? - VIRTUAL_BASE;
    let c = sys.get(at as usize + 8..at as usize + 10)?;
    Some((c[0], c[1]))
}

/// The embedded names (header +0xA8) of a drawable, or of each drawable of a drawable
/// dictionary, from its entry content; empty for anything else.
pub fn labels(sys: &[u8]) -> Vec<String> {
    let members = match dictionary(sys) {
        Some(m) => m,
        None if looks_like(sys) => vec![0],
        None => return Vec::new(),
    };
    let r = Res { sys, gfx: &[] };
    members.iter().filter_map(|&at| r.name(at + 168).ok()).collect()
}

/// Decodes a drawable, or every drawable of a drawable dictionary (merged,
/// mesh names prefixed by drawable name), from the inflated blocks of its
/// entry: system blocks first, then graphics. The page map's declared system
/// block count is tried first, then every other boundary.
pub fn parse_blocks(blocks: &[Vec<u8>], lod: usize) -> Result<Drawable> {
    let first = blocks.first().context("empty entry")?;
    // The page map or dictionary table can sit past the first block.
    let all = blocks.concat();
    if !looks_like(first) && dictionary(&all).is_none() {
        let what = match (first.get(0x10..0x14) == Some(b"0DRP"), page_counts(&all)) {
            (true, _) => format!(" (0DRP v{:#x} metadata resource)", first.get(0x14).copied().unwrap_or(0)),
            (false, Some((s, 0))) => format!(" (paged resource with {s} system and no graphics pages)"),
            _ => String::new(),
        };
        bail!("not a drawable or drawable dictionary{what}; fragments are not supported yet");
    }
    ensure!(blocks.len() >= 2, "drawable entry is a single block, so its system/graphics split is unknown");
    let declared = page_counts(&all).map(|(s, _)| s as usize).filter(|n| (1..blocks.len()).contains(n));
    let mut first_error = None;
    for split in declared.into_iter().chain((1..blocks.len()).filter(|&s| Some(s) != declared)) {
        let (sys, gfx) = (blocks[..split].concat(), blocks[split..].concat());
        let result = match dictionary(&sys) {
            Some(members) => parse_dictionary(&sys, &gfx, &members, lod),
            None => parse(&sys, &gfx, lod),
        };
        match result {
            Ok(d) => return Ok(d),
            Err(e) => { first_error.get_or_insert(e); }
        }
    }
    Err(first_error.unwrap())
}

fn parse_dictionary(sys: &[u8], gfx: &[u8], members: &[usize], lod: usize) -> Result<Drawable> {
    let (mut parts, mut errors) = (Vec::new(), Vec::new());
    for (i, &at) in members.iter().enumerate() {
        match parse_at(sys, gfx, at, lod) {
            Ok(d) => {
                let prefix: String = d.name.chars().map(|c| if c.is_ascii_alphanumeric() || "_.#-".contains(c) { c } else { '_' }).collect();
                parts.push(Drawable {
                    meshes: d.meshes.into_iter().map(|m| Mesh { name: format!("{prefix}/{}", m.name), ..m }).collect(),
                    materials: d.materials.into_iter().map(|m| Material { name: format!("{prefix}/{}", m.name), ..m }).collect(),
                    skipped: d.skipped.into_iter().map(|s| format!("{prefix}/{s}")).collect(),
                    ..d
                });
            }
            Err(e) => errors.push(format!("drawable {i}: {e:#}")),
        }
    }
    let mut d = merge(format!("drawable dictionary ({} drawables)", members.len()), parts);
    ensure!(!d.meshes.is_empty(), "no decodable drawable in the dictionary: {}", errors.join("; "));
    d.skipped.extend(errors);
    Ok(d)
}

/// Several drawables as one scene: meshes, materials and skip notes concatenated, material indices rebased.
pub fn merge(name: String, parts: Vec<Drawable>) -> Drawable {
    let lod = parts.first().map_or(0, |d| d.lod);
    let (mut meshes, mut materials, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for d in parts {
        let base = materials.len();
        meshes.extend(d.meshes.into_iter().map(|m| Mesh { material: m.material.map(|i| base + i), ..m }));
        materials.extend(d.materials);
        skipped.extend(d.skipped);
    }
    Drawable { name, lod, meshes, materials, skipped }
}

/// Decodes one LOD of a standalone drawable given its system and graphics segments.
pub fn parse(sys: &[u8], gfx: &[u8], lod: usize) -> Result<Drawable> {
    ensure!(looks_like(sys), "not a standalone drawable header");
    parse_at(sys, gfx, 0, lod)
}

fn parse_at(sys: &[u8], gfx: &[u8], base: usize, lod: usize) -> Result<Drawable> {
    ensure!(lod < LOD_COUNT, "LOD {lod} out of range (0..{LOD_COUNT})");
    let r = Res { sys, gfx };
    let name = r.name(base + 168)?;
    let lod_field = base + 80 + 8 * lod;
    ensure!(r.u64(lod_field)? != 0, "drawable has no LOD {lod}");
    let group = r.ptr(lod_field, 16)?;
    let models = r.u16(group + 8)? as usize;
    ensure!((1..=256).contains(&models) && models == r.u16(group + 10)? as usize, "bad LOD model count {models}");
    let list = r.ptr(group, models * 8)?;
    let (mut meshes, mut skipped) = (Vec::new(), Vec::new());
    let materials = r.shaders(base + 16).unwrap_or_else(|e| { skipped.push(format!("shader group: {e:#}")); Vec::new() });
    for mi in 0..models {
        let model = r.ptr(list + mi * 8, 56)?;
        let count = r.u16(model + 16)? as usize;
        ensure!((1..=1024).contains(&count) && count == r.u16(model + 18)? as usize, "model {mi}: bad geometry count {count}");
        let geometries = r.ptr(model + 8, count * 8)?;
        let shader_ix = if r.u64(model + 32)? == 0 { None } else { Some(r.ptr(model + 32, count * 2)?) };
        for gi in 0..count {
            let name = format!("model_{mi}_geometry_{gi}");
            match r.ptr(geometries + gi * 8, 160).and_then(|g| r.mesh(g, name.clone())) {
                Ok(mut m) => {
                    m.material = shader_ix.map(|at| r.u16(at + gi * 2).unwrap() as usize).filter(|&i| i < materials.len());
                    meshes.push(m);
                }
                Err(e) => skipped.push(format!("{name}: {e:#}")),
            }
        }
    }
    ensure!(!meshes.is_empty(), "no decodable geometry in LOD {lod}: {}", skipped.join("; "));
    Ok(Drawable { name, lod, meshes, materials, skipped })
}

struct Res<'a> {
    sys: &'a [u8],
    gfx: &'a [u8],
}

impl Res<'_> {
    fn u8(&self, at: usize) -> Result<u8> { self.sys.get(at).copied().with_context(|| format!("system read at {at:#x} out of bounds")) }
    fn u16(&self, at: usize) -> Result<u16> { u16_at(self.sys, at).with_context(|| format!("system read at {at:#x} out of bounds")) }
    fn u32(&self, at: usize) -> Result<u32> { u32_at(self.sys, at).with_context(|| format!("system read at {at:#x} out of bounds")) }
    fn u64(&self, at: usize) -> Result<u64> { u64_at(self.sys, at).with_context(|| format!("system read at {at:#x} out of bounds")) }

    /// System offset of the `size`-byte object the pointer at `at` refers to.
    fn ptr(&self, at: usize, size: usize) -> Result<usize> {
        let p = self.u64(at)?;
        let off = p.checked_sub(VIRTUAL_BASE).filter(|o| o.saturating_add(size as u64) <= self.sys.len() as u64);
        off.map(|o| o as usize).with_context(|| format!("system pointer {p:#x} at {at:#x} (+{size:#x}) out of range"))
    }

    /// The `size` graphics bytes the pointer at `at` refers to.
    fn gfx(&self, at: usize, size: usize) -> Result<&[u8]> {
        let p = self.u64(at)?;
        let off = p.checked_sub(PHYSICAL_BASE).filter(|o| o.saturating_add(size as u64) <= self.gfx.len() as u64);
        let off = off.with_context(|| format!("graphics pointer {p:#x} at {at:#x} (+{size:#x}) out of range"))? as usize;
        Ok(&self.gfx[off..off + size])
    }

    fn name(&self, at: usize) -> Result<String> {
        let s = self.ptr(at, 1)?;
        let bytes = &self.sys[s..self.sys.len().min(s + 256)];
        let end = bytes.iter().position(|&b| b == 0).context("unterminated drawable name")?;
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    fn mesh(&self, geometry: usize, name: String) -> Result<Mesh> {
        let vb = self.ptr(geometry + 8, 80)?;
        let layout = self.ptr(vb + 64, 320)?;
        let flags = self.u32(layout + 0x138)?;
        let count = (flags >> 10) as usize;
        ensure!(flags & 1 != 0 && (1..=1_000_000).contains(&count), "bad vertex layout flags {flags:#x}");
        let len = self.u32(vb + 8)? as usize * self.u16(vb + 12)? as usize;
        let vertex = self.gfx(vb + 24, len)?;
        let at = self.u32(layout)? as usize;
        let (size, format) = (self.u8(layout + 0xD0)?, self.u8(layout + 0x104)?);
        let descriptor = u32_at(vertex, 0).context("vertex buffer shorter than its descriptors")?;
        let stride = match descriptor & 127 {
            99 => 12,
            66 => 8,
            f => bail!("unsupported position format {f} (handled: 66 quantized, 99 float3)"),
        };
        ensure!(size as usize == stride && format == if stride == 12 { 6 } else { 16 },
            "position format {} disagrees with layout size {size} / format {format}", descriptor & 127);
        ensure!(at >= 96 && at + count * stride <= len, "positions {at:#x}+{count}x{stride} overflow the {len:#x}-byte vertex buffer");
        let quant = if stride == 8 { quantization(vertex) } else { None };
        if let Some(Quantization { bias, axis, scale }) = quant {
            ensure!(axis <= 2 && finite(bias) && finite(scale) && scale.iter().all(|&s| s >= 0.0), "bad quantization origin/scale");
        }
        let positions = decode_positions(vertex, descriptor & 127, at, count)?;
        ensure!(positions.iter().all(|&p| finite(p)), "non-finite vertex positions");
        let normals = self.normals(layout, count, vertex)?;
        let [uvs, uvs1] = [0, 1].map(|i| self.uvs(layout, UV_SEMANTICS[i], count, vertex));
        let colors = self.attribute(layout, COLOR_SEMANTIC, 4, 28, 6, count, vertex)?
            .map(|at| (0..count).map(|i| vertex[at + i * 4..][..4].try_into().unwrap()).collect());
        let (indices, mut buffers) = self.indices(geometry, count)?;
        let descriptors: Vec<u32> = (0..16).filter_map(|i| u32_at(vertex, i * 4)).collect();
        for semantic in 0..52 {
            let size = self.u8(layout + 0xD0 + semantic)?;
            if size == 0 { continue; }
            let offset = self.u32(layout + semantic * 4)?;
            let descriptor = descriptors.iter().copied().find(|d| (d >> 7) & 0x1FF_FFFC == offset && d & 127 != 0);
            buffers.attributes.push(Attribute { semantic, offset, size, format: self.u8(layout + 0x104 + semantic)?, descriptor });
        }
        let texcoord_scale = u32_at(vertex, 124).map(f32::from_bits).filter(|_| uvs.as_ref().is_ok_and(|u| u.is_some()) || uvs1.as_ref().is_ok_and(|u| u.is_some()));
        buffers = Buffers { vertex: vertex.to_vec(), elements: self.u32(vb + 8)?, element_stride: self.u16(vb + 12)?,
            position_format: descriptor & 127, position_offset: at, quantization: quant, texcoord_scale, ..buffers };
        Ok(Mesh { name, positions, normals, uvs: uvs?, uvs1: uvs1?, colors, indices, material: None, buffers })
    }

    /// Shaders of the shader group the pointer at `at` refers to, with their texture references.
    fn shaders(&self, at: usize) -> Result<Vec<Material>> {
        let group = self.ptr(at, 32)?;
        let n = self.u16(group + 24)? as usize;
        ensure!(n <= 256 && n == self.u16(group + 26)? as usize, "bad shader count {n}");
        if n == 0 { return Ok(Vec::new()); }
        let list = self.ptr(group + 16, n * 8)?;
        (0..n).map(|i| {
            let shader = self.ptr(list + i * 8, 48)?;
            let table = self.ptr(shader + 32, 8)?;
            let mut textures = Vec::new();
            for k in 0..self.u8(table + 1)? as usize {
                let (param, slot) = (self.u32(table + 8 + k * 8)?, self.u32(table + 12 + k * 8)?);
                ensure!(slot & !0xFC == 0, "shader {i}: texture parameter {param:#010x} has slot word {slot:#x}");
                let slot = self.ptr(shader + 16, slot as usize * 2 + 8)? + slot as usize * 2;
                if self.u64(slot)? == 0 { continue; }
                let name = self.ptr(self.ptr(slot, 80)? + 48, 16)?;
                let tagged = self.u64(name)?;
                let rec = (tagged & !15).checked_sub(VIRTUAL_BASE).map(|o| o as usize).filter(|o| o + 16 <= self.sys.len())
                    .with_context(|| format!("shader {i}: bad texture name pointer {tagged:#x}"))?;
                textures.push(TextureRef { param, name_hash: self.u32(rec + 8)?, key: self.u64(name + 8)? });
            }
            let hash = self.u32(shader)?;
            Ok(Material { name: format!("shader_{i}_{hash:08x}"), shader: hash, textures, ..Default::default() })
        }).collect()
    }

    /// Offset of semantic `sem` when the layout declares `(size, format)` and a vertex
    /// descriptor of `kind` (`& 15`) and `size` points at the same offset.
    fn attribute(&self, layout: usize, sem: usize, size: usize, format: u8, kind: u32, count: usize, vertex: &[u8]) -> Result<Option<usize>> {
        if (self.u8(layout + 0xD0 + sem)? as usize, self.u8(layout + 0x104 + sem)?) != (size, format) { return Ok(None); }
        let at = self.u32(layout + sem * 4)? as usize;
        let described = (0..16).filter_map(|i| u32_at(vertex, i * 4))
            .any(|d| (d >> 7) & 0x1FF_FFFC == at as u32 && ((d >> 3) & 60) as usize == size && d & 15 == kind);
        Ok((described && at + count * size <= vertex.len()).then_some(at))
    }

    /// Semantic 4 as a 10:10 unorm octahedral unit vector; `None` when absent or in another encoding.
    /// The word's top 12 bits (likely the tangent frame) are not decoded.
    fn normals(&self, layout: usize, count: usize, vertex: &[u8]) -> Result<Option<Vec<[f32; 3]>>> {
        let Some(at) = self.attribute(layout, NORMAL_SEMANTIC, 4, 41, 1, count, vertex)? else { return Ok(None) };
        Ok(Some((0..count).map(|i| {
            let w = u32_at(vertex, at + i * 4).unwrap();
            let [x, y] = [w, w >> 10].map(|c| (c & 1023) as f32 / 511.5 - 1.0);
            let z = 1.0 - x.abs() - y.abs();
            let (x, y) = if z < 0.0 { ((1.0 - y.abs()).copysign(x), (1.0 - x.abs()).copysign(y)) } else { (x, y) };
            let l = (x * x + y * y + z * z).sqrt();
            [x / l, y / l, z / l]
        }).collect()))
    }

    /// Texcoords of semantic `sem` as float2 or scaled snorm16x2; `None` when absent or in another encoding.
    fn uvs(&self, layout: usize, sem: usize, count: usize, vertex: &[u8]) -> Result<Option<Vec<[f32; 2]>>> {
        let (snorm, float) = (self.attribute(layout, sem, 4, 38, 8, count, vertex)?, self.attribute(layout, sem, 8, 16, 2, count, vertex)?);
        let (at, float, size) = match (snorm, float) { (Some(at), _) => (at, false, 4), (_, Some(at)) => (at, true, 8), _ => return Ok(None) };
        let scale = if float { 1.0 } else {
            match u32_at(vertex, 124).map(f32::from_bits) { Some(s) if s.is_finite() && s > 0.0 && s <= 1e6 => s, _ => return Ok(None) }
        };
        let uv: Vec<[f32; 2]> = (0..count).map(|i| {
            let p = at + i * size;
            if float { [0, 4].map(|o| f32::from_bits(u32_at(vertex, p + o).unwrap())) }
            else { [0, 2].map(|o| (f32::from(u16_at(vertex, p + o).unwrap() as i16) / 32767.0).max(-1.0) * scale) }
        }).collect();
        Ok(uv.iter().flatten().all(|c| c.is_finite() && c.abs() <= 1e6).then_some(uv))
    }

    /// Triangle list, plus the stored index buffer and its meshlet table (vertex fields left default).
    fn indices(&self, geometry: usize, vertex_count: usize) -> Result<(Vec<u32>, Buffers)> {
        let desc = self.ptr(geometry + 64, 16)?;
        let ib = self.ptr(desc, 64)?;
        ensure!(self.u16(ib + 12)? == 4, "index buffer stride is not 4");
        let len = self.u32(ib + 8)? as usize * 4;
        ensure!(len >= 32, "index buffer too short");
        let data = self.gfx(ib + 24, len)?;
        let word = |at: usize| u32_at(data, at).context("index data truncated");
        let head = word(0)?;
        let palette = head & 0x4000_0000 != 0;
        let clusters = (head & if palette { 0xFFFF } else { 0x7FFF_FFFF }) as usize;
        let (triangles, refs) = (word(4)? as usize, word(8)? as usize);
        let binding = self.u32(desc + 12)?;
        ensure!((1..=2048).contains(&clusters) && triangles > 0 && refs > 0
            && clusters == self.u16(desc + 8)? as usize
            && triangles == (self.u16(desc + 10)? as usize | ((binding & 127) as usize) << 16)
            && refs == ((binding >> 8) & 0x7F_FFFF) as usize,
            "index header ({clusters} clusters, {triangles} triangles, {refs} refs) disagrees with its descriptor");
        let ref_size = if head & 0x8000_0000 != 0 { 2 } else { 4 };
        let tri_at = 12 + clusters * 16;
        let refs_at = tri_at + triangles * 4;
        ensure!(refs_at + refs * ref_size <= len, "cluster tables overflow the {len:#x}-byte index buffer");
        let table: Vec<u32> = (0..refs).map(|i| match ref_size {
            2 => u32::from(u16_at(data, refs_at + i * 2).unwrap()),
            _ => u32_at(data, refs_at + i * 4).unwrap(),
        }).collect();
        ensure!(table.iter().all(|&v| (v as usize) < vertex_count), "vertex reference past {vertex_count} vertices");
        let mut out = Vec::with_capacity(triangles * 3);
        let mut meshlets = Vec::with_capacity(clusters);
        let (mut ref_next, mut tri_next) = (0, 0);
        for cluster in 0..clusters {
            let (packed, offsets) = (word(12 + cluster * 16)?, word(16 + cluster * 16)?);
            let record = [packed, offsets, word(20 + cluster * 16)?, word(24 + cluster * 16)?];
            let verts = (packed & 127) as usize + 1;
            let tris = ((packed >> 7) & 127) as usize + 1;
            let vstart = (offsets & 0xFFFF | ((packed >> 14) & 1) << 16) as usize;
            let tstart = (offsets >> 16 | ((packed >> 15) & 1) << 16) as usize;
            ensure!(vstart == ref_next && tstart == tri_next && vstart + verts <= refs && tstart + tris <= triangles,
                "cluster {cluster} ranges are not contiguous");
            for t in tstart..tstart + tris {
                let w = word(tri_at + t * 4)?;
                ensure!((w >> 21) as usize == cluster, "triangle {t} tagged for cluster {} not {cluster}", w >> 21);
                for corner in 0..3 {
                    let local = ((w >> (corner * 7)) & 127) as usize;
                    ensure!(local < verts, "triangle {t} corner past its cluster's {verts} vertices");
                    out.push(table[vstart + local]);
                }
            }
            meshlets.push(Meshlet { record, vertices: verts, triangles: tris, first_ref: vstart, first_triangle: tstart });
            (ref_next, tri_next) = (ref_next + verts, tri_next + tris);
        }
        ensure!(ref_next == refs && tri_next == triangles, "clusters do not cover the index tables");
        Ok((out, Buffers { index: data.to_vec(), meshlets, ref_size, triangles_offset: tri_at, refs_offset: refs_at, refs: table, ..Default::default() }))
    }
}

/// Wavefront OBJ. V is flipped from the top-left (D3D) to the bottom-left convention.
pub fn to_obj(d: &Drawable) -> String {
    let mut s = format!("# forge9 drawable export: {} LOD{}; positions in source (Z-up) space, no materials\n", d.name.replace(['\r', '\n'], "_"), d.lod);
    let (mut base, mut base_uv, mut base_n) = (1usize, 1usize, 1usize);
    for m in &d.meshes {
        let _ = writeln!(s, "o {}", m.name);
        for p in &m.positions { let _ = writeln!(s, "v {} {} {}", p[0], p[1], p[2]); }
        for uv in m.uvs.iter().flatten() { let _ = writeln!(s, "vt {} {}", uv[0], 1.0 - uv[1]); }
        for n in m.normals.iter().flatten() { let _ = writeln!(s, "vn {} {} {}", n[0], n[1], n[2]); }
        for t in m.indices.chunks_exact(3) {
            let corner = |i: u32| {
                let i = i as usize;
                match (&m.uvs, &m.normals) {
                    (Some(_), Some(_)) => format!("{}/{}/{}", base + i, base_uv + i, base_n + i),
                    (Some(_), None) => format!("{}/{}", base + i, base_uv + i),
                    (None, Some(_)) => format!("{}//{}", base + i, base_n + i),
                    (None, None) => (base + i).to_string(),
                }
            };
            let _ = writeln!(s, "f {} {} {}", corner(t[0]), corner(t[1]), corner(t[2]));
        }
        base += m.positions.len();
        if m.uvs.is_some() { base_uv += m.positions.len(); }
        if m.normals.is_some() { base_n += m.positions.len(); }
    }
    s
}

/// glTF 2.0 JSON plus its binary buffer. `uri` names an external .bin; `None` is for GLB.
/// Material images are embedded PNGs; semantic 24 goes to the custom `_COLOR_0`.
pub fn to_gltf(d: &Drawable, uri: Option<&str>) -> (String, Vec<u8>) {
    let (mut bin, mut views, mut accessors, mut meshes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut view = |bin: &mut Vec<u8>, bytes: Vec<u8>, target: u32| {
        let target = if target == 0 { String::new() } else { format!(r#","target":{target}"#) };
        views.push(format!(r#"{{"buffer":0,"byteOffset":{},"byteLength":{}{target}}}"#, bin.len(), bytes.len()));
        bin.extend(bytes);
        bin.resize(bin.len().next_multiple_of(4), 0);
        views.len() - 1
    };
    let (mut materials, mut images) = (Vec::new(), Vec::new());
    for m in &d.materials {
        let mut texture = |png: &Option<Vec<u8>>, key: &str| png.as_ref().map_or(String::new(), |png| {
            images.push(format!(r#"{{"bufferView":{},"mimeType":"image/png"}}"#, view(&mut bin, png.clone(), 0)));
            format!(r#","{key}":{{"index":{}}}"#, images.len() - 1)
        });
        let base = texture(&m.base_color, "baseColorTexture");
        let normal = texture(&m.normal, "normalTexture");
        let mut emissive = texture(&m.emissive, "emissiveTexture");
        if !emissive.is_empty() { emissive += r#","emissiveFactor":[1,1,1]"#; }
        materials.push(format!(r#"{{"name":"{}","pbrMetallicRoughness":{{"metallicFactor":0{base}}}{normal}{emissive}}}"#, m.name));
    }
    for m in &d.meshes {
        let n = m.positions.len();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &m.positions { for k in 0..3 { min[k] = min[k].min(p[k]); max[k] = max[k].max(p[k]); } }
        let v = view(&mut bin, m.positions.iter().flatten().flat_map(|c| c.to_le_bytes()).collect(), 34962);
        accessors.push(format!(r#"{{"bufferView":{v},"componentType":5126,"count":{n},"type":"VEC3","min":[{},{},{}],"max":[{},{},{}]}}"#,
            min[0], min[1], min[2], max[0], max[1], max[2]));
        let mut attributes = format!(r#""POSITION":{}"#, accessors.len() - 1);
        if let Some(normals) = &m.normals {
            let v = view(&mut bin, normals.iter().flatten().flat_map(|c| c.to_le_bytes()).collect(), 34962);
            accessors.push(format!(r#"{{"bufferView":{v},"componentType":5126,"count":{n},"type":"VEC3"}}"#));
            let _ = write!(attributes, r#","NORMAL":{}"#, accessors.len() - 1);
        }
        for (set, uvs) in [&m.uvs, &m.uvs1].into_iter().enumerate() {
            let Some(uvs) = uvs else { continue };
            let v = view(&mut bin, uvs.iter().flatten().flat_map(|c| c.to_le_bytes()).collect(), 34962);
            accessors.push(format!(r#"{{"bufferView":{v},"componentType":5126,"count":{n},"type":"VEC2"}}"#));
            let _ = write!(attributes, r#","TEXCOORD_{set}":{}"#, accessors.len() - 1);
        }
        if let Some(colors) = &m.colors {
            let v = view(&mut bin, colors.concat(), 34962);
            accessors.push(format!(r#"{{"bufferView":{v},"componentType":5121,"normalized":true,"count":{n},"type":"VEC4"}}"#));
            let _ = write!(attributes, r#","_COLOR_0":{}"#, accessors.len() - 1);
        }
        let v = view(&mut bin, m.indices.iter().flat_map(|i| i.to_le_bytes()).collect(), 34963);
        accessors.push(format!(r#"{{"bufferView":{v},"componentType":5125,"count":{},"type":"SCALAR"}}"#, m.indices.len()));
        let material = m.material.map_or(String::new(), |i| format!(r#","material":{i}"#));
        meshes.push(format!(r#"{{"name":"{}","primitives":[{{"attributes":{{{attributes}}},"indices":{}{material}}}]}}"#, m.name, accessors.len() - 1));
    }
    let list = |key: &str, items: Vec<String>| if items.is_empty() { String::new() } else { format!(r#","{key}":[{}]"#, items.join(",")) };
    let textures = list("textures", (0..images.len()).map(|i| format!(r#"{{"source":{i}}}"#)).collect());
    let extra = list("materials", materials) + &textures + &list("images", images);
    let children: Vec<String> = (1..=meshes.len()).map(|i| i.to_string()).collect();
    let mut nodes = vec![format!(r#"{{"name":"drawable","rotation":[-0.70710677,0,0,0.70710677],"children":[{}]}}"#, children.join(","))];
    nodes.extend((0..meshes.len()).map(|i| format!(r#"{{"mesh":{i}}}"#)));
    let uri = uri.map_or(String::new(), |u| format!(r#","uri":"{u}""#));
    let json = format!(
        r#"{{"asset":{{"version":"2.0","generator":"forge9"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{}],"meshes":[{}]{extra},"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}{uri}}}]}}"#,
        nodes.join(","), meshes.join(","), accessors.join(","), views.join(","), bin.len());
    (json, bin)
}

/// Binary glTF container (`to_gltf` pads every buffer view to 4 bytes).
pub fn to_glb(d: &Drawable) -> Vec<u8> {
    let (json, bin) = to_gltf(d, None);
    let mut json = json.into_bytes();
    json.resize(json.len().next_multiple_of(4), b' ');
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    for word in [0x4654_6C67, 2, total as u32, json.len() as u32, 0x4E4F_534A] { out.extend(u32::to_le_bytes(word)); }
    out.extend(json);
    for word in [bin.len() as u32, 0x004E_4942] { out.extend(u32::to_le_bytes(word)); }
    out.extend(bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(buf: &mut Vec<u8>, at: usize, bytes: &[u8]) {
        if buf.len() < at + bytes.len() { buf.resize(at + bytes.len(), 0); }
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn alloc(buf: &mut Vec<u8>, size: usize) -> usize {
        let at = buf.len().next_multiple_of(16);
        buf.resize(at + size, 0);
        at
    }
    fn sp(off: usize) -> [u8; 8] { (VIRTUAL_BASE + off as u64).to_le_bytes() }
    fn gp(off: usize) -> [u8; 8] { (PHYSICAL_BASE + off as u64).to_le_bytes() }
    fn f32s(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() }
    /// Normals of the quantized fixture, written as unorm10 octahedral words.
    const NORMALS: [[f32; 3]; 4] = [[0.0, 0.0, 1.0], [0.0, 0.0, -1.0], [0.6, 0.0, 0.8], [0.0, -0.6, -0.8]];
    fn oct10(n: [f32; 3]) -> u32 {
        let l1 = n[0].abs() + n[1].abs() + n[2].abs();
        let (x, y) = (n[0] / l1, n[1] / l1);
        let (x, y) = if n[2] < 0.0 { ((1.0 - y.abs()).copysign(x), (1.0 - x.abs()).copysign(y)) } else { (x, y) };
        let q = |c: f32| ((c + 1.0) * 511.5).round() as u32;
        q(x) | q(y) << 10 | 0xA << 28
    }

    /// A quad geometry: `format` 66 (quantized + float2 UVs + normals), 99 (float3), or anything else.
    fn geometry(sys: &mut Vec<u8>, gfx: &mut Vec<u8>, format: u32) -> usize {
        let mut v = vec![0u8; 96];
        let (at, size, lfmt) = if format == 66 { (128, 8u8, 16u8) } else { (96, 12, 6) };
        put(&mut v, 0, &(format | (at as u32) << 7).to_le_bytes());
        if format == 66 {
            put(&mut v, 96, &f32s(&[10.0, 20.0, 30.0]));
            put(&mut v, 108, &2u32.to_le_bytes());
            put(&mut v, 112, &f32s(&[0.5, 0.5, 0.5]));
            for (i, [x, y, z]) in [[0u64, 0, 0], [2, 0, 0], [2, 2, 0], [0, 2, 4]].into_iter().enumerate() {
                let q = x << 8 | (y & 0xFF) | (y >> 8) << 32 | z << 48;
                put(&mut v, at + i * 8, &q.to_le_bytes());
            }
            put(&mut v, 12, &(160u32 << 7 | 8 << 3 | 2).to_le_bytes());
            put(&mut v, 160, &f32s(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]));
            put(&mut v, 8, &(192u32 << 7 | 4 << 3 | 1).to_le_bytes());
            for (i, n) in NORMALS.into_iter().enumerate() { put(&mut v, 192 + i * 4, &oct10(n).to_le_bytes()); }
            put(&mut v, 16, &(208u32 << 7 | 4 << 3 | 6).to_le_bytes());
            for i in 0..4u8 { put(&mut v, 208 + i as usize * 4, &[255, 255, 20 + i, 255]); }
        } else {
            put(&mut v, at, &f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0]));
        }
        v.resize(v.len().next_multiple_of(16), 0);
        let vdata = alloc(gfx, v.len());
        put(gfx, vdata, &v);

        // Header {flags|clusters, triangles, refs}, one 16-byte cluster, 2 triangles, 4 u16 refs in reverse.
        let mut ix = Vec::new();
        for w in [0x8000_0001u32, 2, 4, 3 | 1 << 7, 0, 0, 0, 1 << 7 | 2 << 14, 2 << 7 | 3 << 14] { ix.extend(w.to_le_bytes()); }
        for r in [3u16, 2, 1, 0] { ix.extend(r.to_le_bytes()); }
        ix.resize(48, 0);
        let idata = alloc(gfx, ix.len());
        put(gfx, idata, &ix);

        let g = alloc(sys, 160);
        let vb = alloc(sys, 80);
        let layout = alloc(sys, 320);
        let desc = alloc(sys, 16);
        let ib = alloc(sys, 64);
        put(sys, g + 8, &sp(vb));
        put(sys, g + 64, &sp(desc));
        put(sys, vb + 8, &(v.len() as u32 / 16).to_le_bytes());
        put(sys, vb + 12, &16u16.to_le_bytes());
        put(sys, vb + 24, &gp(vdata));
        put(sys, vb + 64, &sp(layout));
        put(sys, layout, &(at as u32).to_le_bytes());
        put(sys, layout + 0xD0, &[size]);
        put(sys, layout + 0x104, &[lfmt]);
        if format == 66 {
            for (sem, at, size, format) in [(UV_SEMANTICS[0], 160u32, 8, 16), (NORMAL_SEMANTIC, 192, 4, 41), (COLOR_SEMANTIC, 208, 4, 28)] {
                put(sys, layout + sem * 4, &at.to_le_bytes());
                put(sys, layout + 0xD0 + sem, &[size]);
                put(sys, layout + 0x104 + sem, &[format]);
            }
        }
        put(sys, layout + 0x138, &(4u32 << 10 | 1).to_le_bytes());
        put(sys, desc, &sp(ib));
        put(sys, desc + 8, &1u16.to_le_bytes());
        put(sys, desc + 10, &2u16.to_le_bytes());
        put(sys, desc + 12, &(4u32 << 8).to_le_bytes());
        put(sys, ib + 8, &(ix.len() as u32 / 4).to_le_bytes());
        put(sys, ib + 12, &4u16.to_le_bytes());
        put(sys, ib + 24, &gp(idata));
        g
    }

    /// System and graphics blocks of a one-model drawable with quantized, float3 and unsupported geometries,
    /// standalone or as the single entry of a drawable dictionary (offsets as seen in real dictionaries).
    fn synthetic(dict: bool) -> Vec<Vec<u8>> {
        let (mut sys, mut gfx) = (vec![0u8; if dict { 64 } else { 192 }], Vec::new());
        let d = if dict { alloc(&mut sys, 192) } else { 0 };
        put(&mut sys, d + 44, &f32s(&[2.0, 10.0, 20.0, 30.0]));
        put(&mut sys, d + 64, &f32s(&[11.0, 21.0, 32.0]));
        let pages = alloc(&mut sys, 16);
        put(&mut sys, pages + 8, &[1, 1]);
        let shaders = shader_group(&mut sys);
        let name = alloc(&mut sys, 16);
        put(&mut sys, name, b"synthetic_prop\0");
        let group = alloc(&mut sys, 16);
        let list = alloc(&mut sys, 8);
        let model = alloc(&mut sys, 56);
        let geoms = alloc(&mut sys, 24);
        for (i, f) in [66, 99, 48].into_iter().enumerate() {
            let g = geometry(&mut sys, &mut gfx, f);
            put(&mut sys, geoms + i * 8, &sp(g));
        }
        put(&mut sys, 8, &sp(pages));
        for (at, p) in [(16, shaders), (80, group), (168, name)] { put(&mut sys, d + at, &sp(p)); }
        if dict {
            let (hashes, values) = (alloc(&mut sys, 4), alloc(&mut sys, 8));
            put(&mut sys, hashes, &0x1234_5678u32.to_le_bytes());
            put(&mut sys, values, &sp(d));
            put(&mut sys, 24, &1u32.to_le_bytes());
            for (at, p) in [(32, hashes), (48, values)] {
                put(&mut sys, at, &sp(p));
                put(&mut sys, at + 8, &[1, 0, 1, 0]);
            }
        }
        put(&mut sys, group, &sp(list));
        put(&mut sys, group + 8, &[1, 0, 1, 0]);
        put(&mut sys, list, &sp(model));
        put(&mut sys, model + 8, &sp(geoms));
        put(&mut sys, model + 16, &[3, 0, 3, 0]);
        let shader_ix = alloc(&mut sys, 6);
        put(&mut sys, model + 32, &sp(shader_ix));
        vec![sys, gfx]
    }

    /// One shader with `diffusetex` (slot 0), an unbound `tintpalettetex` (slot 1) and `bumptex` (slot 2).
    fn shader_group(sys: &mut Vec<u8>) -> usize {
        let (group, list, shader) = (alloc(sys, 64), alloc(sys, 8), alloc(sys, 64));
        let (table, slots) = (alloc(sys, 32), alloc(sys, 24));
        put(sys, group + 16, &sp(list));
        put(sys, group + 24, &[1, 0, 1, 0]);
        put(sys, list, &sp(shader));
        put(sys, shader, &0xabcd_0001u32.to_le_bytes());
        put(sys, shader + 16, &sp(slots));
        put(sys, shader + 32, &sp(table));
        put(sys, table, &[1, 3, 0, 0, 3]);
        for (k, (param, slot)) in [(DIFFUSE_PARAM, 0u32), (0x2c8d_7182, 4), (BUMP_PARAM, 8)].into_iter().enumerate() {
            put(sys, table + 8 + k * 8, &param.to_le_bytes());
            put(sys, table + 12 + k * 8, &slot.to_le_bytes());
            if k == 1 { continue; }
            let (tex, name, rec) = (alloc(sys, 80), alloc(sys, 16), alloc(sys, 16));
            put(sys, slots + slot as usize * 2, &sp(tex));
            put(sys, tex + 48, &sp(name));
            put(sys, name, &sp(rec + 5));
            put(sys, name + 8, &(0x1111_2222_3333_4444u64 * (k as u64 + 1)).to_le_bytes());
            put(sys, rec, &6u64.to_le_bytes());
            put(sys, rec + 8, &(0x5000 + k as u32).to_le_bytes());
        }
        group
    }

    #[test]
    fn decodes_shader_textures_and_colors() {
        let mut d = parse_blocks(&synthetic(true), 0).unwrap();
        assert_eq!(d.materials.len(), 1);
        let m = &d.materials[0];
        assert_eq!((m.name.as_str(), m.shader), ("synthetic_prop/shader_0_abcd0001", 0xabcd_0001));
        assert_eq!(m.textures, [
            TextureRef { param: DIFFUSE_PARAM, name_hash: 0x5000, key: 0x1111_2222_3333_4444 },
            TextureRef { param: BUMP_PARAM, name_hash: 0x5002, key: 0x3333_6666_9999_cccc },
        ]);
        assert!(d.meshes.iter().all(|m| m.material == Some(0)));
        assert_eq!(d.meshes[0].colors.as_deref(), Some(&[[255, 255, 20, 255], [255, 255, 21, 255], [255, 255, 22, 255], [255, 255, 23, 255]][..]));
        assert!(d.meshes[1].colors.is_none());

        let json = to_gltf(&d, None).0;
        assert!(json.contains(r#""_COLOR_0":3"#) && json.contains(r#""material":0"#) && !json.contains("images"), "{json}");
        d.materials[0].base_color = Some(b"\x89PNG1".to_vec());
        d.materials[0].normal = Some(b"\x89PNG22".to_vec());
        let (json, bin) = to_gltf(&d, None);
        assert!(json.contains(r#""pbrMetallicRoughness":{"metallicFactor":0,"baseColorTexture":{"index":0}},"normalTexture":{"index":1}"#), "{json}");
        assert!(json.contains(r#""textures":[{"source":0},{"source":1}],"images":[{"bufferView":0,"mimeType":"image/png"},{"bufferView":1,"mimeType":"image/png"}]"#), "{json}");
        assert!(json.contains(r#"{"buffer":0,"byteOffset":0,"byteLength":5},{"buffer":0,"byteOffset":8,"byteLength":6},{"buffer":0,"byteOffset":16,"#), "{json}");
        assert_eq!(&bin[8..14], b"\x89PNG22");
        d.materials[0].emissive = Some(b"\x89PNG3".to_vec());
        let json = to_gltf(&d, None).0;
        assert!(json.contains(r#""normalTexture":{"index":1},"emissiveTexture":{"index":2},"emissiveFactor":[1,1,1]}"#), "{json}");
        // Merged drawables keep each mesh on its own material.
        let two = merge("pair".into(), vec![parse_blocks(&synthetic(true), 0).unwrap(), d]);
        assert_eq!((two.name.as_str(), two.meshes.len(), two.materials.len()), ("pair", 4, 2));
        assert_eq!(two.meshes.iter().map(|m| m.material).collect::<Vec<_>>(), [Some(0), Some(0), Some(1), Some(1)]);
        assert!(to_gltf(&two, None).0.contains(r#""material":1"#));
        // A broken shader group drops the materials, not the geometry.
        let mut blocks = synthetic(false);
        put(&mut blocks[0], 16, &sp(1 << 20));
        let d = parse_blocks(&blocks, 0).unwrap();
        assert!(d.materials.is_empty() && d.meshes.iter().all(|m| m.material.is_none()) && d.skipped[0].starts_with("shader group"), "{:?}", d.skipped);
    }

    #[test]
    fn decodes_synthetic_dictionary() {
        let blocks = synthetic(true);
        assert!(!looks_like(&blocks[0]) && dictionary(&blocks[0]) == Some(vec![64]));
        assert_eq!(page_counts(&blocks[0]), Some((1, 1)));
        assert_eq!(labels(&blocks.concat()), ["synthetic_prop"]);
        assert_eq!(labels(&synthetic(false).concat()), ["synthetic_prop"]);
        assert!(labels(&blocks[1]).is_empty());
        let d = parse_blocks(&blocks, 0).unwrap();
        assert_eq!((d.meshes.len(), d.skipped.len()), (2, 1));
        assert_eq!(d.meshes[0].name, "synthetic_prop/model_0_geometry_0");
        assert_eq!(d.meshes[0].indices, [3, 2, 1, 3, 1, 0]);
        assert_eq!(d.meshes[1].positions[2], [1.0, 1.0, 0.0]);
        // A resource without graphics pages (e.g. animation or metadata) is classified, not misparsed.
        let mut meta = vec![0u8; 64];
        put(&mut meta, 8, &sp(32));
        put(&mut meta, 40, &[2, 0]);
        let e = parse_blocks(&[meta], 0).unwrap_err().to_string();
        assert!(e.contains("2 system and no graphics pages"), "{e}");
    }

    /// Blocks of entry `inner` of nested archive `outer` of the story content archive, when present locally.
    fn real_entry(outer: usize, inner: usize) -> Option<Vec<Vec<u8>>> {
        use crate::rpf9::reader::Archive;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let (Ok(raw), Ok(crypto)) = (std::fs::read(root.join("bin/story000_restofworld.rpf")), crate::rpf9::crypto::CryptoContext::load(&root.join("data"))) else {
            eprintln!("skipping: bin/story000_restofworld.rpf or data/*.bin not present");
            return None;
        };
        let archive = Archive::open(&raw, &crypto).unwrap();
        let nested = archive.read_entry(&archive.entries()[outer]).unwrap();
        let nested = Archive::open(&nested, &crypto).unwrap();
        Some(nested.read_entry_blocks(&nested.entries()[inner]).unwrap())
    }

    /// Real-file check: the only drawable resource of the story content archive.
    #[test]
    fn decodes_real_dictionary() {
        let Some(blocks) = real_entry(19, 0) else { return };
        let members = dictionary(&blocks[0]).unwrap();
        let d = parse_blocks(&blocks, 0).unwrap();
        assert_eq!((members.len(), d.meshes.len(), d.skipped.len()), (3, 3, 0));
        let verts: usize = d.meshes.iter().map(|m| m.positions.len()).sum();
        let tris: usize = d.meshes.iter().map(|m| m.indices.len() / 3).sum();
        assert_eq!((verts, tris), (1808, 2620));
        // One shader per drawable, all binding the same four textures; colors are RGBA8 with R = G = A = 255.
        assert_eq!(d.materials.len(), 3);
        let params: Vec<u32> = d.materials[0].textures.iter().map(|t| t.param).collect();
        assert_eq!(params, [DIFFUSE_PARAM, 0x739e_3d0d, 0xc6f4_6e79, BUMP_PARAM]);
        assert!(d.materials.iter().all(|m| m.shader == 0xbeac_bf33 && m.textures == d.materials[0].textures));
        assert!(d.meshes.iter().enumerate().all(|(i, m)| m.material == Some(i)));
        assert!(d.meshes.iter().all(|m| m.colors.as_ref().unwrap().iter().all(|c| c[0] == 255 && c[1] == 255 && c[3] == 255)));
        for (m, &at) in d.meshes.iter().zip(&members) {
            let n = m.positions.len() as u32;
            assert!(m.indices.iter().all(|&i| i < n));
            // Every vertex lies inside its drawable's header bounds, up to quantization error.
            let (min, max) = (vec3(&blocks[0], at + 48).unwrap(), vec3(&blocks[0], at + 64).unwrap());
            for p in &m.positions {
                assert!((0..3).all(|k| p[k] >= min[k] - 1e-3 && p[k] <= max[k] + 1e-3), "{} {p:?}", m.name);
            }
        }
    }

    /// Real-file check of the normal and texcoord decode on the same dictionary.
    #[test]
    fn decodes_real_normals_and_texcoords() {
        let Some(blocks) = real_entry(19, 0) else { return };
        let d = parse_blocks(&blocks, 0).unwrap();
        for (mi, m) in d.meshes.iter().enumerate() {
            // Normals: unit length, and on the same side as the area-weighted face normals for >= 97% of vertices.
            let normals = m.normals.as_ref().unwrap();
            assert!(normals.iter().all(|v| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-5));
            let mut face = vec![[0f32; 3]; normals.len()];
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [0, 1, 2].map(|k| m.positions[t[k] as usize]);
                let (u, w) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
                let cross = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                for &i in t { for k in 0..3 { face[i as usize][k] += cross[k]; } }
            }
            let agree = normals.iter().zip(&face).filter(|(v, f)| (0..3).map(|k| v[k] * f[k]).sum::<f32>() > 0.0).count();
            assert!(agree * 100 >= normals.len() * 97, "{}: {agree}/{} normals agree with faces", m.name, normals.len());
            // Texcoords (snorm16 x 4.0): set 1 everywhere and set 0 of the jaw and fragment lie in 0..1; set 0
            // of skull_01 spans -1.04..1.94 (neighbouring tiles, no wrap seam inside a triangle).
            let in_unit = |uv: &Vec<[f32; 2]>| uv.iter().flatten().all(|c| (0.0..=1.0).contains(c));
            let uv0 = m.uvs.as_ref().unwrap();
            assert!(in_unit(m.uvs1.as_ref().unwrap()) && (mi == 0 || in_unit(uv0)), "{}", m.name);
            assert!(uv0.iter().flatten().all(|c| (-1.05..=1.95).contains(c)));
        }
    }

    /// Real-file check: the particle effect list next to the dictionary has objects with a
    /// `{ptr, count, capacity}` list at +0x50 that used to pass the header check.
    #[test]
    fn rejects_real_effect_objects() {
        let Some(blocks) = real_entry(20, 0) else { return };
        let sys = blocks[..2].concat();
        assert!((0..sys.len()).step_by(16).all(|o| !header_at(&sys, o, false) && !header_at(&sys, o, true)));
        assert!(parse_blocks(&blocks, 0).is_err());
    }

    #[test]
    fn keeps_stored_buffers() {
        let blocks = synthetic(false);
        let d = parse_blocks(&blocks, 0).unwrap();
        let b = &d.meshes[0].buffers;
        assert_eq!((b.vertex.len(), b.elements, b.element_stride, b.position_format, b.position_offset), (224, 14, 16, 66, 128));
        assert_eq!(b.quantization, Some(Quantization { bias: [10.0, 20.0, 30.0], axis: 2, scale: [0.5; 3] }));
        let sems: Vec<(usize, u32, u8)> = b.attributes.iter().map(|a| (a.semantic, a.offset, a.size)).collect();
        assert_eq!(sems, [(0, 128, 8), (NORMAL_SEMANTIC, 192, 4), (COLOR_SEMANTIC, 208, 4), (UV_SEMANTICS[0], 160, 8)]);
        assert!(b.attributes.iter().all(|a| a.descriptor.is_some()));
        assert_eq!((b.index.len(), b.ref_size, b.triangles_offset, b.refs_offset, &b.refs[..]), (48, 2, 28, 36, &[3, 2, 1, 0][..]));
        assert_eq!(b.meshlets, [Meshlet { record: [3 | 1 << 7, 0, 0, 0], vertices: 4, triangles: 2, first_ref: 0, first_triangle: 0 }]);
        assert_eq!(decode_positions(&b.vertex, 66, 128, 4).unwrap(), d.meshes[0].positions);
        assert_eq!(d.meshes[1].buffers.position_format, 99);
        // The reference dump of the same drawable.
        let dir = std::env::temp_dir().join(format!("forge9-dump-{}", std::process::id()));
        let refs = super::super::geometry_check::dump(&d, &dir).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(std::fs::read(dir.join("g0.vertex.bin")).unwrap(), b.vertex);
        assert_eq!(std::fs::read(dir.join("g0.meshlets.bin")).unwrap(), &b.index[12..28]);
        let r: super::super::geometry_check::Reference = serde_json::from_slice(&std::fs::read(dir.join("g0.json")).unwrap()).unwrap();
        assert_eq!((r.triangle_count, r.meshlets.len(), r.positions, r.indices), (2, 1, d.meshes[0].positions.clone(), vec![3, 2, 1, 3, 1, 0]));
        assert_eq!((r.meshlets[0].aabb.min, r.meshlets[0].aabb.max), ([10.0, 20.0, 30.0], [11.0, 21.0, 32.0]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decodes_octahedral_normals() {
        let d = parse_blocks(&synthetic(false), 0).unwrap();
        for (n, want) in d.meshes[0].normals.as_ref().unwrap().iter().zip(NORMALS) {
            assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-5, "{n:?} not unit");
            assert!((0..3).all(|k| (n[k] - want[k]).abs() < 4e-3), "{n:?} != {want:?}");
        }
        assert!(d.meshes[1].normals.is_none());
        let json = to_gltf(&d, None).0;
        assert!(json.contains(r#""POSITION":0,"NORMAL":1,"TEXCOORD_0":2"#), "{json}");
        let obj = to_obj(&d);
        assert_eq!(obj.matches("\nvn ").count(), 4);
        assert!(obj.contains("f 4/4/4 3/3/3 2/2/2") && obj.contains("f 8 6 5"), "{obj}");
    }

    #[test]
    fn rejects_list_in_lod_slot() {
        let mut blocks = synthetic(false);
        assert!(looks_like(&blocks[0]));
        put(&mut blocks[0], 88, &0x0002_0002u64.to_le_bytes());
        assert!(!looks_like(&blocks[0]));
    }

    #[test]
    fn decodes_synthetic_drawable() {
        let d = parse_blocks(&synthetic(false), 0).unwrap();
        assert_eq!((d.name.as_str(), d.meshes.len(), d.skipped.len()), ("synthetic_prop", 2, 1));
        assert!(d.skipped[0].contains("unsupported position format 48"), "{}", d.skipped[0]);
        let q = &d.meshes[0];
        assert_eq!(q.positions, [[10.0, 20.0, 30.0], [11.0, 20.0, 30.0], [11.0, 21.0, 30.0], [10.0, 21.0, 32.0]]);
        assert_eq!(q.uvs.as_deref(), Some(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][..]));
        assert!(q.uvs1.is_none());
        // refs table is reversed, so local (0,1,2),(0,2,3) remaps to (3,2,1),(3,1,0)
        assert_eq!(q.indices, [3, 2, 1, 3, 1, 0]);
        let f = &d.meshes[1];
        assert_eq!((f.positions[2], f.uvs.is_none()), ([1.0, 1.0, 0.0], true));
        assert!(parse_blocks(&synthetic(false), 1).unwrap_err().to_string().contains("no LOD 1"));

        let glb = to_glb(&d);
        assert_eq!((&glb[0..4], u32_at(&glb, 8).unwrap() as usize), (&b"glTF"[..], glb.len()));
        let json_len = u32_at(&glb, 12).unwrap() as usize;
        let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(json.contains(r#""TEXCOORD_0":2"#) && json.contains(r#""min":[10,20,30],"max":[11,21,32]"#), "{json}");
        assert_eq!(u32_at(&glb, 20 + json_len).unwrap() as usize, glb.len() - 28 - json_len);
        let obj = to_obj(&d);
        assert_eq!((obj.matches("\nv ").count(), obj.matches("\nvt ").count(), obj.matches("\nvn ").count(), obj.matches("\nf ").count()), (8, 4, 4, 4));
        assert!(obj.contains("f 4/4/4 3/3/3 2/2/2"), "{obj}");

        // `FORGE9_FIXTURE_OUT=path` writes the entry as a zlib block chain, for `repack` + `model` end-to-end runs.
        if let Ok(path) = std::env::var("FORGE9_FIXTURE_OUT") {
            use std::io::Write;
            let mut out = Vec::new();
            for block in synthetic(false) {
                let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(&block).unwrap();
                out.extend(z.finish().unwrap());
                out.resize(out.len().next_multiple_of(16), 0);
            }
            std::fs::write(path, out).unwrap();
        }
    }
}
