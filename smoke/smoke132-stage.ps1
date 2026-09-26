# smoke132 - #132 M8-4 profile hot reload: the freshness channel must be
# IDEMPOTENT while the judge's raw name and the window's monitor stand
# still - one display-stage breadcrumb, output_gen=1, no ratchet noise,
# across a synthetic WM_DISPLAYCHANGE, two-plus freshness-timer periods,
# and a same-monitor move. The real-switch path (profile A -> B) is NOT
# smoked: flipping the machine's display profile is too invasive for an
# automated harness; its evidence lives in the ticket's probe (design
# comment) and the QA checklist (manual). ASCII-only source (PS 5.1
# encoding trap, #79 lesson). Harness cribbed from smoke130-stage.ps1
# (staged-exe discipline, poll-based waits, one return value per
# function, narration via Write-Host only).
#
# Machine pin: same as smoke130 - the hw arm expects
# display-stage=gpu_effect on the Custom-profile smoke machine; on an
# sRGB-profile machine the hw line says none and this smoke FAILS
# honestly (the idempotence contract still holds either way; the pin is
# deliberate so the hw arm exercises the effect-present shape).
#
# Scenarios:
#   S0  stage the exe; FATAL-exit 3 when a FOREIGN riviv is alive (#109).
#   S1  hw arm: post a synthetic WM_DISPLAYCHANGE mid-session (same
#       profile) -> exactly ONE display-stage breadcrumb, dump
#       output_gen=1 (the freshness check no-ops on an unchanged name).
#   S2  hw arm: idle 5s (>=2 freshness-timer periods) -> ONE breadcrumb,
#       gen=1 (the 2s poll must not spam the smoke channel).
#   S3  hw arm: same-monitor move via SetWindowPos(+80,+80) -> ONE
#       breadcrumb, gen=1, AND the ini lands x=140 y=140 (the WM_MOVE
#       arm demonstrably ran - the monitor compare did not re-establish
#       on the same monitor).
#   S4  warp arm: post WM_DISPLAYCHANGE -> ONE breadcrumb, gen=1 (the
#       none-stage arm of the same contract).
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Drawing;
using System.Drawing.Imaging;
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
    // The smoke126 HashSource recipe (300x200 hash300.png).
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
'@) -ReferencedAssemblies @('System.Drawing')
[void][S80]::SetProcessDPIAware()

$script:pass = 0; $script:fail = 0; $script:skip = 0
function Check($name, $ok, $detail) {
    # Narration only - Write-Host keeps the return channel clean. The
    # detail line is ALWAYS printed (measured values are evidence).
    if ($ok) { $script:pass++; Write-Host ('PASS ' + $name + ' -- ' + $detail) }
    else { $script:fail++; Write-Host ('FAIL ' + $name + ' -- ' + $detail) }
}

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$WM_DISPLAYCHANGE = 0x007E
$CMD_ONE2ONE = 45      # menu.rs Cmd::ViewOneToOne.id()

function Kill-Riviv {
    # #109: only the staged copy is ever killed; the developer's real
    # viewer never matches the deterministic stage path.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $RunExe })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}

$Stage = Join-Path $env:TEMP 'riviv-132-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'

# ---------------------------------------------------------------------------
# S0: the stage (foreign-riviv check first - a real viewer would corrupt
# every result below).
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
    # PS 5.1 rejects an EMPTY -ArgumentList; pass it only when present.
    # The raw handle is captured WHILE ALIVE (-PassThru loses .Handle).
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
    $script:RawH = $script:RawP.Handle
    return $script:RawP
}
function Read-Err($errName) {
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
function Get-Line($err, $prefix) {
    if ($null -eq $err) { return '' }
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t.StartsWith($prefix)) { return $t }
    }
    return ''
}
function Count-Lines($err, $prefix) {
    if ($null -eq $err) { return 0 }
    $n = 0
    foreach ($ln in ($err -split "`n")) {
        if (($ln.TrimEnd("`r")).StartsWith($prefix)) { $n++ }
    }
    return $n
}
function Get-Stage-Line($err, $stage, $backend) {
    if ($null -eq $err) { return '' }
    $want = ('riviv: display-stage=' + $stage + ' profile=')
    $tail = (' backend=' + $backend)
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t.StartsWith($want) -and $t.EndsWith($tail)) { return $t }
    }
    return ''
}
function Profile-Of($stageLine) {
    if ($stageLine -match '^riviv: display-stage=\S+ profile=(.*) backend=\S+$') {
        return $Matches[1]
    }
    return ''
}

