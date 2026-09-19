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
//! arm (identity audit — grep-provable), exact i32 rects, every surface
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
    D2D1_INTERPOLATION_MODE, D2D1_INTERPOLATION_MODE_LINEAR,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_MAP_OPTIONS_READ, D2D1_PRIMITIVE_BLEND_COPY,
    D2D1_UNIT_MODE_PIXELS, D2D1CreateFactory, ID2D1Bitmap, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Image, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT,
    DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
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

/// The #80 minimal two-tier filter map (#81 owns the full table): a 1:1
/// render is NEAREST unconditionally (the pixel-exact contract); a shrink
/// follows `shrink_blit_mode`, a magnify follows `mag_filter` — the same
/// 0=Nearest/1=Linear values the GDI arms read (config.c:41-42).
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
            D2D1_INTERPOLATION_MODE_LINEAR
        } else {
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        }
    } else if mag_linear {
        D2D1_INTERPOLATION_MODE_LINEAR
    } else {
        D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR
    }
}

/// The giant-image gate (design §5): a frame neither axis of which fits
/// under the device's maximum bitmap size cannot upload — the caller tears
/// the stack down and renders through GDI until a new image arrives.
pub(crate) fn frame_exceeds_max_bitmap(wide: u32, high: u32, max_bitmap: u32) -> bool {
    wide > max_bitmap || high > max_bitmap
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
    pub(crate) interp: D2D1_INTERPOLATION_MODE,
}

/// Build the plan from the window state — the SAME scene_rect math the GDI
/// blit runs (design §4's geometry clause), against the master's full-size
/// bitmap: the D2D arm never participates in the mip chain.
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
        interp: d2d_interp_mode(
            rw == sw && rh == sh,
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
    /// The displayed frame exceeds the device's maximum bitmap size — the
    /// caller tears the stack down (giant-image gate) and re-renders this
    /// frame through GDI.
    GiantFrame { wide: u32, high: u32, max: u32 },
    /// EndDraw or Present reported device loss — the failure ladder decides.
    DeviceLost,
}

// ---------------------------------------------------------------------------
// The stack
// ---------------------------------------------------------------------------

