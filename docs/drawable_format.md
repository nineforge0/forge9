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
then embeds `diffusetex` as `baseColorTexture`, `bumptex` as
`normalTexture` and `emissivetex` as `emissiveTexture` (factor 1) (PNG,
mip 0; BC5 gets Z rebuilt as `sqrt(1 - x^2 - y^2)`, BC4 is grey).
Materials set `metallicFactor` 0; other textures are reported, not mapped.
Several entries (`model packs.rpf 102/33 102/30 119/75`) are merged into
one scene: all parts of a character share one bind-pose frame, so they
line up without transforms.

The skulls' 12 references resolve in none of `platform.ttpf`,
`ttdb_graphics.ttpf` or `ttdb_props.ttpf`: their store is not present.
The mapping is confirmed on the larger packs (`core_restofworld.rpf`,
`shared_minigame000.rpf`, `packs.rpf`: 15131 drawables, 145125
references, 16293 distinct keys) against 19 stores (78169 textures):

- 30901 references (3299 distinct keys) equal a store hash exactly. A
  chance 64-bit match is about `16293 * 78169 / 2^64`, i.e. none.
- Resolved formats follow the parameter: `diffusetex` is sRGB in 3481 of
  3525 (BC7, BC1, BC3 `_SRGB`), `bumptex` is linear in all 2543 (BC1,
  BC5, BC7 `UNORM`).
- `cvar_am_python_head_01a` (`core_restofworld.rpf` `1/28`) resolves its
  diffuse and bump maps in `ttdb_props.ttpf`; rendered, the scales, eye
  and mouth land on the mesh where they belong.
- 19166 references use key `5d7a3a87cc19967d` (name hash 0x5c74c225,
  joaat of `givemechecker`) for unbound slots; it is in no store, so it
  reads as a null texture.

#### Deduplicated keys (`EDUP`)

Many keys are in no store because their pixels are stored once under
another key. `textures.rpf` (next to `packs.rpf`) has one top-level entry
starting `EDUP`: u32 count, then count `{u64 key, u64 stored key}`
records sorted by key (17431 records; data follows that is not decoded).
No record's key is in a store; 13707 of the stored keys are, so the
table is key -> canonical copy. `model` reads it from `--dedup`, default
`textures.rpf` next to the archive, and falls back to the stored key
when a key is not found directly. On the character packs it turns e.g.
the player eyes from 2 of 4 to 4 of 4 resolved, and resolves the shared
head `speculartex2` and many head `bumptex` maps (512x1024 BC5).

#### Parameter names found by joaat

`emissivetex` (0x8e0d63ee) and `materialid` (0x8adb8480) match their
hashes among about 8000 candidate names for 10 unknown parameters, so a
chance match is unlikely; they are in the built-in dictionary.

Stores are opened lazily (`texture::LazyStore`): only the header, chunk
table and ETPF index are read, then the chunks of each texture that is
used. A store with no header is skipped with a warning, and a texture whose
chunks are missing or all zeros (a partially downloaded file) is reported
as not decoded instead of failing the export.

