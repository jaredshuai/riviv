//! The display-segment judge (#130, M8-3): which WCS display profile the
//! window's monitor answers with. This is the PRIMARY judge of the
//! transform decision table (transform_stage.rs, D3) — never the ACM
//! bit, never the legacy `GetICMProfileW` (probe P2: on this machine the
//! legacy getter answers pure sRGB while the display is profiled
//! AdobeRGB — trusting it would silently skip the transform on every
//! wide-gamut panel).
//!
//! The judge API is the modern family getter `ColorProfileGetDisplayDefault`
//! (icm.h 26100, real signature transcribed by probe P2):
//! CURRENT_USER + CPT_ICC + CPST_STANDARD_DISPLAY_COLOR_MODE over the
//! monitor's (targetAdapterLUID, sourceID). The export exists only from
//! Windows Server 2022 / Win11 (build 20348) — a raw-dylib static import
//! would fail the Win10 1607 floor AT LOAD TIME, so the address is
//! resolved dynamically (LoadLibraryW + GetProcAddress, one cached
//! resolution per process; the module reference is deliberately never
//! freed). A missing export or a failing call is the table's `Unknown`
//! (never transform on a guess; the accepted Win10 degradation, #127
//! AI3).
//!
//! The (LUID, sourceID) pair comes from `QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)`
//! — probe P2's evidence that this family works in agent contexts, where
//! the `DisplayConfigGetDeviceInfo` family (probe P1) fails across the
//! board. Matching the window's monitor to a path therefore walks a
//! ladder: the canonical source-name match first (interactive sessions),
//! the single-active-path fallback second (one monitor is unambiguous),
//! `Unknown` last.
//!
//! Classification reuses Stage 1's equivalence ladder verbatim (icm.rs
//! `probe_equivalence`): byte-identical to the system sRGB profile, or
//! probe-equivalent, is `SrgbEquivalent` (no work); anything else is
//! `Custom` (the gpu_effect row, hardware only). The outcome carries the
//! profile PATH (the D2D effect loads the ICC from the file) and BYTES
//! (the output fingerprint's digest term) alongside the table's query
//! value, so one call feeds decision, effect, and identity.
//!
//! #132 adds the hot-reload freshness half: one cheap probe re-asks the
//! getter's raw name (the heavy-tail-free prefix of this same chain) and
//! a pure predicate decides whether the full query must re-run — the
//! establishment channel on the window side owns the cadence (a
//! WM_DISPLAYCHANGE arm plus the 2s freshness timer). #134 joins the ACM
//! diagnostic (type 9 bit 1 — the output fingerprint's last placeholder)
//! to the same walk: `current_freshness_inputs` answers BOTH inputs off
//! one path resolution, and the predicate's contract widens to "name OR
//! ac moved", so a bare ACM flip re-establishes too.

use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_HEADER;
use windows::Win32::Devices::Display::DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_MODE_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_PATH_INFO;
use windows::Win32::Devices::Display::DISPLAYCONFIG_SOURCE_DEVICE_NAME;
use windows::Win32::Devices::Display::DisplayConfigGetDeviceInfo;
use windows::Win32::Devices::Display::GetDisplayConfigBufferSizes;
use windows::Win32::Devices::Display::QDC_ONLY_ACTIVE_PATHS;
use windows::Win32::Devices::Display::QueryDisplayConfig;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::LUID;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Graphics::Gdi::GetMonitorInfoW;
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
use windows::Win32::Graphics::Gdi::MONITORINFOEXW;
use windows::Win32::Graphics::Gdi::MonitorFromWindow;
use windows::Win32::System::LibraryLoader::GetProcAddress;
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::UI::ColorSystem::COLORPROFILESUBTYPE;
use windows::Win32::UI::ColorSystem::COLORPROFILETYPE;
use windows::Win32::UI::ColorSystem::CPST_STANDARD_DISPLAY_COLOR_MODE;
use windows::Win32::UI::ColorSystem::CPT_ICC;
use windows::Win32::UI::ColorSystem::WCS_PROFILE_MANAGEMENT_SCOPE;
use windows::Win32::UI::ColorSystem::WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER;
use windows::core::HRESULT;
use windows::core::PWSTR;
use windows::core::s;
use windows::core::w;

use crate::icm::EquivalenceProbe;
use crate::icm::probe_equivalence;
use crate::transform_stage::AcState;
use crate::transform_stage::DisplayProfileQuery;
use crate::transform_stage::DisplayProfileSpace;

