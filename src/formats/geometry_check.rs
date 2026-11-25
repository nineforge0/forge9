//! Reference dumps of drawable geometry, and checks of position buffers captured from a run against them.
//!
//! `dump` writes, per geometry `g<i>`: `g<i>.vertex.bin` and `g<i>.index.bin` (the stored vertex and
//! index buffers, byte for byte), `g<i>.meshlets.bin` (the index buffer's 16-byte meshlet records) and
//! `g<i>.json` (`Reference`: layout, quantization, decoded positions, triangles, meshlets, AABBs).
//! `check` compares a captured buffer with a `Reference`. Accepted layouts (`Layout`):
//!
//! - `stored`: the vertex buffer as stored (same bytes as `g<i>.vertex.bin`); positions decoded with
//!   the capture's own descriptor and quantization header
//! - `float3`: model-space f32x3 positions, `stride` bytes apart
//! - `clip`: post-transform f32x4 clip positions (x, y, z, w), `stride` bytes apart; compared with the
//!   reference through a least-squares fit `clip = M * (x, y, z, 1)` of one 4x4 matrix
//!
//! Elements are in mesh vertex order, or meshlet order (element `k` is `refs[k]`, or with
//! `meshlet_slots = n`, element `m * n + j` is local vertex `j` of meshlet `m`).
//! Format details and a worked example: docs/geometry_check.md.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::Path;
use super::drawable::{self, Drawable, Mesh};

pub const FORMAT: &str = "forge9-geometry-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Aabb { pub min: [f32; 3], pub max: [f32; 3] }

