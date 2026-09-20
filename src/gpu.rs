//! The D2D/DXGI render stack (#80, ADR 0002 D4/D5/D8): one [`GpuStack`]
//! owns every device-dependent object — D3D11 device, DXGI swapchain on
//! the `riviv_view` child, D2D factory/device/device-context, the target
//! bitmap (the swapchain's back buffer) and the uploaded frame bitmap —
//! they live and die together (field order IS the drop order; the
//! windows-rs COM wrappers Release in declaration order).
//!
//! The paint is message-driven like the GDI arm (ADR 0002 D8): WM_PAINT →
//! BeginPaint → BeginDraw → Clear → DrawBitmap → EndDraw → Present(0,0)
//! → EndPaint. The 1:1 five-piece (ADR 0002 D6) is enforced here: unit
//! mode PIXELS set once at build, NO SetTransform call anywhere in the
//! arm (identity audit — grep-provable), exact i32 rects (the giant tile
//! path's DRAWN rects are sub-pixel f32 by design — see tile.rs; the CLIPS
//! stay integer), every surface
//! `B8G8R8A8_UNORM` (never `_SRGB`), and the per-draw interpolation mode
//! from [`d2d_interp_mode`].
//!
//! The frame source is the CPU master (`PixelFrame`, #76): uploads copy
//! the master's bytes straight into a D2D bitmap keyed by
//! (frame_gen, w, h) — device-loss recovery re-uploads without a
//! re-decode. The failure ladder (design §7) is degrade-not-fatal inside
//! the paint; the final tier defers its fatal to after the paint borrow
//! drops (`window.rs` checks `gpu_pending_fatal`).

use std::mem::ManuallyDrop;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, GetLastError, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_RECT_U, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_INTERPOLATION_MODE, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_MAP_OPTIONS_READ, D2D1_PRIMITIVE_BLEND_COPY, D2D1_UNIT_MODE_PIXELS, D2D1CreateFactory,
    ID2D1Bitmap, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1Image, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_DRIVER_INTERNAL_ERROR,
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT,
    DXGI_QUERY_VIDEO_MEMORY_INFO, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter3, IDXGIDevice,
    IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetParent, IsIconic};
use windows::core::Interface;

use crate::config::RendererKind;
use crate::paint::scene_rect;
use crate::window::{fatal, state_of};

// ---------------------------------------------------------------------------
// Pure decisions (design §11's unit-test list) — no windows calls below
// them on these paths.
// ---------------------------------------------------------------------------

/// The runtime failure ladder's sliding window (design §7).
pub(crate) const FAILURE_WINDOW_MS: u32 = 10_000;

/// The verdict after recording one device-loss timestamp (design §7):
/// `None` = rebuild the same kind, `Escalate` = switch to WARP (the
/// permanent software fallback), `Fatal` = WARP failed 3-in-10s too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureVerdict {
    None,
    Escalate,
    Fatal,
}

/// Slide the 10s window over the failure timestamps: prune entries older
/// than the window, record `now_ms`, and fire on the third failure INSIDE
/// the window — clearing the list (the escalation starts its own count).
/// The verdict's Fatal tier only fires when the stack is already WARP.
/// Timestamps are GetTickCount's native u32 ms domain: the wrapping
/// subtraction IS the C DWORD arithmetic (a wrap at 2^32 measures
/// normally, exactly like `handoff_add_mode`'s tick delta).
pub(crate) fn failure_window(
    now_ms: u32,
    already_warp: bool,
    failures: &mut Vec<u32>,
) -> FailureVerdict {
    failures.retain(|&t| now_ms.wrapping_sub(t) < FAILURE_WINDOW_MS);
    failures.push(now_ms);
    if failures.len() < 3 {
        return FailureVerdict::None;
    }
    failures.clear();
    if already_warp {
        FailureVerdict::Fatal
    } else {
        FailureVerdict::Escalate
    }
}

/// The #81 full filter table (ADR 0002 D6): a 1:1 render is NEAREST
/// unconditionally (the pixel-exact contract); a shrink follows
/// `shrink_blit_mode`, a magnify follows `mag_filter` — the same
/// 0=Nearest/1=Linear ini tiers the GDI arms read (config.c:41-42), but
/// the D2D arm renders the shrink Linear tier as HIGH_QUALITY_CUBIC: a
/// prefiltered cubic beats GDI HALFTONE on strong downscales (recorded as
/// a README Differences item; the ini tier's user-visible name stays
/// "Linear").
pub(crate) fn d2d_interp_mode(
    is_1x1: bool,
    shrink_linear: bool,
    mag_linear: bool,
    shrinking: bool,
) -> D2D1_INTERPOLATION_MODE {
    if is_1x1 {
        D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
    } else if shrinking {
        if shrink_linear {
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC
        } else {
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        }
    } else if mag_linear {
        D2D1_INTERPOLATION_MODE_LINEAR
    } else {
        D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
    }
}

/// The per-level CPU source a frame's uploads read from: level 0 is the
/// master itself (no copy), deeper levels are the [`crate::mip::LevelCache`]
/// box-downscales. `gpu.rs` never owns CPU pixels — the window state does,
/// and hands them in per paint (so a device rebuild re-uploads without a
/// re-decode, and a new image drops the levels with its frame).
pub(crate) trait LevelSource {
    /// The (width, height, BGRA bytes) of `level`, building it on demand
    /// (and caching it). `None` = refused (over the CPU budget).
    fn level(&mut self, level: u32) -> Option<(u32, u32, &[u8])>;

    /// The CPU bytes this source is holding right now — the ledger's
    /// `cpu_source` class.
    fn cpu_bytes(&self) -> u64;

    /// Rebind to the frame generation about to draw, dropping any cached
    /// level of a previous frame (see [`crate::mip::LevelCache::rebind`]).
    fn rebind(&mut self, frame_gen: u64);

    /// Levels built so far (each is a full source pass) — the ledger's
    /// `mip_builds`, and the evidence line's proof that a giant's overview
    /// was really computed.
    fn level_builds(&self) -> u64;

    /// The CPU bytes held by display DERIVATIONS outside this stack (the
    /// GDI face's DIB) — the ledger's `cpu_display` class. The D2D arm
    /// never reads it, but it is real memory in the same process.
    fn display_bytes(&self) -> u64;
}

/// The master + level cache pair the window state hands the D2D arm
/// ([`LevelSource`] over `Surface::master()` and `WindowState::levels`).
/// A window with no image yet has `master: None`; the frame is blank then,
/// so only the level cache's bytes are countable.
pub(crate) struct MasterLevels<'a> {
    pub(crate) master: Option<&'a crate::pixels::PixelFrame>,
    pub(crate) cache: &'a mut crate::mip::LevelCache,
    /// The generation the cache's levels belong to (a new frame clears
    /// them: they were built from the previous frame's pixels).
    pub(crate) frame_gen: u64,
    /// The derived-CPU-copy bytes outside this stack (the GDI face's DIB),
    /// read by the caller from the surface it owns.
    pub(crate) display_bytes: u64,
}

impl LevelSource for MasterLevels<'_> {
    fn level(&mut self, level: u32) -> Option<(u32, u32, &[u8])> {
        let master = self.master?;
        if level == 0 {
            return Some((master.width, master.height, &master.pixels));
        }
        self.cache.get_or_build(
            level,
            master.width,
            master.height,
            self.frame_gen,
            &master.pixels,
        )
    }

    fn cpu_bytes(&self) -> u64 {
        self.master.map_or(0, |m| m.pixels.len() as u64) + self.cache.bytes()
    }

    fn level_builds(&self) -> u64 {
        self.cache.builds
    }

    fn display_bytes(&self) -> u64 {
        self.display_bytes
    }

    fn rebind(&mut self, frame_gen: u64) {
        self.frame_gen = frame_gen;
        self.cache.rebind(frame_gen);
    }
}

/// The letterbox background as a D2D color: u8/255 into the UNORM pipeline
/// rounds back to the same byte on write (f32's 24-bit mantissa keeps the
/// relative error under 2⁻²³ — the byte-exact letterbox argument, design
/// §4), and alpha 1.0 writes 255.
pub(crate) fn bg_color_f(bg: [u8; 3]) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(bg[0]) / 255.0,
        g: f32::from(bg[1]) / 255.0,
        b: f32::from(bg[2]) / 255.0,
        a: 1.0,
    }
}

/// The backend label (About line / status-bar suffix / stderr breadcrumbs):
/// the ticket evidence that says WHICH renderer actually drew.
pub(crate) fn backend_label(hardware: bool) -> &'static str {
    if hardware { "d2d/hw" } else { "d2d/warp" }
}

