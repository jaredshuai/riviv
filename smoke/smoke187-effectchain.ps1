# smoke187 - #187 the effect-chain smoke matrix (ADR 0006 ticket 3/3):
# the machine-facing acceptance for the user display-effect chain laid in
# #183 (graph/decision tables) and #185 (the sharpen surface). Gate
# discipline (#182 verdict): effect OFF reuses the existing numeric
# regime; effect ON asserts STRUCTURE only (dump non-empty, size/format
# unchanged, same-input byte-reproducible, and - as the one boolean the
# chain actually ran - the ON dump differs from the OFF dump on
# edge-heavy fixtures; no automated pixel gate, looks stay human QA).
# ASCII-only source (PS 5.1 ANSI trap, #79 lesson). Harness cribbed from
# smoke173-faultinjection.ps1: staged exes under %TEMP%\riviv-187-smoke
# (BOTH the master build and the pre-#183 baseline build run ONLY from
# the stage), the staged ini is rewritten before EVERY launch and wiped
# after every close (WM_CLOSE saves config), poll-based stderr waits,
# Add-Type C# PNG fixture writer + dump stddev analyzer, SetProcessDPI-
# Aware, GetExitCodeProcess on the raw handle captured WHILE ALIVE,
# path-targeted kills only, FATAL-exit when a foreign riviv is alive.
#
# Exes:  $Exe     the master build (chain code, consumes RIVIV_FAULT's
#                 chain kind, reads the sharpen ini key)
#        $BaseExe the PRE-#183 baseline (3a107d8, PR #181's merge): no
#                 chain code at all, ignores the sharpen key - the D8
#                 zero-perturbation control compares against it
#                 BYTE-FOR-BYTE (Cargo.lock is unchanged between the
#                 two, so the PNG/raw writers are the same bytes).
#
# Fixtures (deterministic, edge-heavy so an active sharpen MUST move
# pixels - the ON-vs-OFF boolean needs the fixtures to make "no visible
# change" impossible): 8px checkerboard folded onto both axes' gradients,
# b=(x+y)%256 (smoke156's recipe for the texture term).
#   probe.png  96x96    no iCCP   (narrow arm, legacy face, PNG dump)
#   wide.png   320x240  iCCP=p3   (AC arm on this machine, RAW u16 dump)
#   huge.png   2560x1920 no iCCP  (the D3 look-comparison artifact -
#             fit-shrunk to the 1920x1200 window, height-limited so the
#             axes differ; dumped for HUMAN edge/ringing inspection only)
#
# Scenarios (one launch each unless noted; ini rewritten every launch):
#   S0  stage: foreign-riviv FATAL, leftover-ini FATAL, BOTH exes staged,
#       cargo emitter writes p3.icc (smoke156's test), C# writes the
#       three fixtures
#   S1  A4 negative-control probe (instrument test, no GUI): cargo test
#       -- --ignored smoke187_sharpen_property_domain_probe - the live
#       D2D runtime's own behavior (ADR 0006 D6: docs enum page gave the
#       domain, the positive probe is retired): defaults read back 0.0;
#       out-of-domain writes are NEVER refused - SHARPNESS clamps into
#       0.0..=10.0 on both sides, THRESHOLD stores 1.5 verbatim (no
#       ceiling enforcement) and clamps the negative side - which is why
#       the config int key is the ONLY gate
#   S2  OFF zero-perturbation, narrow: master vs baseline, sharpen=0,
#       renderer=d2d: dump PNGs byte-identical (SHA256), same dims
#   S2w OFF zero-perturbation, wide AC: master vs baseline raw u16 dumps
#       byte-identical, equal nonzero byte counts
#   S3  ON structural contract, narrow: master, sharpen=5: dump exists,
#       dims == S2's, rerun byte-reproducible (two launches, same hash),
#       ON != OFF (the boolean that the chain actually ran)
#   S3w ON structural contract, wide AC: raw dump equal byte count to
#       S2w's, rerun byte-reproducible, ON != OFF
#   S4  WARP exclusion (D5, negative only): renderer=warp, probe.png -
#       chain ON (sharpen=5) vs chain OFF (sharpen=0) dumps byte-identical
#       (the chain never runs on WARP; content draws through), plus the
#       backend=warp identity line in both sessions
#   S5  D9 injection pin: master, sharpen=5, RIVIV_FAULT=chain=1:
#       exactly ONE "user effect chain dropped" line (more = the repaint
#       loop the drop exists to prevent), one display-effect-failure
#       drain line, ZERO degrade latches, window alive, dump == the S2
#       OFF dump (the session collapsed back to the chain-less shape);
#       then the REVIVAL pin - toggle (WM_COMMAND 123) re-arms the chain
#       (the toggle reads the LIVE gpu chain, not the config key), a
#       second toggle clears it, and neither produces a new dropped line
#   S6  the D3 look-comparison artifact: huge.png, sharpen=5, dump kept
#       for human QA (edge/ringing at the anisotropic shrink); machine
#       asserts only structure: exists, window-sized, non-uniform

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe',
      [string]$Repo = 'D:\codespace\riviv',
      [string]$BaseExe = 'D:\codespace\riviv-base-183\target\release\riviv.exe')

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Runtime.InteropServices;

