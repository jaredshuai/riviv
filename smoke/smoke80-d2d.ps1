# smoke80 - #80 D2D renderer stack + the -dump-viewport readback channel.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Staged-ini
# discipline (smoke78/79 convention): every instance runs from a copy of the
# exe in %TEMP%\riviv-80-smoke; the staged ini is deleted before every
# launch (WM_CLOSE saves config - a leftover renderer key would fake
# cross-scenario regressions). Close is always WM_CLOSE (PostMessage to the
# owner): taskkill would skip the WM_CLOSE dump entirely.
#
# The probe must be SetProcessDPIAware (200% dev machine: an unaware reader
# gets DPI-virtualized geometry, diag79 finding).
#
# Waits are poll-based (synthetic-message latency is unbounded under load):
# window/title/file/resize/exit polls with caps. Only intra-scenario command
# settle uses short sleeps (WM_COMMAND and WM_CLOSE ride the same queue in
# FIFO order; the sleeps are belt-and-braces).
#
# Scenarios (S9 manual items are listed in the smoke report, not here):
#   S1 renderer key: gdi/d2d/warp/auto/frobnicate/missing -> the stderr
#      breadcrumb "riviv: renderer=<req> backend=<eff>".
#   S2 dump channel sanity (warp): adopted image + WM_CLOSE -> PNG on disk,
#      viewport-sized, exit code 0.
#   S3 L0 byte-exactness (core): the same 1:1 scene dumped through warp and
#      gdi -> identical file bytes; the non-white bbox is the source rect at
#      source size; every bbox pixel equals the source pixels.
#   S4 resize chain (warp): window resized -> dump follows the new viewport
#      size while the 1:1 bbox stays the source size.
#   S5 giant-image gate: a 16385-wide frame -> stderr gate line after the
#      d2d/warp startup breadcrumb, process alive, dump still succeeds
#      (GDI fallback channel).
#   S6 animation re-upload: 2-frame GIF, AnimationFrameStep between two
#      dump instances -> dump content follows the frame.
#   S7 rotation re-upload: EditRotate90 between two dump instances -> dump2
#      is dump1's content rotated 90 degrees clockwise.
#   S8 background display gate (warp vs gdi): an instance started
#      minimized (-WindowStyle Minimized; Windows activates the iconic
#      window - recorded) and un-minimized with SW_SHOWNOACTIVATE dumps
#      the image fine under warp: the D2D dump renders from the CPU
#      master inside the dump call itself (no Present, no WM_PAINT
#      dependency). The gdi twin is SKIP by design: per the #80 design
#      note the GDI render evidence rides the pre-existing background-
#      paint gate (master-identical, smoke78 SKIP semantics), so it is
#      not a #80 assertion; we still record what it did.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.IO.Compression;
using System.Runtime.InteropServices;
public class S80 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L; public int T; public int R; public int B; }

    // Hand-written PNG (32bpp RGBA, filter 0 rows, zlib/deflate IDAT) for
    // giant widths GDI+ refuses to create (GDI+ Save throws past 65535).
    public static void WriteWidePng(string path, int w, int h, byte r, byte g, byte b) {
        byte[] raw = new byte[(w * 4 + 1) * h];
        for (int y = 0; y < h; y++) {
            raw[y * (w * 4 + 1)] = 0;
            int o = y * (w * 4 + 1) + 1;
            for (int x = 0; x < w; x++) { raw[o + x * 4] = r; raw[o + x * 4 + 1] = g; raw[o + x * 4 + 2] = b; raw[o + x * 4 + 3] = 255; }
        }
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe(ihdr, 0, w); PutBe(ihdr, 4, h);
        ihdr[8] = 8; ihdr[9] = 6;
        Chunk(ms, "IHDR", ihdr);
        MemoryStream comp = new MemoryStream();
        comp.WriteByte(0x78); comp.WriteByte(0x01);
        DeflateStream ds = new DeflateStream(comp, CompressionMode.Compress, true);
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

    // Hand-written 2-frame animated GIF (GIF89a, 4-color global palette:
    // red, blue, white, black; two solid frames). The frames use the
    // standard uncompressed-LZW recipe: 3-bit codes, a CLEAR every two
    // pixel codes so the code width never grows. Delay is in 1/100 s - it
    // is set large (>= 5 s) so the animation cannot auto-advance between
    // the scripted commands and the WM_CLOSE dumps. GDI+ is avoided on
    // purpose: this .NET's Encoder class lacks FrameDelay, and encoder
    // parameter quirks would make the fixture nondeterministic.
    public static void WriteGif2(string path, int w, int h, int delayCs, byte r1, byte g1, byte b1, byte r2, byte g2, byte b2) {
        MemoryStream ms = new MemoryStream();
        byte[] head = { (byte)'G', (byte)'I', (byte)'F', (byte)'8', (byte)'9', (byte)'a' };
        ms.Write(head, 0, 6);
        Put16(ms, w); Put16(ms, h);
        ms.WriteByte(0xF1); // GCT present, 4 entries
        ms.WriteByte(0x00);
        ms.WriteByte(0x00);
        ms.WriteByte(r1); ms.WriteByte(g1); ms.WriteByte(b1);
        ms.WriteByte(r2); ms.WriteByte(g2); ms.WriteByte(b2);
        ms.WriteByte(255); ms.WriteByte(255); ms.WriteByte(255);
        ms.WriteByte(0); ms.WriteByte(0); ms.WriteByte(0);
        GifFrame(ms, w, h, 0, delayCs);
        GifFrame(ms, w, h, 1, delayCs);
        ms.WriteByte(0x3B);
        File.WriteAllBytes(path, ms.ToArray());
    }
    static void Put16(MemoryStream ms, int v) { ms.WriteByte((byte)(v & 0xFF)); ms.WriteByte((byte)((v >> 8) & 0xFF)); }
    static void GifFrame(MemoryStream ms, int w, int h, int idx, int delayCs) {
        ms.WriteByte(0x21); ms.WriteByte(0xF9); ms.WriteByte(0x04);
        ms.WriteByte(0x00);
        ms.WriteByte((byte)(delayCs & 0xFF)); ms.WriteByte((byte)((delayCs >> 8) & 0xFF));
        ms.WriteByte(0x00);
        ms.WriteByte(0x00);
        ms.WriteByte(0x2C);
        Put16(ms, 0); Put16(ms, 0); Put16(ms, w); Put16(ms, h);
        ms.WriteByte(0x00);
        ms.WriteByte(0x02);
        // Codes: CLEAR, then (pixel, pixel, CLEAR) repeating, ending with
        // (pixel, pixel, EOI). The decoder adds one table entry per pair
        // (the first code after CLEAR adds none), so next_code never
        // reaches 8 and every code stays 3 bits.
        List<byte> bits = new List<byte>();
        uint acc = 0; int nb = 0;
        Emit(bits, ref acc, ref nb, 4);
        long n = (long)w * h;
        for (long i = 0; i < n; i++) {
            Emit(bits, ref acc, ref nb, idx);
            if ((i % 2) == 1 && i + 1 < n) Emit(bits, ref acc, ref nb, 4);
        }
        Emit(bits, ref acc, ref nb, 5);
        while (nb > 0) { bits.Add((byte)(acc & 0xFF)); acc >>= 8; nb -= 8; }
        byte[] data = bits.ToArray();
        for (int o = 0; o < data.Length; o += 255) {
            int len = Math.Min(255, data.Length - o);
            ms.WriteByte((byte)len);
            ms.Write(data, o, len);
        }
        ms.WriteByte(0x00);
    }
    static void Emit(List<byte> bits, ref uint acc, ref int nb, int code) {
        acc |= (uint)(code << nb);
        nb += 3;
        while (nb >= 8) { bits.Add((byte)(acc & 0xFF)); acc >>= 8; nb -= 8; }
    }
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
    // Four quadrant colors inside a 1px red border: TL green, TR blue,
    // BL yellow, BR magenta. Nothing white anywhere, so the dump's
    // non-white bounding box is exactly the image rect.
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
    // Left half red, right half blue: the 90-degree-rotation probe.
    public static byte[] LeftRight(int w, int h, int lr, int lg, int lb, int rr, int rg, int rb) {
        byte[] a = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            if (x < w / 2) { a[i] = (byte)lr; a[i + 1] = (byte)lg; a[i + 2] = (byte)lb; }
            else { a[i] = (byte)rr; a[i + 1] = (byte)rg; a[i + 2] = (byte)rb; }
            a[i + 3] = 255;
        }
        return a;
    }
    public static byte[] Solid(int w, int h, int r, int g, int b) {
        byte[] a = new byte[w * h * 4];
        for (int i = 0; i < a.Length; i += 4) { a[i] = (byte)r; a[i + 1] = (byte)g; a[i + 2] = (byte)b; a[i + 3] = 255; }
        return a;
    }
    // Bounding box of every non-white pixel: {l, t, r, b}, or all -1.
    public static int[] BBox(byte[] bgra, int w, int h) {
        int l = w, t = h, r = -1, b = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            if (bgra[i] != 255 || bgra[i + 1] != 255 || bgra[i + 2] != 255) {
                if (x < l) l = x; if (x > r) r = x; if (y < t) t = y; if (y > b) b = y;
            }
        }
        return new int[] { l, t, r, b };
    }
    // Compare a sw x sh region of a locked BGRA dump against the expected
    // RGBA source. Returns null on equality, else the first mismatch.
    public static string CompareRegion(byte[] bgra, int bw, int l, int t, byte[] want, int sw, int tol) {
        int sh = want.Length / 4 / sw;
        for (int y = 0; y < sh; y++) for (int x = 0; x < sw; x++) {
            int di = ((t + y) * bw + (l + x)) * 4;
            int si = (y * sw + x) * 4;
            if (Math.Abs(bgra[di + 2] - want[si]) > tol || Math.Abs(bgra[di + 1] - want[si + 1]) > tol ||
                Math.Abs(bgra[di] - want[si + 2]) > tol || Math.Abs(bgra[di + 3] - want[si + 3]) > tol)
                return String.Format("({0},{1}) got rgba=({2},{3},{4},{5}) want=({6},{7},{8},{9})",
                    x, y, bgra[di + 2], bgra[di + 1], bgra[di], bgra[di + 3],
                    want[si], want[si + 1], want[si + 2], want[si + 3]);
        }
        return null;
    }
    public static bool BytesEqual(byte[] a, byte[] b) {
        if (a == null || b == null || a.Length != b.Length) return false;
        for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) return false;
        return true;
    }
    // Count pixels whose ALPHA byte differs (S3b's diagnosis: RGB may match
    // exactly while the letterbox alpha differs between the dump arms).
    public static int AlphaDiff(byte[] a, byte[] b) {
        if (a == null || b == null) return -1;
        int n = Math.Min(a.Length, b.Length) / 4, diff = 0;
        for (int i = 0; i < n; i++) if (a[i * 4 + 3] != b[i * 4 + 3]) diff++;
        return diff;
    }
    // Clockwise rotation of an RGBA buffer: (w, h) in, (h, w) out.
    public static byte[] RotateCw(byte[] rgba, int w, int h) {
        byte[] o = new byte[rgba.Length];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int si = (y * w + x) * 4, di = (x * h + (h - 1 - y)) * 4;
            o[di] = rgba[si]; o[di + 1] = rgba[si + 1]; o[di + 2] = rgba[si + 2]; o[di + 3] = rgba[si + 3];
        }
        return o;
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