/// The per-paint diagnostics the window state hands the stack (#82's
/// `-tile`): synced on every paint, so a stack built before the switch was
/// applied (a single-instance handoff) still honors it, and clearing it
/// (absent / 0) restores the natural level/tile decision.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Diagnostics {
    /// Force the tiled path with this grid edge in px, bypassing the
    /// single-bitmap shortcut — the smoke's tiled-vs-untiled channel.
    pub(crate) tile_edge: Option<i32>,
}

/// One dump request: the viewport, its background, and the frame to render
/// (with the plan it was measured for). `frame: None` = the blank
/// letterbox dump.
pub(crate) struct DumpRequest {
    pub(crate) cw: u32,
    pub(crate) ch: u32,
    pub(crate) bg: [u8; 3],
    pub(crate) frame: Option<(u64, u32, u32)>,
    pub(crate) plan: Option<DrawPlan>,
}

/// Everything one D2D frame draws with, gathered from the window state by
/// the caller ([`paint_d2d`] builds it from its own borrow; the dump path
/// receives it pre-built because it runs under the caller's borrow).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrawPlan {
    pub(crate) bg: [u8; 3],
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    pub(crate) rw: i32,
    pub(crate) rh: i32,
    /// The viewport the frame is drawn into — the giant path's tiling
    /// window ([`tile::visible_dest`] intersects the scene rect with it).
    pub(crate) cw: i32,
    pub(crate) ch: i32,
    pub(crate) interp: D2D1_INTERPOLATION_MODE,
}

/// The L0 pixel-exact predicate (design §4): a render whose size equals
/// the source's on both axes — NEAREST over an exact integer rect, the
/// five-piece's resample-free clause. Since #81 retired the mip chain the
/// GDI arm compares against the face's size, which IS the master's — so
/// this predicate is the shared 1:1 test of BOTH arms on their whole
/// domain (the off-1:1 filter selection is #81's table's surface).
pub(crate) fn one_to_one_render(rw: i32, rh: i32, sw: i32, sh: i32) -> bool {
    rw == sw && rh == sh
}

/// Build the plan from the window state — the SAME scene_rect math the GDI
/// blit runs (design §4's geometry clause), against the master's full-size
/// bitmap: since #81 retired the mip chain (ADR 0002 D7) both arms draw
/// from the single full-resolution source and the interpolation comes from
/// the #81 full filter table.
pub(crate) fn draw_plan(
    state: &crate::window::WindowState,
    cw: i32,
    ch: i32,
    sw: i32,
    sh: i32,
) -> DrawPlan {
    let fit = crate::window::fit_policy(state);
    let (dx, dy, rw, rh) = scene_rect(&state.view, fit, cw, ch, sw, sh);
    DrawPlan {
        bg: if state.fullscreen {
            state.config.fullscreen_bg()
        } else {
            state.config.windowed_bg()
        },
        dx,
        dy,
        rw,
        rh,
        cw,
        ch,
        interp: d2d_interp_mode(
            one_to_one_render(rw, rh, sw, sh),
            state.config.shrink_blit_mode == 1,
            state.config.mag_filter == 1,
            rw < sw || rh < sh,
        ),
    }
}

/// The WM_PAINT outcome the router (`window.rs paint_view`) resolves after
/// the paint's state borrow has dropped.
pub(crate) enum PaintOutcome {
    /// Rendered (or blank-letterboxed) and presented.
    Painted,
    /// EndDraw or Present reported DEVICE LOSS — the failure ladder decides
    /// (rebuild same kind → 3-in-10s escalate to WARP → deferred fatal).
    DeviceLost,
    /// A NON-loss failure the ladder cannot fix (a deterministic error:
    /// `D2DERR_NOT_SUPPORTED`, Present `INVALID_CALL` after a bad resize …).
    /// Escalating these would rebuild the same broken stack forever and end
    /// in the deferred fatal — but ADR 0001 files them as USER-level: the
    /// caller degrades the session to GDI (teardown + one-shot flash +
    /// stderr), keeping the old image on screen instead of exiting
    /// (external review AI2 P1: the old two-way misclassification froze the
    /// frame on non-loss Present codes and fatal'd on non-loss EndDraws).
    Unrecoverable { hr: i32 },
}

/// The one HRESULT table both arms share (external review AI2 P1): a code
/// is DEVICE LOSS when a same-spec rebuild can plausibly fix it — the
/// documented loss codes plus `DRIVER_INTERNAL_ERROR`, which the D3D11
/// samples treat as device-gone. Everything else that FAILS is
/// deterministic trouble the ladder cannot outlive. `DXGI_STATUS_OCCLUDED`
/// is a SUCCESS code and never reaches here.
pub(crate) fn is_device_loss(hr: windows::core::HRESULT) -> bool {
    hr == DXGI_ERROR_DEVICE_REMOVED
        || hr == DXGI_ERROR_DEVICE_RESET
        || hr == DXGI_ERROR_DRIVER_INTERNAL_ERROR
        || hr == D2DERR_RECREATE_TARGET
}

// ---------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------

pub(crate) struct GpuStack {
    // Field order IS the drop order (COM wrappers Release in declaration
    // order, design §3; the Drop impl above enforces the first, critical
    // steps as executable code): the target bitmap (the back buffer's D2D
    // reference — ours plus the context's own, unbound in Drop), the
    // uploaded frame bitmap (no back-buffer reference; Releases by its own
    // COM refcount), then the context, the swapchain, the D2D/DXGI device
    // pair, the factory, the D3D device.
    // Both bitmaps are stored as their PARENT interface (ID2D1Image /
    // ID2D1Bitmap): windows-rs 0.62 generates no CanInto upcasts, so the
    // draw/target params take the exact type — one QI at creation, none per
    // frame.
    target: Option<ID2D1Image>,
    bitmap: Option<ID2D1Bitmap>,
    context: ID2D1DeviceContext,
    /// The context's render-target base, cached because
    /// `PushAxisAlignedClip`/`PopAxisAlignedClip` live there (the device
    /// context inherits them but windows-rs 0.62 generates them on the base
    /// only) — one QI at creation instead of one per tile.
    render_target: ID2D1RenderTarget,
    swapchain: IDXGISwapChain1,
    d2d_device: ID2D1Device,
    dxgi_device: IDXGIDevice,
    factory: ID2D1Factory1,
    d3d_device: ID3D11Device,
    /// The effective backend label (About line / status suffix / stderr).
    pub(crate) backend: &'static str,
    /// The largest single bitmap this device can create (runtime query —
    /// never the hardcoded 16384, design §3-6). The giant path reads it
    /// per paint: a level whose dimensions fit is drawn as ONE bitmap, a
    /// larger one is tiled (#82).
    max_bitmap: u32,
    /// The (frame_gen, level, w, h) quartet resident in `bitmap`; a paint
    /// whose quartet differs re-uploads from the CPU level source (no
    /// re-decode, design §5).
    uploaded: Option<(u64, u32, u32, u32)>,
    /// The frame generation the tile cache belongs to: a new generation
    /// (a new image, a rotate, an edit) drops every tile — they can never
    /// be drawn again.
    tile_gen: Option<u64>,
    /// The resident tiles' bitmaps, keyed like the LRU beside them.
    tiles: Vec<(crate::tile::TileKey, ID2D1Bitmap)>,
    /// The tile LRU's policy state (bytes, recency; the objects live in
    /// `tiles`) — the same pure [`crate::tile::Lru`] the plan math uses.
    tile_lru: crate::tile::Lru<crate::tile::TileKey>,
    /// The GPU-resident cap derived from `QueryVideoMemoryInfo` at build
    /// ([`crate::tile::budget_cap`]).
    cap_bytes: u64,
    /// `-tile <edge>` (diagnostic): force the tiled path with this grid
    /// edge, bypassing the single-bitmap shortcut — the smoke's
    /// tiled-vs-untiled comparison channel.
    forced_edge: Option<i32>,
    /// The frame's prepared draw list (built by [`GpuStack::prepare`],
    /// consumed by the scene pass) and the levels the stats line reports.
    scene: Scene,
    /// The byte ledger (ticket's 分类记账) — observable at close.
    pub(crate) ledger: crate::tile::MemLedger,
    // NOTE: the device-loss timestamps do NOT live on the stack — the
    // ladder rebuilds the stack on every loss, and history dying with it
    // would make the 3-in-10s escalation unreachable. They sit on the
    // window state (`gpu_failures`), surviving rebuilds (design §7).
}

/// The frame's prepared draw list: what the scene pass draws after `Clear`.
#[derive(Debug, Default)]
enum Scene {
    /// Nothing (a degenerate frame): the clear IS the frame.
    #[default]
    Clear,
    /// One bitmap of `level` over the whole scene rect (level 0 = the plain
    /// #80/#81 path, level ≥ 1 = a giant's prefiltered overview).
    Base { level: u32, w: u32, h: u32 },
    /// The giant path: haloed tiles, each clipped to its logical interior.
    Tiles {
        level: u32,
        quads: Vec<crate::tile::TileRequest>,
    },
}

