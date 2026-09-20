//! Giant-image tiling (#82, ADR 0002 D7): the pure plan math behind the D2D
//! arm's overview + tile path.
//!
//! One frame's source is picked by [`detail_level`]: level 0 for anything
//! needing source-resolution pixels (1:1 and magnify — the five-piece
//! contract), the deepest mip level that still covers the render for a
//! shrink (so the draw is a no-magnification sample of a prefiltered
//! source). [`plan_frame`] then picks the cheapest form that can draw that
//! level: a single level bitmap when the whole level fits under the
//! device's `GetMaximumBitmapSize()` (level 0 = the #80/#81 plain path,
//! level ≥ 1 = the overview), otherwise haloed, interior-clipped tiles over
//! the visible region only.
//!
//! Three properties this module exists to guarantee, all pinned by tests:
//!
//! - **Exact partition.** Tile edges are the *same* i64 projection
//!   ([`src_to_dest`]) evaluated at *shared source* coordinates — the
//!   discipline `stitch.rs` established for the GDI arm's tiling. Adjacent
//!   tiles therefore share their dest and clip edges bit for bit: no gap,
//!   no overlap, no dropped or duplicated column (the ticket's 缝/重复列/
//!   丢列 clause).
//! - **Haloed, interior-clipped.** Each tile's bitmap carries
//!   [`FILTER_HALO_NATIVE`] source pixels of its neighbourhood so a filter tap may
//!   cross the block edge, and the draw is clipped to the tile's *logical*
//!   interior, so every destination pixel is written by exactly one tile
//!   and the halo's edge-clamped samples never reach the screen. Because
//!   every tile maps through the same global affine map, the resampler's
//!   phase is the untiled draw's phase.
//! - **Windowed work.** Only tiles intersecting the visible destination
//!   rect's preimage are ever requested — a 16777217-wide 1:1 giant must
//!   not enumerate 16384 columns to draw one screenful (its resident source
//!   is O(viewport), the ticket's core claim).
//!
//! Pure: no Win32 below, so the whole ladder is unit-testable. `gpu.rs`
//! owns the bitmaps, the LRU and the ledger; this module only decides.

use crate::mip;

/// Default source edge of one tile, in pixels *at the drawn level*: a
/// 1024² tile is 4 MiB of BGRA — small enough that the LRU's granularity
/// is fine (a viewport needs a handful) and large enough that the halo
/// overhead ([`FILTER_HALO_NATIVE`] x 2 per axis, ~3 % at
/// the default edge) stays cheap. The
/// ticket names 1024²/2048²; 1024 is the shipping default and
/// `-tile <edge>` overrides it for the smoke's forced-tiling comparisons.
pub(crate) const TILE_EDGE: i32 = 1024;

/// Filter-support margin carried by every tile bitmap, in source pixels at
/// the drawn level. D2D does not document its tap counts, so this number is
/// EMPIRICAL, and it is deliberately a constant rather than a scale-derived
/// formula — a variation that should have been strictly safer was measured
/// to be worse:
///
/// - A fixed margin of 8 let a 0.65x shrink's filter taps reach outside the
///   tile bitmap into clamped territory: the tile-boundary columns differed
///   from the untiled reference by up to 37 per channel on ~4x the average
///   pixel count (2026-09-20, a 900x600 image into 583x389).
/// - 32 removed that (`smoke82` S2b: whole-frame max delta 1, boundary
///   step equal to the untiled reference's).
/// - Deriving the margin from the draw scale instead (`32 / scale`, the
///   "taps reach further when the draw shrinks" argument) was tried on
///   2026-09-20 and MEASURED WORSE at the very configuration 32 was tuned
///   for: the same S2b comparison went from max delta 1 to 21 and the
///   boundary step from 0.299 to 3.887 — i.e. D2D's filtered pull of a
///   sub-rect source is not monotone in the bitmap's size, so a bigger
///   margin is not automatically a safer one. A derived margin needs its
///   own measurement campaign against the resampler, not an argument.
///
/// Known residual, recorded rather than papered over: on an ANISOTROPIC
/// frame — one axis at or above the master's (which forces level 0, see
/// [`detail_level`]) while the other axis shrinks by more than ~2x — a wide
/// kernel's outermost taps on the shrinking axis can still exceed 32 source
/// pixels. Reachable only with per-axis panscan zoom on a > `max_bitmap`
/// source under a filtered (non-NEAREST) tier; the failure mode is a 1-2 px
/// shading at that axis's tile boundaries. The frames the ticket's
/// acceptance covers (uniform fit / 1:1 / magnify) are unaffected.
///
/// The margin costs bitmap AREA only; a halo pixel is never an extra
/// *upload* (the neighbouring tile carries it anyway).
pub(crate) const FILTER_HALO_NATIVE: i32 = 32;

/// The self-set GPU-resident cap for base bitmap + tiles when the DXGI
/// budget is unavailable (or the adapter is not an `IDXGIAdapter3`).
pub(crate) const SELF_CAP_BYTES: u64 = 256 << 20;

/// The floor of the derived cap: a device reporting a tiny budget still
/// gets enough resident source to draw a viewport (the odd 1024² tile
/// plus a base bitmap), because the alternative — refusing to draw — is
/// the one outcome the ticket forbids ("显存压力先淘汰 GPU 缓存/降 tile
/// 工作集,不跳 GDI").
pub(crate) const MIN_CAP_BYTES: u64 = 16 << 20;

/// The resident-source cap: a conservative fraction of the driver's
/// `DXGI_QUERY_VIDEO_MEMORY_INFO.Budget`, clamped into
/// [`MIN_CAP_BYTES`]..[`SELF_CAP_BYTES`]. `None` = the query failed or the
/// adapter predates `IDXGIAdapter3` → the self-set cap.
///
/// The fraction is deliberately small (a quarter): the budget is what the
/// driver says this process may hold *in total*, and the same process also
/// owns two swapchain buffers; on an integrated GPU the LOCAL segment is
/// system memory, so its Budget is large but must not be read as "free
/// VRAM for tiles". Clamping to our own cap keeps both cases sane.
pub(crate) fn budget_cap(budget: Option<u64>) -> u64 {
    match budget {
        Some(b) => (b / 4).clamp(MIN_CAP_BYTES, SELF_CAP_BYTES),
        None => SELF_CAP_BYTES,
    }
}

/// A half-open pixel rectangle `[x, x+w) × [y, y+h)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

impl Rect {
    pub(crate) fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub(crate) fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    pub(crate) fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// The pixels both rects cover — empty when they do not touch.
    pub(crate) fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Rect::new(x, y, r - x, b - y)
    }

    /// This rect grown by `m` on every side, then clamped to `bounds`
    /// (the tile halo's shape: never a source rect outside the level).
    pub(crate) fn haloed(&self, m: i32, bounds: &Rect) -> Rect {
        let x = (self.x - m).max(bounds.x);
        let y = (self.y - m).max(bounds.y);
        let r = (self.right() + m).min(bounds.right());
        let b = (self.bottom() + m).min(bounds.bottom());
        Rect::new(x, y, r - x, b - y)
    }

    pub(crate) fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            i64::from(self.w) * i64::from(self.h)
        }
    }
}

/// The bytes one BGRA bitmap of `pixels` pixels costs — the accounting
/// unit of the whole budget story (the ticket's 字节预算).
pub(crate) fn bgra_bytes(pixels: i64) -> u64 {
    (pixels.max(0) as u64) * 4
}

/// Truncating (C-style, toward zero) integer division — the arithmetic
/// every projection in this port uses.
fn trunc_div(num: i64, den: i64) -> i64 {
    num / den
}

