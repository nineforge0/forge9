# Ray-traversal census: where the BVH intersect ops are

The title's ray tracing runs as compute shaders that walk the BVH themselves and
call `image_bvh_intersect_ray` / `image_bvh64_intersect_ray` once per node. A
translator that replaces those walks with inline `RayQuery` needs to know which
programs hold them, whether they share a shape, and how much of each program
the walk is. This census answers that over the whole shader database (entry 0
of `shaders.rpf`).

```
forge9 isa shaders.rpf 0 --rt-census          # text
forge9 isa shaders.rpf 0 --rt-census --json   # the same numbers as JSON
```

## Answer

- **716 intersect sites in 64 programs**, all compute: `image_bvh_intersect_ray`
  638 sites in 40 programs, `image_bvh64_intersect_ray` 78 sites in 24 programs.
  No program uses both. There are 2,647 compute programs in all (25,194 programs,
  24,804 records).
- **Every site sits in a backward-branch loop with a child sort and an LDS stack
  push**, but there are four shapes, not one (below).
- **45.4% of the 636,661 instructions** of those 64 programs lie in the innermost
  loop around a site. For the three biggest RT programs
  (`rtdirectionalshadows:CS_RTShadowTrace32*HQ_Tiled`) it is 57.7% to 60.5%.
- **The three biggest programs contain no intersect op.**
  `indirect_diffuse_lighting:CS_ScreenProbes_ActivateProbes`, `ActivateProbesTarget`
  and `ActivateRays` (88,862, 88,444 and 74,570 instructions) have 0 sites, and
  so do the 42 `*:CS_RaytracingResolve` variants and the two
  `CS_ShadowRenderTiledCascades*`. Of the 57 programs over 20,000 instructions
  (see [exec_uniformity.md](exec_uniformity.md)), only 9 have sites, all of them
  `rtdirectionalshadows:CS_RTShadowTrace32*`.

### Checked against an earlier, unverified run

| claim | measured | |
|---|---|---|
| 64 programs, 716 sites | 64, 716 | holds |
| 638 `image_bvh_intersect_ray` in 40, 78 `image_bvh64_intersect_ray` in 24 | same | holds |
| `v_min3/v_med3/v_max3_u32` within 40 instructions after every site | 716 of 716 | holds |
| `ds_write_b32` push within 80 after, ring index masked by `0x7fc`, at every site | a push at 716 of 716, but 319 are `ds_write2_b32` with no ring mask; `0x7fc` at only 56 sites | differs |
| all sites share one template | four shapes | differs |
| node-type tests on `node & 7` | `v_and_b32 .., 7, ..` in the loop at 716 of 716 | holds |
| 16 sites with `s_cmpk_lt_u32 sN, 0xffff` | 16 sites, in 6 programs, all 64-bit | holds |
| 648k instructions in the 64 programs | 636,661 | differs |
| 44.7% of them in the innermost loop | 45.4% | close |
| 58% to 60% for the three `CS_RTShadowTrace32*HQ_Tiled` | 57.7%, 59.3%, 60.5% | about |
| the three biggest programs have zero intersect ops | 0, 0, 0 | holds |

## The four shapes

| op | sites | programs | push | ring mask |
|---|---:|---:|---|---|
| `image_bvh_intersect_ray` | 319 | 40 | `ds_write_b32` | `0x3fc` (256 dwords) |
| `image_bvh_intersect_ray` | 319 | 40 | `ds_write2_b32` | none within 80 instructions |
| `image_bvh64_intersect_ray` | 56 | 15 | `ds_write_b32` | `0x7fc` (512 dwords) |
| `image_bvh64_intersect_ray` | 22 | 9 | `ds_write_b32` | `0xffc` (1,024 dwords) |

Every one of the 40 programs with 32-bit sites has as many `ds_write2_b32` sites
as `ds_write_b32` + `0x3fc` sites. Every program with 64-bit sites uses a single
ring size. The 16 iteration caps are all in 64-bit programs:
`indirect_light_probes:CS_Primary_GatherVBuffer_WithFar` (4),
`CS_IrradianceCache_GatherVBuffer_WithFar` (4), `CS_GatherAlphaVBuffer_WithFar` (2),
`CS_IrradianceCache_ShadowTrace_WithFar` (2), `CS_IrradianceCache_ShadowTrace` (1)
and `sr_tracing:CS_RaytraceShadows_Mixed` (3).