public class S187 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    // ---- PNG fixture writing (smoke98/smoke156/smoke173's verified recipe) ----
    static void PutBe32(byte[] b, int o, uint v) {
        b[o] = (byte)(v >> 24); b[o+1] = (byte)(v >> 16); b[o+2] = (byte)(v >> 8); b[o+3] = (byte)v;
    }
    static uint Crc32(byte[] data) {
        uint c = 0xFFFFFFFF;
        foreach (byte x in data) { c ^= x; for (int k = 0; k < 8; k++) c = ((c & 1) != 0) ? (0xEDB88320 ^ (c >> 1)) : (c >> 1); }
        return c ^ 0xFFFFFFFF;
    }
    static void Chunk(MemoryStream ms, string type, byte[] data) {
        byte[] len = new byte[4]; PutBe32(len, 0, (uint)data.Length);
        ms.Write(len, 0, 4);
        byte[] tb = Encoding.ASCII.GetBytes(type);
        ms.Write(tb, 0, 4);
        ms.Write(data, 0, data.Length);
        byte[] crcIn = new byte[data.Length + 4];
        tb.CopyTo(crcIn, 0); data.CopyTo(crcIn, 4);
        byte[] crc = new byte[4]; PutBe32(crc, 0, Crc32(crcIn));
        ms.Write(crc, 0, 4);
    }
    static byte[] Zlib(byte[] raw) {
        MemoryStream comp = new MemoryStream();
        comp.WriteByte(0x78); comp.WriteByte(0x01);
        DeflateStream ds = new DeflateStream(comp, CompressionLevel.Fastest, true);
        ds.Write(raw, 0, raw.Length);
        ds.Close();
        uint a1 = 1, a2 = 0;
        for (int i = 0; i < raw.Length; i++) { a1 = (a1 + raw[i]) % 65521; a2 = (a2 + a1) % 65521; }
        uint adler = (a2 << 16) | a1;
        comp.WriteByte((byte)(adler >> 24)); comp.WriteByte((byte)(adler >> 16));
        comp.WriteByte((byte)(adler >> 8)); comp.WriteByte((byte)adler);
        return comp.ToArray();
    }
    static byte[] EdgeScanlines(int w, int h) {
        byte[] raw = new byte[h * (1 + w * 4)];
        for (int y = 0; y < h; y++) {
            int row = y * (1 + w * 4);
            raw[row] = 0; // filter: none
            for (int x = 0; x < w; x++) {
                int i = row + 1 + x * 4;
                int gx = x * 255 / (w - 1);
                int gy = y * 255 / (h - 1);
                int cb = ((x / 8) + (y / 8)) % 2;
                raw[i]   = (byte)(cb == 0 ? gx : 255 - gx);
                raw[i+1] = (byte)(cb == 0 ? gy : 255 - gy);
                raw[i+2] = (byte)((x + y) % 256);
                raw[i+3] = 255;
            }
        }
        return raw;
    }
    static void WritePng(string path, int w, int h, byte[] scanlines, byte[] icc) {
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 6; // RGBA8
        Chunk(ms, "IHDR", ihdr);
        if (icc != null) {
            byte[] kw = Encoding.ASCII.GetBytes("ICC profile");
            byte[] head = new byte[kw.Length + 2];
            kw.CopyTo(head, 0); head[kw.Length] = 0; head[kw.Length+1] = 0;
            byte[] z = Zlib(icc);
            byte[] iccp = new byte[head.Length + z.Length];
            head.CopyTo(iccp, 0); z.CopyTo(iccp, head.Length);
            Chunk(ms, "iCCP", iccp);
        }
        Chunk(ms, "IDAT", Zlib(scanlines));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }

    /// The edge-heavy deterministic fixture (#187): 8px checkerboard
    /// folded onto both axis gradients, b=(x+y)%256 - an ACTIVE sharpen
    /// must move pixels on it (the ON != OFF boolean's premise).
    public static void WriteEdgePng(string path, int w, int h, byte[] icc) {
        WritePng(path, w, h, EdgeScanlines(w, h), icc);
    }
}
'@

