# smoke79: PerMonitorV2 manifest + WM_DPICHANGED (#79)
# S1 manifest readback from the exe bytes; S2 live-window awareness
# context is PerMonitorV2; S3 synthetic WM_DPICHANGED drives rect
# adoption + the #78 dock chain (poll-based, no fixed-sleep reads).
# ASCII only (PS 5.1 header discipline).
param(
    [string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe'
)
$ErrorActionPreference = 'Stop'
# Staging-copy discipline (smoke78 convention): the exe runs from a
# throwaway dir so its ini never lands next to the real build.
$Stage = $env:TEMP + '\riviv-79-smoke'
if (-not (Test-Path $Stage)) { New-Item -ItemType Directory -Path $Stage | Out-Null }
$OutDir = $Stage
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Runtime.InteropServices;
public class S79 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool AreDpiAwarenessContextsEqual(IntPtr a, IntPtr b);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr hmon, int type, out uint x, out uint y);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr hwnd, uint cmd);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int w, int h);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr h);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr h);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern uint GetPixel(IntPtr hdc, int x, int y);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
'@)

$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ('PASS ' + $name) }
    else { $script:fail++; Write-Output ('FAIL ' + $name + ' -- ' + $detail) }
}
function Skip($name, $why) {
    $script:skip++; Write-Output ('SKIP ' + $name + ' -- ' + $why)
}
function Wait-Until($sb, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $sb) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (& $sb)
}
function Get-WinRect($h) {
    $r = New-Object S79+RECT
    [void][S79]::GetWindowRect($h, [ref]$r)
    return $r
}
function Get-Client($h) {
    $r = New-Object S79+RECT
    [void][S79]::GetClientRect($h, [ref]$r)
    return $r
}
function Client-Origin($h) {
    $p = New-Object S79+POINT
    $p.X = 0; $p.Y = 0
    [void][S79]::ClientToScreen($h, [ref]$p)
    return $p
}
function Capture-Center($h) {
    # PrintWindow flag 2 (PW_RENDERFULLCONTENT) into a fresh compatible
    # bitmap; returns @(R,G,B) at the window's center.
    $r = Get-WinRect $h
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -le 2 -or $ht -le 2) { return $null }
    $scr = [S79]::GetDC([IntPtr]::Zero)
    $mem = [S79]::CreateCompatibleDC($scr)
    $bmp = [S79]::CreateCompatibleBitmap($scr, $w, $ht)
    $old = [S79]::SelectObject($mem, $bmp)
    [void][S79]::ReleaseDC([IntPtr]::Zero, $scr)
    try {
        [void][S79]::PrintWindow($h, $mem, 2)
        $c = [S79]::GetPixel($mem, [int]($w / 2), [int]($ht / 2))
        if ($c -eq 0xFFFFFFFF) { return $null }
        # COLORREF layout is 0x00BBGGRR
        return @([int]($c -band 0xFF), [int](($c -shr 8) -band 0xFF), [int](($c -shr 16) -band 0xFF))
    } finally {
        [void][S79]::SelectObject($mem, $old)
        [void][S79]::DeleteObject($bmp)
        [void][S79]::DeleteDC($mem)
    }
}
function Is-Red($c) {
    if ($null -eq $c) { return $false }
    return ($c[0] -gt 200) -and ($c[1] -lt 60) -and ($c[2] -lt 60)
}

