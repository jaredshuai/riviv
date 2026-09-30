# smoke173 - #172 the fault-injection seam: the env knob RIVIV_FAULT
# ("device=N,effect=N,ac_create=N,prepare=N") arms counters that
# reclassify real SUCCESSES as synthetic failures at gpu.rs's
# classification boundaries, so the REAL recovery machinery - the
# device-loss ladder (same-kind rebuild, 3-in-10s escalation to WARP,
# WARP failing too -> deferred fatal modal), the three-layer session
# ratchet (ac_surface_latched / wide_effect_latched) and the prepare
# escalation - digests them end to end. With the env unset the seam is
# inert (S0n pins that). ASCII-only source (PS 5.1 ANSI trap, #79
# lesson): the two em-dash stderr lines are matched by their ASCII tails
# ONLY ("device loss ladder", "feeding the device-loss ladder").
# Harness cribbed from smoke156-acarm.ps1: staged exe under
# %TEMP%\riviv-173-smoke (every launch runs ONLY the staged copy), the
# staged ini is rewritten before EVERY launch and deleted after every
# close (WM_CLOSE saves config), poll-based stderr waits (Read-Err with
# FileShare::ReadWrite works on the live redirect), Add-Type C# PNG
# fixture writer + dump stddev analyzer (System.Drawing via
# -ReferencedAssemblies), SetProcessDPIAware, GetExitCodeProcess on the
# raw handle captured WHILE ALIVE, path-targeted kills only, FATAL-exit
# when a foreign riviv is alive.
#
# Scenarios (one launch each; $env:RIVIV_FAULT set before Start-Process,
# removed after close):
#   S0  stage: foreign-riviv FATAL, leftover-ini FATAL, staged exe, the
#       cargo emitter (smoke156's test, RIVIV_S156_ICC env) writes p3.icc,
#       C# writes wide.png (320x240, iCCP=p3, deterministic gradient) and
#       probe.png (64x64, no iCCP)
#   S0n negative: wide.png, NO env: no "fault injection armed" line, the
#       AC baseline holds (output-surface=ac-scrgb present), exit 0
#   S1  device=1: armed line == 1, exactly one "device loss ladder" line,
#       zero backend=warp (same-kind rebuild), window alive, exit 0
#   S2  device=3: three ladder lines, escalation (backend=warp +
#       wide_blank), window alive, exit 0
#   S2b device=3 + -dump-viewport: S2 asserts + dump exists + non-uniform
#       (stddev > 5.0) - the wide image returns after re-derivation
#   S4  effect=9: the AC arm latches ("ac surface latched") then the
#       wide-effect ratchet latches ("wide effect latched"); counting is
#       boot-phase-sensitive so LATCH LINES are asserted, not failure
#       counts; dump non-uniform; exit 0
#   S6  ac_create=1: the AC face never builds - one "ac surface latched"
#       line whose detail contains "fault-injected AC surface refusal",
#       ZERO output-surface=ac-scrgb lines all session, exit 0
#   S7  prepare=3 (narrow probe.png + an external InvalidateRect paint
#       driver on the riviv_view child): a static image parks at one
#       failed upload (the pre-#90 blank-and-wait note in gpu.rs), so the
#       script supplies the paint pressure the escalation needs; three
#       consecutive upload failures then feed the device-loss ladder.
#   S7w the same on WIDE wide.png: #173's regression pin - post-fix
#       (frame_space seeded at create) the escalation fires with ZERO
#       ac-surface latching; pre-fix evidence: s7-prefix-red.err
#       (feeding=0 + a bogus ac latch with a "wiring bug" diagnostic).
#   S7s the stale-stamp pin (external review P2): the prepare injection
#       must sit AFTER prepare's frame_space stamp - ac_create=1 folds
#       to legacy, prepare#1 blanks the wide, the settle repaint stamps
#       F16P3, NavNext flips the arm to SrgbToDisplay on the SAME stack,
#       and prepare#2 must inject post-stamp (ZERO wiring-bug lines);
#       pre-fix red: s7s-prefix-red.err.
#   S3  STRETCH device=6: six ladder lines + backend=warp + the deferred
#       fatal modal (class #32770) dismissed via GetDlgItem(IDOK=2) +
#       BM_CLICK (WM_CLOSE fallback); exit code recorded; SKIP when the
#       modal resists, force-kill, never blocks the rest

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe',
      [string]$Repo = 'D:\codespace\riviv')

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Runtime.InteropServices;