A `bumptex` whose texels are all identical and encode no unit vector is
not embedded (`constant placeholder`). Many ped and prop shaders bind one
of two 4x4 textures (`ttdb_peds_3_3.ttpf` #165 BC5, `ttdb_weapons.ttpf`
#1368 BC1) whose every texel is zero; as a normal map that is (-1, -1, ~0)
and renders black (it blacked out half of each legacy head, below). A flat
`(0, 0, 1)` map is still embedded.

With store textures forced in place of the keys, the output passes the
Khronos glTF validator with 0 errors (warning: no TANGENT, since normal
bits 20..31 are not decoded); the untextured export has 0 errors and 0
warnings.

## Character (ped) drawables

Checked on `packs.rpf`: `144` holds the two player characters
(`player_story000_00_*` body type `fr1`, `player_story000_01_*` `mr1`),
`100`/`102`/`104` cutscene characters (`cs_*`), `151` unique characters
(`u_*`), `148` costumes (`sc_*`), `119` hands, `126` generic heads, `91`,
`96`, `139`, `141`, `146`, `153`, `155` clothing and accessories.
Each name has three entries (LODs as separate drawables; the largest is
the most detailed). All parts of one character share a bind-pose frame
(A-pose, origin at the pelvis, Z up, facing -Y).

What a full textured character needs, and what the data has:

- Heads share one topology (2958 / 8983 / 599 vertices for the three
  entries) and UV layout; the face texture is a 1024x1024 BC1/BC7 sRGB
  `diffusetex`, with a 512x1024 BC5 `bumptex`. Player heads bind no
  texture at all (shader `0x379efcef`, 0 textures), most generic and
  cutscene heads bind a `diffusetex` key that is in none of the available
  stores; a few unique and cutscene heads resolve fully.
- Hair, brows, lashes and beards are 3-vertex stubs in all 661 human
  cases (one triangle, one 16x16 texture): hair geometry is not in these
  drawables.
- There are no body skin drawables for the players; skin shows only
  through hands (`119`) and the head. Most hand `diffusetex` keys are not
  in the available stores.
- Most clothing has no `diffusetex`: in the player pack 306 of the 328
  shaders that bind `bumptex` also bind `materialid`, a BC4 mask with a
  few flat grey levels, and only 42 bind a `diffusetex` (eyes, eye caps,
  the stubs, a watch, a cap). Colour comes from per-region materials
  outside the texture (the shader constants after the texture slots hold
  scalars and a leading u32, not colours), not decoded here.
- The `cs_ghost_*` cutscene character uses shader `0xb8916043` with
  `diffusetex`, `bumptex`, `speculartex`, `speculartex2` and
  `emissivetex` on head, suit, trousers and cap, all of which resolve
  (with EDUP); it is the only complete human found that is textured
  without guessing colours.

### With the full data (60 stores, 358239 textures)

Rechecked with every `.ttpf` of the complete install (57 stores, plus the
3 in `bin/`) and the EDUP table:

- Heads: of 1651 head drawables (the largest entry of each name), the face
  `diffusetex` resolves for 20: `cs_ghost` (3), `cs_hitman`,
  `cs_mangrove_hunter`, `cs_raul` (earring variant), five `sc_*` costumes,
  four `u_*` uniques, three generic `head_mr1_cau`, one creature. The
  leads' heads (`cs_lucia_head_fr1_lat`, `cs_lucia_old_head`,
  `cs_jason_old_head`, the `player_story000_*` heads) and most female
  heads bind no texture at all; most others bind 7 (`diffusetex`,
  `bumptex`, `speculartex`, `speculartex2`, three null slots) and only the
  diffuse key is missing. Those keys are in no store and nowhere in
  `textures.rpf`, so the face colour is produced at run time.
- The 64-bit store key is not FNV-1, FNV-1a, xxHash64, MurmurHash64A or
  CRC-64 of the texture name (checked against the null texture,
  `givemechecker` -> `5d7a3a87cc19967d`), so a missing key cannot be found
  by name.
- Hair drawables (`117`) are still 3-vertex stubs.
- Clothing: the constant buffer (shader +0x08) starts with a pointer to
  its own +0x08, then the values. A constant parameter's packed word is
  consistent with `low 2 bits 3, (w >> 8) & 0x3fff` byte offset into the
  values, `w >> 22` 32-bit words (in `player_..._basebottomssports` the 29
  offsets ascend and each gap fits the count; 4-word values at 16-byte
  boundaries). The values are scalars and flags (1, 0.05, 0.15, 24, ...);
  no colour table is there or in the texture records that follow. The
  `materialid` maps are BC4 or A8 (DXGI 65, tile mode 2, not decoded), so
  clothing colour per material region is outside the drawable (likely in
  the PSIN ped metadata beside the packs, e.g. `packs.rpf` `143`, `145`;
  not decoded).