So a recognizer needs a few patterns, not one per program: the 32-bit pair and
the 64-bit ring, with the ring size as a parameter.

## By family

| family | programs | instructions | 32-bit sites | 64-bit sites | in loop |
|---|---:|---:|---:|---:|---:|
| `rtdirectionalshadows:CS_RTShadowTrace32*` | 20 | 438,557 | 512 | 0 | 43.0% |
| `shadowmask:CS_TraceShadowRays*` | 7 | 54,038 | 22 | 3 | 49.8% |
| `sr_tracing:CS_RaytraceVBuffer*` | 6 | 48,077 | 40 | 10 | 53.8% |
| `indirect_diffuse_lighting:CS_ScreenProbes_TraceRays*` | 8 | 28,047 | 16 | 24 | 53.6% |
| `rtdirectionalshadows:CS_RTDistant*` | 5 | 21,059 | 36 | 0 | 52.3% |
| `indirect_light_probes:CS_Primary_GatherVBuffer*` | 2 | 11,989 | 0 | 11 | 45.1% |
| `indirect_light_probes:CS_IrradianceCache_GatherVBuffer*` | 2 | 11,211 | 0 | 11 | 48.3% |
| `indirect_diffuse_lighting:CS_DeferredGather*` | 4 | 5,592 | 2 | 3 | 27.9% |
| `indirect_light_probes:CS_GatherAlphaVBuffer*` | 2 | 5,560 | 0 | 6 | 53.0% |
| `GpuParticleCollision:CS_CollisionRaytraced` | 1 | 3,750 | 0 | 1 | 19.2% |
| `sr_tracing:CS_RaytraceShadows*` | 2 | 3,587 | 6 | 3 | 69.7% |
| `indirect_diffuse_lighting:CS_PPR_Raytrace` | 1 | 2,274 | 0 | 3 | 57.6% |
| `indirect_light_probes:CS_IrradianceCache_ShadowTrace*` | 2 | 1,495 | 0 | 3 | 69.6% |
| `deferred_tiled:CS_ComputeClosestDepth` | 1 | 786 | 2 | 0 | 62.8% |
| `deferred_tiled:CS_LaserLightShadowRaytracing` | 1 | 639 | 2 | 0 | 74.8% |
| **all** | **64** | **636,661** | **638** | **78** | **45.4%** |

The RT directional shadows are 69% of the instructions in these programs. The
per-program table is in the command's output.

## Method (`src/formats/rt_census.rs`)

Programs whose decoded opcodes include either intersect op are disassembled
with `forge9`'s RDNA2 disassembler. Over the reached instructions in address
order, for each site:

| check | rule |
|---|---|
| sort | a `v_min3_u32`, `v_med3_u32` or `v_max3_u32` within 40 instructions after the site |
| push | the first `ds_write_b32` or `ds_write2_b32` within 80 after |
| ring | the first `v_and_b32` with a literal `4 * (2^k - 1)` (`0xfc` or more) within 80 after |
| loop | the innermost backward branch around the site: the shortest `[target, branch]` span that holds it, from `s_branch` / `s_cbranch_*` with an earlier target, or a long jump (`s_getpc` .. `s_setpc`) backwards |
| node type | a `v_and_b32 .., 7, ..` in that loop |
| cap | an `s_cmpk_lt_u32 sN, 0xffff` in that loop |

"In loop" counts the reached instructions in the union of the sites' innermost
loops, over all instructions of the program.

**Limits.** The checks are syntactic: they look for the instructions, not at
data flow, so a match says the pieces are present near the site, not that they
feed each other. The node-type test is weak, since a long loop body will hold a
`v_and_b32 .., 7` for other reasons. The loop is an address span, not a proven
natural loop, and it holds everything the loop body does besides traversal
(any-hit and alpha tests, for example), so "in loop" is an upper bound on what
a `RayQuery` replacement would remove, not a measurement of it.

**Tests**: `finds_traversal_template` (a synthetic loop with every piece) and
`bare_site_and_windows` (window edges, no loop, nested loops, a backward long
jump, `ds_write2_b32` and the ring-mask rule).