impl Aabb {
    fn of<'a>(points: impl IntoIterator<Item = &'a [f32; 3]>) -> Aabb {
        let mut b = Aabb { min: [f32::MAX; 3], max: [f32::MIN; 3] };
        for p in points { for k in 0..3 { b.min[k] = b.min[k].min(p[k]); b.max[k] = b.max[k].max(p[k]); } }
        b
    }
    /// Distance outside the box (0 inside).
    fn outside(&self, p: [f64; 3]) -> f64 {
        (0..3).map(|k| (f64::from(self.min[k]) - p[k]).max(p[k] - f64::from(self.max[k])).max(0.0)).fold(0.0, f64::max)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Files { pub vertex: String, pub index: String, pub meshlets: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    pub semantic: usize,
    pub name: String,
    /// Byte offset of the stream in the vertex buffer; the stream holds `vertex_count` elements of `size` bytes.
    pub offset: u32,
    pub size: u8,
    pub dxgi_format: u8,
    pub descriptor: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantization {
    pub bias: [f32; 3],
    pub axis_order: u32,
    pub scale: [f32; 3],
    /// Lane widths of X, Y, Z in the 8-byte element.
    pub lane_bits: [u32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// 66: 8-byte quantized; 99: float3.
    pub format: u32,
    pub offset: usize,
    pub stride: usize,
    pub quantization: Option<Quantization>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexBuffer {
    pub bytes: usize,
    pub elements: u32,
    pub element_stride: u16,
    /// The 16 u32 attribute descriptors at the start of the buffer.
    pub descriptors: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBuffer {
    pub bytes: usize,
    pub meshlet_records_offset: usize,
    pub triangles_offset: usize,
    pub refs_offset: usize,
    pub ref_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meshlet {
    pub index: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub first_ref: usize,
    pub first_triangle: usize,
    /// The stored 16-byte record as four u32 (words 2 and 3 are not decoded).
    pub record: [u32; 4],
    pub aabb: Aabb,
}

/// Contents of `g<i>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub format: String,
    pub drawable: String,
    pub lod: usize,
    pub geometry: String,
    pub files: Files,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub aabb: Aabb,
    pub vertex_buffer: VertexBuffer,
    pub attributes: Vec<Attribute>,
    pub position: Position,
    pub texcoord_scale: Option<f32>,
    pub index_buffer: IndexBuffer,
    pub meshlets: Vec<Meshlet>,
    /// Meshlet-local vertex to mesh vertex, concatenated over meshlets.
    pub meshlet_refs: Vec<u32>,
    /// Triangle list in mesh vertex indices, meshlet by meshlet.
    pub indices: Vec<u32>,
    /// Decoded model-space positions in mesh vertex order.
    pub positions: Vec<[f32; 3]>,
}

fn semantic_name(s: usize) -> &'static str {
    match s { 0 => "position", 4 => "normal", 24 => "color", 28 => "texcoord0", 29 => "texcoord1", _ => "unknown" }
}

pub fn reference(d: &Drawable, m: &Mesh, prefix: &str) -> Reference {
    let b = &m.buffers;
    let meshlets = b.meshlets.iter().enumerate().map(|(index, l)| Meshlet {
        index, vertices: l.vertices, triangles: l.triangles, first_ref: l.first_ref, first_triangle: l.first_triangle, record: l.record,
        aabb: Aabb::of(b.refs[l.first_ref..l.first_ref + l.vertices].iter().map(|&v| &m.positions[v as usize])),
    }).collect();
    Reference {
        format: FORMAT.into(),
        drawable: d.name.clone(),
        lod: d.lod,
        geometry: m.name.clone(),
        files: Files { vertex: format!("{prefix}.vertex.bin"), index: format!("{prefix}.index.bin"), meshlets: format!("{prefix}.meshlets.bin") },
        vertex_count: m.positions.len(),
        triangle_count: m.indices.len() / 3,
        aabb: Aabb::of(&m.positions),
        vertex_buffer: VertexBuffer { bytes: b.vertex.len(), elements: b.elements, element_stride: b.element_stride,
            descriptors: (0..16).filter_map(|i| super::u32_at(&b.vertex, i * 4)).collect() },
        attributes: b.attributes.iter().map(|a| Attribute { semantic: a.semantic, name: semantic_name(a.semantic).into(),
            offset: a.offset, size: a.size, dxgi_format: a.format, descriptor: a.descriptor }).collect(),
        position: Position { format: b.position_format, offset: b.position_offset, stride: if b.position_format == 99 { 12 } else { 8 },
            quantization: b.quantization.map(|q| Quantization { bias: q.bias, axis_order: q.axis, scale: q.scale, lane_bits: [24, 24, 16] }) },
        texcoord_scale: b.texcoord_scale,
        index_buffer: IndexBuffer { bytes: b.index.len(), meshlet_records_offset: 12, triangles_offset: b.triangles_offset,
            refs_offset: b.refs_offset, ref_size: b.ref_size },
        meshlets,
        meshlet_refs: b.refs.clone(),
        indices: m.indices.clone(),
        positions: m.positions.clone(),
    }
}

/// Writes the files of every geometry of `d` to `dir`; returns `(prefix, reference)` per geometry.
pub fn dump(d: &Drawable, dir: &Path) -> Result<Vec<(String, Reference)>> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    d.meshes.iter().enumerate().map(|(i, m)| {
        let prefix = format!("g{i}");
        let r = reference(d, m, &prefix);
        let b = &m.buffers;
        let records = &b.index[12..12 + b.meshlets.len() * 16];
        for (name, bytes) in [(&r.files.vertex, &b.vertex[..]), (&r.files.index, &b.index[..]), (&r.files.meshlets, records)] {
            std::fs::write(dir.join(name), bytes)?;
        }
        std::fs::write(dir.join(format!("{prefix}.json")), serde_json::to_string_pretty(&r)?)?;
        Ok((prefix, r))
    }).collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout { Stored, Float3, Clip }

#[derive(Debug, Clone)]
pub struct Options {
    pub layout: Layout,
    /// Bytes between elements (float3 and clip; default 12 and 16).
    pub stride: Option<usize>,
    /// Byte offset of element 0 (of the vertex buffer start for `stored`).
    pub offset: usize,
    /// Elements to check (default: every vertex, or every meshlet slot).
    pub count: Option<usize>,
    pub meshlet_order: bool,
    pub meshlet_slots: Option<usize>,
    /// Model-space tolerance for the reference and AABB comparisons.
    pub tolerance: f64,
    /// Clip-space tolerance, in units of |w| (NDC), for the fit residual.
    pub clip_tolerance: f64,
    /// |x/w| or |y/w| above this is outlying.
    pub ndc_limit: f64,
    pub top: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { layout: Layout::Clip, stride: None, offset: 0, count: None, meshlet_order: false, meshlet_slots: None,
            tolerance: 1e-3, clip_tolerance: 1e-3, ndc_limit: 16.0, top: 20 }
    }
}

// Flags, most severe first.
const NON_FINITE: u8 = 1;
const W_NOT_POSITIVE: u8 = 2;
const OUTLYING: u8 = 4;
const OUTSIDE_AABB: u8 = 8;
const DIFFERS: u8 = 16;
const BAD: u8 = NON_FINITE | W_NOT_POSITIVE | OUTLYING | OUTSIDE_AABB | DIFFERS;

#[derive(Debug, Clone)]
pub struct Element {
    pub element: usize,
    pub vertex: usize,
    pub meshlet: usize,
    pub local: usize,
    pub flags: u8,
    /// Captured value (x, y, z, w; w = 1 for model-space layouts).
    pub value: [f32; 4],
    /// Reference position, or the fitted clip position.
    pub expected: [f64; 4],
    /// Model-space distance, or clip residual in units of |w|.
    pub error: f64,
    pub aabb_distance: f64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub text: String,
    pub checked: usize,
    pub non_finite: usize,
    pub w_not_positive: usize,
    pub outlying: usize,
    pub outside_clip: usize,
    pub outside_aabb: usize,
    pub differs: usize,
    pub bad: usize,
    pub fit: Option<[[f64; 4]; 4]>,
    pub triangles_with_bad: usize,
    pub screens_covered: f64,
    pub large_triangles: usize,
    pub worst: Vec<Element>,
}

fn f32_at(d: &[u8], at: usize) -> f32 { f32::from_bits(super::u32_at(d, at).unwrap()) }

/// Solves `a x = b` (4x4, partial pivoting); `None` when singular.
fn solve4(mut a: [[f64; 4]; 4], mut b: [f64; 4]) -> Option<[f64; 4]> {
    for c in 0..4 {
        let p = (c..4).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 * a.iter().flatten().fold(1e-300f64, |m, v| m.max(v.abs())) { return None; }
        a.swap(c, p);
        b.swap(c, p);
        for r in 0..4 {
            if r == c { continue; }
            let f = a[r][c] / a[c][c];
            for k in 0..4 { a[r][k] -= f * a[c][k]; }
            b[r] -= f * b[c];
        }
    }
    Some([0, 1, 2, 3].map(|i| b[i] / a[i][i]))
}

/// Least-squares `clip = M * (p, 1)` over `samples`; rows of M.
fn fit(samples: &[([f64; 3], [f64; 4])]) -> Option<[[f64; 4]; 4]> {
    let mut ata = [[0f64; 4]; 4];
    let mut aty = [[0f64; 4]; 4];
    for (p, c) in samples {
        let a = [p[0], p[1], p[2], 1.0];
        for i in 0..4 { for j in 0..4 { ata[i][j] += a[i] * a[j]; aty[j][i] += a[i] * c[j]; } }
    }
    let mut m = [[0f64; 4]; 4];
    for (row, y) in m.iter_mut().zip(aty) { *row = solve4(ata, y)?; }
    Some(m)
}

fn apply(m: &[[f64; 4]; 4], p: [f64; 4]) -> [f64; 4] { m.map(|r| (0..4).map(|k| r[k] * p[k]).sum()) }

fn invert(m: &[[f64; 4]; 4]) -> Option<[[f64; 4]; 4]> {
    let mut cols = [[0f64; 4]; 4];
    for (c, col) in cols.iter_mut().enumerate() {
        let mut e = [0f64; 4];
        e[c] = 1.0;
        *col = solve4(*m, e)?;
    }
    Some([0, 1, 2, 3].map(|r| [0, 1, 2, 3].map(|c| cols[c][r])))
}

/// Area of triangle `t` (NDC) inside the [-1, 1] square.
fn clipped_area(t: [[f64; 2]; 3]) -> f64 {
    let mut poly: Vec<[f64; 2]> = t.to_vec();
    for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
        let inside = |p: &[f64; 2]| sign * p[axis] <= 1.0;
        let mut out = Vec::new();
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if inside(&a) { out.push(a); }
            if inside(&a) != inside(&b) {
                let s = (sign - a[axis]) / (b[axis] - a[axis]);
                out.push([a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])]);
            }
        }
        poly = out;
        if poly.is_empty() { return 0.0; }
    }
    (0..poly.len()).map(|i| { let (a, b) = (poly[i], poly[(i + 1) % poly.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>().abs() / 2.0
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() { return 0.0; }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Checks `capture` against `r`. `stored_vertex` is the reference's stored vertex buffer, for a byte
/// comparison in the `stored` layout.
pub fn check(r: &Reference, capture: &[u8], stored_vertex: Option<&[u8]>, o: &Options) -> Result<Report> {
    ensure!(r.format == FORMAT, "reference format {:?} is not {FORMAT:?}", r.format);
    ensure!(r.positions.len() == r.vertex_count && r.meshlet_refs.iter().all(|&v| (v as usize) < r.vertex_count), "reference is inconsistent");
    // Element -> (vertex, meshlet, local); vertex order takes the first meshlet that references the vertex.
    let mut first = vec![(usize::MAX, 0); r.vertex_count];
    for m in r.meshlets.iter().rev() {
        for j in 0..m.vertices { first[r.meshlet_refs[m.first_ref + j] as usize] = (m.index, j); }
    }
    let slots = |m: &Meshlet| o.meshlet_slots.map_or(m.first_ref, |n| m.index * n);
    let mut map: Vec<Option<(usize, usize, usize)>> = if o.meshlet_order {
        let total = r.meshlets.iter().map(|m| slots(m) + o.meshlet_slots.unwrap_or(m.vertices)).max().unwrap_or(0);
        let mut map = vec![None; total];
        for m in &r.meshlets {
            ensure!(o.meshlet_slots.is_none_or(|n| m.vertices <= n), "meshlet {} has {} vertices, more than {} slots", m.index, m.vertices, o.meshlet_slots.unwrap());
            for j in 0..m.vertices { map[slots(m) + j] = Some((r.meshlet_refs[m.first_ref + j] as usize, m.index, j)); }
        }
        map
    } else {
        (0..r.vertex_count).map(|v| Some((v, first[v].0, first[v].1))).collect()
    };
    if let Some(n) = o.count { map.resize(n, None); }

    let mut out = Report::default();
    let mut head = String::new();
    let order = match (o.meshlet_order, o.meshlet_slots) { (false, _) => "vertex order".to_string(), (true, None) => "meshlet order".into(), (true, Some(n)) => format!("meshlet order, {n} slots per meshlet") };
    let _ = writeln!(head, "reference: {} {} LOD{}: {} vertices, {} triangles, {} meshlets, AABB {:?}..{:?}",
        r.drawable, r.geometry, r.lod, r.vertex_count, r.triangle_count, r.meshlets.len(), r.aabb.min, r.aabb.max);

    // Captured values, one per element.
    let values: Vec<Option<[f32; 4]>> = match o.layout {
        Layout::Stored => {
            let data = capture.get(o.offset..).context("offset past the capture")?;
            let format = super::u32_at(data, 0).context("capture shorter than the vertex descriptors")? & 127;
            ensure!(format == r.position.format, "capture position descriptor format {format}, reference {}", r.position.format);
            let n = r.vertex_count;
            let pos = drawable::decode_positions(data, format, r.position.offset, n)?;
            let _ = writeln!(head, "capture: stored vertex buffer, {} bytes from offset {}, positions format {format} at +{:#x}", data.len(), o.offset, r.position.offset);
            if let Some(q) = &r.position.quantization {
                let c = drawable::quantization(data).unwrap();
                if c.bias != q.bias || c.scale != q.scale || c.axis != q.axis_order {
                    let _ = writeln!(head, "  quantization header differs: capture bias {:?} axis {} scale {:?}, reference bias {:?} axis {} scale {:?}",
                        c.bias, c.axis, c.scale, q.bias, q.axis_order, q.scale);
                }
            }
            if let Some(stored) = stored_vertex {
                let n = stored.len().min(data.len());
                let diff: Vec<usize> = (0..n).filter(|&i| stored[i] != data[i]).collect();
                let stream = |at: usize| r.attributes.iter().find(|a| (a.offset as usize..a.offset as usize + a.size as usize * r.vertex_count).contains(&at))
                    .map_or(if at < 128 { "header".to_string() } else { "no stream".into() }, |a| format!("semantic {} {}", a.semantic, a.name));
                match diff.first() {
                    None => { let _ = writeln!(head, "  bytes: identical to the stored buffer over {n} bytes"); }
                    Some(&f) => { let _ = writeln!(head, "  bytes: {} of {n} differ from the stored buffer, first at +{f:#x} ({}), last at +{:#x} ({})",
                        diff.len(), stream(f), diff[diff.len() - 1], stream(diff[diff.len() - 1])); }
                }
                if data.len() < stored.len() { let _ = writeln!(head, "  capture is {} bytes shorter than the stored buffer", stored.len() - data.len()); }
            }
            // Stored layout is in vertex order by definition.
            map = (0..n).map(|v| Some((v, first[v].0, first[v].1))).collect();
            pos.into_iter().map(|p| Some([p[0], p[1], p[2], 1.0])).collect()
        }
        Layout::Float3 | Layout::Clip => {
            let clip = o.layout == Layout::Clip;
            let size = if clip { 16 } else { 12 };
            let stride = o.stride.unwrap_or(size);
            ensure!(stride >= size, "stride {stride} is below the {size}-byte element");
            let avail = capture.len().saturating_sub(o.offset).saturating_add(stride - size) / stride;
            let _ = writeln!(head, "capture: {} f32x{}, stride {stride}, offset {}, {} elements in {order}",
                if clip { "clip-space" } else { "model-space" }, size / 4, o.offset, map.len().min(avail));
            if avail < map.len() { let _ = writeln!(head, "  capture holds {avail} of {} elements; the rest are not checked", map.len()); }
            (0..map.len().min(avail)).map(|k| {
                let at = o.offset + k * stride;
                Some([f32_at(capture, at), f32_at(capture, at + 4), f32_at(capture, at + 8), if clip { f32_at(capture, at + 12) } else { 1.0 }])
            }).collect()
        }
    };
    map.truncate(values.len());

    let tol = o.tolerance;
    let mut elems: Vec<Element> = Vec::new();
    for (k, (slot, v)) in map.iter().zip(&values).enumerate() {
        let (Some((vertex, meshlet, local)), Some(v)) = (slot, v) else { continue };
        let refp = r.positions[*vertex];
        let mut e = Element { element: k, vertex: *vertex, meshlet: *meshlet, local: *local, flags: 0, value: *v,
            expected: [f64::from(refp[0]), f64::from(refp[1]), f64::from(refp[2]), 1.0], error: 0.0, aabb_distance: 0.0 };
        if v.iter().any(|c| !c.is_finite()) { e.flags |= NON_FINITE; }
        else if o.layout == Layout::Clip {
            let w = f64::from(v[3]);
            if w <= 0.0 { e.flags |= W_NOT_POSITIVE; }
            else if (f64::from(v[0]) / w).abs() > o.ndc_limit || (f64::from(v[1]) / w).abs() > o.ndc_limit { e.flags |= OUTLYING; }
        } else {
            let p = [0, 1, 2].map(|i| f64::from(v[i]));
            e.error = (0..3).map(|i| (p[i] - e.expected[i]).abs()).fold(0.0, f64::max);
            if e.error > tol { e.flags |= DIFFERS; }
            e.aabb_distance = r.aabb.outside(p);
            if e.aabb_distance > tol { e.flags |= OUTSIDE_AABB; }
        }
        elems.push(e);
    }
    out.checked = elems.len();

    if o.layout == Layout::Clip {
        out.outside_clip = elems.iter().filter(|e| e.flags & NON_FINITE == 0 && { let [x, y, z, w] = e.value.map(f64::from); x.abs() > w || y.abs() > w || z < 0.0 || z > w }).count();
        // Robust fit: start from the plausible elements, then twice drop those far above the median residual.
        let resid = |m: &[[f64; 4]; 4], e: &Element| {
            let p = apply(m, [e.expected[0], e.expected[1], e.expected[2], 1.0]);
            (0..4).map(|i| (f64::from(e.value[i]) - p[i]).abs()).fold(0.0, f64::max) / p[3].abs().max(1e-30)
        };
        let mut keep: Vec<bool> = elems.iter().map(|e| e.flags & BAD == 0).collect();
        let mut m = None;
        for _ in 0..4 {
            let samples: Vec<_> = elems.iter().zip(&keep).filter(|(_, k)| **k).map(|(e, _)| ([e.expected[0], e.expected[1], e.expected[2]], e.value.map(f64::from))).collect();
            if samples.len() < 8 { m = None; break; }
            let Some(f) = fit(&samples) else { m = None; break };
            let res: Vec<f64> = elems.iter().map(|e| if e.flags & NON_FINITE != 0 { f64::INFINITY } else { resid(&f, e) }).collect();
            let limit = (8.0 * median(res.iter().zip(&keep).filter(|(_, k)| **k).map(|(r, _)| *r).collect())).max(o.clip_tolerance / 4.0);
            keep = res.iter().map(|&r| r <= limit).collect();
            m = Some(f);
        }
        match m {
            None => { let _ = writeln!(head, "fit: not possible (fewer than 8 usable elements, or flat geometry); reference and AABB checks skipped"); }
            Some(m) => {
                let used = keep.iter().filter(|&&k| k).count();
                let inv = invert(&m);
                for e in elems.iter_mut() {
                    e.expected = apply(&m, [e.expected[0], e.expected[1], e.expected[2], 1.0]);
                    if e.flags & NON_FINITE != 0 { continue; }
                    e.error = (0..4).map(|i| (f64::from(e.value[i]) - e.expected[i]).abs()).fold(0.0, f64::max) / e.expected[3].abs().max(1e-30);
                    if e.error > o.clip_tolerance { e.flags |= DIFFERS; }
                    if let Some(inv) = &inv {
                        let p = apply(inv, e.value.map(f64::from));
                        e.aabb_distance = if p[3].abs() < 1e-30 { f64::INFINITY } else { r.aabb.outside([p[0] / p[3], p[1] / p[3], p[2] / p[3]]) };
                        if e.aabb_distance > tol { e.flags |= OUTSIDE_AABB; }
                    }
                }
                let _ = writeln!(head, "fit: clip = M * (x, y, z, 1) from {used} of {} elements{}", elems.len(),
                    if used * 2 < elems.len() { " (under half: the fit, and so the reference comparison, is unreliable)" } else { "" });
                for row in &m { let _ = writeln!(head, "  [{:>12.6} {:>12.6} {:>12.6} {:>12.6}]", row[0], row[1], row[2], row[3]); }
                if inv.is_none() { let _ = writeln!(head, "  M is singular: AABB check skipped"); }
                out.fit = Some(m);
            }
        }
    }
    let count = |f: u8| elems.iter().filter(|e| e.flags & f != 0).count();
    (out.non_finite, out.w_not_positive, out.outlying, out.outside_aabb, out.differs, out.bad) =
        (count(NON_FINITE), count(W_NOT_POSITIVE), count(OUTLYING), count(OUTSIDE_AABB), count(DIFFERS), count(BAD));

    let mut s = head;
    let _ = writeln!(s, "summary ({} elements checked):", out.checked);
    let line = |s: &mut String, what: &str, n: usize| { let _ = writeln!(s, "  {what:<46} {n:>8}"); };
    line(&mut s, "non-finite (NaN/Inf)", out.non_finite);
    if o.layout == Layout::Clip {
        line(&mut s, "w <= 0", out.w_not_positive);
        line(&mut s, &format!("|x/w| or |y/w| > {}", o.ndc_limit), out.outlying);
        line(&mut s, "outside the clip volume (not counted as bad)", out.outside_clip);
        line(&mut s, &format!("differ from the fit by > {} |w|", o.clip_tolerance), out.differs);
    } else {
        line(&mut s, &format!("differ from the reference by > {tol}"), out.differs);
    }
    line(&mut s, &format!("outside the AABB by > {tol}"), out.outside_aabb);
    line(&mut s, "bad (any check above)", out.bad);

    // Triangles: corners mapped to elements the same way as the vertices.
    let elem_of: std::collections::HashMap<usize, usize> = elems.iter().enumerate().map(|(i, e)| (e.element, i)).collect();
    let corner = |m: &Meshlet, g: u32| -> Option<usize> {
        if !o.meshlet_order { return (g as usize).into(); }
        let j = r.meshlet_refs[m.first_ref..m.first_ref + m.vertices].iter().position(|&v| v == g)?;
        Some(slots(m) + j)
    };
    let (mut tris, mut crossing) = (0, 0);
    for m in &r.meshlets {
        for t in m.first_triangle..m.first_triangle + m.triangles {
            let Some(c) = (0..3).map(|i| r.indices.get(t * 3 + i).and_then(|&g| corner(m, g)).and_then(|k| elem_of.get(&k).copied())).collect::<Option<Vec<usize>>>() else { continue };
            tris += 1;
            let es = [&elems[c[0]], &elems[c[1]], &elems[c[2]]];
            if es.iter().any(|e| e.flags & BAD != 0) { out.triangles_with_bad += 1; }
            if o.layout != Layout::Clip || es.iter().any(|e| e.flags & NON_FINITE != 0) { continue; }
            if es.iter().any(|e| e.value[3] <= 0.0) { crossing += 1; continue; }
            let a = clipped_area(es.map(|e| [f64::from(e.value[0]) / f64::from(e.value[3]), f64::from(e.value[1]) / f64::from(e.value[3])])) / 4.0;
            out.screens_covered += a;
            if a > 0.25 { out.large_triangles += 1; }
        }
    }
    let _ = writeln!(s, "triangles ({tris} with every corner captured):");
    line(&mut s, "with a bad corner", out.triangles_with_bad);
    if o.layout == Layout::Clip {
        line(&mut s, "with a corner at w <= 0 (not measured)", crossing);
        line(&mut s, "covering over 1/4 of the screen", out.large_triangles);
        let _ = writeln!(s, "  {:<46} {:>8.3}", "screens covered (sum of on-screen areas)", out.screens_covered);
    }

    if out.bad > 0 {
        let bad: Vec<&Element> = elems.iter().filter(|e| e.flags & BAD != 0).collect();
        let mut by_local = [0usize; 4];
        for e in &bad { by_local[(e.local / 32).min(3)] += 1; }
        let mut meshlets: Vec<usize> = bad.iter().map(|e| e.meshlet).collect();
        meshlets.sort();
        meshlets.dedup();
        let _ = writeln!(s, "bad elements by meshlet-local vertex: 0-31 {}, 32-63 {}, 64-95 {}, 96-127 {}", by_local[0], by_local[1], by_local[2], by_local[3]);
        let _ = writeln!(s, "meshlets with a bad element: {} of {}", meshlets.len(), r.meshlets.len());
    }
    let mut worst: Vec<Element> = elems.into_iter().filter(|e| e.flags & BAD != 0).collect();
    worst.sort_by(|a, b| (a.flags & BAD).trailing_zeros().cmp(&(b.flags & BAD).trailing_zeros())
        .then(b.error.total_cmp(&a.error)).then(b.aabb_distance.total_cmp(&a.aabb_distance)));
    worst.truncate(o.top);
    if !worst.is_empty() {
        let _ = writeln!(s, "worst {} (element, vertex, meshlet:local, flags, captured, expected, error, outside AABB by):", worst.len());
        for e in &worst {
            let flags: Vec<&str> = [(NON_FINITE, "nonfinite"), (W_NOT_POSITIVE, "w<=0"), (OUTLYING, "outlying"), (OUTSIDE_AABB, "aabb"), (DIFFERS, "differs")]
                .into_iter().filter(|(f, _)| e.flags & f != 0).map(|(_, n)| n).collect();
            let v = if o.layout == Layout::Clip { format!("{:?}", e.value) } else { format!("{:?}", &e.value[..3]) };
            let x = if o.layout == Layout::Clip { format!("{:?}", e.expected.map(|c| c as f32)) } else { format!("{:?}", &e.expected.map(|c| c as f32)[..3]) };
            let meshlet = if e.meshlet == usize::MAX { "-".into() } else { e.meshlet.to_string() };
            let _ = writeln!(s, "  {:>6} {:>6} {meshlet:>4}:{:<3} {:<22} {v} {x} {:.3e} {:.3e}", e.element, e.vertex, e.local, flags.join(","), e.error, e.aabb_distance);
        }
    }
    out.text = s;
    out.worst = worst;
    Ok(out)
}

/// Parses `stored`, `float3` or `clip`.
pub fn parse_layout(s: &str) -> Result<Layout> {
    Ok(match s { "stored" => Layout::Stored, "float3" => Layout::Float3, "clip" => Layout::Clip, _ => bail!("layout {s:?}: use stored, float3 or clip") })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 160 vertices on a bumpy cylinder, two meshlets of 96 and 64 vertices, a triangle strip in each.
    fn reference() -> Reference {
        let positions: Vec<[f32; 3]> = (0..160).map(|i| {
            let a = i as f32 * 0.7;
            [a.cos(), a.sin(), (i % 13) as f32 * 0.1 + (i as f32 * 0.37).sin() * 0.05]
        }).collect();
        let refs: Vec<u32> = (0..160).collect();
        let (mut meshlets, mut indices) = (Vec::new(), Vec::new());
        for (index, (first, n)) in [(0usize, 96usize), (96, 64)].into_iter().enumerate() {
            let first_triangle = indices.len() / 3;
            for t in 0..n - 2 { indices.extend([first + t, first + t + 1, first + t + 2].map(|v| v as u32)); }
            meshlets.push(Meshlet { index, vertices: n, triangles: n - 2, first_ref: first, first_triangle, record: [0; 4],
                aabb: Aabb::of(&positions[first..first + n]) });
        }
        let stored = stored(&positions);
        Reference {
            format: FORMAT.into(), drawable: "synthetic".into(), lod: 0, geometry: "model_0_geometry_0".into(),
            files: Files { vertex: "g0.vertex.bin".into(), index: "g0.index.bin".into(), meshlets: "g0.meshlets.bin".into() },
            vertex_count: 160, triangle_count: indices.len() / 3, aabb: Aabb::of(&positions),
            vertex_buffer: VertexBuffer { bytes: stored.len(), elements: stored.len() as u32 / 4, element_stride: 4, descriptors: vec![99 | 96 << 7] },
            attributes: vec![Attribute { semantic: 0, name: "position".into(), offset: 96, size: 12, dxgi_format: 6, descriptor: Some(99 | 96 << 7) }],
            position: Position { format: 99, offset: 96, stride: 12, quantization: None },
            texcoord_scale: None,
            index_buffer: IndexBuffer { bytes: 0, meshlet_records_offset: 12, triangles_offset: 0, refs_offset: 0, ref_size: 2 },
            meshlets, meshlet_refs: refs, indices, positions,
        }
    }

    /// Float3 (format 99) vertex buffer: descriptor, then positions at +96.
    fn stored(p: &[[f32; 3]]) -> Vec<u8> {
        let mut v = vec![0u8; 96];
        v[..4].copy_from_slice(&(99u32 | 96 << 7).to_le_bytes());
        v.extend(p.iter().flatten().flat_map(|c| c.to_le_bytes()));
        v
    }

    const MVP: [[f64; 4]; 4] = [[1.2, 0.1, 0.0, 0.3], [0.0, 0.9, 1.4, -0.2], [0.05, -0.1, 0.2, 4.0], [0.05, -0.1, 0.2, 4.2]];

    fn clip(r: &Reference, order: &[u32], slots: Option<usize>, stride: usize) -> (Vec<[f32; 4]>, Vec<u8>) {
        let mut rows: Vec<[f32; 4]> = Vec::new();
        match slots {
            None => rows.extend(order.iter().map(|&v| { let p = r.positions[v as usize]; apply(&MVP, [p[0], p[1], p[2], 1.0].map(f64::from)).map(|c| c as f32) })),
            Some(n) => for m in &r.meshlets {
                for j in 0..n {
                    rows.push(if j < m.vertices { let p = r.positions[order[m.first_ref + j] as usize]; apply(&MVP, [p[0], p[1], p[2], 1.0].map(f64::from)).map(|c| c as f32) } else { [0.0; 4] });
                }
            },
        }
        (rows.clone(), bytes(&rows, stride))
    }

    fn bytes(rows: &[[f32; 4]], stride: usize) -> Vec<u8> {
        rows.iter().flat_map(|r| { let mut b: Vec<u8> = r.iter().flat_map(|c| c.to_le_bytes()).collect(); b.resize(stride, 0xEE); b }).collect()
    }

    #[test]
    fn clip_capture_matches_and_fit_recovers_the_matrix() {
        let r = reference();
        let order: Vec<u32> = (0..160).collect();
        let (_, data) = clip(&r, &order, None, 20);
        let rep = check(&r, &data, None, &Options { stride: Some(20), ..Default::default() }).unwrap();
        assert_eq!((rep.checked, rep.bad, rep.differs, rep.triangles_with_bad), (160, 0, 0, 0), "{}", rep.text);
        let m = rep.fit.unwrap();
        assert!((0..4).all(|i| (0..4).all(|j| (m[i][j] - MVP[i][j]).abs() < 1e-4)), "{m:?}");
        assert!(rep.screens_covered > 0.0 && rep.text.starts_with("reference: synthetic"));
        // Meshlet order with 128 slots per meshlet and the padding left as zeros.
        let (_, data) = clip(&r, &r.meshlet_refs, Some(128), 16);
        let rep = check(&r, &data, None, &Options { meshlet_order: true, meshlet_slots: Some(128), ..Default::default() }).unwrap();
        assert_eq!((rep.checked, rep.bad), (160, 0), "{}", rep.text);
    }

    #[test]
    fn clip_capture_counts_each_kind_of_bad_vertex() {
        let r = reference();
        let order: Vec<u32> = (0..160).collect();
        let (mut rows, _) = clip(&r, &order, None, 16);
        rows[3][1] = f32::NAN;
        rows[4][3] = f32::INFINITY;
        rows[10][3] = -rows[10][3];
        for i in [20, 21] { rows[i] = [5e4, 1.0, 0.5, 1.0]; }
        for i in [70, 71, 140] { rows[i][0] += 0.05 * rows[i][3]; }
        let rep = check(&r, &bytes(&rows, 16), None, &Options::default()).unwrap();
        assert_eq!((rep.non_finite, rep.w_not_positive, rep.outlying), (2, 1, 2), "{}", rep.text);
        // The flipped w, the two outliers and the three shifted vertices miss the fit; the flipped w and outliers also unproject outside the AABB.
        assert_eq!((rep.differs, rep.bad), (6, 8), "{}", rep.text);
        assert!(rep.outside_aabb >= 3, "{}", rep.text);
        assert_eq!(rep.worst.iter().map(|e| e.element).collect::<Vec<_>>()[..2], [3, 4]);
        assert!(rep.text.contains("bad elements by meshlet-local vertex: 0-31 5, 32-63 1, 64-95 2, 96-127 0"), "{}", rep.text);
        assert!(rep.text.contains("meshlets with a bad element: 2 of 2"), "{}", rep.text);
        // Each corrupted vertex sits in up to three strip triangles.
        assert!(rep.triangles_with_bad >= 8 && rep.triangles_with_bad <= 24);
        let quiet = check(&r, &bytes(&rows, 16), None, &Options { top: 0, ..Default::default() }).unwrap();
        assert!(quiet.worst.is_empty() && !quiet.text.contains("worst"));
    }

    #[test]
    fn stored_and_float3_captures_compare_with_the_reference() {
        let r = reference();
        let good = stored(&r.positions);
        let rep = check(&r, &good, Some(&good), &Options { layout: Layout::Stored, ..Default::default() }).unwrap();
        assert_eq!(rep.bad, 0, "{}", rep.text);
        assert!(rep.text.contains("identical to the stored buffer over 2016 bytes"), "{}", rep.text);
        let mut bad = good.clone();
        bad[96 + 7 * 12..][..4].copy_from_slice(&f32::NAN.to_le_bytes());
        bad[96 + 9 * 12 + 4..][..4].copy_from_slice(&3.0f32.to_le_bytes());
        let rep = check(&r, &bad, Some(&good), &Options { layout: Layout::Stored, ..Default::default() }).unwrap();
        assert_eq!((rep.non_finite, rep.differs, rep.outside_aabb, rep.bad), (1, 1, 1, 2), "{}", rep.text);
        assert!(rep.text.contains("first at +0xb4 (semantic 0 position)"), "{}", rep.text);
        // float3 with stride 16 and an offset of 32; vertex 5 nudged by 2e-3.
        let mut f3: Vec<u8> = vec![0; 32];
        for (i, p) in r.positions.iter().enumerate() {
            let p = if i == 5 { [p[0] + 2e-3, p[1], p[2]] } else { *p };
            f3.extend(p.iter().flat_map(|c| c.to_le_bytes()));
            f3.extend([0; 4]);
        }
        let rep = check(&r, &f3, None, &Options { layout: Layout::Float3, stride: Some(16), offset: 32, ..Default::default() }).unwrap();
        assert_eq!((rep.differs, rep.outside_aabb, rep.bad, rep.worst[0].vertex), (1, 0, 1, 5), "{}", rep.text);
        // A short capture checks what it holds.
        let rep = check(&r, &f3[..32 + 100 * 16], None, &Options { layout: Layout::Float3, stride: Some(16), offset: 32, ..Default::default() }).unwrap();
        assert_eq!(rep.checked, 100);
        assert!(check(&Reference { format: "other".into(), ..r }, &f3, None, &Options::default()).is_err());
    }

    #[test]
    fn clipped_area_and_large_triangles() {
        assert!((clipped_area([[-10.0, -10.0], [10.0, -10.0], [0.0, 30.0]]) - 4.0).abs() < 1e-9);
        assert!((clipped_area([[0.0, 0.0], [0.5, 0.0], [0.0, 0.5]]) - 0.125).abs() < 1e-9);
        assert_eq!(clipped_area([[2.0, 2.0], [3.0, 2.0], [2.0, 3.0]]), 0.0);
        // Two neighbours thrown to opposite corners far off screen make their triangles screen-sized.
        let r = reference();
        let order: Vec<u32> = (0..160).collect();
        let (mut rows, _) = clip(&r, &order, None, 16);
        for (i, x) in [(50, -15.0), (51, 15.0)] { rows[i] = [x * rows[i][3], 15.0 * rows[i][3], rows[i][2], rows[i][3]]; }
        let rep = check(&r, &bytes(&rows, 16), None, &Options::default()).unwrap();
        assert_eq!((rep.outlying, rep.differs), (0, 2), "{}", rep.text);
        assert!(rep.large_triangles >= 1, "{}", rep.text);
    }
}
