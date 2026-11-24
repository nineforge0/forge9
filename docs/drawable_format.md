# Drawable resources on disk (checked on real files)

How paged resources inside the nested content archives of a story/level pack
are laid out, and the exact drawable layout `forge9 model` decodes. Offsets
are byte offsets into the inflated system segment unless noted. Pointers are
64-bit: `0x5000_0000 + offset` (system) or `0x6000_0000 + offset` (graphics).

Source for every claim below: `bin/story000_restofworld.rpf` (18 nested
archives, 465 entries). Reproduce with `forge9 tree`, `forge9 resource` and
`forge9 model`.

## What the paged resources are

Every paged resource starts `u64 0` (vtable slot, zeroed on disk), `u64 page
map pointer`. The page map is `+0x00 u64 0, +0x08 u8 system pages, +0x09 u8
graphics pages`, then one zeroed 8-byte slot per page. Each page is usually
inflated as its own zlib block: 443 of 447 resources have exactly `system +
graphics` blocks; the other 4 pack 2 system pages into 1 block. The page map
can sit in the second system block, so it is read from the joined blocks.

Census of the 447 paged resources by page map:

| Kind | Count | Graphics pages | Notes |
|------|------:|---------------:|-------|
| `0DRP` v0x7a metadata | 101 | 0 | 32 with 1 system page, 69 with 2 |
| Plain, no graphics | 344 | 0 | animation clip dictionaries (`pack:/...anim` names), other metadata |
| Plain, with graphics | 2 | 1 or 2 | `19/0`: drawable dictionary; `20/0`: particle effect list |

So no `0DRP` resource is a drawable, and only `19/0` holds mesh data. `model`
reports the first two kinds as "0DRP v0x7a metadata resource" and "paged
resource with N system and no graphics pages".

### `0DRP` v0x7a header

| Offset | Value | Evidence |
|-------:|-------|----------|
| +0x00 | u64 0 | all 101 |
| +0x08 | page map pointer | all 101 (graphics pages always 0) |
| +0x10 | `0DRP` | all 101 |
| +0x14 | u32 0x7a (version) | all 101 |
| +0x18 | u64 0 | all 101 |
| +0x20 | u32 1..37 (a count; 29 resources have 1) | all 101 |
| +0x24 | u32 0 | all 101 |
| +0x28..+0x40 | four u64; all four are system pointers in 75 of 101 | |

Strings such as `Variables`, `Characters`, `Volumes` sit inside some of them;
they are metadata, not geometry.

## Drawable dictionary (`19/0`, 1 system + 1 graphics page)

```
00000000: 0000 0000 0000 0000 f01c 0050 0000 0000   vtable 0, page map 0x1cf0
00000010: 0000 0000 0000 0000 0100 0000 0000 0000   parent 0, u32 usage count 1
00000020: 901f 0050 0000 0000 0300 0300 0000 0000   hashes 0x1f90, u16 count 3, u16 capacity 3
00000030: 681d 0050 0000 0000 0300 0300 0000 0000   values 0x1d68, u16 count 3, u16 capacity 3
00001cf0: 0000 0000 0000 0000 0101 0000 0000 0000   page map: 1 system, 1 graphics
```

- Hashes are u32 joaat of each drawable name without its `.#dr` suffix
  (checked with `forge9 hash` for all three), sorted ascending.
- Values are drawable pointers in the same order as the hashes.
- Each member drawable has a zero page map pointer (+0x08); only the
  dictionary carries one.

## Drawable header (224 bytes, member at 0xdf0)

| Offset | Field | Value at 0xdf0 |
|-------:|-------|----------------|
| +0x00 | u64 0 | 0 |
| +0x08 | page map pointer (0 inside a dictionary) | 0 |
| +0x10 | shader group pointer | 0x50001990 |
| +0x18 | skeleton pointer | 0 |
| +0x20 | f32x3 bounding sphere center | (0, -0.0194, -0.0024) |
| +0x2C | f32 bounding sphere radius | 1.0741 |
| +0x30 | f32x3 AABB min (+pad) | (-0.7140, -0.6095, -0.5463) |
| +0x40 | f32x3 AABB max (+pad) | (0.7140, 0.5706, 0.5415) |
| +0x50..+0x68 | four LOD group pointers | 0x50001e00, 0, 0, 0 |
| +0x70 | f32x4 LOD distances | 9998.0 x4 |
| +0xA8 | name pointer | `ptfx_model_gore_jaw_01.#dr` |
| +0xB0 | pointer to five `{ptr, u16 count, u16 capacity}` lists | not decoded |

