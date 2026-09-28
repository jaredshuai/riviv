# smoke152 - #152 SVG decode arm: shape sniffing, resvg rasterization into
# the shared still pipeline, and the s-svg.md attack surface.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Harness
# skeleton cribbed from smoke80/98: staged exe under %TEMP%\riviv-152-smoke,
# poll-based waits, Add-Type C# probe helper, SetProcessDPIAware,
# GetExitCodeProcess on the captured live handle, path-targeted Kill-Riviv
# (#109 contract - the developer's real viewer survives teardown).
#
# Fixtures are plain-text SVG files (WriteAllText) plus two PNGs from the
# C# SaveRgba helper. The windowed background is pinned in the ini to
# (8,8,8) so the letterbox is never confusable with a fixture's white
# canvas; every pixel assertion scans the -dump-viewport PNG (the
# occlusion-immune channel, smoke80's pattern).
#
# Observable surface (#152, ticket item 5 - 5 samples + 3 attack pieces):
#   - sample1 icon (48x48 solid green, no-upscale fit): the image rect in
#     the dump is ~48x48 centered and green
#   - sample2 text (400x100 white canvas + black <text>): dark stroke
#     pixels INSIDE the image rect - an empty fontdb renders text blank
#     (s-svg sample2), so this is the load_system_fonts() pin
#   - sample3 filter (feGaussianBlur on a black rect): mid-gray blur-band
#     pixels strictly between the white canvas and the black core
#   - sample4 extref (checker.png linked by ABSOLUTE href, file exists):
#     not loaded - resources_dir=None isolation; red half renders, zero
#     checker-green pixels in the dump
#   - sample5 hugeviewbox (100000x100000, = attack #3): adopts fast, exit
#     0, red/blue halves at fit - without the raster clamp this was the
#     40 GB zero-init allocation that never finished (s-svg attack table)
#   - attack #1 entity bomb (billion-laughs DTD, small enough that <svg
#     sits inside the 512-byte sniff window so the PARSE rejection is
#     what's exercised) forwarded onto a live display: old display kept,
#     failed title adopted, forwarder exit 0, no dialog
#   - attack #2 missing extref: graceful skip - the rect still renders
#   - svgz deliberately NOT admitted: gzip magic -> undetermined-format
#     user-level failure (blank dump, adopted title, exit 0)
#   - renamed/extensionless .svg still decodes (contents-over-extension)
#   - stdin: pipe decodes SVG through the same sniff gate (own window)
#   - A/B: a static PNG's dump is byte-identical between the branch exe
#     and the master parity exe (the produce() prefix-read + rewind and
#     the DecodeEnv viewport term must be invisible to every non-SVG
#     format)
#
# Scenarios:
#   S0 fixture self-checks (sniff shapes, PNG magic, svgz magic)
#   S1..S5 the five samples via pinned-ini dumps
#   S6 entity bomb over a live display (second-instance forward)
#   S7 missing extref renders
#   S8 svgz excluded (blank dump)
#   S9 renamed .bin decodes
#   S10 stdin: pipe
#   S11 static-PNG A/B vs the master parity exe (byte equality)

