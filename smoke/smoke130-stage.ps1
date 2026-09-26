# smoke130 - #130 M8-3 the static gpu_effect display segment: the stderr
# channel, the hw/warp mutual exclusion, and the hw pixel output against a
# CPU (mscms) reference transform. ASCII-only source (PS 5.1 encoding trap,
# #79 lesson). Harness cribbed from smoke126-renderer.ps1: staged-exe
# discipline (every instance runs from a copy of the exe in
# %TEMP%\riviv-130-smoke; the staged ini is rewritten before every launch
# because WM_CLOSE saves config - a leftover key would fake cross-scenario
# regressions); close is always WM_CLOSE (PostMessage to the owner) so the
# close-time dump runs; exit codes are read via GetExitCodeProcess on the
# raw handle captured WHILE ALIVE; waits are poll-based. Every function
# returns EXACTLY ONE value; narration goes through Check/Skip (Write-Host,
# so nothing pollutes a return value).
#
# Machine pin: the decision table maps (backend=hw, a non-sRGB display
# profile) -> gpu_effect and (backend=warp, *) -> none. On the smoke
# machine (Win11 26200, single screen, display profile
# TPLCD_8BAF_AdobeRGB.icm, Custom class) both arms hold. On a machine with
# an sRGB-equivalent display profile the S1 gpu_effect line will not
# appear - this smoke pins the Custom-profile contract and FAILS honestly
# elsewhere rather than skipping silently.
#
# The observed stderr surface (exact forms, src/window.rs; the ac field
# is the #134 ACM diagnostic - a label, machine-dependent, and CONSTANT
# within this matrix because no ACM toggle is ever forced here):
#   riviv: display-stage=<none|cpu|gpu_effect|dwm_acm> profile=<name|none> backend=<hw|warp> ac=<off|on|unknown>
#   riviv: dump-viewport output_gen=<n>
#   riviv: display effect failure #<n>: <detail>   (ratchet, never here)
#
# Scenarios:
#   S0  stage the exe; FATAL-exit 3 when a FOREIGN riviv is alive (#109).
#   S1  hw arm (-renderer d2d): exit 0, display-stage=gpu_effect
#       profile=<name> backend=hw ac=<word> (name parsed and kept, the ac
#       word present and in vocabulary), dump output_gen=1,
#       viewport-sized PNG, the profile file resolvable under the color
#       spool directory.
#   S2  warp arm (-renderer warp): display-stage=none profile=<same>
#       backend=warp ac=<same word as S1 - no ACM switch happened>
#       (the mutual exclusion), dump gen=1, PNG exists; a second warp run
#       dumps a whole-file SHA256-identical PNG (warp determinism
#       survives #130).
#   S3  the correctness core: the warp dump (S2) pushed through an mscms
#       sRGB->display ICC transform (C# Add-Type P/Invoke; the numeric
#       constants are the ones src/icm.rs links via windows-rs 0.62) is
#       the reference; the hw dump (S1) must match it with a max channel
#       diff <= 24 AND differ somewhere (a zero-diff pair would mean both
#       arms ran the same thing - the false pass the old byte-equality
#       check could never catch).
#   S4  breadcrumb shape only (the trigger is never forced here): the hw
#       stderr contains no 'display effect failure' - guards the fake
#       green where a build failure silently downgrades to direct draw
#       but the dump gen keeps printing.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
public class S80 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L; public int T; public int R; public int B; }
}
public class PngData { public int W; public int H; public byte[] B; }
public static class Px {
    // LockBits Format32bppArgb: the byte order in B[] is B,G,R,A.
    public static PngData Load(string path) {
        using (Bitmap bmp = new Bitmap(path)) {
            PngData p = new PngData(); p.W = bmp.Width; p.H = bmp.Height;
            BitmapData d = bmp.LockBits(new Rectangle(0, 0, p.W, p.H), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            p.B = new byte[p.W * p.H * 4];
            Marshal.Copy(d.Scan0, p.B, 0, p.B.Length);
            bmp.UnlockBits(d);
            return p;
        }
    }
    public static void SaveRgba(string path, byte[] rgba, int w, int h) {
        byte[] bgra = new byte[rgba.Length];
        for (int i = 0; i < rgba.Length; i += 4) { bgra[i] = rgba[i + 2]; bgra[i + 1] = rgba[i + 1]; bgra[i + 2] = rgba[i]; bgra[i + 3] = 255; }
        using (Bitmap bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
            BitmapData d = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
            Marshal.Copy(bgra, 0, d.Scan0, bgra.Length);
            bmp.UnlockBits(d);
            bmp.Save(path, ImageFormat.Png);
        }
    }
    // The smoke126 HashSource recipe (300x200 hash300.png): deterministic
    // varied colors with a 1px black border ring.
    public static byte[] HashSource(int w, int h) {
        byte[] a = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            if (x == 0 || y == 0 || x == w - 1 || y == h - 1) { a[i] = 0; a[i + 1] = 0; a[i + 2] = 0; }
            else {
                a[i] = (byte)((x * 7 + y * 13 + 11) & 255);
                a[i + 1] = (byte)((x * 11 + y * 5 + 29) & 255);
                a[i + 2] = (byte)((x * 3 + y * 17 + 101) & 255);
            }
            a[i + 3] = 255;
        }
        return a;
    }
}
// mscms P/Invoke for the CPU reference transform (smoke130 S3). The
// numeric values are pinned from windows-rs 0.62 Win32::UI::ColorSystem -
// the exact constants src/icm.rs links, so both sides of the comparison
// speak the same numbers:
//   PROFILE_FILENAME=1, PROFILE_MEMBUFFER=2, PROFILE_READ=1,
//   OPEN_EXISTING=3, INTENT_RELATIVE_COLORIMETRIC=1, BEST_MODE=3,
//   USE_RELATIVE_COLORIMETRIC=0x20000 (131072), INDEX_DONT_CARE=0,
//   BM_xRGBQUADS=8, LCS_sRGB=0x73524742 ('sRGB').
public class Wcs {
    [DllImport("mscms.dll", SetLastError=true)] public static extern bool GetStandardColorSpaceProfileW(IntPtr machine, uint profileId, IntPtr buffer, ref uint size);
    [DllImport("mscms.dll", SetLastError=true)] public static extern IntPtr OpenColorProfileW(ref PROFILE p, uint access, uint share, uint creation);
    [DllImport("mscms.dll", SetLastError=true)] public static extern IntPtr CreateMultiProfileTransform(IntPtr[] profiles, uint nProfiles, uint[] intents, uint nIntents, uint flags, uint preferredCmm);
    [DllImport("mscms.dll", SetLastError=true)] public static extern bool TranslateBitmapBits(IntPtr xform, IntPtr src, uint inFmt, uint w, uint h, uint inStride, IntPtr dst, uint outFmt, uint outStride, IntPtr cb, IntPtr lp);
    [DllImport("mscms.dll", SetLastError=true)] public static extern bool CloseColorProfile(IntPtr h);
    [DllImport("mscms.dll", SetLastError=true)] public static extern bool DeleteColorTransform(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct PROFILE { public uint dwType; public IntPtr pProfileData; public uint cbDataSize; }
}
public static class RefXform {
    // The system sRGB profile path, two-call pattern; pcbSize is BYTES
    // (src/icm.rs load_system_srgb: the first call's BOOL is not a
    // verdict - only an unusable size is failure).
    public static string StandardSrgbPath() {
        uint size = 0;
        Wcs.GetStandardColorSpaceProfileW(IntPtr.Zero, 0x73524742u, IntPtr.Zero, ref size);
        if (size == 0 || (size % 2) != 0) return null;
        IntPtr buf = Marshal.AllocHGlobal((int)size);
        try {
            uint used = size;
            if (!Wcs.GetStandardColorSpaceProfileW(IntPtr.Zero, 0x73524742u, buf, ref used)) return null;
            return Marshal.PtrToStringUni(buf);
        } finally { Marshal.FreeHGlobal(buf); }
    }
    // sRGB -> display ICC transform over a whole BGRA image. Both sides
    // are BM_xRGBQUADS (= byte order B,G,R,x per src/icm.rs), which is
    // exactly the LockBits Format32bppArgb layout - zero swizzle. The
    // profile blobs are held in unmanaged memory until after their
    // handles close (the OpenColorProfileW contract does not promise a
    // completed copy at open time - src/icm.rs ProfileHandle note).
    // Returns null on any CMM refusal.
    public static byte[] Apply(byte[] srgbBlob, byte[] dispBlob, byte[] pixels, int w, int h) {
        IntPtr pSrc = IntPtr.Zero, pDst = IntPtr.Zero;
        IntPtr srgbPtr = IntPtr.Zero, dispPtr = IntPtr.Zero;
        IntPtr hSrgb = IntPtr.Zero, hDisp = IntPtr.Zero, xform = IntPtr.Zero;
        try {
            srgbPtr = Marshal.AllocHGlobal(srgbBlob.Length);
            Marshal.Copy(srgbBlob, 0, srgbPtr, srgbBlob.Length);
            dispPtr = Marshal.AllocHGlobal(dispBlob.Length);
            Marshal.Copy(dispBlob, 0, dispPtr, dispBlob.Length);
            pSrc = Marshal.AllocHGlobal(pixels.Length);
            Marshal.Copy(pixels, 0, pSrc, pixels.Length);
            pDst = Marshal.AllocHGlobal(pixels.Length);
            hSrgb = OpenMem(srgbPtr, srgbBlob.Length);
            hDisp = OpenMem(dispPtr, dispBlob.Length);
            if (hSrgb == IntPtr.Zero || hDisp == IntPtr.Zero) return null;
            IntPtr[] profs = new IntPtr[] { hSrgb, hDisp };
            uint[] intents = new uint[] { 1u, 1u }; // INTENT_RELATIVE_COLORIMETRIC x2
            xform = Wcs.CreateMultiProfileTransform(profs, 2u, intents, 2u, 3u | 0x20000u, 0u); // BEST_MODE|USE_RELATIVE_COLORIMETRIC, INDEX_DONT_CARE
            if (xform == IntPtr.Zero) return null;
            uint stride = (uint)(w * 4);
            if (!Wcs.TranslateBitmapBits(xform, pSrc, 8u, (uint)w, (uint)h, stride, pDst, 8u, stride, IntPtr.Zero, IntPtr.Zero)) return null; // BM_xRGBQUADS both sides
            byte[] res = new byte[pixels.Length];
            Marshal.Copy(pDst, res, 0, res.Length);
            return res;
        } finally {
            if (xform != IntPtr.Zero) Wcs.DeleteColorTransform(xform);
            if (hSrgb != IntPtr.Zero) Wcs.CloseColorProfile(hSrgb);
            if (hDisp != IntPtr.Zero) Wcs.CloseColorProfile(hDisp);
            if (srgbPtr != IntPtr.Zero) Marshal.FreeHGlobal(srgbPtr);
            if (dispPtr != IntPtr.Zero) Marshal.FreeHGlobal(dispPtr);
            if (pSrc != IntPtr.Zero) Marshal.FreeHGlobal(pSrc);
            if (pDst != IntPtr.Zero) Marshal.FreeHGlobal(pDst);
        }
    }
    static IntPtr OpenMem(IntPtr p, int len) {
        Wcs.PROFILE pr = new Wcs.PROFILE();
        pr.dwType = 2u;         // PROFILE_MEMBUFFER
        pr.pProfileData = p;
        pr.cbDataSize = (uint)len;
        return Wcs.OpenColorProfileW(ref pr, 1u, 0u, 3u); // PROFILE_READ, share=0, OPEN_EXISTING
    }
    // Channel-semantic compare of two BGRA buffers: max per-channel diff
    // and the count of differing pixels (alpha ignored - both dumps are
    // opaque). The D2D display effect and the mscms reference are both
    // byte-order B,G,R,x here.
    public static int[] Compare(byte[] a, byte[] b) {
        int max = 0, count = 0;
        int n = a.Length < b.Length ? a.Length : b.Length;
        for (int i = 0; i + 3 < n; i += 4) {
            int d0 = Math.Abs((int)a[i] - (int)b[i]);
            int d1 = Math.Abs((int)a[i + 1] - (int)b[i + 1]);
            int d2 = Math.Abs((int)a[i + 2] - (int)b[i + 2]);
            int d = d0 > d1 ? d0 : d1;
            if (d2 > d) d = d2;
            if (d > max) max = d;
            if (d > 0) count++;
        }
        return new int[] { max, count };
    }
}
'@) -ReferencedAssemblies @('System.Drawing')

[void][S80]::SetProcessDPIAware()
$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check($name, $ok, $detail) {
    # Narration only - Write-Host keeps the return channel clean. The
    # detail line is ALWAYS printed (the measured values are evidence,
    # not just failure notes).
    if ($ok) { $script:pass++; Write-Host ('PASS ' + $name + ' -- ' + $detail) }
    else { $script:fail++; Write-Host ('FAIL ' + $name + ' -- ' + $detail) }
}
function Wait-Until($sb, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $sb) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (& $sb)
}

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$CMD_ONE2ONE = 45      # menu.rs Cmd::ViewOneToOne.id()