Add-Type -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public class S187Draw {
    /// Grayscale mean/stddev over the decoded dump PNG (LockBits,
    /// 32bpp ARGB copy). A uniform (blank) dump has stddev ~0; a drawn
    /// image lands well above 5.
    public static double StdDev(string path) {
        using (Bitmap bmp = new Bitmap(path)) {
            Rectangle r = new Rectangle(0, 0, bmp.Width, bmp.Height);
            BitmapData d = bmp.LockBits(r, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            int stride = d.Stride;
            int w = bmp.Width, h = bmp.Height;
            byte[] buf = new byte[stride * h];
            Marshal.Copy(d.Scan0, buf, 0, buf.Length);
            bmp.UnlockBits(d);
            double sum = 0, sum2 = 0;
            long n = (long)w * (long)h;
            for (int y = 0; y < h; y++) {
                int row = y * stride;
                for (int x = 0; x < w; x++) {
                    int i = row + x * 4;
                    int v = (buf[i] + buf[i+1] + buf[i+2]) / 3;
                    sum += v; sum2 += (double)v * (double)v;
                }
            }
            double mean = sum / n;
            return Math.Sqrt(sum2 / n - mean * mean);
        }
    }
}
'@ -ReferencedAssemblies @('System.Drawing')

# ---------------------------------------------------------------------------
# Harness (smoke173's shape)
# ---------------------------------------------------------------------------
$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ('PASS ' + $name + ' -- ' + $detail) }
    else { $script:fail++; Write-Output ('FAIL ' + $name + ' -- ' + $detail) }
}
function Skip($name, $detail) {
    $script:skip++
    Write-Output ('SKIP ' + $name + ' -- ' + $detail)
}
function Wait-Until($sb, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $sb) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (& $sb)
}
function Kill-StagedRiviv {
    # Path-targeted (#122 rule): only the staged copies are ever killed.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { ($_.Path -eq $RunExe) -or ($_.Path -eq $BaseRun) })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}
function Reset-Ini($text) {
    if (Test-Path $Ini) { Remove-Item $Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($Ini, $text) }
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
function Start-Riv($exe, $argStr, $errName) {
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    $script:RawP = Start-Process -FilePath $exe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
    $script:RawH = $script:RawP.Handle   # capture WHILE ALIVE (smoke80 lesson)
    return $script:RawP
}
function Wait-Main($p) {
    for ($i = 0; $i -lt 150; $i++) {
        Start-Sleep -Milliseconds 100
        $p.Refresh()
        $h = $p.MainWindowHandle
        if (($h -ne $null) -and ($h -ne [IntPtr]::Zero)) { return $h }
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
function Test-Alive($p) {
    $p.Refresh()
    return ((-not $p.HasExited) -and ($p.MainWindowHandle -ne $null) -and ($p.MainWindowHandle -ne [IntPtr]::Zero))
}
function Wait-ExitP($p, $ms) {
    # Returns the exit code (via the raw handle captured WHILE ALIVE -
    # the smoke80 lesson), or $null when still alive after $ms.
    if ($p.WaitForExit($ms)) {
        $code = -1
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S187]::GetExitCodeProcess($script:RawH, [ref]$code)
        }
        return $code
    }
    return $null
}
function Wait-Err($errName, [scriptblock]$pred, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    $err = Read-Err $errName
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((& $pred $err)) { return $err }
        Start-Sleep -Milliseconds 150
        $err = Read-Err $errName
    }
    return $err
}
function Get-Lines($err, $pattern) {
    $hits = @()
    if ($null -eq $err) { return ,$hits }
    foreach ($ln in ($err -split "`n")) {
        $t = $ln.TrimEnd("`r")
        if ($t -match $pattern) { $hits += $t }
    }
    return ,$hits
}
function Png-Dims($path) {
    try {
        $bmp = [Drawing.Bitmap]::FromFile($path)
        $d = @($bmp.Width, $bmp.Height)
        $bmp.Dispose()
        return $d
    } catch { return @(0, 0) }
}
function File-Hash($path) {
    if (-not (Test-Path $path)) { return '(missing)' }
    return (Get-FileHash -Algorithm SHA256 -Path $path).Hash
}
function Short-Hash($h) {
    # Safe abbreviation: '(missing)' is shorter than 16 chars.
    if ($h.Length -ge 16) { return $h.Substring(0,16) + '..' }
    return $h
}
function Run-CargoTest($filter, $capName) {
    $cap = Join-Path $Stage $capName
    if (Test-Path $cap) { Remove-Item $cap -Force }
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    Push-Location $Repo
    try {
        & cargo test --release -- --ignored $filter *> $cap
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
        $ErrorActionPreference = $prevEap
    }
    $out = ''
    if (Test-Path $cap) { $out = [IO.File]::ReadAllText($cap) }
    return @{ Code = $code; Out = $out }
}
function Clear-FaultEnv {
    Remove-Item Env:\RIVIV_FAULT -ErrorAction SilentlyContinue
}
# The scenario ini: window geometry pinned (dump dims depend on it), icm=1
# (the machine's AC arm stays reachable for the wide fixture), renderer
# per scenario, sharpen per scenario. The BASELINE exe ignores the
# sharpen key entirely (no chain code) - both exes take the same text.
function New-Ini([string]$renderer, [int]$sharpen) {
    return "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nicm=1`r`nrenderer=$renderer`r`nshow_menu=0`r`nsharpen=$sharpen`r`n"
}
# One launch -> adopt -> settle -> close (dump written at close). $exe is
# the FULL path of whichever staged build runs. Returns the exit code and
# leaves the stderr name for Read-Err.
function Run-Once($exe, $iniText, $argStr, $errName, $leaf) {
    Reset-Ini $iniText
    Clear-FaultEnv
    $p = Start-Riv $exe $argStr $errName
    $main = Wait-Main $p
    [void](Wait-Title $p $leaf 15000)
    Start-Sleep -Milliseconds 2000   # paint settle
    $alive = Test-Alive $p
    if (-not $p.HasExited -and $main -ne [IntPtr]::Zero) {
        [void][S187]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    $code = Wait-ExitP $p 30000
    if ($code -eq $null) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        $code = -1
    }
    Clear-FaultEnv
    Reset-Ini ''
    Kill-StagedRiviv
    return @{ Code = $code; Alive = $alive }
}

[void][S187]::SetProcessDPIAware()

# ---------------------------------------------------------------------------
# S0: the stage.
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-187-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'
$BaseRun = Join-Path $Stage 'riviv-base.exe'
$preExisting = @(Get-Process riviv -ErrorAction SilentlyContinue)
$foreign = @($preExisting | Where-Object { ($_.Path -ne $RunExe) -and ($_.Path -ne $BaseRun) })
if ($foreign.Count -gt 0) {
    $fxPids = ($foreign | ForEach-Object { $_.Id }) -join ','
    Write-Output ('FATAL foreign riviv running - close it and rerun (pids: ' + $fxPids + ')')
    exit 3
}
Kill-StagedRiviv
if (Test-Path $Ini) {
    Write-Output ('FATAL leftover ini in the stage (' + $Ini + ') - delete it and rerun')
    exit 3
}
if (-not (Test-Path $Stage)) { New-Item -ItemType Directory -Path $Stage | Out-Null }
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); exit 2 }
if (-not (Test-Path $BaseExe)) { Write-Output ('MISSING BASELINE EXE: ' + $BaseExe + ' (git worktree at 3a107d8, cargo build --release)'); exit 2 }
Copy-Item $Exe $RunExe -Force
Copy-Item $BaseExe $BaseRun -Force
Check 'S0a stage ready: both exes staged, ini absent, no foreign riviv' ((Test-Path $RunExe) -and (Test-Path $BaseRun) -and (-not (Test-Path $Ini))) ('master=' + (Test-Path $RunExe) + ' base=' + (Test-Path $BaseRun) + ' iniExists=' + (Test-Path $Ini))

