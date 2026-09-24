# smoke82 - #82 D2D giant-image tiling (overview + LRU tiles) + VRAM byte
# budget. ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson).
# Harness skeleton cribbed verbatim from smoke80/smoke81: staged exe+ini
# under %TEMP%\riviv-82-smoke, WM_CLOSE-driven -dump-viewport dumps,
# poll-based waits, Add-Type C# probe helper, SetProcessDPIAware,
# GetExitCodeProcess on the captured live handle.
#
# Pixel access is GetPixel-only (this repo's LockBits+Marshal.Copy
# faulted intermittently lesson): byte comparisons go through
# Get-FileHash, small comparisons through GetPixel (measured ~2.3 us per
# read on this machine, so the largest full-frame scan here, the 674x246
# S2b pair, costs under a second). Add-Type -AssemblyName System.Drawing
# precedes the -TypeDefinition block (repo discipline). No function named
# Diff/Compare; functions emit through Write-Host only and return exactly
# one value.
#
# The observable surface (#82): startup breadcrumb
#   riviv: d2d max_bitmap=<n> tile cap=<n> (dxgi budget=<n|unavailable>)
# close-time stats line (only when the giant path ran; 12 fields since #90
# removed the gdi-only display= accounting field)
#   riviv: tiles level=<L> tiles=<n> base=<bytes> gpu=<bytes> peak_gpu=<bytes>
#     inflight=<bytes> peak_inflight=<bytes> uploads=<n> evictions=<n>
#     mip_builds=<n> source=<bytes> cap=<bytes>
# and the -tile <edge> diagnostic (forces the tile grid edge, in DRAWN-LEVEL
# pixels, and bypasses the single-bitmap shortcut). The OLD gate
# ("exceeds the D2D max bitmap ... rendering it via gdi") is gone: asserted
# absent in every giant scenario.
#
# Scenarios:
#   S1 stats channel: -tile 256 on 900x600 in a >=1200x900 view (fit caps
#      at 100%) -> stats level=0 tiles>0 uploads>0; the twin WITHOUT -tile
#      prints no stats line; the d2d breadcrumb regex is present.
#   S2 zero-seam core, tiled vs untiled same image/transform:
#      S2a TRUE 1:1 (WM_COMMAND 45, same calibrated geometry): dump files
#      byte-identical (Get-FileHash), image bbox exactly 900x600, ramp
#      samples within +-2 of the source formula (identity cannot mean
#      identically blank).
#      S2b default fit into 674x246: whole-frame maxDelta <= 1, and the
#      seam scan (max adjacent-column luminance step within +-2 px of each
#      tile boundary; candidate boundaries enumerated for every drawn-level
#      choice that covers the render) shows tiledMax <= untiledMax + 1.
#   S3 hardware giant 40000x256 (renderer=d2d, red band at source x
#      [18000,22000), gray ramp elsewhere): fit -> NO gate line, stats
#      level>=1 tiles=0 base>0 mip_builds>=1, strip shape sane, red band at
#      the expected columns +-10, surrounding row a ramp not black.
#      1:1 (WM_COMMAND 45) -> stats level=0 tiles>0 base=0 uploads>=1, red
#      fills >= 95% of the sampled image rect.
#   S4 WARP giant 16777217x1 (renderer=warp, WARP max = 2^23): NO gate
#      line, breadcrumb regex present, stats mip_builds>=1 tiles=0 base>0,
#      gpu<=cap, strip tracks the fixture's own gradient formula (samples
#      vs lum(f) = 0.299*R + 0.587*(255-R) + 0.114*64, R = trunc(255*f),
#      plus a non-black floor and a per-step smoothness bound; CUBIC
#      ringing is tolerated, strict monotonicity is not demanded). Two
#      record-only evidence runs (warp 1:1, hardware fit) pin whatever
#      the S4d/S4f verdict is.
#   S5 budget bounds + no leak: EVERY stats line captured anywhere in the
#      run satisfies gpu<=cap and peak_gpu<=cap; two more 1:1 banner
#      instances with -tile 256 each show uploads>0 with resident bytes
#      still bounded (not growing with the frame count).
#   S6 RETIRED with the GDI render arm (#90): the renderer=gdi banner
#      record-only run is gone (renderer=gdi now maps to auto with a
#      migration note on stderr). The number stays reserved so S7/S8/S9
#      keep theirs.
#   S2b-d hard-edge probe (R3 P2-1): a flat mid-grey 900x600 with 1-px
#      full-height black columns ON a tile boundary (source x = 256) and
#      MID-TILE (source x = 300): the dip screen positions must hit the
#      expected columns in BOTH dumps and be the SAME in both - the smooth
#      ramp's ~0.38/col step hides a dropped/duplicated boundary column
#      inside the S2b-b/c tolerances; a 1-column fault moves or erases one
#      of these dips.
#   S7 evidence purity (R3 P2-6): no D2D-scenario stderr may contain
#      "trying the gdi channel" (the dump's silent fallback would
#      substitute GDI evidence for D2D evidence), across every captured
#      d2d/warp run; and the S2a/S2b/S2b-d tiled runs assert tiles>0 in
#      the CLOSE stats line - it prints after the close dump re-planned
#      with the same -tile, so it is the dump-side tiling proof.
#   S8 long-animation churn (design section 3) on the stripe family: ONE d2d
#      instance, 16777217x1 giant with -tile 256, 1:1 -> best-fit -> 1:1
#      (WM_COMMAND 45/46/45), then close: uploads>0, evictions>=0, and
#      gpu/peak_gpu stay <= cap after the churn.
#   S9 pressure: -tile 4 on the 900x600 ramp at a fit-capped viewport
#      tiles the whole master in ONE frame: 225x150 = 33750 tiles x
#      (4+2*32)^2x4 B = 18496 B = ~624 MB forced demand vs the 256 MiB
#      cap - the forced plan is admitted, so the LRU must evict while
#      keeping gpu/peak_gpu <= cap: evictions>0. (A gen-churn variant
#      measured evictions=0: generation purges are not capacity evictions.)
#   The S5c budget sweep runs at the END so it covers the stats lines of
#      EVERY scenario, not just S1..S5.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Runtime.InteropServices;
public class S82 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L; public int T; public int R; public int B; }

    // ---- hand-written PNG writers (32bpp RGBA, filter 0 rows, zlib IDAT).
    // GDI+ cannot create bitmaps past 65535, and the banner (40000x256)
    // needs a hand-rolled payload anyway; no SetPixel loops on 10M pixels.

    // 900x600 ramp fixture: r = x*255/(w-1), g = y*255/(h-1), b = 128
    // (constant, so no pixel can equal the magenta letterbox background).
    // Integer division matches the PS-side expected-value math.
    public static void WriteGradRamp(string path, int w, int h) {
        byte[] raw = new byte[(w * 4 + 1) * h];
        for (int y = 0; y < h; y++) {
            raw[y * (w * 4 + 1)] = 0;
            int o = y * (w * 4 + 1) + 1;
            for (int x = 0; x < w; x++) {
                raw[o + x * 4] = (byte)(x * 255 / (w - 1));
                raw[o + x * 4 + 1] = (byte)(y * 255 / (h - 1));
                raw[o + x * 4 + 2] = 128;
                raw[o + x * 4 + 3] = 255;
            }
        }
        PngFromRaw(path, w, h, raw);
    }
    // 40000x256 banner: gray ramp 8..247 EXCEPT a full-height solid red
    // band for x in [band0, band1). Never white (max 247), never black.
    // Every byte including alpha is written (the first run's `continue`
    // left band alpha at 0 - a transparent band composites to the white
    // letterbox and reads as a white hole, which masqueraded as a product
    // bug until the dump profile showed the ramp intact on both sides).
    public static void WriteBanner(string path, int w, int h, int band0, int band1) {
        byte[] raw = new byte[(w * 4 + 1) * h];
        for (int y = 0; y < h; y++) {
            raw[y * (w * 4 + 1)] = 0;
            int o = y * (w * 4 + 1) + 1;
            for (int x = 0; x < w; x++) {
                if (x >= band0 && x < band1) {
                    raw[o + x * 4] = 255; raw[o + x * 4 + 1] = 0; raw[o + x * 4 + 2] = 0; raw[o + x * 4 + 3] = 255;
                } else {
                    byte v = (byte)(8 + (int)(((double)x * 239.0) / (double)(w - 1)));
                    raw[o + x * 4] = v; raw[o + x * 4 + 1] = v; raw[o + x * 4 + 2] = v;
                    raw[o + x * 4 + 3] = 255;
                }
            }
        }
        PngFromRaw(path, w, h, raw);
    }
    // Flat mid-grey field with two 1-px full-height BLACK columns at
    // explicit source columns (the S2b-d hard-edge probe: one column on a
    // -tile 256 grid boundary, one mid-tile). Never white, never magenta.
    public static void WriteHardEdge(string path, int w, int h, int colA, int colB) {
        byte[] raw = new byte[(w * 4 + 1) * h];
        for (int y = 0; y < h; y++) {
            raw[y * (w * 4 + 1)] = 0;
            int o = y * (w * 4 + 1) + 1;
            for (int x = 0; x < w; x++) {
                byte v = (byte)((x == colA || x == colB) ? 0 : 128);
                raw[o + x * 4] = v; raw[o + x * 4 + 1] = v; raw[o + x * 4 + 2] = v;
                raw[o + x * 4 + 3] = 255;
            }
        }
        PngFromRaw(path, w, h, raw);
    }
    // 16777217x1 gradient giant (the #81 writer's payload: r = x*255/(w-1),
    // g = 255-r, b = 64); WARP max_bitmap = 2^23 so width 2^24+1 forces the
    // overview/tile decision.
    public static void WriteWidePngGrad(string path, int w, int h) {
        byte[] raw = new byte[(w * 4 + 1) * h];
        for (int y = 0; y < h; y++) {
            raw[y * (w * 4 + 1)] = 0;
            int o = y * (w * 4 + 1) + 1;
            for (int x = 0; x < w; x++) {
                int r = (int)(((double)x * 255.0) / (double)(w - 1));
                raw[o + x * 4] = (byte)r; raw[o + x * 4 + 1] = (byte)(255 - r);
                raw[o + x * 4 + 2] = 64; raw[o + x * 4 + 3] = 255;
            }
        }
        PngFromRaw(path, w, h, raw);
    }
    static void PngFromRaw(string path, int w, int h, byte[] raw) {
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe(ihdr, 0, w); PutBe(ihdr, 4, h);
        ihdr[8] = 8; ihdr[9] = 6;
        Chunk(ms, "IHDR", ihdr);
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
        Chunk(ms, "IDAT", comp.ToArray());
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }
    static void PutBe(byte[] b, int o, int v) { b[o] = (byte)(v >> 24); b[o + 1] = (byte)(v >> 16); b[o + 2] = (byte)(v >> 8); b[o + 3] = (byte)v; }
    static void Chunk(MemoryStream ms, string type, byte[] data) {
        byte[] len = new byte[4]; PutBe(len, 0, data.Length);
        ms.Write(len, 0, 4);
        byte[] tb = System.Text.Encoding.ASCII.GetBytes(type);
        ms.Write(tb, 0, 4);
        ms.Write(data, 0, data.Length);
        uint c = 0xFFFFFFFF;
        byte[] all = new byte[data.Length + 4];
        tb.CopyTo(all, 0); data.CopyTo(all, 4);
        foreach (byte x in all) { c ^= x; for (int k = 0; k < 8; k++) c = ((c & 1) != 0) ? (0xEDB88320 ^ (c >> 1)) : (c >> 1); }
        c = c ^ 0xFFFFFFFF;
        byte[] crc = new byte[4]; PutBe(crc, 0, unchecked((int)c));
        ms.Write(crc, 0, 4);
    }

    // ---- GetPixel-only probes (no LockBits anywhere in this script).

    // First/last non-background pixel, found with two line scans: the
    // VERTICAL center line first (the fixtures' image rects always cross
    // it - the 1..2 px giant strip spans the full width - so t/b are
    // found even when the rect is too thin for the horizontal line to
    // hit), then the horizontal scan along the rect's middle row. The
    // fixtures render one solid rectangle over a pure background, so the
    // two scans recover its exact rect without a full-frame scan.
    // Returns {l, t, r, b, W, H}.
    public static int[] RectEdges(string path, int br, int bg, int bb) {
        using (Bitmap bmp = new Bitmap(path)) {
            int W = bmp.Width, H = bmp.Height;
            int xm = W / 2;
            int l = -1, r = -1, t = -1, b = -1;
            for (int y = 0; y < H; y++) {
                Color c = bmp.GetPixel(xm, y);
                if (c.R != br || c.G != bg || c.B != bb) { if (t < 0) t = y; b = y; }
            }
            if (t >= 0) {
                int ym = t + (b - t) / 2;
                for (int x = 0; x < W; x++) {
                    Color c = bmp.GetPixel(x, ym);
                    if (c.R != br || c.G != bg || c.B != bb) { if (l < 0) l = x; r = x; }
                }
            }
            return new int[] { l, t, r, b, W, H };
        }
    }
    // RGB triples along one row: {r,g,b} * w, x relative to l.
    public static int[] RowPix(string path, int y, int l, int w) {
        using (Bitmap bmp = new Bitmap(path)) {
            int[] o = new int[w * 3];
            for (int x = 0; x < w; x++) {
                Color c = bmp.GetPixel(l + x, y);
                o[x * 3] = c.R; o[x * 3 + 1] = c.G; o[x * 3 + 2] = c.B;
            }
            return o;
        }
    }
    // Whole-frame two-dump stats: {maxAnyChannelDiff, diffPixels, w, h};
    // max = -1 when the dimensions differ (caller treats that as failure).
    public static int[] PairStats(string pathA, string pathB) {
        using (Bitmap ba = new Bitmap(pathA)) using (Bitmap bb = new Bitmap(pathB)) {
            if (ba.Width != bb.Width || ba.Height != bb.Height)
                return new int[] { -1, -1, ba.Width, ba.Height };
            int mx = 0; long dp = 0;
            for (int y = 0; y < ba.Height; y++) for (int x = 0; x < ba.Width; x++) {
                Color ca = ba.GetPixel(x, y), cb = bb.GetPixel(x, y);
                int d = Math.Max(Math.Abs(ca.R - cb.R), Math.Max(Math.Abs(ca.G - cb.G), Math.Abs(ca.B - cb.B)));
                if (d > mx) mx = d;
                if (d != 0) dp++;
            }
            return new int[] { mx, (int)dp, ba.Width, ba.Height };
        }
    }
    // First/last red column (R-G > 30 and R > 120) along a row, x relative
    // to l; {-1, -1} when no column qualifies.
    public static int[] RedSpan(string path, int y, int l, int w) {
        using (Bitmap bmp = new Bitmap(path)) {
            int first = -1, last = -1;
            for (int x = 0; x < w; x++) {
                Color c = bmp.GetPixel(l + x, y);
                if ((c.R - c.G) > 30 && c.R > 120) { if (first < 0) first = x; last = x; }
            }
            return new int[] { first, last };
        }
    }
    // Grid-sampled red coverage of a rect (step 13 px): {redSamples, total}.
    public static int[] RedFraction(string path, int l, int t, int w, int h) {
        using (Bitmap bmp = new Bitmap(path)) {
            long red = 0, n = 0;
            for (int y = t + 2; y < t + h - 1; y += 13) for (int x = l + 2; x < l + w - 1; x += 13) {
                Color c = bmp.GetPixel(x, y);
                n++;
                if ((c.R - c.G) > 30 && c.R > 120) red++;
            }
            return new int[] { (int)red, (int)n };
        }
    }
    // RGB triples at explicit points: {r,g,b} per (xs[i], ys[i]).
    public static int[] SamplePts(string path, int[] xs, int[] ys) {
        using (Bitmap bmp = new Bitmap(path)) {
            int n = xs.Length;
            int[] o = new int[n * 3];
            for (int i = 0; i < n; i++) {
                Color c = bmp.GetPixel(xs[i], ys[i]);
                o[i * 3] = c.R; o[i * 3 + 1] = c.G; o[i * 3 + 2] = c.B;
            }
            return o;
        }
    }
    // Dark-dip detector for the hard-edge probe: along one row, group the
    // columns whose luminance is below thresh into consecutive runs and
    // return {runCount, argmin(run1), argmin(run2)} with the argmins
    // relative to l (-1 when that run does not exist).
    public static int[] DarkDips(string path, int y, int l, int w, int thresh) {
        using (Bitmap bmp = new Bitmap(path)) {
            List<int> mins = new List<int>();
            bool inRun = false;
            int best = -1;
            long bestV = long.MaxValue;
            for (int x = 0; x < w; x++) {
                Color c = bmp.GetPixel(l + x, y);
                long lum = (long)(0.299 * c.R + 0.587 * c.G + 0.114 * c.B);
                if (lum < thresh) {
                    if (!inRun) { inRun = true; best = -1; bestV = long.MaxValue; }
                    if (lum < bestV) { bestV = lum; best = x; }
                } else {
                    if (inRun) { mins.Add(best); inRun = false; }
                }
            }
            if (inRun) { mins.Add(best); }
            return new int[] { mins.Count, mins.Count > 0 ? mins[0] : -1, mins.Count > 1 ? mins[1] : -1 };
        }
    }
}
'@) -ReferencedAssemblies @('System.Drawing')