pub(crate) struct GpuStack {
    // Field order IS the drop order (COM wrappers Release in declaration
    // order, design §3): the target bitmap (the back buffer's only D2D
    // reference) first, the uploaded frame bitmap next (no back-buffer
    // reference, but it dies with the context), then the context, the
    // swapchain, the D2D/DXGI device pair, the factory, the D3D device.
    // Both stored as their PARENT interface (ID2D1Image / ID2D1Bitmap):
    // windows-rs 0.62 generates no CanInto upcasts, so the draw/target
    // params take the exact type — one QI at creation, none per frame.
    target: Option<ID2D1Image>,
    bitmap: Option<ID2D1Bitmap>,
    context: ID2D1DeviceContext,
    swapchain: IDXGISwapChain1,
    d2d_device: ID2D1Device,
    dxgi_device: IDXGIDevice,
    factory: ID2D1Factory1,
    d3d_device: ID3D11Device,
    /// The effective backend label (About line / status suffix / stderr).
    pub(crate) backend: &'static str,
    /// The largest single bitmap this device can create (runtime query —
    /// never the hardcoded 16384, design §3-6). The giant-image gate reads
    /// it per paint.
    max_bitmap: u32,
    /// The (frame_gen, w, h) triple resident in `bitmap`; a paint whose
    /// triple differs re-uploads from the master (no re-decode, design §5).
    uploaded: Option<(u64, u32, u32)>,
    // NOTE: the device-loss timestamps do NOT live on the stack — the
    // ladder rebuilds the stack on every loss, and history dying with it
    // would make the 3-in-10s escalation unreachable. They sit on the
    // window state (`gpu_failures`), surviving rebuilds (design §7).
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
    // SAFETY: cast is a pure QI over the freshly built, live device.
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
    // 6. The giant-image gate's bound: runtime query (design §3-6).
    // SAFETY: pure size query on the live context.
    let max_bitmap = unsafe { context.GetMaximumBitmapSize() };
    let stack = GpuStack {
        target: Some(target),
        bitmap: None,
        context,
        swapchain,
        d2d_device,
        dxgi_device,
        factory,
        d3d_device,
        backend: backend_label(effective != RendererKind::Warp),
        max_bitmap,
        uploaded: None,
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
    /// (Re)create the frame bitmap when the displayed (gen, w, h) differs
    /// from the resident one — the master's bytes upload in the same step
    /// (one CreateBitmap with source data, no intermediate surface, design
    /// §5). The pitch is the master invariant width*4 (pixels.rs).
    fn ensure_upload(
        &mut self,
        frame_gen: u64,
        wide: u32,
        high: u32,
        pixels: &[u8],
    ) -> Result<(), String> {
        if self.uploaded == Some((frame_gen, wide, high)) {
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
        // master invariant, tightly packed top-down) and outlives this
        // synchronous copy; the properties struct is a valid stack
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
        self.uploaded = Some((frame_gen, wide, high));
        Ok(())
    }

    /// One blank-letterbox frame: BeginDraw → Clear → EndDraw → Present.
    /// The degenerate-image arm of the paint (and the upload-failure
    /// degrade: the previous frame must not linger behind a failed adopt).
    /// BeginDraw reports nothing (void); EndDraw is the loss channel.
    fn present_clear(&mut self, bg: [u8; 3]) -> PaintOutcome {
        // SAFETY: the context is live; the color struct outlives the call.
        unsafe {
            self.context.BeginDraw();
            let color = bg_color_f(bg);
            self.context.Clear(Some(&color));
            if let Err(e) = self.context.EndDraw(None, None) {
                eprintln!(
                    "riviv: EndDraw failed ({e}, {}) — device loss ladder",
                    loss_kind(e.code())
                );
                return PaintOutcome::DeviceLost;
            }
            present(&self.swapchain)
        }
    }

    /// One frame: BeginDraw → Clear (the letterbox IS the Clear — the GDI
    /// arm's strip concept does not exist here, design §4) → DrawBitmap
    /// with the full source rect and the plan's dest rect → EndDraw →
    /// Present(0,0). The rw>0&&rh>0 guard mirrors the GDI arm's.
    fn draw_frame(&mut self, plan: &DrawPlan) -> PaintOutcome {
        // SAFETY: the context and the uploaded bitmap are live; every
        // rectangle/parameter outlives the call; nothing pumps.
        unsafe {
            self.context.BeginDraw();
            let color = bg_color_f(plan.bg);
            self.context.Clear(Some(&color));
            if plan.rw > 0
                && plan.rh > 0
                && let Some(bitmap) = self.bitmap.as_ref()
            {
                let (mw, mh) = match self.uploaded {
                    Some((_, w, h)) => (w, h),
                    None => (0, 0),
                };
                // i32 → f32 is exact in this range (the integer-rect
                // five-piece clause keeps the values small).
                let dest = D2D_RECT_F {
                    left: plan.dx as f32,
                    top: plan.dy as f32,
                    right: (plan.dx + plan.rw) as f32,
                    bottom: (plan.dy + plan.rh) as f32,
                };
                let src = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: mw as f32,
                    bottom: mh as f32,
                };
                // Full source rect, no perspective: the dest rect carries
                // the whole view math (unit PIXELS + identity transform).
                // DrawBitmap reports nothing itself — its errors surface at
                // the EndDraw below (the loss channel).
                self.context
                    .DrawBitmap(bitmap, Some(&dest), 1.0, plan.interp, Some(&src), None);
            }
            if let Err(e) = self.context.EndDraw(None, None) {
                eprintln!(
                    "riviv: EndDraw failed ({e}, {}) — device loss ladder",
                    loss_kind(e.code())
                );
                return PaintOutcome::DeviceLost;
            }
            present(&self.swapchain)
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
        // SAFETY: releasing the target before ResizeBuffers is the
        // documented sequence (every buffer reference must be gone).
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
    /// borrow: the plan and the master bytes arrive as parameters, this
    /// method never re-enters state_of.
    pub(crate) fn dump(
        &mut self,
        cw: u32,
        ch: u32,
        bg: [u8; 3],
        frame: Option<(u64, u32, u32, &[u8])>,
        plan: Option<DrawPlan>,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        if cw == 0 || ch == 0 {
            return Err(format!("viewport is {cw}x{ch} — nothing to dump"));
        }
        if let Some((frame_gen, wide, high, pixels)) = frame {
            if frame_exceeds_max_bitmap(wide, high, self.max_bitmap) {
                return Err(format!(
                    "frame {wide}x{high} exceeds the D2D max bitmap {}",
                    self.max_bitmap
                ));
            }
            self.ensure_upload(frame_gen, wide, high, pixels)?;
        }
        // SAFETY: the context and bitmap are live; the color/rect structs
        // outlive their calls; nothing pumps.
        unsafe {
            self.context.BeginDraw();
            let color = bg_color_f(bg);
            self.context.Clear(Some(&color));
            if let (Some(plan), Some(bitmap)) = (plan.as_ref(), self.bitmap.as_ref())
                && plan.rw > 0
                && plan.rh > 0
            {
                let (mw, mh) = match self.uploaded {
                    Some((_, w, h)) => (w, h),
                    None => (0, 0),
                };
                let dest = D2D_RECT_F {
                    left: plan.dx as f32,
                    top: plan.dy as f32,
                    right: (plan.dx + plan.rw) as f32,
                    bottom: (plan.dy + plan.rh) as f32,
                };
                let src = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: mw as f32,
                    bottom: mh as f32,
                };
                // The dump's DrawBitmap reports nothing itself — errors
                // surface at the EndDraw below.
                self.context
                    .DrawBitmap(bitmap, Some(&dest), 1.0, plan.interp, Some(&src), None);
            }
            self.context
                .EndDraw(None, None)
                .map_err(|e| format!("dump EndDraw failed: {e}"))?;
        }
        // The readback staging bitmap: CPU_READ | CANNOT_DRAW, viewport
        // sized, the same UNORM format (design §9).
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
        // SAFETY: the context upcasts to its render-target base (the copy
        // SOURCE); the readback bitmap is the destination.
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
        // target; both objects are live.
        unsafe { readback.CopyFromRenderTarget(None, &rt, Some(&src_rect)) }
            .map_err(|e| format!("dump CopyFromRenderTarget failed: {e}"))?;
        // SAFETY: a READ map on a CPU_READ bitmap is the documented access;
        // the mapping is released by the Unmap below before anything drops.
        let mapped = unsafe { readback.Map(D2D1_MAP_OPTIONS_READ) }
            .map_err(|e| format!("dump Map failed: {e}"))?;
        let pitch = mapped.pitch as usize;
        let row_bytes = cw as usize * 4;
        let mut bgra = vec![0u8; row_bytes * ch as usize];
        for (row, dst_row) in bgra.chunks_mut(row_bytes).enumerate() {
            // SAFETY: the mapped region holds pitch*ch readable bytes; each
            // row's first cw*4 bytes land in the packed output (pitch ≥
            // row_bytes is D2D's map contract).
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
        Ok((cw, ch, rgba))
    }
}

impl Drop for GpuStack {
    fn drop(&mut self) {
        // The field declaration order IS the release order (Rust drops
        // fields in order; each COM wrapper Releases in its own Drop):
        // target bitmap → uploaded bitmap → context → swapchain → d2d
        // device → dxgi device → factory → d3d device (design §3). The
        // device-chain fields are never dereferenced in normal operation —
        // they exist so the WHOLE stack dies together (ADR 0002 D8, "one
        // struct 生死与共"); reading them here pins that keep-alive intent.
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
/// design §13-4: no polling recovery in #80). Any other failure code is
/// NOT a device loss (the ladder's two codes are checked first) but is
/// never silently swallowed either: an unexpected Present failure that
/// kept recurring would read as "the window went blank for no reason" —
/// one stderr line per occurrence is the repo's degrade-diagnostics
/// channel.
unsafe fn present(swapchain: &IDXGISwapChain1) -> PaintOutcome {
    // SAFETY: the swapchain is live; flags 0 = the plain interactive form.
    let hr = unsafe { swapchain.Present(0, DXGI_PRESENT(0)) };
    if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
        PaintOutcome::DeviceLost
    } else {
        if hr.is_err() {
            eprintln!(
                "riviv: Present returned {:#010x} — frame skipped",
                hr.0 as u32
            );
        }
        PaintOutcome::Painted
    }
}

/// The EndDraw loss classifier (design §7): D2DERR_RECREATE_TARGET is the
/// documented device-loss code; ANY other EndDraw failure routes through
/// the same ladder — a rebuild is cheaper than painting on a dead device,
/// and the ladder's 3-in-10s window bounds the churn either way.
fn loss_kind(code: windows::core::HRESULT) -> &'static str {
    if code == D2DERR_RECREATE_TARGET {
        "D2DERR_RECREATE_TARGET"
    } else {
        "other failure"
    }
}

// ---------------------------------------------------------------------------
// The WM_PAINT arm (routed from view_proc by window.rs paint_view)
// ---------------------------------------------------------------------------

/// The D2D paint (design §4, ADR 0002 D8 verbatim): IsIconic early-exit →
/// BeginPaint (rcPaint ignored — a flip back buffer is discarded, the
/// whole viewport redraws; failure is the GDI arm's fatal) → the giant
/// gate → upload-if-stale → BeginDraw/Clear/DrawBitmap/EndDraw/Present →
/// EndPaint → the #76 paint handshake (the render stack's health is
/// irrelevant to the decode worker's first-frame wait). Device losses
/// return as [`PaintOutcome::DeviceLost`] for the router's ladder; nothing
/// in here fatals after BeginPaint.
pub(crate) fn paint_d2d(view: HWND, owner: HWND) -> PaintOutcome {
    // SAFETY: read-only query on the live owner.
    if unsafe { IsIconic(owner) }.as_bool() {
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
        let max_bitmap = state
            .gpu
            .as_ref()
            .map(|gpu| gpu.max_bitmap)
            .unwrap_or_default();
        if let Some((mw, mh)) = dims
            && frame_exceeds_max_bitmap(mw, mh, max_bitmap)
        {
            // The giant-image gate (design §5): this frame paints through
            // GDI; the router tears the stack down after this borrow ends.
            let _ = EndPaint(view, &ps);
            return PaintOutcome::GiantFrame {
                wide: mw,
                high: mh,
                max: max_bitmap,
            };
        }
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
                let master = image.surface().master();
                match gpu.ensure_upload(frame_gen, *mw, *mh, &master.pixels) {
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

    // ---- d2d_interp_mode (#80's two tiers) ----

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
            D2D1_INTERPOLATION_MODE_LINEAR,
            "shrink_blit_mode=1 (HALFTONE) draws linear"
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

    // ---- the giant-image gate (design §5) ----

    #[test]
    fn the_giant_gate_admits_the_max_and_blocks_one_past_it() {
        // 16384 is the typical GetMaximumBitmapSize: it uploads, 16385 does
        // not — on either axis, independently.
        assert!(!frame_exceeds_max_bitmap(16384, 16384, 16384));
        assert!(frame_exceeds_max_bitmap(16385, 100, 16384));
        assert!(frame_exceeds_max_bitmap(100, 16385, 16384));
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
    fn recreate_target_is_the_documented_loss_code() {
        // The classifier names the documented EndDraw code and lumps every
        // other failure into the same ladder bucket.
        assert_eq!(loss_kind(D2DERR_RECREATE_TARGET), "D2DERR_RECREATE_TARGET");
        assert_eq!(
            loss_kind(windows::core::HRESULT(0x8000_4005_u32 as _)),
            "other failure"
        );
    }
}
