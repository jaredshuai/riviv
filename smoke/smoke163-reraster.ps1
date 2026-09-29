# smoke163 - #163 SVG interactive re-raster: zooming past 2x swaps in a
# background-rasterized face at the display size with a pixel-exact 1:1
# view adoption (ADR 0005 D6). ASCII-only source (PS 5.1 trap, #79).
# Harness skeleton cribbed from smoke152: staged exe under
# %TEMP%\riviv-163-smoke, pinned ini (fixed geometry, no auto-zoom,
# letterbox (8,8,8)), -dump-viewport as the occlusion-immune channel,
# Add-Type probe, path-targeted Kill-Riviv (#109 contract).
#
# Discriminator (run 1's evidence inverted the naive guess): the fixture
# is a 48x48 SVG of 1-unit BLACK/WHITE vertical stripes, and this host's
# default magnify filter POINT-samples - an upscaled raster renders as
# pure hard stripes, ZERO gray. The #163 swap re-rasterizes AT the face
# through resvg/tiny-skia, whose ANTIALIASED stripe edges leave a real
# gray band. So the dump's gray-band census inside the image rect IS the
# "re-raster happened" observable: branch-with-swap -> gray present;
# master-without-swap -> gray ~0. (The on-screen rect itself is zoom-
# driven either way and proves nothing. The bbox additionally merges the
# outer BLACK stripes into the letterbox pin - |0-8| sits inside the
# tolerance - shaving the measured width by one outer stripe per side;
# the width windows below span that.)
#
# Wheel zoom from the probe: WM_MOUSEWHEEL (delta +120) posted to the
# riviv_view child (its router forwards to the owner's on_mousewheel,
# window.rs view arm). Posted-message latency is unbounded -> every
# state assertion polls (Wait-Title / generous settle sleeps); the
# final dump at WM_CLOSE renders the settled scene (smoke80/152).
#
# Ladder math (why the notch counts are what they are): the preset curve
# of a 48px fit is 48 + (768-48)*preset, and needs_reraster fires only
# when the face passes 2x the CURRENT raster. A swap raises the raster
# to the face, so single-level steps (~x1.3) never re-trigger - the
# swaps land exactly at the threshold-crossing levels: L6 (48 ->
# 0.1098 -> 127), L11 (127 -> 0.4007 -> 336), L15 (336 -> 1.0 -> 768).
# S1 therefore fires 6 notches (ends ON the first swap, face 127 crisp);
# S5 fires 15 (walks all three swaps, face 768 crisp); S4 rounds L6 back
# down to fit. The master exe never swaps - its 48px raster upscales to
# 127 as filtered mush (the S2 negative control).
#
# Scenarios:
#   S0  staged exe + fixture self-checks
#   S1  SVG 6 notches -> dump: rect ~127x127, AA gray band PRESENT
#   S2  master A/B: same steps on the parity exe -> hard stripes, NO
#       gray (proves the S1 verdict is the NEW behavior)
#   S3  PNG A/B: 48x48 striped PNG zoomed 6 -> branch dump == master dump
#       byte-identical (zero behavior change for bitmaps)
#   S4  6 up then 6 wheel-downs -> back at the SWAPPED face's own L0 fit
#       (~106-127 band, the raster IS the image now), AA intact, exit 0
#   S5  race: all 15 notches back-to-back, no inter-settle -> clean exit
#       0, ladder climbed (>=420 wide); the interleaving ends in either a
#       swapped final face or a display at/below exactly 2x the raster
#       (the D6 debounce threshold - both legal, probe-evidenced)
#   S6  close immediately after the notches (raster may still be in
#       flight) -> clean exit 0 (no teardown hang)
#   S7  huge viewBox (100000) + 9 notches -> exit 0, red/blue halves
#       present (clamps hold on the interactive path too; its swaps land
#       at L5/L9 of the ~660 fit)
#   S8  (#164 Codex P1) keep_zoom carry: 6 notches on a 48px striped
#       PNG (bitmaps never re-raster - S3 - so the slot stays virgin)
#       then NavNext onto a 2048px SVG whose load raster is the 1024
#       intermediate: the carried level 6 displays it at ~2.5x its
#       raster -> the view-edge tail re-rasters (AA gray band present,
#       full-bleed dump). -OnlyS8 skips S1-S7 (the control-exe negative
#       run: S8 must FAIL there - master never re-considers on an
#       adoption edge).

