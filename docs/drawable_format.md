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
| +0x10 | shader group pointer (see below) | 0x50001990 |
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
  same bounds as the header), +0x20 pointer to one u16 shader index per
  geometry (0x1e10: 0; the width is assumed, every sample is a single 0),
  +0x30 flags.
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
+0x138 u32 `vertex count << 10 | 1` (0x59461: 357 vertices). The format
bytes match DXGI numbering (16 R32G32_FLOAT, 41 R32_FLOAT, 28 R8G8B8A8_UNORM,
38 R16G16_SNORM); the declared format is the storage width, not always the
meaning. All three members use the same five semantics:

| Semantic | Size | Format | Descriptor | Meaning | Decoded |
|---------:|-----:|-------:|-----------:|---------|---------|
| 0 | 8 | 16 | 0x42 (66) | quantized position | yes |
| 4 | 4 | 41 | 0x21 (33) | normal, octahedral 10:10 + 12 more bits | normal yes, upper bits no |
| 24 | 4 | 28 | 0x26 (38) | RGBA8 per vertex | yes (`_COLOR_0`) |
| 28 | 4 | 38 | 0x28 (40) | texcoord set 0, snorm16x2 | yes (TEXCOORD_0) |
| 29 | 4 | 38 | 0x28 (40) | texcoord set 1, snorm16x2 | yes (TEXCOORD_1) |

Vertex data starts with 16 u32 attribute descriptors `kind | size << 3 |
offset << 7` (low 4 bits a kind, `(d >> 3) & 60` the byte size; 0x4042:
format 66 at offset 0x80). Descriptor slots are not in semantic order
(skull_01: position, 24, 4, 28, 29), so the decoder matches a descriptor to
a semantic by offset. Format-66 positions are 8 bytes: 24-bit X, 24-bit Y,
16-bit Z lanes, decoded as `bias + scale * lane` with `+96 f32x3 bias`,
`+108 u32 axis order`, `+112 f32x3 scale`. In the jaw the bias equals the
header AABB min and `scale * (2^24 - 1, 2^24 - 1, 2^16 - 1)` spans the AABB
extent exactly. `+0x5C` is u32 0x200 in all three (not decoded).

### Normals (semantic 4)

One u32 per vertex. Bits 0..9 and 10..19 are unorm10 `x, y` of an
octahedral unit vector (`c / 511.5 - 1`; `z = 1 - |x| - |y|`, and when
`z < 0` the lower hemisphere fold `x' = (1 - |y|) sign x, y' = (1 - |x|)
sign y`), then normalized. Bits 20..31 are not decoded; bits 30..31 are
`10` or `11` for 98% of vertices, so they are probably a tangent angle plus
a bitangent sign.

How it was found: per-vertex area-weighted face normals were computed from
the decoded positions and indices and compared with candidate decodes of
every 4-byte semantic. Plain 10:10:10 vectors had the right direction but
lengths of 0.06..1.5; the first two fields matched `n.xy / (|x|+|y|+|z|)`
of the face normal (for example face `(-0.060, 0.267, 0.962)` vs stored
`(-0.046, 0.208)`), which is the octahedral projection. Semantic 24 decoded
in every candidate scheme gave a mean dot of at most 0.2 (it is color).

### RGBA8 attribute (semantic 24)

Bytes in R, G, B, A order. In all 1808 vertices R = G = A = 255 and B is
19..34 (jaw_01: 23 or 24 for 157 of 357). That is not an albedo tint (as glTF
`COLOR_0` it would turn the models saturated yellow), so it is exported as
the application-specific `_COLOR_0` attribute, which viewers ignore and DCC
tools import as a plain attribute. Its meaning (masks, wind, AO) is unknown.

### Texcoords (semantics 28, 29)

snorm16x2 (`/ 32767`, clamped at -1) times the f32 at vertex data +0x7C
(4.0 in all three). Evidence for the 4.0 factor: before scaling, set 1 of
all three meshes and set 0 of jaw_01 and skull_02 top out at 0.2401..0.2485,
just under 0.25, so `x 4` maps them into 0..1. Set 0 of skull_01 spans
u -1.04..1.94: its charts sit in the neighbouring tiles (249 vertices with
u in -1..0, 740 in 1..2), but no triangle spans more than 0.25 in UV, and
its UV-per-length scale (median 0.25) matches the other meshes (0.19..0.22).
That is the shape of islands offset by whole tiles under a repeating
sampler, not a decode error; it cannot be confirmed without the material.

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