param(
    [string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe',
    [string]$MasterExe = "$env:TEMP\riviv-152-parity\riviv.exe"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
public class S152 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
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
    // All scans below read the dump's BGRA byte order. The letterbox is
    // pinned to (8,8,8) in the ini, so "not letterbox" bounds the drawn
    // image rect; tolerance absorbs the renderer's edge interpolation.
    public static int[] BBoxNotLetterbox(byte[] b, int w, int h) {
        int l = -1, t = -1, r = -1, bm = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            if (Math.Abs(b[i] - 8) > 12 || Math.Abs(b[i + 1] - 8) > 12 || Math.Abs(b[i + 2] - 8) > 12) {
                if (l < 0 || x < l) l = x;
                if (x > r) r = x;
                if (t < 0) t = y;
                if (y > bm) bm = y;
            }
        }
        return new int[] { l, t, r, bm };
    }
    public static long CountNear(byte[] b, int w, int h, int r, int g, int bl, int tol) {
        long n = 0;
        for (int i = 0; i < b.Length; i += 4)
            if (Math.Abs(b[i + 2] - r) <= tol && Math.Abs(b[i + 1] - g) <= tol && Math.Abs(b[i] - bl) <= tol) n++;
        return n;
    }
    public static long CountDarkInside(byte[] b, int w, int h, int[] box) {
        long n = 0;
        for (int y = box[1]; y <= box[3]; y++) for (int x = box[0]; x <= box[2]; x++) {
            int i = (y * w + x) * 4;
            if (b[i] < 100 && b[i + 1] < 100 && b[i + 2] < 100) n++;
        }
        return n;
    }
    public static long CountMidGrayInside(byte[] b, int w, int h, int[] box) {
        // A blur-band pixel: R==G==B within 6 and value in [60,200] -
        // strictly between the black core and the white canvas (and far
        // from the letterbox pin), so only a real blur can produce it.
        long n = 0;
        for (int y = box[1]; y <= box[3]; y++) for (int x = box[0]; x <= box[2]; x++) {
            int i = (y * w + x) * 4;
            int R = b[i + 2], G = b[i + 1], B = b[i];
            if (Math.Abs(R - G) <= 6 && Math.Abs(G - B) <= 6 && R >= 60 && R <= 200) n++;
        }
        return n;
    }
    public static string PixelAt(PngData p, int x, int y) {
        int i = (y * p.W + x) * 4;
        return (p.B[i + 2]) + "," + (p.B[i + 1]) + "," + (p.B[i]);
    }
    // Green-DOMINANT count: this host's display stage runs the gpu_effect
    // (an sRGB->AdobeRGB transform, see the s1 stderr breadcrumb), which
    // moves pure green (0,170,0) to ~(95,169,40) in the dump - exact-RGB
    // matching is profile-hostage. Dominance (G high AND clearly above
    // both R and B) survives any moderate gamut transform and rejects the
    // letterbox (16,16,16 post-effect), red and blue fills alike.
    public static long CountGreenDominant(byte[] b, int w, int h) {
        long n = 0;
        for (int i = 0; i < b.Length; i += 4) {
            int R = b[i + 2], G = b[i + 1], B = b[i];
            if (G >= 120 && G >= R + 40 && G >= B + 60) n++;
        }
        return n;
    }
}
'@) -ReferencedAssemblies @('System.Drawing')