param(
    [string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe',
    [string]$MasterExe = "$env:TEMP\riviv-163-parity\riviv.exe",
    [switch]$OnlyS8
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
public class S163 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rc);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    public struct RECT { public int L; public int T; public int R; public int B; }
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
    public static int[] BBoxNotLetterbox(byte[] b, int w, int h) {
        int l = -1, t = -1, r = -1, bm = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            if (Math.Abs(b[i] - 8) > 12 || Math.Abs(b[i + 1] - 8) > 12 || Math.Abs(b[i + 2] - 8) > 12) {
                if (l < 0 || x < l) l = x;
                if (x > r) r = x;
                if (t < 0 || y < t) t = y;
                if (y > bm) bm = y;
            }
        }
        return new int[] { l, t, r, bm };
    }
    // Stripe census INSIDE the rect (BGRA bytes): extremes = near-black or
    // near-white; mid = clearly gray. The DISCRIMINATOR (inverted from the
    // naive guess, per run 1's evidence): this host's default magnify
    // filter POINT-samples, so an upscaled raster is pure hard stripes
    // (zero gray); the re-rasterized face carries resvg's antialiased
    // stripe edges (a real gray band). Branch-with-swap -> mid high;
    // master-without-swap -> mid ~0. Gray-scale check (R==G==B within 8)
    // excludes the letterbox pin and any chroma shift.
    public static long[] StripeCensus(byte[] b, int w, int h, int[] box) {
        long ext = 0, mid = 0;
        for (int y = box[1]; y <= box[3]; y++) for (int x = box[0]; x <= box[2]; x++) {
            int i = (y * w + x) * 4;
            int R = b[i + 2], G = b[i + 1], B = b[i];
            if (Math.Abs(R - G) > 8 || Math.Abs(G - B) > 8) continue; // chromatic -> not a stripe pixel
            if (R < 60) ext++;
            else if (R > 195) ext++;
            else if (R >= 70 && R <= 190) mid++;
        }
        return new long[] { ext, mid };
    }
    public static long CountNear(byte[] b, int w, int h, int r, int g, int bl, int tol) {
        long n = 0;
        for (int i = 0; i < b.Length; i += 4)
            if (Math.Abs(b[i + 2] - r) <= tol && Math.Abs(b[i + 1] - g) <= tol && Math.Abs(b[i] - bl) <= tol) n++;
        return n;
    }
}
'@) -ReferencedAssemblies @('System.Drawing')

[void][S163]::SetProcessDPIAware()
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
    $ps = @(Get-Process riviv -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $script:RunExe -or $_.Path -eq $script:MasterRunExe })
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}
function Reset-Ini($text) {
    if (Test-Path $script:Ini) { Remove-Item $script:Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($script:Ini, $text) }
}
function Start-Riv($argStr, $errName) {
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
        [void][S163]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S163]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else {
            return -2
        }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}
function Get-ViewChild($main) {
    return [S163]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_view', $null)
}
function Send-WheelIn($main, $count) {
    # One wheel-up notch = one zoom level (wparam high word = +120). The
    # anchor point rides the screen-coord lparam: use the window center.
    $view = Get-ViewChild $main
    if ($view -eq [IntPtr]::Zero) { $view = $main }
    $rc = New-Object S163+RECT
    [void][S163]::GetWindowRect($main, [ref]$rc)
    $cx = $rc.L + [int](($rc.R - $rc.L) / 2)
    $cy = $rc.T + [int](($rc.B - $rc.T) / 2)
    $lp = [IntPtr](($cy -shl 16) -bor ($cx -band 0xFFFF))
    for ($i = 0; $i -lt $count; $i++) {
        [void][S163]::PostMessage($view, 0x020A, [IntPtr]0x00780000, $lp)
        Start-Sleep -Milliseconds 30
    }
}

$WM_CLOSE = 0x0010
$Stage = Join-Path $env:TEMP 'riviv-163-smoke'
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
Check 'S0a staged exe copied' (Test-Path $script:RunExe) 'Copy-Item failed'
if (-not $OnlyS8) {
    Check 'S0b master parity exe present' $haveMaster "MasterExe='$MasterExe' not found"
}

$BaseIni = "[riviv]`r`nx=60`r`ny=60`r`nwide=1000`r`nhigh=700`r`nauto_zoom=0`r`nicm=0`r`nwindowed_background_color_r=8`r`nwindowed_background_color_g=8`r`nwindowed_background_color_b=8`r`n"

