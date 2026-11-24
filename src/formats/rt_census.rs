//! Ray-traversal census: where RDNA2 programs call the BVH intersect ops, and
//! whether each call sits in the usual software traversal loop.
//!
//! A site is an `image_bvh_intersect_ray` or `image_bvh64_intersect_ray`. For each
//! site, over the reached instructions in address order of [`disassemble`]:
//!
//! - `sort`: a `v_min3_u32` / `v_med3_u32` / `v_max3_u32` within [`SORT_WINDOW`] after it
//!   (the hit children ordered by distance);
//! - `push`: the first `ds_write_b32` or `ds_write2_b32` within [`PUSH_WINDOW`] after it
//!   (the LDS stack);
//! - `ring`: the first `v_and_b32` with a literal ring mask `4 * (2^k - 1)` (`0xfc` or
//!   more, e.g. `0x3fc`, `0x7fc`) within [`PUSH_WINDOW`] after it (the stack byte
//!   index wrapped to a power-of-two ring of dwords);
//! - `loop`: the innermost backward branch around it: the shortest `[target, branch]`
//!   address span holding the site, from `s_branch` / `s_cbranch_*` with a target at
//!   or before the branch, or a long jump (`s_getpc` .. `s_setpc`) backwards;
//! - `node_type`: a `v_and_b32 .., 7, ..` in that loop (the node type in a node
//!   pointer's low bits), or within [`PUSH_WINDOW`] after the site when there is no loop;
//! - `cap`: an `s_cmpk_lt_u32 sN, 0xffff` in the same range (an iteration cap).
//!
//! `in_loop` counts reached instructions in the union of the sites' innermost loops.

use super::rdna2::{Flow, Stats};
use super::rdna2_disasm::{disassemble, Line};
use super::uniformity::long_jump;