# The p3.icc destination profile from riviv's own cargo emitter (the env
# var must reach the cargo child; this runs the CURRENT checkout's tree,
# which is the #187 branch - the emitter test is unchanged since #156).
$P3Icc = Join-Path $Stage 'p3.icc'
Clear-FaultEnv
$env:RIVIV_S156_ICC = $P3Icc
$c0 = Run-CargoTest 'smoke156_write_p3_profile' 'cargo-s0.txt'
Remove-Item Env:\RIVIV_S156_ICC -ErrorAction SilentlyContinue
$iccBytes = 0
if (Test-Path $P3Icc) { $iccBytes = (Get-Item $P3Icc).Length }
$iccOk = ($c0.Code -eq 0) -and ($c0.Out -match 'test result: ok') -and ($c0.Out -match '1 passed') -and ($c0.Out -cnotmatch 'FAILED') -and ($iccBytes -eq 6680)
Check 'S0b cargo emitter wrote p3.icc (6680 bytes, 1 passed)' $iccOk ("exit=$($c0.Code) bytes=$iccBytes")
if (-not $iccOk) {
    Write-Output ('FAILURES: S0 emitter failed - evidence kept in ' + $Stage)
    Write-Output ('SUMMARY smoke187 pass=' + $script:pass + ' fail=' + ($script:fail + 1))
    Write-Output 'SMOKE187 RESULT: FAIL'
    exit 1
}

$Probe = Join-Path $Stage 'probe.png'
[S187]::WriteEdgePng($Probe, 96, 96, $null)
$pd = Png-Dims $Probe
Check 'S0c probe.png written (96x96, edge-heavy, no iCCP)' (($pd[0] -eq 96) -and ($pd[1] -eq 96)) ('dims=' + $pd[0] + 'x' + $pd[1])
$Wide = Join-Path $Stage 'wide.png'
[S187]::WriteEdgePng($Wide, 320, 240, [IO.File]::ReadAllBytes($P3Icc))
$wd = Png-Dims $Wide
Check 'S0d wide.png written (320x240, iCCP=p3, edge-heavy)' (($wd[0] -eq 320) -and ($wd[1] -eq 240)) ('dims=' + $wd[0] + 'x' + $wd[1])
$Huge = Join-Path $Stage 'huge.png'
[S187]::WriteEdgePng($Huge, 2560, 1920, $null)
$hd = Png-Dims $Huge
Check 'S0e huge.png written (2560x1920, the D3 look-comparison artifact)' (($hd[0] -eq 2560) -and ($hd[1] -eq 1920)) ('dims=' + $hd[0] + 'x' + $hd[1])

# ---------------------------------------------------------------------------
# S1: the A4 negative-control probe - the ignored instrument test runs on
# the CURRENT TREE's test binary (needs D2D, which the cargo test host
# has; WARP carries it, no GUI window involved).
# ---------------------------------------------------------------------------
Clear-FaultEnv
$c1 = Run-CargoTest 'smoke187_sharpen_property_domain_probe' 'cargo-s1.txt'
$a4Ok = ($c1.Code -eq 0) -and ($c1.Out -match 'test result: ok') -and ($c1.Out -match '1 passed') -and ($c1.Out -cnotmatch 'FAILED')
Check 'S1-1 A4 probe: defaults 0.0; out-of-domain never refused (SHARPNESS clamps, THRESHOLD verbatim over-domain) (1 passed)' $a4Ok ("exit=$($c1.Code) out=[$($c1.Out.Trim())]")