# One zoom-and-dump run: pin the ini, launch (optionally -dump-viewport),
# wait for the title, fire $notches wheel-ups, settle, close, return the
# dump facts. $exePath swaps the staged binary for the A/B master run.
function Run-ZoomDump($exePath, $img, $outName, $errName, $notches, $settleMs) {
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
        Start-Sleep -Milliseconds 600
        if ($main -ne [IntPtr]::Zero -and $adopted) {
            Send-WheelIn $main $notches
        }
        Start-Sleep -Milliseconds $settleMs
        $code = Close-Main $p $main
    } finally {
        $script:RunExe = $prevExe
    }
    $facts = @{ Out = $out; Code = $code; Adopted = $adopted; Win = ($main -ne [IntPtr]::Zero) }
    if (Test-Path $out) {
        $q = [Px]::Load($out)
        $box = [Px]::BBoxNotLetterbox($q.B, $q.W, $q.H)
        $facts.Png = $q
        $facts.Box = $box
        if ($box[0] -ge 0) {
            $facts.Rw = $box[2] - $box[0] + 1
            $facts.Rh = $box[3] - $box[1] + 1
            $cen = [Px]::StripeCensus($q.B, $q.W, $q.H, $box)
            $facts.Ext = $cen[0]
            $facts.Mid = $cen[1]
        }
    }
    return $facts
}

# ---------------------------------------------------------------------------
# Fixtures: the 48x48 stripe SVG (1-unit black/white bars), the striped
# PNG twin (A/B for bitmaps), the huge viewBox sample.
# ---------------------------------------------------------------------------
$bars = '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48">'
for ($x = 0; $x -lt 48; $x += 2) {
    $fill = if (($x / 2) % 2 -eq 0) { '#000000' } else { '#FFFFFF' }
    $bars += '<rect x="' + $x + '" y="0" width="1" height="48" fill="' + $fill + '"/>'
    $bars += '<rect x="' + ($x + 1) + '" y="0" width="1" height="48" fill="' + $(if (($x / 2) % 2 -eq 0) { '#FFFFFF' } else { '#000000' }) + '"/>'
}
$bars += '</svg>'
$F = @{}
$F.stripes = Join-Path $Stage 'stripes.svg'
[IO.File]::WriteAllText($F.stripes, $bars)
$F.huge = Join-Path $Stage 'huge.svg'
[IO.File]::WriteAllText($F.huge, '<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000" viewBox="0 0 100000 100000"><rect x="0" y="0" width="50000" height="100000" fill="#CC0000"/><rect x="50000" y="0" width="50000" height="100000" fill="#0000CC"/></svg>')
$F.pngTwin = Join-Path $Stage 'stripes.png'
$pt = New-Object byte[] (48 * 48 * 4)
for ($y = 0; $y -lt 48; $y++) {
    for ($x = 0; $x -lt 48; $x++) {
        $i = ($y * 48 + $x) * 4
        $v = if (($x % 2) -eq 0) { 0 } else { 255 }
        $pt[$i] = $v; $pt[$i + 1] = $v; $pt[$i + 2] = $v; $pt[$i + 3] = 255
    }
}
[Px]::SaveRgba($F.pngTwin, $pt, 48, 48)
# S8's second playlist entry: 2048px natural, WHITE-first 4-unit vertical
# stripes. The load raster is the 1024 INTERMEDIATE (raster_target's
# zoom-headroom arm: fit ~0.32 < intermediate 0.5), at exactly scale 0.5
# - so the 4-unit stripes land on EXACT 2px boundaries: a crisp B/W
# raster (no AA fringe -> the control's point-sampled upscale of it has
# zero gray). The swapped face (~2482, scale ~1.21) re-rasterizes the
# same stripes at fractional boundaries: a real gray band. White-first
# keeps the outer edges outside the letterbox pin.
$F.carry2 = Join-Path $Stage 'carry2.svg'
$c2 = '<svg xmlns="http://www.w3.org/2000/svg" width="2048" height="2048">'
for ($x = 0; $x -lt 2048; $x += 8) {
    $c2 += '<rect x="' + $x + '" y="0" width="4" height="2048" fill="#FFFFFF"/>'
    $c2 += '<rect x="' + ($x + 4) + '" y="0" width="4" height="2048" fill="#000000"/>'
}
$c2 += '</svg>'
[IO.File]::WriteAllText($F.carry2, $c2)
$svgHead = [Text.Encoding]::ASCII.GetString(([IO.File]::ReadAllBytes($F.stripes))[0..4])
Check 'S0c stripes.svg is an svg root' ($svgHead -eq '<svg ') "head=[$svgHead]"
$pngHead = [IO.File]::ReadAllBytes($F.pngTwin)
Check 'S0d stripes.png is a PNG' ($pngHead[0] -eq 0x89 -and $pngHead[1] -eq 0x50) 'not png'