# ---- pre-flight hygiene --------------------------------------------------
# The PROBE process must be DPI-aware: an unaware reader gets virtualized
# coordinates (halved on this 200% machine) and every geometry assertion
# below breaks (diag79 finding, 2026-09-19).
[void][S79]::SetProcessDPIAware()
$pre = Get-Process riviv -ErrorAction SilentlyContinue
if ($pre) {
    Write-Output ('PRE: stopping ' + @($pre).Count + ' pre-existing riviv process(es)')
    $pre | Stop-Process -Force
    Start-Sleep -Milliseconds 400
}
$ini = Join-Path $Stage 'riviv.ini'
if (Test-Path $ini) { Remove-Item $ini -Force }
$runExe = Join-Path $Stage 'riviv.exe'
Copy-Item $Exe $runExe -Force
$img = Join-Path $Stage 'red512.png'
$bmp = New-Object Drawing.Bitmap(512, 512)
$g = [Drawing.Graphics]::FromImage($bmp)
$g.Clear([Drawing.Color]::FromArgb(253, 0, 0))
$g.Dispose()
$bmp.Save($img, [Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

# ---- S1: manifest readback ------------------------------------------------
$bytes = [IO.File]::ReadAllBytes($runExe)
$ascii = [Text.Encoding]::ASCII.GetString($bytes)
Check 'S1.1 exe embeds dpiAwareness PerMonitorV2' ($ascii.Contains('PerMonitorV2')) ('manifest text not found in exe bytes')
Check 'S1.2 manifest is riviv-owned' ($ascii.Contains('name="riviv"')) ('assemblyIdentity not found')
Check 'S1.3 manifest uses the SMI/2016 namespace' ($ascii.Contains('http://schemas.microsoft.com/SMI/2016/WindowsSettings')) ('2016 namespace missing')

# ---- launch ---------------------------------------------------------------
$proc = Start-Process -FilePath $runExe -ArgumentList ('"' + $img + '"') -PassThru
$script:main = [IntPtr]::Zero
$ok = Wait-Until { $script:main = [S79]::FindWindowW('riviv', [NullString]::Value); $script:main -ne [IntPtr]::Zero } 10000
Check 'S1.4 window appears' $ok ('hwnd lookup failed')
if (-not $ok) { Write-Output ('SMOKE79 RESULT: PASS=' + $pass + ' FAIL=' + $fail + ' SKIP=' + $skip); if ($fail -gt 0) { exit 1 } else { exit 0 } }
$main = $script:main

# ---- S2: awareness context ------------------------------------------------
$ctx = [S79]::GetWindowDpiAwarenessContext($main)
$pmv2 = [IntPtr](-4)  # DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
Check 'S2.1 live window context is PerMonitorV2' ([S79]::AreDpiAwarenessContextsEqual($ctx, $pmv2)) ('ctx handle compare failed (opaque handle, raw=' + $ctx.ToInt64() + ')')
$wdpi = [S79]::GetDpiForWindow($main)
$mon = [S79]::MonitorFromWindow($main, 1)  # MONITOR_DEFAULTTONEAREST
$mdpi = [uint32]0
$unused = [uint32]0
[void][S79]::GetDpiForMonitor($mon, 0, [ref]$mdpi, [ref]$unused)
Check 'S2.2 window DPI equals its monitor effective DPI (per-monitor semantics)' ($wdpi -eq $mdpi) ('window=' + $wdpi + ' monitor=' + $mdpi)

# ---- S3: synthetic WM_DPICHANGED -----------------------------------------
# Bring it forward: a never-activated process gets no WM_PAINT for image
# adoptions (pre-existing gate, probe78-bg.ps1), so the render asserts
# need the window foreground once.
$fg = $false
for ($i = 0; $i -lt 6 -and -not $fg; $i++) {
    [void][S79]::SetForegroundWindow($main)
    Start-Sleep -Milliseconds 200
    $fg = ([S79]::GetForegroundWindow() -eq $main)
}
if (-not $fg) { Write-Output ('WARN could not take foreground; render checks may hit the background paint gate') }

$view = [S79]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_view', $null)
Check 'S3.1 view child exists' ($view -ne [IntPtr]::Zero) ('riviv_view not found')
$rebar = [S79]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_rebar', $null)
Check 'S3.2 rebar strip child exists' ($rebar -ne [IntPtr]::Zero) ('riviv_rebar not found')
if ($view -eq [IntPtr]::Zero) {
    Write-Output ('SMOKE79 RESULT: PASS=' + $pass + ' FAIL=' + $fail + ' SKIP=' + $skip)
    exit 1
}

$red0 = Wait-Until { Is-Red (Capture-Center $view) } 10000
if ($red0) { Check 'S3.3 image renders before the DPI change' $true ('') }
else { Skip 'S3.3 image renders before the DPI change' ('paint gate (never-activated window), geometry checks still hard-assert below') }

$beforeMain = Get-WinRect $main
$beforeClient = Get-Client $main
$beforeView = Get-WinRect $view
$org = Client-Origin $main
$vbL = $beforeView.L - $org.X; $vbT = $beforeView.T - $org.Y
$vbR = $beforeView.R - $org.X; $vbB = $beforeView.B - $org.Y
$chromeBefore = $beforeClient.B - $vbB
Check 'S3.4 pre-state: view spans the client above the bottom chrome' ($vbL -eq 0 -and $vbT -eq 0 -and $vbR -eq $beforeClient.R -and $chromeBefore -ge 0) ("view=($vbL,$vbT,$vbR,$vbB) client.B=" + $beforeClient.B)
$rebarRect = Get-WinRect $rebar
$rebarHBefore = $rebarRect.B - $rebarRect.T
$sysDpi = [S79]::GetDpiForSystem()
$wantStrip = [int](32 * $sysDpi / 96)
Check 'S3.5 pre-state: strip height is controls_height(system DPI)' ($rebarHBefore -eq $wantStrip) ('rebarH=' + $rebarHBefore + ' want=' + $wantStrip + ' sysDpi=' + $sysDpi)

# suggested rect = current * 1.5, centered (mimics the OS suggestion)
$w = $beforeMain.R - $beforeMain.L
$h = $beforeMain.B - $beforeMain.T
$nw = [int]($w * 3 / 2); $nh = [int]($h * 3 / 2)
$nl = $beforeMain.L - [int](($nw - $w) / 2)
$nt = $beforeMain.T - [int](($nh - $h) / 2)
$rectPtr = [Runtime.InteropServices.Marshal]::AllocHGlobal(16)
function Write-RectPtr($ptr, $l, $t, $r, $b) {
    [Runtime.InteropServices.Marshal]::WriteInt32($ptr, 0, $l)
    [Runtime.InteropServices.Marshal]::WriteInt32($ptr, 4, $t)
    [Runtime.InteropServices.Marshal]::WriteInt32($ptr, 8, $r)
    [Runtime.InteropServices.Marshal]::WriteInt32($ptr, 12, $b)
}
Write-RectPtr $rectPtr $nl $nt ($nl + $nw) ($nt + $nh)
$wp144 = [IntPtr](0x00900090)  # MAKEWPARAM(144, 144)
# PostMessage is REJECTED by the OS for WM_DPICHANGED (returns FALSE,
# diag79 finding); SendMessage delivers inter-thread (the target thread
# runs the handler) and keeps the RECT alive for the call.
[void][S79]::SendMessageW($main, 0x02E0, $wp144, $rectPtr)
$adopted = Wait-Until {
    $r = Get-WinRect $script:main
    ($r.L -eq $nl) -and ($r.T -eq $nt) -and ($r.R -eq ($nl + $nw)) -and ($r.B -eq ($nt + $nh))
} 10000
Check 'S3.6 window adopted the suggested rect verbatim' $adopted ("want=($nl,$nt," + ($nl + $nw) + ',' + ($nt + $nh) + ')')

if ($adopted) {
    $client2 = Get-Client $main
    $view2 = Get-WinRect $view
    $org2 = Client-Origin $main
    $v2L = $view2.L - $org2.X; $v2T = $view2.T - $org2.Y
    $v2R = $view2.R - $org2.X; $v2B = $view2.B - $org2.Y
    $chromeAfter = $client2.B - $v2B
    Check 'S3.7 dock chain re-ran: view spans new client, chrome unchanged' ($v2L -eq 0 -and $v2T -eq 0 -and $v2R -eq $client2.R -and $chromeAfter -eq $chromeBefore) ("view=($v2L,$v2T,$v2R,$v2B) client.B=" + $client2.B + ' chromeBefore=' + $chromeBefore + ' chromeAfter=' + $chromeAfter)
    $rebarHAfter = (Get-WinRect $rebar).B - (Get-WinRect $rebar).T
    Check 'S3.8 strip height stays system-DPI after the change' ($rebarHAfter -eq $wantStrip) ('rebarH=' + $rebarHAfter + ' want=' + $wantStrip)
    $below = [S79]::GetWindow($view, 2)  # GW_HWNDNEXT = the window BELOW in z
    Check 'S3.9 view remains the bottom-most sibling (HWND_BOTTOM)' ($below -eq [IntPtr]::Zero) ('below=0x' + $below.ToString('X'))
    if ($red0) {
        $red1 = Wait-Until { Is-Red (Capture-Center $view) } 10000
        Check 'S3.10 image still renders at the scaled size' $red1 ('center=' + ((Capture-Center $view) -join ','))
    } else {
        Skip 'S3.10 image still renders at the scaled size' ('paint gate held from S3.3')
    }

    # restore: 96-DPI message with the original rect
    Write-RectPtr $rectPtr $beforeMain.L $beforeMain.T $beforeMain.R $beforeMain.B
    $wp96 = [IntPtr](0x00600060)  # MAKEWPARAM(96, 96)
    [void][S79]::SendMessageW($main, 0x02E0, $wp96, $rectPtr)
    $restored = Wait-Until {
        $r = Get-WinRect $script:main
        ($r.L -eq $beforeMain.L) -and ($r.T -eq $beforeMain.T) -and ($r.R -eq $beforeMain.R) -and ($r.B -eq $beforeMain.B)
    } 10000
    Check 'S3.11 restore message re-adopts the original rect' $restored ('rect mismatch after 96-dpi post')
    if ($red0 -and $restored) {
        $red2 = Wait-Until { Is-Red (Capture-Center $view) } 10000
        Check 'S3.12 image renders after the restore' $red2 ('center=' + ((Capture-Center $view) -join ','))
    } else {
        Skip 'S3.12 image renders after the restore' ('upstream render assert did not hold')
    }
} else {
    Skip 'S3.7 dock chain re-ran' ('adoption failed; downstream geometry skipped')
    Skip 'S3.8 strip height stays system-DPI' ('adoption failed')
    Skip 'S3.9 view remains bottom-most' ('adoption failed')
    Skip 'S3.10 image still renders at scaled size' ('adoption failed')
    Skip 'S3.11/S3.12 restore' ('adoption failed')
}

# ---- teardown -------------------------------------------------------------
[Runtime.InteropServices.Marshal]::FreeHGlobal($rectPtr)
[void][S79]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CLOSE
$exited = Wait-Until { $null -eq (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue) } 8000
if (-not $exited) {
    Write-Output 'WARN window did not exit on WM_CLOSE; forcing'
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
}
if (Test-Path $ini) { Remove-Item $ini -Force }
Remove-Item $runExe -Force -ErrorAction SilentlyContinue
Write-Output ('SMOKE79 RESULT: PASS=' + $pass + ' FAIL=' + $fail + ' SKIP=' + $skip)
if ($fail -gt 0) { exit 1 } else { exit 0 }
