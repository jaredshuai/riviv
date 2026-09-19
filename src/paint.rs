//! WM_PAINT rendering for the `riviv_view` viewport child (#78): draw the
//! current frame's view (zoom level + pan offset) from `zoom::View`'s
//! render-size math, THEN fill the letterbox strips around it — blit first,
//! background second, the image never covered (upstream order, viv.c:4164+
//! blit → 4396-4407 fill).
//!
//! The paint runs on the viewport CHILD's DC: its client rect IS the render
//! viewport (the parent docks the chrome below it and sizes the child to
//! client-minus-chrome in `on_size`) — the old subtract-chrome arithmetic is
//! gone, the child rect is the single source of truth. The state still
//! lives on the owner (main) window; `paint` takes both HWNDs.
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
//!
//! #80 splits the scene from the WM_PAINT bracket: [`render_scene`] draws
//! blit + strips onto ANY DC — the GDI dump channel and the giant-frame
//! degrade pass reuse it verbatim, while the D2D arm (`gpu.rs`) consumes
//! the same [`scene_rect`] math against the master's full-size bitmap.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use windows::Win32::Foundation::{COLORREF, GetLastError, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, COLORONCOLOR, CreateCompatibleDC,
    CreateDIBSection, CreateRectRgn, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    ERROR, EndPaint, FillRect, GetDC, HALFTONE, HDC, HGDIOBJ, PAINTSTRUCT, ReleaseDC, SRCCOPY,
    SelectClipRgn, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::mip;

use crate::stitch::STRETCH_EXTENT_LIMIT;
use crate::window::{fatal, state_of};
use crate::zoom::{FitPolicy, View, Viewport};

pub(crate) fn paint(view: HWND, owner: HWND) {
    // SAFETY: all GDI calls are bracketed by BeginPaint/EndPaint on the
    // WM_PAINT DC of the viewport child; handles are valid for the duration
    // of the message. The state reads run against the owner window's slot —
    // the child stores nothing in GWLP_USERDATA.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(view, &mut ps);
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
        // The child's client rect IS the viewport (#78) — no chrome
        // subtraction: the parent sizes this window to client-minus-chrome
        // after docking the chrome children.
        let _ = GetClientRect(view, &mut client);
        // The scene body (blit + letterbox) extracted verbatim; the WM_PAINT
        // bracket, the #76 handshake and the EndPaint stay here.
        render_scene(hdc, owner, client, ps.rcPaint);
        // The animation first-frame paint handshake (#76): this paint has
        // rendered (or blanked) the adoption the decode worker is holding
        // frame 1 for — signal it before the handler returns. A fresh
        // short borrow: none of the draws above hold one. (Inside this
        // function's outer unsafe block already.)
        if let Some(signal) = state_of(owner).and_then(|s| s.paint_signal.take()) {
            let (lock, cvar) = &*signal;
            // unwrap on poisoning: a panicked UI thread would be dead
            // anyway; the worker's wait cap covers it regardless.
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
        let _ = EndPaint(view, &ps);
    }
}