fn floor_div(num: i64, den: i64) -> i64 {
    let q = num / den;
    if num % den != 0 && (num < 0) != (den < 0) {
        q - 1
    } else {
        q
    }
}

fn ceil_div(num: i64, den: i64) -> i64 {
    let q = num / den;
    if num % den != 0 && (num < 0) == (den < 0) {
        q + 1
    } else {
        q
    }
}

/// A destination rectangle at SUB-PIXEL precision: the projection rounded
/// once to f32, never truncated. The distinction is the whole seam story
/// for filtered draws — the clipped (integer) rects decide *which* pixels a
/// tile owns, while the drawn rect decides *where* the resampler samples;
/// an integer-truncated draw rect makes each tile's implied scale
/// (`dest_w / src_w`) differ from the global one by up to a pixel per tile,
/// which D2D's continuous filters turn into a ±1 shading difference across
/// the whole frame (measured 2026-09-20: 19 % of the pixels of a 900x600
/// shrink differed by ±1 with integer draw rects, 0 with these).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DestRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl DestRect {
    pub(crate) fn right(&self) -> f32 {
        self.x + self.w
    }

    pub(crate) fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// The projection for DRAWN rects: the same expression as [`src_to_dest`],
/// evaluated in f64 and rounded ONCE to f32 — so a tile's implied affine map
/// (`dest_origin + src * dest_extent / src_extent`) reproduces the untiled
/// draw's to within an f32 ULP, which is what keeps a filtered draw from
/// drifting per tile.
pub(crate) fn src_to_dest_f(src: f32, dest_origin: i32, dest_extent: i32, src_extent: i32) -> f32 {
    (f64::from(dest_origin) + f64::from(src) * f64::from(dest_extent) / f64::from(src_extent))
        as f32
}

/// The projection for the tile GRID: integer edges, which the partition
/// argument needs — two tiles meeting at one source column must produce the
/// identical integer edge (the exact-partition property
/// `stitch::stitch_tiles_sized` pins for the GDI arm). Bounds are integers;
/// draws use [`src_to_dest_f`].
pub(crate) fn src_to_dest(src: i32, dest_origin: i32, dest_extent: i32, src_extent: i32) -> i32 {
    (i64::from(dest_origin)
        + trunc_div(
            i64::from(src) * i64::from(dest_extent),
            i64::from(src_extent),
        )) as i32
}

/// The source-level rect whose image covers `vis` (a destination rect):
/// the inverse map of the two visible corners, taken with floor/ceil so
/// the result is a SUPERSET of the true preimage (a subset would drop the
/// edge pixels the last tile must draw), then clamped to the level and
/// widened by one pixel for the truncating division's asymmetric rounding.
fn preimage(vis: &Rect, dest: &Rect, level_w: i32, level_h: i32) -> Rect {
    let x0 = floor_div(
        i64::from(vis.x - dest.x) * i64::from(level_w),
        i64::from(dest.w),
    );
    let y0 = floor_div(
        i64::from(vis.y - dest.y) * i64::from(level_h),
        i64::from(dest.h),
    );
    let x1 = ceil_div(
        i64::from(vis.right() - dest.x) * i64::from(level_w),
        i64::from(dest.w),
    );
    let y1 = ceil_div(
        i64::from(vis.bottom() - dest.y) * i64::from(level_h),
        i64::from(dest.h),
    );
    let x0 = (x0 - 1).clamp(0, i64::from(level_w));
    let y0 = (y0 - 1).clamp(0, i64::from(level_h));
    let x1 = (x1 + 1).clamp(0, i64::from(level_w));
    let y1 = (y1 + 1).clamp(0, i64::from(level_h));
    Rect::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
}

/// The destination rect a scene draws into, intersected with the viewport
/// — the only region a frame must actually render. `None` when the scene
/// is entirely off-screen (the caller clears to the background and stops).
pub(crate) fn visible_dest(dest: &Rect, viewport: &Rect) -> Option<Rect> {
    let vis = dest.intersect(viewport);
    if vis.is_empty() { None } else { Some(vis) }
}

/// The source level a render draws from:
///
/// - **0** when the render needs source-resolution pixels — the five-piece
///   contract's "resample-free" clause (1:1 and magnify, plus the #81
///   filter table's mag rows), or a shrink so shallow that no mip level
///   still covers it (a 900-wide render of a 1000-wide master samples
///   level 0 through the prefiltered cubic, exactly as #81 shipped).
/// - Otherwise the **deepest** level whose dimensions still cover the
///   render on both axes. That level is prefiltered (no aliasing stripes
///   under a deep shrink) and never smaller than the render (no
///   magnification of a mip), i.e. the cheapest source that is still
///   correct.
///
/// The "needs source resolution" test is per axis and asks whether a mip
/// would actually CHANGE that axis (`mip_size(master, 1) < master`): a
/// 1-px-tall master keeps height 1 at every level, so a 16777217x1 stripe
/// rendered 974x1 is exact in y at ANY level and needs the deep shrink in
/// x — forcing level 0 for it (the pre-#82 fix rule, "either axis at or
/// above the master's") made every tile's destination footprint a fraction
/// of a pixel wide, which D2D's aliased rasterizer drops outright: the
/// extreme stripe rendered BLANK while uploading all 16385 level-0 tiles
/// (71 MB) — found by the #82 smoke on this repo's own 2^24+1 fixture. The
/// per-axis test both fixes that and restores the invariant the halo
/// sizing relies on (a tiled draw's scale is in (0.5, 1] per axis).
pub(crate) fn detail_level(master_w: i32, master_h: i32, render_w: i32, render_h: i32) -> u32 {
    if master_w <= 0 || master_h <= 0 || render_w <= 0 || render_h <= 0 {
        return 0;
    }
    let (mip_w_1, mip_h_1) = mip::mip_size(master_w, master_h, 1);
    let x_needs_source = render_w >= master_w && mip_w_1 < master_w;
    let y_needs_source = render_h >= master_h && mip_h_1 < master_h;
    if x_needs_source || y_needs_source {
        return 0;
    }
    let mut best = 0u32;
    let mut level = 1u32;
    while level <= 31 {
        let (w, h) = mip::mip_size(master_w, master_h, level);
        if w >= render_w && h >= render_h {
            best = level;
            level += 1;
        } else {
            break;
        }
    }
    best
}

/// One tile's identity in the cache: the frame it was cut from, the level
/// it was cut at, and its grid cell. The frame generation is part of the
/// key, so a new image (or a rotate, which bumps the generation) can never
/// read a previous image's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TileKey {
    pub(crate) frame_gen: u64,
    pub(crate) level: u32,
    pub(crate) tx: i32,
    pub(crate) ty: i32,
}

/// One tile to draw: where its pixels come from, where they go, and the
/// logical interior the draw is clipped to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TileRequest {
    pub(crate) key: TileKey,
    /// The source rect the bitmap must carry (interior + halo, clipped to
    /// the level).
    pub(crate) src: Rect,
    /// `src` mapped through the global projection, at sub-pixel precision
    /// ([`src_to_dest_f`]) — the resampler's phase rides on these edges.
    pub(crate) dest: DestRect,
    /// The tile's logical interior (its grid cell ∩ the preimage) mapped
    /// through the same projection at INTEGER precision — the clip that
    /// makes the halo invisible and keeps the tiles an exact partition.
    pub(crate) clip: Rect,
}

impl TileRequest {
    /// Bytes the uploaded bitmap costs.
    pub(crate) fn bytes(&self) -> u64 {
        bgra_bytes(self.src.area())
    }
}