[void][S82]::SetProcessDPIAware()
$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Host ('PASS ' + $name) }
    else { $script:fail++; Write-Host ('FAIL ' + $name + ' -- ' + $detail) }
}
function Skip-Scenario($name, $why) {
    $script:skip++; Write-Host ('SKIP ' + $name + ' -- ' + $why)
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
    $ps = Get-Process riviv -ErrorAction SilentlyContinue
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}
function Reset-Ini($text) {
    if (Test-Path $Ini) { Remove-Item $Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($Ini, $text) }
}
function Start-Riv($argStr, $errName) {
    # PS 5.1's Start-Process rejects an EMPTY -ArgumentList string, so the
    # parameter is only passed when there is one. The raw process handle is
    # captured WHILE ALIVE: Close-Main reads the exit code via
    # GetExitCodeProcess on this handle (smoke80 lesson).
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    if ($argStr -eq '') {
        $script:RawP = Start-Process -FilePath $RunExe -PassThru -RedirectStandardError $errPath
    } else {
        $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
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
    # 150 polls = 15 s (the 10 s of smoke80/81 plus margin for a first-launch
    # Defender scan of a freshly copied exe - run-8's S1a start hiccup).
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
function View-Of($main) {
    if (($main -eq $null) -or ($main -eq [IntPtr]::Zero)) { return [IntPtr]::Zero }
    return [S82]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_view', [NullString]::Value)
}
function View-Size($view) {
    if ($view -eq [IntPtr]::Zero) { return @(0, 0) }
    $r = New-Object S82+RECT
    [void][S82]::GetClientRect($view, [ref]$r)
    return @([int]($r.R - $r.L), [int]($r.B - $r.T))
}
function Close-Main($p, $main) {
    if (($main -ne $null) -and ($main -ne [IntPtr]::Zero)) {
        [void][S82]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S82]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else {
            return -2
        }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}

# Calibrate the main window until the riviv_view child's client rect is
# EXACTLY tw x th (chrome is integer pixels, so the delta walk converges).
# Returns the achieved view size.
function Calibrate-View($main, $tw, $th) {
    $script:calView = View-Of $main
    for ($i = 0; $i -lt 5; $i++) {
        $script:calVs = View-Size $script:calView
        if (($script:calVs[0] -eq $tw) -and ($script:calVs[1] -eq $th)) { return $script:calVs }
        $wr = New-Object S82+RECT
        [void][S82]::GetWindowRect($main, [ref]$wr)
        $nw = ($wr.R - $wr.L) + ($tw - $script:calVs[0])
        $nh = ($wr.B - $wr.T) + ($th - $script:calVs[1])
        [void][S82]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, $nw, $nh, 0x0006)  # SWP_NOMOVE|SWP_NOZORDER
        $script:calTw = $tw
        $script:calTh = $th
        $null = Wait-Until { $v = (View-Size $script:calView); ($v[0] -eq $script:calTw) -and ($v[1] -eq $script:calTh) } 3000
    }
    return (View-Size (View-Of $main))
}

# The #82 close-time stats line, parsed and recorded. Returns the first
# match as a hashtable (or $null); every match lands in $script:StatsSeen
# for the S5 sweep.
$StatsPattern = 'riviv: tiles level=(\d+) tiles=(\d+) base=(\d+) gpu=(\d+) peak_gpu=(\d+) inflight=(\d+) peak_inflight=(\d+) uploads=(\d+) evictions=(\d+) mip_builds=(\d+) source=(\d+) cap=(\d+)'
$script:StatsSeen = New-Object System.Collections.ArrayList
# Every d2d/warp scenario's stderr, tagged (S7 scans them for the dump's
# silent "trying the gdi channel" fallback - a negative guard that stays
# after #90: it must never fire again).
$script:D2dErrs = New-Object System.Collections.ArrayList
function Note-D2dErr($err, $tag) {
    [void]$script:D2dErrs.Add(@{ Tag = $tag; Err = $err })
}
function Parse-Stats($err, $tag) {
    $found = $null
    foreach ($mm in [regex]::Matches($err, $StatsPattern)) {
        $h = @{
            Tag = $tag
            Level = [int]$mm.Groups[1].Value
            Tiles = [int]$mm.Groups[2].Value
            Base = [long]$mm.Groups[3].Value
            Gpu = [long]$mm.Groups[4].Value
            PeakGpu = [long]$mm.Groups[5].Value
            Uploads = [int]$mm.Groups[8].Value
            Evictions = [int]$mm.Groups[9].Value
            MipBuilds = [int]$mm.Groups[10].Value
            Cap = [long]$mm.Groups[12].Value
            Line = $mm.Value
        }
        [void]$script:StatsSeen.Add($h)
        if ($found -eq $null) { $found = $h }
    }
    return $found
}
function Stats-Detail($h) {
    if ($h -eq $null) { return '(no stats line)' }
    return ('level={0} tiles={1} base={2} gpu={3} peak_gpu={4} uploads={5} evictions={6} mip_builds={7} cap={8}' -f $h.Level, $h.Tiles, $h.Base, $h.Gpu, $h.PeakGpu, $h.Uploads, $h.Evictions, $h.MipBuilds, $h.Cap)
}

# One adopt -> (calibrate) -> (commands) -> >=1.1 s settle -> WM_CLOSE dump
# instance. The staged ini is rewritten before every launch (Run-Scene
# callers build it via Scene-Ini). Returns a hashtable, emits nothing.
# NOTE: the ini-text parameter is deliberately named $iniText, NOT $ini -
# PowerShell scope lookup is CASE-INSENSITIVE, so a function-local $ini
# would shadow the script-level $Ini (the staged ini path) inside anything
# Run-Scene calls (Reset-Ini would then Test-Path the ini TEXT - the
# "illegal characters in path" abort caught on the first run).
function Scene-Ini($renderer, $wide, $high, $magenta) {
    $lines = @('[riviv]', 'x=40', 'y=40', "wide=$wide", "high=$high", 'auto_zoom=0', 'icm=0')
    if ($magenta) { $lines += @('windowed_background_color_r=255', 'windowed_background_color_g=0', 'windowed_background_color_b=255') }
    if ($renderer -ne '') { $lines += "renderer=$renderer" }
    return (($lines -join "`r`n") + "`r`n")
}
function Run-Scene($iniText, $img, $outName, $errName, $extraArgs, $tw, $th, $cmds, $titleMs) {
    if ($titleMs -eq $null) { $titleMs = 12000 }
    Reset-Ini $iniText
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    $argStr = "`"$img`" -dump-viewport `"$out`""
    if (($extraArgs -ne $null) -and ($extraArgs -ne '')) { $argStr = "`"$img`" $extraArgs -dump-viewport `"$out`"" }
    $p = Start-Riv $argStr $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p ([IO.Path]::GetFileNameWithoutExtension($img)) $titleMs
    $vs = @(0, 0)
    if (($main -ne $null) -and ($main -ne [IntPtr]::Zero)) {
        if (($tw -ne $null) -and ($tw -gt 0)) { $vs = Calibrate-View $main $tw $th }
        else { $vs = View-Size (View-Of $main) }
        if ($cmds) { & $cmds $main }
        Start-Sleep -Milliseconds 1100   # >=1000 ms settle before the WM_CLOSE dump
    }
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Vs = $vs; Adopted = $adopted; Err = (Read-Err $errName) }
}

# Luminance per column from a RowPix buffer (a PS helper, not a C# one, so
# the seam math stays inspectable here).
function Lums-Of($pix, $n) {
    $o = New-Object double[] $n
    for ($c = 0; $c -lt $n; $c++) {
        $o[$c] = 0.299 * $pix[$c * 3] + 0.587 * $pix[$c * 3 + 1] + 0.114 * $pix[$c * 3 + 2]
    }
    return $o
}
function Max-Step-All($lums) {
    $mx = -1.0
    for ($c = 0; ($c + 1) -lt $lums.Length; $c++) {
        $d = [math]::Abs($lums[$c + 1] - $lums[$c])
        if ($d -gt $mx) { $mx = $d }
    }
    return $mx
}
function Max-Step-Near($lums, $bounds, $halfwin) {
    # Largest adjacent-column |step| with the column within +-halfwin of any
    # candidate tile boundary. Returns @(max, column) or @(-1, -1).
    $n = $lums.Length
    $mx = -1.0
    $at = -1
    foreach ($b in $bounds) {
        for ($c = $b - $halfwin; $c -le ($b + $halfwin); $c++) {
            if (($c -ge 0) -and (($c + 1) -lt $n)) {
                $d = [math]::Abs($lums[$c + 1] - $lums[$c])
                if ($d -gt $mx) { $mx = $d; $at = $c }
            }
        }
    }
    return @($mx, $at)
}
function Tile-Boundaries($rectW, $srcW, $srcH, $edge) {
    # Dest-space tile-boundary positions inside a render rect of rectW
    # columns, enumerated for EVERY drawn-level choice that could cover the
    # render (the tile grid edge is in DRAWN-LEVEL pixels; the smoke does
    # not know which level the product picked, so all candidates join the
    # scan window union - the same union is applied to both dumps).
    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    for ($L = 0; $L -le 6; $L++) {
        $lw = $srcW -shr $L
        $lh = $srcH -shr $L
        if (($lw -lt 1) -or ($lh -lt 1)) { break }
        if ($lw -lt $rectW) { continue }   # a level narrower than the render is not the drawn level
        $s = $rectW / [double]$lw
        for ($k = 1; ; $k++) {
            $pos = [int][math]::Round($k * $edge * $s)
            if ($pos -gt ($rectW - 3)) { break }
            if ($pos -ge 3) { [void]$seen.Add($pos) }
        }
    }
    $arr = New-Object System.Collections.Generic.List[int]
    foreach ($v in $seen) { $arr.Add($v) }
    $arr.Sort()
    return $arr.ToArray()
}

# Shape + red band + ramp analysis of a 40000x256 banner dump. Returns a
# hashtable (no pipeline output). bg = white (the default letterbox).
function Banner-Strip-Checks($dumpPath, $viewW) {
    $r = @{}
    $e = [S82]::RectEdges($dumpPath, 255, 255, 255)
    $r.DumpW = $e[4]; $r.DumpH = $e[5]
    $r.L = $e[0]; $r.T = $e[1]
    $r.Bw = $e[2] - $e[0] + 1
    $r.Bh = $e[3] - $e[1] + 1
    $r.ShapeOk = ($r.Bh -ge 1) -and ($r.Bh -le 8) -and ($r.Bw -eq $viewW)
    $r.RedOk = $false
    $r.RampOk = $false
    $r.Detail = 'shape failed'
    if (-not $r.ShapeOk) { return $r }
    $row = $e[1] + [int]($r.Bh / 2)
    $span = [S82]::RedSpan($dumpPath, $row, $e[0], $r.Bw)
    $r.First = $span[0]
    $r.Last = $span[1]
    # Theoretical slot of source x in [18000, 22000) within the strip.
    $r.E0 = [math]::Floor(18000 * $r.Bw / 40000)
    $r.E1 = [math]::Ceiling(22000 * $r.Bw / 40000) - 1
    $r.RedOk = (($span[0] -ge 0) -and ([math]::Abs($span[0] - $r.E0) -le 10) -and ([math]::Abs($span[1] - $r.E1) -le 10))
    # The rest of the strip row is a gray ramp: gray-ish outside the band
    # (>=95%), darker on the left of the band than on the right, neither
    # black nor clipped.
    $pix = [S82]::RowPix($dumpPath, $row, $e[0], $r.Bw)
    $gray = 0; $grayN = 0
    for ($c = 0; $c -lt $r.Bw; $c++) {
        if (($c -ge ($r.First - 5)) -and ($c -le ($r.Last + 5))) { continue }
        $grayN++
        $rr = $pix[$c * 3]; $gg = $pix[$c * 3 + 1]; $bb2 = $pix[$c * 3 + 2]
        if (([math]::Abs($rr - $gg) -le 12) -and ([math]::Abs($gg - $bb2) -le 12)) { $gray++ }
    }
    $r.GrayFrac = -1.0
    if ($grayN -gt 0) { $r.GrayFrac = $gray / $grayN }
    $lums = Lums-Of $pix $r.Bw
    # Left flank = the WHOLE run left of the band (a window at the ramp
    # knee reads ~113 and masquerades as a flat profile - run-4 finding).
    $l0 = 8
    $l1 = [int][math]::Max($l0, $r.First - 9)
    $ls = 0.0; $ln = 0; $rs = 0.0; $rn = 0
    for ($c = $l0; $c -le $l1; $c++) { $ls += $lums[$c]; $ln++ }
    # Right flank runs to the strip's right edge (the band sits mid-strip;
    # a short window right of the band would sample the ~140 ramp knee and
    # read as a flat profile - the run-3 finding).
    $r0 = [int][math]::Min($r.Bw - 9, $r.Last + 8)
    for ($c = $r0; $c -le ($r.Bw - 1); $c++) { $rs += $lums[$c]; $rn++ }
    $r.LeftMean = -1.0; $r.RightMean = -1.0
    if ($ln -gt 0) { $r.LeftMean = $ls / $ln }
    if ($rn -gt 0) { $r.RightMean = $rs / $rn }
    $r.RampOk = (($r.GrayFrac -ge 0.95) -and ($r.LeftMean -ge 5) -and ($r.LeftMean -le 120) -and
        ($r.RightMean -ge 140) -and ($r.RightMean -le 250) -and (($r.RightMean - $r.LeftMean) -ge 90))
    $r.Detail = ('bbox l={0} t={1} {2}x{3} row={4} red=[{5}..{6}] want=[{7}..{8}] grayFrac={9:N3} leftMean={10:N1} rightMean={11:N1}' -f
        $e[0], $e[1], $r.Bw, $r.Bh, $row, $r.First, $r.Last, $r.E0, $r.E1, $r.GrayFrac, $r.LeftMean, $r.RightMean)
    return $r
}

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$CMD_ONE2ONE = 45      # menu.rs Cmd::ViewOneToOne.id()
$CMD_BESTFIT = 46      # menu.rs Cmd::ViewBestFit.id()
$CMD_ROTATE90 = 23     # menu.rs Cmd::EditRotate90.id()
$one2one = { param($m) [void][S82]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }

# View targets (physical px, 200% DPI dev machine - the measured facts'
# geometry): tall = fit caps at 100% for 900x600 (true 1:1 possible);
# slim = the measured fractional-scale fit viewport; strip = the measured
# banner-fit viewport.
$TallW = 1200; $TallH = 900
$SlimW = 674; $SlimH = 246
$StripW = 974; $StripH = 484

$Stage = Join-Path $env:TEMP 'riviv-82-smoke'
$Ini = Join-Path $Stage 'riviv.ini'

Kill-Riviv
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
$RunExe = Join-Path $Stage 'riviv.exe'
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied, stage ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini))) 'Copy-Item failed or leftover ini'