The center is the midpoint of the AABB and the radius its half-diagonal, for
all three members.

## LOD group, model, geometry

- LOD group: `{ptr model pointers, u16 count, u16 capacity}`
  (0x1e00: `0x50001fa8, 1, 1`).
- Model (0x1a10): +0x08 pointer to geometry pointers, +0x10 u16 count,
  +0x12 u16 capacity, +0x18 pointer to f32x4 min/max AABBs (0x1bd0 holds the
  same bounds as the header), +0x20 a pointer (likely per-geometry shader
  indices; not decoded), +0x30 flags.
- Geometry (0x1440): +0x08 vertex buffer pointer, +0x40 index descriptor
  pointer, +0x90 vertex layout pointer (same as the vertex buffer's), +0xA0
  f32x4 min, f32x4 max, then three f32x4 rows of an orthonormal 3x3 (looks
  like an oriented bounding box; not decoded).

## Vertex buffer and layout

Vertex buffer (0x1490): +0x08 u32 element count (0x87e), +0x0C u16 element
stride (4), +0x10 u32 0x1601, +0x18 graphics pointer (0x6000a2f0), +0x40
layout pointer (0x50000a30). Byte length is count x stride (8696).

Layout (320 bytes, 0xa30): +0x00 one u32 byte offset per semantic (0x80 when
absent), +0xD0 one u8 size per semantic, +0x104 one u8 format per semantic,
+0x138 u32 `vertex count << 10 | 1` (0x59461: 357 vertices). Present in the
jaw: semantic 0 (position, size 8, format 16), 4 (size 4, format 41), 24
(size 4, format 28), 28 and 29 (texcoords, size 4, format 38 = snorm16x2).

Vertex data starts with u32 attribute descriptors `format | offset << 7`
(0x4042: format 66 at offset 0x80). Format-66 positions are 8 bytes:
24-bit X, 24-bit Y, 16-bit Z lanes, decoded as `bias + scale * lane` with
`+96 f32x3 bias`, `+108 u32 axis order`, `+112 f32x3 scale`. In the jaw the
bias equals the header AABB min and `scale * (2^24 - 1, 2^24 - 1, 2^16 - 1)`
spans the AABB extent exactly. Texcoord snorm16 values scale by `+124 f32`.

## Index data

Index descriptor (0x1bf0): `ptr index buffer, u16 clusters (4), u16
triangles (446), u32 refs << 8 | triangles >> 16 (0x17a00: 378 refs)`.
Index buffer (0x11e0): +0x08 u32 word count (0x290), +0x0C u16 stride 4,
+0x18 graphics pointer (0x6000d560). The graphics data repeats the counts
(`0x80000004` = u16 refs + 4 clusters, 446, 378), then 16-byte cluster
records, one u32 per triangle (three 7-bit cluster-local corners, cluster
index from bit 21), and the u16 cluster-to-vertex table. This is the
clustered decode documented at the top of `src/formats/drawable.rs`.

## Validation of the export

`forge9 model bin/story000_restofworld.rpf 19/0 -o out.glb` (or `--obj`):

| Drawable | Vertices | Triangles | Decoded AABB | Header AABB |
|----------|---------:|----------:|--------------|-------------|
| skull_01 | 1281 | 1884 | (-0.7510, -0.5356, -0.9414)..(0.7510, 0.5356, 0.9048) | (-0.7512, -0.5354, -0.9415)..(0.7512, 0.5354, 0.9048) |
| jaw_01 | 357 | 446 | (-0.7139, -0.6094, -0.5464)..(0.7139, 0.5708, 0.5415) | (-0.7140, -0.6095, -0.5463)..(0.7140, 0.5706, 0.5415) |
| skull_02 | 170 | 290 | (-0.7656, -0.9102, -0.7734)..(0.7495, 0.9404, 0.7593) | (-0.7657, -0.9102, -0.7736)..(0.7494, 0.9403, 0.7594) |

- All positions finite; decoded bounds match the header within 3e-4
  (quantization step).
- Every index in range, every vertex referenced, no degenerate triangles.
- 83% to 95% of directed edges have an opposite twin (consistent winding on a
  connected surface); a wrong index decode does not produce this.
- Rendered, the three meshes are a skull, a jaw and a skull fragment.
- `decodes_real_dictionary` in `src/formats/drawable.rs` re-checks counts and
  bounds whenever the archive is present locally.

Not validated: texcoords (decoded; the jaw and skull_02 fall in 0..1, skull_01
spans -1.04..1.94), normals, the +0xB0 lists, the geometry OBB, LODs 1..3
(this file has only LOD 0), standalone drawables and fragments on real data.