/// How one frame draws its level.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FramePlan {
    /// One bitmap of `level` over the whole scene rect: level 0 is the
    /// plain path (#80/#81 unchanged), level ≥ 1 is the overview a giant
    /// shrinks through.
    Base { level: u32 },
    /// Level `level` is too large for a single device bitmap: draw these
    /// tiles. They cover the visible destination exactly (the plan is only
    /// returned when it fits the frame budget, so there is never a
    /// half-tiled frame — see [`plan_frame`]).
    Tiles { level: u32, tiles: Vec<TileRequest> },
    /// Nothing to draw (degenerate sizes, or the scene fully off-screen):
    /// the caller clears to the background.
    Blank,
}

/// The frame's geometry as the plan sees it: the master's dimensions, the
/// scene's render size, the scene's destination rect, and the viewport it
/// is drawn into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameGeometry {
    pub(crate) master: (i32, i32),
    pub(crate) render: (i32, i32),
    pub(crate) dest: Rect,
    pub(crate) viewport: Rect,
}

/// The per-frame ladder (design §1):
///
/// 1. start at [`detail_level`];
/// 2. a level whose dimensions fit under `max_bitmap` AND whose CPU bytes
///    fit `level_budget_bytes` is drawn as ONE bitmap ([`FramePlan::Base`])
///    unless a diagnostic `forced_edge` asks for tiles;
/// 3. otherwise cut the visible region's preimage into tiles — if the
///    tiles that are not already resident fit `cap_bytes`, draw them;
/// 4. if they do not, deepen the level (a coarser mip needs strictly fewer
///    and smaller tiles, and — for the Base arm — strictly fewer CPU
///    bytes) and retry. Coarsening is the pressure response ("先淘汰 GPU
///    缓存/降 tile 工作集"): it degrades the whole frame uniformly instead
///    of leaving part of it sharp and part of it stale, and it terminates —
///    a level eventually fits the device and returns `Base`, at the extreme
///    the 1×1 level.
///
/// The CPU level budget is an input because the level bitmap is materialized
/// on the CPU before it is uploaded ([`crate::mip::LevelCache`]): a level the
/// cache would refuse must not be planned at all, or the caller has nothing
/// to draw and blanks the frame. Without it, a just-under-the-load-cap
/// extreme-aspect frame (e.g. 16389x8189 at 50% — level 1 is 134 MB against a
/// 128 MB cache) rendered as an empty letterbox on every paint in that zoom
/// band, a regression the removed #80 gate had covered with its GDI
/// hand-off. Deepening to a level the cache can hold keeps the frame drawing.
///
/// `resident` reports whether a tile is already uploaded (the caller's LRU
/// answers it) so the budget is charged for *new* uploads only.
pub(crate) fn plan_frame(
    frame_gen: u64,
    geometry: FrameGeometry,
    max_bitmap: u32,
    cap_bytes: u64,
    level_budget_bytes: u64,
    forced_edge: Option<i32>,
    resident: &dyn Fn(&TileKey) -> bool,
) -> FramePlan {
    let (master_w, master_h) = geometry.master;
    let (render_w, render_h) = geometry.render;
    let dest = geometry.dest;
    let viewport = geometry.viewport;

    if master_w <= 0 || master_h <= 0 || render_w <= 0 || render_h <= 0 {
        return FramePlan::Blank;
    }
    let Some(vis) = visible_dest(&dest, &viewport) else {
        return FramePlan::Blank;
    };
    // A master that fits the device is drawn as itself (level 0): the level
    // ladder exists for the GIANT regime the ticket scopes (> the device
    // maximum), and an ordinary shrink keeps #81's behavior exactly — the
    // full-resolution master through the prefiltered cubic, which is both
    // sharper than any box-level tap-in and the byte-for-byte path every
    // existing assertion and golden was frozen against.
    let level_w = master_w <= max_bitmap as i32;
    let level_h = master_h <= max_bitmap as i32;
    let mut level = if level_w && level_h {
        0
    } else {
        detail_level(master_w, master_h, render_w, render_h)
    };
    let mut forced = forced_edge.filter(|edge| *edge > 0);
    loop {
        let (level_w, level_h) = mip::mip_size(master_w, master_h, level);
        let device_fits = level_w <= max_bitmap as i32 && level_h <= max_bitmap as i32;
        // The CPU side gates BOTH arms: the level bitmap is materialized
        // before it is uploaded, and a tiled draw is cut FROM that same
        // bitmap — a level the cache would refuse is undrawable either way,
        // so the ladder must deepen instead of planning it (planning it
        // leaves the caller nothing to draw, which blanks the frame).
        let cpu_fits = bgra_bytes(i64::from(level_w) * i64::from(level_h)) <= level_budget_bytes;
        if device_fits && cpu_fits && forced.is_none() {
            return FramePlan::Base { level };
        }
        if cpu_fits {
            let edge = forced.unwrap_or(TILE_EDGE);
            let tiles = tile_requests(
                frame_gen,
                level,
                (level_w, level_h),
                &dest,
                &vis,
                edge,
                resident,
            );
            let fresh: u64 = tiles
                .iter()
                .filter(|t| !resident(&t.key))
                .map(|t| t.bytes())
                .sum();
            if !tiles.is_empty() && (forced.is_some() || fresh <= cap_bytes) {
                return FramePlan::Tiles { level, tiles };
            }
        }
        // Deepen (the forced edge is a one-shot diagnostic: the coarser
        // levels follow the normal rules). A fitting master is never
        // deepened — its next iteration returns Base{0}.
        forced = None;
        if level >= 31 {
            return FramePlan::Base { level: 31 };
        }
        level += 1;
    }
}

/// Cut the visible region's preimage into grid tiles at `level`.
///
/// The grid is enumerated windowed — only the cells the preimage actually
/// touches — and each cell contributes its intersection with the preimage
/// (the logical interior), expanded by [`FILTER_HALO_NATIVE`] for the bitmap. A
/// cell whose interior is empty contributes nothing.
fn tile_requests(
    frame_gen: u64,
    level: u32,
    level_dims: (i32, i32),
    dest: &Rect,
    vis: &Rect,
    edge: i32,
    _resident: &dyn Fn(&TileKey) -> bool,
) -> Vec<TileRequest> {
    let (level_w, level_h) = level_dims;
    let bounds = Rect::new(0, 0, level_w, level_h);
    // One constant margin, both axes (see FILTER_HALO_NATIVE for why it is
    // not scale-derived).
    let halo = FILTER_HALO_NATIVE;
    let pre = preimage(vis, dest, level_w, level_h);
    if pre.is_empty() {
        return Vec::new();
    }
    let tx0 = floor_div(i64::from(pre.x), i64::from(edge)) as i32;
    let ty0 = floor_div(i64::from(pre.y), i64::from(edge)) as i32;
    let tx1 = floor_div(i64::from(pre.right() - 1), i64::from(edge)) as i32;
    let ty1 = floor_div(i64::from(pre.bottom() - 1), i64::from(edge)) as i32;
    let mut tiles = Vec::new();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let cell = Rect::new(tx * edge, ty * edge, edge, edge).intersect(&bounds);
            let interior = cell.intersect(&pre);
            if interior.is_empty() {
                continue;
            }
            let src = interior.haloed(halo, &bounds);
            let dest_x = src_to_dest_f(src.x as f32, dest.x, dest.w, level_w);
            let dest_y = src_to_dest_f(src.y as f32, dest.y, dest.h, level_h);
            let dest_rect = DestRect {
                x: dest_x,
                y: dest_y,
                w: src_to_dest_f(src.right() as f32, dest.x, dest.w, level_w) - dest_x,
                h: src_to_dest_f(src.bottom() as f32, dest.y, dest.h, level_h) - dest_y,
            };
            // The clip partitions the mapped preimage — except that an
            // interior whose mapped extent truncates to zero pixels would
            // own nothing, and the destination pixel it should have shared
            // would be left at the letterbox colour (the untiled draw has
            // no such hole). Degenerate-fringe rule: every non-empty
            // interior owns at least one destination pixel; only a source
            // run that maps under a pixel wide can trigger it, in which
            // case the 1-px overlap with the next tile (row-major draw
            // order) is strictly better than a hole.
            let cx = src_to_dest(interior.x, dest.x, dest.w, level_w);
            let cy = src_to_dest(interior.y, dest.y, dest.h, level_h);
            let clip = Rect::new(
                cx,
                cy,
                (src_to_dest(interior.right(), dest.x, dest.w, level_w) - cx).max(1),
                (src_to_dest(interior.bottom(), dest.y, dest.h, level_h) - cy).max(1),
            );
            tiles.push(TileRequest {
                key: TileKey {
                    frame_gen,
                    level,
                    tx,
                    ty,
                },
                src,
                dest: dest_rect,
                clip,
            });
        }
    }
    tiles
}