$Stage = Join-Path $env:TEMP 'riviv-80-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
$RunExe = Join-Path $Stage 'riviv.exe'
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied' (Test-Path $RunExe) 'Copy-Item failed'

function Kill-Riviv {
    $ps = Get-Process riviv -ErrorAction SilentlyContinue
    if ($ps) { $ps | Stop-Process -Force; Start-Sleep -Milliseconds 300 }
}
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

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$CMD_ONE2ONE = 45      # menu.rs Cmd::ViewOneToOne.id()
$CMD_ROTATE90 = 23     # menu.rs Cmd::EditRotate90.id()
$CMD_FRAMESTEP = 100   # menu.rs Cmd::AnimationFrameStep.id()

Kill-Riviv

# ---------------------------------------------------------------------------
# S1: the renderer ini key -> the always-on stderr breadcrumb.
# ---------------------------------------------------------------------------
$bc = 'riviv: renderer='
$s1vals = @(
    @('gdi', 'riviv: renderer=gdi backend=gdi'),
    @('d2d', 'riviv: renderer=d2d backend=d2d/hw'),
    @('warp', 'riviv: renderer=warp backend=d2d/warp'),
    @('auto', 'riviv: renderer=auto backend=d2d/hw')
)
foreach ($c in $s1vals) {
    $val = $c[0]; $want = $c[1]
    Reset-Ini ("[riviv]`r`nrenderer={0}`r`n" -f $val)
    $p = Start-Riv '' ("s1-$val.err") $false
    $main = Wait-Main $p
    $winOk = ($main -ne [IntPtr]::Zero)
    $code = Close-Main $p $main
    $err = Read-Err ("s1-$val.err")
    $hasBc = ($null -ne $err) -and $err.Contains($want)
    Check ("S1a-$val breadcrumb '$want'") ($winOk -and ($code -eq 0) -and $hasBc) ("exit=$code win=$winOk stderr=[$($err.Trim())]")
}
Reset-Ini "[riviv]`r`nrenderer=frobnicate`r`n"
$p = Start-Riv '' 's1-frob.err' $false
$main = Wait-Main $p
$code = Close-Main $p $main
$err = Read-Err 's1-frob.err'
Check 'S1b frobnicate falls back to gdi + unrecognized hint' (($code -eq 0) -and $err.Contains('riviv: renderer=gdi backend=gdi') -and $err.Contains('unrecognized renderer value')) ("exit=$code stderr=[$($err.Trim())]")
Reset-Ini "[riviv]`r`nx=60`r`ny=60`r`nwide=1000`r`nhigh=700`r`n"
$p = Start-Riv '' 's1-missing.err' $false
$main = Wait-Main $p
$code = Close-Main $p $main
$err = Read-Err 's1-missing.err'
Check 'S1c missing key defaults to gdi' (($code -eq 0) -and $err.Contains('riviv: renderer=gdi backend=gdi') -and (-not $err.Contains('unrecognized'))) ("exit=$code stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2: the warp dump channel end to end (adopted image -> WM_CLOSE -> PNG).
# ---------------------------------------------------------------------------
$red64 = Join-Path $Stage 'red64.png'
[Px]::SaveRgba($red64, [Px]::Solid(64, 64, 255, 0, 0), 64, 64)
$baseIni = "[riviv]`r`nx=60`r`ny=60`r`nwide=1000`r`nhigh=700`r`nauto_zoom=0`r`nicm=0`r`n"
Reset-Ini ($baseIni + "renderer=warp`r`n")
$out2 = Join-Path $Stage 's2-out.png'
if (Test-Path $out2) { Remove-Item $out2 -Force }
$p = Start-Riv ("`"$red64`" -dump-viewport `"$out2`"") 's2.err' $false
$main = Wait-Main $p
$adopted = Wait-Title $p 'red64' 10000
Start-Sleep -Milliseconds 700
$view = View-Of $main
$vs2 = View-Size $view
$code = Close-Main $p $main
$pngOk = Test-Path $out2
$dims2 = '(none)'
$sizeOk = $false
if ($pngOk) {
    $q2 = [Px]::Load($out2)
    $dims2 = "$($q2.W)x$($q2.H)"
    $sizeOk = ($q2.W -eq $vs2[0]) -and ($q2.H -eq $vs2[1])
}
Check 'S2 warp dump: adopted, exit 0, viewport-sized PNG' (($main -ne [IntPtr]::Zero) -and $adopted -and ($code -eq 0) -and $pngOk -and $sizeOk) ("adopted=$adopted exit=$code png=$pngOk dims=$dims2 viewport=$($vs2[0])x$($vs2[1])")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3 (core): the 1:1 scene dumped through warp and gdi is byte-identical
# and pixel-exact against the source.
# ---------------------------------------------------------------------------
$SRCW = 300
$SRCH = 200
$src3 = [Px]::Source($SRCW, $SRCH)
$png3 = Join-Path $Stage 'quad.png'
[Px]::SaveRgba($png3, $src3, $SRCW, $SRCH)

function Run-Dump($renderer, $img, $outName, $errName, $cmds) {
    Reset-Ini ($baseIni + "renderer=$renderer`r`n")
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    $p = Start-Riv ("`"$img`" -dump-viewport `"$out`"") $errName $false
    $main = Wait-Main $p
    $adopted = Wait-Title $p ([IO.Path]::GetFileNameWithoutExtension($img)) 12000
    Start-Sleep -Milliseconds 400
    if ($cmds) { & $cmds $main }
    Start-Sleep -Milliseconds 400
    $view = View-Of $main
    $vs = View-Size $view
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Vs = $vs; Adopted = $adopted; Main = $main; Err = (Read-Err $errName) }
}
$one2one = { param($m) [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }
$w3 = Run-Dump 'warp' $png3 's3-warp.png' 's3-warp.err' $one2one
$g3 = Run-Dump 'gdi' $png3 's3-gdi.png' 's3-gdi.err' $one2one
$bothOk = ($w3.Code -eq 0) -and ($g3.Code -eq 0) -and (Test-Path $w3.Out) -and (Test-Path $g3.Out)
Check 'S3a both dumps ran clean (exit 0, files exist)' $bothOk ("warp exit=$($w3.Code) gdi exit=$($g3.Code) warpPng=$(Test-Path $w3.Out) gdiPng=$(Test-Path $g3.Out)")
if ($bothOk) {
    $bw3 = [IO.File]::ReadAllBytes($w3.Out)
    $bg3 = [IO.File]::ReadAllBytes($g3.Out)
    $eq3 = [Px]::BytesEqual($bw3, $bg3)
    $alphaNote = ''
    if (-not $eq3) {
        $qa3 = [Px]::Load($w3.Out)
        $qb3 = [Px]::Load($g3.Out)
        $alphaNote = (' alphaDiffPixels=' + [Px]::AlphaDiff($qa3.B, $qb3.B) + ' (RGB compared below; an alpha-only letterbox difference means the GDI dump leaves the DIB-zeroed alpha 0 while the D2D Clear writes 255 - adjudication finding)')
    }
    Check 'S3b warp and gdi dumps byte-identical' $eq3 ("bytes $($bw3.Length) vs $($bg3.Length)" + $alphaNote)
    $q3 = [Px]::Load($w3.Out)
    $sizeOk3 = ($q3.W -eq $w3.Vs[0]) -and ($q3.H -eq $w3.Vs[1])
    Check 'S3c warp dump is viewport-sized' $sizeOk3 ("dump=$($q3.W)x$($q3.H) viewport=$($w3.Vs[0])x$($w3.Vs[1])")
    $bb3 = [Px]::BBox($q3.B, $q3.W, $q3.H)
    $bwid3 = $bb3[2] - $bb3[0] + 1
    $bhei3 = $bb3[3] - $bb3[1] + 1
    $bboxOk3 = ($bwid3 -eq $SRCW) -and ($bhei3 -eq $SRCH)
    Check 'S3d non-white bbox == source rect at source size' $bboxOk3 ("bbox l=$($bb3[0]) t=$($bb3[1]) ${bwid3}x${bhei3} want=${SRCW}x${SRCH}")
    $mism3 = '(bbox failed)'
    if ($bboxOk3) { $mism3 = [Px]::CompareRegion($q3.B, $q3.W, $bb3[0], $bb3[1], $src3, $SRCW, 0) }
    Check 'S3e every warp bbox pixel == source (RGBA, exact)' ($mism3 -eq $null) "$mism3"
    $qg3 = [Px]::Load($g3.Out)
    $bbg3 = [Px]::BBox($qg3.B, $qg3.W, $qg3.H)
    $gwid3 = $bbg3[2] - $bbg3[0] + 1
    $ghei3 = $bbg3[3] - $bbg3[1] + 1
    $gOk3 = (($gwid3 -eq $SRCW) -and ($ghei3 -eq $SRCH))
    $gm3 = '(bbox failed)'
    if ($gOk3) { $gm3 = [Px]::CompareRegion($qg3.B, $qg3.W, $bbg3[0], $bbg3[1], $src3, $SRCW, 0) }
    Check 'S3f gdi dump pixel-exact too (both arms, not just one)' (($gOk3 -and ($gm3 -eq $null))) "bbox=$gwid3 x $ghei3 first-mismatch=$gm3"
} else {
    Skip-Scenario 'S3b-f pixel comparisons' 'a dump channel failed; see S3a detail and stderr captures'
    Write-Output ("S3 warp stderr: " + ($w3.Err.Trim()))
    Write-Output ("S3 gdi stderr: " + ($g3.Err.Trim()))
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: the resize chain - dump follows the new viewport, the 1:1 bbox stays.
# ---------------------------------------------------------------------------
Reset-Ini ($baseIni + "renderer=warp`r`n")
$out4 = Join-Path $Stage 's4-out.png'
if (Test-Path $out4) { Remove-Item $out4 -Force }
$p = Start-Riv ("`"$png3`" -dump-viewport `"$out4`"") 's4.err' $false
$main = Wait-Main $p
$adopted4 = Wait-Title $p 'quad' 12000
& $one2one $main
Start-Sleep -Milliseconds 400
$view = View-Of $main
$vs4a = View-Size $view
# Resize the window (SWP_NOMOVE|SWP_NOZORDER); the dock chain resizes the
# view child, its WM_SIZE runs the swapchain resize, the dump rides WM_CLOSE.
[void][S80]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, 1500, 900, 0x0006)
$resized4 = Wait-Until { ((View-Size $view)[0] -ne $vs4a[0]) -and ((View-Size $view)[1] -ne $vs4a[1]) } 8000
Start-Sleep -Milliseconds 500
$vs4b = View-Size $view
$code4 = Close-Main $p $main
$pngOk4 = Test-Path $out4
$dims4 = '(none)'
$sizeOk4 = $false
if ($pngOk4) {
    $q4 = [Px]::Load($out4)
    $dims4 = "$($q4.W)x$($q4.H)"
    $sizeOk4 = ($q4.W -eq $vs4b[0]) -and ($q4.H -eq $vs4b[1])
}
$bb4 = $null; $bboxOk4 = $false; $mism4 = '(png missing)'
if ($pngOk4) {
    $bb4 = [Px]::BBox($q4.B, $q4.W, $q4.H)
    $bboxOk4 = (($bb4[2] - $bb4[0] + 1) -eq $SRCW) -and (($bb4[3] - $bb4[1] + 1) -eq $SRCH)
    $mism4 = [Px]::CompareRegion($q4.B, $q4.W, $bb4[0], $bb4[1], $src3, $SRCW, 0)
}
Check 'S4a window resize reached the view child' ($resized4 -and ($vs4b[0] -ne $vs4a[0])) ("view $($vs4a[0])x$($vs4a[1]) -> $($vs4b[0])x$($vs4b[1])")
Check 'S4b dump is the NEW viewport size (swapchain resized)' ($adopted4 -and ($code4 -eq 0) -and $pngOk4 -and $sizeOk4) ("adopted=$adopted4 exit=$code4 dump=$dims4 want=$($vs4b[0])x$($vs4b[1])")
Check 'S4c content sane: 1:1 bbox still the source size, pixels exact' ($bboxOk4 -and ($mism4 -eq $null)) "bboxOk=$bboxOk4 first-mismatch=$mism4"
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S5: the giant-image gate. A frame wider than the device's runtime
# GetMaximumBitmapSize must trip the gate (stderr "exceeds the D2D max
# bitmap"), leave the process alive, and still dump through the GDI
# fallback with exit 0. The width is 2^24+1: it exceeds the D3D11/WARP
# texture cap (16384, the ticket's "typical") AND the larger WARP value
# (2^23) measured on this dev machine via the same stderr line - the gate
# threshold is a runtime query (design section 3-6), never a constant.
# GDI+ cannot create bitmaps past 65535, so the PNG is hand-written.
# ---------------------------------------------------------------------------
$GIANT_W = 16777217
$giant = Join-Path $Stage 'giant.png'
[S80]::WriteWidePng($giant, $GIANT_W, 1, 255, 0, 0)
Reset-Ini ($baseIni + "renderer=warp`r`n")
$out5 = Join-Path $Stage 's5-out.png'
if (Test-Path $out5) { Remove-Item $out5 -Force }
$p = Start-Riv ("`"$giant`" -dump-viewport `"$out5`"") 's5.err' $false
$main = Wait-Main $p
$adopted5 = Wait-Title $p 'giant' 30000
$gateSeen = Wait-Until { (Read-Err 's5.err') -match 'exceeds the D2D max bitmap' } 15000
Start-Sleep -Milliseconds 500
$alive5 = -not $p.HasExited
$view = View-Of $main
$vs5 = View-Size $view
$code5 = Close-Main $p $main
$err = Read-Err 's5.err'
$iBc = $err.IndexOf('renderer=warp backend=')
$iGate = $err.IndexOf('exceeds the D2D max bitmap')
$order5 = ($iBc -ge 0) -and ($iGate -ge 0) -and ($iBc -lt $iGate)
$maxLine = '(no gate line)'
if ($err -match 'exceeds the D2D max bitmap (\d+)') { $maxLine = "device max=" + $Matches[1] }
$pngOk5 = Test-Path $out5
$sizeOk5 = $false
$dims5 = '(none)'
if ($pngOk5) {
    $q5 = [Px]::Load($out5)
    $dims5 = "$($q5.W)x$($q5.H)"
    $sizeOk5 = ($q5.W -eq $vs5[0]) -and ($q5.H -eq $vs5[1])
}
Check 'S5a gate line present, after the d2d/warp breadcrumb' ($order5 -and $err.Contains('rendering it via gdi')) ("breadcrumbIdx=$iBc gateIdx=$iGate seen=$gateSeen $maxLine stderr=[$($err.Trim())]")
Check 'S5b process stayed alive with the giant frame' ($alive5 -and $adopted5) "alive=$alive5 adopted=$adopted5"
Check 'S5c dump still succeeds through the GDI channel (exit 0)' (($code5 -eq 0) -and $pngOk5 -and $sizeOk5) ("exit=$code5 png=$pngOk5 dims=$dims5 viewport=$($vs5[0])x$($vs5[1])")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S6: animation frame re-upload - the dump follows AnimationFrameStep.
# The fixture GIF is hand-built by [S80]::WriteGif2 (2 frames, 10 s delays):
# this .NET's GDI+ Encoder lacks FrameDelay, so the GDI+ multiframe route
# cannot set reliable delays.
# ---------------------------------------------------------------------------
$gif = Join-Path $Stage 'anim.gif'
[S80]::WriteGif2($gif, 64, 64, 1000, 255, 0, 0, 0, 0, 255)
$gifOk = $false
$gifDetail = 'unreadable'
$r6a = $null
$r6b = $null
try {
    $img = [Drawing.Image]::FromFile($gif)
    $n6 = $img.GetFrameCount([Drawing.Imaging.FrameDimension]::Time)
    $d6 = 0
    try {
        $pi = $img.GetPropertyItem(0x5100)
        $d6 = [BitConverter]::ToUInt32($pi.Value, 0)
    } catch { $d6 = -1 }
    [void]$img.SelectActiveFrame([Drawing.Imaging.FrameDimension]::Time, 0)
    $b1 = New-Object Drawing.Bitmap($img)
    $c1 = $b1.GetPixel(32, 32)
    [void]$img.SelectActiveFrame([Drawing.Imaging.FrameDimension]::Time, 1)
    $b2 = New-Object Drawing.Bitmap($img)
    $c2 = $b2.GetPixel(32, 32)
    $b1.Dispose(); $b2.Dispose(); $img.Dispose()
    $gifOk = ($n6 -eq 2) -and ($d6 -ge 500) -and ($c1.R -gt 200) -and ($c1.B -lt 60) -and ($c2.B -gt 200) -and ($c2.R -lt 60)
    $gifDetail = "frames=$n6 delayCs=$d6 f0=($($c1.R),$($c1.G),$($c1.B)) f1=($($c2.R),$($c2.G),$($c2.B))"
} catch { $gifDetail = "exception: $_" }
Check 'S6a gif fixture: 2 frames, >=5s delays, red then blue' $gifOk $gifDetail
$d1p = Join-Path $Stage 's6-f0.png'
$d2p = Join-Path $Stage 's6-f1.png'
$both6 = $false
if ($gifOk) {
    $r6a = Run-Dump 'warp' $gif 's6-f0.png' 's6-a.err' $null
    $r6b = Run-Dump 'warp' $gif 's6-f1.png' 's6-b.err' { param($m) [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_FRAMESTEP, [IntPtr]::Zero) }
    $both6 = ($r6a.Code -eq 0) -and ($r6b.Code -eq 0) -and (Test-Path $d1p) -and (Test-Path $d2p)
    if (-not $both6) {
        Write-Output ("S6a stderr: " + ($r6a.Err.Trim()))
        Write-Output ("S6b stderr: " + ($r6b.Err.Trim()))
    }
}
if ($both6) {
    $qa = [Px]::Load($d1p)
    $qb = [Px]::Load($d2p)
    $diff6 = -not [Px]::BytesEqual([IO.File]::ReadAllBytes($d1p), [IO.File]::ReadAllBytes($d2p))
    $bba = [Px]::BBox($qa.B, $qa.W, $qa.H)
    $bbb = [Px]::BBox($qb.B, $qb.W, $qb.H)
    $bbox6 = ((($bba[2] - $bba[0] + 1) -eq 64) -and (($bba[3] - $bba[1] + 1) -eq 64) -and
        (($bbb[2] - $bbb[0] + 1) -eq 64) -and (($bbb[3] - $bbb[1] + 1) -eq 64))
    $mA = '(bbox failed)'; $mB = '(bbox failed)'
    if ($bbox6) {
        $mA = [Px]::CompareRegion($qa.B, $qa.W, $bba[0], $bba[1], [Px]::Solid(64, 64, 255, 0, 0), 64, 2)
        $mB = [Px]::CompareRegion($qb.B, $qb.W, $bbb[0], $bbb[1], [Px]::Solid(64, 64, 0, 0, 255), 64, 2)
    }
    Check 'S6b the two dumps differ (frame_gen re-upload visible)' $diff6 'dump1 == dump2 bytes'
    Check 'S6c dump1 shows frame 0 (red) at 1:1' ($bbox6 -and ($mA -eq $null)) "bboxOk=$bbox6 first-mismatch=$mA"
    Check 'S6d dump2 shows frame 1 (blue) after AnimationFrameStep' ($bbox6 -and ($mB -eq $null)) "bboxOk=$bbox6 first-mismatch=$mB"
} else {
    Skip-Scenario 'S6b/S6c/S6d animation dump comparisons' ("fixture or dump channel unavailable: $gifDetail")
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S7: rotation re-upload - the dump follows EditRotate90 (clockwise).
# ---------------------------------------------------------------------------
$src7 = [Px]::LeftRight(120, 80, 255, 0, 0, 0, 0, 255)
$png7 = Join-Path $Stage 'leftright.png'
[Px]::SaveRgba($png7, $src7, 120, 80)
$rot7 = [Px]::RotateCw($src7, 120, 80)
$r7a = Run-Dump 'warp' $png7 's7-r0.png' 's7-a.err' $one2one
$rotate = { param($m)
    [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [void][S80]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ROTATE90, [IntPtr]::Zero)
}
$r7b = Run-Dump 'warp' $png7 's7-r1.png' 's7-b.err' $rotate
$both7 = ($r7a.Code -eq 0) -and ($r7b.Code -eq 0) -and (Test-Path $r7a.Out) -and (Test-Path $r7b.Out)
if ($both7) {
    $qa7 = [Px]::Load($r7a.Out)
    $qb7 = [Px]::Load($r7b.Out)
    $bba7 = [Px]::BBox($qa7.B, $qa7.W, $qa7.H)
    $bbb7 = [Px]::BBox($qb7.B, $qb7.W, $qb7.H)
    $w1 = $bba7[2] - $bba7[0] + 1; $h1 = $bba7[3] - $bba7[1] + 1
    $w2 = $bbb7[2] - $bbb7[0] + 1; $h2 = $bbb7[3] - $bbb7[1] + 1
    $swap7 = (($w1 -eq 120) -and ($h1 -eq 80) -and ($w2 -eq 80) -and ($h2 -eq 120))
    $m7a = '(bbox failed)'; $m7b = '(bbox failed)'
    if ($swap7) {
        $m7a = [Px]::CompareRegion($qa7.B, $qa7.W, $bba7[0], $bba7[1], $src7, 120, 0)
        $m7b = [Px]::CompareRegion($qb7.B, $qb7.W, $bbb7[0], $bbb7[1], $rot7, 80, 0)
    }
    Check 'S7a dump1 is the 120x80 source, pixels exact' (($w1 -eq 120) -and ($h1 -eq 80) -and ($m7a -eq $null)) "bbox=${w1}x${h1} first-mismatch=$m7a"
    Check 'S7b dump2 bbox swapped (80x120) and content == source rotated 90 CW' ($swap7 -and ($m7b -eq $null)) "bbox=${w2}x${h2} first-mismatch=$m7b"
} else {
    Check 'S7a/S7b rotation dumps ran clean' $false ("a exit=$($r7a.Code) b exit=$($r7b.Code) pngs=$(Test-Path $r7a.Out)/$(Test-Path $r7b.Out)")
    Write-Output ("S7a stderr: " + ($r7a.Err.Trim()))
    Write-Output ("S7b stderr: " + ($r7b.Err.Trim()))
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S8: the background display gate. Warp: a never-foregrounded instance dumps
# at WM_CLOSE (the D2D dump renders from the CPU master, no Present, no
# WM_PAINT dependency). Gdi twin: SKIP per the #80 design note (the GDI
# render evidence rides the pre-existing background-paint gate,
# master-identical, smoke78 SKIP semantics) - we still record what it did.
# ---------------------------------------------------------------------------
function Run-Background($renderer, $outName, $errName) {
    Reset-Ini ($baseIni + "renderer=$renderer`r`n")
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    $p = Start-Riv ("`"$red64`" -dump-viewport `"$out`"") $errName $true
    $main = Wait-Main $p
    if ($main -eq [IntPtr]::Zero) { return @{ Out = $out; Code = -2; Note = 'no window' } }
    $adopted = Wait-Title $p 'red64' 12000
    $iconic0 = [S80]::IsIconic($main)
    $vs0 = View-Size (View-Of $main)
    if (($vs0[0] -le 0) -or ($vs0[1] -le 0)) {
        # Iconic viewport is 0x0 (the design's documented dump-refusal shape):
        # un-minimize WITHOUT activating so geometry exists but the process
        # still never owned the foreground. The predicate reads the script
        # scope: a scriptblock invoked from Wait-Until cannot see this
        # function's locals (dynamic scoping).
        $script:main = $main
        [void][S80]::ShowWindow($main, 4)  # SW_SHOWNOACTIVATE
        $null = Wait-Until { -not [S80]::IsIconic($script:main) } 5000
        Start-Sleep -Milliseconds 1000
    }
    $fg = [S80]::GetForegroundWindow()
    $neverFg = ($fg -ne $main)
    $view = View-Of $main
    $vs = View-Size $view
    $code = Close-Main $p $main
    $pngOk = Test-Path $out
    $dims = '(none)'
    $sizeOk = $false
    $contentOk = $false
    if ($pngOk) {
        $q = [Px]::Load($out)
        $dims = "$($q.W)x$($q.H)"
        $sizeOk = ($q.W -eq $vs[0]) -and ($q.H -eq $vs[1])
        $bb = [Px]::BBox($q.B, $q.W, $q.H)
        $contentOk = ((($bb[2] - $bb[0] + 1) -eq 64) -and (($bb[3] - $bb[1] + 1) -eq 64))
        if ($contentOk) { $contentOk = ($null -eq [Px]::CompareRegion($q.B, $q.W, $bb[0], $bb[1], [Px]::Solid(64, 64, 255, 0, 0), 64, 2)) }
    }
    return @{ Out = $out; Code = $code; Png = $pngOk; SizeOk = $sizeOk; ContentOk = $contentOk; Dims = $dims; Vs = $vs; Adopted = $adopted; Iconic0 = $iconic0; Vs0 = $vs0; NeverFg = $neverFg }
}
$r8w = Run-Background 'warp' 's8-warp.png' 's8-warp.err'
$note8w = "adopted=$($r8w.Adopted) iconicAtStart=$($r8w.Iconic0) viewWhileIconic=$($r8w.Vs0[0])x$($r8w.Vs0[1]) fgHeldByRiviv=$(-not $r8w.NeverFg) exit=$($r8w.Code) png=$($r8w.Png) dims=$($r8w.Dims) viewport=$($r8w.Vs[0])x$($r8w.Vs[1]) imageContentOk=$($r8w.ContentOk)"
Write-Output ('  S8a evidence: ' + $note8w)
Check 'S8a warp: minimized-start instance dumps the image at WM_CLOSE (dump path needs no display pipeline)' (($r8w.Code -eq 0) -and $r8w.Png -and $r8w.SizeOk -and $r8w.ContentOk) $note8w
$r8g = Run-Background 'gdi' 's8-gdi.png' 's8-gdi.err'
$note8g = "observed: adopted=$($r8g.Adopted) exit=$($r8g.Code) png=$($r8g.Png) dims=$($r8g.Dims) viewport=$($r8g.Vs[0])x$($r8g.Vs[1]) imageContentOk=$($r8g.ContentOk) fgHeldByRiviv=$(-not $r8g.NeverFg)"
Skip-Scenario 'S8b gdi twin (background gate; master-identical, not a #80 assertion)' $note8g
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# Teardown: the staged ini must never outlive the run; keep the stage dir
# only when something failed (dump PNGs + stderr captures are the evidence).
# ---------------------------------------------------------------------------
$iniCleaned = -not (Test-Path $Ini)
$leftover = Get-Process riviv -ErrorAction SilentlyContinue
if ($leftover) { $leftover | Stop-Process -Force }
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage }
else { Write-Output ("FAILURES: evidence kept in " + $Stage) }
Write-Output ('SMOKE80 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip + ' iniCleaned=' + $iniCleaned)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