/// The driver's video-memory budget for this process — DXGI's
/// `QueryVideoMemoryInfo` on the LOCAL segment, which is what the driver
/// reserves for us specifically (not the adapter's total VRAM). `None`
/// when the adapter is not an `IDXGIAdapter3` (pre-1709 or a wrapper) or
/// the query fails: the caller then runs on [`crate::tile::SELF_CAP_BYTES`].
/// A missing budget costs a conservative cap, never a renderer.
fn video_memory_budget(dxgi_device: &IDXGIDevice) -> Option<u64> {
    // SAFETY: read-only parent query on the live adapter.
    let adapter = unsafe { dxgi_device.GetAdapter() }.ok()?;
    let adapter3: IDXGIAdapter3 = adapter.cast().ok()?;
    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
    // SAFETY: node 0 always exists on an adapter D3D11 handed us; the out
    // struct is a valid local.
    unsafe { adapter3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) }.ok()?;
    Some(info.Budget)
}

/// The bitmap properties all three UNORM surfaces share (the 1:1
/// five-piece #3): `B8G8R8A8_UNORM` + alpha IGNORE — an `_SRGB` variant
/// would linearize the bytes and break the ±0 tolerance; DPI 96 keeps the
/// metadata honest (unit mode PIXELS ignores it for math). `options`
/// varies per call site.
fn bitmap_properties(options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        colorContext: ManuallyDrop::new(None),
    }
}

/// Build the whole stack (design §3's chain, in order). Returns the stack
/// plus the EFFECTIVE renderer kind (auto resolves to hardware or WARP by
/// what actually created) — the caller stores it for same-kind runtime
/// rebuilds. Errors are environment diagnoses (strings): the caller
/// degrades to GDI, never fatals.
pub(crate) fn create(
    view: HWND,
    top: HWND,
    request: RendererKind,
) -> Result<(GpuStack, RendererKind), String> {
    // 1. The D3D device. The request picks the driver ladder: `warp` and
    //    `d2d` are single-driver diagnostics (d2d's failure goes straight
    //    to GDI, never WARP — design §2); `auto` retries on WARP.
    let (d3d_device, effective) = match request {
        RendererKind::Warp => (create_d3d_device(true)?, RendererKind::Warp),
        RendererKind::D2d => (create_d3d_device(false)?, RendererKind::D2d),
        _ => match create_d3d_device(false) {
            Ok(device) => (device, RendererKind::D2d),
            Err(hw_error) => {
                let device = create_d3d_device(true)
                    .map_err(|warp_error| format!("hardware: {hw_error}; warp: {warp_error}"))?;
                (device, RendererKind::Warp)
            }
        },
    };
    // 2. DXGI device → D2D factory → D2D device → device context (design
    //    §3-2). SINGLE_THREADED: every object here lives on the UI thread
    //    only (ADR 0002 D8).
    // (cast is a safe QI in windows-core 0.62 — no unsafe, no label.)
    let dxgi_device: IDXGIDevice = d3d_device
        .cast()
        .map_err(|e| format!("cast to IDXGIDevice failed: {e}"))?;
    // The generic D2D1CreateFactory QIs the requested interface — ask for
    // Factory1 directly (CreateDevice lives there, not on the base
    // factory).
    // SAFETY: process-wide factory creation taking only the enum + options
    // None; the result is checked.
    let factory: ID2D1Factory1 =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }
            .map_err(|e| format!("D2D1CreateFactory failed: {e}"))?;
    // SAFETY: both operands are live; the result is checked.
    let d2d_device = unsafe { factory.CreateDevice(&dxgi_device) }
        .map_err(|e| format!("D2D factory CreateDevice failed: {e}"))?;
    // SAFETY: the device is live; the options enum is a plain value.
    let context: ID2D1DeviceContext =
        unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }
            .map_err(|e| format!("CreateDeviceContext failed: {e}"))?;
    // 3. The flip swapchain on the viewport child (design §3-3): two
    //    buffers, FLIP_DISCARD, no scaling — the D4 posture. Width/height 0
    //    take the target window's current client size.
    // SAFETY: read-only adapter query on the live device.
    let adapter =
        unsafe { dxgi_device.GetAdapter() }.map_err(|e| format!("GetAdapter failed: {e}"))?;
    // SAFETY: read-only parent query on the live adapter.
    let dxgi_factory: IDXGIFactory2 = unsafe { adapter.GetParent() }
        .map_err(|e| format!("GetParent(IDXGIFactory2) failed: {e}"))?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: 0,
        Height: 0,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM, // five-piece #3: never _SRGB
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: 0,
    };
    // SAFETY: the factory, device and target window are live; `desc`
    // outlives the call; the two None slots (fullscreen desc, restrict-
    // to-output) are the documented windowed form.
    let swapchain =
        unsafe { dxgi_factory.CreateSwapChainForHwnd(&d3d_device, view, &desc, None, None) }
            .map_err(|e| format!("CreateSwapChainForHwnd failed: {e}"))?;
    // 4. Alt+Enter would race the viewer's own fullscreen toggle (design
    //    §3-4) — and the association counts per TOP-LEVEL window, hence
    //    `top`, not the child.
    // SAFETY: both windows are live on this thread.
    unsafe { dxgi_factory.MakeWindowAssociation(top, DXGI_MWA_NO_ALT_ENTER) }
        .map_err(|e| format!("MakeWindowAssociation failed: {e}"))?;
    // 5. The target bitmap over the swapchain's buffer 0 (design §3-5),
    //    then the context posture: unit mode PIXELS once (dest coordinates
    //    are physical pixels), ALIASED + COPY for the pixel-exact contract.
    //    SetTransform is NEVER called anywhere in this arm — the identity
    //    audit is a grep, design §4-2.
    // SAFETY: read-only buffer query on the live swapchain.
    let surface: IDXGISurface = unsafe { swapchain.GetBuffer(0) }
        .map_err(|e| format!("swapchain GetBuffer(0) failed: {e}"))?;
    // SAFETY: the surface and context are live; the properties struct is a
    // stack temporary outliving the call.
    let target_bitmap = unsafe {
        context.CreateBitmapFromDxgiSurface(
            &surface,
            Some(&bitmap_properties(
                D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            )),
        )
    }
    .map_err(|e| format!("CreateBitmapFromDxgiSurface failed: {e}"))?;
    // SetTarget takes the parent ID2D1Image (no auto upcast in 0.62).
    let target: ID2D1Image = target_bitmap
        .cast()
        .map_err(|e| format!("cast target to ID2D1Image failed: {e}"))?;
    drop(target_bitmap); // the Image reference keeps the object alive
    // SAFETY: plain mode setters and the target bind on the live context.
    unsafe {
        context.SetUnitMode(D2D1_UNIT_MODE_PIXELS);
        context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        context.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_COPY);
        context.SetTarget(Some(&target));
    }
    // 6. The giant path's bound: runtime query (design §3-6), plus the
    //    video-memory budget the tile LRU's cap derives from (#82).
    // SAFETY: pure size query on the live context.
    let max_bitmap = unsafe { context.GetMaximumBitmapSize() };
    // The clip primitives live on the render-target base (see the struct
    // field's note) — one QI here, none per tile.
    let render_target: ID2D1RenderTarget = context
        .cast()
        .map_err(|e| format!("cast context to ID2D1RenderTarget failed: {e}"))?;
    let budget = video_memory_budget(&dxgi_device);
    let cap_bytes = crate::tile::budget_cap(budget);
    eprintln!(
        "riviv: d2d max_bitmap={max_bitmap} tile cap={cap_bytes} (dxgi budget={})",
        match budget {
            Some(b) => b.to_string(),
            None => "unavailable".to_string(),
        }
    );
    let stack = GpuStack {
        target: Some(target),
        bitmap: None,
        context,
        render_target,
        swapchain,
        d2d_device,
        dxgi_device,
        factory,
        d3d_device,
        backend: backend_label(effective != RendererKind::Warp),
        max_bitmap,
        uploaded: None,
        tile_gen: None,
        tiles: Vec::new(),
        tile_lru: crate::tile::Lru::new(cap_bytes),
        cap_bytes,
        // The `-tile` diagnostic arrives at paint time (apply_diagnostics),
        // not here: the window state owns it and syncs it per paint.
        forced_edge: None,
        scene: Scene::Clear,
        ledger: crate::tile::MemLedger {
            cap: cap_bytes,
            ..crate::tile::MemLedger::default()
        },
    };
    Ok((stack, effective))
}