/// A small LRU with a byte cap — the resident-source cache's policy,
/// pure and Win32-free so the eviction order and the accounting are
/// unit-testable. `K` is the key (`TileKey` for tiles, `u32` for CPU mip
/// levels); entries are `(key, bytes, last_use)`.
#[derive(Debug, Clone)]
pub(crate) struct Lru<K> {
    cap: u64,
    bytes: u64,
    clock: u64,
    entries: Vec<(K, u64, u64)>,
}

impl<K: PartialEq + Copy> Lru<K> {
    pub(crate) fn new(cap: u64) -> Self {
        Self {
            cap,
            bytes: 0,
            clock: 0,
            entries: Vec::new(),
        }
    }

    pub(crate) fn cap(&self) -> u64 {
        self.cap
    }

    pub(crate) fn total_bytes(&self) -> u64 {
        self.bytes
    }

    /// Entry count — the stats line's `tiles=` reads the request list, so
    /// this is the policy's own test surface.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn contains(&self, key: &K) -> bool {
        self.entries.iter().any(|(k, _, _)| k == key)
    }

    /// Bump `key` if resident (a hit) — `false` means the caller must
    /// upload.
    pub(crate) fn touch(&mut self, key: &K) -> bool {
        self.clock += 1;
        let now = self.clock;
        for entry in self.entries.iter_mut() {
            if entry.0 == *key {
                entry.2 = now;
                return true;
            }
        }
        false
    }

    /// Admit `key` (with its byte cost), evicting least-recently-used
    /// entries until it fits, and return the evicted keys so the caller
    /// can release their bitmaps. A cost above the whole cap is refused
    /// without evicting anything (the caller then draws without this
    /// entry — the ladder's job is to avoid asking for such a thing).
    pub(crate) fn insert(&mut self, key: K, bytes: u64) -> Vec<K> {
        let mut evicted = Vec::new();
        if bytes > self.cap {
            return evicted;
        }
        while self.bytes + bytes > self.cap && !self.entries.is_empty() {
            let idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.2)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let (old, old_bytes, _) = self.entries.remove(idx);
            self.bytes -= old_bytes;
            evicted.push(old);
        }
        self.clock += 1;
        self.entries.push((key, bytes, self.clock));
        self.bytes += bytes;
        evicted
    }

    /// Lower (or raise) the cap, returning whatever no longer fits. The
    /// live path derives its cap once at stack build, so this is the
    /// policy's test surface plus the hook a per-frame re-cap under
    /// driver pressure would use (the ticket's 显存压力 clause).
    #[cfg(test)]
    pub(crate) fn set_cap(&mut self, cap: u64) -> Vec<K> {
        self.cap = cap;
        let mut evicted = Vec::new();
        while self.bytes > self.cap && !self.entries.is_empty() {
            let idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.2)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let (old, old_bytes, _) = self.entries.remove(idx);
            self.bytes -= old_bytes;
            evicted.push(old);
        }
        evicted
    }

    /// Drop everything (a frame-generation change, a device rebuild).
    pub(crate) fn clear(&mut self) -> Vec<K> {
        self.bytes = 0;
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(k, _, _)| k)
            .collect()
    }
}

/// The byte ledger (the ticket's 分类记账): every buffer class the render
/// path owns, so "bounded" and "not leaking" are statements about numbers
/// rather than a feeling. Classes are the ticket's four; the counters feed
/// the close-time stats line the smoke asserts on.
#[derive(Debug, Clone, Default)]
pub(crate) struct MemLedger {
    /// The decoded master plus every cached CPU mip level.
    pub(crate) cpu_source: u64,
    /// CPU copies derived for display (the GDI face's DIB, the dump's
    /// staging when it is a persistent buffer).
    pub(crate) cpu_display: u64,
    /// Transient upload/readback staging alive right now.
    pub(crate) inflight: u64,
    /// Uploaded bitmaps: the base level bitmap plus every resident tile.
    pub(crate) gpu_resident: u64,
    /// How much of `gpu_resident` is the base (level) bitmap.
    pub(crate) gpu_base: u64,
    pub(crate) peak_gpu: u64,
    pub(crate) peak_inflight: u64,
    /// Uploaded tile bitmaps since the stack was built, and how many were
    /// evicted — the leak channel (a long animation must churn these
    /// rather than grow `gpu_resident`).
    pub(crate) tile_uploads: u64,
    pub(crate) tile_evictions: u64,
    /// CPU mip levels built (each is a full source pass).
    pub(crate) mip_builds: u64,
    /// The cap in force, for the stats line.
    pub(crate) cap: u64,
}

impl MemLedger {
    /// Record a GPU-resident total (base + tiles) and its peak.
    pub(crate) fn note_gpu(&mut self, bytes: u64) {
        self.gpu_resident = bytes;
        self.peak_gpu = self.peak_gpu.max(bytes);
    }

    /// Record the in-flight total and its peak.
    pub(crate) fn note_inflight(&mut self, bytes: u64) {
        self.inflight = bytes;
        self.peak_inflight = self.peak_inflight.max(bytes);
    }

