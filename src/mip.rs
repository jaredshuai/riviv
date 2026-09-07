//! Mipmap level selection (issue #9's port of upstream `_viv_get_mipmap`'s
//! size math, viv.c:14146-14300).
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
//! 14287, identical at both sites). We keep it byte-for-byte because it
//! changes which level paints (a wide-short image in a tall viewport picks
//! a shallower level than the "correct" compare would), and parity beats
//! correctness here (see the counterexample tests below).
//!
//! Invariants that downstream paint relies on (provable from the loop, and
//! pinned by tests): a returned level > 0 always satisfies `rw < mw` (the
//! mag arm never sees a mip); `rh < mh` is NOT guaranteed — the quirk can
//! hand the shrink arm a level far shorter than the render height, and the
//! shrink arm then vertically magnifies that mip (upstream does the same).

/// Size of mipmap `level` for an `image_wide x image_high` frame. Level 0
/// is the frame itself. Each dimension rounds `(dim+1)/2^k` down and clamps
/// at 1 (viv.c:14158-14169/14264-14275).
pub(crate) fn mip_size(image_w: i32, image_h: i32, level: u32) -> (i32, i32) {
    if level == 0 {
        return (image_w.max(1), image_h.max(1));
    }
    let w = ((image_w + 1) >> level.min(31)).max(1);
    let h = ((image_h + 1) >> level.min(31)).max(1);
    (w, h)
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
    fn pregeneration_depth_matches_what_paint_selects_for_the_same_view() {
        // The worker pre-generates with (viewport/2) — for the same
        // window, paint's fit render (≈ viewport) always fits within what
        // that depth cached, so resize-grow needs nothing new. A
        // regression here would strand paint generating on the UI thread.
        let (vw, vh) = (2000i32, 1200i32);
        let pregen = select_mip_level(40000, 256, vw / 2, vh / 2);
        let fit_render_w = vw; // 40000x256 fit into 2000x1200 → 2000x12
        let paint = select_mip_level(40000, 256, fit_render_w, 12);
        assert!(pregen >= paint);
    }
}
