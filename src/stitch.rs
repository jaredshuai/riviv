//! Stitched StretchBlt decomposition for ≥32768-px surfaces (issue #9's
//! port of upstream `_viv_StretchBltStitch`, viv.c:14929-15018).
//!
//! GDI's StretchBlt is documented-empirically unreliable when any of the
//! four extents (dest w/h, src w/h) reaches 32768 — the classic "black
//! image" for huge panoramas (voidtools/voidImageViewer#45). The fix is to
//! cut the source into 512-px tiles (`_VIV_STRETCH_BLT_STITCH_SIZE`,
//! viv.c:288 — sized so `512 * 3.1 (pan+zoom) * 16 (max zoom)` stays under
//! 32768, per the constraint comment there) and stretch each tile with its
//! own plain StretchBlt. Tile dst edges come from the SAME i64 projection
//! evaluated at shared source coordinates, so adjacent tiles partition the
//! dest rect exactly — no gaps, no overlap (only per-tile resampler phase,
//! which upstream accepts; viv.c:14233-14235).
//!
//! This module is the pure math; the GDI shell (mip generation in
//! `surface.rs`) consumes it. Paint never needs it: after mips, shrink
//! sources stay under the limit except through upstream's no-mip quirk
//! cases, where the HALFTONE path must NOT be cut (filter alignment,
//! viv.c:4253-4257) and uses a clip region instead (viv.c:4264-4283), and
//! magnify is viewport-clipped before blitting (#7's `clip_blit`).

use crate::zoom::BlitRect;

/// Source tile edge in px (upstream `_VIV_STRETCH_BLT_STITCH_SIZE`,
/// viv.c:288). Must satisfy `TILE * 3.1 * 16 < 32768` so one tile's dest
/// extent can never hit the StretchBlt limit under max zoom + pan.
pub(crate) const STITCH_TILE_SIZE: i32 = 512;

/// The StretchBlt extent limit that forces the tiled path (upstream's
/// `< 32768` fast-path test, viv.c:14933).
pub(crate) const STRETCH_EXTENT_LIMIT: i32 = 32768;