# ---------------------------------------------------------------------------
# S8 (#164 Codex P1): the keep_zoom CARRY re-raster. With keep_zoom=1 a
# NavNext carries the zoom LEVEL onto the next image; a carried view can
# land a new SVG past 2x its load-time raster. The fix runs the #163
# consideration at the view-edge tail (window.rs reraster_request_if_needed),
# so the adopted image immediately queues a background re-raster. The
# pre-fix behavior (the control exe): the carry shows the load raster
# upscaled (point-sampled hard stripes, zero gray) until the next USER
# zoom - an adoption edge re-considers nowhere else.
#
# Why image 1 is a PNG (run-1/run-2 diag evidence, kept as the why):
# with the default fit (anamorphic stretch-to-viewport, per-axis
# no-upscale clamp; this window's viewport measures ~972x484), the
# zoom ladder references the FIT face, and a load raster is NEVER
# smaller than its fit face (raster_target's no-upscale/intermediate
# arms) - so for ANY two SVGs, image 2's carried 2x-crossing level is
# >= image 1's own crossing level. Climbing there puts a request in
# flight on image 1; letting its swap land 1:1 re-bases the ladder and
# the carry lands BELOW 2x (run 1 measured pos 2, display 80), and
# carrying while the reply is in flight re-arms the CONTROL exe too via
# the drain's re-consider: a false PASS. A non-SVG image 1 has no
# reraster story at all (S3: bitmaps untouched), so it can idle at ANY
# level with the slot virgin - the only sound anchor.
#
#   image 1: stripes.png (the 48x48 striped PNG twin). 6 notches ->
#     pos 6, display 127 = 2.65x shown point-sampled, no request ever,
#     slot idle, pos exactly 6, view NOT 1:1.
#   image 2: carry2.svg, 2048px natural -> load raster 1024 (the
#     INTERMEDIATE headroom arm, exact scale 0.5 -> crisp B/W). Carried
#     pos 6 renders it at fit*(1+15*0.1098) = ~2573x~1281 (this
#     viewport) = 2.51x its 1024 width -> only the FIX exe requests
#     there. Both axes exceed the viewport, so BOTH arms dump a
#     full-bleed center crop: the rect asserts "covers the viewport"
#     (>=800x400; the height is the 200%-DPI chrome reality here),
#     never a face size.
#
# The dump channel renders the settled scene at WM_CLOSE (S1-S7's
# pattern - there is no live dump readback to poll; adoption is polled
# via the title, then a fixed settle covers rasterize+kick+drain). The
# census IS the discriminator (S1's): fix = resvg AA gray columns
# (mid > 500); control = point-sampled hard stripes (mid ~0 -> FAIL,
# the expected negative control).
# ---------------------------------------------------------------------------
function Invoke-S8 {
    Reset-Ini ($BaseIni + 'keep_zoom=1' + "`r`n")
    $out8 = Join-Path $Stage 's8-out.png'
    if (Test-Path $out8) { Remove-Item $out8 -Force }
    $p8 = Start-Riv ('"' + $F.pngTwin + '" "' + $F.carry2 + '" -dump-viewport "' + $out8 + '"') 's8.err'
    $main8 = Wait-Main $p8
    $adopt1 = Wait-Title $p8 'stripes' 15000
    Start-Sleep -Milliseconds 600
    $ok8 = $false
    $d8 = "exit=-9 adopt1=$adopt1 win=$($main8 -ne [IntPtr]::Zero)"
    $code8 = -9
    if ($main8 -ne [IntPtr]::Zero -and $adopt1) {
        Send-WheelIn $main8 6
        Start-Sleep -Milliseconds 300
        [void][S163]::PostMessage($main8, 0x0111, [IntPtr]107, [IntPtr]::Zero)  # NavNext
        $adopt2 = Wait-Title $p8 'carry2' 15000
        Start-Sleep -Milliseconds 1500
        $code8 = Close-Main $p8 $main8
        $d8 = "exit=$code8 adopt1=$adopt1 carry2=$adopt2"
        if ($adopt2 -and (Test-Path $out8)) {
            $q8 = [Px]::Load($out8)
            $box8 = [Px]::BBoxNotLetterbox($q8.B, $q8.W, $q8.H)
            if ($box8[0] -ge 0) {
                $w8 = $box8[2] - $box8[0] + 1
                $h8 = $box8[3] - $box8[1] + 1
                $cen8 = [Px]::StripeCensus($q8.B, $q8.W, $q8.H, $box8)
                # The re-rastered ~2573x1281 face covers the viewport and
                # carries resvg's AA stripe edges: mid high. The
                # control's point-sampled 1024 raster: mid ~0 -> FAIL.
                $ok8 = ($code8 -eq 0) -and ($w8 -ge 800) -and ($h8 -ge 400) -and ($cen8[1] -gt 500) -and ($cen8[0] -gt 5000)
                $d8 = "rect=${w8}x${h8} ext=$($cen8[0]) mid=$($cen8[1]) exit=$code8 carry2=$adopt2"
            } else {
                $d8 = "blank dump exit=$code8 carry2=$adopt2"
            }
        }
    }
    Check 'S8 keep_zoom carry past 2x: NavNext face re-rastered (AA gray band present)' $ok8 $d8
}
if ($OnlyS8) {
    Invoke-S8
    Kill-Riviv
    if (Test-Path $script:Ini) { Remove-Item $script:Ini -Force }
    Write-Output ('SUMMARY pass=' + $script:pass + ' fail=' + $script:fail)
    if ($script:fail -gt 0) { exit 1 } else { exit 0 }
}