Geometry:

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

Normals and texcoords (dot = decoded normal vs area-weighted face normal):

| Drawable | Unit length (1e-3) | dot > 0 | dot > 0.9 | Mean dot | TEXCOORD_0 | TEXCOORD_1 |
|----------|-------------------:|--------:|----------:|---------:|------------|------------|
| skull_01 | 100% | 99.1% | 80.2% | 0.927 | u -1.039..1.942, v 0.028..0.981 | u 0.011..0.985, v 0.315..0.931 |
| jaw_01 | 100% | 97.2% | 68.6% | 0.857 | u 0.058..0.909, v 0.343..0.779 | u 0.011..0.960, v 0.669..0.994 |
| skull_02 | 100% | 99.4% | 94.1% | 0.970 | u 0.110..0.592, v 0.358..0.991 | u 0.027..0.900, v 0.008..0.326 |

The vertices with dot below 0 sit where a face normal is a poor reference
(teeth, thin creases). A Lambert render with the decoded normals is smoothly
shaded. Positions and indices are byte-identical to the export before
normals were added.

Tests in `src/formats/drawable.rs` (the real-file ones skip when the archive
is absent): `decodes_real_dictionary` (counts, bounds),
`decodes_real_normals_and_texcoords` (unit length, at least 97% agreement
with face normals, UV ranges), `rejects_real_effect_objects`, plus the
synthetic `decodes_octahedral_normals`, `rejects_list_in_lod_slot` and
`decodes_shader_textures_and_colors`. `decodes_real_dictionary` also checks
the shader decode (one shader per drawable, the four texture parameters
below, R = G = A = 255 in semantic 24).

## Shader group and texture references

Shader group (64 bytes, jaw at 0x1990, skull_01 0x1a50, skull_02 0x1b10):

| Offset | Value | Evidence |
|-------:|-------|----------|
| +0x00 | u64 0 | all 3 |
| +0x08 | u64 0 (an embedded texture dictionary would be expected here; none to check) | all 3 |
| +0x10 | `{ptr shader pointers, u16 count, u16 capacity}` | `0x50001fa0, 1, 1` in all 3 |
| +0x20..+0x40 | zero | all 3 |

Shader (64 bytes, jaw at 0x19d0):

| Offset | Value | Evidence |
|-------:|-------|----------|
| +0x00 | u32 0xbeacbf33 (shader hash; name not found) | all 3 |
| +0x04 | u32 0x6d657461 | all 3 |
| +0x08 | pointer to the constant buffer (f32 values: 0.2, 1.0, -0.3, 24.0, ...) | all 3 |
| +0x10 | pointer to one u64 per texture slot (0 = unbound) | all 3 |
| +0x20 | pointer to the parameter table | all 3 |
| +0x30 | `00 80 78 01 01 ff 00 00` | all 3 |

Parameter table (jaw at 0x40): `+0x01 u8` texture parameters (6), `+0x04 u8`
parameters in total (0x39 = 57), then from +0x08 one `{u32 name hash, u32
packed}` pair per parameter, textures first. For a texture the packed word is
`slot << 2` (0, 4, ... 0x14: slots 0..5); for the 51 constants its low two
bits are 3 and the upper bits look like a word offset and a vector count
(not decoded). The texture parameter names are joaat hashes, matched with
`forge9 hash`:

| Hash | Name | Slot | Bound in all 3 |
|------|------|-----:|:--------------:|
| 0xc8e8c282 | `diffusetex` | 0 | yes |
| 0x739e3d0d | `speculartex2` | 1 | yes |
| 0x2c8d7182 | `tintpalettetex` | 2 | no |
| 0x0cd27763 | (not found) | 3 | no |
| 0xc6f46e79 | `speculartex` | 4 | yes |
| 0x1e77bf4c | `bumptex` | 5 | yes |

Texture reference (80 bytes, jaw slot 0 at 0x1300): +0x10 u32 0x980, +0x30
pointer to a 16-byte name record, +0x3c `u16 1, u16 1`, +0x40 `ff 00 01 00
00 00 01 00`; no width, height, format or graphics pointer, so the texels are
external. Name record (0x1d90): `+0x00` a pointer with low bits 5 (0x50001d85)
to `{u64 6, u32 name hash}` 16 bytes earlier, `+0x08` a u64 key. All three
shaders bind the same four textures:

| Parameter | Name hash | Key |
|-----------|-----------|-----|
| `diffusetex` | 0x0d601c9a | 75a4cddb894939a6 |
| `speculartex2` | 0x5d4a8a97 | f790e14cb486206c |
| `speculartex` | 0xd3c59b35 | 53bd159e0279ee67 |
| `bumptex` | 0x6dc85865 | 3ca737af680d8a96 |

### Texture resolution

`model` looks each key up in the 64-bit hash of every ETPF descriptor of the
`.ttpf` stores (`--textures`, default every `.ttpf` next to the archive),
then embeds `diffusetex` as `baseColorTexture` and `bumptex` as
`normalTexture` (PNG, mip 0; BC5 gets Z rebuilt as `sqrt(1 - x^2 - y^2)`).
Materials set `metallicFactor` 0; other textures are reported, not mapped.

Locally, 0 of the 12 references resolve: none of the four keys, either
32-bit half, or the name hashes occurs in `platform.ttpf`,
`ttdb_graphics.ttpf` or `ttdb_props.ttpf` (1124 textures), anywhere else in
those files, or in any inflated entry of the story, common or script packs
besides the dictionary itself. The ttdb_props textures that look like gore
(a flesh atlas, a skull-like face) do not fit the skulls' UV layouts. So the
key-to-store mapping is the working assumption, not yet confirmed on a
sample: the gore textures live in a store that is not present.

With store textures forced in place of the keys, the output passes the
Khronos glTF validator with 0 errors (warning: no TANGENT, since normal
bits 20..31 are not decoded); the untextured export has 0 errors and 0
warnings.

## Header false positives in the particle effect list (`20/0`)

The drawable header check used to accept 16 objects in `20/0`'s system
pages: their +0x50 is a pointer followed by `u16 count 2, u16 capacity 2`
(or 1, 1) in the LOD 1 slot, with an empty name. Requiring the LOD 1..3
slots (+0x58..+0x68) to be null or system pointers rejects all 16 and keeps
all three real members. The effect list names models (`brain_s1`,
`bone_02`, `bone_03` and others) that are not in this archive; only
skull_01, skull_02 and jaw_01 are.

## Which local packs hold meshes

`forge9 search <pack> --drawable` lists every entry `model` can export.
Census of every entry, recursing into nested archives (the texture stores
were listed with `forge9 texture`):

| Pack | Entries | Meshes |
|------|---------|--------|
| `story000_restofworld.rpf` | 486 leaf entries (top level and nested): 101 `0DRP` metadata, 344 paged without graphics (animation, metadata), 1 drawable dictionary, 1 particle effect list, 7 Bink videos, 14 AWC, 15 XML/PSIN, 3 small blobs | 1 dictionary (`19/0`), 3 drawables, LOD 0 only |
| `common.rpf` | 418: XML, PSIN, ADAT, high-entropy blobs | none |
| `script.rpf` | 1011 PSIN, one 2.7 MB blob | none |
| `shaders.rpf` | 2 SGD5 shader stores, 1 PS09 pipeline cache | none |
| `appdata0.rpf` | 2 small encrypted blobs | none |
| `platform.ttpf`, `ttdb_graphics.ttpf`, `ttdb_props.ttpf` | RDS5 texture stores (1, 912, 211 textures) | none (textures only) |

So the only local mesh content is three small effect models. Props,
vehicles, peds and world geometry are in packs that are not present: per-
level/streamed content archives with more nested archives like the story
pack (where world drawables, drawable dictionaries and fragments would be),
and dedicated vehicle and character packs. Those are where standalone
drawables, fragments, skinned meshes, float3 positions and LODs 1..3 need
to be validated.

Not validated (no local sample): standalone drawables, LODs 1..3, the
float3 (99) position path, float2 texcoords, fragments, skinned meshes.
Not decoded: normal bits 20..31 (likely tangent frame), the meaning of
semantic 24, the +0xB0 lists, the geometry OBB, shader constants, embedded
texture dictionaries (no sample), the texture reference fields besides the
name record.