# One adopt -> mid-action -> WM_CLOSE dump instance. $midAction receives
# the main window handle after adoption; returns the closed instance.
function Run-Case($argStr, $outName, $errName, $midAction) {
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    Reset-Ini $script:BaseIni
    $p = Start-Riv $argStr $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p 'hash300' 12000
    if (($main -ne [IntPtr]::Zero) -and ($null -ne $midAction)) {
        & $midAction $main
    }
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Adopted = $adopted; Main = $main; Err = (Read-Err $errName) }
}

$script:BaseIni = "[riviv]`r`nx=60`r`ny=60`r`nwide=940`r`nhigh=720`r`nauto_zoom=0`r`nicm=0`r`nrenderer=auto`r`n"
$hash300 = Join-Path $Stage 'hash300.png'
[Px]::SaveRgba($hash300, [Px]::HashSource(300, 200), 300, 200)

Kill-Riviv

# ---------------------------------------------------------------------------
# S1: a synthetic WM_DISPLAYCHANGE against an UNCHANGED profile - the
# freshness check must no-op (one breadcrumb, gen stays 1).
# ---------------------------------------------------------------------------
$script:postDisplaychange = {
    param($m)
    [void][S80]::PostMessage($m, $WM_DISPLAYCHANGE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 1500
}
$r1 = Run-Case ("`"$hash300`" -renderer d2d -dump-viewport `"$(Join-Path $Stage 's1.png')`"") 's1.png' 's1.err' $script:postDisplaychange
$stage1 = Get-Stage-Line $r1.Err 'gpu_effect' 'hw'
$gen1 = Get-Line $r1.Err 'riviv: dump-viewport output_gen='
$nStage1 = Count-Lines $r1.Err 'riviv: display-stage='
Check 'S1 hw: synthetic WM_DISPLAYCHANGE keeps exactly one breadcrumb, gen=1' `
    (($r1.Code -eq 0) -and $r1.Adopted -and ($stage1 -ne '') -and ($nStage1 -eq 1) -and ($gen1 -ceq 'riviv: dump-viewport output_gen=1')) `
    ("exit=$($r1.Code) adopted=$($r1.Adopted) stage=[$stage1] breadcrumbs=$nStage1 gen=[$gen1]")

# ---------------------------------------------------------------------------
# S2: two-plus freshness-timer periods of idle - the 2s poll must not
# re-establish (one breadcrumb, gen=1) and must not feed the ratchet.
# ---------------------------------------------------------------------------
$script:idle5s = {
    param($m)
    Start-Sleep -Milliseconds 5000
}
$r2 = Run-Case ("`"$hash300`" -renderer d2d -dump-viewport `"$(Join-Path $Stage 's2.png')`"") 's2.png' 's2.err' $script:idle5s
$stage2 = Get-Stage-Line $r2.Err 'gpu_effect' 'hw'
$gen2 = Get-Line $r2.Err 'riviv: dump-viewport output_gen='
$nStage2 = Count-Lines $r2.Err 'riviv: display-stage='
$noRatchet2 = -not $r2.Err.Contains('display effect failure')
Check 'S2 hw: 5s idle (>=2 timer periods) keeps one breadcrumb, gen=1, ratchet untouched' `
    (($r2.Code -eq 0) -and $r2.Adopted -and ($stage2 -ne '') -and ($nStage2 -eq 1) -and ($gen2 -ceq 'riviv: dump-viewport output_gen=1') -and $noRatchet2) `
    ("exit=$($r2.Code) adopted=$($r2.Adopted) breadcrumbs=$nStage2 gen=[$gen2] ratchet=$noRatchet2")

# ---------------------------------------------------------------------------
# S3: a same-monitor move - the WM_MOVE arm runs (the ini records the new
# x/y at close) but the monitor compare does not re-establish (one
# breadcrumb, gen=1).
# ---------------------------------------------------------------------------
$script:moveSameMonitor = {
    param($m)
    $wr = New-Object S80+RECT
    [void][S80]::GetWindowRect($m, [ref]$wr)
    [void][S80]::SetWindowPos($m, [IntPtr]::Zero, ($wr.L + 80), ($wr.T + 80), 0, 0, 0x0015)  # SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE
    Start-Sleep -Milliseconds 800
    $script:s3Moved = $false
    $wr2 = New-Object S80+RECT
    [void][S80]::GetWindowRect($m, [ref]$wr2)
    $script:s3Dx = $wr2.L - $wr.L
    $script:s3Dy = $wr2.T - $wr.T
    if (($script:s3Dx -eq 80) -and ($script:s3Dy -eq 80)) { $script:s3Moved = $true }
}
$r3 = Run-Case ("`"$hash300`" -renderer d2d -dump-viewport `"$(Join-Path $Stage 's3.png')`"") 's3.png' 's3.err' $script:moveSameMonitor
$stage3 = Get-Stage-Line $r3.Err 'gpu_effect' 'hw'
$gen3 = Get-Line $r3.Err 'riviv: dump-viewport output_gen='
$nStage3 = Count-Lines $r3.Err 'riviv: display-stage='
$iniText = ''
if (Test-Path $Ini) { $iniText = [IO.File]::ReadAllText($Ini) }
$iniX = if ($iniText -match '(?m)^x=(\d+)\r?$') { $Matches[1] } else { '' }
$iniY = if ($iniText -match '(?m)^y=(\d+)\r?$') { $Matches[1] } else { '' }
Check 'S3 hw: same-monitor move ran WM_MOVE (ini x=140 y=140) without re-establishing' `
    (($r3.Code -eq 0) -and $r3.Adopted -and ($stage3 -ne '') -and ($nStage3 -eq 1) -and ($gen3 -ceq 'riviv: dump-viewport output_gen=1') -and $script:s3Moved -and ($iniX -eq '140') -and ($iniY -eq '140')) `
    ("exit=$($r3.Code) moved=$($script:s3Moved) dx=$($script:s3Dx) dy=$($script:s3Dy) breadcrumbs=$nStage3 gen=[$gen3] ini=[$iniX,$iniY]")

# ---------------------------------------------------------------------------
# S4: the warp arm of the same contract - none-stage breadcrumb, the
# synthetic message still no-ops (one breadcrumb, gen=1).
# ---------------------------------------------------------------------------
$r4 = Run-Case ("`"$hash300`" -renderer warp -dump-viewport `"$(Join-Path $Stage 's4.png')`"") 's4.png' 's4.err' $script:postDisplaychange
$stage4 = Get-Stage-Line $r4.Err 'none' 'warp'
$gen4 = Get-Line $r4.Err 'riviv: dump-viewport output_gen='
$nStage4 = Count-Lines $r4.Err 'riviv: display-stage='
Check 'S4 warp: synthetic WM_DISPLAYCHANGE keeps exactly one breadcrumb, gen=1' `
    (($r4.Code -eq 0) -and $r4.Adopted -and ($stage4 -ne '') -and ($nStage4 -eq 1) -and ($gen4 -ceq 'riviv: dump-viewport output_gen=1')) `
    ("exit=$($r4.Code) adopted=$($r4.Adopted) stage=[$stage4] breadcrumbs=$nStage4 gen=[$gen4]")

Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# Teardown: kill staged leftovers, delete the stage dir unless something
# failed (stderr captures + dump PNGs are the evidence).
# ---------------------------------------------------------------------------
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $RunExe })
if ($leftover) { $leftover | Stop-Process -Force }
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue }
else { Write-Host ('FAILURES: evidence kept in ' + $Stage) }
Write-Host ('SUMMARY smoke132 pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
if ($script:fail -gt 0) { Write-Host 'SMOKE132 RESULT: FAIL'; exit 1 }
Write-Host 'SMOKE132 RESULT: PASS'
exit 0