pub const SORT_WINDOW: usize = 40;
pub const PUSH_WINDOW: usize = 80;
const PUSHES: [&str; 2] = ["ds_write_b32", "ds_write2_b32"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Site {
    pub at: usize,
    pub bvh64: bool,
    pub sort: bool,
    /// `ds_write_b32` or `ds_write2_b32`.
    pub push: Option<&'static str>,
    /// The ring mask literal.
    pub ring: Option<u32>,
    pub node_type: bool,
    pub cap: bool,
    /// Byte span `[target, branch]` of the innermost backward branch around the site.
    pub loop_span: Option<(usize, usize)>,
}

#[derive(Debug, Default)]
pub struct Census {
    pub stats: Stats,
    pub sites: Vec<Site>,
    pub in_loop: u32,
}

/// Mnemonic and operand tokens of a disassembly line.
fn tokens(text: &str) -> (&str, Vec<&str>) {
    let (m, rest) = text.split_once(' ').unwrap_or((text, ""));
    (m, rest.split([',', ' ']).filter(|t| !t.is_empty()).collect())
}

pub fn census(code: &[u8], wave32: bool) -> Census {
    let (stats, lines) = disassemble(code, wave32);
    let lines: Vec<Line> = lines.into_iter().filter(|l| l.inst.is_some()).collect();
    // Backward edges as (target, branch) byte offsets.
    let mut back = Vec::new();
    for l in &lines {
        let i = l.inst.unwrap();
        let next = (l.addr + i.len) / 4;
        let t = match i.flow {
            Flow::Jump(d) | Flow::Branch(d) => next.checked_add_signed(d as isize).map(|t| t * 4),
            Flow::End => long_jump(code, l.addr),
            Flow::Next => None,
        };
        if let Some(t) = t.filter(|&t| t <= l.addr) { back.push((t, l.addr)); }
    }
    let win = |from: usize, to: usize| lines[from.min(lines.len())..to.min(lines.len())].iter().map(|l| tokens(&l.text));
    let has = |from: usize, to: usize, f: &dyn Fn(&str, &[&str]) -> bool| win(from, to).any(|(m, ops)| f(m, &ops));
    let ring_mask = |(m, ops): (&str, Vec<&str>)| {
        if !m.starts_with("v_and_b32") { return None; }
        ops.iter().filter_map(|t| u32::from_str_radix(t.strip_prefix("0x")?, 16).ok())
            .find(|&v| v >= 0xfc && v & 3 == 0 && (v | 3).wrapping_add(1).is_power_of_two())
    };
    let span_idx = |(a, b): (usize, usize)| (lines.partition_point(|l| l.addr < a), lines.partition_point(|l| l.addr <= b));
    let mut sites = Vec::new();
    let mut covered = vec![false; lines.len()];
    for (k, l) in lines.iter().enumerate() {
        let (m, _) = tokens(&l.text);
        if m != "image_bvh_intersect_ray" && m != "image_bvh64_intersect_ray" { continue; }
        let loop_span = back.iter().filter(|&&(t, b)| t <= l.addr && l.addr <= b).min_by_key(|&&(t, b)| b - t).copied();
        let (lo, hi) = loop_span.map_or((k + 1, k + 1 + PUSH_WINDOW), span_idx);
        if loop_span.is_some() { covered[lo..hi].fill(true); }
        sites.push(Site {
            at: l.addr,
            bvh64: m.starts_with("image_bvh64"),
            sort: has(k + 1, k + 1 + SORT_WINDOW, &|m, _| ["v_min3_u32", "v_med3_u32", "v_max3_u32"].iter().any(|p| m.starts_with(p))),
            push: win(k + 1, k + 1 + PUSH_WINDOW).find_map(|(m, _)| PUSHES.into_iter().find(|&p| p == m)),
            ring: win(k + 1, k + 1 + PUSH_WINDOW).find_map(ring_mask),
            node_type: has(lo, hi, &|m, ops| m.starts_with("v_and_b32") && ops.contains(&"7")),
            cap: has(lo, hi, &|m, ops| m == "s_cmpk_lt_u32" && ops.get(1) == Some(&"0xffff")),
            loop_span,
        });
    }
    Census { stats, sites, in_loop: covered.iter().filter(|&&c| c).count() as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> { words.iter().flat_map(|w| w.to_le_bytes()).collect() }

    const BVH: [u32; 2] = [0xF198_0001, 0]; // image_bvh_intersect_ray
    const BVH64: [u32; 2] = [0xF19C_0001, 0]; // image_bvh64_intersect_ray
    const MIN3: [u32; 2] = [0xD553_0000, 0x040A_0300]; // v_min3_u32 v0, v0, v1, v2
    const DS_WRITE: [u32; 2] = [0xD834_0000, 0x0000_0100]; // ds_write_b32 v0, v1
    const RING: [u32; 2] = [0x3600_02FF, 0x7FC]; // v_and_b32 v0, 0x7fc, v1
    const NODE7: u32 = 0x3600_0287; // v_and_b32 v0, 7, v1
    const CAP: u32 = 0xB680_FFFF; // s_cmpk_lt_u32 s0, 0xffff
    const V_MOV: u32 = 0x7E00_0280;
    const ENDPGM: u32 = 0xBF81_0000;

    #[test]
    fn finds_traversal_template() {
        // L: v_and 7 ; cap ; bvh ; min3 ; ds_write ; ring ; s_cbranch_scc1 L ; v_mov ; end
        let body = [&[NODE7, CAP][..], &BVH, &MIN3, &DS_WRITE, &RING].concat();
        let back = 0xBF85_0000 | (-(body.len() as i16 + 1) as u16 as u32);
        let prog = [&body[..], &[back, V_MOV, ENDPGM]].concat();
        let c = census(&bytes(&prog), false);
        assert_eq!(c.sites.len(), 1);
        let s = c.sites[0];
        assert_eq!((s.at, s.bvh64, s.sort, s.push, s.ring, s.node_type, s.cap), (8, false, true, Some("ds_write_b32"), Some(0x7fc), true, true));
        assert_eq!(s.loop_span, Some((0, 4 * body.len())));
        assert_eq!((c.stats.insts, c.in_loop), (9, 7));
    }

    #[test]
    fn bare_site_and_windows() {
        // bvh64 with no loop; the sort lands past the 40-instruction window, the push inside 80.
        let pad = |n: usize| vec![V_MOV; n];
        let prog = [&BVH64[..], &pad(SORT_WINDOW), &MIN3, &DS_WRITE, &[ENDPGM]].concat();
        let c = census(&bytes(&prog), false);
        let s = c.sites[0];
        assert_eq!((s.bvh64, s.sort, s.push, s.ring, s.node_type, s.cap, s.loop_span), (true, false, Some("ds_write_b32"), None, false, false, None));
        // ds_write2_b32 v0, v1, v2 offset1:1 pushes two entries; 0x3fc is a 256-dword ring, 0x7fd is no ring mask.
        let prog = [&BVH[..], &[0x3600_02FF, 0x7FD, 0x3600_02FF, 0x3FC, 0xD838_0100, 0x0002_0100, ENDPGM]].concat();
        let s = census(&bytes(&prog), false).sites[0];
        assert_eq!((s.push, s.ring), (Some("ds_write2_b32"), Some(0x3fc)));
        assert_eq!(c.in_loop, 0);
        // An outer loop around an inner one: the inner (shorter) span wins.
        // O: v_mov ; I: bvh ; s_cbranch_scc1 I ; s_cbranch_scc1 O ; end
        let prog = [&[V_MOV][..], &BVH, &[0xBF85_FFFD, 0xBF85_FFFB, ENDPGM]].concat();
        let c = census(&bytes(&prog), false);
        assert_eq!((c.sites[0].loop_span, c.in_loop), (Some((4, 12)), 2));
        // Long jump backwards: s_getpc s[4:5] ; s_add_u32 s4, s4, lit ; s_addc_u32 s5, s5, 0 ; s_setpc s[4:5]
        let prog = [&BVH[..], &[0xBE84_1F00, 0x8004_FF04, (-12i32) as u32, 0x8205_8005, 0xBE80_2004]].concat();
        let c = census(&bytes(&prog), false);
        assert_eq!(c.sites[0].loop_span, Some((0, 24)));
    }
}
