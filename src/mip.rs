//! Mipmap level selection (issue #9's port of upstream `_viv_get_mipmap`'s
//! size math, viv.c:14146-14300).
//!
//! Since #81 (ADR 0002 D7 — mip retirement for regular sizes) the runtime
//! NEVER calls [`select_mip_level`]: the GDI arm draws shrinks from the
//! single full-resolution face and the D2D arm samples the full master.
//! The quirk parity below is deliberately DROPPED, not kept: level
//! selection is no longer an observable behavior, so replicating the quirk
//! would be deliberately worse output (recorded in README Differences).
//! The function and its counterexample tests stay as the historical
//! record; #82's giant tiering uses [`mip_size`] (via
//! `tile::detail_level`) and [`downscale_box`], NOT this quirk-parity
//! loop — a giant must descend to the deepest level that still covers the
//! render, which is a different rule (see `tile::detail_level`).
//!
//! Levels are numbered from 0 = the original frame; level k has size
//! `((w+1) >> k, (h+1) >> k)` computed from the ORIGINAL dimensions each
//! time — never by iteratively halving the previous level (for w=5 the
//! from-original level 2 is 6/4 = 1, the iterative one (3+1)/2 = 2; viv.c
//! always uses `(image_wide+1)/(2<<(k-1))`, viv.c:14158/14264).
//!
//! [`select_mip_level`] is a verbatim port of upstream's loop, including
//! its quirk: the render size is compared against `mip_wide` on BOTH sides
//! — `render_h >= mip_w` — and `mip_h` never participates (viv.c:14181 and
//! 14287, identical at both sites). Kept byte-for-byte as the historical
//! record of what the #9–#80 chain actually selected (a wide-short image
//! in a tall viewport picked a shallower level than the "correct" compare
//! would have — see the counterexample tests below).
//!
//! Invariants of the loop (provable, and pinned by tests): a returned
//! level > 0 always satisfies `rw < mw` (the mag arm never saw a mip);
//! `rh < mh` is NOT guaranteed — the quirk could hand the shrink arm a
//! level far shorter than the render height, and the shrink arm then
//! vertically magnified that mip (upstream does the same). #82's tiering
//! inherits these properties if it reuses the loop.

/// Size of mipmap `level` for an `image_wide x image_high` frame. Level 0
/// is the frame itself. Each dimension rounds `(dim+1)/2^k` down and clamps
/// at 1 (viv.c:14158-14169/14264-14275).
/// Live since #82 (the tiering and the level cache both size through it).
pub(crate) fn mip_size(image_w: i32, image_h: i32, level: u32) -> (i32, i32) {
    if level == 0 {
        return (image_w.max(1), image_h.max(1));
    }
    let w = ((image_w + 1) >> level.min(31)).max(1);
    let h = ((image_h + 1) >> level.min(31)).max(1);
    (w, h)
}

/// Box-downsample a top-down, tightly packed BGRA buffer to `level`
/// ([`mip_size`]'s dimensions) — the CPU side of the #82 overview: a
/// giant's frame that cannot be a single device bitmap still needs *some*
/// uploadable source for its shrunken form, and that source is a
/// prefiltered box average of the master.
///
/// One pass from the ORIGINAL dimensions (never by halving a previous
/// level — the `(w+1)>>k` from-original rule [`mip_size`] implements), so
/// a deep level costs one source read, not k.
///
/// Block edges are integer `floor(x*dim/level_dim)` boundaries, which
/// partition the source exactly: every source pixel lands in exactly one
/// block (the tiles-cover-everything property the seam standard leans on),
/// with fractional-boundary area weighting deliberately not attempted —
/// upstream's own generation was a GDI HALFTONE StretchBlt, i.e. also a
/// resampler's approximation, and the #82 contract for the overview is
/// "uniform, prefiltered, no aliasing stripes", not "area-exact".
///
/// The master is opaque by construction (`composite_over_background_*`
/// forces alpha 255), so a straight 4-byte average is the correct
/// non-premultiplied filter — no channel needs special treatment, and the
/// alpha byte averages back to 255 on its own.
pub(crate) fn downscale_box(src: &[u8], image_w: i32, image_h: i32, level: u32) -> Vec<u8> {
    let (dest_w, dest_h) = mip_size(image_w, image_h, level);
    debug_assert_eq!(src.len(), image_w as usize * image_h as usize * 4);
    let (sw, sh) = (image_w.max(1) as usize, image_h.max(1) as usize);
    let mut out = vec![0u8; dest_w as usize * dest_h as usize * 4];
    for dy in 0..dest_h as usize {
        let y0 = dy * sh / dest_h as usize;
        let y1 = ((dy + 1) * sh / dest_h as usize).max(y0 + 1).min(sh);
        for dx in 0..dest_w as usize {
            let x0 = dx * sw / dest_w as usize;
            let x1 = ((dx + 1) * sw / dest_w as usize).max(x0 + 1).min(sw);
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in y0..y1 {
                let row = y * sw * 4;
                for x in x0..x1 {
                    let px = row + x * 4;
                    for (c, s) in sum.iter_mut().zip(&src[px..px + 4]) {
                        *c += u32::from(*s);
                    }
                    count += 1;
                }
            }
            let out_px = (dy * dest_w as usize + dx) * 4;
            for (c, s) in out[out_px..out_px + 4].iter_mut().zip(sum) {
                *c = (s / count) as u8;
            }
        }
    }
    out
}