/// The D3D11 device half of the creation chain (design §3-1). BGRA_SUPPORT
/// is mandatory — D2D interop requires it. The debug layer is deliberately
/// NOT requested: the shipped exe carries no SDK debug binaries contract.
fn create_d3d_device(warp: bool) -> Result<ID3D11Device, String> {
    let driver = if warp {
        D3D_DRIVER_TYPE_WARP
    } else {
        D3D_DRIVER_TYPE_HARDWARE
    };
    let name = if warp { "warp" } else { "hardware" };
    let mut device: Option<ID3D11Device> = None;
    // SAFETY: the out-pointer is a valid local; the None adapter + driver
    // enum is the standard creation form; feature levels None = the
    // default 11_0→9_1 ladder; the immediate context is not needed (all
    // drawing routes through D2D).
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .map_err(|e| format!("D3D11CreateDevice({name}) failed: {e}"))?;
    device.ok_or_else(|| format!("D3D11CreateDevice({name}) returned a null device"))
}

impl GpuStack {
    /// (Re)create the base bitmap when the displayed `(gen, level, w, h)`
    /// differs from the resident one — the CPU level's bytes upload in the
    /// same step (one CreateBitmap with source data, no intermediate
    /// surface, design §5). The pitch is the tightly-packed invariant
    /// `width * 4` (pixels.rs). Level 0 is the master; level ≥ 1 is a
    /// giant's prefiltered overview.
    fn ensure_base(
        &mut self,
        frame_gen: u64,
        level: u32,
        wide: u32,
        high: u32,
        pixels: &[u8],
    ) -> Result<(), String> {
        if self.uploaded == Some((frame_gen, level, wide, high)) {
            return Ok(());
        }
        // Drop the old bitmap first: the context holds no other reference
        // and the creation below copies synchronously.
        self.bitmap = None;
        self.uploaded = None;
        let pitch = wide
            .checked_mul(4)
            .ok_or_else(|| format!("frame {wide}x{high} pitch overflows"))?;
        debug_assert_eq!(pixels.len(), wide as usize * high as usize * 4);
        // SAFETY: `pixels` holds exactly wide*high*4 readable bytes (the
        // master/level invariant, tightly packed top-down) and outlives
        // this synchronous copy; the properties struct is a valid stack
        // temporary.
        let bitmap = unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: wide,
                    height: high,
                },
                Some(pixels.as_ptr().cast()),
                pitch,
                &bitmap_properties(D2D1_BITMAP_OPTIONS_NONE),
            )
        }
        .map_err(|e| format!("D2D CreateBitmap({wide}x{high}) failed: {e}"))?;
        // DrawBitmap takes the parent ID2D1Bitmap (no auto upcast in 0.62)
        // — one QI per upload, none per frame.
        let bitmap_base: ID2D1Bitmap = bitmap
            .cast()
            .map_err(|e| format!("cast frame bitmap to ID2D1Bitmap failed: {e}"))?;
        drop(bitmap); // the base-interface reference keeps the object alive
        self.bitmap = Some(bitmap_base);
        self.uploaded = Some((frame_gen, level, wide, high));
        Ok(())
    }

    /// Copy one tile's haloed source rectangle out of the level's tightly
    /// packed CPU pixels into a tight staging buffer and upload it. The
    /// staging copy is the `in-flight` byte class; it is released before
    /// this returns (its peak is what the ledger keeps).
    fn upload_tile(
        &mut self,
        quad: &crate::tile::TileRequest,
        level_pixels: &[u8],
        level_w: u32,
        level_h: u32,
    ) -> Result<ID2D1Bitmap, String> {
        let wide = quad.src.w as u32;
        let high = quad.src.h as u32;
        if wide == 0 || high == 0 {
            return Err("a zero-area tile has no bitmap".into());
        }
        let row_bytes = wide as usize * 4;
        let pitch = level_w as usize * 4;
        if quad.src.right() as u32 > level_w || quad.src.bottom() as u32 > level_h {
            return Err(format!(
                "tile source {:?} escapes the {level_w}x{level_h} level",
                quad.src
            ));
        }
        let mut staging = vec![0u8; row_bytes * high as usize];
        for row in 0..high as usize {
            let src_off = (quad.src.y as usize + row) * pitch + quad.src.x as usize * 4;
            let dst_off = row * row_bytes;
            let Some(src_row) = level_pixels.get(src_off..src_off + row_bytes) else {
                return Err(format!("tile source row {row} is outside the level buffer"));
            };
            staging[dst_off..dst_off + row_bytes].copy_from_slice(src_row);
        }
        self.ledger.note_inflight(staging.len() as u64);
        // SAFETY: `staging` holds exactly wide*high*4 readable bytes (filled
        // row by row above) and outlives this synchronous copy; the
        // properties struct is a valid stack temporary.
        let bitmap = match unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: wide,
                    height: high,
                },
                Some(staging.as_ptr().cast()),
                row_bytes as u32,
                &bitmap_properties(D2D1_BITMAP_OPTIONS_NONE),
            )
        } {
            Ok(bitmap) => bitmap,
            Err(e) => {
                // The staging buffer dies with this call either way; the
                // ledger must not keep counting it (external review AI1
                // P2-4: a failed creation used to leave `inflight` stuck at
                // the staging size for the rest of the session).
                self.ledger.note_inflight(0);
                return Err(format!("D2D tile CreateBitmap({wide}x{high}) failed: {e}"));
            }
        };
        // Dropped here, not at the end of the function: the staging buffer
        // is dead the moment the (synchronous) copy returned.
        drop(staging);
        self.ledger.note_inflight(0);
        let bitmap_base: ID2D1Bitmap = bitmap
            .cast()
            .map_err(|e| format!("cast tile bitmap to ID2D1Bitmap failed: {e}"))?;
        drop(bitmap);
        Ok(bitmap_base)
    }

    /// Build this frame's draw list: pick the level/form ([`tile::plan_frame`]),
    /// upload whatever is missing (the base bitmap, or the tiles that are
    /// not resident), and record the ledger. Runs BEFORE BeginDraw: every
    /// device-side allocation happens here, so the scene pass itself is
    /// only draw commands.
    ///
    /// `forced_edge` is the `-tile` diagnostic, synced from the window
    /// state per paint — so it also reaches a stack built before the
    /// switch was applied (a single-instance handoff), and `None`/0 always
    /// means the natural level/tile decision.
    pub(crate) fn prepare(
        &mut self,
        frame_gen: u64,
        master: (u32, u32),
        plan: &DrawPlan,
        diag: Diagnostics,
        src: &mut dyn LevelSource,
    ) -> Result<(), String> {
        self.forced_edge = diag.tile_edge.filter(|edge| *edge > 0);
        // A new frame generation invalidates every tile and every cached
        // CPU level (their pixels can never be drawn again).
        src.rebind(frame_gen);
        if self.tile_gen != Some(frame_gen) {
            for key in self.tile_lru.clear() {
                self.tiles.retain(|(k, _)| *k != key);
            }
            self.tile_gen = Some(frame_gen);
        }
        let dest = crate::tile::Rect::new(plan.dx, plan.dy, plan.rw, plan.rh);
        let viewport = crate::tile::Rect::new(0, 0, plan.cw, plan.ch);
        let frame_plan = {
            let lru = &self.tile_lru;
            let resident = move |key: &crate::tile::TileKey| lru.contains(key);
            crate::tile::plan_frame(
                frame_gen,
                crate::tile::FrameGeometry {
                    master: (master.0 as i32, master.1 as i32),
                    render: (plan.rw, plan.rh),
                    dest,
                    viewport,
                },
                self.max_bitmap,
                self.cap_bytes,
                // The CPU level budget: the level bitmap is materialized
                // before upload, so a level the cache would refuse must
                // deepen here instead of blanking the frame later.
                crate::mip::LEVEL_CACHE_BYTES,
                self.forced_edge, // synced from the window state per paint
                &resident,
            )
        };
        match frame_plan {
            crate::tile::FramePlan::Blank => {
                // A stale base bitmap must not stay device-resident behind a
                // frame that does not draw it (the ledger reports tiles only).
                self.bitmap = None;
                self.uploaded = None;
                self.scene = Scene::Clear;
            }
            crate::tile::FramePlan::Base { level } => {
                let (wide, high, pixels) = src
                    .level(level)
                    .ok_or_else(|| format!("level {level} source is unavailable"))?;
                self.ensure_base(frame_gen, level, wide, high, pixels)?;
                self.scene = Scene::Base {
                    level,
                    w: wide,
                    h: high,
                };
            }
            crate::tile::FramePlan::Tiles { level, tiles } => {
                // Same for the tiled form: the tiles ARE the source here, and a
                // level bitmap left from an earlier zoom would inflate real VRAM
                // without showing up in `gpu_resident`.
                self.bitmap = None;
                self.uploaded = None;
                let (level_w, level_h, pixels) = src
                    .level(level)
                    .ok_or_else(|| format!("level {level} tile source is unavailable"))?;
                let mut missing = 0usize;
                let mut last_error: Option<String> = None;
                // Pass 1: mark every already-resident tile of THIS frame hot
                // before any insert can evict — an insert then only reclaims
                // NON-frame (cold) entries. Without this, panning towards
                // decreasing tile indices could insert new tiles whose
                // eviction of the not-yet-touched still-visible ones left
                // letterbox holes mid-image (external review AI1 P2-1: the
                // plan's `frame_bytes <= cap` guarantee is only sound with
                // this recency fixup; the forced `-tile` diagnostic bypasses
                // the guarantee and may partially cover by design).
                for quad in &tiles {
                    self.tile_lru.touch(&quad.key);
                }
                // Pass 2: upload what is missing.
                for quad in &tiles {
                    if self.tile_lru.touch(&quad.key) {
                        continue;
                    }
                    match self.upload_tile(quad, pixels, level_w, level_h) {
                        Ok(bitmap) => {
                            let bytes = quad.bytes();
                            for key in self.tile_lru.insert(quad.key, bytes) {
                                self.tiles.retain(|(k, _)| *k != key);
                                self.ledger.tile_evictions += 1;
                            }
                            if self.tile_lru.contains(&quad.key) {
                                self.tiles.push((quad.key, bitmap));
                                self.ledger.tile_uploads += 1;
                            } else {
                                // Refused: a single tile larger than the
                                // whole cap. Its column shows the cleared
                                // background this frame (the fallback the
                                // ticket's ladder is meant to keep rare).
                                missing += 1;
                            }
                        }
                        Err(e) => {
                            // The per-tile detail goes to the summary line
                            // below: a persistent failure would otherwise print
                            // once per tile per paint (animations included).
                            last_error = Some(e);
                            missing += 1;
                        }
                    }
                }
                // Post-pass verification (external review AI2, P2-1 side-note):
                // a tile uploaded EARLIER in this frame can still have been
                // evicted by a later insert on the forced-diagnostic path —
                // count those too, so the evidence line does not underreport
                // the holes the draw will actually show.
                let resident_now = tiles
                    .iter()
                    .filter(|q| self.tiles.iter().any(|(k, _)| *k == q.key))
                    .count();
                missing += tiles.len() - resident_now;
                if missing > 0 {
                    eprintln!(
                        "riviv: {missing} of {} tiles are not resident this frame{}",
                        tiles.len(),
                        match &last_error {
                            Some(e) => format!(" (last error: {e})"),
                            None => String::new(),
                        }
                    );
                }
                self.scene = Scene::Tiles {
                    level,
                    quads: tiles,
                };
            }
        }
        self.ledger.cpu_source = src.cpu_bytes();
        self.ledger.cpu_display = src.display_bytes();
        self.ledger.mip_builds = src.level_builds();
        // The base class counts the CURRENT plan's level bitmap: a tiled
        // frame holds no base (the tiles ARE the source), while an overview
        // frame's cost is exactly that bitmap — reading `uploaded` instead
        // would report a stale level from an earlier frame of the session.
        self.ledger.gpu_base = match &self.scene {
            Scene::Base { level, w, h } if *level > 0 => {
                crate::tile::bgra_bytes(*w as i64 * *h as i64)
            }
            _ => 0,
        };
        self.ledger
            .note_gpu(self.ledger.gpu_base + self.tile_lru.total_bytes());
        self.ledger.cap = self.cap_bytes;
        Ok(())
    }

    /// One blank-letterbox frame: BeginDraw → Clear → EndDraw → Present.
    /// The degenerate-image arm of the paint (and the upload-failure
    /// degrade: the previous frame must not linger behind a failed adopt).
    /// BeginDraw reports nothing (void); EndDraw is the loss channel.
    /// One BeginDraw→Clear[→DrawBitmap]→EndDraw pass — the ONE scene body
    /// the present paths and the dump share (external review AI2: the dump
    /// used to duplicate the sequence, so a paint-path drift would have
    /// been invisible to the L0 channel). The EndDraw HRESULT is the sole
    /// error channel (BeginDraw/DrawBitmap report nothing themselves).
    fn draw_pass(
        &mut self,
        bg: [u8; 3],
        plan: Option<&DrawPlan>,
    ) -> Result<(), windows::core::Error> {
        // SAFETY: the context and (when the scene draws) the uploaded
        // bitmaps are live (the stack holds them; `prepare` only ever
        // replaces a bitmap while no draw is running); every
        // rectangle/parameter outlives the calls; nothing pumps. The tile
        // loop's `PushAxisAlignedClip`/`PopAxisAlignedClip` calls are
        // paired one-for-one inside it, so the context never reaches
        // `EndDraw` with a clip still on the stack (D2D rejects that).
        unsafe {
            self.context.BeginDraw();
            let color = bg_color_f(bg);
            self.context.Clear(Some(&color));
            if let Some(plan) = plan
                && plan.rw > 0
                && plan.rh > 0
            {
                // i32 → f32 is exact in this range (the integer-rect
                // five-piece clause keeps the values small).
                let dest = D2D_RECT_F {
                    left: plan.dx as f32,
                    top: plan.dy as f32,
                    right: (plan.dx + plan.rw) as f32,
                    bottom: (plan.dy + plan.rh) as f32,
                };
                match &self.scene {
                    Scene::Clear => {}
                    Scene::Base { w, h, .. } => {
                        if let Some(bitmap) = self.bitmap.as_ref() {
                            let src = D2D_RECT_F {
                                left: 0.0,
                                top: 0.0,
                                right: *w as f32,
                                bottom: *h as f32,
                            };
                            // Full source rect, no perspective: the dest
                            // rect carries the whole view math (unit PIXELS
                            // + identity transform).
                            self.context.DrawBitmap(
                                bitmap,
                                Some(&dest),
                                1.0,
                                plan.interp,
                                Some(&src),
                                None,
                            );
                        }
                    }
                    Scene::Tiles { quads, .. } => {
                        for quad in quads {
                            let Some(bitmap) = self
                                .tiles
                                .iter()
                                .find(|(key, _)| *key == quad.key)
                                .map(|(_, bitmap)| bitmap)
                            else {
                                continue; // not resident: the clear stands
                            };
                            let clip = D2D_RECT_F {
                                left: quad.clip.x as f32,
                                top: quad.clip.y as f32,
                                right: quad.clip.right() as f32,
                                bottom: quad.clip.bottom() as f32,
                            };
                            // ALIASED: a half-covered clip edge pixel would
                            // blend with the neighbour tile's write and
                            // break the exact partition.
                            self.render_target
                                .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                            // Sub-pixel, never truncated: the resampler derives
                            // its scale/offset from these edges, and an integer
                            // rect would drift the phase per tile (the ±1 shading
                            // difference the plan math measures).
                            let tile_dest = D2D_RECT_F {
                                left: quad.dest.x,
                                top: quad.dest.y,
                                right: quad.dest.right(),
                                bottom: quad.dest.bottom(),
                            };
                            let tile_src = D2D_RECT_F {
                                left: 0.0,
                                top: 0.0,
                                right: quad.src.w as f32,
                                bottom: quad.src.h as f32,
                            };
                            // The haloed source maps through the SAME
                            // global projection as the untiled draw, so the
                            // resampler's phase matches; the clip hides the
                            // halo's edge-clamped samples.
                            self.context.DrawBitmap(
                                bitmap,
                                Some(&tile_dest),
                                1.0,
                                plan.interp,
                                Some(&tile_src),
                                None,
                            );
                            self.render_target.PopAxisAlignedClip();
                        }
                    }
                }
            }
            self.context.EndDraw(None, None)
        }
    }

    /// Classify one EndDraw failure through the shared table: loss codes
    /// take the ladder, everything else is session-degrade (the old code
    /// lumped ALL failures into the ladder — a deterministic error would
    /// have rebuilt, escalated to WARP and finally fatal'd what ADR 0001
    /// files as a user-level degrade; external review AI2 P1).
    fn enddraw_outcome(&self, e: windows::core::Error) -> PaintOutcome {
        let hr = e.code();
        if is_device_loss(hr) {
            eprintln!("riviv: EndDraw failed ({e}) — device loss ladder");
            PaintOutcome::DeviceLost
        } else {
            eprintln!("riviv: EndDraw failed ({e}) — degrading the session to gdi");
            PaintOutcome::Unrecoverable { hr: hr.0 }
        }
    }

    /// The #82 evidence line for the close path: the byte ledger plus this
    /// frame's level/tile count — `None` when the tile path never ran (an
    /// ordinary session's stderr stays clean).
    pub(crate) fn stats_line(&self) -> Option<String> {
        if self.ledger.tile_uploads == 0
            && self.ledger.mip_builds == 0
            && self.forced_edge.is_none()
        {
            return None;
        }
        let (level, tiles) = match &self.scene {
            Scene::Tiles { level, quads } => (*level, quads.len()),
            Scene::Base { level, .. } => (*level, 0),
            Scene::Clear => (0, 0),
        };
        Some(self.ledger.stats_line(level, tiles))
    }

    /// One blank-letterbox frame: the scene pass plus Present.
    /// The degenerate-image arm of the paint (and the upload-failure
    /// degrade: the previous frame must not linger behind a failed adopt).
    fn present_clear(&mut self, bg: [u8; 3]) -> PaintOutcome {
        match self.draw_pass(bg, None) {
            Ok(()) => present(&self.swapchain),
            Err(e) => self.enddraw_outcome(e),
        }
    }

    /// One frame: the scene pass (the letterbox IS the Clear — the GDI
    /// arm's strip concept does not exist here, design §4) plus
    /// Present(0,0). The rw>0&&rh>0 guard lives in the shared pass.
    fn draw_frame(&mut self, plan: &DrawPlan) -> PaintOutcome {
        match self.draw_pass(plan.bg, Some(plan)) {
            Ok(()) => present(&self.swapchain),
            Err(e) => self.enddraw_outcome(e),
        }
    }

    /// The WM_SIZE chain (design §6): release the target (the ONLY back-
    /// buffer reference — the uploaded frame bitmap survives untouched),
    /// resize the buffers, rebuild the target. Zero sizes (minimized) skip
    /// and keep the old buffers for the restore.
    pub(crate) fn resize(&mut self, wide: u32, high: u32) -> Result<(), String> {
        if wide == 0 || high == 0 {
            return Ok(());
        }
        // Release the target before ResizeBuffers — the documented
        // sequence requires EVERY back-buffer reference gone: the context's
        // own (SetTarget(None) below) and ours (the field drop). The
        // uploaded frame bitmap holds no back-buffer reference and stays.
        // SAFETY: plain unbind on the live context.
        unsafe {
            self.context.SetTarget(None);
        }
        self.target = None;
        // SAFETY: the swapchain is live; buffer count 0 + size 0 + UNKNOWN
        // keep the current buffer count and the window's client size and
        // format; flags 0.
        unsafe {
            self.swapchain
                .ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
        }
        .map_err(|e| format!("ResizeBuffers({wide}x{high}) failed: {e}"))?;
        // SAFETY: read-only buffer query on the live swapchain.
        let surface: IDXGISurface = unsafe { self.swapchain.GetBuffer(0) }
            .map_err(|e| format!("resize GetBuffer(0) failed: {e}"))?;
        // SAFETY: the surface is live and buffer-owned; the properties
        // struct is a stack temporary outliving the call.
        let target_bitmap = unsafe {
            self.context.CreateBitmapFromDxgiSurface(
                &surface,
                Some(&bitmap_properties(
                    D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                )),
            )
        }
        .map_err(|e| format!("resize CreateBitmapFromDxgiSurface failed: {e}"))?;
        // SetTarget takes the parent ID2D1Image (no auto upcast in 0.62).
        let target: ID2D1Image = target_bitmap
            .cast()
            .map_err(|e| format!("resize cast target to ID2D1Image failed: {e}"))?;
        drop(target_bitmap);
        // SAFETY: plain target bind on the live context.
        unsafe {
            self.context.SetTarget(Some(&target));
        }
        self.target = Some(target);
        Ok(())
    }

    /// The D2D dump channel (design §9): render the CURRENT scene into the
    /// target — no Present, so a never-shown window dumps identically —
    /// then copy the target into a CPU-readable staging bitmap, Map it and
    /// hand the RGBA bytes back. Runs under the caller's window-state
    /// borrow: the plan and the frame arrive as parameters, this method
    /// never re-enters state_of. Since #82 the giant path renders THROUGH
    /// here too (tiles), so the dump channel covers the whole domain — the
    /// GDI fallback stays for a dead/absent stack only.
    pub(crate) fn dump(
        &mut self,
        req: DumpRequest,
        diag: Diagnostics,
        src: &mut dyn LevelSource,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let DumpRequest {
            cw,
            ch,
            bg,
            frame,
            plan,
        } = req;
        self.forced_edge = diag.tile_edge.filter(|edge| *edge > 0);
        if cw == 0 || ch == 0 {
            return Err(format!("viewport is {cw}x{ch} — nothing to dump"));
        }
        // The readback copies cw×ch out of the TARGET — a stale smaller
        // back buffer (a resize that failed and latched) would zero-fill
        // the staging bitmap and sail through with exit 0 (external review
        // AI2). Validate against the swapchain's actual buffer size first;
        // a mismatch is an error the caller answers with the GDI channel.
        // SAFETY: read-only description query on the live swapchain.
        let desc = unsafe { self.swapchain.GetDesc1() }
            .map_err(|e| format!("dump GetDesc1 failed: {e}"))?;
        if desc.Width != cw || desc.Height != ch {
            return Err(format!(
                "swapchain is {}x{} but the viewport is {cw}x{ch}",
                desc.Width, desc.Height
            ));
        }
        // A frame (and its plan — they arrive together) prepares the draw
        // list through the same ladder the paint uses; nothing at all means
        // the blank letterbox dump.
        match (frame, plan) {
            (Some((frame_gen, wide, high)), Some(plan)) => {
                self.prepare(frame_gen, (wide, high), &plan, diag, src)?;
            }
            _ => self.scene = Scene::Clear,
        }
        // The SAME scene pass the present paths run (external review AI2:
        // a duplicated sequence here would let paint-path drift go
        // invisible to the L0 channel) — minus the Present, so a
        // never-shown window dumps identically.
        self.draw_pass(bg, plan.as_ref())
            .map_err(|e| format!("dump EndDraw failed: {e}"))?;
        // The readback staging bitmap: CPU_READ | CANNOT_DRAW, viewport
        // sized, the same UNORM format (design §9).
        // The readback plus the two decoded copies below are this call's
        // in-flight buffers: the ledger's inflight class covers them, and the
        // peak is what survives the call.
        self.ledger
            .note_inflight((cw as u64 * ch as u64 * 4).saturating_mul(2));
        // SAFETY: the context is live; the properties struct outlives the
        // call; the bitmap is created bare (never set as the target).
        let readback = unsafe {
            self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: cw,
                    height: ch,
                },
                None,
                0,
                &bitmap_properties(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )
        }
        .map_err(|e| format!("dump readback CreateBitmap failed: {e}"))?;
        // The context upcasts to its render-target base (the copy SOURCE);
        // the readback bitmap is the destination (cast is a safe QI in
        // windows-core 0.62 — no SAFETY label needed, external review AI2).
        let rt: ID2D1RenderTarget = self
            .context
            .cast()
            .map_err(|e| format!("dump cast to ID2D1RenderTarget failed: {e}"))?;
        let src_rect = D2D_RECT_U {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        };
        // SAFETY: dest point None = (0,0); the source rect covers the whole
        // target (size-validated above); both objects are live.
        unsafe { readback.CopyFromRenderTarget(None, &rt, Some(&src_rect)) }
            .map_err(|e| format!("dump CopyFromRenderTarget failed: {e}"))?;
        // SAFETY: a READ map on a CPU_READ bitmap is the documented access;
        // the mapping is released by the Unmap below before anything drops.
        let mapped = unsafe { readback.Map(D2D1_MAP_OPTIONS_READ) }
            .map_err(|e| format!("dump Map failed: {e}"))?;
        let pitch = mapped.pitch as usize;
        let row_bytes = cw as usize * 4;
        if mapped.bits.is_null() || pitch < row_bytes {
            let mapped_pitch = mapped.pitch;
            // Unmap before failing: the mapping must not outlive the call
            // chain even on the error path.
            // SAFETY: paired with the Map above; nothing reads the bits.
            let _ = unsafe { readback.Unmap() };
            return Err(format!(
                "dump Map gave pitch {mapped_pitch} for a {row_bytes}-byte row"
            ));
        }
        let mut bgra = vec![0u8; row_bytes * ch as usize];
        for (row, dst_row) in bgra.chunks_mut(row_bytes).enumerate() {
            // SAFETY: the guards above pinned pitch ≥ row_bytes and a
            // non-null base; the mapped region holds pitch*ch readable
            // bytes for the lifetime of the map.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    mapped.bits.add(row * pitch),
                    dst_row.as_mut_ptr(),
                    row_bytes,
                );
            }
        }
        // SAFETY: paired with Map above; nothing reads the mapped bits
        // after this point.
        unsafe { readback.Unmap() }.map_err(|e| format!("dump Unmap failed: {e}"))?;
        let mut rgba = vec![0u8; bgra.len()];
        crate::pixels::bgra_to_rgba(&bgra, &mut rgba);
        self.ledger.note_inflight(0);
        Ok((cw, ch, rgba))
    }
}

