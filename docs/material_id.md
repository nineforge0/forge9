# Clothing material-ID (issue #16)

Findings (packs.rpf `144/*` clothing vs ped-meta `5`):

- The `materialid` key (hoodie `144/53`: `f3d83182034e96d0`; `144/64` `153c8ea9d5d0caca`;
  `144/65` `deec5ce7da570559`; `144/15` `6d435fa20dfcc781`) matches NO field of the 7227 variation
  records (`part`, `diffuse`, `normal`, `extra`, `variation`). It is a plain store key: a 1024x1024
  BC4 mask in `ttdb_peds_0_3.ttpf` (#6230 for the hoodie), i.e. a region mask, not a lookup id.
- The working link is `bumptex` key == variation `normal` key. For the hoodie this gives variation
  `47dd78c2`, part `639ad17b7da3261e`, diffuse `2e67967223889f9d` = `ttdb_peds_1_3` #1142, the
  512x512 black BC1 null texture. So clothing "diffuse" is mostly a placeholder; the real colour is
  not in these records (see docs/drawable_format.md, ped metadata).
- `forge9 model ... --ped-meta 5` applies that link: a material with `bumptex` but no `diffusetex`
  gets a synthetic `diffusetex` from the matching variation's `diffuse` key (skipped if zero).
- `--textures` must list `.ttpf` stores (e.g. `data/ttdb/**/*.ttpf`), not `textures.rpf`.