# ---------------------------------------------------------------------------
# S2: OFF zero-perturbation, narrow arm. Master vs the pre-#183 baseline:
# same ini (sharpen=0), same fixture, same pinned geometry - the D8
# contract says the dump is byte-identical (the chain code must be
# invisible with the chain empty).
# ---------------------------------------------------------------------------
$DumpS2M = Join-Path $Stage 'dump-s2-master.png'
$DumpS2B = Join-Path $Stage 'dump-s2-base.png'
if (Test-Path $DumpS2M) { Remove-Item $DumpS2M -Force }
if (Test-Path $DumpS2B) { Remove-Item $DumpS2B -Force }
$r2m = Run-Once $RunExe (New-Ini 'd2d' 0) ('"' + $Probe + '" -dump-viewport "' + $DumpS2M + '"') 's2m.err' 'probe'
$r2b = Run-Once $BaseRun (New-Ini 'd2d' 0) ('"' + $Probe + '" -dump-viewport "' + $DumpS2B + '"') 's2b.err' 'probe'
$err2m = Read-Err 's2m.err'
$err2b = Read-Err 's2b.err'
$d2m = Png-Dims $DumpS2M
$d2b = Png-Dims $DumpS2B
$h2m = File-Hash $DumpS2M
$h2b = File-Hash $DumpS2B
Check 'S2-1 master OFF dump exists, window-sized' (($d2m[0] -gt 0) -and ($d2m[1] -gt 0)) ('dims=' + $d2m[0] + 'x' + $d2m[1])
Check 'S2-2 baseline OFF dump exists, SAME dims as master' (($d2b[0] -eq $d2m[0]) -and ($d2b[1] -eq $d2m[1])) ('base dims=' + $d2b[0] + 'x' + $d2b[1] + ' master dims=' + $d2m[0] + 'x' + $d2m[1])
Check 'S2-3 OFF dumps BYTE-IDENTICAL (D8 zero-perturbation, narrow arm)' (($h2m -ne '(missing)') -and ($h2m -eq $h2b)) ('master=' + (Short-Hash $h2m) + ' base=' + (Short-Hash $h2b))
Check 'S2-4 both sessions show a display-stage line (baseline sanity)' (((Get-Lines $err2m 'display-stage=').Count -ge 1) -and ((Get-Lines $err2b 'display-stage=').Count -ge 1)) ('master=' + (Get-Lines $err2m 'display-stage=').Count + ' base=' + (Get-Lines $err2b 'display-stage=').Count)
Check 'S2-5 master exit 0 / baseline exit 0' (($r2m.Code -eq 0) -and ($r2b.Code -eq 0)) ('master=' + $r2m.Code + ' base=' + $r2b.Code)

# ---------------------------------------------------------------------------
# S2w: OFF zero-perturbation, WIDE content on the AC arm (this machine's
# icm=1 baseline): the dump is the RAW scRGB u16 halves file (no PNG
# container) - byte-identical between master and baseline.
# ---------------------------------------------------------------------------
$DumpS2wM = Join-Path $Stage 'dump-s2w-master.raw'
$DumpS2wB = Join-Path $Stage 'dump-s2w-base.raw'
if (Test-Path $DumpS2wM) { Remove-Item $DumpS2wM -Force }
if (Test-Path $DumpS2wB) { Remove-Item $DumpS2wB -Force }
$r2wm = Run-Once $RunExe (New-Ini 'd2d' 0) ('"' + $Wide + '" -dump-viewport "' + $DumpS2wM + '"') 's2wm.err' 'wide'
$r2wb = Run-Once $BaseRun (New-Ini 'd2d' 0) ('"' + $Wide + '" -dump-viewport "' + $DumpS2wB + '"') 's2wb.err' 'wide'
$err2wm = Read-Err 's2wm.err'
$n2wm = 0; if (Test-Path $DumpS2wM) { $n2wm = (Get-Item $DumpS2wM).Length }
$n2wb = 0; if (Test-Path $DumpS2wB) { $n2wb = (Get-Item $DumpS2wB).Length }
$h2wm = File-Hash $DumpS2wM
$h2wb = File-Hash $DumpS2wB
$ac2wm = (Get-Lines $err2wm 'output-surface=ac-scrgb').Count
Check 'S2w-1 master ran the AC face (this machine baseline)' ($ac2wm -ge 1) ('ac-scrgb lines=' + $ac2wm)
Check 'S2w-2 both raw dumps nonzero and equal-length' (($n2wm -gt 0) -and ($n2wm -eq $n2wb)) ('master=' + $n2wm + 'B base=' + $n2wb + 'B')
Check 'S2w-3 OFF raw dumps BYTE-IDENTICAL (D8 zero-perturbation, AC arm)' (($h2wm -ne '(missing)') -and ($h2wm -eq $h2wb)) ('master=' + (Short-Hash $h2wm) + ' base=' + (Short-Hash $h2wb))
Check 'S2w-4 master exit 0 / baseline exit 0' (($r2wm.Code -eq 0) -and ($r2wb.Code -eq 0)) ('master=' + $r2wm.Code + ' base=' + $r2wb.Code)