    /// The close-time evidence line (stderr, only when the tile path was
    /// used) — the smoke's machine-readable budget channel.
    pub(crate) fn stats_line(&self, level: u32, tiles: usize) -> String {
        format!(
            "riviv: tiles level={level} tiles={tiles} base={} gpu={} peak_gpu={} \
             inflight={} peak_inflight={} uploads={} evictions={} mip_builds={} \
             source={} display={} cap={}",
            self.gpu_base,
            self.gpu_resident,
            self.peak_gpu,
            self.inflight,
            self.peak_inflight,
            self.tile_uploads,
            self.tile_evictions,
            self.mip_builds,
            self.cpu_source,
            self.cpu_display,
            self.cap
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &TileKey) -> bool {
        false
    }

    /// The plan's geometry in one value: `(master, render, dest, viewport)`.
    fn geom(master: (i32, i32), render: (i32, i32), dest: Rect, viewport: Rect) -> FrameGeometry {
        FrameGeometry {
            master,
            render,
            dest,
            viewport,
        }
    }

    // ---- detail_level ----

    #[test]
    fn detail_level_is_zero_for_one_to_one_and_magnify() {
        // The five-piece contract: 1:1 and magnify must sample the real
        // source, never a prefiltered level.
        assert_eq!(detail_level(40000, 256, 40000, 256), 0);
        assert_eq!(detail_level(40000, 256, 80000, 512), 0);
        assert_eq!(detail_level(40000, 256, 100, 256), 0);
        assert_eq!(detail_level(40000, 256, 40000, 100), 0);
    }

    #[test]
    fn detail_level_takes_the_deepest_level_that_still_covers_the_render() {
        // 40000x256 fit into 1900x12: level 4 is 2500x16 (covers), level 5
        // is 1250x8 (does not) — level 4 is the cheapest correct source.
        assert_eq!(detail_level(40000, 256, 1900, 12), 4);
        // A deeper shrink descends further (level 5 = 1250x8 covers 800x5).
        assert_eq!(detail_level(40000, 256, 800, 5), 5);
        // A 2x shrink takes level 1.
        assert_eq!(detail_level(40000, 256, 20000, 128), 1);
    }

    #[test]
    fn detail_level_falls_back_to_the_original_when_no_mip_still_covers() {
        // A shallow shrink: mip 1 is 500 wide and the render is 900 — no
        // level covers it, so level 0 draws through the prefiltered cubic
        // (#81's behavior for this regime).
        assert_eq!(detail_level(1000, 1000, 900, 900), 0);
        assert_eq!(detail_level(1000, 1000, 1000, 1000), 0);
    }

    #[test]
    fn a_one_pixel_axis_does_not_force_the_original_level() {
        // The #82 smoke's finding: a 16777217x1 stripe fit into 974x1 is
        // EXACT in y at every level (a 1-px axis never shrinks), so it
        // needs the deep x shrink — level 0 would cut the whole stripe into
        // 16385 tiles whose destinations are a fraction of a pixel wide,
        // which D2D's aliased rasterizer drops: the frame rendered blank
        // while uploading 71 MB. The level must cover the render in x.
        let level = detail_level(16777217, 1, 974, 1);
        assert!(level >= 1, "the stripe must shrink through a level");
        let (w, h) = mip::mip_size(16777217, 1, level);
        assert!(w >= 974 && h >= 1, "level {level} is {w}x{h}");
        assert!(
            w <= 1024,
            "level {level} = {w} should be the tightest cover"
        );
        // The same shape MAGNIFIED still needs the real row: no level can
        // cover a render taller than the master's only row.
        assert_eq!(detail_level(16777217, 1, 974, 8), 0);
    }

    #[test]
    fn the_plan_for_a_one_pixel_stripe_is_the_overview_bitmap() {
        // End to end through the ladder: the stripe's plan is ONE bitmap
        // (Base at a deep level, a few KB), not the 16385-tile level 0.
        let plan = plan_frame(
            1,
            geom(
                (16777217, 1),
                (974, 1),
                Rect::new(0, 0, 974, 1),
                Rect::new(0, 0, 1920, 1080),
            ),
            1 << 23,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Base { level } = plan else {
            panic!("the stripe's overview must be a single bitmap: {plan:?}");
        };
        assert!(level >= 1);
        let (w, h) = mip::mip_size(16777217, 1, level);
        assert!(w <= 1024 && h == 1, "the bitmap is {w}x{h}");
        assert!(bgra_bytes(i64::from(w) * i64::from(h)) <= 4096);
    }

    #[test]
    fn every_tile_clips_a_non_empty_region_and_the_clips_cover_the_visible_dest() {
        // The degenerate-fringe rule's invariant, over the regimes the
        // ladder actually produces: no tile is pushed with an empty clip
        // (it would upload a bitmap for a draw that cannot land), and the
        // clips together cover the whole visible destination (no
        // destination pixel is dropped back to the letterbox colour).
        for &(mw, mh, rw, rh, vw, vh, edge) in &[
            (1025i32, 4i32, 512i32, 4i32, 512i32, 4i32, 1024i32),
            (40000, 256, 40000, 256, 974, 484, 1024),
            (40000, 256, 1900, 12, 974, 484, 256),
            (16777217, 1, 8, 1, 1200, 900, 1024),
            (3000, 2000, 3000, 2000, 3000, 2000, 512),
            (1000, 1000, 999, 999, 500, 500, 128),
        ] {
            let dest = Rect::new(0, 0, rw, rh);
            let viewport = Rect::new(0, 0, vw, vh);
            let vis = match visible_dest(&dest, &viewport) {
                Some(vis) => vis,
                None => continue,
            };
            let plan = plan_frame(
                1,
                geom((mw, mh), (rw, rh), dest, viewport),
                1 << 23,
                u64::MAX,
                u64::MAX,
                Some(edge),
                &never,
            );
            let FramePlan::Tiles { tiles, .. } = plan else {
                panic!("{mw}x{mh} -> {rw}x{rh} (edge {edge}): expected tiles, got {plan:?}");
            };
            assert!(!tiles.is_empty());
            for t in &tiles {
                assert!(
                    t.clip.w >= 1 && t.clip.h >= 1,
                    "empty clip {:?} for {:?}",
                    t.clip,
                    t.key
                );
            }
            assert!(
                tiles[0].clip.x <= vis.x && tiles[0].clip.y <= vis.y,
                "the first clip must reach the visible origin"
            );
            let last = tiles.last().unwrap();
            assert!(
                last.clip.right() >= vis.right() && last.clip.bottom() >= vis.bottom(),
                "the last clip must reach the visible end"
            );
            // Adjacent clips share an edge (or overlap by the degenerate
            // 1 px) — never a gap.
            for pair in tiles.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                assert!(b.clip.x <= a.clip.right(), "gap at {:?}", b.clip);
            }
        }
    }
    #[test]
    fn detail_level_never_magnifies_the_level_it_picks() {
        // Whichever level comes back, it covers the render on both axes
        // (level 0 trivially does).
        for &(mw, mh, rw, rh) in &[
            (40000, 40000, 2160, 2160),
            (40000, 256, 1900, 12),
            (20000, 200, 1900, 19),
            (3000, 40000, 150, 2000),
            (16777217, 1, 1900, 1),
        ] {
            let level = detail_level(mw, mh, rw, rh);
            let (w, h) = mip::mip_size(mw, mh, level);
            assert!(
                w >= rw && h >= rh,
                "{mw}x{mh} into {rw}x{rh}: level {level} is {w}x{h}"
            );
            if level > 0 {
                let (nw, nh) = mip::mip_size(mw, mh, level + 1);
                assert!(
                    nw < rw || nh < rh,
                    "{mw}x{mh} into {rw}x{rh}: level {level} was not the deepest ({nw}x{nh} still covers)"
                );
            }
        }
    }

    // ---- the projection / partition ----

    #[test]
    fn src_to_dest_is_the_shared_edge_projection() {
        // dest 0..1900 from source 0..40000: source 20000 maps to 950
        // exactly, and the same source coordinate always maps to the same
        // destination (the property adjacent tiles rely on).
        assert_eq!(src_to_dest(0, 0, 1900, 40000), 0);
        assert_eq!(src_to_dest(20000, 0, 1900, 40000), 950);
        assert_eq!(src_to_dest(40000, 0, 1900, 40000), 1900);
        // A nonzero destination origin is an additive offset only.
        assert_eq!(src_to_dest(20000, 137, 1900, 40000), 950 + 137);
    }

    #[test]
    fn the_drawn_rect_keeps_the_global_scale_at_sub_pixel_precision() {
        // The property that makes a filtered frame match its untiled
        // reference: the affine map D2D derives from a tile's (src, dest)
        // pair must be the global one. With integer draw rects the implied
        // scale drifts by up to 1/sw per tile; the f64-then-f32 projection
        // keeps it within an ULP.
        let (mw, mh, rw, rh) = (900i32, 600i32, 583, 389);
        let scene = Rect::new(153, 0, rw, rh);
        let vis = visible_dest(&scene, &Rect::new(0, 0, 674, 246)).expect("visible");
        let tiles = tile_requests(1, 0, (mw, mh), &scene, &vis, 256, &never);
        assert!(tiles.len() >= 4);
        let global = f64::from(rw) / f64::from(mw);
        for t in &tiles {
            let scale = f64::from(t.dest.w) / f64::from(t.src.w);
            assert!(
                (scale - global).abs() < 1e-5,
                "tile {:?} implies scale {scale}, global {global}",
                t.key
            );
            let offset = f64::from(t.dest.x) - f64::from(t.src.x) * scale;
            let global_offset = f64::from(scene.x);
            assert!(
                (offset - global_offset).abs() < 0.01,
                "tile {:?} implies offset {offset}, global {global_offset}",
                t.key
            );
        }
    }

    #[test]
    fn adjacent_tiles_partition_the_destination_exactly() {
        // The seam standard: for a giant's 1:1 render, tile clip rects must
        // share edges bit for bit — no gap, no overlap, no dropped or
        // duplicated column.
        let dest = Rect::new(-3000, 0, 16777217, 1);
        let viewport = Rect::new(0, 0, 1920, 1080);
        let plan = plan_frame(
            1,
            geom((16777217, 1), (16777217, 1), dest, viewport),
            1 << 23,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("a 1:1 giant must tile, got {plan:?}");
        };
        assert!(tiles.len() >= 2, "the viewport spans several tiles");
        for pair in tiles.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert_eq!(a.clip.right(), b.clip.x, "gap or overlap at {:?}", b.clip);
            // The haloed DRAW rects deliberately overlap their neighbour by
            // 2×FILTER_HALO_NATIVE — that overlap is exactly what the clip hides,
            // and a gap here would expose the letterbox colour.
            assert!(
                a.dest.right() > b.dest.x,
                "the halo must overlap the neighbour, not leave a gap"
            );
            assert!(a.dest.x <= a.clip.x as f32 && a.dest.right() >= a.clip.right() as f32);
        }
        // ...and the clips cover the whole visible destination.
        assert!(tiles[0].clip.x <= viewport.x);
        assert!(tiles.last().unwrap().clip.right() >= viewport.right());
    }

