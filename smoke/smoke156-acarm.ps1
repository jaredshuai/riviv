# smoke156 - #156 M8-14 the L2 GPU AC arm: the scRGB output face, the
# wide-content breadcrumb word, the AC-face dump channel, and the
# half-exact oracle. ASCII-only source (PS 5.1 ANSI/param-binding trap,
# #79 lesson). Harness skeleton cribbed from smoke98/smoke130: staged exe
# under %TEMP%\riviv-s156, poll-based waits, Add-Type C# PNG fixture
# writer, SetProcessDPIAware, GetExitCodeProcess on the captured live
# handle, exit codes via the raw process handle.
#
# Observable surface (#156; exact stderr forms, src/gpu.rs + src/window.rs):
#   riviv: output-surface=ac-scrgb                     <- AC face creation ONLY (legacy face: zero output)
#   riviv: display-stage=<word> profile=<n|none> backend=<hw|warp> ac=<off|on|unknown>
#       <- unchanged format; the wide-content AC arm word = p3_to_scrgb
#   riviv: dump-viewport face=ac-scrgb w=<W> h=<H> ac-draw=<X.XXX>ms  <- AC dump only
#   riviv: dump-viewport output_gen=<n>                <- unchanged; wide single-image session expects 2
# The AC dump output file = RAW LITTLE-ENDIAN u16 RGBA quadruples
# (W*H*4 halves, 8 bytes per pixel, no PNG container). On the AC face the
# dump draw runs twice (warm-up + measurement) and the line times the
# second pass. The legacy face dump stays PNG + output_gen with zero new
# lines. WARP never runs the AC arm (the face is always legacy there).
#
# Machine pin: ACM+HDR ON (type9 bit1 = on) and the WCS display getter
# returns a Custom-class profile (TPLCD AdobeRGB) - a wide ICC image on
# hardware MUST take the AC arm on this machine. On a machine without
# ACM/HDR the S2 assertions will not hold; the smoke pins THIS machine's
# contract and FAILS honestly elsewhere.
#
# Scenarios:
#   S0 stage: foreign-riviv FATAL check, staged exe, cargo emitter writes
#      p3.icc (riviv's own P3 destination profile, 6680 bytes), C# writes
#      probe.png (64x64 solid, no iCCP); leftover-ini pre-checks
#   S1 geometry probe: probe.png dump under the pinned 1080p-CLIENT INI
#      -> the dump PNG's size IS the client area (VW x VH, physical
#      pixels on this 200% DPI machine; VW in [1800,1920], VH in
#      [1000,1080] - the INI window height (1200) minus this machine's
#      chrome (caption+bars, ~177 physical px) lands the client there.
#   S2 AC arm live anchor: wide.png (VW x VH, iCCP = p3.icc, non-trivial
#      gradient) dumped to s2.f16; exit 0; output-surface=ac-scrgb exactly
#      once; display-stage=p3_to_scrgb line; face=ac-scrgb dump line with
#      w/h = VW/VH and ac-draw <= 8.0; output_gen=2; f16 byte count
#      VW*VH*8; then cargo test --release -- --ignored smoke156_half_oracle
#      (the env-gated half oracle) must PASS with exactly 1 filtered-in test
#   S3 WARP control: same wide.png under renderer=warp -> no
#      output-surface line, no face=ac-scrgb line, a display-stage line
#      exists (the word is machine-dependent on warp: the judge still
#      answers, the narrow stage it names varies), dump is a valid PNG
#   S4 narrow zero-disturbance: probe.png under renderer=d2d (the ACM
#      machine's narrow content runs gpu_effect) -> no output-surface/
#      face= lines, display-stage line exists, output_gen line exists,
#      dump PNG valid
#   S5 teardown: kill staged leftovers, remove the stage ini (a leftover
#      ini would fake cross-build regressions)

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe',
      [string]$Repo = 'D:\codespace\riviv')

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Runtime.InteropServices;

public class S156 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    // ---- PNG fixture writing (smoke98's verified chunk/zlib recipe) ----
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

    /// A plain 64x64 solid PNG with NO iCCP (the narrow-content fixture).
    public static void WriteProbePng(string path) {
        WritePng(path, 64, 64, Solid(64, 64, 200, 60, 10), null);
    }

    /// The wide-content fixture: VW x VH RGBA8 with the iCCP profile bytes
    /// and a non-trivial deterministic pattern:
    /// r = x*255/(w-1), g = y*255/(h-1), b = (x+y) mod 256.
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