/// The scene body of the GDI arm: gather the fit inputs and the
/// mode-resolved background, blit the current frame's view (mip-selected),
/// then fill the letterbox strips around it. Shared verbatim by the
/// WM_PAINT bracket (above), the giant-frame degrade pass
/// ([`paint_degraded`]) and the GDI dump channel ([`dump_viewport_gdi`]).
/// `paint_clip` is the update-rect source for the giant-extent clip region
/// (WM_PAINT passes ps.rcPaint; the off-paint channels pass the whole
/// client — nothing clips away either way). No fatal inside: the off-paint
/// callers run without a BeginPaint bracket, and a dump must fail with a
/// string, never a modal (ADR 0001's user-level channel).
pub(crate) fn render_scene(hdc: HDC, owner: HWND, client: RECT, paint_clip: RECT) {
    // SAFETY: GDI draws onto `hdc` only (the caller owns its lifetime);
    // the state borrows span the draw calls, none of which pump messages —
    // no second `state_of` borrow can be taken while one is live (PR #10
    // P1). The mip chain extension inside (ensure_mips) is
    // degrade-not-fatal by design for the same reason: the fatal modal
    // pumps messages and would alias this borrow.
    unsafe {
        let cw = (client.right - client.left).max(1);
        let ch = (client.bottom - client.top).max(1);
        // The image blit runs BEFORE the background fill, and the fill
        // excludes the image rect (upstream paints this way: blit at
        // viv.c:4164+, then `os_fill_clipped_rect` for the letterbox,
        // viv.c:4396-4407 + os.c:1502-1522). Filling the whole client first
        // and blitting over it flashes the background at fullscreen sizes —
        // a drag's per-mousemove repaint then strobes white between the
        // fill and a >1-vsync StretchBlt (user QA on #8, 2026-09-07).
        let mut img = (0, 0, 0, 0); // degenerate: the strips cover everything
        // The mode-resolved background (upstream viv.c:4396 picks by
        // `_viv_is_fullscreen`): the letterbox strips repaint live with the
        // config color. Transparent-pixel compositing is NOT re-run on a
        // color change (it flattens at decode against the windowed color,
        // #3) — the same stale composite upstream shows after an Options
        // color change (its own TODO list, viv.c:224).
        let mut bg = [255u8, 255, 255];
        // Gather the fit inputs and the background under one immutable
        // borrow FIRST — the image borrow below is mutable.
        // SAFETY: the borrow spans only the two Copy reads.
        let fit = state_of(owner)
            .map(|state| crate::window::fit_policy(state))
            .unwrap_or(FitPolicy::WITHOUT_FILL);
        if let Some(state) = state_of(owner) {
            bg = if state.fullscreen {
                state.config.fullscreen_bg()
            } else {
                state.config.windowed_bg()
            };
        }
        if let Some(state) = state_of(owner)
            && let Some(image) = state.image.as_mut()
        {
            let surface = image.surface_mut();
            let (sw, sh) = (surface.width(), surface.height());
            // The two blit filters (config.c:41-42; 0 = COLORONCOLOR
            // "Nearest", 1 = HALFTONE "Linear"): the shrink arm keys on
            // `shrink_blit_mode`, the magnify arm on `mag_filter`
            // (upstream viv.c:4205-4237 — brush-org realignment rides the
            // HALFTONE SHRINK only).
            let halftone_shrink = state.config.shrink_blit_mode == 1;
            let halftone_mag = state.config.mag_filter == 1;
            // The zoom/pan view decides the destination rect (upstream
            // `_viv_get_render_size` + the panscan layer, viv.c:4136-4154):
            // the preset-curve size scaled per axis by the panscan factors,
            // centered on the panscan position term (plain viewport center
            // at the default 500) minus the drag offset; GDI clips whatever
            // pans off-window. The math itself is [`scene_rect`] — the D2D
            // arm (#80) consumes the identical numbers.
            let (rdx, rdy, rw, rh) = scene_rect(&state.view, fit, cw, ch, sw, sh);
            let dx = client.left + rdx;
            let dy = client.top + rdy;
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
                        // Upstream shrink path (viv.c:4205-4214): HALFTONE +
                        // brush-org realignment anchored to the destination
                        // image (viv.c:4205-4209 uses -rx,-ry, so the dither
                        // pattern does not drift as the image moves) when
                        // `shrink_blit_mode` is Linear; Nearest is plain
                        // COLORONCOLOR with no brush org.
                        if halftone_shrink {
                            let _ = SetStretchBltMode(hdc, HALFTONE);
                            let _ = SetBrushOrgEx(hdc, -dx, -dy, None);
                        } else {
                            let _ = SetStretchBltMode(hdc, COLORONCOLOR);
                        }
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
                            let l = paint_clip.left.max(client.left);
                            let t = paint_clip.top.max(client.top);
                            let r = paint_clip.right.min(client.right);
                            let b = paint_clip.bottom.min(client.bottom);
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
                        // Magnify path (viv.c:4215-4227): COLORONCOLOR by
                        // default (`config_mag_filter`, config.c:42), clipped
                        // to the viewport with the cut mapped back to source
                        // coords — GDI walks the whole dest extent of a
                        // StretchBlt no matter the clip region
                        // (viv.c:4056-4062), so an unclipped 16x blit
                        // stretches a rect tens of thousands of pixels wide
                        // on every paint (upstream's tiled stretch exists for
                        // exactly this, viv.c:14929-14936). After the cut both
                        // extents stay viewport-bounded, so no stitching here.
                        // `mag_filter` Linear = HALFTONE magnified WITHOUT the
                        // clip cut: cutting realigns the filter taps, so the
                        // full rect stretches behind GDI's own clipping
                        // (upstream's is_halftone keeps simple full rects for
                        // the same reason, viv.c:4240-4250).
                        if halftone_mag {
                            let _ = SetStretchBltMode(hdc, HALFTONE);
                            // Fail-soft like the shrink path (viv.c:4278).
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
                        } else {
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
                    }
                });
            }
        }
        // The letterbox fill, AFTER the blit and excluding its rect —
        // upstream viv.c:4396-4407 + os_fill_clipped_rect (os.c:1502-1522),
        // in the mode-resolved config color gathered above.
        let [bg_r, bg_g, bg_b] = bg;
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
    }
}