function Kill-Riviv {
    # #109: only the staged copy is ever killed; the developer's real
    # viewer never matches the deterministic stage path.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $RunExe })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}

$Stage = Join-Path $env:TEMP 'riviv-130-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'

# ---------------------------------------------------------------------------
# S0: the stage. The foreign-riviv check fires BEFORE anything else: a real
# viewer running would corrupt the handoff/baseline results.
# ---------------------------------------------------------------------------
$preExisting = @(Get-Process riviv -ErrorAction SilentlyContinue)
$foreign = @($preExisting | Where-Object { $_.Path -ne $RunExe })
if ($foreign.Count -gt 0) {
    $fxPids = ($foreign | ForEach-Object { $_.Id }) -join ','
    Write-Host ('FATAL foreign riviv running - close it and rerun (pids: ' + $fxPids + ')')
    exit 3
}
Kill-Riviv
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied, no foreign riviv' (Test-Path $RunExe) 'Copy-Item failed'

function Reset-Ini($text) {
    if (Test-Path $Ini) { Remove-Item $Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($Ini, $text) }
}
function Start-Riv($argStr, $errName) {
    # PS 5.1's Start-Process rejects an EMPTY -ArgumentList string, so the
    # parameter is only passed when there is one. The raw process handle is
    # captured WHILE ALIVE (the -PassThru object loses .Handle after exit).
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
    $script:RawH = $script:RawP.Handle
    return $script:RawP
}
function Read-Err($errName) {
    # The file may still be held by the writer's redirect when polled.
    $errPath = Join-Path $Stage $errName
    for ($i = 0; $i -lt 10; $i++) {
        try {
            if (-not (Test-Path $errPath)) { return '' }
            $fs = [IO.File]::Open($errPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            try {
                $sr = New-Object IO.StreamReader($fs)
                return $sr.ReadToEnd()
            } finally { $fs.Dispose() }
        } catch { Start-Sleep -Milliseconds 100 }
    }
    return '(stderr unreadable)'
}
function Wait-Main($p) {
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 100
        $p.Refresh()
        if ($p.MainWindowHandle -ne 0) { return $p.MainWindowHandle }
        if ($p.HasExited) { break }
    }
    return [IntPtr]::Zero
}
function Wait-Title($p, $leaf, $ms) {
    for ($i = 0; $i -lt [int]($ms / 100); $i++) {
        Start-Sleep -Milliseconds 100
        $p.Refresh()
        if ($p.MainWindowTitle -like ('*' + $leaf + '*')) { return $true }
    }
    return $false
}
function View-Of($main) {
    return [S80]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_view', [NullString]::Value)
}
function View-Size($view) {
    $r = New-Object S80+RECT
    [void][S80]::GetClientRect($view, [ref]$r)
    return @([int]($r.R - $r.L), [int]($r.B - $r.T))
}
function Close-Main($p, $main) {
    if ($main -ne [IntPtr]::Zero) {
        [void][S80]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S80]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else {
            return -2
        }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}
# First whole line of $err that starts with $prefix ('' when absent).
function Get-Line($err, $prefix) {
    if ($null -eq $err) { return '' }
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t.StartsWith($prefix)) { return $t }
    }
    return ''
}
# The FIRST display-stage line matching a stage/backend pair ('' when
# absent). $stage/$backend are exact words (gpu_effect/none x hw/warp).
# #134: the line carries a trailing ' ac=<word>' diagnostic field, so the
# tail anchors on the ac= VALUE: a line-end-anchored match requires
# ' backend=<word> ac=' followed by a non-empty value.
function Get-Stage-Line($err, $stage, $backend) {
    if ($null -eq $err) { return '' }
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t -match ('^riviv: display-stage=' + $stage + ' profile=.* backend=' + $backend + ' ac=\S+$')) { return $t }
    }
    return ''
}
function Profile-Of($stageLine) {
    # 'riviv: display-stage=<stage> profile=<name> backend=<backend> ac=<ac>' ->
    # <name>. Greedy .* is safe: ' backend=' terminates the name.
    if ($stageLine -match '^riviv: display-stage=\S+ profile=(.*) backend=\S+ ac=\S+$') {
        return $Matches[1]
    }
    return ''
}
function Ac-Of($stageLine) {
    # Same line shape -> the ac word (<off|on|unknown>), '' when absent.
    if ($stageLine -match '^riviv: display-stage=\S+ profile=.* backend=\S+ ac=(\S+)$') {
        return $Matches[1]
    }
    return ''
}

