# smoke126 - #126 the -renderer <auto|d2d|warp> CLI switch + the renderer and
# dump stderr channel. ASCII-only source (PS 5.1 ANSI/param-binding trap, #79
# lesson). Harness cribbed from smoke80-d2d.ps1: staged-exe discipline (every
# instance runs from a copy of the exe in %TEMP%\riviv-126-smoke; the staged
# ini is rewritten before every launch because WM_CLOSE saves config - a
# leftover key would fake cross-scenario regressions); close is always
# WM_CLOSE (PostMessage to the owner) so the close-time dump runs; exit codes
# are read via GetExitCodeProcess on the raw handle captured WHILE ALIVE
# (the -PassThru object loses .Handle after exit). Waits are poll-based
# (synthetic-message latency is unbounded under load); the only fixed sleeps
# are short settle delays after adoption/commands. Every function returns
# EXACTLY ONE value; narration goes through Write-Host/Write-Output only.
#
# The observable surface (stderr, exact forms):
#   riviv: renderer=<request> backend=<d2d/hw|d2d/warp>
#   riviv: d2d max_bitmap=<n> tile cap=<n> (dxgi budget=<n|unavailable>)
#   riviv: dump-viewport output_gen=<n>
#   riviv: -renderer <kind> ignored - device already built (<backend>)
#
# Scenarios:
#   S0  stage the exe; FATAL-exit when a FOREIGN riviv is alive (#109: a
#       real viewer corrupts handoff/baseline results).
#   S1  force-warp via the switch (ini auto): exact startup line, dump line
#       output_gen=1, viewport-sized PNG at close.
#   S1b the switch overrides the ini in both directions (auto over warp,
#       warp over d2d).
#   S2  hw vs warp 1:1 byte equality: same fixture, same calibrated 900x600
#       view, WM_COMMAND 45 (1:1) in both arms, dumps whole-file identical.
#   S3  the override is never persisted: the ini still reads renderer=auto.
#   S4  handoff: second instance forwards and exits 0; the first prints the
#       ignore breadcrumb and keeps its original startup line.
#   S5  dangling "-renderer" / unknown value: intent unarmed, ini governs,
#       no ignore breadcrumb, clean close (no usage modal blocking close).
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
    public static byte[] Solid(int w, int h, int r, int g, int b) {
        byte[] a = new byte[w * h * 4];
        for (int i = 0; i < a.Length; i += 4) { a[i] = (byte)r; a[i + 1] = (byte)g; a[i + 2] = (byte)b; a[i + 3] = 255; }
        return a;
    }
    public static byte[] Source(int w, int h) {
        byte[] a = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int r, g, b;
            if (x == 0 || y == 0 || x == w - 1 || y == h - 1) { r = 255; g = 0; b = 0; }
            else if (x < w / 2 && y < h / 2) { r = 0; g = 180; b = 0; }
            else if (y < h / 2) { r = 0; g = 0; b = 220; }
            else if (x < w / 2) { r = 230; g = 230; b = 0; }
            else { r = 200; g = 0; b = 200; }
            int i = (y * w + x) * 4;
            a[i] = (byte)r; a[i + 1] = (byte)g; a[i + 2] = (byte)b; a[i + 3] = 255;
        }
        return a;
    }
    // Deterministic varied-color pattern with a 1px black border ring -
    // the #90 golden90 fixture recipe (identical to smoke81's Px.HashSource
    // so a corpus stays reproducible from either harness).
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
$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ('PASS ' + $name) }
    else { $script:fail++; Write-Output ('FAIL ' + $name + ' -- ' + $detail) }
}
function Skip-Scenario($name, $why) {
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

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$CMD_ONE2ONE = 45      # menu.rs Cmd::ViewOneToOne.id()

function Kill-Riviv {
    # #109: match the smoke98 path-targeted contract - the developer's real
    # viewer must survive every teardown. $RunExe is the staged copy (same
    # deterministic stage path across runs, so a leftover from a crashed run
    # still gets killed here); a real viewer never matches it.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $RunExe })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}

$Stage = Join-Path $env:TEMP 'riviv-126-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'

# ---------------------------------------------------------------------------
# S0: the stage. The foreign-riviv check fires BEFORE anything else: a real
# viewer running would eat the handoff line (S4) and corrupt the baselines.
# ---------------------------------------------------------------------------
$preExisting = @(Get-Process riviv -ErrorAction SilentlyContinue)
$foreign = @($preExisting | Where-Object { $_.Path -ne $RunExe })
if ($foreign.Count -gt 0) {
    $fxPids = ($foreign | ForEach-Object { $_.Id }) -join ','
    Write-Output ('FATAL foreign riviv running - close it and rerun (pids: ' + $fxPids + ')')
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
function Start-Riv($argStr, $errName, [bool]$minimized) {
    # PS 5.1's Start-Process rejects an EMPTY -ArgumentList string, so the
    # parameter is only passed when there is one. The raw process handle is
    # captured WHILE ALIVE: the -PassThru object loses .Handle (and with it
    # ExitCode) once the process exits, so Close-Main reads the exit code
    # via GetExitCodeProcess on this handle.
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    if ($minimized) {
        if ($argStr -eq '') {
            $script:RawP = Start-Process -FilePath $RunExe -PassThru -RedirectStandardError $errPath -WindowStyle Minimized
        } else {
            $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath -WindowStyle Minimized
        }
    } else {
        if ($argStr -eq '') {
            $script:RawP = Start-Process -FilePath $RunExe -PassThru -RedirectStandardError $errPath
        } else {
            $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
        }
    }
    $script:RawH = $script:RawP.Handle
    return $script:RawP
}
function Read-Err($errName) {
    # S4 polls this file WHILE the writer is alive, so open with FileShare
    # ReadWrite (a plain Get-Content can lose the share race against the
    # live stderr redirect).
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
# First whole line of $err that starts with $prefix ('' when absent). Exact
# string assertions ride on this, so a line must never be prefix-matched by
# two different checks.
function Get-Line($err, $prefix) {
    if ($null -eq $err) { return '' }
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t.StartsWith($prefix)) { return $t }
    }
    return ''
}

# Calibrate the main window until the riviv_view child's client rect is
# EXACTLY tw x th (chrome and the status bar are integer pixels, so the
# delta walk converges; same recipe as smoke80/81). Returns the achieved size.
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

# One adopt -> (calibrate) -> (commands) -> WM_CLOSE dump instance. $argStr
# is the full command line (quoted paths included); $tw/$th calibrate the
# riviv_view child to EXACTLY that size (byte-compare scenes need exact
# geometry), $null records the as-is view size at close time.
function Run-Dump($argStr, $imgLeaf, $iniText, $outName, $errName, $cmds, $tw, $th) {
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    Reset-Ini $iniText
    $p = Start-Riv $argStr $errName $false
    $main = Wait-Main $p
    $adopted = Wait-Title $p $imgLeaf 12000
    $vs = @(0, 0)
    if ($main -ne [IntPtr]::Zero) {
        if ($null -ne $tw) { $vs = Calibrate-View $main $tw $th }
        else { $vs = View-Size (View-Of $main) }
        if ($cmds) { & $cmds $main }
        Start-Sleep -Milliseconds 500
    }
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Vs = $vs; Adopted = $adopted; Main = $main; Err = (Read-Err $errName) }
}
$one2one = { param($m) [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }

$BaseRect = "[riviv]`r`nx=60`r`ny=60`r`nwide=1000`r`nhigh=700`r`nauto_zoom=0`r`nicm=0`r`n"
$hash300 = Join-Path $Stage 'hash300.png'
[Px]::SaveRgba($hash300, [Px]::HashSource(300, 200), 300, 200)

Kill-Riviv

# ---------------------------------------------------------------------------
# S1: force-warp via the switch (ini says auto), dump at close, output_gen=1.
# ---------------------------------------------------------------------------
$s1ini = $BaseRect + "renderer=auto`r`n"
$s1out = Join-Path $Stage 's1-out.png'
$r1 = Run-Dump ("`"$hash300`" -renderer warp -dump-viewport `"$s1out`"") 'hash300' $s1ini 's1-out.png' 's1.err' $null $null $null
$err1 = $r1.Err
$line1 = Get-Line $err1 'riviv: renderer='
$genLine1 = Get-Line $err1 'riviv: dump-viewport output_gen='
$maxLine1 = Get-Line $err1 'riviv: d2d max_bitmap='
$pngOk1 = Test-Path $r1.Out
$dims1 = '(none)'
$sizeOk1 = $false
if ($pngOk1) {
    $q1 = [Px]::Load($r1.Out)
    $dims1 = "$($q1.W)x$($q1.H)"
    $sizeOk1 = ($q1.W -eq $r1.Vs[0]) -and ($q1.H -eq $r1.Vs[1])
}
Check 'S1a force-warp: exit 0, exact "renderer=warp backend=d2d/warp" startup line' (($r1.Code -eq 0) -and $r1.Adopted -and ($line1 -ceq 'riviv: renderer=warp backend=d2d/warp')) ("exit=$($r1.Code) adopted=$($r1.Adopted) line=[$line1] stderr=[$($err1.Trim())]")
Check 'S1b dump: PNG exists, viewport-sized, output_gen=1' ($pngOk1 -and $sizeOk1 -and ($genLine1 -ceq 'riviv: dump-viewport output_gen=1')) ("png=$pngOk1 dims=$dims1 viewport=$($r1.Vs[0])x$($r1.Vs[1]) gen=[$genLine1]")
Check 'S1c second startup line present (d2d max_bitmap + dxgi budget)' (($maxLine1.StartsWith('riviv: d2d max_bitmap=')) -and $err1.Contains('(dxgi budget=')) "maxLine=[$maxLine1] stderr=[$($err1.Trim())]"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S1b: the switch overrides the ini in BOTH directions.
# ---------------------------------------------------------------------------
Reset-Ini ($BaseRect + "renderer=warp`r`n")
$p = Start-Riv ("`"$hash300`" -renderer auto") 's1b-a.err' $false
$main = Wait-Main $p
$adopted = Wait-Title $p 'hash300' 12000
$code = Close-Main $p $main
$err = Read-Err 's1b-a.err'
$lineA = Get-Line $err 'riviv: renderer='
Check 'S1b-a ini=warp + -renderer auto: exact "renderer=auto backend=d2d/hw" line' (($main -ne [IntPtr]::Zero) -and $adopted -and ($code -eq 0) -and ($lineA -ceq 'riviv: renderer=auto backend=d2d/hw')) ("exit=$code adopted=$adopted line=[$lineA] stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ($BaseRect + "renderer=d2d`r`n")
$p = Start-Riv ("`"$hash300`" -renderer warp") 's1b-b.err' $false
$main = Wait-Main $p
$adopted = Wait-Title $p 'hash300' 12000
$code = Close-Main $p $main
$err = Read-Err 's1b-b.err'
$lineB = Get-Line $err 'riviv: renderer='
Check 'S1b-b ini=d2d + -renderer warp: exact "renderer=warp backend=d2d/warp" line' (($main -ne [IntPtr]::Zero) -and $adopted -and ($code -eq 0) -and ($lineB -ceq 'riviv: renderer=warp backend=d2d/warp')) ("exit=$code adopted=$adopted line=[$lineB] stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2: hw vs warp 1:1 byte equality. Same fixture, same calibrated 900x600
# riviv_view client, 1:1 (NEAREST, pure copy of the 300x200 source) in both
# arms, dump at close -> the two PNGs must be whole-file identical.
# ---------------------------------------------------------------------------
$s2ini = "[riviv]`r`nx=60`r`ny=60`r`nwide=940`r`nhigh=720`r`nauto_zoom=0`r`nicm=0`r`nrenderer=auto`r`n"
$s2outA = Join-Path $Stage 's2a.png'
$s2outB = Join-Path $Stage 's2b.png'
$r2a = Run-Dump ("`"$hash300`" -dump-viewport `"$s2outA`"") 'hash300' $s2ini 's2a.png' 's2a.err' $one2one 900 600
$r2b = Run-Dump ("`"$hash300`" -renderer warp -dump-viewport `"$s2outB`"") 'hash300' $s2ini 's2b.png' 's2b.err' $one2one 900 600
$line2a = Get-Line $r2a.Err 'riviv: renderer='
$line2b = Get-Line $r2b.Err 'riviv: renderer='
Check 'S2a arm A (ini auto): clean run, exact "renderer=auto backend=d2d/hw" line, PNG' (($r2a.Code -eq 0) -and $r2a.Adopted -and ($line2a -ceq 'riviv: renderer=auto backend=d2d/hw') -and (Test-Path $r2a.Out)) ("exit=$($r2a.Code) adopted=$($r2a.Adopted) line=[$line2a] png=$(Test-Path $r2a.Out) stderr=[$($r2a.Err.Trim())]")
Check 'S2b arm B (-renderer warp): clean run, exact "renderer=warp backend=d2d/warp" line, PNG' (($r2b.Code -eq 0) -and $r2b.Adopted -and ($line2b -ceq 'riviv: renderer=warp backend=d2d/warp') -and (Test-Path $r2b.Out)) ("exit=$($r2b.Code) adopted=$($r2b.Adopted) line=[$line2b] png=$(Test-Path $r2b.Out) stderr=[$($r2b.Err.Trim())]")
$calOk2 = (($r2a.Vs[0] -eq 900) -and ($r2a.Vs[1] -eq 600) -and ($r2b.Vs[0] -eq 900) -and ($r2b.Vs[1] -eq 600))
Check 'S2c both runs calibrated to exactly 900x600' $calOk2 ("a=$($r2a.Vs[0])x$($r2a.Vs[1]) b=$($r2b.Vs[0])x$($r2b.Vs[1])")
$hashEq2 = $false
$hA2 = '(missing)'
$hB2 = '(missing)'
if ((Test-Path $r2a.Out) -and (Test-Path $r2b.Out)) {
    $hA2 = (Get-FileHash $r2a.Out -Algorithm SHA256).Hash
    $hB2 = (Get-FileHash $r2b.Out -Algorithm SHA256).Hash
    $hashEq2 = ($hA2 -ceq $hB2)
}
Check 'S2d hw vs warp 1:1 dumps whole-file identical (SHA256)' $hashEq2 "a=$hA2 b=$hB2"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the -renderer override is NEVER persisted. After a -renderer warp
# session the saved ini must still read renderer=auto, with no warp leak.
# ---------------------------------------------------------------------------
$s3out = Join-Path $Stage 's3-out.png'
$r3 = Run-Dump ("`"$hash300`" -renderer warp -dump-viewport `"$s3out`"") 'hash300' ($BaseRect + "renderer=auto`r`n") 's3-out.png' 's3.err' $null $null $null
$iniText3 = ''
if (Test-Path $Ini) { $iniText3 = [IO.File]::ReadAllText($Ini) }
$keepAuto3 = ($iniText3 -match '(?m)^renderer=auto\r?$')
$noLeak3 = (-not $iniText3.Contains('renderer=warp'))
Check 'S3 override not persisted: exit 0, ini still renderer=auto, no renderer=warp' (($r3.Code -eq 0) -and $keepAuto3 -and $noLeak3) ("exit=$($r3.Code) keepAuto=$keepAuto3 noLeak=$noLeak3 savedRendererLine=[$(Get-Line $iniText3 'renderer=')]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: single-instance handoff = breadcrumb + ignore. A runs plain (ini auto);
# B forwards "<hash224> -renderer warp" via WM_COPYDATA and exits 0; A prints
# the ignore breadcrumb and keeps its original startup line. A's raw handle
# is saved before B's launch overwrites $script:RawH and restored for the
# close (Close-Main reads the exit code from that handle).
# ---------------------------------------------------------------------------
$hash224 = Join-Path $Stage 'hash224.png'
[Px]::SaveRgba($hash224, [Px]::HashSource(224, 140), 224, 140)
Reset-Ini ($BaseRect + "renderer=auto`r`n")
$pa = Start-Riv "`"$hash300`"" 's4-a.err' $false
$mainA = Wait-Main $pa
$adoptA = Wait-Title $pa 'hash300' 12000
$hA = $script:RawH
$bExited = $false
$exitB = -99
$bcOk = $false
if (($mainA -ne [IntPtr]::Zero) -and $adoptA) {
    Start-Sleep -Milliseconds 300
    $pb = Start-Riv ("`"$hash224`" -renderer warp") 's4-b.err' $false
    $hB = $script:RawH
    $bExited = $pb.WaitForExit(10000)
    $codeB = 0
    if ($bExited -and ($hB -ne [IntPtr]::Zero)) {
        [void][S80]::GetExitCodeProcess($hB, [ref]$codeB)
    }
    $exitB = $codeB
    $bcOk = Wait-Until { (Read-Err 's4-a.err').Contains('riviv: -renderer warp ignored - device already built (d2d/hw)') } 10000
}
$script:RawH = $hA
$codeA = Close-Main $pa $mainA
$errA = Read-Err 's4-a.err'
$origLine4 = Get-Line $errA 'riviv: renderer='
Check 'S4a A adopted, forwarder B exits 0 quickly' ($adoptA -and $bExited -and ($exitB -eq 0)) "adoptedA=$adoptA bExited=$bExited exitB=$exitB"
Check 'S4b A gained the exact ignore breadcrumb' $bcOk "stderr=[$($errA.Trim())]"
Check 'S4c A startup line unchanged (auto/hw), no warp renderer line' (($origLine4 -ceq 'riviv: renderer=auto backend=d2d/hw') -and (-not $errA.Contains('riviv: renderer=warp backend='))) "line=[$origLine4] stderr=[$($errA.Trim())]"
Check 'S4d A closed normally (exit 0)' ($codeA -eq 0) "exit=$codeA"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S5: dangling switch / unrecognized value = unarmed intent. The ini key
# governs, no ignore breadcrumb appears, and WM_CLOSE is not blocked by any
# usage modal (a blocked close turns the exit code into -1).
# ---------------------------------------------------------------------------
Reset-Ini ($BaseRect + "renderer=auto`r`n")
$p = Start-Riv ("`"$hash300`" -renderer") 's5a.err' $false
$main = Wait-Main $p
$adopted = Wait-Title $p 'hash300' 12000
$code = Close-Main $p $main
$err = Read-Err 's5a.err'
$line5a = Get-Line $err 'riviv: renderer='
Check 'S5a dangling -renderer: window + adoption, ini governs (exact auto/hw), no breadcrumb, exit 0' (($main -ne [IntPtr]::Zero) -and $adopted -and ($code -eq 0) -and ($line5a -ceq 'riviv: renderer=auto backend=d2d/hw') -and (-not $err.Contains('ignored - device already built'))) ("exit=$code adopted=$adopted line=[$line5a] stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ($BaseRect + "renderer=auto`r`n")
$p = Start-Riv ("`"$hash300`" -renderer oops") 's5b.err' $false
$main = Wait-Main $p
$adopted = Wait-Title $p 'hash300' 12000
$code = Close-Main $p $main
$err = Read-Err 's5b.err'
$line5b = Get-Line $err 'riviv: renderer='
Check 'S5b -renderer oops: window + adoption, ini governs (exact auto/hw), no breadcrumb, exit 0' (($main -ne [IntPtr]::Zero) -and $adopted -and ($code -eq 0) -and ($line5b -ceq 'riviv: renderer=auto backend=d2d/hw') -and (-not $err.Contains('ignored - device already built'))) ("exit=$code adopted=$adopted line=[$line5b] stderr=[$($err.Trim())]")
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
else { Write-Output ('FAILURES: evidence kept in ' + $Stage) }
Write-Output ('SMOKE126 TOTALS: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip)
if ($script:fail -gt 0) { Write-Output 'SMOKE126 RESULT: FAIL'; exit 1 }
Write-Output 'SMOKE126 RESULT: PASS'
exit 0