- The leads do have fully textured low-detail stand-ins in `102`:
  `cs_lucia_legacypedcs_fr1_000_fullbody` + `cs_lucia_legacyheadcs_fr1_001`
  (874 + 296 vertices) and `cs_jason_legacypedcs_mr1_000_fullbody` +
  `cs_jason_legacyheadcs_mr1_000` (772 + 302). Shader `0x7574b09b`;
  each binds `diffusetex` (256x256 BC1 with the face, hair, outfit and
  skin baked in), `speculartex`, `speculartex2`, the placeholder
  `bumptex` and one null slot: 4 of 5 references resolve per part. Body
  and head share the bind-pose frame of the detailed parts. The
  `legacypedworld_*` crowd peds (`135`) use the same scheme.

### Ped metadata (`forge9 ped-meta`)

The PSIN entries beside the character packs in `packs.rpf` tie drawables
to textures outside the drawable. Common layout (all checked on real
files; parsed by `formats::pedmeta`, synthetic tests in that module):

- PSIN data: `PSIN`, u32 big-endian size, then 16-byte block rows `{u32
  struct hash, u32 size, 8 x 0xaa}` ended by 8 x 0x70. Block data
  follows, every block starting on a 16-byte boundary (without the
  alignment, records of the 2.6 MB `packs.rpf` `5` land 44 bytes off).
- Hash maps are node arrays: 16 control bytes, then 16 entries; a control
  byte with the top bit set (0x80) is an empty slot. In `143` the first
  node's control bytes are `60 43 59 1b 05 80 x11`, and exactly its first
  five entries hold data.

**Part table** (`143` before the player pack `144`, `101` before `102`,
and the other small PSINs between packs): block `0x101`, 64-byte entries,
one per drawable name of the pack (`143`: 130 parts, `144`: 130 distinct
names):

| Offset | Field | Evidence |
|-------:|-------|----------|
| +0x00 | u64 part id | equals +0x20 in all 130 |
| +0x08 | properties map (24 bytes): `{u64, u32 ref, u32, u16 count, u16 capacity}` to `{u32 group hash, u32 0, u32 n, u32 n, u32 ref}` rows of u32 tag hashes | group `0x90810417` is joaat `baseskintone`, tag `0xc28f3ecd` is `skin_01`; another group lists `head_fr1_base_000_m_full` and `head_mr1_base_000_m_full` (joaat) |
| +0x20 | u64 part id again | |
| +0x28, +0x30 | u64, u64 | both zero in the 20 hair parts; +0x28 zero, +0x30 set in the 2 head parts; both set elsewhere |
| +0x38 | u32 | 0 in all 130 |
| +0x3C | u8 kind | `143`: 101 x 23 (garments), 20 x 30 (hair stubs), 2 x 62 (the two heads), 2 x 60 (eyes), 2 x 66, 3 x 1 (accessories, e.g. the watch) |

The part id is a 64-bit hash of the drawable name; it is not FNV-1,
FNV-1a, xxHash64, MurmurHash64A or CRC-64 of any of the 130 names (with
or without `_full`, `.#dr`), so a part is tied to its drawable through its
textures (below). A property ref `v` is block `v & 0xfff` (1-based),
byte offset `v >> 20`, checked only on offsets below 4 KB.

**Texture variation records** (`5`, 2.6 MB, blocks of struct
`0x37e52277`, 184 bytes, 7227 non-empty records):

| Offset | Field | Evidence |
|-------:|-------|----------|
| +0x00 | u32 variation name hash | `0x44e21c90` on most |
| +0x08 | u64 part id | 254 records name player parts of `143` |
| +0x10 | u64 diffuse or region-map key | in a store for 4459 records |
| +0x18 | u64 normal key | in a store for 3710; equals the drawable's bound `bumptex` key, which is how a part id is matched to its drawable (256 player drawables linked) |
| +0x20 | u64 | in no store |

For player garments +0x10 is mostly `ttdb_peds_1_3` #1142, a black
512x512 BC1 marked `DONTDEDUPE` (a null), or a flat green/orange/blue
region map (material regions, colours not decoded); a few are real
albedo maps (`player_story000_00_jacketbiker`: `ttdb_peds_1_3` #3250,
worn black leather). Hair parts point at the 16x16 stub texture.

`159` maps part ids to `{u64, u64 store key}` (a 16-slot map, block
`0x103`), where the key is one of three shared 512x512 body region masks
(torso, legs).