# Calibrate the main window until the riviv_view child's client rect is
# EXACTLY tw x th (same recipe as smoke80/81/126). Returns the size.
function Calibrate-View($main, $tw, $th) {
    $script:calView = View-Of $main
    for ($i = 0; $i -lt 5; $i++) {
        $script:calVs = View-Size $script:calView
        if (($script:calVs[0] -eq $tw) -and ($script:calVs[1] -eq $th)) { return $script:calVs }
        $wr = New-Object S80+RECT
        [void][S80]::GetWindowRect($main, [ref]$wr)
        $nw = ($wr.R - $wr.L) + ($tw - $script:calVs[0])
        $nh = ($wr.B - $wr.T) + ($th - $script:calVs[1])
        [void][S80]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, $nw, $nh, 0x0006)  # SWP_NOMOVE|SWP_NOZORDER
        $script:calTw = $tw
        $script:calTh = $th
        $null = Wait-Until { $v = (View-Size $script:calView); ($v[0] -eq $script:calTw) -and ($v[1] -eq $script:calTh) } 3000
    }
    return (View-Size (View-Of $main))
}

# One adopt -> calibrate -> commands -> WM_CLOSE dump instance. The view is
# calibrated to EXACTLY 900x600 so the S1/S2 dumps are pixel-comparable.
function Run-Dump($argStr, $outName, $errName) {
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    Reset-Ini $script:BaseIni
    $p = Start-Riv $argStr $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p 'hash300' 12000
    $vs = @(0, 0)
    if ($main -ne [IntPtr]::Zero) {
        $vs = Calibrate-View $main 900 600
        & $script:one2one $main
        Start-Sleep -Milliseconds 500
    }
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Vs = $vs; Adopted = $adopted; Main = $main; Err = (Read-Err $errName) }
}
$script:one2one = { param($m) [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }

# Base window 900x600 (via calibration), no auto zoom, ICM decode stage off,
# renderer=auto (the CLI switch pins the arm per scenario).
$script:BaseIni = "[riviv]`r`nx=60`r`ny=60`r`nwide=940`r`nhigh=720`r`nauto_zoom=0`r`nicm=0`r`nrenderer=auto`r`n"
$hash300 = Join-Path $Stage 'hash300.png'
[Px]::SaveRgba($hash300, [Px]::HashSource(300, 200), 300, 200)

Kill-Riviv

# ---------------------------------------------------------------------------
# S1: the hw arm. -renderer d2d pins the hardware backend; the display
# segment (Custom-class profile) must come up as gpu_effect.
# ---------------------------------------------------------------------------
$r1 = Run-Dump ("`"$hash300`" -renderer d2d -dump-viewport `"$(Join-Path $Stage 's1-hw.png')`"") 's1-hw.png' 's1.err'
$stageLine1 = Get-Stage-Line $r1.Err 'gpu_effect' 'hw'
$profName = Profile-Of $stageLine1
$genLine1 = Get-Line $r1.Err 'riviv: dump-viewport output_gen='
$pngOk1 = Test-Path $r1.Out
$dims1 = '(none)'
$sizeOk1 = $false
if ($pngOk1) {
    $q1 = [Px]::Load($r1.Out)
    $dims1 = "$($q1.W)x$($q1.H)"
    $sizeOk1 = ($q1.W -eq $r1.Vs[0]) -and ($q1.H -eq $r1.Vs[1])
}
Check 'S1a hw arm: exit 0, adopted, display-stage=gpu_effect profile=<name> backend=hw ac=<word>, profile name non-empty' (($r1.Code -eq 0) -and $r1.Adopted -and ($stageLine1 -ne '') -and ($profName -ne '')) "exit=$($r1.Code) adopted=$($r1.Adopted) stageLine=[$stageLine1] stderr=[$($r1.Err.Trim())]"
$acWord1 = Ac-Of $stageLine1
Check 'S1d ac word present and one of off|on|unknown (the #134 diagnostic field, in vocabulary)' (($acWord1 -ceq 'off') -or ($acWord1 -ceq 'on') -or ($acWord1 -ceq 'unknown')) "ac=[$acWord1] stageLine=[$stageLine1]"
Check 'S1b dump: PNG exists, viewport-sized, output_gen=1' ($pngOk1 -and $sizeOk1 -and ($genLine1 -ceq 'riviv: dump-viewport output_gen=1')) "png=$pngOk1 dims=$dims1 viewport=$($r1.Vs[0])x$($r1.Vs[1]) gen=[$genLine1]"

# Resolve the profile file for the S3 reference transform. The judge's path
# lands in %windir%\System32\spool\drivers\color (src/display_profile.rs);
# the per-user spool dir is checked as an honest fallback, and neither hit
# is a FAIL that reports both candidates.
$profFile = ''
$profWhere = ''
if ($profName -ne '') {
    $sysProf = Join-Path $env:windir ("System32\spool\drivers\color\" + $profName)
    $usrProf = Join-Path $env:LOCALAPPDATA ("Microsoft\Windows\Spool\Drivers\Color\" + $profName)
    if (Test-Path $sysProf) { $profFile = $sysProf; $profWhere = 'system-spool' }
    elseif (Test-Path $usrProf) { $profFile = $usrProf; $profWhere = 'user-spool' }
}
Check 'S1c display profile file resolvable in the color spool directory' ($profFile -ne '') ("profile=[$profName] where=[$profWhere] sys=[$(Join-Path $env:windir ("System32\spool\drivers\color\" + $profName))] usr=[$(Join-Path $env:LOCALAPPDATA ("Microsoft\Windows\Spool\Drivers\Color\" + $profName))]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2: the warp arm. The decision table is warp -> none (mutual exclusion);
# warp dumps stay deterministic across runs (whole-file SHA256).
# ---------------------------------------------------------------------------
# The exact warp-arm line. The ac word is machine-dependent (off|on|
# unknown) but CONSTANT within the matrix (no ACM toggle is ever forced
# here - the #134 design comment's regression note), so S1's observed
# word is the honest expectation for both warp arms: it pins the field's
# presence AND its stability without hardcoding the machine's ACM state.
$expectedWarpLine = 'riviv: display-stage=none profile=' + $profName + ' backend=warp ac=' + $acWord1
$r2a = Run-Dump ("`"$hash300`" -renderer warp -dump-viewport `"$(Join-Path $Stage 's2-warp-a.png')`"") 's2-warp-a.png' 's2a.err'
$stageLine2a = Get-Stage-Line $r2a.Err 'none' 'warp'
$genLine2a = Get-Line $r2a.Err 'riviv: dump-viewport output_gen='
Check 'S2a warp arm: exit 0, adopted, display-stage=none profile=<same> backend=warp ac=<same as S1>, gen=1, PNG' (($r2a.Code -eq 0) -and $r2a.Adopted -and ($stageLine2a -ceq $expectedWarpLine) -and ($genLine2a -ceq 'riviv: dump-viewport output_gen=1') -and (Test-Path $r2a.Out)) "exit=$($r2a.Code) adopted=$($r2a.Adopted) stageLine=[$stageLine2a] expected=[$expectedWarpLine] gen=[$genLine2a] png=$(Test-Path $r2a.Out) stderr=[$($r2a.Err.Trim())]"
$r2b = Run-Dump ("`"$hash300`" -renderer warp -dump-viewport `"$(Join-Path $Stage 's2-warp-b.png')`"") 's2-warp-b.png' 's2b.err'
$stageLine2b = Get-Stage-Line $r2b.Err 'none' 'warp'
Check 'S2b second warp arm: exit 0, adopted, same display-stage line, PNG' (($r2b.Code -eq 0) -and $r2b.Adopted -and ($stageLine2b -ceq $expectedWarpLine) -and (Test-Path $r2b.Out)) "exit=$($r2b.Code) adopted=$($r2b.Adopted) stageLine=[$stageLine2b] png=$(Test-Path $r2b.Out) stderr=[$($r2b.Err.Trim())]"
$hashEq2 = $false
$hA2 = '(missing)'
$hB2 = '(missing)'
if ((Test-Path $r2a.Out) -and (Test-Path $r2b.Out)) {
    $hA2 = (Get-FileHash $r2a.Out -Algorithm SHA256).Hash
    $hB2 = (Get-FileHash $r2b.Out -Algorithm SHA256).Hash
    $hashEq2 = ($hA2 -ceq $hB2)
}
Check 'S2c warp determinism: two warp dumps whole-file SHA256-identical' $hashEq2 "a=$hA2 b=$hB2"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the correctness core. Reference = the warp dump (S2, untransformed
# master pixels) through the mscms sRGB->display transform; actual = the
# hw dump (S1, the D2D ColorManagement effect applied). The D2D effect
# runs over the whole composited viewport (gpu.rs, one post-composition
# pass), so the compare is whole-frame. Tolerance 24 is the initial gate:
# the measured max is reported verbatim; relaxing it is a human decision,
# not the smoke's.
# ---------------------------------------------------------------------------
$TOL = 24
$srgbPath = ''
$srgbBytes = $null
$dispBytes = $null
$refBytes = $null
$hwQ = $null
$warpQ = $null
$dimOk3 = $false
if ($pngOk1 -and (Test-Path $r2a.Out)) {
    $hwQ = [Px]::Load($r1.Out)
    $warpQ = [Px]::Load($r2a.Out)
    $dimOk3 = ($hwQ.W -eq $warpQ.W) -and ($hwQ.H -eq $warpQ.H) -and ($hwQ.W -eq $r1.Vs[0]) -and ($hwQ.H -eq $r1.Vs[1])
}
if ($profFile -ne '') { $dispBytes = [IO.File]::ReadAllBytes($profFile) }
$srgbPath = [RefXform]::StandardSrgbPath()
if ($srgbPath -and (Test-Path $srgbPath)) { $srgbBytes = [IO.File]::ReadAllBytes($srgbPath) }
if ($dimOk3 -and ($null -ne $srgbBytes) -and ($null -ne $dispBytes)) {
    $refBytes = [RefXform]::Apply($srgbBytes, $dispBytes, $warpQ.B, $warpQ.W, $warpQ.H)
}
Check 'S3a reference prerequisites: dumps same size, sRGB + display profiles read, mscms transform ran' ($dimOk3 -and ($null -ne $refBytes)) "dimsOk=$dimOk3 hw=$($hwQ.W)x$($hwQ.H) warp=$($warpQ.W)x$($warpQ.H) srgbPath=[$srgbPath] profFile=[$profFile] refBuilt=$($null -ne $refBytes)"
$maxDiff = -1
$diffPixels = -1
if ($null -ne $refBytes) {
    $cmp = [RefXform]::Compare($hwQ.B, $refBytes)
    $maxDiff = $cmp[0]
    $diffPixels = $cmp[1]
}
Check "S3b hw dump matches the mscms reference within tolerance (max channel diff <= $TOL)" (($maxDiff -ge 0) -and ($maxDiff -le $TOL)) "measured max channel diff=$maxDiff tolerance=$TOL diffPixels=$diffPixels"
Check 'S3c the transform actually moved pixels (differing pixel count > 0)' ($diffPixels -gt 0) "diffPixels=$diffPixels maxDiff=$maxDiff"

# ---------------------------------------------------------------------------
# S4: breadcrumb shape (never triggered on the clean path): the hw stderr
# carries no 'display effect failure'. A latched segment would ALSO have
# flipped the S1 line to none, so S1a and this check bracket the ratchet
# from both ends.
# ---------------------------------------------------------------------------
Check 'S4 hw stderr has no display effect failure breadcrumb (ratchet untouched)' (-not $r1.Err.Contains('display effect failure')) "stderr=[$($r1.Err.Trim())]"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# Teardown: kill any staged leftover, then delete the stage dir - unless
# something failed (stderr captures + dump PNGs are the evidence, the
# smoke80 convention).
# ---------------------------------------------------------------------------
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $RunExe })
if ($leftover) { $leftover | Stop-Process -Force }
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue }
else { Write-Host ('FAILURES: evidence kept in ' + $Stage) }
Write-Host ('SUMMARY smoke130 pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
if ($script:fail -gt 0) { Write-Host 'SMOKE130 RESULT: FAIL'; exit 1 }
Write-Host 'SMOKE130 RESULT: PASS'
exit 0