    #[test]
    fn tiles_are_windowed_to_the_visible_region_only() {
        // A 16777217-wide giant at 1:1: the plan must hold the handful of
        // tiles the viewport touches, NOT the 16384 columns of the image
        // (the ticket's O(viewport) claim, and the reason a full-range
        // enumeration would be a hang rather than a slow paint).
        let plan = plan_frame(
            7,
            geom(
                (16777217, 1),
                (16777217, 1),
                Rect::new(0, 0, 16777217, 1),
                Rect::new(0, 0, 1920, 1080),
            ),
            1 << 23,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles");
        };
        assert!(tiles.len() <= 4, "got {} tiles", tiles.len());
        // Scrolled far right, the window follows (never back to column 0).
        let plan = plan_frame(
            7,
            geom(
                (16777217, 1),
                (16777217, 1),
                Rect::new(-4000000, 0, 16777217, 1),
                Rect::new(0, 0, 1920, 1080),
            ),
            1 << 23,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles");
        };
        assert!(tiles.iter().all(|t| t.key.tx > 3000), "{:?}", tiles[0].key);
        assert!(tiles.len() <= 4);
    }

    #[test]
    fn the_halo_expands_the_bitmap_and_draw_but_never_the_clip() {
        // A tile in the middle of a giant carries FILTER_HALO_NATIVE source pixels
        // on each side for its taps, and clips those away again.
        let dest = Rect::new(0, 0, 8192, 1024);
        let plan = plan_frame(
            1,
            geom(
                (8192, 1024),
                (8192, 1024),
                dest,
                Rect::new(0, 0, 8192, 1024),
            ),
            1 << 23,
            u64::MAX,
            u64::MAX,
            Some(1024),
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles");
        };
        assert_eq!(tiles.len(), 8);
        // An interior tile (not the first): its bitmap is haloed, its clip
        // is its own grid cell exactly.
        let t = tiles[1];
        assert_eq!(t.clip, Rect::new(1024, 0, 1024, 1024));
        assert_eq!(
            t.src,
            Rect::new(
                1024 - FILTER_HALO_NATIVE,
                0,
                1024 + 2 * FILTER_HALO_NATIVE,
                1024
            )
        );
        assert_eq!(
            t.dest,
            DestRect {
                x: t.src.x as f32,
                y: t.src.y as f32,
                w: t.src.w as f32,
                h: t.src.h as f32
            },
            "1:1 maps the integer source rect onto itself exactly"
        );
        // The first tile's halo is clamped at the level's edge (no
        // out-of-image source rect).
        assert_eq!(tiles[0].src.x, 0);
        assert_eq!(tiles[0].src.w, 1024 + FILTER_HALO_NATIVE);
        assert_eq!(tiles[0].clip.x, 0);
        // The last tile's halo is clamped at the right edge.
        let last = tiles[7];
        assert_eq!(last.src.right(), 8192);
        assert_eq!(last.clip.right(), 8192);
    }

    #[test]
    fn the_halo_bytes_are_the_bitmap_cost() {
        let plan = plan_frame(
            1,
            geom(
                (4096, 4096),
                (4096, 4096),
                Rect::new(0, 0, 4096, 4096),
                Rect::new(0, 0, 1000, 1000),
            ),
            1 << 23,
            u64::MAX,
            u64::MAX,
            Some(1024),
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles");
        };
        let t = tiles[0];
        assert_eq!(
            t.bytes(),
            (t.src.w as u64) * (t.src.h as u64) * 4,
            "the haloed bitmap is what gets uploaded"
        );
    }

    // ---- the level / budget ladder ----