# ---------------------------------------------------------------------------
# S1: 6 notches on the branch exe. Level 6 of a 48px fit: 48 + 720*0.1098
# = 127 - past 2x48, the swap lands at 127: an AT-face raster whose
# antialiased stripe edges show a GRAY BAND (the point-sampled upscale
# the master produces has none - run 1's evidence). The bbox merges the
# outer black stripes into the letterbox pin (|0-8| within the 12
# tolerance): ~2.6px per side at this scale -> the window spans it.
# ---------------------------------------------------------------------------
$s1 = Run-ZoomDump $script:RunExe $F.stripes 's1-out.png' 's1.err' 6 1500
$s1ok = $false; $s1d = "exit=$($s1.Code) adopted=$($s1.Adopted) win=$($s1.Win)"
if ($s1.Box -ne $null -and $s1.Box[0] -ge 0 -and $s1.Code -eq 0 -and $s1.Adopted) {
    $s1ok = ($s1.Rw -ge 113 -and $s1.Rw -le 135) -and ($s1.Rh -ge 113 -and $s1.Rh -le 135) -and ($s1.Mid -gt 500)
    $s1d = "rect=$($s1.Rw)x$($s1.Rh) ext=$($s1.Ext) mid=$($s1.Mid) exit=$($s1.Code)"
}
Check 'S1 SVG zoomed 6: face ~127, AA gray band present (re-raster landed)' $s1ok $s1d

# ---------------------------------------------------------------------------
# S2: the same steps on the MASTER parity exe -> the 48px raster point-
# sampled to 127: hard stripes, ZERO gray (the fixture's negative
# control; also proves S1's verdict is the NEW behavior).
# ---------------------------------------------------------------------------
$s2ok = $false; $s2d = 'master exe missing'
if ($haveMaster) {
    $s2 = Run-ZoomDump $script:MasterRunExe $F.stripes 's2-out.png' 's2.err' 6 1500
    $s2d = "exit=$($s2.Code) adopted=$($s2.Adopted)"
    if ($s2.Box -ne $null -and $s2.Box[0] -ge 0 -and $s2.Code -eq 0 -and $s2.Adopted) {
        $s2ok = ($s2.Mid -lt 50) -and ($s2.Ext -gt 5000)
        $s2d = "rect=$($s2.Rw)x$($s2.Rh) ext=$($s2.Ext) mid=$($s2.Mid) exit=$($s2.Code)"
    }
}
Check 'S2 master A/B: same zoom, point-sampled hard stripes (no gray)' $s2ok $s2d