/// What one judge call settled: the table's query value plus the raw
/// material the wiring consumes — the profile path (the D2D effect's
/// `CreateColorContextFromFilename` input) and bytes (the output
/// fingerprint's digest term). `path`/`bytes` are `Some` exactly when
/// the query is `Profile(_)`; the OS-managed and unreachable-judge
/// shapes carry none.
#[derive(Debug, Clone)]
pub(crate) struct DisplayQueryOutcome {
    pub query: DisplayProfileQuery,
    pub path: Option<PathBuf>,
    pub bytes: Option<Vec<u8>>,
    /// The getter's RAW answer (bare name or path, exactly as returned) —
    /// the #132 freshness key. `None` iff the chain broke at or before
    /// the getter (the Unknown-from-failure shape) OR the profile file
    /// was unreadable (a RETRYABLE failure — the next tick re-runs the
    /// full query until the file opens; Codex P2, PR #133); an
    /// empty-but-successful answer is `Some("")` (the NoProfile signal);
    /// a profile the downstream classification then REFUSES keeps
    /// `Some(raw)` — the refusal is a verdict, stable by design, so a
    /// classification-failing machine never loops the full query.
    pub name: Option<String>,
    /// The ACM diagnostic off the resolved path's TARGET (#134): the
    /// type 9 `advancedColorEnabled` bit — a label for the output
    /// fingerprint, never a decision input (D3). `Unknown` whenever the
    /// monitor ladder did not resolve (no active paths / no matchable
    /// monitor) or the type 9 read failed; a path that RESOLVED keeps
    /// its ac answer even when the profile chain below it breaks (the
    /// ac read keys on the monitor, not on the profile getter).
    pub ac: AcState,
}

impl Default for DisplayQueryOutcome {
    /// The pre-judge posture: unreachable judge, no material, no ACM
    /// answer — the window state's zero value before the first
    /// establishment.
    fn default() -> Self {
        Self {
            query: DisplayProfileQuery::Unknown,
            path: None,
            bytes: None,
            name: None,
            ac: AcState::Unknown,
        }
    }
}

/// One active display path, reduced to the judge's inputs plus the
/// canonical matching key. The profile getter keys on the TARGET
/// adapter + SOURCE id (icm.h), the #134 ACM read keys on the TARGET
/// adapter + TARGET id (wingdi.h) — the path carries both axes.
#[derive(Clone)]
struct PathInfo {
    adapter_luid: LUID,
    source_id: u32,
    /// The TARGET side's id (`targetInfo.id`, e.g. 0x800050 on the P1
    /// probe's single path) — the `header.id` of a
    /// DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO request.
    target_id: u32,
    /// The GDI device name ("\\\\.\\DISPLAY1") from
    /// `DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME` — `None` when the
    /// query failed (the family works in-process — #134's P1 probe
    /// corrected #127 P1's family-level-failure verdict to a
    /// PS-context artifact; see `advanced_color_state`'s doc).
    source_name: Option<String>,
}