public class S173 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr hwnd, IntPtr rect, bool erase);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dlg, int id);

    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int max);

    // The deferred-fatal modal: MessageBoxW creates a top-level #32770
    // dialog inside the staged process (null owner). Enumerate that
    // process's windows and collect the dialog class handles.
    public static List<IntPtr> DialogsOf(uint pid) {
        List<IntPtr> found = new List<IntPtr>();
        EnumProc cb = delegate(IntPtr h, IntPtr lp) {
            uint wpid;
            GetWindowThreadProcessId(h, out wpid);
            if (wpid == pid) {
                StringBuilder sb = new StringBuilder(256);
                GetClassName(h, sb, 256);
                if (sb.ToString() == "#32770") { found.Add(h); }
            }
            return true;
        };
        EnumWindows(cb, IntPtr.Zero);
        return found;
    }

    // ---- PNG fixture writing (smoke98/smoke156's verified recipe) ----
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
    static byte[] Scanlines(int w, int h, byte[] rgba) {
        byte[] raw = new byte[h * (1 + w * 4)];
        for (int y = 0; y < h; y++) {
            raw[y * (1 + w * 4)] = 0; // filter: none
            Array.Copy(rgba, y * w * 4, raw, y * (1 + w * 4) + 1, w * 4);
        }
        return raw;
    }
    static void WritePng(string path, int w, int h, byte[] rgba, byte[] icc) {
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
            kw.CopyTo(head, 0); head[kw.Length] = 0; head[kw.Length + 1] = 0;
            byte[] z = Zlib(icc);
            byte[] iccp = new byte[head.Length + z.Length];
            head.CopyTo(iccp, 0); z.CopyTo(iccp, head.Length);
            Chunk(ms, "iCCP", iccp);
        }
        Chunk(ms, "IDAT", Zlib(Scanlines(w, h, rgba)));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }
    static byte[] Solid(int w, int h, byte r, byte g, byte b) {
        byte[] px = new byte[w * h * 4];
        for (int i = 0; i < w * h; i++) { px[i*4] = r; px[i*4+1] = g; px[i*4+2] = b; px[i*4+3] = 255; }
        return px;
    }

    /// A plain 64x64 solid PNG with NO iCCP (narrow-content control).
    public static void WriteProbePng(string path) {
        WritePng(path, 64, 64, Solid(64, 64, 200, 60, 10), null);
    }

    /// The wide-content fixture: w x h RGBA8 carrying the iCCP profile
    /// bytes, deterministic pattern r = x*255/(w-1), g = y*255/(h-1),
    /// b = (x+y) mod 256 (smoke156's recipe).
    public static void WriteWidePng(string path, int w, int h, byte[] icc) {
        byte[] px = new byte[w * h * 4];
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                int i = (y * w + x) * 4;
                px[i] = (byte)(x * 255 / (w - 1));
                px[i+1] = (byte)(y * 255 / (h - 1));
                px[i+2] = (byte)((x + y) % 256);
                px[i+3] = 255;
            }
        }
        WritePng(path, w, h, px, icc);
    }
}
'@

