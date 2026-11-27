# Head textures: negative results (shader 0x379efcef, 0 bindings)
Checked on packs.rpf (Xbox v1.0.2.1); no mapping found.
- Eyecap `144/40` bumptex `d100a0c3f8f516f3` occurs in 2 variation records of `5` (parts
  `a7f9101fee04a757`, `fed127d5714fadc4`); their diffuse `5ce55baa340248bd` is the eyecap's own
  diffusetex, so the clothing join (bumptex == variation normal) works for eyecaps but yields
  nothing for heads (heads have no bumptex).
- Entry `0` parses as 0 data blocks; `5` has 28 blocks but no part table (the part table lives in
  the small PSIN before each pack, see drawable_format.md). Head parts have no variation record.
- joaat of the labels (`head_fr1_lat_000_m` 0x88ffc97a, `player_story000_00_head_fr1_lat_000_m`
  0x1a0130e1) and the shader hash 0x379efcef match none of the 591 distinct variation hashes; the
  shader hash (LE bytes `efcf9e37`) is in no entry of data.rpf, common.rpf or script.rpf.
- `--grep head_fr1` finds nothing in data.rpf or common.rpf (names are not stored as strings).
- levels.rpf now opens (2,078 entries, key #2, 0.7s) after the offset bit 32
  fix (`8329918`), but it contains only world/map data: buildings, props,
  terrain, NPC spawn schedules. Zero head textures, zero skin tones, zero
  clothing palettes.
Conclusion: head skin is composed at run time from the face library (`7`..`24`, `62`..`89`,
`baseskintone` tags) in packs.rpf; not from levels.rpf or any other currently decoded archive.