# ---------------------------------------------------------------------------
# S3: the PNG twin zoomed 6 on both exes -> byte-identical dumps (zero
# behavior change for bitmap formats; the handle gate is invisible).
# ---------------------------------------------------------------------------
$s3ok = $false; $s3d = 'master exe missing'
if ($haveMaster) {
    $s3a = Run-ZoomDump $script:RunExe $F.pngTwin 's3-branch.png' 's3a.err' 6 1200
    $s3b = Run-ZoomDump $script:MasterRunExe $F.pngTwin 's3-master.png' 's3b.err' 6 1200
    $both = (Test-Path $s3a.Out) -and (Test-Path $s3b.Out)
    $hashEq = $false; $h1s = ''; $h2s = ''
    if ($both) {
        $h1 = (Get-FileHash $s3a.Out -Algorithm SHA256).Hash
        $h2 = (Get-FileHash $s3b.Out -Algorithm SHA256).Hash
        $hashEq = ($h1 -eq $h2)
        $h1s = $h1.Substring(0, 12); $h2s = $h2.Substring(0, 12)
    }
    $s3ok = ($s3a.Code -eq 0) -and ($s3b.Code -eq 0) -and $hashEq
    $s3d = "branchExit=$($s3a.Code) masterExit=$($s3b.Code) hashEq=$hashEq h1=$h1s h2=$h2s"
}
Check 'S3 PNG A/B: zoomed dumps byte-identical (bitmaps untouched)' $s3ok $s3d