# ---------------------------------------------------------------------------
# S3: ON structural contract, narrow arm. Master, sharpen=5: dump exists,
# same dims as S2's, a RERUN is byte-identical (same input -> same
# output), and ON differs from OFF (the edge-heavy fixture makes an
# active sharpen impossible to miss - the boolean that the chain ran).
# ---------------------------------------------------------------------------
$DumpS3a = Join-Path $Stage 'dump-s3-run1.png'
$DumpS3b = Join-Path $Stage 'dump-s3-run2.png'
if (Test-Path $DumpS3a) { Remove-Item $DumpS3a -Force }
if (Test-Path $DumpS3b) { Remove-Item $DumpS3b -Force }
$r3a = Run-Once $RunExe (New-Ini 'd2d' 5) ('"' + $Probe + '" -dump-viewport "' + $DumpS3a + '"') 's3a.err' 'probe'
$r3b = Run-Once $RunExe (New-Ini 'd2d' 5) ('"' + $Probe + '" -dump-viewport "' + $DumpS3b + '"') 's3b.err' 'probe'
$d3a = Png-Dims $DumpS3a
$h3a = File-Hash $DumpS3a
$h3b = File-Hash $DumpS3b
$sd3 = -1.0
if (Test-Path $DumpS3a) { $sd3 = [S187Draw]::StdDev($DumpS3a) }
Check 'S3-1 ON dump exists, dims == OFF dims (structure unchanged)' (($d3a[0] -eq $d2m[0]) -and ($d3a[1] -eq $d2m[1])) ('ON dims=' + $d3a[0] + 'x' + $d3a[1] + ' OFF dims=' + $d2m[0] + 'x' + $d2m[1])
Check 'S3-2 ON rerun byte-reproducible (same input, two launches)' (($h3a -ne '(missing)') -and ($h3a -eq $h3b)) ('run1=' + (Short-Hash $h3a) + ' run2=' + (Short-Hash $h3b))
Check 'S3-3 ON dump DIFFERS from OFF dump (the chain actually ran)' ($h3a -ne $h2m) ('ON=' + (Short-Hash $h3a) + ' OFF=' + (Short-Hash $h2m))
Check 'S3-4 ON dump non-uniform (content drawn)' ($sd3 -gt 5.0) ('stddev=' + [Math]::Round($sd3, 2))
Check 'S3-5 exit 0 both runs' (($r3a.Code -eq 0) -and ($r3b.Code -eq 0)) ('run1=' + $r3a.Code + ' run2=' + $r3b.Code)

# ---------------------------------------------------------------------------
# S3w: ON structural contract, wide AC arm (raw scRGB dump).
# ---------------------------------------------------------------------------
$DumpS3wa = Join-Path $Stage 'dump-s3w-run1.raw'
$DumpS3wb = Join-Path $Stage 'dump-s3w-run2.raw'
if (Test-Path $DumpS3wa) { Remove-Item $DumpS3wa -Force }
if (Test-Path $DumpS3wb) { Remove-Item $DumpS3wb -Force }
$r3wa = Run-Once $RunExe (New-Ini 'd2d' 5) ('"' + $Wide + '" -dump-viewport "' + $DumpS3wa + '"') 's3wa.err' 'wide'
$r3wb = Run-Once $RunExe (New-Ini 'd2d' 5) ('"' + $Wide + '" -dump-viewport "' + $DumpS3wb + '"') 's3wb.err' 'wide'
$err3wa = Read-Err 's3wa.err'
$n3wa = 0; if (Test-Path $DumpS3wa) { $n3wa = (Get-Item $DumpS3wa).Length }
$n3wb = 0; if (Test-Path $DumpS3wb) { $n3wb = (Get-Item $DumpS3wb).Length }
$h3wa = File-Hash $DumpS3wa
$h3wb = File-Hash $DumpS3wb
$ac3wa = (Get-Lines $err3wa 'output-surface=ac-scrgb').Count
Check 'S3w-1 ON ran the AC face (the chain rides the CM arm, F16P3)' ($ac3wa -ge 1) ('ac-scrgb lines=' + $ac3wa)
Check 'S3w-2 ON raw dump equal length to OFF raw dump' (($n3wa -gt 0) -and ($n3wa -eq $n2wm)) ('ON=' + $n3wa + 'B OFF=' + $n2wm + 'B')
Check 'S3w-3 ON rerun byte-reproducible' (($h3wa -ne '(missing)') -and ($h3wa -eq $h3wb)) ('run1=' + (Short-Hash $h3wa) + ' run2=' + (Short-Hash $h3wb))
Check 'S3w-4 ON raw dump DIFFERS from OFF raw dump' ($h3wa -ne $h2wm) ('ON=' + (Short-Hash $h3wa) + ' OFF=' + (Short-Hash $h2wm))
Check 'S3w-5 exit 0 both runs' (($r3wa.Code -eq 0) -and ($r3wb.Code -eq 0)) ('run1=' + $r3wa.Code + ' run2=' + $r3wb.Code)