    #[test]
    fn a_level_that_fits_the_device_is_drawn_as_one_bitmap() {
        // Any ordinary image — fitting master, shrink or not — is level 0:
        // the #80/#81 path, byte for byte (a 2x shrink keeps sampling the
        // full master through the prefiltered cubic, not a box level).
        let plan = plan_frame(
            1,
            geom(
                (3000, 2000),
                (1500, 1000),
                Rect::new(0, 0, 1500, 1000),
                Rect::new(0, 0, 1920, 1080),
            ),
            16384,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        assert_eq!(plan, FramePlan::Base { level: 0 });
        assert_eq!(
            plan_frame(
                1,
                geom(
                    (3000, 2000),
                    (3000, 2000),
                    Rect::new(0, 0, 3000, 2000),
                    Rect::new(0, 0, 1920, 1080)
                ),
                16384,
                u64::MAX,
                u64::MAX,
                None,
                &never
            ),
            FramePlan::Base { level: 0 }
        );
        // A giant's deep shrink: the chosen level fits, so the overview is
        // a single bitmap too (no tiles at all).
        let plan = plan_frame(
            1,
            geom(
                (40000, 40000),
                (2160, 2160),
                Rect::new(0, 0, 2160, 2160),
                Rect::new(0, 0, 1920, 1080),
            ),
            16384,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Base { level } = plan else {
            panic!("a deep shrink of a giant is the overview: {plan:?}");
        };
        // Level 4 is 2500x2500: the deepest level that still covers the
        // 2160 render (level 5's 1250 would magnify).
        assert_eq!(level, 4);
        assert_eq!(mip::mip_size(40000, 40000, level), (2500, 2500));
    }

    #[test]
    fn a_giant_at_one_to_one_tiles_instead_of_uploading_the_level() {
        // 40000x256 on a 16384 device: level 0 does not fit → tiles.
        let plan = plan_frame(
            2,
            geom(
                (40000, 256),
                (40000, 256),
                Rect::new(0, 0, 40000, 256),
                Rect::new(0, 0, 1920, 1080),
            ),
            16384,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { level, tiles } = plan else {
            panic!("expected tiles");
        };
        assert_eq!(level, 0);
        assert!(tiles.iter().all(|t| t.key.level == 0));
    }

    #[test]
    fn the_plan_deepens_when_the_tile_set_exceeds_the_frame_budget() {
        // The whole 40000-wide level is visible (a fit-ish shrink whose
        // level is still too big for the device): 40 tiles is more than
        // this budget allows, so the plan coarsens to the first level that
        // fits as ONE bitmap instead of drawing a half-tiled frame.
        let plan = plan_frame(
            3,
            geom(
                (40000, 40000),
                (30000, 30000),
                Rect::new(0, 0, 30000, 30000),
                Rect::new(0, 0, 30000, 30000),
            ),
            16384,
            1024 * 1024,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Base { level } = plan else {
            panic!("pressure must coarsen to a single bitmap: {plan:?}");
        };
        let (w, h) = mip::mip_size(40000, 40000, level);
        assert!(w <= 16384 && h <= 16384, "level {level} is {w}x{h}");
        assert!(level >= 1, "level 0 does not fit the device");
    }

    #[test]
    fn resident_tiles_are_not_charged_against_the_frame_budget() {
        // The same plan twice: with every tile already uploaded the second
        // frame costs nothing, so it stays tiled even under a tiny budget.
        let dest = Rect::new(0, 0, 40000, 40000);
        let viewport = Rect::new(0, 0, 1920, 1080);
        let args = (40000, 40000, 40000, 40000);
        let probe = plan_frame(
            4,
            geom((args.0, args.1), (args.2, args.3), dest, viewport),
            16384,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { tiles: first, .. } = probe else {
            panic!("expected tiles at 1:1");
        };
        let resident = first.clone();
        let hit = move |k: &TileKey| resident.iter().any(|t| t.key == *k);
        let plan = plan_frame(
            4,
            geom((args.0, args.1), (args.2, args.3), dest, viewport),
            16384,
            0,
            u64::MAX,
            None,
            &hit,
        );
        assert!(
            matches!(plan, FramePlan::Tiles { .. }),
            "zero fresh bytes must not force a coarser plan: {plan:?}"
        );
    }

    #[test]
    fn a_forced_edge_overrides_the_single_bitmap_shortcut() {
        // The smoke's zero-diff channel: a small image that would draw as
        // one bitmap is cut anyway, so "tiled vs untiled" is comparable on
        // the SAME image and transform.
        let plan = plan_frame(
            5,
            geom(
                (3000, 2000),
                (3000, 2000),
                Rect::new(0, 0, 3000, 2000),
                Rect::new(0, 0, 3000, 2000),
            ),
            16384,
            u64::MAX,
            u64::MAX,
            Some(512),
            &never,
        );
        let FramePlan::Tiles { level, tiles } = plan else {
            panic!("the forced edge must produce tiles: {plan:?}");
        };
        assert_eq!(level, 0);
        assert_eq!(
            tiles.len(),
            6 * 4,
            "3000/512 = 6 columns, 2000/512 = 4 rows"
        );
        // Every tile still clips to its own cell: the union is the image.
        assert!(tiles.iter().all(|t| t.clip.w <= 512 && t.clip.h <= 512));
    }

    #[test]
    fn a_scene_off_screen_is_blank_and_degenerate_sizes_too() {
        assert_eq!(
            plan_frame(
                1,
                geom(
                    (100, 100),
                    (100, 100),
                    Rect::new(5000, 0, 100, 100),
                    Rect::new(0, 0, 1920, 1080)
                ),
                16384,
                u64::MAX,
                u64::MAX,
                None,
                &never
            ),
            FramePlan::Blank
        );
        assert_eq!(
            plan_frame(
                1,
                geom(
                    (0, 0),
                    (0, 0),
                    Rect::new(0, 0, 0, 0),
                    Rect::new(0, 0, 1920, 1080)
                ),
                16384,
                u64::MAX,
                u64::MAX,
                None,
                &never
            ),
            FramePlan::Blank
        );
    }

    #[test]
    fn the_preimage_covers_the_visible_dest_when_panned_negative() {
        // A panned-to-the-left giant: the visible corners map to source
        // coordinates that must still bracket what is on screen (floor/ceil
        // + the one-pixel margin).
        let dest = Rect::new(-1000, -500, 40000, 40000);
        let vis = Rect::new(0, 0, 1920, 1080).intersect(&dest);
        let pre = preimage(&vis, &dest, 40000, 40000);
        // Source x for dest x=0 is (0 - -1000) = 1000; the visible right
        // edge 1920 → 2920. The bracket must contain [1000, 2920).
        assert!(pre.x <= 1000, "{pre:?}");
        assert!(pre.right() >= 2920, "{pre:?}");
    }

    // ---- LRU + ledger ----

    #[test]
    fn the_lru_evicts_the_least_recently_used_entry_first() {
        let mut lru = Lru::new(300);
        assert!(lru.insert(1u32, 100).is_empty());
        assert!(lru.insert(2, 100).is_empty());
        assert!(lru.insert(3, 100).is_empty());
        assert_eq!(lru.total_bytes(), 300);
        // A hit refreshes #1, so #2 is now the coldest: admitting #4 evicts
        // #2, not #1.
        assert!(lru.touch(&1));
        let evicted = lru.insert(4, 100);
        assert_eq!(evicted, vec![2]);
        assert!(lru.contains(&1) && lru.contains(&3) && lru.contains(&4));
        assert_eq!(lru.total_bytes(), 300);
    }

    #[test]
    fn the_lru_evicts_several_entries_for_one_large_admission() {
        let mut lru = Lru::new(1000);
        lru.insert(1u32, 100);
        lru.insert(2u32, 100);
        lru.insert(3u32, 100);
        // 300 + 900 > 1000: the two coldest go, and the total lands exactly
        // on the cap.
        let evicted = lru.insert(4u32, 900);
        assert_eq!(evicted, vec![1, 2]);
        assert_eq!(lru.total_bytes(), 1000);
        assert!(lru.contains(&3) && lru.contains(&4));
    }

    #[test]
    fn the_lru_refuses_an_entry_larger_than_its_whole_cap() {
        // A 4096² tile against a 16 MiB cap is fine; a 16384² one is not —
        // refusing it (rather than evicting everything and still failing)
        // leaves the cache usable for the next frame.
        let mut lru = Lru::new(1000);
        lru.insert(1u32, 100);
        assert!(lru.insert(2u32, 2000).is_empty());
        assert!(!lru.contains(&2));
        assert!(lru.contains(&1), "a refused admission must not evict");
        assert_eq!(lru.total_bytes(), 100);
    }

    #[test]
    fn the_lru_cap_can_shrink_under_pressure() {
        // The GPU-budget path: the cap drops, the coldest entries go first,
        // and the total lands inside the new cap.
        let mut lru = Lru::new(1000);
        lru.insert(1u32, 400);
        lru.insert(2u32, 400);
        lru.insert(3u32, 200);
        let evicted = lru.set_cap(500);
        assert_eq!(evicted, vec![1, 2]);
        assert_eq!(lru.total_bytes(), 200);
        assert!(lru.total_bytes() <= lru.cap());
    }

    #[test]
    fn clearing_the_lru_returns_every_key_for_release() {
        let mut lru = Lru::new(1000);
        lru.insert(1u32, 10);
        lru.insert(2u32, 10);
        let mut gone = lru.clear();
        gone.sort();
        assert_eq!(gone, vec![1, 2]);
        assert_eq!(lru.total_bytes(), 0);
        assert_eq!(lru.len(), 0);
    }

    #[test]
    fn budget_cap_clamps_the_driver_budget_into_our_own_range() {
        // No DXGI answer → the self-set cap.
        assert_eq!(budget_cap(None), SELF_CAP_BYTES);
        // A typical budget (a few GB) → a quarter of it, capped.
        assert_eq!(budget_cap(Some(8 << 30)), SELF_CAP_BYTES);
        assert_eq!(budget_cap(Some(512 << 20)), 128 << 20);
        // A tiny budget → the floor, never less (the viewport still draws).
        assert_eq!(budget_cap(Some(4 << 20)), MIN_CAP_BYTES);
        assert_eq!(budget_cap(Some(0)), MIN_CAP_BYTES);
    }

    #[test]
    fn the_ledger_tracks_peaks_and_renders_the_stats_line() {
        let mut ledger = MemLedger {
            cap: budget_cap(Some(512 << 20)),
            ..MemLedger::default()
        };
        ledger.note_gpu(1000);
        ledger.note_gpu(4000);
        ledger.note_gpu(2000);
        assert_eq!(ledger.gpu_resident, 2000);
        assert_eq!(ledger.peak_gpu, 4000);
        ledger.note_inflight(64);
        assert_eq!(ledger.peak_inflight, 64);
        ledger.cpu_source = 1 << 20;
        ledger.tile_uploads = 9;
        ledger.tile_evictions = 4;
        ledger.mip_builds = 1;
        let line = ledger.stats_line(4, 16);
        for field in [
            "level=4",
            "tiles=16",
            "gpu=2000",
            "peak_gpu=4000",
            "uploads=9",
            "evictions=4",
            "mip_builds=1",
            "source=1048576",
            "cap=134217728",
        ] {
            assert!(line.contains(field), "{field} missing from {line}");
        }
    }

    #[test]
    fn adjacent_tile_rows_partition_the_destination_on_the_y_axis() {
        // The seam partition is per-axis symmetric, but the shipped tests
        // only exercised x (a 1-px-tall giant): this pins the same edges
        // vertically, where a gap would be a horizontal seam.
        let plan = plan_frame(
            3,
            geom(
                (4096, 8192),
                (4096, 8192),
                Rect::new(-100, -2500, 4096, 8192),
                Rect::new(0, 0, 1920, 1080),
            ),
            2048,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles: {plan:?}");
        };
        assert!(tiles.len() >= 2);
        // Row-major order: consecutive entries must share an edge only
        // within a row; a row change restarts at the row's first column,
        // and the COLUMN comparison is what pins the y axis (a gap there
        // would be a horizontal seam).
        for pair in tiles.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.key.ty == b.key.ty {
                assert_eq!(
                    a.clip.right(),
                    b.clip.x,
                    "column gap in row {} at {:?}",
                    a.key.ty,
                    b.clip
                );
            } else {
                assert_eq!(
                    b.clip.x, tiles[0].clip.x,
                    "row {} restarts off-grid",
                    b.key.ty
                );
                assert!(b.key.ty > a.key.ty, "rows stay in order");
            }
        }
        for t in &tiles {
            let below = tiles
                .iter()
                .find(|o| o.key.tx == t.key.tx && o.key.ty == t.key.ty + 1);
            if let Some(below) = below {
                assert!(
                    below.clip.y <= t.clip.bottom() && t.clip.bottom() <= below.clip.y + 1,
                    "row gap at {:?} (above ends {})",
                    below.clip,
                    t.clip.bottom()
                );
            }
        }
        assert!(tiles.iter().any(|t| t.key.ty > 0), "more than one row");
    }

    #[test]
    fn the_halo_is_the_measured_constant_not_a_derived_one() {
        // The margin is empirical (see FILTER_HALO_NATIVE): D2D's filtered
        // pull of a sub-rect source is not monotone in the bitmap size, and
        // a scale-derived widening (`32 / scale`) was measured WORSE at the
        // configuration the constant was tuned for (smoke82 S2b: whole-frame
        // max delta 1 -> 21, boundary step 0.299 -> 3.887). Pinning the value
        // here means a future "optimization" of it has to read that record
        // and re-run the S2b comparison, rather than silently halving the
        // margin.
        assert_eq!(FILTER_HALO_NATIVE, 32);
        // The tiles carry exactly that margin, on both axes, clamped to the
        // level (a tile at the level's edge must not name an out-of-image
        // source rect).
        let plan = plan_frame(
            1,
            geom(
                (8192, 512),
                (8192, 512),
                Rect::new(0, 0, 8192, 512),
                Rect::new(0, 0, 8192, 512),
            ),
            2048,
            u64::MAX,
            u64::MAX,
            Some(2048),
            &never,
        );
        let FramePlan::Tiles { tiles, .. } = plan else {
            panic!("expected tiles: {plan:?}");
        };
        assert!(tiles.len() >= 3);
        assert_eq!(tiles[1].src.x, 2048 - FILTER_HALO_NATIVE);
        assert_eq!(tiles[1].src.w, 2048 + 2 * FILTER_HALO_NATIVE);
        assert_eq!(
            tiles[0].src.x, 0,
            "the first tile's halo clamps at the level"
        );
    }

    #[test]
    fn a_level_over_the_cpu_budget_deepens_instead_of_blanking() {
        // The refused-level regression the pre-review found: a level whose
        // CPU bytes exceed the cache cap must never be planned as `Base`
        // (the cache would refuse it and the paint would blank). 16389x8189
        // at 50% picks level 1 (134 MB) — over a 128 MB budget — so the
        // ladder must deepen until a level fits as one bitmap.
        let budget = 128u64 << 20;
        let plan = plan_frame(
            1,
            geom(
                (16389, 8189),
                (8194, 4094),
                Rect::new(0, 0, 8194, 4094),
                Rect::new(0, 0, 1920, 1080),
            ),
            16384,
            u64::MAX,
            budget,
            None,
            &never,
        );
        let FramePlan::Base { level } = plan else {
            panic!("the frame must draw as one bitmap: {plan:?}");
        };
        let (w, h) = mip::mip_size(16389, 8189, level);
        assert!(
            bgra_bytes(i64::from(w) * i64::from(h)) <= budget,
            "level {level} = {w}x{h} still exceeds the CPU budget"
        );
        assert!(level >= 1, "level 0 does not fit the device either");
        // With an ample budget the same frame takes the shallowest level
        // that covers the render — the deepen is budget-driven, not a
        // blanket coarsening.
        let ample = plan_frame(
            1,
            geom(
                (16389, 8189),
                (8194, 4094),
                Rect::new(0, 0, 8194, 4094),
                Rect::new(0, 0, 1920, 1080),
            ),
            16384,
            u64::MAX,
            u64::MAX,
            None,
            &never,
        );
        let FramePlan::Base { level: deep } = ample else {
            panic!("expected Base");
        };
        assert!(deep < level, "an ample budget plans the deeper level");
    }
}