impl Drop for GpuStack {
    fn drop(&mut self) {
        // Executable teardown order (external review AI2 P2-5: the order
        // used to live only in a comment — any future field inserted with a
        // back-buffer reference would silently break ResizeBuffers' "no
        // buffer references" precondition). Unbind the target first so the
        // CONTEXT's own back-buffer reference is released deterministically,
        // then our two bitmaps (each Releases by its own COM refcount —
        // the uploaded frame bitmap never depended on the context). The
        // remaining fields (context → swapchain → d2d device → dxgi device
        // → factory → d3d device) drop in DECLARATION order, which the
        // language guarantees; they exist so the whole stack dies together
        // (ADR 0002 D8).
        // SAFETY: the context is live; SetTarget(None) is the plain unbind.
        unsafe { self.context.SetTarget(None) };
        self.target = None;
        self.bitmap = None;
        // The device-chain fields are never dereferenced — they exist to
        // HOLD the COM references so the whole stack dies together (ADR
        // 0002 D8); this read is the deliberate keep-alive pin (and keeps
        // dead_code quiet about fields whose only job is existence).
        let _keepalive = (
            &self.d2d_device,
            &self.dxgi_device,
            &self.factory,
            &self.d3d_device,
        );
    }
}

/// Present(0,0) — the interactive/animation posture (no vsync-blocking
/// Present(1), #8's lesson). DXGI_STATUS_OCCLUDED is a SUCCESS code — a
/// benign skip (another window covers us; the next WM_PAINT re-renders,
/// design §13-4: no polling recovery in #80). Loss codes take the ladder;
/// any OTHER failure is deterministic trouble a rebuild cannot fix
/// (`INVALID_CALL` after a bad resize, …) — the old code just logged it
/// and reported Painted, leaving a permanently frozen frame with no
/// recovery path (external review AI2 P1); it now degrades the session to
/// GDI through [`PaintOutcome::Unrecoverable`].
fn present(swapchain: &IDXGISwapChain1) -> PaintOutcome {
    // SAFETY: the shared reference guarantees the swapchain is live; flags
    // 0 = the plain interactive form.
    let hr = unsafe { swapchain.Present(0, DXGI_PRESENT(0)) };
    if is_device_loss(hr) {
        PaintOutcome::DeviceLost
    } else if hr.is_err() {
        eprintln!(
            "riviv: Present returned {:#010x} — degrading the session to gdi",
            hr.0 as u32
        );
        PaintOutcome::Unrecoverable { hr: hr.0 }
    } else {
        PaintOutcome::Painted
    }
}