# ---------------------------------------------------------------------------
# S4: WARP exclusion (D5 - negative only). renderer=warp, probe.png: the
# chain ON (sharpen=5) and OFF (sharpen=0) sessions dump byte-identical
# PNGs: on WARP the chain never runs, the content draws through (the
# pass_shape row pinned by #183's unit tests, here driven live).
# ---------------------------------------------------------------------------
$DumpS4a = Join-Path $Stage 'dump-s4-chainon.png'
$DumpS4b = Join-Path $Stage 'dump-s4-chainoff.png'
if (Test-Path $DumpS4a) { Remove-Item $DumpS4a -Force }
if (Test-Path $DumpS4b) { Remove-Item $DumpS4b -Force }
$r4a = Run-Once $RunExe (New-Ini 'warp' 5) ('"' + $Probe + '" -dump-viewport "' + $DumpS4a + '"') 's4a.err' 'probe'
$r4b = Run-Once $RunExe (New-Ini 'warp' 0) ('"' + $Probe + '" -dump-viewport "' + $DumpS4b + '"') 's4b.err' 'probe'
$err4a = Read-Err 's4a.err'
$err4b = Read-Err 's4b.err'
$h4a = File-Hash $DumpS4a
$h4b = File-Hash $DumpS4b
Check 'S4-1 both WARP sessions show backend=warp' (((Get-Lines $err4a 'backend=warp').Count -ge 1) -and ((Get-Lines $err4b 'backend=warp').Count -ge 1)) ('on=' + (Get-Lines $err4a 'backend=warp').Count + ' off=' + (Get-Lines $err4b 'backend=warp').Count)
Check 'S4-2 WARP chain-ON dump == chain-OFF dump (D5: the chain is dropped, content identical)' (($h4a -ne '(missing)') -and ($h4a -eq $h4b)) ('on=' + (Short-Hash $h4a) + ' off=' + (Short-Hash $h4b))
Check 'S4-3 WARP dump non-uniform (content still draws)' ((Test-Path $DumpS4a) -and ([S187Draw]::StdDev($DumpS4a) -gt 5.0)) ('stddev=' + [Math]::Round([S187Draw]::StdDev($DumpS4a), 2))
Check 'S4-4 exit 0 both runs' (($r4a.Code -eq 0) -and ($r4b.Code -eq 0)) ('on=' + $r4a.Code + ' off=' + $r4b.Code)