/// The judge entry point, called once per output-decision point (stack
/// creation and every rebuild) on the UI thread. Failures degrade to
/// `Unknown` — the table's never-on-a-guess row — never to a guess.
/// The prefix this walks (paths → device → path choice) is the shared
/// `chosen_path` helper, also walked by the freshness probe — see the
/// LOCKSTEP CONTRACT note there before changing either side.
pub(crate) fn query(view: HWND) -> DisplayQueryOutcome {
    let Some(path) = chosen_path(view) else {
        return unknown(AcState::Unknown);
    };
    // #134: the ACM diagnostic rides the SAME resolved path (its TARGET
    // side, the wingdi.h keying), so one query feeds decision, effect,
    // and identity — and a profile-chain break below keeps the ac answer
    // the monitor did give (the ac read keys on the monitor, not the
    // profile getter).
    let ac = advanced_color_state(path.adapter_luid, path.target_id);
    let Some(getter) = display_default_getter() else {
        return unknown(ac);
    };
    let Some(name) = call_display_default(getter, path.adapter_luid, path.source_id) else {
        return unknown(ac);
    };
    if name.is_empty() {
        // A successful answer with no name: the OS owns the display
        // transform (auto color management) — riviv's sRGB output is
        // already correct, the segment must not stack on top.
        return DisplayQueryOutcome {
            query: DisplayProfileQuery::NoProfile,
            path: None,
            bytes: None,
            name: Some(String::new()),
            ac,
        };
    }
    let full = resolve_profile_path(&name);
    let bytes = std::fs::read(&full);
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(e) => {
            // The read is RETRYABLE (Codex P2, PR #133): a switch window can
            // hold the file locked exactly when the freshness timer's first
            // tick lands, and caching the name against a failed read would
            // pin Unknown forever (same name compares stable forever after).
            // Answering name=None makes the next tick's Some(name) a change
            // again — the full query retries every 2s while unreadable,
            // each attempt just one registry read plus one failed file read
            // (the heavy equivalence probe never runs on this path), and
            // the retry loop self-heals the moment the file opens.
            eprintln!(
                "riviv: display profile {}: unreadable ({e}) — display judge falls to unknown (will retry)",
                full.display()
            );
            return unknown(ac);
        }
    };
    let shown = full
        .file_name()
        .map(|n| format!("display profile {}", n.to_string_lossy()))
        .unwrap_or_else(|| format!("display profile {}", full.to_string_lossy()));
    match probe_equivalence(bytes.clone(), &shown, "display judge falls to unknown") {
        Some(EquivalenceProbe::Equivalent) => DisplayQueryOutcome {
            query: DisplayProfileQuery::Profile(DisplayProfileSpace::SrgbEquivalent),
            path: Some(full),
            bytes: Some(bytes),
            name: Some(name),
            ac,
        },
        Some(EquivalenceProbe::Different(_)) => DisplayQueryOutcome {
            query: DisplayProfileQuery::Profile(DisplayProfileSpace::Custom),
            path: Some(full),
            bytes: Some(bytes),
            name: Some(name),
            ac,
        },
        // The equivalence ladder failed on a profile the OS handed us:
        // not classifiable is not transformable — Unknown, never a guess.
        None => unclassifiable(name, ac),
    }
}

fn unknown(ac: AcState) -> DisplayQueryOutcome {
    DisplayQueryOutcome {
        query: DisplayProfileQuery::Unknown,
        path: None,
        bytes: None,
        name: None,
        ac,
    }
}

/// The judge's answer with the getter's raw name attached but the
/// DOWNSTREAM CLASSIFICATION refused (the equivalence ladder rejected
/// the profile — a verdict, stable by machine): the query is Unknown,
/// while the freshness key stays the raw name so the hot-reload compare
/// sees a STABLE key instead of looping the full query. The other
/// name-carrying failure — an unreadable FILE — deliberately does NOT
/// come here: it answers `unknown(ac)` (name=None) because a read is
/// retryable (Codex P2, PR #133); a classification refusal is not.
fn unclassifiable(name: String, ac: AcState) -> DisplayQueryOutcome {
    DisplayQueryOutcome {
        query: DisplayProfileQuery::Unknown,
        path: None,
        bytes: None,
        name: Some(name),
        ac,
    }
}

/// The freshness gate's two inputs off ONE ladder walk (#132's name,
/// #134's ac): the getter's raw answer (`None` = the chain broke at or
/// before the getter, the Unknown-from-failure shape) and the ACM
/// diagnostic (`Unknown` = the path ladder or the type 9 read failed).
/// Both come from the same path resolution so one tick can never
/// straddle a topology change between its two answers.
#[derive(Debug)]
pub(crate) struct FreshnessInputs {
    pub(crate) name: Option<String>,
    pub(crate) ac: AcState,
}

/// The freshness probe's cheap half (#132, widened by #134): re-ask ONLY
/// the getter's raw name AND the type 9 ACM bit for the window's monitor
/// — no bytes read, no equivalence probe (the full query's heavy tail is
/// reserved for an actual change). Breaks anywhere in the chain answer
/// the same Unknown-from-failure shapes `query` produces, so a broken
/// chain compares stable against a broken establishment (the ac half:
/// two `Unknown`s, like two `None`s, are stable).
///
/// LOCKSTEP CONTRACT (pre-review P3, now per its own "factor the shared
/// prefix" branch): this and `query` walk ONE shared prefix —
/// `chosen_path` — so their idea of "the window's monitor" cannot drift
/// apart; do not re-inline the ladder in either walker.
pub(crate) fn current_freshness_inputs(view: HWND) -> FreshnessInputs {
    let Some(path) = chosen_path(view) else {
        return FreshnessInputs {
            name: None,
            ac: AcState::Unknown,
        };
    };
    let ac = advanced_color_state(path.adapter_luid, path.target_id);
    let name = display_default_getter()
        .and_then(|getter| call_display_default(getter, path.adapter_luid, path.source_id));
    FreshnessInputs { name, ac }
}