// ---------------------------------------------------------------------------
// The WM_PAINT arm (routed from view_proc by window.rs paint_view)
// ---------------------------------------------------------------------------

/// The D2D paint (design §4, ADR 0002 D8 verbatim): IsIconic early-exit →
/// BeginPaint (rcPaint ignored — a flip back buffer is discarded, the
/// whole viewport redraws; failure is the GDI arm's fatal) → `prepare`
/// (the plan + its uploads) → BeginDraw/Clear/DrawBitmap/EndDraw/Present →
/// EndPaint → the #76 paint handshake (the render stack's health is
/// irrelevant to the decode worker's first-frame wait). Device losses
/// return as [`PaintOutcome::DeviceLost`] for the router's ladder; nothing
/// in here fatals after BeginPaint.
pub(crate) fn paint_d2d(view: HWND, owner: HWND) -> PaintOutcome {
    // SAFETY: read-only query on the live owner.
    if unsafe { IsIconic(owner) }.as_bool() {
        // The skip still must CONSUME the update region (pre-review P2-1):
        // the animation timer keeps invalidating a minimized window, and a
        // WM_PAINT answered without BeginPaint/ValidateRect leaves the
        // region dirty — the queue regenerates WM_PAINT every idle pass and
        // the paint spins hot until restore. The GDI arm's unconditional
        // BeginPaint validates implicitly (upstream viv.c:4066 too); this
        // is the D2D arm's explicit equivalent.
        // SAFETY: validates our own child's whole client area; no borrow is
        // live.
        unsafe {
            let _ = ValidateRect(Some(view), None);
        }
        // The #76 handshake fires here too (pre-review 3-a): the GDI arm's
        // iconic paint still runs its body and releases the worker's held
        // frame — skipping it here would park the decode at the 5 s cap and
        // delay an animation adopted while minimized (an arm divergence,
        // not a D2D constraint: "rendered" for the handshake's purpose
        // means "the adoption was consumed by a paint", iconic included).
        // SAFETY: the borrow spans the signal take and notify; nothing
        // pumps.
        if let Some(signal) = unsafe { state_of(owner) }.and_then(|s| s.paint_signal.take()) {
            let (lock, cvar) = &*signal;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
        return PaintOutcome::Painted;
    }
    // SAFETY: BeginPaint/EndPaint bracket the whole draw; the state borrow
    // below spans only calls that pump no messages (D2D commands, a mutex
    // notify — the paint-borrow contract, PR #10 P1). The device objects
    // are UI-thread-only (ADR 0002 D8), so no cross-thread hazard exists.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(view, &mut ps);
        if hdc.is_invalid() {
            // Already inside this function's outer unsafe block. System-
            // level failure (ADR 0001) — and no state borrow is live yet,
            // so the fatal modal cannot alias anything.
            let gle = GetLastError().0;
            fatal(&format!("BeginPaint failed (GLE={gle})"));
        }
        let mut client = RECT::default();
        let _ = GetClientRect(view, &mut client);
        let cw = (client.right - client.left).max(1);
        let ch = (client.bottom - client.top).max(1);
        let Some(state) = state_of(owner) else {
            let _ = EndPaint(view, &ps);
            return PaintOutcome::Painted;
        };
        let bg = if state.fullscreen {
            state.config.fullscreen_bg()
        } else {
            state.config.windowed_bg()
        };
        let frame_gen = state.frame_gen;
        // The master facts before the mutable borrow (Copy values — the
        // shared borrow ends here).
        let dims = state.image.as_ref().map(|img| {
            let master = img.surface().master();
            (master.width, master.height)
        });
        // The plan reads the whole state — computed before the mutable gpu
        // borrow (plain data, the borrow ends here).
        let plan = dims.map(|(mw, mh)| (draw_plan(state, cw, ch, mw as i32, mh as i32), mw, mh));
        let Some(gpu) = state.gpu.as_mut() else {
            // Defensive: the router only calls here with a live stack.
            let _ = EndPaint(view, &ps);
            return PaintOutcome::Painted;
        };
        let outcome = match (&plan, state.image.as_ref()) {
            (Some((plan, mw, mh)), Some(image)) => {
                // The CPU level source (master + mip cache) — a disjoint
                // field borrow from `gpu`; the window state owns the
                // pixels, the stack only uploads them (#82).
                let mut levels = MasterLevels {
                    master: Some(image.surface().master()),
                    cache: &mut state.levels,
                    frame_gen,
                    display_bytes: image.surface().face_bytes(),
                };
                let diag = crate::gpu::Diagnostics {
                    tile_edge: state.tile_edge,
                };
                match gpu.prepare(frame_gen, (*mw, *mh), plan, diag, &mut levels) {
                    Ok(()) => gpu.draw_frame(plan),
                    Err(e) => {
                        // Upload trouble: blank this frame (the previous
                        // frame must not linger behind a failed adopt); a
                        // real device-gone condition reports through the
                        // EndDraw/Present channel on a later paint.
                        eprintln!("riviv: d2d frame upload failed, blanking this frame: {e}");
                        gpu.present_clear(bg)
                    }
                }
            }
            _ => gpu.present_clear(bg),
        };
        // The #76 handshake (design §4): fired whether the frame rendered,
        // blanked or degraded — the worker's decode must not stall on the
        // render stack's health (the 5s cap exists for exactly this).
        if let Some(signal) = state.paint_signal.take() {
            let (lock, cvar) = &*signal;
            *lock.lock().unwrap() = true;
            cvar.notify_all();
        }
        let _ = EndPaint(view, &ps);
        outcome
    }
}