# ---------------------------------------------------------------------------
# Harness
# ---------------------------------------------------------------------------
$script:pass = 0
$script:fail = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ('PASS ' + $name + ' -- ' + $detail) }
    else { $script:fail++; Write-Output ('FAIL ' + $name + ' -- ' + $detail) }
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
function Close-Main($p, $main) {
    if (($main -ne $null) -and ($main -ne [IntPtr]::Zero)) {
        [void][S156]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(30000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S156]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else { return -2 }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}
# One adopt -> settle -> WM_CLOSE dump instance with its own ini text.
function Run-Dump($imgPath, $outName, $errName, $iniText, $titleLeaf) {
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    Reset-Ini $iniText
    $p = Start-Riv ('"' + $imgPath + '" -dump-viewport "' + $out + '"') $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p $titleLeaf 15000
    if ($adopted) { Start-Sleep -Milliseconds 2000 }   # paint settle (decode + wide face transfer)
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Main = $main; Adopted = $adopted; Err = (Read-Err $errName) }
}
# Every whole line of $err matching $pattern (anchored regexes; CRLF-safe).
# The leading comma keeps the ARRAY shape through PowerShell pipeline
# unroll: a bare `return $hits' on a single hit degrades to a SCALAR
# string, and $lines[0] then reads the first CHARACTER - the S2e re-match
# would silently fail on exactly the one-line case it exists for.
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

[void][S156]::SetProcessDPIAware()

# ---------------------------------------------------------------------------
# S0: the stage. The foreign-riviv check fires BEFORE anything else: a real
# viewer running would take the command line over (second-instance
# forwarding would destroy every assertion below).
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-s156'
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
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); exit 2 }
Copy-Item $Exe $RunExe -Force
# Leftover-ini pre-checks (cross-build false-regression discipline): the
# stage was just wiped, and the SOURCE exe dir must not carry an ini either.
$srcIni = Join-Path (Split-Path $Exe -Parent) 'riviv.ini'
Check 'S0a staged exe copied, stage ini absent, source exe dir ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini)) -and (-not (Test-Path $srcIni))) ('stageIni=' + (Test-Path $Ini) + ' srcIni=' + (Test-Path $srcIni))

$P3Icc = Join-Path $Stage 'p3.icc'
$env:RIVIV_S156_ICC = $P3Icc
$c0 = Run-CargoTest 'smoke156_write_p3_profile' 'cargo-s0.txt'
Remove-Item Env:\RIVIV_S156_ICC -ErrorAction SilentlyContinue
$iccBytes = 0
if (Test-Path $P3Icc) { $iccBytes = (Get-Item $P3Icc).Length }
# -cnotmatch (case-SENSITIVE): -notmatch is case-insensitive in PS and the
# legitimate "0 failed" in the result line would match 'FAILED'.
$iccOk = ($c0.Code -eq 0) -and ($c0.Out -match 'test result: ok') -and ($c0.Out -match '1 passed') -and ($c0.Out -cnotmatch 'FAILED') -and ($iccBytes -eq 6680)
Check 'S0b cargo emitter wrote p3.icc (6680 bytes, test result: ok, 1 passed)' $iccOk ("exit=$($c0.Code) bytes=$iccBytes out=[$($c0.Out.Trim())]")
if (-not $iccOk) {
    Write-Output ('FAILURES: S0 emitter failed - evidence kept in ' + $Stage)
    Write-Output ('SUMMARY smoke156 pass=' + $script:pass + ' fail=' + ($script:fail + 1))
    Write-Output 'SMOKE156 RESULT: FAIL'
    exit 1
}
$Probe = Join-Path $Stage 'probe.png'
[S156]::WriteProbePng($Probe)
Check 'S0c probe.png written (64x64, no iCCP)' (Test-Path $Probe) 'fixture write failed'

# The pinned 1080p-caliber INI (x/y fix the window across scenarios; the
# 200% DPI machine reports the dump in physical pixels).
$IniText = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nicm=1`r`nrenderer=d2d`r`nshow_menu=0`r`n"