Add-Type -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public class S173Draw {
    /// Grayscale mean/stddev over the decoded dump PNG (LockBits,
    /// 32bpp ARGB copy). A uniform (blank) dump has stddev ~0; the
    /// re-derived wide image lands well above 5.
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
# Harness
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
    # Path-targeted (#122 rule): only the staged copy is ever killed; the
    # developer's real viewer never matches the deterministic stage path.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $RunExe })
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
function Start-Riv($argStr, $errName) {
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
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
function Close-Main($p, $main) {
    if (($main -ne $null) -and ($main -ne [IntPtr]::Zero)) {
        [void][S173]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    return (Wait-Exit $p 30000)
}
function Wait-Exit($p, $ms) {
    # Returns the exit code, or $null when still alive after $ms.
    if ($p.WaitForExit($ms)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S173]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else { return -2 }
        return $code
    }
    return $null
}
# Poll the LIVE stderr until the predicate holds (or the deadline passes);
# returns the last full stderr text either way.
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
# Every whole line of $err matching $pattern (CRLF-safe; the leading comma
# keeps the ARRAY shape through pipeline unroll - the smoke156 lesson).
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
# Run one cargo test invocation in $Repo with the caller's env vars set;
# all output to a capture file (the PS 5.1 native-stderr trap avoided via
# a Continue window around the redirection). Returns @{ Code; Out }.
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
# One launch -> adopt -> settle. The ini is rewritten first (WM_CLOSE saves
# config; a leftover key would fake cross-scenario regressions).
function Start-Scenario($envVal, $argStr, $errName, $leaf = 'wide') {
    Reset-Ini $IniText
    Clear-FaultEnv
    if ($envVal -ne '') { $env:RIVIV_FAULT = $envVal }
    $p = Start-Riv $argStr $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p $leaf 15000
    if ($adopted) { Start-Sleep -Milliseconds 2000 }   # paint settle
    return @{ P = $p; Main = $main; Adopted = $adopted }
}
function End-Scenario($sc, $errName) {
    $code = Close-Main $sc.P $sc.Main
    Clear-FaultEnv
    Reset-Ini ''
    Kill-StagedRiviv
    return @{ Code = $code; Err = (Read-Err $errName) }
}

[void][S173]::SetProcessDPIAware()

# ---------------------------------------------------------------------------
# S0: the stage. The foreign-riviv check fires BEFORE anything else: a real
# viewer running would take the command line over (second-instance
# forwarding would destroy every assertion below). A leftover ini in the
# stage is FATAL too (cross-build false-regression discipline).
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-173-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'
$preExisting = @(Get-Process riviv -ErrorAction SilentlyContinue)
$foreign = @($preExisting | Where-Object { $_.Path -ne $RunExe })
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
Copy-Item $Exe $RunExe -Force
Check 'S0a stage ready: staged exe copied, ini absent, no foreign riviv' ((Test-Path $RunExe) -and (-not (Test-Path $Ini))) ('exe=' + (Test-Path $RunExe) + ' iniExists=' + (Test-Path $Ini))

# The p3.icc destination profile comes from riviv's own cargo emitter
# (smoke156's env-gated test); the env var must reach the cargo child.
$P3Icc = Join-Path $Stage 'p3.icc'
Clear-FaultEnv
$env:RIVIV_S156_ICC = $P3Icc
$c0 = Run-CargoTest 'smoke156_write_p3_profile' 'cargo-s0.txt'
Remove-Item Env:\RIVIV_S156_ICC -ErrorAction SilentlyContinue
$iccBytes = 0
if (Test-Path $P3Icc) { $iccBytes = (Get-Item $P3Icc).Length }
# -cnotmatch (case-SENSITIVE): the legitimate "0 failed" would match
# a case-insensitive 'FAILED'.
$iccOk = ($c0.Code -eq 0) -and ($c0.Out -match 'test result: ok') -and ($c0.Out -match '1 passed') -and ($c0.Out -cnotmatch 'FAILED') -and ($iccBytes -eq 6680)
Check 'S0b cargo emitter wrote p3.icc (6680 bytes, test result: ok, 1 passed)' $iccOk ("exit=$($c0.Code) bytes=$iccBytes out=[$($c0.Out.Trim())]")
if (-not $iccOk) {
    Write-Output ('FAILURES: S0 emitter failed - evidence kept in ' + $Stage)
    Write-Output ('SUMMARY smoke173 pass=' + $script:pass + ' fail=' + ($script:fail + 1))
    Write-Output 'SMOKE173 RESULT: FAIL'
    Clear-FaultEnv
    exit 1
}

$Wide = Join-Path $Stage 'wide.png'
[S173]::WriteWidePng($Wide, 320, 240, [IO.File]::ReadAllBytes($P3Icc))
$wd = Png-Dims $Wide
Check 'S0c wide.png written (320x240, iCCP=p3.icc)' ((Test-Path $Wide) -and ($wd[0] -eq 320) -and ($wd[1] -eq 240)) ("dims=$($wd[0])x$($wd[1]) size=" + (Get-Item $Wide).Length)
$Probe = Join-Path $Stage 'probe.png'
[S173]::WriteProbePng($Probe)
$pd = Png-Dims $Probe
Check 'S0d probe.png written (64x64, no iCCP)' (($pd[0] -eq 64) -and ($pd[1] -eq 64)) ('dims=' + $pd[0] + 'x' + $pd[1])

# The pinned INI (x/y fix the window across scenarios; icm=1 keeps the
# machine's AC arm baseline reachable).
$IniText = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nicm=1`r`nrenderer=d2d`r`nshow_menu=0`r`n"

# ---------------------------------------------------------------------------
# S0n: the negative control. No RIVIV_FAULT: the seam is inert (no armed
# breadcrumb) and the machine's wide-content AC baseline holds.
# ---------------------------------------------------------------------------
$sc = Start-Scenario '' ('"' + $Wide + '"') 's0n.err'
[void](Wait-Err 's0n.err' { param($e) ((Get-Lines $e 'output-surface=ac-scrgb').Count -ge 1) -and ((Get-Lines $e 'display-stage=').Count -ge 1) } 20000)
$r0n = End-Scenario $sc 's0n.err'
$err0n = $r0n.Err
Check 'S0n-1 zero fault-injection-armed lines with the env unset' ((Get-Lines $err0n 'fault injection armed').Count -eq 0) ("count=" + (Get-Lines $err0n 'fault injection armed').Count + " stderr=[$($err0n.Trim())]")
Check 'S0n-2 display-stage line present (baseline)' ((Get-Lines $err0n 'display-stage=').Count -ge 1) ("count=" + (Get-Lines $err0n 'display-stage=').Count)
Check 'S0n-3 output-surface=ac-scrgb present (this machine AC arm baseline)' ((Get-Lines $err0n 'output-surface=ac-scrgb').Count -ge 1) ("count=" + (Get-Lines $err0n 'output-surface=ac-scrgb').Count)
Check 'S0n-4 exit 0' ($r0n.Code -eq 0) ('exit=' + $r0n.Code)

# ---------------------------------------------------------------------------
# S1: device=1 - one synthetic loss, the ladder answers with a same-kind
# rebuild (backend stays hw, zero warp), the window survives, exit 0.
# ---------------------------------------------------------------------------
$sc = Start-Scenario 'device=1' ('"' + $Wide + '"') 's1.err'
[void](Wait-Err 's1.err' { param($e) (Get-Lines $e 'device loss ladder').Count -ge 1 } 20000)
Start-Sleep -Milliseconds 1000   # let the rebuild's identity lines land
$alive1 = Test-Alive $sc.P
$r1 = End-Scenario $sc 's1.err'
$err1 = $r1.Err
Check 'S1-1 fault injection armed line == 1' ((Get-Lines $err1 'fault injection armed').Count -eq 1) ("count=" + (Get-Lines $err1 'fault injection armed').Count)
Check 'S1-2 device loss ladder lines == 1' ((Get-Lines $err1 'device loss ladder').Count -eq 1) ("count=" + (Get-Lines $err1 'device loss ladder').Count + " lines=[" + ((Get-Lines $err1 'device loss ladder') -join ' | ') + "]")
Check 'S1-3 zero backend=warp lines (same-kind rebuild)' ((Get-Lines $err1 'backend=warp').Count -eq 0) ("count=" + (Get-Lines $err1 'backend=warp').Count)
Check 'S1-4 window alive before close' $alive1 ('alive=' + $alive1)
Check 'S1-5 exit 0' ($r1.Code -eq 0) ('exit=' + $r1.Code)

# ---------------------------------------------------------------------------
# S2: device=3 - three ladder lines then the 3-in-10s escalation: WARP +
# the wide_blank transient word.
# ---------------------------------------------------------------------------
$sc = Start-Scenario 'device=3' ('"' + $Wide + '"') 's2.err'
[void](Wait-Err 's2.err' { param($e) ((Get-Lines $e 'device loss ladder').Count -ge 3) -and ((Get-Lines $e 'backend=warp').Count -ge 1) -and ((Get-Lines $e 'wide_blank').Count -ge 1) } 30000)
Start-Sleep -Milliseconds 1000
$alive2 = Test-Alive $sc.P
$r2 = End-Scenario $sc 's2.err'
$err2 = $r2.Err
Check 'S2-1 device loss ladder lines == 3' ((Get-Lines $err2 'device loss ladder').Count -eq 3) ("count=" + (Get-Lines $err2 'device loss ladder').Count)
Check 'S2-2 backend=warp line present (escalation)' ((Get-Lines $err2 'backend=warp').Count -ge 1) ("count=" + (Get-Lines $err2 'backend=warp').Count)
Check 'S2-3 wide_blank stage word present' ((Get-Lines $err2 'wide_blank').Count -ge 1) ("count=" + (Get-Lines $err2 'wide_blank').Count + " lines=[" + ((Get-Lines $err2 'wide_blank') -join ' | ') + "]")
Check 'S2-4 window alive before close' $alive2 ('alive=' + $alive2)
Check 'S2-5 exit 0' ($r2.Code -eq 0) ('exit=' + $r2.Code)

# ---------------------------------------------------------------------------
# S2b: device=3 + the close-time viewport dump: the WARP session re-derives
# the wide masters, so the dump is window-sized and NON-uniform.
# ---------------------------------------------------------------------------
$dumpS2 = Join-Path $Stage 'dump-s2.png'
if (Test-Path $dumpS2) { Remove-Item $dumpS2 -Force }
$sc = Start-Scenario 'device=3' ('"' + $Wide + '" -dump-viewport "' + $dumpS2 + '"') 's2b.err'
[void](Wait-Err 's2b.err' { param($e) ((Get-Lines $e 'device loss ladder').Count -ge 3) -and ((Get-Lines $e 'backend=warp').Count -ge 1) -and ((Get-Lines $e 'wide_blank').Count -ge 1) } 30000)
Start-Sleep -Milliseconds 2000   # re-derivation + repaint settle
$alive2b = Test-Alive $sc.P
$r2b = End-Scenario $sc 's2b.err'
$err2b = $r2b.Err
$sd2 = -1.0
if (Test-Path $dumpS2) { $sd2 = [S173Draw]::StdDev($dumpS2) }
Check 'S2b-1 device loss ladder lines == 3' ((Get-Lines $err2b 'device loss ladder').Count -eq 3) ("count=" + (Get-Lines $err2b 'device loss ladder').Count)
Check 'S2b-2 backend=warp line present' ((Get-Lines $err2b 'backend=warp').Count -ge 1) ("count=" + (Get-Lines $err2b 'backend=warp').Count)
Check 'S2b-3 wide_blank stage word present' ((Get-Lines $err2b 'wide_blank').Count -ge 1) ("count=" + (Get-Lines $err2b 'wide_blank').Count)
Check 'S2b-4 window alive before close' $alive2b ('alive=' + $alive2b)
Check 'S2b-5 dump file exists' (Test-Path $dumpS2) ('dump=' + $dumpS2)
Check 'S2b-6 dump non-uniform (stddev > 5.0; wide image returned after re-derivation)' ($sd2 -gt 5.0) ('stddev=' + [Math]::Round($sd2, 2))
Check 'S2b-7 exit 0' ($r2b.Code -eq 0) ('exit=' + $r2b.Code)

# ---------------------------------------------------------------------------
# S4: effect=9 - the AC arm latches (ac surface latched) and the
# wide-effect ratchet latches (wide effect latched -> re-derivation).
# Counts are boot-phase-sensitive: LATCH LINES are asserted, not failure
# counts. The close-time dump must be non-uniform (the wide image returns
# re-derived to srgb).
# ---------------------------------------------------------------------------
$dumpS4 = Join-Path $Stage 'dump-s4.png'
if (Test-Path $dumpS4) { Remove-Item $dumpS4 -Force }
$sc = Start-Scenario 'effect=9' ('"' + $Wide + '" -dump-viewport "' + $dumpS4 + '"') 's4.err'
[void](Wait-Err 's4.err' { param($e) ((Get-Lines $e 'ac surface latched').Count -ge 1) -and ((Get-Lines $e 'wide effect latched').Count -ge 1) } 45000)
Start-Sleep -Milliseconds 3000   # re-derivation + repaint settle
$alive4 = Test-Alive $sc.P
$r4 = End-Scenario $sc 's4.err'
$err4 = $r4.Err
$sd4 = -1.0
if (Test-Path $dumpS4) { $sd4 = [S173Draw]::StdDev($dumpS4) }
$acLatch4 = Get-Lines $err4 'ac surface latched'
Check 'S4-1 ac surface latched line present' ($acLatch4.Count -ge 1) ("count=" + $acLatch4.Count + " lines=[" + ($acLatch4 -join ' | ') + "]")
Check 'S4-2 wide effect latched line present' ((Get-Lines $err4 'wide effect latched').Count -ge 1) ("count=" + (Get-Lines $err4 'wide effect latched').Count)
Check 'S4-3 window alive before close' $alive4 ('alive=' + $alive4)
Check 'S4-4 dump non-uniform (stddev > 5.0; re-derived wide image returned)' ($sd4 -gt 5.0) ('stddev=' + [Math]::Round($sd4, 2))
Check 'S4-5 exit 0' ($r4.Code -eq 0) ('exit=' + $r4.Code)

# ---------------------------------------------------------------------------
# S6: ac_create=1 - the AC surface creation itself refuses: the AC face
# never builds (zero output-surface=ac-scrgb all session), the session
# folds to the legacy face, exit 0.
# ---------------------------------------------------------------------------
$sc = Start-Scenario 'ac_create=1' ('"' + $Wide + '"') 's6.err'
[void](Wait-Err 's6.err' { param($e) (Get-Lines $e 'ac surface latched.*fault-injected AC surface refusal').Count -ge 1 } 20000)
Start-Sleep -Milliseconds 1000
$r6 = End-Scenario $sc 's6.err'
$err6 = $r6.Err
$refusal6 = Get-Lines $err6 'ac surface latched.*fault-injected AC surface refusal'
Check 'S6-1 ac surface latched line with the fault-injected refusal detail' ($refusal6.Count -ge 1) ("count=" + $refusal6.Count + " lines=[" + ($refusal6 -join ' | ') + "]")
Check 'S6-2 zero output-surface=ac-scrgb lines all session (AC face never built)' ((Get-Lines $err6 'output-surface=ac-scrgb').Count -eq 0) ("count=" + (Get-Lines $err6 'output-surface=ac-scrgb').Count)
Check 'S6-3 exit 0' ($r6.Code -eq 0) ('exit=' + $r6.Code)

# ---------------------------------------------------------------------------
# S7 + S7w: the prepare escalation on BOTH fixtures (a static image parks
# at one failed upload - the pre-#90 blank-and-wait note - so the script
# drives paint pressure from outside: InvalidateRect on the riviv_view
# child every 200ms until the escalation line lands).
#   S7  narrow probe.png: the plain escalation path.
#   S7w WIDE wide.png: #173's regression pin. Pre-fix (archived
#       s7-prefix-red.err of the first matrix run) the first prepare
#       failure on a fresh AC stack was rerouted by the blank path into a
#       display-effect failure: feeding=0 AND an ac-surface latch with a
#       bogus "wiring bug" diagnostic. Post-fix (frame_space seeded with
#       the session's content class at create) the blank draws through
#       the effect and the escalation fires with ZERO latching.
# ---------------------------------------------------------------------------
$s7cases = @(
    @{ Name = 'S7';  Fixture = $Probe; Leaf = 'probe'; BanLatch = $false },
    @{ Name = 'S7w'; Fixture = $Wide;  Leaf = 'wide';  BanLatch = $true }
)
foreach ($c7 in $s7cases) {
    $errName7 = $c7.Name.ToLower() + '.err'
    $sc = Start-Scenario 'prepare=3' ('"' + $c7.Fixture + '"') $errName7 $c7.Leaf
    $view7 = [S173]::FindWindowExW($sc.Main, [IntPtr]::Zero, 'riviv_view', [NullString]::Value)
    $driveDeadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $driveDeadline) {
        $e7 = Read-Err $errName7
        if ((Get-Lines $e7 'feeding the device-loss ladder').Count -ge 1) { break }
        if ($sc.P.HasExited) { break }
        if ($view7 -ne [IntPtr]::Zero) { [void][S173]::InvalidateRect($view7, [IntPtr]::Zero, $false) }
        Start-Sleep -Milliseconds 200
    }
    Start-Sleep -Milliseconds 1000
    $alive7 = Test-Alive $sc.P
    $r7 = End-Scenario $sc $errName7
    $err7 = $r7.Err
    Check ($c7.Name + '-0 paint driver had a view child to press') ($view7 -ne [IntPtr]::Zero) ('view=' + $view7)
    Check ($c7.Name + '-1 frame upload failed lines == 3') ((Get-Lines $err7 'frame upload failed').Count -eq 3) ("count=" + (Get-Lines $err7 'frame upload failed').Count)
    Check ($c7.Name + '-2 feeding-the-device-loss-ladder escalation line == 1') ((Get-Lines $err7 'feeding the device-loss ladder').Count -eq 1) ("count=" + (Get-Lines $err7 'feeding the device-loss ladder').Count + " lines=[" + ((Get-Lines $err7 'feeding the device-loss ladder') -join ' | ') + "]")
    if ($c7.BanLatch) {
        Check ($c7.Name + '-3 ZERO ac surface latched lines (#173 regression pin: no reroute)') ((Get-Lines $err7 'ac surface latched').Count -eq 0) ("count=" + (Get-Lines $err7 'ac surface latched').Count + " lines=[" + ((Get-Lines $err7 'ac surface latched') -join ' | ') + "]")
    }
    Check ($c7.Name + '-4 window alive before close') $alive7 ('alive=' + $alive7)
    Check ($c7.Name + '-5 exit 0') ($r7.Code -eq 0) ('exit=' + $r7.Code)
}

# ---------------------------------------------------------------------------
# S7s: the stale-stamp pin (external review finding 1, P2). The prepare
# injection used to sit BEFORE prepare's frame_space stamp, so on a
# stack whose stamp is stale (an arm flip without a face rebuild) the
# synthetic failure fed the mismatch table a bogus "wiring bug"
# refusal. Choreography: ac_create=1 forces the creation-time latch +
# fold to legacy (seeded F16P3); prepare#1 blanks the wide; the settle
# repaint's prepare SUCCEEDS (the F16P3 stamp lands); NavNext (WM_COMMAND
# 107) brings the narrow fixture on the SAME stack - the arm flips
# SrgbToDisplay - and prepare#2 must inject AFTER stamping (post-fix:
# clean blank, zero wiring lines). Pre-fix red evidence:
# s7s-prefix-red.err (wiring bug x1 + display effect failure x1; a
# debug build would have panicked the debug_assert on this path).
# ---------------------------------------------------------------------------
Reset-Ini $IniText
Clear-FaultEnv
$env:RIVIV_FAULT = 'ac_create=1,prepare=2'
$ps7s = Start-Riv ('"' + $Wide + '" "' + $Probe + '"') 's7s.err'
$main7s = Wait-Main $ps7s
[void](Wait-Title $ps7s 'wide' 15000)
$latch7s = $false
$dl7s = [DateTime]::UtcNow.AddSeconds(20)
while ([DateTime]::UtcNow -lt $dl7s) {
    if ((Get-Lines (Read-Err 's7s.err') 'ac surface latched').Count -ge 1) { $latch7s = $true; break }
    if ($ps7s.HasExited) { break }
    Start-Sleep -Milliseconds 200
}
# settle LONG: the blanked wide must repaint through a SUCCESSFUL prepare
# so the F16P3 stamp lands on the folded stack before the navigation
Start-Sleep -Milliseconds 4000
if ($main7s -ne [IntPtr]::Zero) {
    [void][S173]::PostMessage($main7s, 0x0111, [IntPtr]107, [IntPtr]::Zero)
}
$dl7s2 = [DateTime]::UtcNow.AddSeconds(20)
while ([DateTime]::UtcNow -lt $dl7s2) {
    if ((Get-Lines (Read-Err 's7s.err') 'frame upload failed').Count -ge 2) { break }
    if ($ps7s.HasExited) { break }
    Start-Sleep -Milliseconds 200
}
Start-Sleep -Milliseconds 1500
$alive7s = (($main7s -ne [IntPtr]::Zero) -and (-not $ps7s.HasExited))
if (-not $ps7s.HasExited -and $main7s -ne [IntPtr]::Zero) {
    [void][S173]::PostMessage($main7s, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
}
$code7s = Wait-Exit $ps7s 30000
if ($code7s -eq $null) { Stop-Process -Id $ps7s.Id -Force -ErrorAction SilentlyContinue; $code7s = -1 }
Clear-FaultEnv
Reset-Ini ''
Kill-StagedRiviv
$err7s = Read-Err 's7s.err'
Check 'S7s-0 ac surface latched (the fold that keeps a legacy stack)' ($latch7s -and ((Get-Lines $err7s 'ac surface latched.*fault-injected AC surface refusal').Count -ge 1)) ("latchWaited=" + $latch7s + " lines=[" + ((Get-Lines $err7s 'ac surface latched') -join ' | ') + "]")
Check 'S7s-1 frame upload failed lines == 2 (wide blank + stale-stamp narrow)' ((Get-Lines $err7s 'frame upload failed').Count -eq 2) ("count=" + (Get-Lines $err7s 'frame upload failed').Count)
Check 'S7s-2 ZERO wiring-bug lines (review P2 pin: the injection stamps first)' ((Get-Lines $err7s 'wiring bug').Count -eq 0) ("count=" + (Get-Lines $err7s 'wiring bug').Count + " lines=[" + ((Get-Lines $err7s 'wiring bug') -join ' | ') + "]")
Check 'S7s-3 ZERO display effect failure lines (no spurious ratchet feed)' ((Get-Lines $err7s 'display effect failure').Count -eq 0) ("count=" + (Get-Lines $err7s 'display effect failure').Count)
Check 'S7s-4 window alive before close' $alive7s ('alive=' + $alive7s)
Check 'S7s-5 exit 0' ($code7s -eq 0) ('exit=' + $code7s)

# ---------------------------------------------------------------------------
# S3 STRETCH: device=6 - two same-kind rebuilds, the 3rd escalates to WARP,
# three more on WARP trip the deferred fatal: a modal dialog (class
# #32770) blocks exit. Dismissed via GetDlgItem(IDOK=2) + BM_CLICK (the
# WM_CLOSE fallback second). Everything recorded honestly; the scenario
# SKIPs (and force-kills) when the modal resists, never blocking the rest.
# ---------------------------------------------------------------------------
Reset-Ini $IniText
Clear-FaultEnv
$env:RIVIV_FAULT = 'device=6'
$p3s = Start-Riv ('"' + $Wide + '"') 's3.err'
$main3 = Wait-Main $p3s
[void](Wait-Title $p3s 'wide' 15000)
Start-Sleep -Milliseconds 2000
$err3 = Wait-Err 's3.err' { param($e) (Get-Lines $e 'device loss ladder').Count -ge 6 } 40000
$ladder3 = (Get-Lines $err3 'device loss ladder').Count
$warp3 = (Get-Lines $err3 'backend=warp').Count
Check 'S3-1 device loss ladder lines == 6 (2 rebuilds + escalate + 3 on WARP)' ($ladder3 -eq 6) ("count=$ladder3 stderr=[$($err3.Trim())]")
Check 'S3-2 backend=warp line present (escalated)' ($warp3 -ge 1) ("count=$warp3")
$modalSeen = $false
$dismissPath = ''
$dlgDeadline = [DateTime]::UtcNow.AddSeconds(20)
while ([DateTime]::UtcNow -lt $dlgDeadline) {
    $dlgs = [S173]::DialogsOf([uint32]$p3s.Id)
    if ($dlgs.Count -gt 0) {
        $modalSeen = $true
        $btn = [S173]::GetDlgItem($dlgs[0], 2)   # IDOK
        if ($btn -ne [IntPtr]::Zero) {
            $dismissPath = 'GetDlgItem(IDOK=2)+BM_CLICK'
            [void][S173]::PostMessage($btn, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
        } else {
            $dismissPath = 'WM_CLOSE-to-dialog fallback'
            [void][S173]::PostMessage($dlgs[0], 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        break
    }
    if ($p3s.HasExited) { break }
    Start-Sleep -Milliseconds 200
}
if ($modalSeen) {
    Check 'S3-3 deferred fatal modal observed (#32770)' $true ('dismiss=' + $dismissPath)
    $code3 = Wait-Exit $p3s 10000
    if ($code3 -eq $null) {
        # The click did not land: the WM_CLOSE fallback on the dialog.
        $dlgs2 = [S173]::DialogsOf([uint32]$p3s.Id)
        if ($dlgs2.Count -gt 0) {
            $dismissPath += ' then WM_CLOSE'
            [void][S173]::PostMessage($dlgs2[0], 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        $code3 = Wait-Exit $p3s 10000
    }
    if ($code3 -ne $null) {
        Check 'S3-4 process exits after dismissal (exit code recorded)' $true ('exit=' + $code3 + ' path=' + $dismissPath)
    } else {
        Stop-Process -Id $p3s.Id -Force -ErrorAction SilentlyContinue
        $p3s.WaitForExit(3000) | Out-Null
        Skip 'S3-4' 'modal undismissed (stretch) - staged exe force-killed'
    }
} else {
    Check 'S3-3 deferred fatal modal observed (#32770)' $false 'no #32770 window of the staged pid within 20s (or process exited first)'
    if (-not $p3s.HasExited) {
        Stop-Process -Id $p3s.Id -Force -ErrorAction SilentlyContinue
        $p3s.WaitForExit(3000) | Out-Null
    }
    Skip 'S3-4' 'modal undismissed (stretch)'
}
Clear-FaultEnv
Reset-Ini ''
Kill-StagedRiviv

# ---------------------------------------------------------------------------
# Teardown: the ini removal is UNCONDITIONAL (a leftover ini fakes
# cross-build regressions); a leftover staged riviv process is a failure.
# The stage dir itself is KEPT as the archive (per-scenario stderr logs).
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe })
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S9 teardown: stage ini removed, no staged riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')

$total = $script:pass + $script:fail
if ($script:fail -gt 0) {
    Write-Output ('FAILURES: per-scenario stderr follows (archive dir ' + $Stage + ').')
    foreach ($e in @(@('S0n', (Read-Err 's0n.err')), @('S1', (Read-Err 's1.err')), @('S2', (Read-Err 's2.err')), @('S2b', (Read-Err 's2b.err')), @('S4', (Read-Err 's4.err')), @('S6', (Read-Err 's6.err')), @('S7', (Read-Err 's7.err')), @('S7w', (Read-Err 's7w.err')), @('S7s', (Read-Err 's7s.err')), @('S3', (Read-Err 's3.err')))) {
        Write-Output ('--- scenario ' + $e[0] + ' stderr ---')
        Write-Output $e[1]
    }
}
Write-Output ('SUMMARY smoke173 pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
if ($script:fail -gt 0) {
    Write-Output ('SMOKE173 FAIL ' + $script:pass + '/' + $total)
    exit 1
}
Write-Output ('SMOKE173 PASS ' + $script:pass + '/' + $total)
exit 0