**Face library** (`7`..`24`, `62`..`89`): 0x101 map nodes of 96-byte
entries `{u64 store key, properties map, u64 key again, two empty maps,
u32 ARGB average colour, bool}`. 15302 records, 1564 of the 1572
1024x1024 sRGB ped textures (all the bald face skins); tags only on
`baseskintone` (`skin_00`..`skin_05`). cs_hitman's face (`0ea5da3b...`)
has average colour `ff5f6c96`.

### Why the leads' faces stay untextured

- Lucia's head (`cs_lucia_head_fr1_lat_000_y`, `102/478`, `482`, `483`)
  and the player heads bind shader `0x379efcef` with 9 texture parameters
  (`diffusetex`, `bumptex`, `speculartex`, `speculartex2`, five unnamed)
  and every slot pointer null; no u64 in any of their entries is a store
  key (scanned at every 4-byte offset against all 358239 keys and the
  EDUP table).
- The two player head parts (kind 62, ids `d8f651e6867c7a36`,
  `1500dd133e3da6b2`) have no variation record in `5` and no entry in
  `159`; both ids and their +0x30 values occur in no other top-level PSIN
  of `packs.rpf`, and `d8f651e6867c7a36` occurs in no entry of
  `data.rpf`, `common.rpf`, `script.rpf`, `story000_script.rpf`,
  `story000.rpf` or the four other packs (`search --grep-hex`, which
  inflates every entry; `1500dd...` checked in `story000.rpf` only).
- 7-slot heads (`cs_artim` etc.) and some cutscene garments
  (`cs_lucia_tshirtcrop`, all four textures, normal map included) bind keys
  that are in no store and occur nowhere else in readable data; the face
  library's tags never match a head part's tags.
- Every file listed by the install layout is present. What is not
  readable: 23 nested archives of `levels.rpf` (0.75 GB) and one of
  `data.rpf` open with none of the 9 key schedules. The executable's
  schedule table (`{u64 address, u32 0x10f4}` descriptors) lists 22
  schedules: the 9 known ones and 13 more with other leading words
  (`5dce1827` x8 and five singles); with the current cipher tables none
  of the 13 opens those archives. The missing face, garment and 7-slot
  head textures are most likely there or composed at run time; neither is
  decoded.

The leads' only complete textures are therefore the legacy stand-ins
(`cs_lucia_legacypedcs` + `cs_lucia_legacyheadcs`,
`cs_jason_legacypedcs` + `cs_jason_legacyheadcs`), whose 256x256 maps
carry the face, hair and outfit.

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

### Full install

With the complete data (`forge9 search <archive> --drawable`, which now
prints each hit's embedded names):

| Archive | Drawable entries | Content |
|---------|-----------------:|---------|
| `pack/story000/story000.rpf` (762 entries) | 4160 | terrain/map tiles (`s09_w13_*` ... `s16_w17_*`), 6 effect models, 5 `anm_story0_*`; the largest nested archives (up to 2.8 GB) are AWC audio. No characters or vehicles |
| `packs.rpf` | 24423 | peds: clothing, heads, hair stubs, cutscene and unique characters, animals (`97`), legacy crowd peds (`135`) |
| `levels.rpf` | 80000+ | world and interiors; vehicles only as `*_batchlod` stand-ins (626 in `321`, 7 to 30 KB, LOD0 one mesh) and one interior display plane (`bk04_sky1b_vehicles`, `346/991`: 21 meshes, 59810 vertices, 124 of 129 references resolve; the paint shaders bind no `diffusetex`) |
| `data.rpf` | 1163 | weapons (`w_*`), effect and core models |

23 nested archives of `levels.rpf` (0.75 GB, e.g. `333`, `420`, `446`..`456`)
and one of `data.rpf` do not open with any key of `keys.bin`; detailed
vehicles, if anywhere, are in those or are fragments.

Not validated (no local sample): standalone drawables, LODs 1..3, the
float3 (99) position path, float2 texcoords, fragments, skinned meshes.
Not decoded: normal bits 20..31 (likely tangent frame), the meaning of
semantic 24, the +0xB0 lists, the geometry OBB, shader constants, embedded
texture dictionaries (no sample), the texture reference fields besides the
name record.