/// The top-level HWND behind the viewport child (the MakeWindowAssociation
/// and rebuild paths need the OWNER — the flip association counts per
/// top-level window, design §3-4).
pub(crate) fn owner_of(view: HWND) -> HWND {
    // SAFETY: read-only ancestor query on a live child; a failed query
    // degrades to the null handle (the callers guard it).
    unsafe { GetParent(view) }.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct2D::{
        D2D1_INTERPOLATION_MODE_ANISOTROPIC, D2D1_INTERPOLATION_MODE_CUBIC,
        D2D1_INTERPOLATION_MODE_MULTI_SAMPLE_LINEAR,
    };

    // ---- failure_window (design §7) ----

    #[test]
    fn the_first_two_failures_inside_the_window_recover_in_place() {
        // One loss: rebuild the same kind, no escalation.
        let mut failures = Vec::new();
        assert_eq!(
            failure_window(1_000, false, &mut failures),
            FailureVerdict::None
        );
        assert_eq!(
            failure_window(2_000, false, &mut failures),
            FailureVerdict::None
        );
        assert_eq!(
            failures,
            vec![1_000, 2_000],
            "both timestamps stay recorded"
        );
    }

    #[test]
    fn the_third_failure_inside_the_window_escalates_and_clears() {
        let mut failures = Vec::new();
        let _ = failure_window(1_000, false, &mut failures);
        let _ = failure_window(2_000, false, &mut failures);
        assert_eq!(
            failure_window(3_000, false, &mut failures),
            FailureVerdict::Escalate,
            "3-in-10s switches to WARP"
        );
        assert!(failures.is_empty(), "the escalation starts a fresh count");
    }

    #[test]
    fn an_escalated_warp_stack_that_fails_again_is_fatal() {
        let mut failures = Vec::new();
        let _ = failure_window(1_000, true, &mut failures);
        let _ = failure_window(2_000, true, &mut failures);
        assert_eq!(
            failure_window(3_000, true, &mut failures),
            FailureVerdict::Fatal,
            "WARP is the permanent fallback; failing it repeatedly is the end"
        );
    }

    #[test]
    fn timestamps_outside_the_window_are_pruned() {
        // 9_999 ms after a failure it still counts; at 10_000 it pruned —
        // so 3 failures spread 6s apart never accumulate to an escalation.
        let mut failures = Vec::new();
        let _ = failure_window(0, false, &mut failures);
        assert_eq!(
            failure_window(10_000, false, &mut failures),
            FailureVerdict::None,
            "the t=0 entry is exactly one step past the window"
        );
        assert_eq!(failures, vec![10_000]);
        // The wrapping compare (GetTickCount wraps at 2^32 ms).
        let mut failures = vec![u32::MAX - 100];
        assert_eq!(
            failure_window(300, false, &mut failures),
            FailureVerdict::None,
            "a pre-wrap timestamp is 400ms old across the wrap"
        );
        assert_eq!(failures, vec![u32::MAX - 100, 300]);
    }

    // ---- d2d_interp_mode (#81's full filter table) ----

    #[test]
    fn the_one_to_one_predicate_requires_both_axes_to_match_the_source() {
        // L0's core predicate (pre-review P3-3: pinned directly, not just
        // end-to-end through the smoke): both axes must equal the source —
        // a match on one axis alone is still a filtered render.
        assert!(one_to_one_render(300, 200, 300, 200));
        assert!(!one_to_one_render(300, 201, 300, 200));
        assert!(!one_to_one_render(301, 200, 300, 200));
        assert!(!one_to_one_render(150, 100, 300, 200));
    }

    #[test]
    fn one_to_one_is_always_nearest() {
        // The pixel-exact contract outranks every config value.
        assert_eq!(
            d2d_interp_mode(true, true, true, false),
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        );
        assert_eq!(
            d2d_interp_mode(true, false, false, true),
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        );
    }

    #[test]
    fn shrinking_follows_shrink_blit_mode() {
        assert_eq!(
            d2d_interp_mode(false, true, false, true),
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            "shrink_blit_mode=1 (HALFTONE) draws high-quality cubic — \
             the #81/ADR 0002 D6 upgrade over GDI HALFTONE"
        );
        assert_eq!(
            d2d_interp_mode(false, false, false, true),
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            "shrink_blit_mode=0 draws nearest"
        );
    }

    #[test]
    fn magnifying_follows_mag_filter() {
        assert_eq!(
            d2d_interp_mode(false, false, true, false),
            D2D1_INTERPOLATION_MODE_LINEAR,
            "mag_filter=1 (HALFTONE) draws linear"
        );
        assert_eq!(
            d2d_interp_mode(false, false, false, false),
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            "mag_filter=0 (COLORONCOLOR) draws nearest"
        );
    }

    #[test]
    fn the_draw_bitmap_mode_enum_pins_the_six_d2d1_1_h_values() {
        // The full D2D1_INTERPOLATION_MODE table the DrawBitmap signature
        // takes, value-pinned against d2d1_1.h so a windows-rs
        // regeneration or a wrong-tier constant in the #81 filter table
        // fails loudly instead of shipping a silently different kernel.
        assert_eq!(D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR.0 as u32, 0);
        assert_eq!(D2D1_INTERPOLATION_MODE_LINEAR.0 as u32, 1);
        assert_eq!(D2D1_INTERPOLATION_MODE_CUBIC.0 as u32, 2);
        assert_eq!(D2D1_INTERPOLATION_MODE_MULTI_SAMPLE_LINEAR.0 as u32, 3);
        assert_eq!(D2D1_INTERPOLATION_MODE_ANISOTROPIC.0 as u32, 4);
        assert_eq!(
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC.0 as u32, 5,
            "the shrink Linear tier's kernel"
        );
    }

    // ---- the giant path's device bound (design §5, #82) ----
    //
    // #80's gate (a frame past `GetMaximumBitmapSize` tears the stack down
    // and renders through GDI) is GONE: the D2D arm draws giants itself,
    // as a level bitmap or as tiles. The device bound is now a plan input
    // — `tile::plan_frame`'s `max_bitmap` argument — and its behavior is
    // pinned by tile.rs's level tests (a level at/past the max tiles, one
    // under it is a single bitmap). What remains worth pinning here is the
    // budget the stack derives from it:

    #[test]
    fn the_stack_cap_comes_from_the_driver_budget_with_our_own_ceiling() {
        use crate::tile::{SELF_CAP_BYTES, budget_cap};
        // The stack stores whatever `budget_cap` decided; a huge driver
        // budget must not become an unbounded tile cache, and a tiny one
        // must not starve the viewport.
        assert_eq!(budget_cap(None), SELF_CAP_BYTES);
        assert_eq!(budget_cap(Some(u64::MAX)), SELF_CAP_BYTES);
        assert!(budget_cap(Some(0)) >= crate::tile::MIN_CAP_BYTES);
    }

    // ---- the background color roundtrip (design §4's byte-exact argument) ----

    #[test]
    fn every_background_byte_round_trips_through_the_unorm_pipeline() {
        // u8 → f32(v/255) → UNORM write round(v*255): the mantissa argument
        // in design §4 says this is exact for ALL 256 values — pin it.
        for v in 0u32..=255 {
            let color = bg_color_f([v as u8, 0, 0]);
            assert_eq!((color.r * 255.0).round() as u32, v, "r channel {v}");
            let color = bg_color_f([0, v as u8, 0]);
            assert_eq!((color.g * 255.0).round() as u32, v, "g channel {v}");
            let color = bg_color_f([0, 0, v as u8]);
            assert_eq!((color.b * 255.0).round() as u32, v, "b channel {v}");
        }
        // Alpha is forced opaque: the letterbox writes 255.
        assert_eq!(bg_color_f([0, 0, 0]).a, 1.0);
    }

    // ---- backend labels (§8's evidence channel) ----

    #[test]
    fn backend_labels_distinguish_hardware_from_warp() {
        assert_eq!(backend_label(true), "d2d/hw");
        assert_eq!(backend_label(false), "d2d/warp");
    }

    #[test]
    fn the_shared_loss_table_separates_rebuildable_from_deterministic() {
        // External review AI2 P1: the loss codes (ladder — a same-spec
        // rebuild can plausibly fix them) vs everything else (session
        // degrade to GDI — a deterministic error the ladder would only
        // escalate into a wrong fatal).
        for hr in [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_RESET,
            DXGI_ERROR_DRIVER_INTERNAL_ERROR,
            D2DERR_RECREATE_TARGET,
        ] {
            assert!(is_device_loss(hr), "{:#010x} is a loss code", hr.0 as u32);
        }
        // E_INVALIDARG / D2DERR_NOT_SUPPORTED style failures: NOT loss.
        for hr in [
            windows::core::HRESULT(0x8007_0057_u32 as _), // E_INVALIDARG
            windows::core::HRESULT(0x8899_0001_u32 as _), // D2DERR_NOT_SUPPORTED-ish
        ] {
            assert!(
                !is_device_loss(hr),
                "{:#010x} is not a loss code",
                hr.0 as u32
            );
        }
    }
}
