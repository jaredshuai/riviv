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

use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
use windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_HEADER;
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
}

impl Default for DisplayQueryOutcome {
    /// The pre-judge posture: unreachable judge, no material — the
    /// window state's zero value before the first establishment.
    fn default() -> Self {
        Self {
            query: DisplayProfileQuery::Unknown,
            path: None,
            bytes: None,
        }
    }
}

/// One active display path, reduced to the judge's inputs plus the
/// canonical matching key.
struct PathInfo {
    adapter_luid: LUID,
    source_id: u32,
    /// The GDI device name ("\\\\.\\DISPLAY1") from
    /// `DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME` — `None` when the
    /// query failed (probe P1: the whole `DisplayConfigGetDeviceInfo`
    /// family returns ERROR_GEN_FAILURE in agent contexts).
    source_name: Option<String>,
}

/// The judge entry point, called once per output-decision point (stack
/// creation and every rebuild) on the UI thread. Failures degrade to
/// `Unknown` — the table's never-on-a-guess row — never to a guess.
pub(crate) fn query(view: HWND) -> DisplayQueryOutcome {
    let Some(paths) = active_paths() else {
        return unknown();
    };
    let device = monitor_device_name(view);
    let Some(path) = choose_path(&paths, device.as_deref()) else {
        return unknown();
    };
    let Some(getter) = display_default_getter() else {
        return unknown();
    };
    let Some(name) = call_display_default(getter, path.adapter_luid, path.source_id) else {
        return unknown();
    };
    if name.is_empty() {
        // A successful answer with no name: the OS owns the display
        // transform (auto color management) — riviv's sRGB output is
        // already correct, the segment must not stack on top.
        return DisplayQueryOutcome {
            query: DisplayProfileQuery::NoProfile,
            path: None,
            bytes: None,
        };
    }
    let full = resolve_profile_path(&name);
    let bytes = std::fs::read(&full);
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!(
                "riviv: display profile {}: unreadable ({e}) — display judge falls to unknown",
                full.display()
            );
            return unknown();
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
        },
        Some(EquivalenceProbe::Different(_)) => DisplayQueryOutcome {
            query: DisplayProfileQuery::Profile(DisplayProfileSpace::Custom),
            path: Some(full),
            bytes: Some(bytes),
        },
        // The equivalence ladder failed on a profile the OS handed us:
        // not classifiable is not transformable — Unknown, never a guess.
        None => unknown(),
    }
}

fn unknown() -> DisplayQueryOutcome {
    DisplayQueryOutcome {
        query: DisplayProfileQuery::Unknown,
        path: None,
        bytes: None,
    }
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

/// The canonical per-path GDI device name. Probe P1: this API family
/// returns ERROR_GEN_FAILURE across the board in agent contexts — the
/// caller's ladder treats a failure as "no name" and falls through.
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

    fn path(luid_low: u32, source: u32, name: Option<&str>) -> PathInfo {
        PathInfo {
            adapter_luid: LUID {
                LowPart: luid_low,
                HighPart: 0,
            },
            source_id: source,
            source_name: name.map(str::to_string),
        }
    }

    #[test]
    fn the_canonical_source_name_match_wins() {
        // Interactive shape: names are readable and one matches the
        // window's monitor — the matched path, regardless of order.
        let paths = vec![
            path(1, 0, Some(r"\\.\DISPLAY1")),
            path(2, 3, Some(r"\\.\DISPLAY2")),
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
        let paths = vec![path(7, 1, None)];
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY9")).is_some());
        assert!(choose_path(&paths, None).is_some());
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY1")).is_some());
    }

    #[test]
    fn multiple_paths_without_names_never_guess() {
        // Multi-monitor with the name family dead: which path is the
        // window's monitor is a coin flip — Unknown (None) is the only
        // honest answer.
        let paths = vec![path(1, 0, None), path(2, 0, None)];
        assert!(choose_path(&paths, None).is_none());
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY1")).is_none());
    }

    #[test]
    fn an_unmatched_name_with_multiple_named_paths_does_not_guess() {
        // A stale device name against a healthy multi-monitor desktop:
        // the single-path fallback must NOT fire (len > 1) — no guess.
        let paths = vec![
            path(1, 0, Some(r"\\.\DISPLAY1")),
            path(2, 3, Some(r"\\.\DISPLAY2")),
        ];
        assert!(choose_path(&paths, Some(r"\\.\DISPLAY7")).is_none());
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
