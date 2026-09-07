//! WM_PAINT rendering: draw the current frame's view (zoom level + pan
//! offset) from `zoom::View`'s render-size math, THEN fill the letterbox
//! strips around it — blit first, background second, the image never
//! covered (upstream order, viv.c:4164+ blit → 4396-4407 fill).
//!
//! The blit path follows upstream's paint (viv.c:4133-4236): destination
//! size == source size → BitBlt (the pixel-exact 1:1 path); shrinking →
//! HALFTONE + brush-org realignment; magnifying → COLORONCOLOR (the default
//! `config_mag_filter`). Alpha compositing (#3) is landed — transparent
//! pixels are resolved against the windowed background at decode time, so
//! this path blits opaque pixels only.
//!
//! #9: the source is the mipmap level selected for the render size
//! (upstream `_viv_get_mipmap` at viv.c:4167 — the chain extends lazily,
//! then all three arms measure against the LEVEL's size, not the
//! original's). The ≥32768 stitch itself lives in mip generation
//! (`surface.rs` via `stitch.rs`); paint only meets giant extents on the
//! HALFTONE shrink path, where upstream keeps ONE full-rect StretchBlt
//! behind a simple clip region (viv.c:4264-4283) — cutting the rect would
//! realign the filter taps (viv.c:4253-4257).