/// Decompose `blit` into source-tiled pieces whose dst rects tile the
/// original dst rect exactly, culling everything outside `clip`
/// (an `(x, y, w, h)` viewport rect).
///
/// Faithful to viv.c:14933-15014 including its asymmetries: the row guard
/// is `dst_y2 >= clip_y` (INCLUSIVE — a row whose bottom edge sits exactly
/// on the clip top is still emitted, viv.c:14966) while the column guard
/// is `dst_x2 > clip_x` (strict, viv.c:14996); rows above the clip only
/// skip (the guard), rows below break (viv.c:14957) — same for columns
/// (viv.c:14991). With the full-rect clip used for mip generation both
/// guards are trivially true, so the quirks are inert there.
///
/// Zero-area tiles (a projection so steep that consecutive tiles share a
/// dst edge) are dropped instead of emitted: upstream would hand GDI a
/// zero-extent StretchBlt and abort the whole chain on its failure
/// (viv.c:14999-15002 returns FALSE) — a latent upstream defect this port
/// deliberately does not carry (review finding, PR #18).
pub(crate) fn stitch_tiles(blit: BlitRect, clip: (i32, i32, i32, i32)) -> Vec<BlitRect> {
    let BlitRect {
        dx,
        dy,
        dw: w_dest,
        dh: h_dest,
        sx,
        sy,
        sw: w_src,
        sh: h_src,
    } = blit;
    if w_dest < STRETCH_EXTENT_LIMIT
        && h_dest < STRETCH_EXTENT_LIMIT
        && w_src < STRETCH_EXTENT_LIMIT
        && h_src < STRETCH_EXTENT_LIMIT
    {
        // Fast path: plain StretchBlt handles it (viv.c:14933-14936) —
        // zero-extent degenerate blits yield nothing instead.
        return if w_dest > 0 && h_dest > 0 && w_src > 0 && h_src > 0 {
            vec![blit]
        } else {
            Vec::new()
        };
    }
    if w_dest <= 0 || h_dest <= 0 || w_src <= 0 || h_src <= 0 {
        // Upstream's `(wDest>0)&&…` gate: nothing to stretch (viv.c:14938).
        return Vec::new();
    }
    let (clip_x, clip_y, clip_w, clip_h) = clip;
    let clip_right = clip_x + clip_w;
    let clip_bottom = clip_y + clip_h;
    let mut tiles = Vec::new();
    let mut dst_y = dy;
    let mut src_y = sy;
    let mut src_yrun = h_src;
    while src_yrun > 0 {
        if dst_y >= clip_bottom {
            break;
        }
        let src_high = src_yrun.min(STITCH_TILE_SIZE);
        let dst_y2 = (i64::from(src_y + src_high) * i64::from(h_dest) / i64::from(h_src)
            + i64::from(dy)) as i32;
        if dst_y2 >= clip_y {
            let mut dst_x = dx;
            let dst_high = dst_y2 - dst_y;
            let mut src_x = sx;
            let mut src_xrun = w_src;
            while src_xrun > 0 {
                let src_wide = src_xrun.min(STITCH_TILE_SIZE);
                let dst_x2 = (i64::from(src_x + src_wide) * i64::from(w_dest) / i64::from(w_src)
                    + i64::from(dx)) as i32;
                let dst_wide = dst_x2 - dst_x;
                if dst_x >= clip_right {
                    break;
                }
                if dst_x2 > clip_x && dst_wide > 0 && dst_high > 0 {
                    tiles.push(BlitRect {
                        dx: dst_x,
                        dy: dst_y,
                        dw: dst_wide,
                        dh: dst_high,
                        sx: src_x,
                        sy: src_y,
                        sw: src_wide,
                        sh: src_high,
                    });
                }
                dst_x = dst_x2;
                src_x += src_wide;
                src_xrun -= src_wide;
            }
        }
        dst_y = dst_y2;
        src_y += src_high;
        src_yrun -= src_high;
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blit(dw: i32, dh: i32, sw: i32, sh: i32) -> BlitRect {
        BlitRect {
            dx: 0,
            dy: 0,
            dw,
            dh,
            sx: 0,
            sy: 0,
            sw,
            sh,
        }
    }

    #[test]
    fn small_blits_take_the_single_tile_fast_path() {
        // Everything < 32768: one tile, verbatim (viv.c:14933-14936).
        let tiles = stitch_tiles(blit(100, 50, 200, 100), (0, 0, 100, 50));
        assert_eq!(tiles, vec![blit(100, 50, 200, 100)]);
    }

    #[test]
    fn one_extent_over_the_limit_forces_tiling() {
        // sw = 40000 crosses 32768 even though the rest is small — the
        // panorama case the whole module exists for.
        let tiles = stitch_tiles(blit(2000, 16, 40000, 256), (0, 0, 2000, 16));
        assert_eq!(tiles.len(), 40000usize.div_ceil(512));
        // Each tile maps its 512-px source run onto a proportional dst run.
        assert_eq!(tiles[0].sx, 0);
        assert_eq!(tiles[0].sw, 512);
        assert_eq!(tiles.last().unwrap().sx, 78 * 512);
        assert_eq!(tiles.last().unwrap().sw, 40000 - 78 * 512);
    }

    #[test]
    fn tile_dst_edges_partition_the_dest_exactly() {
        // The seam property: adjacent tiles share dst edges (the same i64
        // projection at shared source coordinates), so the dst rect is
        // partitioned — no gaps, no overlap (viv.c:14987/15005).
        let tiles = stitch_tiles(blit(16384, 128, 32768, 256), (0, 0, 16384, 128));
        let mut prev_right = 0;
        for t in &tiles {
            assert_eq!(t.dx, prev_right, "gap/overlap at {}", prev_right);
            prev_right = t.dx + t.dw;
        }
        assert_eq!(prev_right, 16384);
    }

    #[test]
    fn rows_and_columns_below_the_clip_break_out() {
        // Culling below/right: once a tile starts past the clip edge the
        // loops stop (viv.c:14957/14991) — nothing is emitted beyond it.
        let tiles = stitch_tiles(blit(40000, 100, 40000, 100), (0, 0, 5000, 40));
        assert!(tiles.iter().all(|t| t.dx < 5000 && t.dy < 40));
        let last = tiles.last().unwrap();
        // The last emitted tile starts inside the clip and its run crosses
        // the edge (GDI's DC clip bounds the actual writes, like upstream).
        assert!(last.dx < 5000 && last.dx + last.dw > 5000);
    }

    #[test]
    fn rows_above_the_clip_are_skipped_until_one_reaches_it() {
        // Two 512-px source rows → dst rows [0,64) and [64,100); with the
        // clip starting at y=70 the first row is guarded out (viv.c:14966)
        // while the loop keeps scanning and emits the second.
        let tiles = stitch_tiles(blit(40000, 100, 40000, 800), (0, 70, 5000, 30));
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.dy == 64));
        assert!(tiles.iter().all(|t| t.dy + t.dh == 100));
    }

    #[test]
    fn a_row_bottom_edge_exactly_on_the_clip_top_is_emitted() {
        // Upstream's inclusive row guard (>=, viv.c:14966): a row whose
        // dst bottom edge equals clip_y is still produced even though it
        // paints nothing visible. One row [0,4), clip starting at y=4.
        let tiles = stitch_tiles(blit(40000, 4, 40000, 4), (0, 4, 40000, 10));
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|t| t.dy + t.dh == 4));
        // One px lower and the row is gone entirely.
        let tiles = stitch_tiles(blit(40000, 4, 40000, 4), (0, 5, 40000, 10));
        assert!(tiles.is_empty());
    }

    #[test]
    fn degenerate_extents_yield_no_tiles() {
        // Zero dims: nothing (upstream's positivity gate, viv.c:14938) —
        // on both the fast and the tiled path.
        assert!(stitch_tiles(blit(0, 100, 40000, 100), (0, 0, 99999, 99999)).is_empty());
        assert!(stitch_tiles(blit(100, 0, 40000, 100), (0, 0, 99999, 99999)).is_empty());
        assert!(stitch_tiles(blit(100, 100, 0, 100), (0, 0, 99999, 99999)).is_empty());
        assert!(stitch_tiles(blit(0, 0, 0, 0), (0, 0, 99999, 99999)).is_empty());
    }

    #[test]
    fn non_zero_offsets_and_partial_last_blocks_map_back_exactly() {
        // A dest offset (the blit sits at dx=1000) and a source that is not
        // a multiple of 512: the projection keeps the pieces seamless.
        let whole = BlitRect {
            dx: 1000,
            dy: 7,
            dw: 30000,
            dh: 60,
            sx: 0,
            sy: 0,
            sw: 40000,
            sh: 80,
        };
        let tiles = stitch_tiles(whole, (0, 0, 40000, 2000));
        assert!(tiles.len() > 1);
        let mut prev_right = 1000;
        for t in &tiles {
            assert_eq!(t.dx, prev_right);
            prev_right = t.dx + t.dw;
        }
        assert_eq!(prev_right, 1000 + 30000);
        // Source runs also partition the source width.
        let mut prev_src_right = 0;
        for t in &tiles {
            assert_eq!(t.sx, prev_src_right);
            prev_src_right = t.sx + t.sw;
        }
        assert_eq!(prev_src_right, 40000);
    }

    #[test]
    fn the_boundary_at_32767_stays_on_the_fast_path() {
        // 32767 everywhere: still a single tile (the limit is strict <).
        let tiles = stitch_tiles(blit(32767, 32767, 32767, 32767), (0, 0, 32767, 32767));
        assert_eq!(tiles.len(), 1);
    }
}