# ---------------------------------------------------------------------------
# S5: the D9 injection pin. RIVIV_FAULT=chain=1 + sharpen=5: the first
# build refuses AFTER CreateEffect succeeded - the session drops the
# chain (ONE dropped line: more would be the repaint loop the drop
# exists to prevent), the drain counts ONE display-effect failure (not
# enough to latch anything), the content draws through, and the close
# dump equals the S2 OFF dump (the session collapsed back onto the
# chain-less shape). Then the REVIVAL pin: WM_COMMAND 123 (ViewSharpen)
# re-arms the chain from the LIVE gpu state (not the config key), a
# second toggle clears it - neither produces a new dropped line.
# ---------------------------------------------------------------------------
$DumpS5 = Join-Path $Stage 'dump-s5.png'
if (Test-Path $DumpS5) { Remove-Item $DumpS5 -Force }
Reset-Ini (New-Ini 'd2d' 5)
Clear-FaultEnv
$env:RIVIV_FAULT = 'chain=1'
$p5 = Start-Riv $RunExe ('"' + $Probe + '" -dump-viewport "' + $DumpS5 + '"') 's5.err'
$main5 = Wait-Main $p5
[void](Wait-Title $p5 'probe' 15000)
[void](Wait-Err 's5.err' { param($e) (Get-Lines $e 'user effect chain dropped').Count -ge 1 } 20000)
Start-Sleep -Milliseconds 1500   # the collapsed repaint settles
# revival toggle #1: the chain re-arms (injection spent, build succeeds)
if ($main5 -ne [IntPtr]::Zero) {
    [void][S187]::PostMessage($main5, 0x0111, [IntPtr]123, [IntPtr]::Zero)
}
Start-Sleep -Milliseconds 2000   # the re-armed chain repaints
# revival toggle #2: back off
if ($main5 -ne [IntPtr]::Zero) {
    [void][S187]::PostMessage($main5, 0x0111, [IntPtr]123, [IntPtr]::Zero)
}
Start-Sleep -Milliseconds 1500
$alive5 = Test-Alive $p5
if (-not $p5.HasExited -and $main5 -ne [IntPtr]::Zero) {
    [void][S187]::PostMessage($main5, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
}
$code5 = Wait-ExitP $p5 30000
if ($code5 -eq $null) { Stop-Process -Id $p5.Id -Force -ErrorAction SilentlyContinue; $code5 = -1 }
Clear-FaultEnv
Reset-Ini ''
Kill-StagedRiviv
$err5 = Read-Err 's5.err'
$dropped5 = Get-Lines $err5 'user effect chain dropped'
$fail5 = Get-Lines $err5 'display effect failure'
$latch5 = Get-Lines $err5 'degrade latched'
$armed5 = Get-Lines $err5 'fault injection armed'
$h5 = File-Hash $DumpS5
Check 'S5-1 fault injection armed line == 1, carrying chain=1' (($armed5.Count -eq 1) -and ($armed5[0] -match 'chain=1')) ("count=" + $armed5.Count + " lines=[" + ($armed5 -join ' | ') + "]")
Check 'S5-2 user effect chain dropped line == 1 (exactly one drop - more would be a repaint loop)' (($dropped5.Count -eq 1) -and ($dropped5[0] -match 'fault-injected effect chain build failure')) ("count=" + $dropped5.Count + " lines=[" + ($dropped5 -join ' | ') + "]")
Check 'S5-3 display effect failure drain lines == 1 (counted once, then the collapse)' ($fail5.Count -eq 1) ("count=" + $fail5.Count + " lines=[" + ($fail5 -join ' | ') + "]")
Check 'S5-4 ZERO degrade latches (one failure never reaches the threshold)' ($latch5.Count -eq 0) ("count=" + $latch5.Count + " lines=[" + ($latch5 -join ' | ') + "]")
Check 'S5-5 window alive through drop + revival toggles' $alive5 ('alive=' + $alive5)
Check 'S5-6 close dump == the S2 OFF dump (the session collapsed back; revival toggles ended OFF)' (($h5 -ne '(missing)') -and ($h5 -eq $h2m)) ('S5=' + (Short-Hash $h5) + ' S2-OFF=' + (Short-Hash $h2m))
Check 'S5-7 exit 0' ($code5 -eq 0) ('exit=' + $code5)

# ---------------------------------------------------------------------------
# S6: the D3 look-comparison artifact. huge.png fit-shrinks into the
# 1920x1200 window (height-limited: the axes differ) with the chain ON -
# the dump is KEPT for the human QA item (edge/ringing at the anisotropic
# shrink through the sharpen kernel); the machine asserts structure only.
# ---------------------------------------------------------------------------
$DumpS6 = Join-Path $Stage 'dump-s6-huge.png'
if (Test-Path $DumpS6) { Remove-Item $DumpS6 -Force }
$r6 = Run-Once $RunExe (New-Ini 'd2d' 5) ('"' + $Huge + '" -dump-viewport "' + $DumpS6 + '"') 's6.err' 'huge'
$d6 = Png-Dims $DumpS6
$sd6 = -1.0
if (Test-Path $DumpS6) { $sd6 = [S187Draw]::StdDev($DumpS6) }
Check 'S6-1 huge ON dump exists, window-sized' (($d6[0] -eq $d2m[0]) -and ($d6[1] -eq $d2m[1])) ('dims=' + $d6[0] + 'x' + $d6[1])
Check 'S6-2 huge dump non-uniform (content drawn; the artifact is for HUMAN edge inspection)' ($sd6 -gt 5.0) ('stddev=' + [Math]::Round($sd6, 2))
Check 'S6-3 exit 0' ($r6.Code -eq 0) ('exit=' + $r6.Code)

# ---------------------------------------------------------------------------
# Teardown: the ini removal is UNCONDITIONAL (a leftover ini fakes
# cross-build regressions); a leftover staged riviv process is a failure.
# The stage dir itself is KEPT as the archive (dumps + per-scenario logs).
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { ($_.Path -eq $RunExe) -or ($_.Path -eq $BaseRun) })
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S9 teardown: stage ini removed, no staged riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')

$total = $script:pass + $script:fail
if ($script:fail -gt 0) {
    Write-Output ('FAILURES: per-scenario stderr follows (archive dir ' + $Stage + ').')
    foreach ($e in @(@('S2-master', (Read-Err 's2m.err')), @('S2-base', (Read-Err 's2b.err')), @('S2w-master', (Read-Err 's2wm.err')), @('S2w-base', (Read-Err 's2wb.err')), @('S3-run1', (Read-Err 's3a.err')), @('S3-run2', (Read-Err 's3b.err')), @('S3w-run1', (Read-Err 's3wa.err')), @('S3w-run2', (Read-Err 's3wb.err')), @('S4-on', (Read-Err 's4a.err')), @('S4-off', (Read-Err 's4b.err')), @('S5', (Read-Err 's5.err')), @('S6', (Read-Err 's6.err')))) {
        Write-Output ('--- scenario ' + $e[0] + ' stderr ---')
        Write-Output $e[1]
    }
}
Write-Output ('SUMMARY smoke187 pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
if ($script:fail -gt 0) {
    Write-Output ('SMOKE187 FAIL ' + $script:pass + '/' + $total)
    exit 1
}
Write-Output ('SMOKE187 PASS ' + $script:pass + '/' + $total)
exit 0