/// The freshness predicate's NAME half (#132, pure): has the judge's raw
/// name actually changed? `None` on either side means the chain broke
/// there — a break appearing or healing counts as a change (the judge's
/// answer genuinely moved), while two equal `Some`s (including two
/// `Some("")` OS-managed answers) are stable.
pub(crate) fn profile_name_changed(old: Option<&str>, fresh: Option<&str>) -> bool {
    old != fresh
}

/// The freshness predicate, widened by #134: the gate re-establishes on
/// a NAME move or an ACM move. A bare ac flip with the name standing
/// still must reach the establishment and mint a gen (an ACM/HDR toggle
/// can flip the type 9 bit in place — WM_DISPLAYCHANGE's documented
/// bit-depth class — without touching the profile name); two equal
/// (name, ac) pairs are stable, including two broken chains and two
/// stable `Unknown`s.
pub(crate) fn freshness_changed(
    old_name: Option<&str>,
    fresh_name: Option<&str>,
    old_ac: AcState,
    fresh_ac: AcState,
) -> bool {
    profile_name_changed(old_name, fresh_name) || old_ac != fresh_ac
}

/// The shared ladder prefix of every judge walk (`query` and
/// `current_freshness_inputs`): the active paths, the window's monitor
/// as a GDI device name, and the one chosen path — `None` anywhere is
/// the Unknown shape (nothing to judge against). The owned copy lets
/// each walker carry the path's inputs without borrowing the locals.
fn chosen_path(view: HWND) -> Option<PathInfo> {
    let paths = active_paths()?;
    let device = monitor_device_name(view);
    choose_path(&paths, device.as_deref()).cloned()
}

/// The window's monitor as a GDI device name, the canonical match key
/// for the source-name route. `MONITORINFOEXW` extends `MONITORINFO`
/// with the name; the C layouts are prefix-compatible and `cbSize`
/// declares the larger one.
fn monitor_device_name(view: HWND) -> Option<String> {
    // SAFETY: read-only monitor query on a live window handle we own;
    // MONITOR_DEFAULTTONEAREST makes the null-monitor case impossible.
    let monitor = unsafe { MonitorFromWindow(view, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `mi` is a valid MONITORINFOEXW-sized writable buffer (the
    // API reads it as its prefix-compatible MONITORINFO); the handle
    // came from MonitorFromWindow above and is valid for the call.
    let ok = unsafe { GetMonitorInfoW(monitor, &mut mi as *mut _ as *mut _) };
    if !ok.as_bool() {
        return None;
    }
    let len = mi
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(mi.szDevice.len());
    Some(String::from_utf16_lossy(&mi.szDevice[..len]))
}

/// The active display paths, each with the canonical name when the
/// `DisplayConfigGetDeviceInfo` family cooperates. `None` = the path
/// enumeration itself failed (nothing to match against).
fn active_paths() -> Option<Vec<PathInfo>> {
    let mut n_paths = 0u32;
    let mut n_modes = 0u32;
    // SAFETY: pure size query; both out pointers are valid locals.
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) }
        != ERROR_SUCCESS
        || n_paths == 0
    {
        return None;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); n_paths as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); n_modes as usize];
    // SAFETY: both arrays match the counts the size query reported and
    // stay live for the call; the counts are passed by reference so the
    // API can rewrite them with what it actually filled.
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            None,
        )
    } != ERROR_SUCCESS
    {
        return None;
    }
    Some(
        paths[..n_paths as usize]
            .iter()
            .map(|p| PathInfo {
                adapter_luid: p.targetInfo.adapterId,
                source_id: p.sourceInfo.id,
                // The TARGET side's id: the #134 ACM read's header.id
                // (wingdi.h keys GET_ADVANCED_COLOR_INFO on the target),
                // distinct from the getter's (target adapter + SOURCE id)
                // and the name query's (source adapter + source id) axes.
                target_id: p.targetInfo.id,
                // The source-name request keys on the SOURCE side
                // (sourceInfo.adapterId + sourceInfo.id); the getter below
                // keys on the TARGET side (targetInfo.adapterId + the
                // same source id) — two different adapter axes of one
                // path, and mixing them fails the name lookup on
                // split-adapter topologies (Codex P2, PR #131).
                source_name: source_name(p.sourceInfo.adapterId, p.sourceInfo.id),
            })
            .collect(),
    )
}