[void][S152]::SetProcessDPIAware()
$script:pass = 0
$script:fail = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ('PASS ' + $name) }
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
function Kill-Riviv {
    # #109: path-targeted only - the developer's real viewer survives.
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $script:RunExe -or $_.Path -eq $script:MasterRunExe })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}
function Reset-Ini($text) {
    if (Test-Path $script:Ini) { Remove-Item $script:Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($script:Ini, $text) }
}
function Start-Riv($argStr, $errName) {
    # Raw handle captured while alive (#82 lesson: the -PassThru object
    # loses .Handle after exit; Close-Main reads the code via
    # GetExitCodeProcess on that handle).
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    if ($argStr -eq '') {
        $script:RawP = Start-Process -FilePath $script:RunExe -PassThru -RedirectStandardError $errPath
    } else {
        $script:RawP = Start-Process -FilePath $script:RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
    }
    $script:RawH = $script:RawP.Handle
    return $script:RawP
}
function Read-Err($errName) {
    $errPath = Join-Path $Stage $errName
    for ($i = 0; $i -lt 10; $i++) {
        try {
            if (Test-Path $errPath) { return (Get-Content $errPath -Raw) }
            return ''
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
function Close-Main($p, $main) {
    if ($main -ne [IntPtr]::Zero) {
        [void][S152]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S152]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else {
            return -2
        }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}

$WM_CLOSE = 0x0010
$Stage = Join-Path $env:TEMP 'riviv-152-smoke'
$script:Ini = Join-Path $Stage 'riviv.ini'
$script:RunExe = Join-Path $Stage 'riviv.exe'
$script:MasterRunExe = Join-Path $Stage 'master-riviv.exe'
Kill-Riviv
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
Copy-Item $Exe $script:RunExe -Force
$haveMaster = $false
if ($MasterExe -ne '' -and (Test-Path $MasterExe)) {
    Copy-Item $MasterExe $script:MasterRunExe -Force
    $haveMaster = $true
}
Check 'S0 staged exe copied' (Test-Path $script:RunExe) 'Copy-Item failed'

# The pinned base: fixed geometry, no auto-zoom, no ICM, letterbox (8,8,8).
$BaseIni = "[riviv]`r`nx=60`r`ny=60`r`nwide=1000`r`nhigh=700`r`nauto_zoom=0`r`nicm=0`r`nwindowed_background_color_r=8`r`nwindowed_background_color_g=8`r`nwindowed_background_color_b=8`r`n"

# One dump run: pin the ini, launch with -dump-viewport, wait for title
# adoption, close, hand back exit code + dump path. $exePath swaps the
# staged binary for the A/B master run; everything else is identical.
function Run-Dump($exePath, $img, $outName, $errName) {
    Reset-Ini $BaseIni
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    $prevExe = $script:RunExe
    $script:RunExe = $exePath
    try {
        $p = Start-Riv ("`"$img`" -dump-viewport `"$out`"") $errName
        $main = Wait-Main $p
        $leaf = [IO.Path]::GetFileNameWithoutExtension($img)
        $adopted = Wait-Title $p $leaf 15000
        Start-Sleep -Milliseconds 700
        $code = Close-Main $p $main
    } finally {
        $script:RunExe = $prevExe
    }
    return @{ Out = $out; Code = $code; Adopted = $adopted }
}

# ---------------------------------------------------------------------------
# S0: fixtures. SVG bodies are plain text; the two PNGs come from
# Px::SaveRgba; the svgz is a real gzip of icon.svg.
# ---------------------------------------------------------------------------
$F = @{}
$F.icon = Join-Path $Stage 'icon.svg'
[IO.File]::WriteAllText($F.icon, '<?xml version="1.0" encoding="UTF-8"?>' + "`r`n" + '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="#00AA00"/></svg>' + "`r`n")
$F.text = Join-Path $Stage 'text.svg'
[IO.File]::WriteAllText($F.text, '<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100"><rect width="400" height="100" fill="#FFFFFF"/><text x="200" y="66" font-family="Arial" font-size="40" text-anchor="middle" fill="#000000">SVG TEXT 123</text></svg>')
$F.filter = Join-Path $Stage 'filter.svg'
[IO.File]::WriteAllText($F.filter, '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200"><rect width="200" height="200" fill="#FFFFFF"/><defs><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="6"/></filter></defs><rect x="60" y="60" width="80" height="80" fill="#000000" filter="url(#b)"/></svg>')
# extref: the checker is linked by ABSOLUTE href and the file EXISTS - a
# broken isolation (resources_dir set) would load it; the pin demands zero.
$checker = Join-Path $Stage 'checker.png'
$ck = New-Object byte[] (200 * 100 * 4)
for ($y = 0; $y -lt 100; $y++) {
    for ($x = 0; $x -lt 200; $x++) {
        $i = ($y * 200 + $x) * 4
        if (((($x / 25) + ($y / 25)) % 2) -eq 0) { $ck[$i] = 0; $ck[$i + 1] = 160; $ck[$i + 2] = 0 }
        else { $ck[$i] = 200; $ck[$i + 1] = 0; $ck[$i + 2] = 0 }
    }
}
[Px]::SaveRgba($checker, $ck, 200, 100)
$F.extref = Join-Path $Stage 'extref.svg'
$absChecker = $checker.Replace('\', '/')
[IO.File]::WriteAllText($F.extref, '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200"><rect width="200" height="200" fill="#FFFFFF"/><rect width="200" height="100" fill="#CC0000"/><image x="0" y="100" width="200" height="100" xlink:href="file:///' + $absChecker + '"/></svg>')
$F.huge = Join-Path $Stage 'huge.svg'
[IO.File]::WriteAllText($F.huge, '<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000" viewBox="0 0 100000 100000"><rect x="0" y="0" width="50000" height="100000" fill="#CC0000"/><rect x="50000" y="0" width="50000" height="100000" fill="#0000CC"/></svg>')
# Entity bomb: compact enough that the <svg> root sits INSIDE the 512-byte
# sniff window (the parse rejection is what this exercises, not the sniff
# fallback). c expands to 400 references of a - past roxmltree's
# 255-per-entity loop detector.
$bomb = '<?xml version="1.0"?>' + "`r`n" + '<!DOCTYPE svg [' + "`r`n"
$bomb += '<!ENTITY a "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">' + "`r`n"
$bomb += '<!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">' + "`r`n"
$bomb += '<!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">' + "`r`n"
$bomb += ']>' + "`r`n"
$bomb += '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text x="1" y="8" font-size="6">&c;</text></svg>'
$F.bomb = Join-Path $Stage 'bomb.svg'
[IO.File]::WriteAllText($F.bomb, $bomb)
$F.missing = Join-Path $Stage 'missing.svg'
[IO.File]::WriteAllText($F.missing, '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="200" height="200"><rect width="200" height="100" fill="#CC8800"/><image x="0" y="100" width="200" height="100" xlink:href="no-such-file.png"/></svg>')
$F.svgz = Join-Path $Stage 'icon.svgz'
$rawSvg = [IO.File]::ReadAllBytes($F.icon)
$ms = New-Object System.IO.MemoryStream
$gz = New-Object System.IO.Compression.GZipStream($ms, [System.IO.Compression.CompressionLevel]::Fastest)
$gz.Write($rawSvg, 0, $rawSvg.Length); $gz.Close()
[IO.File]::WriteAllBytes($F.svgz, $ms.ToArray())
$F.renamed = Join-Path $Stage 'icon.bin'
Copy-Item $F.icon $F.renamed -Force
$abPng = Join-Path $Stage 'ab-static.png'
$absrc = New-Object byte[] (64 * 64 * 4)
for ($i = 0; $i -lt 64 * 64; $i++) {
    $absrc[$i * 4] = 200; $absrc[$i * 4 + 1] = 60; $absrc[$i * 4 + 2] = 30; $absrc[$i * 4 + 3] = 255
}
[Px]::SaveRgba($abPng, $absrc, 64, 64)

# S0 self-checks: the shapes the sniff must admit / refuse.
$iconHead = [Text.Encoding]::ASCII.GetString(([IO.File]::ReadAllBytes($F.icon))[0..4])
Check 'S0a icon.svg head carries the xml prolog' ($iconHead -eq '<?xml') "head=[$iconHead]"
$svgzHead = [IO.File]::ReadAllBytes($F.svgz)
Check 'S0b icon.svgz head is gzip magic (the excluded shape)' ($svgzHead[0] -eq 0x1F -and $svgzHead[1] -eq 0x8B) 'not gzip'
$pngHead = [IO.File]::ReadAllBytes($checker)
Check 'S0c checker.png is a PNG' ($pngHead[0] -eq 0x89 -and $pngHead[1] -eq 0x50) 'not png'
$bombBytes = [IO.File]::ReadAllBytes($F.bomb)
$bombHead = [Text.Encoding]::ASCII.GetString($bombBytes[0..80])
$svgPos = [Text.Encoding]::ASCII.GetString($bombBytes).IndexOf('<svg')
Check 'S0d bomb head is doctype prelude; <svg inside the sniff window' ($bombHead.StartsWith('<?xml') -and $bombHead.Contains('<!DOCTYPE svg') -and ($svgPos -ge 0) -and ($svgPos -lt 512)) "svgPos=$svgPos len=$($bombBytes.Length)"

# ---------------------------------------------------------------------------
# S1..S5: the five samples, each a pinned-ini dump run.
# ---------------------------------------------------------------------------
function Dump-Scan($r) {
    if (-not (Test-Path $r.Out)) { return $null }
    $q = [Px]::Load($r.Out)
    $box = [Px]::BBoxNotLetterbox($q.B, $q.W, $q.H)
    return @{ Png = $q; Box = $box }
}

# S1 icon: 48x48 green at 1:1 (fit never upscales), centered.
$r1 = Run-Dump $script:RunExe $F.icon 's1-out.png' 's1.err'
$s1 = Dump-Scan $r1
$s1ok = $false; $s1d = "exit=$($r1.Code) adopted=$($r1.Adopted)"
if ($null -ne $s1 -and $r1.Code -eq 0 -and $r1.Adopted -and $s1.Box[0] -ge 0) {
    $w1 = $s1.Box[2] - $s1.Box[0] + 1; $h1 = $s1.Box[3] - $s1.Box[1] + 1
    $green = [Px]::CountGreenDominant($s1.Png.B, $s1.Png.W, $s1.Png.H)
    $s1ok = ($w1 -ge 42 -and $w1 -le 54) -and ($h1 -ge 42 -and $h1 -le 54) -and ($green -gt 1500)
    $s1d = "rect=${w1}x${h1} greenPx=$green exit=$($r1.Code)"
}
Check 'S1 icon decodes: ~48x48 green rect at 1:1, exit 0' $s1ok $s1d

# S2 text: dark strokes inside the 400x100 white canvas (fontdb pin).
$r2 = Run-Dump $script:RunExe $F.text 's2-out.png' 's2.err'
$s2 = Dump-Scan $r2
$s2ok = $false; $s2d = "exit=$($r2.Code) adopted=$($r2.Adopted)"
if ($null -ne $s2 -and $r2.Code -eq 0 -and $r2.Adopted -and $s2.Box[0] -ge 0) {
    $w2 = $s2.Box[2] - $s2.Box[0] + 1; $h2 = $s2.Box[3] - $s2.Box[1] + 1
    $dark = [Px]::CountDarkInside($s2.Png.B, $s2.Png.W, $s2.Png.H, $s2.Box)
    $s2ok = ($w2 -ge 390 -and $w2 -le 410) -and ($h2 -ge 95 -and $h2 -le 105) -and ($dark -ge 300)
    $s2d = "rect=${w2}x${h2} darkPx=$dark exit=$($r2.Code)"
}
Check 'S2 text renders strokes (load_system_fonts pin), exit 0' $s2ok $s2d

# S3 filter: a blur band strictly between white canvas and black core.
$r3 = Run-Dump $script:RunExe $F.filter 's3-out.png' 's3.err'
$s3 = Dump-Scan $r3
$s3ok = $false; $s3d = "exit=$($r3.Code) adopted=$($r3.Adopted)"
if ($null -ne $s3 -and $r3.Code -eq 0 -and $r3.Adopted -and $s3.Box[0] -ge 0) {
    $mid = [Px]::CountMidGrayInside($s3.Png.B, $s3.Png.W, $s3.Png.H, $s3.Box)
    $s3ok = ($mid -ge 400)
    $s3d = "midGrayPx=$mid exit=$($r3.Code)"
}
Check 'S3 filter blurs (mid-gray band present), exit 0' $s3ok $s3d

# S4 extref isolation: the existing checker (absolute href) NOT loaded.
$r4 = Run-Dump $script:RunExe $F.extref 's4-out.png' 's4.err'
$s4 = Dump-Scan $r4
$s4ok = $false; $s4d = "exit=$($r4.Code) adopted=$($r4.Adopted)"
if ($null -ne $s4 -and $r4.Code -eq 0 -and $r4.Adopted -and $s4.Box[0] -ge 0) {
    $greenCk = [Px]::CountGreenDominant($s4.Png.B, $s4.Png.W, $s4.Png.H)
    $redTop = [Px]::CountNear($s4.Png.B, $s4.Png.W, $s4.Png.H, 204, 0, 0, 45)
    $s4ok = ($greenCk -eq 0) -and ($redTop -ge 5000)
    $s4d = "checkerGreenPx=$greenCk redPx=$redTop exit=$($r4.Code)"
}
Check 'S4 extref isolated: zero checker pixels, red half renders, exit 0' $s4ok $s4d

# S5 huge viewBox (attack #3): fast adopt + fit halves = the clamp held.
$r5 = Run-Dump $script:RunExe $F.huge 's5-out.png' 's5.err'
$s5 = Dump-Scan $r5
$s5ok = $false; $s5d = "exit=$($r5.Code) adopted=$($r5.Adopted)"
if ($null -ne $s5 -and $r5.Code -eq 0 -and $r5.Adopted -and $s5.Box[0] -ge 0) {
    $w5 = $s5.Box[2] - $s5.Box[0] + 1; $h5 = $s5.Box[3] - $s5.Box[1] + 1
    $lred = [Px]::CountNear($s5.Png.B, $s5.Png.W, $s5.Png.H, 204, 0, 0, 45)
    $rblue = [Px]::CountNear($s5.Png.B, $s5.Png.W, $s5.Png.H, 0, 0, 204, 45)
    $s5ok = ($w5 -ge 400) -and ($lred -gt 50000) -and ($rblue -gt 50000)
    $s5d = "rect=${w5}x${h5} redPx=$lred bluePx=$rblue exit=$($r5.Code)"
}
Check 'S5 huge viewBox (attack 3): clamped fit, red/blue halves, exit 0' $s5ok $s5d

# ---------------------------------------------------------------------------
# S6: entity bomb (attack #1) forwarded onto a live display. The first
# instance shows icon.svg WITH -dump-viewport; a second instance opens
# bomb.svg (single-instance handoff, exits 0); the FIRST window must keep
# the old display, adopt the failed title, and still dump green at close.
# ---------------------------------------------------------------------------
Reset-Ini $BaseIni
$out6 = Join-Path $Stage 's6-out.png'
if (Test-Path $out6) { Remove-Item $out6 -Force }
$p6 = Start-Riv ("`"$($F.icon)`" -dump-viewport `"$out6`"") 's6-a.err'
$main6 = Wait-Main $p6
$adopt6a = Wait-Title $p6 'icon' 15000
Start-Sleep -Milliseconds 800
$p6b = Start-Process -FilePath $script:RunExe -ArgumentList ('"' + $F.bomb + '"') -PassThru
# Handle captured while alive (#82): .ExitCode is unreadable after exit.
$h6b = [IntPtr]::Zero
if (-not $p6b.HasExited) { $h6b = $p6b.Handle }
$p6b.WaitForExit(15000) | Out-Null
$bombExit = -1
if ($h6b -ne [IntPtr]::Zero) {
    $c6b = 0
    [void][S152]::GetExitCodeProcess($h6b, [ref]$c6b)
    $bombExit = $c6b
}
$adopt6b = Wait-Title $p6 'bomb' 15000
Start-Sleep -Milliseconds 800
$code6 = Close-Main $p6 $main6
$s6ok = $false; $s6d = "adoptA=$adopt6a adoptBomb=$adopt6b fwdExit=$bombExit mainExit=$code6 win=$($main6 -ne [IntPtr]::Zero)"
if (Test-Path $out6) {
    $q6 = [Px]::Load($out6)
    $green6 = [Px]::CountGreenDominant($q6.B, $q6.W, $q6.H)
    $s6ok = ($adopt6a -and $adopt6b -and ($bombExit -eq 0) -and ($code6 -eq 0) -and ($green6 -gt 1500))
    $s6d = "adoptA=$adopt6a adoptBomb=$adopt6b fwdExit=$bombExit mainExit=$code6 greenPx=$green6"
}
Check 'S6 entity bomb: old display kept, failed title adopted, exits 0' $s6ok $s6d

# ---------------------------------------------------------------------------
# S7: missing extref (attack #2) - graceful skip, the rect still renders.
# ---------------------------------------------------------------------------
$r7 = Run-Dump $script:RunExe $F.missing 's7-out.png' 's7.err'
$s7 = Dump-Scan $r7
$s7ok = $false; $s7d = "exit=$($r7.Code) adopted=$($r7.Adopted)"
if ($null -ne $s7 -and $r7.Code -eq 0 -and $r7.Adopted -and $s7.Box[0] -ge 0) {
    $orange = [Px]::CountNear($s7.Png.B, $s7.Png.W, $s7.Png.H, 204, 136, 0, 45)
    $s7ok = ($orange -ge 5000)
    $s7d = "orangePx=$orange exit=$($r7.Code)"
}
Check 'S7 missing extref skipped gracefully, rect renders, exit 0' $s7ok $s7d

# ---------------------------------------------------------------------------
# S8: svgz NOT admitted - blank dump, adopted title, exit 0 (the same
# user-level failure the undetermined formats have always shown).
# ---------------------------------------------------------------------------
$r8 = Run-Dump $script:RunExe $F.svgz 's8-out.png' 's8.err'
$s8 = Dump-Scan $r8
$s8ok = $false; $s8d = "exit=$($r8.Code) adopted=$($r8.Adopted)"
if ($null -ne $s8) {
    $blank = ($s8.Box[0] -lt 0)
    $green8 = [Px]::CountGreenDominant($s8.Png.B, $s8.Png.W, $s8.Png.H)
    $s8ok = ($r8.Code -eq 0) -and $r8.Adopted -and $blank -and ($green8 -lt 100)
    $s8d = "blank=$blank greenPx=$green8 exit=$($r8.Code) adopted=$($r8.Adopted)"
}
Check 'S8 svgz excluded: blank display, user-level failure, exit 0' $s8ok $s8d

# ---------------------------------------------------------------------------
# S9: renamed/extensionless - contents over extension.
# ---------------------------------------------------------------------------
$r9 = Run-Dump $script:RunExe $F.renamed 's9-out.png' 's9.err'
$s9 = Dump-Scan $r9
$s9ok = $false; $s9d = "exit=$($r9.Code) adopted=$($r9.Adopted)"
if ($null -ne $s9 -and $r9.Code -eq 0 -and $r9.Adopted -and $s9.Box[0] -ge 0) {
    $green9 = [Px]::CountGreenDominant($s9.Png.B, $s9.Png.W, $s9.Png.H)
    $s9ok = ($green9 -gt 1500)
    $s9d = "greenPx=$green9 exit=$($r9.Code)"
}
Check 'S9 renamed .bin decodes by contents, exit 0' $s9ok $s9d

# ---------------------------------------------------------------------------
# S10: stdin: pipe - the bytes arm of the sniff gate (own window, #65).
# cmd /c type pumps raw bytes (PS piping would re-encode; #65's recipe).
# The window is targeted by EXE PATH (smoke98 S3's contract), never by
# process name.
# ---------------------------------------------------------------------------
Reset-Ini $BaseIni
$out10 = Join-Path $Stage 's10-out.png'
if (Test-Path $out10) { Remove-Item $out10 -Force }
$cmdLine = '/c type "' + $F.icon + '" | "' + $script:RunExe + '" stdin: -dump-viewport "' + $out10 + '"'
$null = Start-Process -FilePath 'cmd.exe' -ArgumentList $cmdLine -PassThru -WindowStyle Hidden
$null = Wait-Until { $script:r10 = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $script:RunExe }); ($script:r10.Count -eq 1) } 15000
$riv10 = $null
if ($script:r10 -ne $null -and $script:r10.Count -eq 1) { $riv10 = $script:r10[0] }
$main10 = [IntPtr]::Zero
if ($riv10 -ne $null) { $main10 = Wait-Main $riv10 }
$title10 = $false
if ($riv10 -ne $null) { $title10 = Wait-Title $riv10 'stdin' 12000 }
$h10 = [IntPtr]::Zero
if ($riv10 -ne $null -and -not $riv10.HasExited) { $h10 = $riv10.Handle }
$code10 = -9
if ($main10 -ne [IntPtr]::Zero) {
    Start-Sleep -Milliseconds 800
    [void][S152]::PostMessage($main10, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
}
if ($riv10 -ne $null -and $riv10.WaitForExit(12000) -and ($h10 -ne [IntPtr]::Zero)) {
    $c10 = 0
    [void][S152]::GetExitCodeProcess($h10, [ref]$c10)
    $code10 = $c10
} elseif ($riv10 -ne $null) {
    Stop-Process -Id $riv10.Id -Force -ErrorAction SilentlyContinue
    $code10 = -1
}
$green10 = -1
if (Test-Path $out10) {
    $q10 = [Px]::Load($out10)
    $green10 = [Px]::CountGreenDominant($q10.B, $q10.W, $q10.H)
}
Check 'S10 stdin: pipe decodes SVG (green icon, exit 0)' (($code10 -eq 0) -and ($green10 -gt 1500)) ("exit=$code10 win=$($main10 -ne [IntPtr]::Zero) title=$title10 greenPx=$green10")
Kill-Riviv

# ---------------------------------------------------------------------------
# S11: A/B - a static PNG's dump must be byte-identical between the branch
# exe and the master parity exe (the produce() prefix-read + rewind and
# the DecodeEnv viewport term must be invisible to every non-SVG format).
# ---------------------------------------------------------------------------
if ($haveMaster) {
    $ra = Run-Dump $script:RunExe $abPng 's11-branch.png' 's11a.err'
    $rb = Run-Dump $script:MasterRunExe $abPng 's11-master.png' 's11b.err'
    $bothPng = (Test-Path $ra.Out) -and (Test-Path $rb.Out)
    $hashEq = $false
    $h1 = ''; $h2 = ''
    if ($bothPng) {
        $h1 = (Get-FileHash $ra.Out -Algorithm SHA256).Hash
        $h2 = (Get-FileHash $rb.Out -Algorithm SHA256).Hash
        $hashEq = ($h1 -eq $h2)
    }
    $h1s = ''; $h2s = ''
    if ($h1.Length -ge 12) { $h1s = $h1.Substring(0, 12) }
    if ($h2.Length -ge 12) { $h2s = $h2.Substring(0, 12) }
    Check 'S11 static PNG A/B: branch dump == master parity dump' (($ra.Code -eq 0) -and ($rb.Code -eq 0) -and $hashEq) ("branchExit=$($ra.Code) masterExit=$($rb.Code) hashEq=$hashEq h1=$h1s h2=$h2s")
} else {
    Check 'S11 static PNG A/B: master parity exe present' $false "MasterExe='$MasterExe' not found"
}

Kill-Riviv
if (Test-Path $script:Ini) { Remove-Item $script:Ini -Force }
Write-Output ('SUMMARY pass=' + $script:pass + ' fail=' + $script:fail)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