/// The CPU cap for cached mip levels (#82): a giant's overview levels are
/// bounded by ~4× the render dimensions each, so a handful of zoom steps'
/// worth costs tens of MB — far under this, and small next to the 512 MB
/// frame budget the master itself lives in.
pub(crate) const LEVEL_CACHE_BYTES: u64 = 128 << 20;

/// The CPU cache behind [`crate::gpu::LevelSource`]'s deeper levels (#82):
/// a giant's overview level is a full source pass, so it is built once per
/// (frame, level) and kept until the LRU's byte cap pushes it out. The
/// master itself is never copied — level 0 is served straight from the
/// frame (the caller's `LevelSource` handles that).
#[derive(Debug)]
pub(crate) struct LevelCache {
    /// The frame generation the entries belong to; a new generation clears
    /// them (the build reads that frame's pixels).
    frame_gen: u64,
    /// Up to a handful of levels, coldest evicted first — the same pure
    /// LRU policy the tile cache uses.
    lru: crate::tile::Lru<u32>,
    entries: Vec<(u32, Vec<u8>)>,
    /// Levels built since the last clear (the ledger's `mip_builds`).
    pub(crate) builds: u64,
}

impl LevelCache {
    pub(crate) fn new(cap: u64) -> Self {
        Self {
            frame_gen: u64::MAX,
            lru: crate::tile::Lru::new(cap),
            entries: Vec::new(),
            builds: 0,
        }
    }

    /// Rebind to a frame generation, dropping the previous frame's levels
    /// — a caller that never asks for a level of the new frame (an ordinary
    /// image after a giant) would otherwise keep the old frame's bytes
    /// resident AND counted into the ledger's source class.
    pub(crate) fn rebind(&mut self, frame_gen: u64) {
        if self.frame_gen != frame_gen {
            self.clear();
            self.frame_gen = frame_gen;
            self.builds = 0;
        }
    }

    /// Drop every level (a new frame generation, or a caller that wants
    /// the CPU bytes back).
    pub(crate) fn clear(&mut self) {
        self.lru.clear();
        self.entries.clear();
    }

    /// The CPU bytes held (the ledger's `cpu_source` half).
    pub(crate) fn bytes(&self) -> u64 {
        self.lru.total_bytes()
    }

    /// `level` of `image_w x image_h`, built from `src` on first use.
    /// `None` when the level's bytes exceed the whole cap (the caller then
    /// degrades — refusing is the honest answer; the ladder's coarser
    /// levels are the intended response and they are 4× smaller each
    /// step).
    pub(crate) fn get_or_build(
        &mut self,
        level: u32,
        image_w: u32,
        image_h: u32,
        frame_gen: u64,
        src: &[u8],
    ) -> Option<(u32, u32, &[u8])> {
        self.rebind(frame_gen);
        let (wide, high) = mip_size(image_w as i32, image_h as i32, level);
        if let Some(idx) = self.entries.iter().position(|(l, _)| *l == level) {
            self.lru.touch(&level);
            let (_, data) = &self.entries[idx];
            return Some((wide as u32, high as u32, data));
        }
        let bytes = wide as u64 * high as u64 * 4;
        if bytes > self.lru.cap() {
            return None;
        }
        let data = downscale_box(src, image_w as i32, image_h as i32, level);
        for evicted in self.lru.insert(level, bytes) {
            self.entries.retain(|(l, _)| *l != evicted);
        }
        self.entries.push((level, data));
        self.builds += 1;
        let (_, data) = self.entries.last()?;
        Some((wide as u32, high as u32, data))
    }
}