use windows::Win32::Foundation::{COLORREF, GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, COLORONCOLOR, CreateRectRgn, CreateSolidBrush, DeleteObject, ERROR,
    EndPaint, FillRect, HALFTONE, HGDIOBJ, PAINTSTRUCT, SRCCOPY, SelectClipRgn, SetBrushOrgEx,
    SetStretchBltMode, StretchBlt,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::mip;
use crate::pixels::WINDOWED_BACKGROUND_RGB;
use crate::stitch::STRETCH_EXTENT_LIMIT;
use crate::window::{fatal, state_of};
use crate::zoom::Viewport;

pub(crate) fn paint(hwnd: HWND) {
    // SAFETY: all GDI calls are bracketed by BeginPaint/EndPaint on the WM_PAINT
    // DC; handles are valid for the duration of the message.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            // Already inside this function's outer unsafe block.
            let gle = GetLastError().0;
            // System-level failure (ADR 0001): painting with a null DC would
            // silently produce a blank client.
            fatal(&format!("BeginPaint failed (GLE={gle})"));
        }
        let mut client = RECT::default();
        // Fail-soft on purpose: upstream viv.c:4070/4098 also ignores
        // GetClientRect's return and reads the (zeroed) rect — a failed query
        // yields a degenerate paint, not a dead window (ADR 0001 leaves
        // paint-path diagnostics to the debug-log channel landing in M2).
        let _ = GetClientRect(hwnd, &mut client);
        // The render area excludes the status bar (#5) — the image fits
        // and centers above it (upstream subtracts `_viv_get_status_high()`
        // from the client height at paint time, viv.c:4072).
        let status_h = state_of(hwnd).map(|s| crate::status::height(s.status));
        if let Some(h) = status_h {
            client.bottom = (client.bottom - h).max(client.top);
        }
        let cw = (client.right - client.left).max(1);
        let ch = (client.bottom - client.top).max(1);
        // The image blit runs BEFORE the background fill, and the fill
        // excludes the image rect (upstream paints this way: blit at
        // viv.c:4164+, then `os_fill_clipped_rect` for the letterbox,
        // viv.c:4396-4407 + os.c:1502-1522). Filling the whole client first
        // and blitting over it flashes the background at fullscreen sizes —
        // a drag's per-mousemove repaint then strobes white between the
        // fill and a >1-vsync StretchBlt (user QA on #8, 2026-09-07).
        // SAFETY (for state_of, nested in this fn's outer unsafe block): the
        // borrow lives only across the GDI draw calls below — none pump
        // messages, so no second `state_of` borrow can be taken while this
        // one is live. The mip chain extension inside (ensure_mips) is
        // degrade-not-fatal by design for the same reason: the fatal modal
        // pumps messages and would alias this borrow (PR #10 P1).
        let mut img = (0, 0, 0, 0); // degenerate: the strips cover everything
        if let Some(state) = state_of(hwnd)
            && let Some(image) = state.image.as_mut()
        {
            let surface = image.surface_mut();
            let (sw, sh) = (surface.width(), surface.height());
            // The zoom/pan view decides the destination rect (upstream
            // `_viv_get_render_size` + `rx = wide/2 - rw/2 - _viv_view_x`,
            // viv.c:4136-4149); GDI clips whatever pans off-window.
            let (rw, rh) = state
                .view
                .render_size(sw, sh, Viewport { wide: cw, high: ch });
            let dx = client.left + cw / 2 - rw / 2 - state.view.view_x;
            let dy = client.top + ch / 2 - rh / 2 - state.view.view_y;
            img = (dx, dy, rw, rh);
            if rw > 0 && rh > 0 {
                // Mip selection by render size (upstream viv.c:4167): extend
                // the frame's chain to the selected level on demand, then
                // paint from the level that is actually usable — a truncated
                // chain degrades to a shallower source, never a blank image.
                let target = mip::select_mip_level(sw, sh, rw, rh);
                let level = surface.ensure_mips(sw, sh, target);
                surface.with_mip_source(level, |src_dc, mw, mh| {
                    if rw == mw && rh == mh {
                        // Pixel-exact 1:1 — BitBlt, no resampling (upstream's
                        // equal-size arm, viv.c:4164-4173; plain BitBlt even
                        // for huge extents, GDI clips).
                        let _ = BitBlt(hdc, dx, dy, rw, rh, Some(src_dc), 0, 0, SRCCOPY);
                    } else if rw < mw || rh < mh {
                        // Upstream shrink path: HALFTONE + brush-org
                        // realignment anchored to the destination image
                        // (viv.c:4205-4209 uses -rx,-ry) so the dither
                        // pattern does not drift as the image moves.
                        let _ = SetStretchBltMode(hdc, HALFTONE);
                        let _ = SetBrushOrgEx(hdc, -dx, -dy, None);
                        if mw >= STRETCH_EXTENT_LIMIT
                            || mh >= STRETCH_EXTENT_LIMIT
                            || rw >= STRETCH_EXTENT_LIMIT
                            || rh >= STRETCH_EXTENT_LIMIT
                        {
                            // Giant extents (a mid-zoom shrink whose selected
                            // level is still ≥32768): one full-rect StretchBlt
                            // behind a simple clip region — upstream's
                            // halftone pattern (viv.c:4264-4283). The rect
                            // must NOT be cut (filter alignment), and GDI
                            // walks a complex clip region slowly, so the DC
                            // gets a single-rect region instead: the update
                            // paint rect intersected with the viewport.
                            let l = ps.rcPaint.left.max(client.left);
                            let t = ps.rcPaint.top.max(client.top);
                            let r = ps.rcPaint.right.min(client.right);
                            let b = ps.rcPaint.bottom.min(client.bottom);
                            if r > l && b > t {
                                let clip_rgn = CreateRectRgn(l, t, r, b);
                                if !clip_rgn.is_invalid() {
                                    // Upstream blits only when the clip took
                                    // (SelectClipRgn != ERROR, viv.c:4271) and
                                    // always restores the region afterwards
                                    // (viv.c:4361) — the reset below runs on
                                    // every path out of this block so the
                                    // letterbox fill is never clipped away.
                                    if SelectClipRgn(hdc, Some(clip_rgn)).0 != ERROR {
                                        let _ = StretchBlt(
                                            hdc,
                                            dx,
                                            dy,
                                            rw,
                                            rh,
                                            Some(src_dc),
                                            0,
                                            0,
                                            mw,
                                            mh,
                                            SRCCOPY,
                                        );
                                    }
                                    SelectClipRgn(hdc, None);
                                    let _ = DeleteObject(HGDIOBJ(clip_rgn.0));
                                }
                            }
                        } else {
                            // Fail-soft by design: upstream viv.c:4278 also
                            // continues past a failed StretchBlt (debug_printf
                            // only) — one bad frame must not kill the window.
                            // The full rect is stretched on purpose: GDI
                            // honors the DC clip for shrinks (viv.c:4056-4062),
                            // and cutting the rect would realign the HALFTONE
                            // filter taps.
                            let _ = StretchBlt(
                                hdc,
                                dx,
                                dy,
                                rw,
                                rh,
                                Some(src_dc),
                                0,
                                0,
                                mw,
                                mh,
                                SRCCOPY,
                            );
                        }
                    } else {
                        // Upstream magnify default: COLORONCOLOR
                        // (`config_mag_filter`, config.c:42), clipped to the
                        // viewport with the cut mapped back to source coords —
                        // GDI walks the whole dest extent of a StretchBlt no
                        // matter the clip region (viv.c:4056-4062), so an
                        // unclipped 16x blit stretches a rect tens of
                        // thousands of pixels wide on every paint (upstream's
                        // tiled stretch exists for exactly this,
                        // viv.c:14929-14936). After the cut both extents stay
                        // viewport-bounded, so no stitching here.
                        let _ = SetStretchBltMode(hdc, COLORONCOLOR);
                        let whole = crate::zoom::BlitRect {
                            dx,
                            dy,
                            dw: rw,
                            dh: rh,
                            sx: 0,
                            sy: 0,
                            sw: mw,
                            sh: mh,
                        };
                        if let Some(b) = crate::zoom::clip_blit(
                            whole,
                            client.left,
                            client.top,
                            Viewport { wide: cw, high: ch },
                        ) {
                            // Fail-soft like the shrink path (viv.c:4278).
                            let _ = StretchBlt(
                                hdc,
                                b.dx,
                                b.dy,
                                b.dw,
                                b.dh,
                                Some(src_dc),
                                b.sx,
                                b.sy,
                                b.sw,
                                b.sh,
                                SRCCOPY,
                            );
                        }
                    }
                });
            }
        }
        // The letterbox fill, AFTER the blit and excluding its rect —
        // upstream viv.c:4396-4407 + os_fill_clipped_rect (os.c:1502-1522).
        // Background color: the same constant the decode path composites
        // transparent pixels against — the two must never diverge or
        // composited images show a fringe (riviv keeps the windowed color
        // in fullscreen too; README deviation).
        let [bg_r, bg_g, bg_b] = WINDOWED_BACKGROUND_RGB;
        // Already inside this function's outer unsafe block.
        let brush = CreateSolidBrush(COLORREF(
            (u32::from(bg_b) << 16) | (u32::from(bg_g) << 8) | u32::from(bg_r),
        ));
        if !brush.is_invalid() {
            for strip in letterbox_strips((client.left, client.top, cw, ch), img) {
                let _ = FillRect(hdc, &strip, brush);
            }
            // Already inside this function's outer unsafe block; the brush
            // was created above and FillRect does not retain it, so plain
            // DeleteObject is the documented teardown.
            let _ = DeleteObject(HGDIOBJ(brush.0));
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// The four background strips around the image rect, clamped into the
/// viewport (upstream `os_fill_clipped_rect` + `os_fill_clamped_rect`,
/// os.c:1472-1527): top / left / right / bottom, each intersected with the
/// viewport and skipping anything ≤0-wide/high. A degenerate image rect
/// (blank display) yields the whole viewport via the bottom strip, exactly
/// like upstream's rx=ry=rw=rh=0 case; an image covering the viewport
/// yields nothing.
fn letterbox_strips(
    viewport: (i32, i32, i32, i32),
    image: (i32, i32, i32, i32),
) -> impl Iterator<Item = RECT> {
    let (vx, vy, vw, vh) = viewport;
    let (vr, vb) = (vx + vw, vy + vh);
    let (ix, iy, iw, ih) = image;
    let (ir, ib) = (ix + iw, iy + ih);
    [
        (vx, vy, vr, iy), // top
        (vx, iy, ix, ib), // left
        (ir, iy, vr, ib), // right
        (vx, ib, vr, vb), // bottom
    ]
    .into_iter()
    .filter_map(move |(mut l, mut t, mut r, mut b)| {
        l = l.max(vx);
        t = t.max(vy);
        r = r.min(vr);
        b = b.min(vb);
        (r > l && b > t).then_some(RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_display_fills_the_whole_viewport() {
        // The degenerate image rect (upstream's rx=ry=rw=rh=0): the bottom
        // strip degenerates into the entire viewport (os.c:1502-1522).
        let strips: Vec<RECT> = letterbox_strips((0, 0, 2880, 1800), (0, 0, 0, 0)).collect();
        assert_eq!(
            strips,
            vec![RECT {
                left: 0,
                top: 0,
                right: 2880,
                bottom: 1800
            }]
        );
    }

    #[test]
    fn centered_image_yields_four_strips() {
        // 300x300 image centered in a 1000x700 viewport at (350,200).
        let strips: Vec<RECT> = letterbox_strips((0, 0, 1000, 700), (350, 200, 300, 300)).collect();
        assert_eq!(
            strips,
            vec![
                RECT {
                    left: 0,
                    top: 0,
                    right: 1000,
                    bottom: 200
                }, // top
                RECT {
                    left: 0,
                    top: 200,
                    right: 350,
                    bottom: 500
                }, // left
                RECT {
                    left: 650,
                    top: 200,
                    right: 1000,
                    bottom: 500
                }, // right
                RECT {
                    left: 0,
                    top: 500,
                    right: 1000,
                    bottom: 700
                }, // bottom
            ]
        );
    }

    #[test]
    fn image_covering_the_viewport_fills_nothing() {
        // Zoomed in past the edges: no letterbox, no overdraw over the
        // image (the strobe fix's core property).
        let strips: Vec<RECT> =
            letterbox_strips((0, 0, 800, 600), (-500, -500, 2000, 2000)).collect();
        assert!(strips.is_empty());
    }

    #[test]
    fn image_panned_off_left_keeps_only_the_right_strip() {
        // Half the image is off-viewport and it spans the viewport's full
        // height: left/top/bottom strips clamp away, the strip right of the
        // image stays.
        let strips: Vec<RECT> =
            letterbox_strips((0, 0, 800, 600), (-400, -100, 1000, 800)).collect();
        assert_eq!(
            strips,
            vec![RECT {
                left: 600,
                top: 0,
                right: 800,
                bottom: 600
            }]
        );
    }

    #[test]
    fn image_below_the_viewport_fills_the_top_only() {
        // The whole image is out of view below: only the top strip covers
        // the viewport (everything above the image's top edge).
        let strips: Vec<RECT> = letterbox_strips((0, 0, 400, 300), (100, 500, 200, 200)).collect();
        assert_eq!(
            strips,
            vec![RECT {
                left: 0,
                top: 0,
                right: 400,
                bottom: 300
            }]
        );
    }
}