/// The canonical per-path GDI device name — the caller's ladder
/// treats a failure as "no name" and falls through. The family works
/// in-process: #134's P1 probe corrected #127 P1's
/// family-level-failure verdict to a PS-context artifact (see
/// `advanced_color_state`'s doc).
fn source_name(adapter: LUID, source: u32) -> Option<String> {
    let mut info = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    info.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    info.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    info.header.adapterId = adapter;
    info.header.id = source;
    // SAFETY: `info` is a valid request packet of the exact type/size its
    // header declares; the API writes back into the same buffer only.
    let rc = unsafe {
        DisplayConfigGetDeviceInfo(&mut info.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER)
    };
    if rc != 0 {
        return None;
    }
    let len = info
        .viewGdiDeviceName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.viewGdiDeviceName.len());
    Some(String::from_utf16_lossy(&info.viewGdiDeviceName[..len]))
}

/// wingdi.h 26100 lines 3167-3186, transcribed (the docs page is gone —
/// probe P1's SDK copy, the same pin the `AcState` doc carries). The
/// #134 request: type 9 over a path's TARGET, 32 bytes total.
///
/// ```c
/// typedef struct DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
///   DISPLAYCONFIG_DEVICE_INFO_HEADER header; // type 9 = GET_ADVANCED_COLOR_INFO
///   union {
///     struct {
///       UINT32 advancedColorSupported     : 1;  // bit 0
///       UINT32 advancedColorEnabled       : 1;  // bit 1 <- "ACM in effect"
///       UINT32 wideColorEnforced          : 1;  // bit 2
///       UINT32 advancedColorForceDisabled : 1;  // bit 3
///     } DUMMYSTRUCTNAME;
///     UINT32 value;
///   } DUMMYUNIONNAME;
///   DISPLAYCONFIG_COLOR_ENCODING colorEncoding; // probe machine: 0 (RGB)
///   UINT32 bitsPerColorChannel;                 // probe machine: 10
/// } DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO;      // sizeof = 32
/// ```
///
/// Probe P1 evidence (2026-09-26, in-riviv-process `#[cfg(test)]` probe,
/// cargo test context): rc = 0, byte-stable across three repeats, flags
/// = 0x3 (supported | enabled), colorEncoding RGB, 10 bpc — and the
/// #127 P1 "whole family ERROR_GEN_FAILURE" verdict is hereby corrected
/// to a PS-context/marshaling artifact, not an API absence (the type 1
/// GET_SOURCE_NAME control answered `\\.\DISPLAY1` the same run).
fn advanced_color_state(target_adapter: LUID, target_id: u32) -> AcState {
    let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
    info.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
    info.header.size = std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
    info.header.adapterId = target_adapter;
    info.header.id = target_id;
    // SAFETY: `info` is a valid request packet of the exact type/size its
    // header declares (the wingdi.h-26100 layout transcribed above);
    // DisplayConfigGetDeviceInfo writes back into the same buffer only.
    let rc = unsafe {
        DisplayConfigGetDeviceInfo(&mut info.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER)
    };
    // SAFETY: reads the union's `value` arm the call above just filled
    // (both arms are the same 4 bytes — no provenance question, and
    // nothing else touches `info` afterwards).
    let flags = unsafe { info.Anonymous.value };
    advanced_color_state_from_call(rc, flags)
}

/// The ACM read's pure half (#134): a successful type 9 answer maps bit
/// 1 — `advancedColorEnabled`, the transcription above's "ACM in effect"
/// bit — to On/Off, and ANY nonzero return code (the WIN32 error the API
/// returns directly: ERROR_GEN_FAILURE, ERROR_ACCESS_DENIED, ...) maps
/// to `Unknown` (D3: the table already covers it — the read is a
/// diagnostic label, never a decision input, so a failed read degrades
/// to the honest label instead of a guess or a stale latch).
fn advanced_color_state_from_call(rc: i32, flags: u32) -> AcState {
    if rc != 0 {
        return AcState::Unknown;
    }
    if flags & 0b10 != 0 {
        AcState::On
    } else {
        AcState::Off
    }
}