/// The deepest mipmap level worth rendering `render_wide x render_high`
/// from (upstream `_viv_get_mipmap`'s selection loop, viv.c:14158-14298).
///
/// Returns 0 (the original) when the render is at least half the image's
/// width — or, per the quirk, at least half its width measured against the
/// render HEIGHT. Otherwise returns the smallest level whose NEXT level
/// would no longer fit the render; generation fills levels 1..=level on
/// demand (the chain caches them for the frame's lifetime, freed on frame
/// replacement like upstream's `_viv_mipmap_free`, viv.c:1252-1258).
#[allow(dead_code)] // the historical record + counterexample net (see the module doc)
pub(crate) fn select_mip_level(image_w: i32, image_h: i32, render_w: i32, render_h: i32) -> u32 {
    // Level 1's size and the early returns, viv.c:14158-14190.
    let (mip_w, mip_h) = mip_size(image_w, image_h, 1);
    if mip_w == 1 && mip_h == 1 {
        return 0;
    }
    // Upstream quirk (viv.c:14181): render_h is compared against
    // mip_wide, not mip_high — kept verbatim at both check sites.
    if render_w >= mip_w || render_h >= mip_w {
        return 0;
    }
    // The descent: each iteration's level is `level`; after sizing the
    // NEXT level, either it is degenerate / fits the render (return the
    // current level) or the loop continues (viv.c:14200-14298).
    let mut level = 1u32;
    loop {
        let (next_w, next_h) = mip_size(image_w, image_h, level + 1);
        if next_w == 1 && next_h == 1 {
            return level;
        }
        if render_w >= next_w || render_h >= next_w {
            return level;
        }
        level += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_sizes_come_from_the_original_dimensions() {
        // (w+1)/2^k from the original, each dim clamped at 1 — the w=5
        // divergence test (iterative halving would give level 2 = 2).
        assert_eq!(mip_size(5, 5, 0), (5, 5));
        assert_eq!(mip_size(5, 5, 1), (3, 3));
        assert_eq!(mip_size(5, 5, 2), (1, 1));
        assert_eq!(mip_size(40000, 256, 1), (20000, 128));
        assert_eq!(mip_size(40000, 256, 3), (5000, 32));
        // The formula is from the ORIGINAL dims even past level 1: for
        // height 257, level 2 is (257+1)/4 = 64 — iteratively halving
        // level 1's 129 would give 65 (the F6 divergence, review PR #18).
        assert_eq!(mip_size(1000, 257, 1), (500, 129));
        assert_eq!(mip_size(1000, 257, 2), (250, 64));
    }

    #[test]
    fn degenerate_levels_clamp_to_one() {
        assert_eq!(mip_size(2, 2, 1), (1, 1));
        assert_eq!(mip_size(2, 2, 5), (1, 1));
        assert_eq!(mip_size(1, 1, 1), (1, 1));
        assert_eq!(mip_size(40000, 256, 31), (1, 1));
    }

    #[test]
    fn a_render_at_least_half_the_width_uses_the_original() {
        // 2000 >= 20000 fails, 400 < 20000 fails → descend; but a render
        // of 20000 or more takes the original (viv.c:14181).
        assert_eq!(select_mip_level(40000, 256, 20000, 100), 0);
        assert_eq!(select_mip_level(40000, 256, 40000, 256), 0);
        assert_eq!(select_mip_level(800, 600, 400, 300), 0);
    }

    #[test]
    fn small_images_never_descend() {
        // mip1 is 1x1 → the original, before any render compare
        // (viv.c:14171).
        assert_eq!(select_mip_level(2, 2, 1, 1), 0);
        assert_eq!(select_mip_level(1, 1, 1, 1), 0);
    }

    #[test]
    fn a_deep_zoomout_lands_on_the_smallest_level_that_still_exceeds_it() {
        // 40000x256 banner, fit into ~2000x13: levels 20000/10000/5000/
        // 2500/1250; the render fits 1250 (next after 2500) → level 4.
        assert_eq!(select_mip_level(40000, 256, 2000, 13), 4);
        // Deeper still: render 800 wide → level 5 (1250 -> next 625 fits).
        assert_eq!(select_mip_level(40000, 256, 800, 5), 5);
        // A returned level > 0 always exceeds the render width (the mag
        // arm never sees a mip; proven from the loop, pinned here).
        let level = select_mip_level(40000, 256, 800, 5);
        let (mw, _) = mip_size(40000, 256, level);
        assert!(800 < mw);
    }

    #[test]
    fn the_height_quirk_compares_against_the_level_width() {
        // Upstream compares render_h with mip_wide (viv.c:14181). Wide
        // banner 40000x100: mip1_w = 20000; a render 1000x2000 should
        // early-out on the HEIGHT term (2000 >= 20000 is false here...)
        // — instead build the loop-internal case: 40000x100, render
        // 1000x2000 descends until a next level whose WIDTH fits 2000:
        // levels 20000,10000,5000,2500 (next 1250 <= 2000) → level 4
        // with mh=6 — the render height towers over the level height and
        // the shrink arm vertically magnifies (counterexample A).
        let level = select_mip_level(40000, 100, 1000, 2000);
        assert_eq!(level, 4);
        let (mw, mh) = mip_size(40000, 100, level);
        assert_eq!((mw, mh), (2500, 6));
    }

    #[test]
    fn the_quirk_can_return_the_original_where_the_correct_compare_would_not() {
        // 3000x40000 tower in a 400x2000 viewport: fit render 150x2000,
        // mip1_w = 1500; 150 < 1500 but 2000 >= 1500 hits the QUIRK term
        // (viv.c:14181) → the original, even though mip1_h (20000) towers
        // over the render (counterexample B). A height-correct compare
        // (2000 >= 20000) would have descended.
        assert_eq!(select_mip_level(3000, 40000, 150, 2000), 0);
    }

    #[test]
    fn tall_narrow_images_never_get_a_mip() {
        // 100-wide tower: mip1_w = 50 fits any sane render width → the
        // original forever (counterexample C; upstream the same).
        assert_eq!(select_mip_level(100, 40000, 100, 30000), 0);
        assert_eq!(select_mip_level(100, 40000, 50, 50), 0);
    }

    #[test]
    fn degenerate_viewports_sink_to_the_deepest_level() {
        // A zero/degenerate render (riviv clamps the sampled viewport at
        // 0): every compare is false, the chain runs until the next level
        // would be 1x1 — mirroring upstream, whose _viv_load_render_high
        // can go negative (viv.c:1558) with the same all-false compares.
        let level = select_mip_level(40000, 256, 0, 0);
        assert_eq!(mip_size(40000, 256, level + 1), (1, 1));
        assert_ne!(mip_size(40000, 256, level), (1, 1));
    }

    #[test]
    fn selection_at_half_the_viewport_never_exceeds_selection_at_the_full_viewport() {
        // A monotonicity property of the selection loop (the #82 tiering's
        // coarse-then-fine ladder leans on it): for the same window,
        // selecting against HALF the viewport (the coarse estimate the old
        // #9 pre-generation used) always lands on a level >= the one the
        // full-viewport fit render selects — never shallower.
        let (vw, vh) = (2000i32, 1200i32);
        let coarse = select_mip_level(40000, 256, vw / 2, vh / 2);
        let fit_render_w = vw; // 40000x256 fit into 2000x1200 → 2000x12
        let fine = select_mip_level(40000, 256, fit_render_w, 12);
        assert!(coarse >= fine);
    }

    // ---- downscale_box (#82's overview source) ----

    /// One opaque pixel of `v` at (x, y) on a black, opaque `w x h` BGRA
    /// canvas — the probe shape the block-partition assertions read back.
    fn canvas(w: usize, h: usize, spot: Option<(usize, usize, u8)>) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for px in v.chunks_mut(4) {
            px[3] = 255;
        }
        if let Some((x, y, val)) = spot {
            let i = (y * w + x) * 4;
            v[i] = val;
            v[i + 1] = val;
            v[i + 2] = val;
        }
        v
    }

    #[test]
    fn box_downscale_averages_each_two_by_two_block_of_level_one() {
        // 4x4 -> level 1 (2x2): the single lit pixel is averaged over its
        // own 2x2 block only (255/4 = 63) — the block boundaries partition
        // the source, so no neighbour block sees any of it.
        let src = canvas(4, 4, Some((0, 0, 255)));
        let out = downscale_box(&src, 4, 4, 1);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(&out[0..3], &[63, 63, 63]);
        assert_eq!(&out[4..7], &[0, 0, 0]);
        assert_eq!(&out[8..11], &[0, 0, 0]);
        assert_eq!(&out[12..15], &[0, 0, 0]);
        // Diagonal blocks are untouched: the lit pixel's block is the top
        // LEFT one only.
        let src = canvas(4, 4, Some((3, 3, 200)));
        let out = downscale_box(&src, 4, 4, 1);
        assert_eq!(&out[12..15], &[50, 50, 50]);
        assert_eq!(&out[0..3], &[0, 0, 0]);
    }

    #[test]
    fn box_downscale_leaves_a_uniform_opaque_source_uniform() {
        // The composite invariant (alpha 255 everywhere) survives every
        // level: a flat grey stays that grey at every depth, alpha and all.
        let src = canvas(8, 8, None);
        let mut src = src;
        for px in src.chunks_mut(4) {
            px[0] = 40;
            px[1] = 40;
            px[2] = 40;
        }
        for level in 0..=3 {
            let out = downscale_box(&src, 8, 8, level);
            assert!(!out.is_empty());
            for px in out.chunks(4) {
                assert_eq!(px, [40, 40, 40, 255], "level {level} drifted");
            }
        }
    }

    #[test]
    fn box_downscale_covers_the_odd_tail_dimension() {
        // 5x1 -> level 1 is (5+1)>>1 = 3 wide: blocks are [0,1), [1,3),
        // [3,5) — the tail block is wider, never dropped. The lit pixel at
        // x=4 lands in the LAST block (2 px -> 255/2 = 127), x=2's block
        // spans 1..3.
        let mut src = canvas(5, 1, None);
        src[4 * 4] = 250;
        src[4 * 4 + 1] = 250;
        src[4 * 4 + 2] = 250;
        let out = downscale_box(&src, 5, 1, 1);
        assert_eq!(out.len(), 3 * 4);
        assert_eq!(&out[8..11], &[125, 125, 125]);
        assert_eq!(&out[0..3], &[0, 0, 0]);
        assert_eq!(&out[4..7], &[0, 0, 0]);
    }

    #[test]
    fn box_downscale_at_level_zero_is_the_source_itself() {
        // Level 0's dimensions are the image's, so every block is one pixel
        // — the identity the #82 level picker relies on for the plain path.
        let src = canvas(3, 2, Some((1, 1, 77)));
        assert_eq!(downscale_box(&src, 3, 2, 0), src);
    }

    #[test]
    fn box_downscale_of_a_deep_level_is_one_source_pass() {
        // 40000x256 -> level 4 (2500x16) with ONE lit source pixel: its
        // block is 16x16 = 256 pixels, so the average is 255/256 = 0 — a
        // single bright pixel is diluted by its own block, which is what a
        // box prefilter is for (and the reason a deep shrink needs no
        // aliasing stripes).
        let (w, h) = (40000usize, 256usize);
        let mut src = canvas(w, h, None);
        src[0] = 255;
        src[1] = 255;
        src[2] = 255;
        let out = downscale_box(&src, w as i32, h as i32, 4);
        assert_eq!(out.len(), 2500 * 16 * 4);
        assert_eq!(&out[0..3], &[0, 0, 0]);
        // The SAME source at level 0 is the identity — the lit pixel is
        // still exactly 255 there (levels are independent reads of the
        // original, never a halving of a previous level).
        let identity = downscale_box(&src, w as i32, h as i32, 0);
        assert_eq!(&identity[0..3], &[255, 255, 255]);
    }

    // ---- LevelCache (#82's overview/level source) ----

    /// One 4x4 opaque mid-grey canvas whose top-left pixel is white — the
    /// fixture the cache's level identity is read back from.
    fn level_fixture() -> Vec<u8> {
        let mut v = vec![40u8; 4 * 4 * 4];
        for px in v.chunks_mut(4) {
            px[3] = 255;
        }
        v[0] = 255;
        v[1] = 255;
        v[2] = 255;
        v
    }

    #[test]
    fn the_level_cache_builds_once_and_serves_the_same_bytes_afterwards() {
        let src = level_fixture();
        let mut cache = LevelCache::new(LEVEL_CACHE_BYTES);
        let first = cache.get_or_build(1, 4, 4, 7, &src).expect("level 1");
        let (w, h) = (first.0, first.1);
        let first_bytes = first.2.to_vec();
        assert_eq!((w, h), (2, 2), "level 1 of 4x4 is 2x2");
        assert_eq!(cache.builds, 1, "the first ask builds");
        let again = cache.get_or_build(1, 4, 4, 7, &src).expect("level 1 again");
        assert_eq!(
            again.2,
            &first_bytes[..],
            "the second ask serves the cached bytes"
        );
        assert_eq!(cache.builds, 1, "and does not rebuild");
        assert_eq!(cache.bytes(), 2 * 2 * 4);
    }

    #[test]
    fn a_new_frame_generation_clears_the_cached_levels() {
        // The levels are pixels of ONE frame: a generation change must drop
        // them rather than serve the previous image's bytes.
        let src = level_fixture();
        let mut cache = LevelCache::new(LEVEL_CACHE_BYTES);
        cache.get_or_build(1, 4, 4, 7, &src).expect("level 1");
        assert!(cache.bytes() > 0);
        let rebuilt = cache
            .get_or_build(1, 4, 4, 8, &src)
            .expect("level 1 of the new frame");
        assert_eq!(rebuilt.0, 2);
        assert_eq!(cache.builds, 1, "the counter restarts with the frame");
        assert_eq!(cache.bytes(), 2 * 2 * 4, "one level, not two");
    }

    #[test]
    fn a_level_larger_than_the_whole_cap_is_refused_not_truncated() {
        // The refusal the plan ladder reads (`plan_frame`'s level budget):
        // a level that cannot be held must return None, never a partial
        // bitmap — the caller deepens instead of drawing garbage.
        let src = level_fixture();
        let mut cache = LevelCache::new(4);
        assert!(cache.get_or_build(1, 4, 4, 7, &src).is_none());
        assert_eq!(cache.builds, 0, "a refused level is not built");
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn the_level_cache_evicts_the_coldest_level_and_rebuilds_it_on_demand() {
        // The cap holds a 2x2 level (16 B) and a 4x4 level (64 B) exactly;
        // admitting a 1x1 level must push the COLDEST out, and the pushed
        // level must come back as a rebuild (never as stale bytes) when it
        // is asked for again.
        let src = level_fixture();
        let mut cache = LevelCache::new(80);
        cache.get_or_build(1, 4, 4, 7, &src).expect("level 1");
        cache.get_or_build(0, 4, 4, 7, &src).expect("level 0");
        assert_eq!(cache.bytes(), 80, "16 + 64");
        cache.get_or_build(1, 4, 4, 7, &src).expect("level 1 hit");
        assert_eq!(cache.builds, 2, "the re-ask was a hit");
        let small = cache
            .get_or_build(2, 4, 4, 7, &src)
            .expect("level 2 admits");
        assert_eq!((small.0, small.1), (1, 1));
        assert_eq!(cache.builds, 3);
        assert_eq!(cache.bytes(), 20, "level 0 (coldest) was evicted");
        let hit = cache.get_or_build(1, 4, 4, 7, &src).expect("level 1 hit");
        assert_eq!(hit.0, 2);
        assert_eq!(cache.builds, 3, "level 1 survived the eviction");
        cache
            .get_or_build(0, 4, 4, 7, &src)
            .expect("level 0 rebuild");
        assert_eq!(cache.builds, 4, "the evicted level rebuilds on demand");
    }
}