# ---------------------------------------------------------------------------
# S1: the geometry probe. The dump PNG's size IS the viewport client area
# (physical pixels) - the numbers every later fixture is sized to.
# ---------------------------------------------------------------------------
$r1 = Run-Dump $Probe 's1.png' 's1.err' $IniText 'probe'
$dims1 = Png-Dims $r1.Out
$VW = $dims1[0]
$VH = $dims1[1]
$geomOk = ($r1.Code -eq 0) -and (Test-Path $r1.Out) -and ($VW -ge 1800) -and ($VW -le 1920) -and ($VH -ge 1000) -and ($VH -le 1080)
Check 'S1 geometry probe: exit 0, dump is the client area, VW in [1800,1920], VH in [1000,1080]' $geomOk ("exit=$($r1.Code) dump=$($r1.Out) VW=$VW VH=$VH stderr=[$($r1.Err.Trim())]")
if (-not $geomOk) {
    Write-Output 'FAILURES: S1 geometry probe failed - full stderr follows.'
    Write-Output $r1.Err
    Kill-StagedRiviv
    Reset-Ini ''
    Write-Output ('SUMMARY smoke156 pass=' + $script:pass + ' fail=' + $script:fail)
    Write-Output 'SMOKE156 RESULT: FAIL'
    exit 1
}
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2: the AC arm live anchor. wide.png (VW x VH, iCCP = riviv's own P3
# destination profile) MUST take the AC arm on this ACM+HDR machine: the
# output-surface breadcrumb, the p3_to_scrgb word, the AC-face dump line,
# gen 2, and the half-exact oracle over the raw f16 dump.
# ---------------------------------------------------------------------------
$Wide = Join-Path $Stage 'wide.png'
[S156]::WriteWidePng($Wide, $VW, $VH, [IO.File]::ReadAllBytes($P3Icc))
Check 'S2a wide.png written (VW x VH with iCCP)' ((Test-Path $Wide) -and ((Get-Item $Wide).Length -gt 0)) ('size=' + (Get-Item $Wide).Length)

$r2 = Run-Dump $Wide 's2.f16' 's2.err' $IniText 'wide'
$err2 = $r2.Err
$surfaceLines = Get-Lines $err2 '^riviv: output-surface=ac-scrgb$'
$stageLines2 = Get-Lines $err2 '^riviv: display-stage=p3_to_scrgb profile=.+ backend=hw ac=\S+$'
$faceLines = Get-Lines $err2 '^riviv: dump-viewport face=ac-scrgb w=(\d+) h=(\d+) ac-draw=([0-9.]+)ms$'
$genLines2 = Get-Lines $err2 '^riviv: dump-viewport output_gen=2$'
$acDraw = -1.0
$faceWH = ''
if ($faceLines.Count -ge 1) {
    if ($faceLines[0] -match '^riviv: dump-viewport face=ac-scrgb w=(\d+) h=(\d+) ac-draw=([0-9.]+)ms$') {
        $faceWH = ($Matches[1] + 'x' + $Matches[2])
        $acDraw = [double]$Matches[3]
    }
}
$f16Bytes = 0
if (Test-Path $r2.Out) { $f16Bytes = (Get-Item $r2.Out).Length }
Check 'S2b AC dump: exit 0, adopted' (($r2.Code -eq 0) -and $r2.Adopted) ("exit=$($r2.Code) adopted=$($r2.Adopted) stderr=[$($err2.Trim())]")
Check 'S2c output-surface=ac-scrgb appears EXACTLY once (legacy face creates print nothing)' ($surfaceLines.Count -eq 1) ("count=$($surfaceLines.Count) lines=[$($surfaceLines -join ' | ')]")
Check 'S2d display-stage=p3_to_scrgb breadcrumb line present (wide-content AC arm word)' ($stageLines2.Count -ge 1) ("count=$($stageLines2.Count) lines=[$($stageLines2 -join ' | ')]")
Check 'S2e AC dump line: face=ac-scrgb w=VW h=VH, ac-draw <= 8.0ms' (($faceLines.Count -ge 1) -and ($faceWH -ceq ($VW.ToString() + 'x' + $VH.ToString())) -and ($acDraw -ge 0.0) -and ($acDraw -le 8.0)) ("count=$($faceLines.Count) wh=$faceWH acDraw=$acDraw line=[$($faceLines -join ' | ')]")
Check 'S2f output_gen=2 (startup establishment + the wide face transfer)' ($genLines2.Count -ge 1) ("count=$($genLines2.Count) lines=[$($genLines2 -join ' | ')]")
Check 'S2g s2.f16 exists with VW*VH*8 bytes (raw LE u16 RGBA quadruples)' (($f16Bytes -eq ([long]$VW * $VH * 8))) ("bytes=$f16Bytes want=" + ([long]$VW * $VH * 8))

$env:RIVIV_S156_DUMP = $r2.Out
$env:RIVIV_S156_PNG = $Wide
$c2 = Run-CargoTest 'smoke156_half_oracle' 'cargo-s2.txt'
Remove-Item Env:\RIVIV_S156_DUMP -ErrorAction SilentlyContinue
Remove-Item Env:\RIVIV_S156_PNG -ErrorAction SilentlyContinue
$oracleOk = ($c2.Code -eq 0) -and ($c2.Out -match 'test result: ok') -and ($c2.Out -match '1 passed') -and ($c2.Out -match '0 failed') -and ($c2.Out -cnotmatch 'FAILED')
Check 'S2h half oracle (cargo test smoke156_half_oracle): PASS with exactly 1 filtered-in test' $oracleOk ("exit=$($c2.Code) out=[$($c2.Out.Trim())]")
if (-not $oracleOk) {
    Write-Output 'FAILURES: S2h oracle output follows.'
    Write-Output $c2.Out
}
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the WARP control. WARP never runs the AC arm: no output-surface
# breadcrumb, no AC dump line, the dump stays a legacy PNG.
# ---------------------------------------------------------------------------
$WarpIni = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nicm=1`r`nrenderer=warp`r`nshow_menu=0`r`n"
$r3 = Run-Dump $Wide 's3.png' 's3.err' $WarpIni 'wide'
$err3 = $r3.Err
$surfaceLines3 = Get-Lines $err3 'output-surface='
$faceLines3 = Get-Lines $err3 'face=ac-scrgb'
$stageLines3 = Get-Lines $err3 '^riviv: display-stage=\S+ profile=.* backend=\S+ ac=\S+$'
$dims3 = Png-Dims $r3.Out
Check 'S3a warp control: exit 0, adopted, dump PNG exists' (($r3.Code -eq 0) -and $r3.Adopted -and (Test-Path $r3.Out)) ("exit=$($r3.Code) adopted=$($r3.Adopted) dump=$(Test-Path $r3.Out) stderr=[$($err3.Trim())]")
Check 'S3b warp stderr has NO output-surface= and NO face=ac-scrgb line' (($surfaceLines3.Count -eq 0) -and ($faceLines3.Count -eq 0)) ("surface=$($surfaceLines3.Count) face=$($faceLines3.Count)")
Check 'S3c warp display-stage line present (word is machine-dependent on warp)' ($stageLines3.Count -ge 1) ("lines=[$($stageLines3 -join ' | ')]")
Check 'S3d warp dump is a valid viewport-sized PNG' (($dims3[0] -eq $VW) -and ($dims3[1] -eq $VH)) ("dims=$($dims3[0])x$($dims3[1]) want=${VW}x${VH}")
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: the narrow zero-disturbance arm. probe.png on hardware: the ACM
# machine's narrow content runs gpu_effect on the legacy face - the
# #156 additions must add NOTHING to this session's stderr.
# ---------------------------------------------------------------------------
$r4 = Run-Dump $Probe 's4.png' 's4.err' $IniText 'probe'
$err4 = $r4.Err
$surfaceLines4 = Get-Lines $err4 'output-surface='
$faceLines4 = Get-Lines $err4 'face=ac-scrgb'
$stageLines4 = Get-Lines $err4 '^riviv: display-stage=\S+ profile=.* backend=\S+ ac=\S+$'
$genLines4 = Get-Lines $err4 '^riviv: dump-viewport output_gen=\d+$'
$dims4 = Png-Dims $r4.Out
Check 'S4a narrow hw: exit 0, adopted, dump PNG exists' (($r4.Code -eq 0) -and $r4.Adopted -and (Test-Path $r4.Out)) ("exit=$($r4.Code) adopted=$($r4.Adopted) dump=$(Test-Path $r4.Out) stderr=[$($err4.Trim())]")
Check 'S4b narrow stderr has NO output-surface= and NO face=ac-scrgb line' (($surfaceLines4.Count -eq 0) -and ($faceLines4.Count -eq 0)) ("surface=$($surfaceLines4.Count) face=$($faceLines4.Count)")
Check 'S4c narrow display-stage line present' ($stageLines4.Count -ge 1) ("lines=[$($stageLines4 -join ' | ')]")
Check 'S4d narrow output_gen line present' ($genLines4.Count -ge 1) ("lines=[$($genLines4 -join ' | ')]")
Check 'S4e narrow dump is a valid PNG' (($dims4[0] -eq $VW) -and ($dims4[1] -eq $VH)) ("dims=$($dims4[0])x$($dims4[1]) want=${VW}x${VH}")
Kill-StagedRiviv

# ---------------------------------------------------------------------------
# S5 + teardown. The ini removal is UNCONDITIONAL (a leftover ini fakes
# cross-build regressions); a leftover staged riviv process is a failure,
# not something to sweep silently. On failure the rest of the stage is
# KEPT (stderr captures + dumps are the evidence).
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe })
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S5 teardown: stage ini removed, no staged riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')

Write-Output ('SUMMARY smoke156 pass=' + $script:pass + ' fail=' + $script:fail)
if ($script:fail -gt 0) {
    Write-Output ('FAILURES: per-scenario stderr follows.')
    foreach ($e in @(@('S1', (Read-Err 's1.err')), @('S2', (Read-Err 's2.err')), @('S3', (Read-Err 's3.err')), @('S4', (Read-Err 's4.err')))) {
        Write-Output ('--- scenario ' + $e[0] + ' stderr ---')
        Write-Output $e[1]
    }
    Write-Output 'SMOKE156 RESULT: FAIL'
    exit 1
}
Write-Output 'SMOKE156 RESULT: PASS'
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
exit 0