# ---------------------------------------------------------------------------
# S4: 6 up (a swap lands somewhere in the 106..127 band - the exact face
# depends on the drop/re-request interleaving) then 6 wheel-downs. The
# ladder's L0 is the fit OF THE CURRENT RASTER: the downs walk back to
# L0-of-the-swapped-face (NOT back to 48 - the swapped raster IS the
# image now), and the AA band survives (probe evidence across both
# builds: 104x106 gray=1378 and 125x127 gray=1524).
# ---------------------------------------------------------------------------
Reset-Ini $BaseIni
$out4 = Join-Path $Stage 's4-out.png'
if (Test-Path $out4) { Remove-Item $out4 -Force }
$p4 = Start-Riv ("`"$($F.stripes)`" -dump-viewport `"$out4`"") 's4.err'
$main4 = Wait-Main $p4
$adopt4 = Wait-Title $p4 'stripes' 15000
Start-Sleep -Milliseconds 600
if ($main4 -ne [IntPtr]::Zero -and $adopt4) {
    Send-WheelIn $main4 6
    Start-Sleep -Milliseconds 1500
    # 6 wheel-DOWN notches: -120 in the signed high word.
    $view4 = Get-ViewChild $main4
    if ($view4 -eq [IntPtr]::Zero) { $view4 = $main4 }
    $rc4 = New-Object S163+RECT
    [void][S163]::GetWindowRect($main4, [ref]$rc4)
    $cx4 = $rc4.L + [int](($rc4.R - $rc4.L) / 2)
    $cy4 = $rc4.T + [int](($rc4.B - $rc4.T) / 2)
    $lp4 = [IntPtr](($cy4 -shl 16) -bor ($cx4 -band 0xFFFF))
    $wpDown = [IntPtr]((-120 -shl 16))
    for ($i = 0; $i -lt 6; $i++) {
        [void][S163]::PostMessage($view4, 0x020A, $wpDown, $lp4)
        Start-Sleep -Milliseconds 30
    }
}
Start-Sleep -Milliseconds 800
$code4 = Close-Main $p4 $main4
$s4ok = $false; $s4d = "exit=$code4 adopted=$adopt4"
if (Test-Path $out4) {
    $q4 = [Px]::Load($out4)
    $box4 = [Px]::BBoxNotLetterbox($q4.B, $q4.W, $q4.H)
    if ($box4[0] -ge 0) {
        $w4 = $box4[2] - $box4[0] + 1; $h4 = $box4[3] - $box4[1] + 1
        $cen4 = [Px]::StripeCensus($q4.B, $q4.W, $q4.H, $box4)
        # The swapped face's own L0 fit, still carrying the AA band: the
        # 1:1 exit through the wheel's bracket search landed cleanly.
        $s4ok = ($code4 -eq 0) -and ($w4 -ge 95 -and $w4 -le 140) -and ($h4 -ge 95 -and $h4 -le 140) -and ($cen4[1] -gt 200)
        $s4d = "rect=${w4}x${h4} ext=$($cen4[0]) mid=$($cen4[1]) exit=$code4"
    } elseif ($code4 -eq 0) {
        $s4ok = $false; $s4d = "blank dump exit=$code4"
    }
}
Check 'S4 round trip: back at the swapped face L0 fit, AA intact, exit 0' $s4ok $s4d

# ---------------------------------------------------------------------------
# S5: race - all 15 notches fired back-to-back (30ms cadence, no settle
# between swaps). The notches outpace the swaps: every in-flight raster
# the ladder passes drops by seq and re-requests; the interleaving (probe
# evidence, both builds) ends in ONE of the two LEGAL outcomes - a swap
# at the final face (AA band present), or a display sitting at/below
# exactly 2x the current raster (the D6 debounce threshold: needs_reraster
# is strictly-greater, so 2x-exact never fires - by design). The RACE
# CONTRACT is: no crash, no hang, clean exit, and a final face somewhere
# up the ladder (>= 420 wide). The swap quality itself is S1's
# deterministic pin; this scenario guards the storm, not the pixel.
# ---------------------------------------------------------------------------
$s5 = Run-ZoomDump $script:RunExe $F.stripes 's5-out.png' 's5.err' 15 2500
$s5ok = $false; $s5d = "exit=$($s5.Code) adopted=$($s5.Adopted)"
if ($s5.Box -ne $null -and $s5.Box[0] -ge 0 -and $s5.Code -eq 0 -and $s5.Adopted) {
    $s5ok = ($s5.Rw -ge 420)
    $s5d = "rect=$($s5.Rw)x$($s5.Rh) ext=$($s5.Ext) mid=$($s5.Mid) exit=$($s5.Code)"
}
Check 'S5 race 15 fast notches: ladder climbed (>=420), clean exit 0' $s5ok $s5d

# ---------------------------------------------------------------------------
# S6: close IMMEDIATELY after the notches - the raster may still be in
# flight (or queued) when the window dies. The teardown must join the
# worker and exit cleanly.
# ---------------------------------------------------------------------------
Reset-Ini $BaseIni
$p6 = Start-Riv ('"' + $F.stripes + '"') 's6.err'
$main6 = Wait-Main $p6
$adopt6 = Wait-Title $p6 'stripes' 15000
Start-Sleep -Milliseconds 600
if ($main6 -ne [IntPtr]::Zero -and $adopt6) {
    Send-WheelIn $main6 6
}
$code6 = Close-Main $p6 $main6
Check 'S6 close during in-flight raster: clean exit 0' (($code6 -eq 0) -and $adopt6) "exit=$code6 adopted=$adopt6"

# ---------------------------------------------------------------------------
# S7: huge viewBox + 9 notches. The load face is the ~660 fit; the
# threshold crossings land at L5 (0.0806 -> 1458) and L9 (0.2465 ->
# 3100) - the interactive target re-rasterizes under the 16384/axis cap
# inside the byte budget. Assert: exit 0, both halves present (the
# clamped/centered pan of a 3100-wide face shows the middle band, red
# left + blue right), rect spanning the viewport.
# ---------------------------------------------------------------------------
$s7 = Run-ZoomDump $script:RunExe $F.huge 's7-out.png' 's7.err' 9 2500
$s7ok = $false; $s7d = "exit=$($s7.Code) adopted=$($s7.Adopted)"
if ($s7.Box -ne $null -and $s7.Box[0] -ge 0 -and $s7.Code -eq 0 -and $s7.Adopted) {
    $red7 = [Px]::CountNear($s7.Png.B, $s7.Png.W, $s7.Png.H, 204, 0, 0, 45)
    $blue7 = [Px]::CountNear($s7.Png.B, $s7.Png.W, $s7.Png.H, 0, 0, 204, 45)
    $s7ok = ($s7.Rw -ge 950) -and ($red7 -gt 100000) -and ($blue7 -gt 0)
    $s7d = "rect=$($s7.Rw)x$($s7.Rh) redPx=$red7 bluePx=$blue7 exit=$($s7.Code)"
}
Check 'S7 huge viewBox zoom: clamped interactive raster, halves, exit 0' $s7ok $s7d

Invoke-S8

Kill-Riviv
if (Test-Path $script:Ini) { Remove-Item $script:Ini -Force }
Write-Output ('SUMMARY pass=' + $script:pass + ' fail=' + $script:fail)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