# ---------------------------------------------------------------------------
# S0 fixtures
# ---------------------------------------------------------------------------
$grad900 = Join-Path $Stage 'grad900.png'
[S82]::WriteGradRamp($grad900, 900, 600)
$banner = Join-Path $Stage 'banner82.png'
[S82]::WriteBanner($banner, 40000, 256, 18000, 22000)
$giant = Join-Path $Stage 'giant82.png'
[S82]::WriteWidePngGrad($giant, 16777217, 1)
$hardedge900 = Join-Path $Stage 'hardedge900.png'
[S82]::WriteHardEdge($hardedge900, 900, 600, 256, 300)
Check 'S0 fixtures built (grad900 + 40000x256 banner + 16777217x1 giant + hardedge900)' ((Test-Path $grad900) -and (Test-Path $banner) -and (Test-Path $giant) -and (Test-Path $hardedge900)) 'a fixture PNG is missing'

# ---------------------------------------------------------------------------
# S1 + S2a: the tall window (view exactly 1200x900 -> fit caps at 100%).
# Run A: -tile 256 -> the close stats line must show the tile path at
# level 0. Run B: no -tile -> no stats line at all. The two dumps at TRUE
# 1:1 (WM_COMMAND 45 in both) must be byte-identical (S2a), and the ramp
# content sampled against the source formula so identity cannot mean blank.
# ---------------------------------------------------------------------------
$tallIni = Scene-Ini 'd2d' ($TallW + 40) ($TallH + 160) $true
$a1 = Run-Scene $tallIni $grad900 's1-tiled.png' 's1-tiled.err' '-tile 256' $TallW $TallH $one2one 12000
$st1 = Parse-Stats $a1.Err 'S1-tiled'
Note-D2dErr $a1.Err 'S1-tiled'
Check 'S1a -tile 256 run clean (adopted, exit 0, dump exists)' (($a1.Code -eq 0) -and $a1.Adopted -and (Test-Path $a1.Out)) ("exit=$($a1.Code) adopted=$($a1.Adopted) view=$($a1.Vs[0])x$($a1.Vs[1]) stderr=[$($a1.Err.Trim())]")
Check 'S1b d2d breadcrumb "riviv: d2d max_bitmap=<n> tile cap=<n>" present' (($a1.Err -match 'riviv: d2d max_bitmap=\d+ tile cap=\d+')) ("stderr=[$($a1.Err.Trim())]")
Check 'S1c -tile 256 stats line: level=0, tiles>0, uploads>0' (($st1 -ne $null) -and ($st1.Level -eq 0) -and ($st1.Tiles -gt 0) -and ($st1.Uploads -gt 0)) (Stats-Detail $st1)
$b1 = Run-Scene $tallIni $grad900 's1-untiled.png' 's1-untiled.err' '' $TallW $TallH $one2one 12000
$st1b = Parse-Stats $b1.Err 'S1-untiled'
Note-D2dErr $b1.Err 'S1-untiled'
Check 'S1d run WITHOUT -tile prints no stats line and no gate line (breadcrumb present)' (($st1b -eq $null) -and ($b1.Err -match 'backend=d2d') -and (-not $b1.Err.Contains('exceeds the D2D max bitmap'))) ("statsSeen=$($st1b -ne $null) exit=$($b1.Code) stderr=[$($b1.Err.Trim())]")
if (($st1 -eq $null) -or (-not (Test-Path $a1.Out)) -or (-not (Test-Path $b1.Out))) {
    Skip-Scenario 'S2a 1:1 byte-identity + ramp samples' 'the S1 run pair did not produce both dumps; see S1a/S1d evidence'
} else {
    $ha = (Get-FileHash $a1.Out -Algorithm SHA256).Hash
    $hb = (Get-FileHash $b1.Out -Algorithm SHA256).Hash
    $dimA = [S82]::RectEdges($a1.Out, 255, 0, 255)
    $dimB = [S82]::RectEdges($b1.Out, 255, 0, 255)
    Check 'S2a-1 TRUE 1:1: tiled and untiled dump files byte-identical (Get-FileHash)' (($ha -eq $hb) -and ($dimA[4] -eq $dimB[4]) -and ($dimA[5] -eq $dimB[5])) ("hashA=$($ha.Substring(0, 16)) hashB=$($hb.Substring(0, 16)) dimsA=$($dimA[4])x$($dimA[5]) dimsB=$($dimB[4])x$($dimB[5])")
    # The bbox must be exactly the 900x600 source (true 1:1 proof: fit
    # capped at 100%, then WM_COMMAND 45).
    $bw = $dimB[2] - $dimB[0] + 1
    $bh = $dimB[3] - $dimB[1] + 1
    Check 'S2a-2 image bbox is exactly 900x600 (fit capped at 100%)' (($bw -eq 900) -and ($bh -eq 600)) ("bbox l=$($dimB[0]) t=$($dimB[1]) ${bw}x${bh}")
    # Ramp samples at known source coords vs the writer's integer formula,
    # +-2 per channel, so byte-identity cannot mean identically blank.
    $ixs = @(50, 200, 400, 600, 750, 850)
    $iys = @(30, 150, 300, 420, 520, 560)
    $xs = New-Object System.Collections.Generic.List[int]
    $ys = New-Object System.Collections.Generic.List[int]
    for ($i = 0; $i -lt $ixs.Count; $i++) { $xs.Add($dimB[0] + $ixs[$i]); $ys.Add($dimB[1] + $iys[$i]) }
    $smp = [S82]::SamplePts($b1.Out, $xs.ToArray(), $ys.ToArray())
    $smpOk = $true
    $smpDetail = ''
    for ($i = 0; $i -lt $ixs.Count; $i++) {
        $eR = [math]::Floor($ixs[$i] * 255 / 899)
        $eG = [math]::Floor($iys[$i] * 255 / 599)
        $gR = $smp[$i * 3]; $gG = $smp[$i * 3 + 1]; $gB = $smp[$i * 3 + 2]
        if (([math]::Abs($gR - $eR) -gt 2) -or ([math]::Abs($gG - $eG) -gt 2) -or ([math]::Abs($gB - 128) -gt 2)) {
            $smpOk = $false
            $smpDetail += (" src({0},{1}) got=({2},{3},{4}) want=({5},{6},128)" -f $ixs[$i], $iys[$i], $gR, $gG, $gB, $eR, $eG)
        }
    }
    if ($smpDetail -eq '') { $smpDetail = 'all 6 ramp samples within +-2 of the source formula' }
    Check 'S2a-3 ramp content present and correct (6 samples, +-2/channel)' $smpOk $smpDetail
    # R3 P2-6: S1c's tiles>0 proves the PAINT tiled; this close stats line
    # prints AFTER the WM_CLOSE dump re-planned with the same -tile, so
    # tiles>0 here is the DUMP-side tiling proof.
    Check 'S2a-4 tiled dump re-planned with tiles at close (close stats tiles>0)' (($st1 -ne $null) -and ($st1.Tiles -gt 0)) (Stats-Detail $st1)
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2b: zero-seam at the fractional scale. The same 900x600 ramp at the
# DEFAULT fit into exactly 674x246 (measured viewport), -tile 256 vs no
# -tile. Whole-frame maxDelta <= 1; the seam scan (max adjacent-column
# luminance step within +-2 px of every candidate tile boundary, the same
# candidate union applied to both dumps) must satisfy
# tiledMax <= untiledMax + 1.
# ---------------------------------------------------------------------------
$slimIni = Scene-Ini 'd2d' ($SlimW + 40) ($SlimH + 160) $true
$b2 = Run-Scene $slimIni $grad900 's2b-untiled.png' 's2b-untiled.err' '' $SlimW $SlimH $null 12000
$t2 = Run-Scene $slimIni $grad900 's2b-tiled.png' 's2b-tiled.err' '-tile 256' $SlimW $SlimH $null 12000
[void](Parse-Stats $b2.Err 'S2b-untiled')
$st2t = Parse-Stats $t2.Err 'S2b-tiled'
Note-D2dErr $b2.Err 'S2b-untiled'
Note-D2dErr $t2.Err 'S2b-tiled'
$s2bRan = ($b2.Code -eq 0) -and ($t2.Code -eq 0) -and (Test-Path $b2.Out) -and (Test-Path $t2.Out)
Check 'S2b-a both fit dumps ran clean (exit 0, files exist)' $s2bRan ("untiled exit=$($b2.Code) tiled exit=$($t2.Code) views=$($b2.Vs[0])x$($b2.Vs[1])/$($t2.Vs[0])x$($t2.Vs[1])")
if ($s2bRan) {
    $ps2 = [S82]::PairStats($t2.Out, $b2.Out)
    Check 'S2b-b fractional scale: whole-frame maxDelta <= 1' (($ps2[0] -ge 0) -and ($ps2[0] -le 1)) ("max=$($ps2[0]) diffPx=$($ps2[1]) of $($ps2[2])x$($ps2[3]) (measured reference: max=1 on ~9.5% of pixels)")
    # R3 P2-6: the dump re-plans with the same -tile at WM_CLOSE (before
    # the stats line prints) - tiles>0 here is the DUMP-side tiling proof
    # for this scene; S1c only proved the live paint.
    Check 'S2b-b2 ramp tiled dump re-planned with tiles at close (close stats tiles>0)' (($st2t -ne $null) -and ($st2t.Tiles -gt 0)) (Stats-Detail $st2t)
    $eu2 = [S82]::RectEdges($b2.Out, 255, 0, 255)
    $rw2 = $eu2[2] - $eu2[0] + 1
    $rh2 = $eu2[3] - $eu2[1] + 1
    $row2 = $eu2[1] + [int]($rh2 / 2)
    $lu2 = Lums-Of ([S82]::RowPix($b2.Out, $row2, $eu2[0], $rw2)) $rw2
    $lt2 = Lums-Of ([S82]::RowPix($t2.Out, $row2, $eu2[0], $rw2)) $rw2
    $bnds2 = Tile-Boundaries $rw2 900 600 256
    if ($bnds2.Count -eq 0) {
        Skip-Scenario 'S2b-c seam scan' ('no candidate tile boundary inside the render rect (rectW=' + $rw2 + ')')
    } else {
        $su2 = Max-Step-Near $lu2 $bnds2 2
        $st2 = Max-Step-Near $lt2 $bnds2 2
        $gu2 = Max-Step-All $lu2
        $gt2 = Max-Step-All $lt2
        Check 'S2b-c seam scan: tiled boundary step <= untiled step + 1' (($st2[0] -ge 0) -and ($st2[0] -le ($su2[0] + 1.0))) ("bounds=[$($bnds2 -join ',')] tiledMaxStep={0} untiledMaxStep={1} globalTiled={2:N2} globalUntiled={3:N2} (measured reference: 0.299 vs 0.299)" -f $st2[0], $su2[0], $gt2, $gu2)
        Write-Host ('  S2b evidence: maxDelta=' + $ps2[0] + ' diffPx=' + $ps2[1] + ' of ' + $ps2[2] + 'x' + $ps2[3] + ' rect=' + $rw2 + 'x' + $rh2 + ' bounds=[' + ($bnds2 -join ',') + '] boundaryStepTiled=' + $st2[0] + ' boundaryStepUntiled=' + $su2[0] + ' globalRowStepTiled=' + $gt2 + ' globalRowStepUntiled=' + $gu2)
    }
} else {
    Skip-Scenario 'S2b-b/S2b-c fit comparisons' 'a dump channel failed; see S2b-a detail'
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2b-d (R3 P2-1): the hard-edge probe. The smooth ramp's ~0.38/col step
# hides a dropped or duplicated column at a tile boundary inside the
# S2b-b/c tolerances, so this fixture puts a 1-px full-height BLACK column
# ON a -tile 256 grid boundary (source x = 256; a device-fitting master
# tiles at level 0, so the grid edges are source x = 256, 512, ...) and a
# second one MID-TILE (source x = 300), on a flat mid-grey field. The two
# dips' screen columns must sit at the expected projection positions in
# BOTH dumps and be the SAME in both: both draws share the global i64
# projection, so a dropped/duplicated boundary column moves or erases the
# tiled dip while the untiled one stays put.
# ---------------------------------------------------------------------------
$slimIni2 = Scene-Ini 'd2d' ($SlimW + 40) ($SlimH + 160) $true
$bdu = Run-Scene $slimIni2 $hardedge900 's2bd-untiled.png' 's2bd-untiled.err' '' $SlimW $SlimH $null 12000
$bdt = Run-Scene $slimIni2 $hardedge900 's2bd-tiled.png' 's2bd-tiled.err' '-tile 256' $SlimW $SlimH $null 12000
[void](Parse-Stats $bdu.Err 'S2b-d-untiled')
$stdbd = Parse-Stats $bdt.Err 'S2b-d-tiled'
Note-D2dErr $bdu.Err 'S2b-d-untiled'
Note-D2dErr $bdt.Err 'S2b-d-tiled'
$bdRan = ($bdu.Code -eq 0) -and ($bdt.Code -eq 0) -and (Test-Path $bdu.Out) -and (Test-Path $bdt.Out)
Check 'S2b-d hard-edge runs clean + tiled dump re-planned with tiles (close stats tiles>0)' ($bdRan -and ($stdbd -ne $null) -and ($stdbd.Tiles -gt 0)) ("untiled exit=$($bdu.Code) tiled exit=$($bdt.Code) statsTiled=" + (Stats-Detail $stdbd))
if ($bdRan) {
    $eud = [S82]::RectEdges($bdu.Out, 255, 0, 255)
    $rwd = $eud[2] - $eud[0] + 1
    $rhd = $eud[3] - $eud[1] + 1
    $edd = [S82]::RectEdges($bdt.Out, 255, 0, 255)
    $rwt = $edd[2] - $edd[0] + 1
    $rht = $edd[3] - $edd[1] + 1
    $rowd = $eud[1] + [int]($rhd / 2)
    $rowt = $edd[1] + [int]($rht / 2)
    $du = [S82]::DarkDips($bdu.Out, $rowd, $eud[0], $rwd, 100)
    $dt = [S82]::DarkDips($bdt.Out, $rowt, $edd[0], $rwt, 100)
    # Expected dip columns: the shared projection dest = l + src*rectW/900.
    $expA = [math]::Floor(256 * $rwd / 900)
    $expB = [math]::Floor(300 * $rwd / 900)
    $bdOk = (($rwt -eq $rwd) -and ($du[0] -eq 2) -and ($dt[0] -eq 2) -and
        ([math]::Abs($du[1] - $expA) -le 2) -and ([math]::Abs($du[2] - $expB) -le 2) -and
        ($dt[1] -eq $du[1]) -and ($dt[2] -eq $du[2]))
    Check 'S2b-d hard-edge: 2 dark columns at the expected screen columns in BOTH dumps, tiled == untiled exactly (no dropped/duplicated boundary column)' $bdOk ("untiledDips=$($du[0]) at [$($du[1]),$($du[2])] tiledDips=$($dt[0]) at [$($dt[1]),$($dt[2])] expected=[$expA,$expB] rect=$rwd x $rhd rows=$rowd/$rowt (thresh 100)")
    Write-Host ('  S2b-d evidence: untiledDips=' + $du[0] + ' at [' + $du[1] + ',' + $du[2] + '] tiledDips=' + $dt[0] + ' at [' + $dt[1] + ',' + $dt[2] + '] expected=[' + $expA + ',' + $expB + '] srcCols=[256 boundary,300 mid-tile] rect=' + $rwd + 'x' + $rhd)
    # The high-contrast BOUND (external review AI2 P2-2): the recorded
    # content-edge figure (<= 21 per channel, README/ADR) finally has an
    # assertion watching it - on THIS hard-edge pair, which is the content
    # class the number was measured on.
    $psd = [S82]::PairStats($bdt.Out, $bdu.Out)
    Check 'S2b-e hard-edge whole-frame maxDelta <= 21 (the recorded high-contrast bound)' (($psd[0] -ge 0) -and ($psd[0] -le 21)) ("max=$($psd[0]) diffPx=$($psd[1]) of $($psd[2])x$($psd[3]) (README/ADR recorded bound: 21)")
} else {
    Skip-Scenario 'S2b-d hard-edge dip comparisons' 'a dump channel failed; see the S2b-d runs-clean detail'
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the hardware giant. 40000x256 banner (renderer=d2d): fit -> the
# overview path; 1:1 (WM_COMMAND 45) -> the tile path. The old gate line
# must never appear.
# ---------------------------------------------------------------------------
$stripIni = Scene-Ini 'd2d' ($StripW + 40) ($StripH + 160) $false
$f3 = Run-Scene $stripIni $banner 's3-fit.png' 's3-fit.err' '' $StripW $StripH $null 30000
$st3f = Parse-Stats $f3.Err 'S3-fit'
Note-D2dErr $f3.Err 'S3-fit'
Check 'S3a fit run clean (adopted, exit 0, dump exists)' (($f3.Code -eq 0) -and $f3.Adopted -and (Test-Path $f3.Out)) ("exit=$($f3.Code) adopted=$($f3.Adopted) view=$($f3.Vs[0])x$($f3.Vs[1]) stderr=[$($f3.Err.Trim())]")
Check 'S3b fit: NO old gate line (breadcrumb present)' (($f3.Err -match 'backend=d2d') -and (-not $f3.Err.Contains('exceeds the D2D max bitmap'))) ("stderr=[$($f3.Err.Trim())]")
Check 'S3c fit stats: level>=1, tiles=0, base>0, mip_builds>=1 (overview path)' (($st3f -ne $null) -and ($st3f.Level -ge 1) -and ($st3f.Tiles -eq 0) -and ($st3f.Base -gt 0) -and ($st3f.MipBuilds -ge 1)) (Stats-Detail $st3f)
$fchk = $null
if (Test-Path $f3.Out) {
    $fchk = Banner-Strip-Checks $f3.Out $f3.Vs[0]
    Check 'S3d fit strip shape: spans the viewport width, 1..8 px tall' $fchk.ShapeOk ($fchk.Detail + " viewport=$($f3.Vs[0])x$($f3.Vs[1])")
    Check 'S3e fit red band at the expected columns (+-10)' $fchk.RedOk $fchk.Detail
    Check 'S3f fit surrounding row is a ramp, not black (gray>=95%, left<right means)' $fchk.RampOk $fchk.Detail
} else {
    Skip-Scenario 'S3d/S3e/S3f fit strip assertions' 'no fit dump on disk'
}
if ($fchk -ne $null) { Write-Host ('  S3 fit evidence: ' + $fchk.Detail + ' stats=[' + (Stats-Detail $st3f) + ']') }
Kill-Riviv
Reset-Ini ''
$o3 = Run-Scene $stripIni $banner 's3-one2one.png' 's3-1to1.err' '' $StripW $StripH $one2one 30000
$st3o = Parse-Stats $o3.Err 'S3-1to1'
Note-D2dErr $o3.Err 'S3-1to1'
Check 'S3g 1:1 run clean (adopted, exit 0, dump exists)' (($o3.Code -eq 0) -and $o3.Adopted -and (Test-Path $o3.Out)) ("exit=$($o3.Code) adopted=$($o3.Adopted) stderr=[$($o3.Err.Trim())]")
Check 'S3h 1:1: NO old gate line (breadcrumb present)' (($o3.Err -match 'backend=d2d') -and (-not $o3.Err.Contains('exceeds the D2D max bitmap'))) ("stderr=[$($o3.Err.Trim())]")
Check 'S3i 1:1 stats: level=0, tiles>0, base=0, uploads>=1 (tile path)' (($st3o -ne $null) -and ($st3o.Level -eq 0) -and ($st3o.Tiles -gt 0) -and ($st3o.Base -eq 0) -and ($st3o.Uploads -ge 1)) (Stats-Detail $st3o)
$redOk3 = $false
$redDet3 = 'no dump'
if (Test-Path $o3.Out) {
    $eo3 = [S82]::RectEdges($o3.Out, 255, 255, 255)
    $iw = $eo3[2] - $eo3[0] + 1
    $ih = $eo3[3] - $eo3[1] + 1
    if (($iw -gt 4) -and ($ih -gt 4)) {
        $rf = [S82]::RedFraction($o3.Out, $eo3[0], $eo3[1], $iw, $ih)
        $frac = 0.0
        if ($rf[1] -gt 0) { $frac = $rf[0] / $rf[1] }
        $redOk3 = ($frac -ge 0.95)
        $redDet3 = ('image rect {0}x{1} at ({2},{3}): red {4}/{5} samples = {6:P1} (1:1 centers source x ~20000, inside the [18000,22000) band)' -f $iw, $ih, $eo3[0], $eo3[1], $rf[0], $rf[1], $frac)
    } else {
        $redDet3 = "image rect degenerate ${iw}x${ih} at l=$($eo3[0]) t=$($eo3[1])"
    }
}
Check 'S3j 1:1: red band fills >=95% of the sampled image rect' $redOk3 $redDet3
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: the WARP giant. 16777217x1 gradient (2^24+1 > WARP max_bitmap 2^23):
# the overview path, no gate line, non-black gradient, gpu <= cap.
# ---------------------------------------------------------------------------
$w4 = Run-Scene (Scene-Ini 'warp' ($StripW + 40) ($StripH + 160) $false) $giant 's4-warp.png' 's4-warp.err' '' $null $null $null 90000
$st4 = Parse-Stats $w4.Err 'S4-warp'
Note-D2dErr $w4.Err 'S4-warp'
$bcMax4 = ''
if ($w4.Err -match 'riviv: d2d max_bitmap=(\d+) tile cap=(\d+)') { $bcMax4 = 'max_bitmap=' + $Matches[1] + ' cap=' + $Matches[2] }
Check 'S4a warp giant run clean (adopted, exit 0, dump exists)' (($w4.Code -eq 0) -and $w4.Adopted -and (Test-Path $w4.Out)) ("exit=$($w4.Code) adopted=$($w4.Adopted) view=$($w4.Vs[0])x$($w4.Vs[1]) stderr=[$($w4.Err.Trim())]")
Check 'S4b warp giant: NO old gate line (breadcrumb present)' (($w4.Err -match 'backend=d2d/warp') -and (-not $w4.Err.Contains('exceeds the D2D max bitmap'))) ("stderr=[$($w4.Err.Trim())]")
Check 'S4c warp giant breadcrumb present (riviv: d2d max_bitmap=...)' ($bcMax4 -ne '') ("bc=[$bcMax4] stderr=[$($w4.Err.Trim())]")
Check 'S4d warp giant stats: mip_builds>=1, tiles=0, base>0, level>=1' (($st4 -ne $null) -and ($st4.MipBuilds -ge 1) -and ($st4.Tiles -eq 0) -and ($st4.Base -gt 0) -and ($st4.Level -ge 1)) (Stats-Detail $st4)
Check 'S4e warp giant gpu <= cap' (($st4 -ne $null) -and ($st4.Gpu -le $st4.Cap)) (Stats-Detail $st4)
$gradOk4 = $false
$gradDet4 = 'no dump'
if (Test-Path $w4.Out) {
    $e4 = [S82]::RectEdges($w4.Out, 255, 255, 255)
    $gw4 = $e4[2] - $e4[0] + 1
    $gh4 = $e4[3] - $e4[1] + 1
    $shape4 = ($gh4 -ge 1) -and ($gh4 -le 2) -and ($gw4 -eq $w4.Vs[0])
    if ($shape4) {
        $row4 = $e4[1] + [int]($gh4 / 2)
        $pix4 = [S82]::RowPix($w4.Out, $row4, $e4[0], $gw4)
        # Assert against the FIXTURE's own formula (the S2a approach). The
        # giant payload is r = trunc(x*255/(w-1)), g = 255-r, b = 64, so the
        # luminance at strip fraction f is
        #   lum(f) = 0.299*R + 0.587*(255-R) + 0.114*64,  R = trunc(255*f)
        # a DECREASING ramp: 149.8 at 10% down to 88.7 at 90%. The level-14
        # 16x box average reproduces a linear ramp's midpoint exactly and
        # CUBIC reproduces a linear signal in the interior, so the drawn
        # profile tracks the formula closely (measured maxErr 2.32 at the
        # 90% sample, the farthest from the strip's anchored left edge).
        # Per-sample tolerance 3 leaves margin for box/truncation noise
        # while still failing any real content error (a channel swap moves
        # lum by tens, a black frame by ~100). Smoothness is asserted as a
        # per-step deviation from the formula's own steps, bounded by 2
        # (measured <= 1.2) - deliberately NOT strict monotonicity, which
        # CUBIC ringing near a sample point may legitimately reverse by < 1.
        $fr4 = @(0.1, 0.3, 0.5, 0.7, 0.9)
        $sams4 = New-Object System.Collections.Generic.List[double]
        $exp4 = New-Object System.Collections.Generic.List[double]
        $minL4 = 999.0
        $maxErr4 = 0.0
        foreach ($frv in $fr4) {
            $c4 = [int][math]::Round($frv * ($gw4 - 1))
            $v4 = 0.299 * $pix4[$c4 * 3] + 0.587 * $pix4[$c4 * 3 + 1] + 0.114 * $pix4[$c4 * 3 + 2]
            [void]$sams4.Add($v4)
            if ($v4 -lt $minL4) { $minL4 = $v4 }
            $rExp4 = [math]::Floor(255.0 * $frv)
            [void]$exp4.Add((0.299 * $rExp4) + (0.587 * (255.0 - $rExp4)) + (0.114 * 64.0))
        }
        for ($i = 0; $i -lt 5; $i++) {
            $err4 = [math]::Abs($sams4[$i] - $exp4[$i])
            if ($err4 -gt $maxErr4) { $maxErr4 = $err4 }
        }
        $maxStepErr4 = 0.0
        for ($i = 0; $i -lt 4; $i++) {
            $se4 = [math]::Abs(($sams4[$i + 1] - $sams4[$i]) - ($exp4[$i + 1] - $exp4[$i]))
            if ($se4 -gt $maxStepErr4) { $maxStepErr4 = $se4 }
        }
        $gradOk4 = (($minL4 -gt 8.0) -and ($maxErr4 -le 3.0) -and ($maxStepErr4 -le 2.0))
        $gradDet4 = ('strip {0}x{1}: got=[{2:N1} {3:N1} {4:N1} {5:N1} {6:N1}] fixtureFormula=[{7:N1} {8:N1} {9:N1} {10:N1} {11:N1}] maxErr={12:N2}(tol 3) maxStepErr={13:N2}(tol 2; profile smooth vs formula, not asserted strictly monotonic) min={14:N1}(floor 8)' -f $gw4, $gh4, $sams4[0], $sams4[1], $sams4[2], $sams4[3], $sams4[4], $exp4[0], $exp4[1], $exp4[2], $exp4[3], $exp4[4], $maxErr4, $maxStepErr4, $minL4)
    } else {
        $gradDet4 = "strip shape wrong: l=$($e4[0]) t=$($e4[1]) ${gw4}x${gh4} viewport=$($w4.Vs[0])x$($w4.Vs[1]) (want width == viewport, 1..2 tall)"
    }
}
Check 'S4f warp giant dump tracks the fixture gradient formula at 5 sampled columns (non-black, smooth)' $gradOk4 $gradDet4
Write-Host ('  S4 fit evidence: ' + $gradDet4 + ' stats=[' + (Stats-Detail $st4) + ']')
Kill-Riviv
Reset-Ini ''

# S4 record-only diagnostics pinning the S4d/S4f finding (no Checks - the
# assertions above already carry the verdict; these answer "how deep does
# it go"): (a) the same giant at 1:1 on warp (dest height 1 px there, so a
# visible row would confine the blankness to the sub-pixel fit height);
# (b) the same giant at fit on the HARDWARE arm (max_bitmap 16384), to
# show whether the finding is WARP-specific.
function Strip-Evidence($dumpPath) {
    $e = [S82]::RectEdges($dumpPath, 255, 255, 255)
    $gw = $e[2] - $e[0] + 1
    $gh = $e[3] - $e[1] + 1
    if ($e[0] -lt 0) { return 'all-white dump (no non-white pixel on the center lines)' }
    return ('strip ' + $gw + 'x' + $gh + ' at (' + $e[0] + ',' + $e[1] + ')')
}
$w4g = Run-Scene (Scene-Ini 'warp' ($StripW + 40) ($StripH + 160) $false) $giant 's4g-warp-1to1.png' 's4g.err' '' $null $null $one2one 90000
$st4g = Parse-Stats $w4g.Err 'S4g-warp-1to1-record'
Note-D2dErr $w4g.Err 'S4g-warp-1to1-record'
Write-Host ('  S4g evidence (record-only) warp giant at 1:1: exit=' + $w4g.Code + ' dump=' + (Strip-Evidence $w4g.Out) + ' stats=' + (Stats-Detail $st4g))
Kill-Riviv
Reset-Ini ''
$w4h = Run-Scene (Scene-Ini 'd2d' ($StripW + 40) ($StripH + 160) $false) $giant 's4h-hw-fit.png' 's4h.err' '' $null $null $null 90000
$st4h = Parse-Stats $w4h.Err 'S4h-hw-fit-record'
Note-D2dErr $w4h.Err 'S4h-hw-fit-record'
Write-Host ('  S4h evidence (record-only) hardware giant at fit: exit=' + $w4h.Code + ' dump=' + (Strip-Evidence $w4h.Out) + ' stats=' + (Stats-Detail $st4h))
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S5: budget bounds + no leak. Two more 1:1 banner instances with
# -tile 256 (repeated tile uploads, independent sessions), then the sweep
# over EVERY stats line captured anywhere in this run.
# ---------------------------------------------------------------------------
$p5a = Run-Scene $stripIni $banner 's5-p1.png' 's5-p1.err' '-tile 256' $StripW $StripH $one2one 30000
$st5a = Parse-Stats $p5a.Err 'S5-p1'
Note-D2dErr $p5a.Err 'S5-p1'
Kill-Riviv
Reset-Ini ''
$p5b = Run-Scene $stripIni $banner 's5-p2.png' 's5-p2.err' '-tile 256' $StripW $StripH $one2one 30000
$st5b = Parse-Stats $p5b.Err 'S5-p2'
Note-D2dErr $p5b.Err 'S5-p2'
Kill-Riviv
Reset-Ini ''
Check 'S5a repeated-paint instance 1: uploads>0, gpu<=cap, peak_gpu<=cap' (($st5a -ne $null) -and ($st5a.Uploads -gt 0) -and ($st5a.Gpu -le $st5a.Cap) -and ($st5a.PeakGpu -le $st5a.Cap)) ("exit=$($p5a.Code) " + (Stats-Detail $st5a))
Check 'S5b repeated-paint instance 2: uploads>0, gpu<=cap, peak_gpu<=cap (bounded, not growing)' (($st5b -ne $null) -and ($st5b.Uploads -gt 0) -and ($st5b.Gpu -le $st5b.Cap) -and ($st5b.PeakGpu -le $st5b.Cap)) ("exit=$($p5b.Code) " + (Stats-Detail $st5b))
# The S5c budget sweep moved below S9: running it as the LAST assertion
# covers the stats lines of EVERY scenario (S8/S9 included), which is
# strictly stronger than the old S1..S5-only sweep.

# ---------------------------------------------------------------------------
# S6 is RETIRED with the GDI render arm (#90): the renderer=gdi banner
# record-only run (same shape/band/ramp assertions on the GDI giant relief)
# cannot run anymore - renderer=gdi maps to auto, and the relief path the
# section exercised no longer exists. The number stays reserved.
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# S8: the long-animation churn on the stripe family (design section 3's "long
# animation" scenario). ONE d2d instance: adopt the 16777217x1 giant with
# -tile 256, then 1:1 -> best-fit -> 1:1 (WM_COMMAND 45/46/45), then close.
# Each zoom change repaints through a different plan shape (level-0 tiles
# vs the level-14 overview), so the final close stats line must show a real
# upload history with resident bytes still bounded.
# ---------------------------------------------------------------------------
$churnCmds = { param($m)
    [void][S82]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    [void][S82]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_BESTFIT, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    [void][S82]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero)
}
$c8 = Run-Scene (Scene-Ini 'd2d' ($StripW + 40) ($StripH + 160) $false) $giant 's8-churn.png' 's8-churn.err' '-tile 256' $null $null $churnCmds 90000
Note-D2dErr $c8.Err 'S8-churn'
$st8 = Parse-Stats $c8.Err 'S8-churn'
Check 'S8 stripe churn runs clean (adopted, exit 0, dump exists)' (($c8.Code -eq 0) -and $c8.Adopted -and (Test-Path $c8.Out)) ("exit=$($c8.Code) adopted=$($c8.Adopted) stderr=[$($c8.Err.Trim())]")
Check 'S8a churn: uploads>0 after 1:1->fit->1:1' (($st8 -ne $null) -and ($st8.Uploads -gt 0)) (Stats-Detail $st8)
Check 'S8b churn: gpu<=cap and peak_gpu<=cap after the churn' (($st8 -ne $null) -and ($st8.Gpu -le $st8.Cap) -and ($st8.PeakGpu -le $st8.Cap)) (Stats-Detail $st8)
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# NOTE (external review AI1 P2-1): the FORCED -tile plan deliberately
# bypasses the frame-budget check, so this frame runs over cap and the LRU
# evicts frame tiles DURING the frame - a diagnostic half-coverage by
# design. S9 asserts the budget ARITHMETIC (evictions happen, resident
# stays under cap); it does NOT assert full coverage, and the natural
# (unforced) path is what the no-half-cover unit tests pin.
# S9: the pressure Check - force LRU evictions cheaply in ONE frame. With
# -tile 4 on the 900x600 ramp at a fit-capped (1:1) viewport, the whole
# master is the visible preimage: ceil(900/4) x ceil(600/4) = 225 x 150 =
# 33,750 tiles, each (4+2*32)^2*4 = 18,496 bytes (halo 32, FILTER_HALO_
# NATIVE), so the forced plan's fresh demand is ~624 MB - far above the
# 256 MiB cap. The ladder ADMITS a forced plan regardless of its budget
# (the -tile diagnostic must stay usable on small images), so the draw
# pushes ~624 MB through the LRU: after ~14.5k admissions the cache is
# full and every further tile must evict - evictions>0, while the LRU
# keeps resident (hence gpu/peak_gpu) <= cap. A gen-churn variant (-tile 8
# + rotate, 2 x 175.7 MB) measured evictions=0: the generation change
# purges dead entries without counting capacity evictions, so the
# over-cap forced frame is the honest pressure.
# ---------------------------------------------------------------------------
$p9 = Run-Scene (Scene-Ini 'd2d' ($TallW + 40) ($TallH + 160) $true) $grad900 's9-pressure.png' 's9-pressure.err' '-tile 4' $TallW $TallH $null 12000
Note-D2dErr $p9.Err 'S9-pressure'
$st9 = Parse-Stats $p9.Err 'S9-pressure'
Check 'S9 pressure runs clean (adopted, exit 0, dump exists)' (($p9.Code -eq 0) -and $p9.Adopted -and (Test-Path $p9.Out)) ("exit=$($p9.Code) adopted=$($p9.Adopted) stderr=[$($p9.Err.Trim())]")
Check 'S9a pressure: evictions>0 and uploads>0 (33750 tiles x 18496B = ~624MB forced demand vs 268435456 cap)' (($st9 -ne $null) -and ($st9.Evictions -gt 0) -and ($st9.Uploads -gt 0)) (Stats-Detail $st9)
Check 'S9b pressure: gpu<=cap and peak_gpu<=cap under eviction pressure' (($st9 -ne $null) -and ($st9.Gpu -le $st9.Cap) -and ($st9.PeakGpu -le $st9.Cap)) (Stats-Detail $st9)
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S7 (R3 P2-6): evidence purity. Through #89 the D2D dump's failure arm
# silently fell back to the GDI channel ("riviv: d2d dump failed (...);
# trying the gdi channel") - had that fired, every dump assertion above
# would have been GDI evidence wearing a D2D label. #90 deleted the arm,
# so the string can never legitimately reappear - this stays as a
# tripwire. Assert no captured d2d/warp stderr contains it. Runs last so
# it covers S8/S9 too.
# ---------------------------------------------------------------------------
$fallback7 = New-Object System.Collections.Generic.List[string]
foreach ($de in $script:D2dErrs) {
    if (($de.Err -ne $null) -and $de.Err.Contains('trying the gdi channel')) {
        [void]$fallback7.Add($de.Tag)
    }
}
Check 'S7 no D2D scenario fell back to the gdi dump channel ("trying the gdi channel" absent everywhere)' (($fallback7.Count -eq 0) -and ($script:D2dErrs.Count -ge 12)) ("d2dStderrs=$($script:D2dErrs.Count) fallbacks=[$($fallback7 -join ',')]")

# ---------------------------------------------------------------------------
# S5c (relocated): the budget sweep over EVERY stats line captured anywhere
# in the run - S1, S2b, S2b-d, S3, S4, S4g/S4h, S5, S8, S9 - must satisfy
# gpu<=cap and peak_gpu<=cap. 12 lines are expected (the untiled runs print
# none); fewer means a tiled/mip scenario lost its evidence channel.
# ---------------------------------------------------------------------------
$viol5 = New-Object System.Collections.Generic.List[string]
foreach ($s in $script:StatsSeen) {
    if (($s.Gpu -gt $s.Cap) -or ($s.PeakGpu -gt $s.Cap)) {
        [void]$viol5.Add(($s.Tag + ': gpu=' + $s.Gpu + ' peak_gpu=' + $s.PeakGpu + ' cap=' + $s.Cap))
    }
}
$lines5 = ''
foreach ($s in $script:StatsSeen) { $lines5 += ('    ' + $s.Tag + ': ' + (Stats-Detail $s) + "`r`n") }
Write-Host ("  S5 evidence - every stats line captured this run ($($script:StatsSeen.Count)):`r`n$lines5")
Check 'S5c EVERY captured stats line satisfies gpu<=cap and peak_gpu<=cap (>=12 lines seen)' (($viol5.Count -eq 0) -and ($script:StatsSeen.Count -ge 12)) ("lines=$($script:StatsSeen.Count) violations=[$($viol5 -join '; ')]")

# ---------------------------------------------------------------------------
# Teardown: the staged ini must never outlive the run; keep the stage dir
# only when something failed (dump PNGs + stderr captures are evidence).
# ---------------------------------------------------------------------------
$iniCleaned = -not (Test-Path $Ini)
$leftover = Get-Process riviv -ErrorAction SilentlyContinue
if ($leftover) { $leftover | Stop-Process -Force }
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage }
else { Write-Output ('FAILURES: evidence kept in ' + $Stage) }
Write-Output ('SMOKE82 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip + ' iniCleaned=' + $iniCleaned)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