/// The shared view math (upstream `_viv_get_render_size` + the panscan
/// layer, viv.c:4136-4154): the image rect in VIEWPORT-RELATIVE client
/// coordinates for a `sw x sh` source. Pure — the GDI blit and the D2D arm
/// (#80) consume the identical numbers; dx/dy/rw/rh stay exact i32s (the
/// 1:1 five-piece's integer-rect clause). The caller adds the client
/// origin, which is (0,0) for the viewport child.
pub(crate) fn scene_rect(
    view: &View,
    fit: FitPolicy,
    cw: i32,
    ch: i32,
    sw: i32,
    sh: i32,
) -> (i32, i32, i32, i32) {
    let (rw, rh) = view.render_size(sw, sh, Viewport { wide: cw, high: ch }, fit);
    let panscan = view.panscan;
    let rw = crate::panscan::scale(rw, panscan.zoom_x);
    let rh = crate::panscan::scale(rh, panscan.zoom_y);
    let dx = crate::panscan::center_term(cw, panscan.pos_x) - rw / 2 - view.view_x;
    let dy = crate::panscan::center_term(ch, panscan.pos_y) - rh / 2 - view.view_y;
    (dx, dy, rw, rh)
}

/// The off-WM_PAINT GDI frame (#80): the giant-frame gate's degrade pass —
/// the D2D arm began (and validated) the paint, so a second BeginPaint
/// would hand back an empty-clip DC; the scene draws through GetDC
/// instead. Fires the #76 paint handshake like `paint` does: this IS the
/// adoption render when the adoptee is a giant frame.
pub(crate) fn paint_degraded(view: HWND, owner: HWND) {
    // SAFETY: GetDC/ReleaseDC bracket the scene draw on the child's DC;
    // the handshake borrow runs after the draw with none live, and nothing
    // here pumps messages.
    unsafe {
        let hdc = GetDC(Some(view));
        if hdc.is_invalid() {
            // Already inside this function's outer unsafe block. Degrade
            // quietly with a breadcrumb — one lost frame must not fatal.
            let gle = GetLastError().0;
            eprintln!("riviv: degrade-frame GetDC failed (GLE={gle})");
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(view, &mut client);
        render_scene(hdc, owner, client, client);
        let _ = ReleaseDC(Some(view), hdc);
        if let Some(signal) = state_of(owner).and_then(|s| s.paint_signal.take()) {
            let (lock, cvar) = &*signal;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
    }
}

/// The GDI dump channel (#80 design §9): render the current scene into a
/// fresh 32bpp top-down DIB section on a memory DC and hand the pixels
/// back as RGBA (the PNG writer's input). Never touches the screen — the
/// flip-model GDI interop ban is per-HWND, and a memory DC never draws to
/// the one the swapchain owns. Errors are plain strings: the WM_CLOSE dump
/// path reports on stderr and exits 2 (the automation channel's loud, no
/// modal — ADR 0001's user-level tier).
pub(crate) fn dump_viewport_gdi(view: HWND, owner: HWND) -> Result<(u32, u32, Vec<u8>), String> {
    let mut client = RECT::default();
    // SAFETY: read-only rect query on our own child; a failed read leaves
    // the zeroed rect and the size check below fails the dump.
    let _ = unsafe { GetClientRect(view, &mut client) };
    let cw = client.right - client.left;
    let ch = client.bottom - client.top;
    if cw <= 0 || ch <= 0 {
        return Err(format!("viewport is {cw}x{ch} — nothing to dump"));
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: cw,
            biHeight: -ch, // negative = top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: `info` is a valid stack BITMAPINFO outliving the call; the
    // returned section is owned here (no file mapping, no palette with
    // BI_RGB).
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|e| format!("dump CreateDIBSection failed: {e}"))?;
    if bits.is_null() {
        // SAFETY: the section is owned and selected nowhere — plain
        // DeleteObject is the correct teardown.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        return Err("dump CreateDIBSection returned NULL bits".into());
    }
    // SAFETY: None gives a screen-compatible memory DC, owned on this
    // (UI) thread for the duration of the dump.
    let memdc = unsafe { CreateCompatibleDC(None) };
    if memdc.is_invalid() {
        // SAFETY: the section is owned and selected nowhere.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        return Err("dump CreateCompatibleDC failed".into());
    }
    // SAFETY: `bitmap` is a valid owned section; the stock handle returned
    // here is restored before teardown.
    let stock = unsafe { SelectObject(memdc, HGDIOBJ(bitmap.0)) };
    if stock.is_invalid() {
        // SAFETY: selection failed — the DC still holds its stock bitmap;
        // both objects are independently deletable.
        unsafe {
            let _ = DeleteDC(memdc);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        return Err("dump SelectObject failed".into());
    }
    // The scene renders into the section exactly as it would on screen.
    render_scene(memdc, owner, client, client);
    // Read the section's own memory: a DIB section's bits ARE the pixels
    // (the GetDIBits roundtrip of the design sketch is redundant for a
    // section we own — the bytes are identical).
    let len = cw as usize * ch as usize * 4;
    // SAFETY: `bits` points at exactly len readable bytes of the fresh
    // section (32bpp, top-down, tightly packed — the header above).
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), len) };
    let mut bgra = bgra.to_vec();
    // Force the alpha byte opaque (smoke80 S3b): GDI never writes a
    // destination alpha — FillRect leaves the section's zero-initialized
    // 0 in the letterbox while the blit copies the master's 255 — but the
    // dump channel's contract is the fully opaque viewport BOTH arms
    // render (the window has no transparency semantics), and the D2D arm
    // writes 255 everywhere (Clear's a=1.0). Without this the two stacks'
    // goldens differ in the letterbox alpha and byte comparison breaks.
    for px in bgra.chunks_mut(4) {
        px[3] = 255;
    }
    let mut rgba = vec![0u8; len];
    crate::pixels::bgra_to_rgba(&bgra, &mut rgba);
    // SAFETY: restore the stock bitmap so the section is deletable, then
    // tear the DC down (the documented GDI order, as in surface.rs's Face).
    unsafe {
        let _ = SelectObject(memdc, stock);
        let _ = DeleteDC(memdc);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    Ok((cw as u32, ch as u32, rgba))
}

/// Write RGBA pixels as a PNG (#80's dump tail; the image crate is already
/// a decode dependency, its png feature on). Failures are plain strings —
/// the WM_CLOSE dump path reports on stderr and exits 2 without a modal.
pub(crate) fn save_rgba_png(
    path: &Path,
    wide: u32,
    high: u32,
    rgba: Vec<u8>,
) -> Result<(), String> {
    let image = image::RgbaImage::from_raw(wide, high, rgba).ok_or("dump buffer size mismatch")?;
    image
        .save(path)
        .map_err(|e| format!("png save failed: {e}"))
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

    // ---- scene_rect (#80: the shared view math) ----

    fn one_to_one_view() -> View {
        // The pixel-exact mode (the D2D arm's L0 case): render_size returns
        // the source verbatim while `one_to_one` is on (zoom.rs's own
        // pinned behavior).
        let mut view = View::new();
        view.toggle_one_to_one(
            300,
            300,
            Viewport {
                wide: 1000,
                high: 700,
            },
            FitPolicy::WITHOUT_FILL,
        );
        view
    }

    #[test]
    fn scene_rect_centers_the_render_with_the_default_panscan() {
        // Default panscan (pos 500 = plain center, zoom factor 1.0) and a
        // 300x300 source in 1000x700: dx = 500 - 150 = 350, dy = 350 - 150
        // = 200 — the same rect the GDI strips test above pins, so both
        // arms blit into the same place.
        let view = one_to_one_view();
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (350, 200, 300, 300)
        );
    }

    #[test]
    fn scene_rect_subtracts_the_pan_offset() {
        // The drag offset moves the rect opposite to the drag, one-to-one.
        let mut view = one_to_one_view();
        view.view_x = 40;
        view.view_y = 15;
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (310, 185, 300, 300)
        );
    }

    #[test]
    fn scene_rect_follows_the_panscan_center_term() {
        // Pan position 750 (past the default 500 center) moves the center
        // term to (750-250)*2000/1000 = 1000, so the rect shifts right by
        // 500 — the same term both arms add (viv.c:4153).
        let mut view = one_to_one_view();
        view.panscan.pos_x = 750;
        // center_term(1000, 750) = (750-250)*2000/1000 = 1000.
        assert_eq!(
            scene_rect(&view, FitPolicy::WITHOUT_FILL, 1000, 700, 300, 300),
            (850, 200, 300, 300)
        );
    }
}