/// The monitor-matching ladder (the wiring's one decision left to
/// itself, pure):
/// 1. the canonical source-name match (interactive sessions);
/// 2. the single-active-path fallback — a one-monitor desktop is
///    unambiguous, and it is exactly the agent-context shape where the
///    name query fails (probes P1/P2);
/// 3. nothing — multi-path with no readable names is never a guess.
fn choose_path<'a>(paths: &'a [PathInfo], device: Option<&str>) -> Option<&'a PathInfo> {
    if let Some(device) = device
        && let Some(hit) = paths
            .iter()
            .find(|p| p.source_name.as_deref() == Some(device))
    {
        return Some(hit);
    }
    if paths.len() == 1 {
        return Some(&paths[0]);
    }
    None
}

/// The modern getter's signature (icm.h 26100, transcribed by probe P2):
/// HRESULT ColorProfileGetDisplayDefault(scope, targetAdapterLUID,
/// sourceID, type, subtype, LPWSTR* profileName) — the caller frees the
/// name with LocalFree.
type DisplayDefaultGetter = unsafe extern "system" fn(
    WCS_PROFILE_MANAGEMENT_SCOPE,
    LUID,
    u32,
    COLORPROFILETYPE,
    COLORPROFILESUBTYPE,
    *mut PWSTR,
) -> HRESULT;

/// The dynamically resolved `ColorProfileGetDisplayDefault`, cached for
/// the process. The LoadLibraryW reference is deliberately never freed:
/// the fn pointer must stay valid for the process lifetime (S1 floor —
/// a raw-dylib import of this 20348+ export would fail at load time on
/// the Win10 1607 floor).
fn display_default_getter() -> Option<DisplayDefaultGetter> {
    static GETTER: OnceLock<Option<DisplayDefaultGetter>> = OnceLock::new();
    *GETTER.get_or_init(|| {
        // SAFETY: load-only call with a static library name; the handle
        // is intentionally leaked (kept alive by the OS loader's module
        // refcount for the process lifetime, so the resolved address
        // cannot dangle). The Result form maps a load failure to None.
        let Ok(mscms) = (unsafe { LoadLibraryW(w!("mscms.dll")) }) else {
            return None;
        };
        // SAFETY: the module handle is live (leaked above) and the name
        // is a static ANSI string; a null result is the Win10 case (the
        // export does not exist there) and is the checked failure.
        let addr = unsafe { GetProcAddress(mscms, s!("ColorProfileGetDisplayDefault")) }?;
        // SAFETY: the transmute re-types the address GetProcAddress
        // returned into the exact icm.h-declared shape (probe P2 called
        // this exact form successfully through P/Invoke); the signature
        // and calling convention are fixed by the ABI, not the OS.
        Some(unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, DisplayDefaultGetter>(addr)
        })
    })
}

/// Calls the getter with the P2-verified combination and frees the
/// returned name; `None` = the call failed (Unknown, never a guess). An
/// empty-but-successful answer is `Some(String::new())` — the
/// OS-managed (NoProfile) signal, kept distinct from failure.
fn call_display_default(
    getter: DisplayDefaultGetter,
    adapter: LUID,
    source: u32,
) -> Option<String> {
    let mut name = PWSTR::null();
    // SAFETY: the getter was resolved from mscms's export table (exact
    // icm.h signature, probe P2); `name` is a valid out pointer; the
    // profile-name allocation is LocalFree'd on every path below.
    let hr = unsafe {
        getter(
            WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER,
            adapter,
            source,
            CPT_ICC,
            CPST_STANDARD_DISPLAY_COLOR_MODE,
            &mut name,
        )
    };
    if hr.is_err() {
        return None;
    }
    if name.is_null() {
        // Defensive: a success HRESULT with no allocation. Probe P2
        // never observed this shape, but treating it as "no name"
        // matches the OS-managed reading and avoids freeing a null.
        return Some(String::new());
    }
    // SAFETY: `name` is the non-null, NUL-terminated wide string the
    // getter allocated (its documented encoding); to_string reads within
    // the allocation, and the LocalFree releases exactly that
    // allocation, paired with the getter's out pointer above.
    let read = unsafe {
        let s = name.to_string().ok()?;
        // LocalFree returns the freed handle (null on success); the
        // return value carries no information worth checking beyond the
        // free itself.
        let _ = LocalFree(Some(HLOCAL(name.as_ptr().cast())));
        s
    };
    Some(read)
}

/// The getter returns either a full path or a bare filename in the
/// system color directory (probe P2 saw both shapes across the subtype
/// sweep); resolve the bare form against
/// `%windir%\System32\spool\drivers\color`.
fn resolve_profile_path(name: &str) -> PathBuf {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let windir = std::env::var_os("windir").unwrap_or_default();
    Path::new(&windir)
        .join(r"System32\spool\drivers\color")
        .join(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(luid_low: u32, source: u32, target: u32, name: Option<&str>) -> PathInfo {
        PathInfo {
            adapter_luid: LUID {
                LowPart: luid_low,
                HighPart: 0,
            },
            source_id: source,
            target_id: target,
            source_name: name.map(str::to_string),
        }
    }

    #[test]
    fn the_canonical_source_name_match_wins() {
        // Interactive shape: names are readable and one matches the
        // window's monitor — the matched path, regardless of order.
        let paths = vec![
            path(1, 0, 0x800001, Some(r"\\.\DISPLAY1")),
            path(2, 3, 0x800002, Some(r"\\.\DISPLAY2")),
        ];
        let hit = choose_path(&paths, Some(r"\\.\DISPLAY2")).unwrap();
        assert_eq!(hit.adapter_luid.LowPart, 2);
        assert_eq!(hit.source_id, 3);
    }

    #[test]
    fn a_single_active_path_matches_without_names() {
        // The agent-context shape (probe P1: the name query fails; probe
        // P2: the path enumeration still works): one monitor is
        // unambiguous, with or without a readable name or device key.
        let paths = vec![path(7, 1, 0x800003, None)];
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY9")).is_some());
        assert!(choose_path(&paths, None).is_some());
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY1")).is_some());
    }

    #[test]
    fn multiple_paths_without_names_never_guess() {
        // Multi-monitor with the name family dead: which path is the
        // window's monitor is a coin flip — Unknown (None) is the only
        // honest answer.
        let paths = vec![path(1, 0, 0x800004, None), path(2, 0, 0x800005, None)];
        assert!(choose_path(&paths, None).is_none());
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY1")).is_none());
    }

    #[test]
    fn an_unmatched_name_with_multiple_named_paths_does_not_guess() {
        // A stale device name against a healthy multi-monitor desktop:
        // the single-path fallback must NOT fire (len > 1) — no guess.
        let paths = vec![
            path(1, 0, 0x800001, Some(r"\\.\DISPLAY1")),
            path(2, 3, 0x800002, Some(r"\\.\DISPLAY2")),
        ];
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY7")).is_none());
    }

    #[test]
    fn the_freshness_predicate_keys_on_the_raw_name_only() {
        // Stable shapes: the two nulls (a broken chain against a broken
        // establishment) and two equal raw names — including the
        // OS-managed empty answer and a name whose downstream
        // classification fails (the unclassifiable shape keeps Some).
        assert!(!profile_name_changed(None, None));
        assert!(!profile_name_changed(Some("panel.icm"), Some("panel.icm")));
        assert!(!profile_name_changed(Some(""), Some("")));
        // Changes: a real profile switch, both switch directions, a
        // break appearing (Some -> None) and healing (None -> Some),
        // and the OS-managed edge flipping either way.
        assert!(profile_name_changed(Some("adobe.icm"), Some("srgb.icm")));
        assert!(profile_name_changed(Some("srgb.icm"), Some("adobe.icm")));
        assert!(profile_name_changed(Some("adobe.icm"), None));
        assert!(profile_name_changed(None, Some("adobe.icm")));
        assert!(profile_name_changed(Some(""), Some("adobe.icm")));
        assert!(profile_name_changed(Some("adobe.icm"), Some("")));
    }

    #[test]
    fn the_ac_read_is_bit_one_and_any_failed_call_is_unknown() {
        // #134's decode contract (the wingdi.h 26100 transcription over
        // advanced_color_state): bit 1 = advancedColorEnabled is THE "ACM
        // in effect" bit; every other bit is decoration for this purpose.
        // 0x3 is the probe machine's measured value (supported | enabled,
        // P1 evidence), so it must read On.
        assert_eq!(advanced_color_state_from_call(0, 0x3), AcState::On);
        assert_eq!(advanced_color_state_from_call(0, 0x2), AcState::On);
        assert_eq!(advanced_color_state_from_call(0, 0x1), AcState::Off);
        assert_eq!(advanced_color_state_from_call(0, 0x0), AcState::Off);
        assert_eq!(
            advanced_color_state_from_call(0, 0x9),
            AcState::Off,
            "force-disabled with bit 1 clear is still Off"
        );
        // ANY nonzero rc is a failed read: Unknown, whatever the packet
        // happens to hold (ERROR_GEN_FAILURE = 31, ERROR_ACCESS_DENIED = 5).
        assert_eq!(advanced_color_state_from_call(31, 0x3), AcState::Unknown);
        assert_eq!(advanced_color_state_from_call(5, 0x0), AcState::Unknown);
    }

    #[test]
    fn the_widened_freshness_predicate_fires_on_a_bare_ac_move_and_stays_stable_when_both_stand() {
        // #134's gate contract: name OR ac. A name flip with equal acs
        // fires (the #132 shape); a bare ac flip with the name standing
        // still — including two Nones (a broken chain on both sides) and
        // the Unknown-after-failure shape — fires; two equal (name, ac)
        // pairs never do (the smoke132 idempotence contract).
        assert!(freshness_changed(
            Some("a.icm"),
            Some("b.icm"),
            AcState::Off,
            AcState::Off
        ));
        assert!(freshness_changed(
            Some("a.icm"),
            Some("a.icm"),
            AcState::Off,
            AcState::On
        ));
        assert!(freshness_changed(
            Some("a.icm"),
            None,
            AcState::Unknown,
            AcState::Unknown
        ));
        assert!(freshness_changed(None, None, AcState::Off, AcState::On));
        assert!(!freshness_changed(None, None, AcState::Off, AcState::Off));
        assert!(!freshness_changed(
            Some(""),
            Some(""),
            AcState::On,
            AcState::On
        ));
        assert!(!freshness_changed(
            Some("a.icm"),
            Some("a.icm"),
            AcState::Unknown,
            AcState::Unknown
        ));
    }

    #[test]
    fn the_outcome_name_field_separates_broken_chain_from_failed_classification() {
        // The freshness key's None/Some shapes: a chain broken at or
        // before the getter — or an UNREADABLE FILE, the retryable
        // failure (Codex P2, PR #133) — carries None (the default and
        // the failure paths; the next tick retries the full query),
        // while a getter answer the downstream classification then
        // refuses keeps the raw name — a stable verdict that must not
        // loop the full query every 2s.
        assert_eq!(DisplayQueryOutcome::default().name, None);
        assert_eq!(
            DisplayQueryOutcome::default().ac,
            AcState::Unknown,
            "the pre-establishment posture has no ACM answer either"
        );
        assert_eq!(unknown(AcState::Unknown).name, None);
        let unclassified = unclassifiable("panel.icm".to_string(), AcState::On);
        assert_eq!(unclassified.query, DisplayProfileQuery::Unknown);
        assert_eq!(unclassified.name, Some("panel.icm".to_string()));
        assert_eq!(
            unclassified.ac,
            AcState::On,
            "a classification refusal keeps the monitor's ACM answer"
        );
        assert!(unclassified.path.is_none() && unclassified.bytes.is_none());
    }

    #[test]
    fn bare_names_resolve_into_the_system_color_directory() {
        // Probe P2: the getter can answer a bare filename; the resolved
        // path lands in %windir%\System32\spool\drivers\color. The two
        // windir values prove the resolution follows the variable.
        // Edition 2024: the env mutation is unsafe (a process-global
        // race in principle) — the test is single-threaded and restores
        // the value before asserting.
        let restore = std::env::var_os("windir");
        // SAFETY: single-threaded test (cargo runs this file's tests on
        // one thread; the only other windir consumer here is the second
        // set_var below) and the value is restored before asserting.
        unsafe {
            std::env::set_var("windir", r"C:\Windows");
        }
        let a = resolve_profile_path("TPLCD_8BAF_AdobeRGB.icm");
        // SAFETY: same single-threaded contract as above.
        unsafe {
            std::env::set_var("windir", r"C:\WINNT");
        }
        let b = resolve_profile_path("TPLCD_8BAF_AdobeRGB.icm");
        if let Some(restore) = restore {
            // SAFETY: same single-threaded contract; restores the
            // pre-test value so later tests see the real environment.
            unsafe { std::env::set_var("windir", restore) };
        }
        assert!(
            a.ends_with(r"spool\drivers\color\TPLCD_8BAF_AdobeRGB.icm"),
            "{a:?}"
        );
        assert!(
            b.ends_with(r"spool\drivers\color\TPLCD_8BAF_AdobeRGB.icm"),
            "{b:?}"
        );
        assert_ne!(a, b, "the resolution must follow %windir%");
        assert_eq!(
            resolve_profile_path(r"C:\Custom\Profiles\panel.icm"),
            Path::new(r"C:\Custom\Profiles\panel.icm")
        );
    }
}
