# smoke81 - #81 filter tier remap + mip retirement + renderer default flip.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Harness
# skeleton cribbed verbatim from smoke80-d2d.ps1 (staged exe+ini under
# %TEMP%\riviv-81-smoke, WM_CLOSE dump channel, DPI-aware probe, poll-based
# waits, GetExitCodeProcess on the captured live handle).
#
# Scenarios:
#   S1 renderer default flip: missing key -> "riviv: renderer=auto
#      backend=..."; frobnicate -> "unrecognized renderer value" + "using
#      auto" + the auto line; gdi -> the gdi escape hatch still works.
#   S2 L0/L1 integer magnify byte-exactness (core): 96x64 hash-pattern
#      source, fill_window=1 recipe. k=3,4 calibrate the riviv_view child
#      to EXACTLY k*96 x k*64 (fill renders edge to edge) -> warp (NEAREST)
#      vs gdi (COLORONCOLOR) file bytes AND decoded pixels identical, dump
#      exactly k*96 x k*64, pixel-exact vs the replication model
#      expected[x,y] = src[x/k, y/k], gdi arm frozen as a golden.
#      k=2 is special: the window's minimum track width (the toolbar strip,
#      ~256 px at this machine's 200% DPI) makes a 192-wide view
#      unreachable, so the k=2 target is the 256x128 clamped viewport
#      where fill_window=1 still renders EXACTLY 192x128 (2x) centered at
#      (32,0) with 32px magenta side margins - the magnify path is fully
#      engaged (this also serves as the letterbox boundary case; riviv
#      centers the render, so the ticket's "x=2*96 is bg" reads x=223
#      image / x=224 bg here).
#      Padded case: exact-2x-with-margins-on-both-axes is unreachable in
#      the zoom model (contain-fit binds one axis; no preset level lands
#      on 2x for a 96x64 source), so the padded geometry uses a 192x128
#      source that IS the exact 2x replication of the 96x64 pattern at the
#      default fit (fill_window=0) in a 256x158 viewport (the 232 target
#      clamps to the same min width) -> the same assertions (margins pure
#      magenta, image box boundary +-0px, warp==gdi, golden): box
#      32..223 x 15..142, side margins 32, top/bottom margins 15.
#   S3 L2 filter tiers: mag=1 on a 256x256 smooth gradient at exact 2x
#      (warp LINEAR vs gdi HALFTONE: per-channel MAE <= 2, max <= 12);
#      shrink=1 on 640x480 at exact 2x down (warp HIGH_QUALITY_CUBIC vs
#      gdi HALFTONE: genuinely different, max diff outside block halos
#      <= 40, no blown-out pixels); shrink=0 (warp NEAREST vs gdi
#      COLORONCOLOR: byte-equal OR each arm matches ONE uniform 2x phase
#      model of the source, census reported).
#   S4 giant correctness: 40000x256 banner (luminance gradient + a 4000px
#      2px-period stripe band at x=20000) at fit in a 1000x700 window ->
#      warp uploads 40000 (no D2D gate) and CUBIC-deep-shrinks it; gdi
#      takes the NEW full-res-face >=32768 clip-region branch; both must
#      keep the gradient monotone, the averaged stripes ~128 and the bbox
#      sane. 16777217x1 gradient giant via the hand-written PNG writer:
#      renderer=gdi content (1px strip, gradient survived) plus one warp
#      run asserting the gate line + the same content. Boundary census on
#      the #81 stitch trigger (source extent 2^22): 4000000x1 (below ->
#      single full-rect path must still render), 4194304x1 (= 2^22, the
#      first sliced width) and 8388608x1 (= 2^23, the previously-black
#      width), renderer=gdi, each with the extreme-giant content
#      assertions; the two sliced widths also get a seam scan over the
#      strip's row: max adjacent-column |delta mean| <= 3*median + 2.
#   S5 dump failure: -dump-viewport into a missing directory -> exit 2,
#      stderr says so, no modal hang.
#   S6 frame-time baseline (record-only): launch->exit wall time of the
#      dump run, 3 runs per scene per renderer, printed as a table.
#
# Golden corpus: smoke/golden81/ holds the frozen gdi-arm dumps (S2 exact
# k=2,3,4 + padded). -Regolden (re)creates them from this run; by default
# the dumps are compared against them; a missing golden is a SKIP-with-note.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe', [switch]$Regolden)
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
public class S81 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
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
        WritePngRaw(path, w, h, raw);
    }
    // Same skeleton, horizontal gradient payload: r = x*255/(w-1),
    // g = 255-r, b = 64. Fastest deflate: a gradient row is nearly
    // incompressible and Optimal buys nothing at 67 MB.
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
    static void WritePngRaw(string path, int w, int h, byte[] raw) {
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
    public static bool BytesEqual(byte[] a, byte[] b) {
        if (a == null || b == null || a.Length != b.Length) return false;
        for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) return false;
        return true;
    }
    public static int AlphaDiff(byte[] a, byte[] b) {
        if (a == null || b == null) return -1;
        int n = Math.Min(a.Length, b.Length) / 4, diff = 0;
        for (int i = 0; i < n; i++) if (a[i * 4 + 3] != b[i * 4 + 3]) diff++;
        return diff;
    }
    // ---- #81 helpers ----

    // Deterministic varied-color pattern with a 1px black border ring, so a
    // magnified draw's boundary is unambiguous and every source pixel is
    // distinguishable from its neighbors.
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
    // Integer-k replication: dest[x,y] = src[x/k, y/k] (the magnify model).
    public static byte[] UpsampleK(byte[] src, int sw, int sh, int k) {
        int dw = sw * k, dh = sh * k;
        byte[] o = new byte[dw * dh * 4];
        for (int y = 0; y < dh; y++) for (int x = 0; x < dw; x++) {
            int si = ((y / k) * sw + (x / k)) * 4, di = (y * dw + x) * 4;
            o[di] = src[si]; o[di + 1] = src[si + 1]; o[di + 2] = src[si + 2]; o[di + 3] = 255;
        }
        return o;
    }
    // First RGB mismatch of a locked BGRA buffer region vs an expected RGBA
    // buffer (alpha deliberately ignored: the GDI dump may leave the DIB's
    // zeroed alpha where the D2D Clear writes 255 - smoke80 S3b finding).
    public static string CompareRgb(byte[] bgra, int bw, int l, int t, byte[] want, int sw) {
        int sh = want.Length / 4 / sw;
        for (int y = 0; y < sh; y++) for (int x = 0; x < sw; x++) {
            int di = ((t + y) * bw + (l + x)) * 4, si = (y * sw + x) * 4;
            if (bgra[di + 2] != want[si] || bgra[di + 1] != want[si + 1] || bgra[di] != want[si + 2])
                return String.Format("({0},{1}) got rgb=({2},{3},{4}) want=({5},{6},{7})",
                    x, y, bgra[di + 2], bgra[di + 1], bgra[di], want[si], want[si + 1], want[si + 2]);
        }
        return null;
    }
    // First RGB mismatch between two same-dims BGRA buffers, or null.
    public static string RgbDiff(byte[] a, byte[] b) {
        if (a == null || b == null || a.Length != b.Length) return "buffer mismatch";
        for (int i = 0; i < a.Length; i += 4)
            if (a[i] != b[i] || a[i + 1] != b[i + 1] || a[i + 2] != b[i + 2])
                return String.Format("byte {0}: rgb=({1},{2},{3}) vs ({4},{5},{6})",
                    i, a[i + 2], a[i + 1], a[i], b[i + 2], b[i + 1], b[i]);
        return null;
    }
    // Per-channel mean abs diff + max any-channel diff of a BGRA region vs
    // an RGBA expectation: {maeR, maeG, maeB, maxAny}.
    public static double[] StatVs(byte[] bgra, int bw, int l, int t, int w, int h, byte[] want, int sw) {
        double mr = 0, mg = 0, mb = 0; int mx = 0; long n = (long)w * h;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = ((t + y) * bw + (l + x)) * 4, si = (y * sw + x) * 4;
            int dr = Math.Abs(bgra[di + 2] - want[si]), dg = Math.Abs(bgra[di + 1] - want[si + 1]), db = Math.Abs(bgra[di] - want[si + 2]);
            mr += dr; mg += dg; mb += db;
            int m = Math.Max(dr, Math.Max(dg, db)); if (m > mx) mx = m;
        }
        return new double[] { mr / n, mg / n, mb / n, mx };
    }
    static bool InExcl(int x, int y, int[] ex) {
        if (ex == null) return false;
        for (int i = 0; i + 3 < ex.Length; i += 4)
            if (x >= ex[i] && x < ex[i] + ex[i + 2] && y >= ex[i + 1] && y < ex[i + 1] + ex[i + 3]) return true;
        return false;
    }
    // Whole-buffer two-dump stats: {maeR, maeG, maeB, maxAny, maxOutsideExcl,
    // diffPixels}. excl = flattened {x,y,w,h} quads in buffer coords.
    public static double[] StatPair(byte[] a, byte[] b, int w, int h, int[] excl) {
        double mr = 0, mg = 0, mb = 0; int mx = 0, mxe = 0; long dp = 0;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = (y * w + x) * 4;
            int dr = Math.Abs(a[di + 2] - b[di + 2]), dg = Math.Abs(a[di + 1] - b[di + 1]), db = Math.Abs(a[di] - b[di]);
            mr += dr; mg += dg; mb += db;
            int m = Math.Max(dr, Math.Max(dg, db));
            if (m > mx) mx = m;
            if (m != 0) dp++;
            if (!InExcl(x, y, excl) && m > mxe) mxe = m;
        }
        long n = (long)w * h;
        return new double[] { mr / n, mg / n, mb / n, mx, mxe, (double)dp };
    }
    // Count pixels with any RGB channel outside [lo,hi], skipping excl rects.
    public static long CountOutside(byte[] bgra, int bw, int w, int h, int lo, int hi, int[] excl) {
        long n = 0;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            if (InExcl(x, y, excl)) continue;
            int di = (y * bw + x) * 4;
            if (bgra[di] < lo || bgra[di] > hi || bgra[di + 1] < lo || bgra[di + 1] > hi || bgra[di + 2] < lo || bgra[di + 2] > hi) n++;
        }
        return n;
    }
    // Count pixels in a region whose RGB differs from the given background.
    public static long CountNotBg(byte[] bgra, int bw, int l, int t, int w, int h, int r, int g, int b) {
        long n = 0;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = ((t + y) * bw + (l + x)) * 4;
            if (bgra[di + 2] != r || bgra[di + 1] != g || bgra[di] != b) n++;
        }
        return n;
    }
    static bool CmpPx(byte[] bgra, int di, byte[] src, int sw, int sh, int sx, int sy) {
        if (sx < 0 || sy < 0 || sx >= sw || sy >= sh) return false;
        int si = (sy * sw + sx) * 4;
        return bgra[di + 2] == src[si] && bgra[di + 1] == src[si + 1] && bgra[di] == src[si + 2];
    }
    // 2x-down phase census: for every dump pixel, which uniform source phase
    // model matches - D=(2x,2y) A=(2x,2y+1) C=(2x+1,2y) B=(2x+1,2y+1) -
    // plus per-model whole-region exactness and the first mismatch coords.
    public static string PhaseCensus(byte[] bgra, int bw, int l, int t, int w, int h, byte[] src, int sw, int sh) {
        long cD = 0, cA = 0, cC = 0, cB = 0;
        bool eD = true, eA = true, eC = true, eB = true;
        string first = "";
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = ((t + y) * bw + (l + x)) * 4;
            bool mD = CmpPx(bgra, di, src, sw, sh, 2 * x, 2 * y);
            bool mA = CmpPx(bgra, di, src, sw, sh, 2 * x, 2 * y + 1);
            bool mC = CmpPx(bgra, di, src, sw, sh, 2 * x + 1, 2 * y);
            bool mB = CmpPx(bgra, di, src, sw, sh, 2 * x + 1, 2 * y + 1);
            if (mD) cD++; else eD = false;
            if (mA) cA++; else eA = false;
            if (mC) cC++; else eC = false;
            if (mB) cB++; else eB = false;
            if (first == "" && !mD && !mA && !mC && !mB)
                first = String.Format("({0},{1}) got rgb=({2},{3},{4})", x, y, bgra[di + 2], bgra[di + 1], bgra[di]);
        }
        string ex = "";
        if (eD) ex += "D"; if (eA) ex += "A"; if (eC) ex += "C"; if (eB) ex += "B";
        if (ex == "") ex = "none";
        return String.Format("census D(2x,2y)={0} A(2x,2y+1)={1} C(2x+1,2y)={2} B(2x+1,2y+1)={3} of {4} exact={5} firstMismatch={6}",
            cD, cA, cC, cB, (long)w * h, ex, first);
    }
    // Mean luminance (0.299R+0.587G+0.114B) per column of a region.
    public static double[] ColumnMeans(byte[] bgra, int bw, int l, int t, int w, int h) {
        double[] m = new double[w];
        for (int x = 0; x < w; x++) {
            double s = 0;
            for (int y = 0; y < h; y++) {
                int di = ((t + y) * bw + (l + x)) * 4;
                s += 0.299 * bgra[di + 2] + 0.587 * bgra[di + 1] + 0.114 * bgra[di];
            }
            m[x] = s / h;
        }
        return m;
    }
    public static int DistinctLums(byte[] bgra, int bw, int l, int t, int w, int h) {
        bool[] seen = new bool[256]; int n = 0;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = ((t + y) * bw + (l + x)) * 4;
            int v = (int)(0.299 * bgra[di + 2] + 0.587 * bgra[di + 1] + 0.114 * bgra[di]);
            if (v < 0) v = 0; if (v > 255) v = 255;
            if (!seen[v]) { seen[v] = true; n++; }
        }
        return n;
    }
    // Photo-like fixture: smooth 2-axis gradients (kept inside [10,245] /
    // [64,191] so a [4,251] blown-out bound is clean) + three sharp-edged
    // solid blocks (absolute rects, valid for 640x480 and 800x600).
    public static byte[] PhotoSource(int w, int h) {
        byte[] a = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            a[i] = (byte)(10 + (int)(((double)x * 235.0) / (w - 1)));
            a[i + 1] = (byte)(10 + (int)(((double)y * 235.0) / (h - 1)));
            a[i + 2] = (byte)(64 + (int)(((double)x * 127.0) / (w - 1)));
            a[i + 3] = 255;
        }
        int[] bx = { 60, 420, 250 };
        int[] by = { 60, 90, 320 };
        int[] bw2 = { 80, 100, 90 };
        int[] bh2 = { 60, 80, 70 };
        int[] br = { 220, 30, 230 };
        int[] bg2 = { 30, 60, 220 };
        int[] bb2 = { 30, 220, 40 };
        for (int bi = 0; bi < 3; bi++)
            for (int y = by[bi]; y < by[bi] + bh2[bi] && y < h; y++)
                for (int x = bx[bi]; x < bx[bi] + bw2[bi] && x < w; x++) {
                    int i = (y * w + x) * 4;
                    a[i] = (byte)br[bi]; a[i + 1] = (byte)bg2[bi]; a[i + 2] = (byte)bb2[bi];
                }
        return a;
    }
    // Smooth both-axis gradient (full 0..255 range on R and G).
    public static byte[] GradSource(int w, int h) {
        byte[] a = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4;
            a[i] = (byte)(x * 255 / (w - 1));
            a[i + 1] = (byte)(y * 255 / (h - 1));
            a[i + 2] = (byte)((x + y) / 2);
            a[i + 3] = 255;
        }
        return a;
    }
    // 40000x256 banner: gray luminance ramp 8..247 EXCEPT a bandW-wide
    // 2px-period black/white stripe band centered at bandC (full height).
    public static byte[] BannerSource(int w, int h, int bandC, int bandW) {
        byte[] a = new byte[w * h * 4];
        int b0 = bandC - bandW / 2, b1 = bandC + bandW / 2;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int i = (y * w + x) * 4; int v;
            if (x >= b0 && x < b1) { v = ((x / 2) % 2 == 0) ? 0 : 255; }
            else { v = 8 + (int)(((double)x * 239.0) / (double)(w - 1)); }
            a[i] = (byte)v; a[i + 1] = (byte)v; a[i + 2] = (byte)v; a[i + 3] = 255;
        }
        return a;
    }
}
'@) -ReferencedAssemblies @('System.Drawing')

[void][S81]::SetProcessDPIAware()
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
    return [S81]::FindWindowExW($main, [IntPtr]::Zero, 'riviv_view', [NullString]::Value)
}
function View-Size($view) {
    $r = New-Object S81+RECT
    [void][S81]::GetClientRect($view, [ref]$r)
    return @([int]($r.R - $r.L), [int]($r.B - $r.T))
}
function Close-Main($p, $main) {
    if ($main -ne [IntPtr]::Zero) {
        [void][S81]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S81]::GetExitCodeProcess($script:RawH, [ref]$code)
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

$Stage = Join-Path $env:TEMP 'riviv-81-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$GoldenDir = Join-Path $PSScriptRoot 'golden81'

# The letterbox background: pure magenta where a scenario needs it (S2),
# injected as extra keys; banner/giant scenarios keep the DEFAULT WHITE
# background so the non-white BBox finds the image strip. The fixture
# sources never render white at their box edges under either bg.
function Scene-Ini($renderer, $fill, $wide, $high, $extra) {
    $lines = @('[riviv]', 'x=40', 'y=40', "wide=$wide", "high=$high", 'auto_zoom=0', 'icm=0',
        "fill_window=$fill")
    if ($extra) { $lines += $extra }
    $lines += "renderer=$renderer"
    return (($lines -join "`r`n") + "`r`n")
}
$BgMagenta = @('windowed_background_color_r=255', 'windowed_background_color_g=0', 'windowed_background_color_b=255')

# Calibrate the main window until the riviv_view child's client rect is
# EXACTLY tw x th (chrome and the status bar are integer pixels, so the
# delta walk converges). Returns the achieved view size.
function Calibrate-View($main, $tw, $th) {
    $script:calView = View-Of $main
    for ($i = 0; $i -lt 5; $i++) {
        $script:calVs = View-Size $script:calView
        if (($script:calVs[0] -eq $tw) -and ($script:calVs[1] -eq $th)) { return $script:calVs }
        $wr = New-Object S81+RECT
        [void][S81]::GetWindowRect($main, [ref]$wr)
        $nw = ($wr.R - $wr.L) + ($tw - $script:calVs[0])
        $nh = ($wr.B - $wr.T) + ($th - $script:calVs[1])
        [void][S81]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, $nw, $nh, 0x0006)  # SWP_NOMOVE|SWP_NOZORDER
        $script:calTw = $tw
        $script:calTh = $th
        $null = Wait-Until { $v = (View-Size $script:calView); ($v[0] -eq $script:calTw) -and ($v[1] -eq $script:calTh) } 3000
    }
    return (View-Size (View-Of $main))
}

# One adopt -> (calibrate) -> (commands) -> WM_CLOSE dump instance.
function Run-Scene($renderer, $fill, $extra, $img, $outName, $errName, $tw, $th, $cmds, $titleMs) {
    if ($titleMs -eq $null) { $titleMs = 12000 }
    if ($tw -ne $null) { $initW = $tw + 40; $initH = $th + 120 } else { $initW = 1000; $initH = 700 }
    Reset-Ini (Scene-Ini $renderer $fill $initW $initH $extra)
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    $p = Start-Riv ("`"$img`" -dump-viewport `"$out`"") $errName
    $main = Wait-Main $p
    $adopted = Wait-Title $p ([IO.Path]::GetFileNameWithoutExtension($img)) $titleMs
    $vs = @(0, 0)
    $alive = $false
    if ($main -ne [IntPtr]::Zero) {
        if ($tw -ne $null) { $vs = Calibrate-View $main $tw $th }
        else { $vs = View-Size (View-Of $main) }
        if ($cmds) { & $cmds $main }
        Start-Sleep -Milliseconds 400
        $alive = -not $p.HasExited
    }
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code; Vs = $vs; Adopted = $adopted; Alive = $alive; Err = (Read-Err $errName) }
}

# Golden judge for the frozen gdi-arm dumps. Emits nothing; the verdicts
# come back through the hashtable.
function Golden-Judge($dumps) {
    $res = @{}
    foreach ($key in @('k2', 'k3', 'k4', 'pad')) {
        $dump = $dumps[$key]
        $golden = Join-Path $GoldenDir ('s2-' + $key + '-gdi.png')
        if ($dump -eq $null -or -not (Test-Path $dump)) { $res[$key] = 'nodump'; continue }
        if (-not (Test-Path $golden)) { $res[$key] = 'missing'; continue }
        $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($dump))
        if ($same) { $res[$key] = 'match' } else { $res[$key] = 'differs' }
    }
    return $res
}
function Golden-Summary($gj) {
    $parts = @()
    foreach ($key in @('k2', 'k3', 'k4', 'pad')) { $parts += ($key + '=' + $gj[$key]) }
    return ($parts -join ' ')
}

Kill-Riviv
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
$RunExe = Join-Path $Stage 'riviv.exe'
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied, stage ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini))) 'Copy-Item failed or leftover ini'
if ($Regolden -and -not (Test-Path $GoldenDir)) { New-Item -ItemType Directory -Path $GoldenDir | Out-Null }

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------
$src96 = [Px]::HashSource(96, 64)
$png96 = Join-Path $Stage 'hash96.png'
[Px]::SaveRgba($png96, $src96, 96, 64)
$src192 = [Px]::UpsampleK($src96, 96, 64, 2)
$pngPad = Join-Path $Stage 'pad192.png'
[Px]::SaveRgba($pngPad, $src192, 192, 128)
$pngGrad = Join-Path $Stage 'grad256.png'
[Px]::SaveRgba($pngGrad, [Px]::GradSource(256, 256), 256, 256)
$photo640 = [Px]::PhotoSource(640, 480)
$png640 = Join-Path $Stage 'photo640.png'
[Px]::SaveRgba($png640, $photo640, 640, 480)
$png800 = Join-Path $Stage 'photo800.png'
[Px]::SaveRgba($png800, [Px]::PhotoSource(800, 600), 800, 600)
$pngBanner = Join-Path $Stage 'banner.png'
[Px]::SaveRgba($pngBanner, [Px]::BannerSource(40000, 256, 20000, 4000), 40000, 256)
$giantPng = Join-Path $Stage 'giant.png'
[S81]::WriteWidePngGrad($giantPng, 16777217, 1)
$giant4mPng = Join-Path $Stage 'giant4m.png'
[S81]::WriteWidePngGrad($giant4mPng, 4000000, 1)
$giant4nPng = Join-Path $Stage 'giant4n.png'
[S81]::WriteWidePngGrad($giant4nPng, 4194304, 1)
$giant4pPng = Join-Path $Stage 'giant4p.png'
[S81]::WriteWidePngGrad($giant4pPng, 8388608, 1)
Check 'S0 fixtures built' ((Test-Path $png96) -and (Test-Path $pngPad) -and (Test-Path $pngGrad) -and (Test-Path $png640) -and (Test-Path $png800) -and (Test-Path $pngBanner) -and (Test-Path $giantPng) -and (Test-Path $giant4mPng) -and (Test-Path $giant4nPng) -and (Test-Path $giant4pPng)) 'a fixture PNG is missing'

# ---------------------------------------------------------------------------
# S1: the default renderer flip (#81): missing key AND unrecognized value
# both land on auto; gdi stays the escape hatch.
# ---------------------------------------------------------------------------
Reset-Ini "[riviv]`r`nx=40`r`ny=40`r`nwide=800`r`nhigh=600`r`n"
$p = Start-Riv '' 's1-missing.err'
$main = Wait-Main $p
$code = Close-Main $p $main
$err = Read-Err 's1-missing.err'
$bcLine = '(none)'
if ($err -match 'riviv: renderer=[^\r\n]+') { $bcLine = $Matches[0] }
Check 'S1a missing renderer key -> auto breadcrumb' (($code -eq 0) -and $err.Contains('riviv: renderer=auto backend=')) ("exit=$code line=[$bcLine]")
Reset-Ini "[riviv]`r`nrenderer=frobnicate`r`n"
$p = Start-Riv '' 's1-frob.err'
$main = Wait-Main $p
$code = Close-Main $p $main
$err = Read-Err 's1-frob.err'
$bcLine = '(none)'
if ($err -match 'riviv: renderer=[^\r\n]+') { $bcLine = $Matches[0] }
Check 'S1b frobnicate -> unrecognized hint + using auto + auto backend' (($code -eq 0) -and $err.Contains('unrecognized renderer value') -and $err.Contains('using auto') -and $err.Contains('riviv: renderer=auto backend=')) ("exit=$code line=[$bcLine] stderr=[$($err.Trim())]")
Reset-Ini "[riviv]`r`nrenderer=gdi`r`n"
$p = Start-Riv '' 's1-gdi.err'
$main = Wait-Main $p
$code = Close-Main $p $main
$err = Read-Err 's1-gdi.err'
Check 'S1c renderer=gdi escape hatch still works' (($code -eq 0) -and $err.Contains('riviv: renderer=gdi backend=gdi')) ("exit=$code stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2 (core): integer magnify byte-exactness, k = 2, 3, 4, warp and gdi.
# k=3/4: exact k*W x k*H viewport, fill renders edge to edge. k=2: the
# window min track width clamps the view to 256, where fill_window=1 still
# renders EXACTLY 2x (192x128) centered at (32,0) with 32px magenta sides.
# ---------------------------------------------------------------------------
$dumps = @{}
$s2geo = @(
    @{ K = 2; TW = 256; TH = 128; BL = 32; BT = 0; BW = 192; BH = 128 },
    @{ K = 3; TW = 288; TH = 192; BL = 0; BT = 0; BW = 288; BH = 192 },
    @{ K = 4; TW = 384; TH = 256; BL = 0; BT = 0; BW = 384; BH = 256 }
)
foreach ($geo in $s2geo) {
    $k = $geo.K
    $w = Run-Scene 'warp' 1 (@('mag_filter=0') + $BgMagenta) $png96 ("s2-k${k}-warp.png") ("s2-k${k}-warp.err") $geo.TW $geo.TH $null $null
    $g = Run-Scene 'gdi' 1 (@('mag_filter=0') + $BgMagenta) $png96 ("s2-k${k}-gdi.png") ("s2-k${k}-gdi.err") $geo.TW $geo.TH $null $null
    $dumps["k$k"] = $g.Out
    $ranOk = ($w.Code -eq 0) -and ($g.Code -eq 0) -and (Test-Path $w.Out) -and (Test-Path $g.Out)
    Check "S2a-k$k both dumps ran clean (exit 0, files exist)" $ranOk ("warp exit=$($w.Code) gdi exit=$($g.Code) warpPng=$(Test-Path $w.Out) gdiPng=$(Test-Path $g.Out) warpView=$($w.Vs[0])x$($w.Vs[1]) gdiView=$($g.Vs[0])x$($g.Vs[1])")
    if (-not $ranOk) {
        Write-Output ("S2-k$k warp stderr: " + ($w.Err.Trim()))
        Write-Output ("S2-k$k gdi stderr: " + ($g.Err.Trim()))
        continue
    }
    $eqFile = [Px]::BytesEqual([IO.File]::ReadAllBytes($w.Out), [IO.File]::ReadAllBytes($g.Out))
    $qw = [Px]::Load($w.Out)
    $qg = [Px]::Load($g.Out)
    $ad = [Px]::AlphaDiff($qw.B, $qg.B)
    $note = ''
    if (-not $eqFile) { $note = " alphaDiffPixels=$ad (decoded RGB compared separately below; the known GDI-dump zeroed-alpha finding if RGB matches)" }
    Check "S2b-k$k warp and gdi dump FILE bytes identical" $eqFile ("bytes $($qw.W)x$($qw.H)" + $note)
    $rgbDiff = [Px]::RgbDiff($qw.B, $qg.B)
    Check "S2c-k$k warp and gdi DECODED pixels identical (RGB; alpha counted apart)" (($rgbDiff -eq $null)) "rgbDiff=$rgbDiff alphaDiffPixels=$ad"
    Check "S2d-k$k dump is the calibrated viewport $($geo.TW)x$($geo.TH)" (($qw.W -eq $geo.TW) -and ($qw.H -eq $geo.TH) -and ($qg.W -eq $geo.TW) -and ($qg.H -eq $geo.TH)) ("warp=$($qw.W)x$($qw.H) gdi=$($qg.W)x$($qg.H) want=$($geo.TW)x$($geo.TH)")
    # Margins pure magenta where the clamped viewport leaves any (k=2 sides).
    $margOk2 = $true
    $margDet2 = ''
    if (($geo.BL -gt 0) -or ($geo.BT -gt 0)) {
        foreach ($q in @($qw, $qg)) {
            $strips = @()
            if ($geo.BL -gt 0) { $strips += ,(0, $geo.BT, $geo.BL, $geo.BH) }
            if ($geo.BT -gt 0) { $strips += ,($geo.BL, 0, $geo.BW, $geo.BT) }
            if (($geo.TW - $geo.BL - $geo.BW) -gt 0) { $strips += ,(($geo.BL + $geo.BW), $geo.BT, ($geo.TW - $geo.BL - $geo.BW), $geo.BH) }
            if (($geo.TH - $geo.BT - $geo.BH) -gt 0) { $strips += ,($geo.BL, ($geo.BT + $geo.BH), $geo.BW, ($geo.TH - $geo.BT - $geo.BH)) }
            foreach ($s in $strips) {
                $bad = [Px]::CountNotBg($q.B, $q.W, $s[0], $s[1], $s[2], $s[3], 255, 0, 255)
                if ($bad -ne 0) { $margOk2 = $false; $margDet2 += " rect($($s[0]),$($s[1]),$($s[2]),$($s[3]))nonbg=$bad" }
            }
        }
        if ($margDet2 -eq '') { $margDet2 = 'all margin strips pure magenta in both arms' }
    } else {
        $margDet2 = 'exact fill - no margin strips by geometry'
    }
    Check "S2e-k$k margins pure background color (both arms)" $margOk2 $margDet2
    $model = [Px]::UpsampleK($src96, 96, 64, $k)
    $mw = [Px]::CompareRgb($qw.B, $qw.W, $geo.BL, $geo.BT, $model, $geo.BW)
    $mg = [Px]::CompareRgb($qg.B, $qg.W, $geo.BL, $geo.BT, $model, $geo.BW)
    Check "S2f-k$k image box == replication model src[x/k,y/k] at ($($geo.BL),$($geo.BT)) (both arms, exact RGB)" (($mw -eq $null) -and ($mg -eq $null)) "warp-first=$mw gdi-first=$mg"
    $golden = Join-Path $GoldenDir ("s2-k$k-gdi.png")
    if (-not (Test-Path $golden)) {
        Skip-Scenario "S2g-k$k golden compare" "no golden yet (run -Regolden to freeze smoke\golden81\s2-k$k-gdi.png)"
    } else {
        $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($g.Out))
        Check "S2g-k$k gdi dump matches frozen golden" $same 'golden bytes differ from this run'
    }
}
Kill-Riviv
Reset-Ini ''

# S2 padded: the 192x128 source IS the exact 2x replication of the 96x64
# pattern; the default fit (fill_window=0, never upscale) renders it 1:1
# centered in a 256x158 viewport (232 clamps to the min track width):
# box 32..223 x 15..142, side margins 32, top/bottom margins 15.
$PADW = 256
$PADH = 158
$padL = 32
$padT = 15
$padW2 = Run-Scene 'warp' 0 $BgMagenta $pngPad 's2-pad-warp.png' 's2-pad-warp.err' $PADW $PADH $null $null
$padG = Run-Scene 'gdi' 0 $BgMagenta $pngPad 's2-pad-gdi.png' 's2-pad-gdi.err' $PADW $PADH $null $null
$dumps['pad'] = $padG.Out
$padOk = ($padW2.Code -eq 0) -and ($padG.Code -eq 0) -and (Test-Path $padW2.Out) -and (Test-Path $padG.Out)
Check 'S2p-a padded both dumps ran clean' $padOk ("warp exit=$($padW2.Code) gdi exit=$($padG.Code) views=$($padW2.Vs[0])x$($padW2.Vs[1])/$($padG.Vs[0])x$($padG.Vs[1])")
if ($padOk) {
    $qw = [Px]::Load($padW2.Out)
    $qg = [Px]::Load($padG.Out)
    Check 'S2p-b padded dump is 256x158' (($qw.W -eq $PADW) -and ($qw.H -eq $PADH) -and ($qg.W -eq $PADW) -and ($qg.H -eq $PADH)) ("warp=$($qw.W)x$($qw.H) gdi=$($qg.W)x$($qg.H)")
    $eqPad = [Px]::BytesEqual([IO.File]::ReadAllBytes($padW2.Out), [IO.File]::ReadAllBytes($padG.Out))
    Check 'S2p-c padded warp and gdi file bytes identical' $eqPad ("alphaDiffPixels=$([Px]::AlphaDiff($qw.B, $qg.B)) rgbDiff=$([Px]::RgbDiff($qw.B, $qg.B))")
    # Margins pure magenta, both arms (the whole frame outside the box).
    $margsOk = $true
    $margDetail = ''
    foreach ($q in @($qw, $qg)) {
        $strips = @(
            @(0, 0, $PADW, $padT),
            @(0, ($padT + 128), $PADW, ($PADH - $padT - 128)),
            @(0, $padT, $padL, 128),
            @(($padL + 192), $padT, ($PADW - $padL - 192), 128)
        )
        foreach ($s in $strips) {
            $bad = [Px]::CountNotBg($q.B, $q.W, $s[0], $s[1], $s[2], $s[3], 255, 0, 255)
            if ($bad -ne 0) { $margsOk = $false; $margDetail += " rect($($s[0]),$($s[1]),$($s[2]),$($s[3]))nonbg=$bad" }
        }
    }
    if ($margDetail -eq '') { $margDetail = 'all four strips pure magenta in both arms' }
    Check 'S2p-d margins pure background color (both arms)' $margsOk $margDetail
    # Image box == the 2x replica at exactly (20,15): boundary +-0 by build.
    $bw2 = [Px]::CompareRgb($qw.B, $qw.W, $padL, $padT, $src192, 192)
    $bg2 = [Px]::CompareRgb($qg.B, $qg.W, $padL, $padT, $src192, 192)
    Check 'S2p-e image box == 2x replica at (32,15), boundary +-0px (both arms)' (($bw2 -eq $null) -and ($bg2 -eq $null)) "warp-first=$bw2 gdi-first=$bg2"
    $golden = Join-Path $GoldenDir 's2-pad-gdi.png'
    if (-not (Test-Path $golden)) {
        Skip-Scenario 'S2p-f golden compare' 'no golden yet (run -Regolden to freeze smoke\golden81\s2-pad-gdi.png)'
    } else {
        $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($padG.Out))
        Check 'S2p-f gdi dump matches frozen golden' $same 'golden bytes differ from this run'
    }
}
if ($Regolden) {
    foreach ($key in @('k2', 'k3', 'k4', 'pad')) {
        $src = $dumps[$key]
        if (($src -ne $null) -and (Test-Path $src)) { Copy-Item $src (Join-Path $GoldenDir ('s2-' + $key + '-gdi.png')) -Force }
    }
    Write-Output 'GOLDENS: (re)written from this run (-Regolden)'
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the L2 filter tiers.
# ---------------------------------------------------------------------------
# S3a mag=1: 256x256 smooth gradient at exact 2x (fill recipe, 512x512):
# warp LINEAR vs gdi HALFTONE.
$m1w = Run-Scene 'warp' 1 @('mag_filter=1') $pngGrad 's3a-mag1-warp.png' 's3a-warp.err' 512 512 $null $null
$m1g = Run-Scene 'gdi' 1 @('mag_filter=1') $pngGrad 's3a-mag1-gdi.png' 's3a-gdi.err' 512 512 $null $null
$m1ok = ($m1w.Code -eq 0) -and ($m1g.Code -eq 0) -and (Test-Path $m1w.Out) -and (Test-Path $m1g.Out)
Check 'S3a-mag1 both dumps ran clean (exit 0, files exist)' $m1ok ("warp exit=$($m1w.Code) gdi exit=$($m1g.Code) views=$($m1w.Vs[0])x$($m1w.Vs[1])/$($m1g.Vs[0])x$($m1g.Vs[1])")
if ($m1ok) {
    $qw = [Px]::Load($m1w.Out)
    $qg = [Px]::Load($m1g.Out)
    Check 'S3a-mag1 both dumps are exactly 512x512 (exact-fill recipe held)' (($qw.W -eq 512) -and ($qw.H -eq 512) -and ($qg.W -eq 512) -and ($qg.H -eq 512)) ("warp=$($qw.W)x$($qw.H) gdi=$($qg.W)x$($qg.H)")
    $s = [Px]::StatPair($qw.B, $qg.B, 512, 512, $null)
    $maeTxt = 'maeR={0:N3} maeG={1:N3} maeB={2:N3} max={3}' -f $s[0], $s[1], $s[2], [int]$s[3]
    Check 'S3a-mag1 warp(LINEAR) vs gdi(HALFTONE) per-channel MAE <= 2.0' (($s[0] -le 2.0) -and ($s[1] -le 2.0) -and ($s[2] -le 2.0)) $maeTxt
    Check 'S3a-mag1 max channel diff <= 12' ([int]$s[3] -le 12) $maeTxt
    Write-Output ('  S3a evidence: ' + $maeTxt)
}
Kill-Riviv
Reset-Ini ''

# S3b shrink=1 (default tier): 640x480 at exactly half size (320x240):
# warp HIGH_QUALITY_CUBIC vs gdi HALFTONE (from the full-res face).
$exclFlat = @(26, 26, 48, 38, 206, 41, 58, 48, 121, 156, 53, 43)  # blocks dilated 8 src px, /2
$s1w = Run-Scene 'warp' 0 @('shrink_blit_mode=1') $png640 's3b-shrink1-warp.png' 's3b-warp.err' 320 240 $null $null
$s1g = Run-Scene 'gdi' 0 @('shrink_blit_mode=1') $png640 's3b-shrink1-gdi.png' 's3b-gdi.err' 320 240 $null $null
$s1ok = ($s1w.Code -eq 0) -and ($s1g.Code -eq 0) -and (Test-Path $s1w.Out) -and (Test-Path $s1g.Out)
Check 'S3b-shrink1 both dumps ran clean (exit 0, files exist)' $s1ok ("warp exit=$($s1w.Code) gdi exit=$($s1g.Code)")
if ($s1ok) {
    $qw = [Px]::Load($s1w.Out)
    $qg = [Px]::Load($s1g.Out)
    Check 'S3b-shrink1 both dumps are exactly 320x240' (($qw.W -eq 320) -and ($qw.H -eq 240) -and ($qg.W -eq 320) -and ($qg.H -eq 240)) ("warp=$($qw.W)x$($qw.H) gdi=$($qg.W)x$($qg.H)")
    $s = [Px]::StatPair($qw.B, $qg.B, 320, 240, $exclFlat)
    $maeTxt = 'maeR={0:N3} maeG={1:N3} maeB={2:N3} max={3} maxOutsideBlocks={4} diffPixels={5}' -f $s[0], $s[1], $s[2], [int]$s[3], [int]$s[4], [int]$s[5]
    Check 'S3b-shrink1 CUBIC and HALFTONE genuinely differ (>=1000 px)' ([int]$s[5] -ge 1000) $maeTxt
    Check 'S3b-shrink1 max diff outside block halos <= 40' ([int]$s[4] -le 40) $maeTxt
    $blowW = [Px]::CountOutside($qw.B, $qw.W, 320, 240, 4, 251, $exclFlat)
    $blowG = [Px]::CountOutside($qg.B, $qg.W, 320, 240, 4, 251, $exclFlat)
    Check 'S3b-shrink1 no blown-out pixels outside block halos (both arms)' (($blowW -eq 0) -and ($blowG -eq 0)) "warpOutside=$blowW gdiOutside=$blowG"
    Write-Output ('  S3b evidence: ' + $maeTxt)
}
Kill-Riviv
Reset-Ini ''

# S3c shrink=0: warp NEAREST vs gdi COLORONCOLOR at exact 2x down. Byte
# equality is the ticket claim; a uniform one-source-pixel sampling phase
# (src[2i] vs src[2i+1]) is the accepted legitimate divergence - the census
# adjudicates, anything else is a FAIL.
$s0w = Run-Scene 'warp' 0 @('shrink_blit_mode=0') $png640 's3c-shrink0-warp.png' 's3c-warp.err' 320 240 $null $null
$s0g = Run-Scene 'gdi' 0 @('shrink_blit_mode=0') $png640 's3c-shrink0-gdi.png' 's3c-gdi.err' 320 240 $null $null
$s0ok = ($s0w.Code -eq 0) -and ($s0g.Code -eq 0) -and (Test-Path $s0w.Out) -and (Test-Path $s0g.Out)
Check 'S3c-shrink0 both dumps ran clean (exit 0, files exist)' $s0ok ("warp exit=$($s0w.Code) gdi exit=$($s0g.Code)")
if ($s0ok) {
    $eq0 = [Px]::BytesEqual([IO.File]::ReadAllBytes($s0w.Out), [IO.File]::ReadAllBytes($s0g.Out))
    if ($eq0) {
        Check 'S3c-shrink0 warp(NEAREST) and gdi(COLORONCOLOR) byte-identical' $true 'file bytes equal'
    } else {
        $cw = [Px]::PhaseCensus([Px]::Load($s0w.Out).B, 320, 0, 0, 320, 240, $photo640, 640, 480)
        $cg = [Px]::PhaseCensus([Px]::Load($s0g.Out).B, 320, 0, 0, 320, 240, $photo640, 640, 480)
        $okW = ($cw -match 'exact=[A-D]+')
        $okG = ($cg -match 'exact=[A-D]+')
        Check 'S3c-shrink0 not byte-identical but each arm matches ONE uniform phase model' ($okW -and $okG) "warp: $cw | gdi: $cg"
    }
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: giant correctness. Banner 40000x256 (>=32768 extent -> the gdi arm
# takes the NEW full-res-face clip-region branch; warp still uploads).
# ---------------------------------------------------------------------------
function Banner-Checks($q) {
    # Returns the measured banner properties (no pipeline output). The
    # stripes band columns are DERIVED from the actual strip width (the
    # viewport is whatever the window gave, e.g. 974 at 200% DPI, not 1000).
    $r = @{}
    $bb = [Px]::BBox($q.B, $q.W, $q.H)
    $r.Bb = $bb
    $r.Bw = $bb[2] - $bb[0] + 1
    $r.Bh = $bb[3] - $bb[1] + 1
    if (($r.Bw -le 0) -or ($r.Bh -le 0) -or ($bb[0] -lt 0) -or ($bb[1] -lt 0)) { return $r }
    $m = [Px]::ColumnMeans($q.B, $q.W, $bb[0], $bb[1], $r.Bw, $r.Bh)
    $r.ColMeans = $m
    # the stripes band lands at source x 18000..22000 of 40000, scaled
    $bx0 = [int](18000 * $r.Bw / 40000)
    $bx1 = [int](22000 * $r.Bw / 40000)
    $r.Bx0 = $bx0
    $r.Bx1 = $bx1
    $bandSum = 0.0
    $bandN = 0
    for ($c = ($bx0 + 10); $c -le ($bx1 - 10); $c++) { $bandSum += $m[$c]; $bandN++ }
    $r.BandMean = -1.0
    if ($bandN -gt 0) { $r.BandMean = $bandSum / $bandN }
    # monotone non-decreasing over the left gradient half (+-2), stopping
    # 10 columns short of the band's transition
    $r.Monotone = $true
    $r.WorstDrop = 0.0
    for ($c = 6; $c -le ($bx0 - 10); $c++) {
        $d = $m[$c - 1] - $m[$c]
        if ($d -gt $r.WorstDrop) { $r.WorstDrop = $d }
        if ($d -gt 2.0) { $r.Monotone = $false }
    }
    # seam scan: adjacent-column |delta|, median+max over the smooth halves
    # (band excluded by +-10); the full-region max is recorded too.
    $deltas = New-Object System.Collections.Generic.List[double]
    $r.MaxFull = 0.0
    for ($c = 1; $c -lt $r.Bw; $c++) {
        $d = [Math]::Abs($m[$c] - $m[$c - 1])
        if ($d -gt $r.MaxFull) { $r.MaxFull = $d }
        if (($c -le ($bx0 - 10)) -or ($c -ge ($bx1 + 11))) { $deltas.Add($d) }
    }
    $r.Median = -1.0
    $r.MaxExcl = -1.0
    if ($deltas.Count -gt 0) {
        $arr = $deltas.ToArray()
        [Array]::Sort($arr)
        $r.Median = $arr[[int]($arr.Length / 2)]
        $r.MaxExcl = $arr[$arr.Length - 1]
    }
    $r.Lums = [Px]::DistinctLums($q.B, $q.W, $bb[0], $bb[1], $r.Bw, $r.Bh)
    return $r
}
$baw = Run-Scene 'warp' 0 @('shrink_blit_mode=1') $pngBanner 's4-banner-warp.png' 's4-warp.err' $null $null $null 30000
$bag = Run-Scene 'gdi' 0 @('shrink_blit_mode=1') $pngBanner 's4-banner-gdi.png' 's4-gdi.err' $null $null $null 30000
$baOk = ($baw.Code -eq 0) -and ($bag.Code -eq 0) -and (Test-Path $baw.Out) -and (Test-Path $bag.Out)
Check 'S4a-banner both dumps ran clean (exit 0)' $baOk ("warp exit=$($baw.Code) gdi exit=$($bag.Code) warpAlive=$($baw.Alive) gdiAlive=$($bag.Alive)")
if ($baOk) {
    $gateW = $baw.Err.Contains('exceeds the D2D max bitmap')
    Check 'S4a-banner-warp no D2D gate line (40000 uploaded, CUBIC deep shrink really ran)' (-not $gateW) ("stderr=[$($baw.Err.Trim())]")
    $qw = [Px]::Load($baw.Out)
    $rw = Banner-Checks $qw
    Check 'S4b-banner-warp bbox: height <= 8, width == viewport width' (($rw.Bh -ge 1) -and ($rw.Bh -le 8) -and ($rw.Bw -eq $baw.Vs[0])) ("bbox l=$($rw.Bb[0]) t=$($rw.Bb[1]) $($rw.Bw)x$($rw.Bh) viewport=$($baw.Vs[0])x$($baw.Vs[1])")
    Check 'S4c-banner-warp stripes band mean luminance in [96,160] (averaged, no aliasing garbage)' (($rw.BandMean -ge 96) -and ($rw.BandMean -le 160)) ('bandMean={0:N1}' -f $rw.BandMean)
    Check 'S4d-banner-warp left-half monotone (+-2) and seam scan max <= 3*median+2' ($rw.Monotone -and ($rw.MaxExcl -le (3 * $rw.Median + 2))) ('worstDrop={0:N2} medianDelta={1:N3} maxExcl={2:N2} maxFull={3:N2}' -f $rw.WorstDrop, $rw.Median, $rw.MaxExcl, $rw.MaxFull)
    $qg = [Px]::Load($bag.Out)
    $rg = Banner-Checks $qg
    Check 'S4e-banner-gdi bbox same shape (full-res-face branch drew the whole frame)' (($rg.Bh -ge 1) -and ($rg.Bh -le 8) -and ($rg.Bw -eq $bag.Vs[0])) ("bbox $($rg.Bw)x$($rg.Bh) viewport=$($bag.Vs[0])x$($bag.Vs[1])")
    Check 'S4f-banner-gdi band mean in [80,176] (HALFTONE) + monotone + >=64 distinct lums' (($rg.BandMean -ge 80) -and ($rg.BandMean -le 176) -and $rg.Monotone -and ($rg.Lums -ge 64)) ('bandMean={0:N1} monotone={1} worstDrop={2:N2} lums={3}' -f $rg.BandMean, $rg.Monotone, $rg.WorstDrop, $rg.Lums)
    $mae = [Px]::StatPair($qw.B, $qg.B, [Math]::Min($qw.W, $qg.W), [Math]::Min($qw.H, $qg.H), $null)
    Write-Output ('  S4 banner record: warp bandMean={0:N1} gdi bandMean={1:N1} warp maxFull={2:N2} gdi maxFull={3:N2} warpVsGdiRegionMAE(R/G/B)={4:N2}/{5:N2}/{6:N2} max={7}' -f $rw.BandMean, $rg.BandMean, $rw.MaxFull, $rg.MaxFull, $mae[0], $mae[1], $mae[2], [int]$mae[3])
}
Kill-Riviv
Reset-Ini ''

# Extreme giant 16777217x1 horizontal gradient. The gdi run asserts the
# CONTENT of the >=32768 giant branch; the warp run asserts the gate line
# plus the same content (smoke80 S5 upgraded with a content assertion).
$giantExtra = @('shrink_blit_mode=1')
$ggr = Run-Scene 'gdi' 0 $giantExtra $giantPng 's4-giant-gdi.png' 's4-giant-gdi.err' $null $null $null 90000
$ggOk = ($ggr.Code -eq 0) -and (Test-Path $ggr.Out)
Check 'S4g-giant-gdi ran clean (exit 0, dump exists, no gate on the gdi arm)' ($ggOk -and (-not $ggr.Err.Contains('exceeds the D2D max bitmap'))) ("exit=$($ggr.Code) stderr=[$($ggr.Err.Trim())]")
if ($ggOk) {
    $q = [Px]::Load($ggr.Out)
    $bb = [Px]::BBox($q.B, $q.W, $q.H)
    $bh = $bb[3] - $bb[1] + 1
    $bwd = $bb[2] - $bb[0] + 1
    $shapeOk = ($bh -ge 1) -and ($bh -le 2) -and ($bwd -eq $ggr.Vs[0])
    Check 'S4h-giant-gdi image bbox is a strip spanning the viewport width, 1-2px tall' $shapeOk ("bbox l=$($bb[0]) t=$($bb[1]) ${bwd}x${bh} viewport=$($ggr.Vs[0])x$($ggr.Vs[1])")
    $gradOk = $false
    $gradDetail = 'bbox degenerate'
    if ($shapeOk) {
        $cm = [Px]::ColumnMeans($q.B, $q.W, $bb[0], $bb[1], $bwd, $bh)
        $q4 = [int]($bwd / 4)
        $lq = 0.0
        $rq = 0.0
        for ($c = 0; $c -lt $q4; $c++) { $lq += $cm[$c] }
        for ($c = ($bwd - $q4); $c -lt $bwd; $c++) { $rq += $cm[$c] }
        $lq = $lq / $q4
        $rq = $rq / $q4
        $maxRun = 0
        $curRun = 0
        for ($c = 0; $c -lt $bwd; $c++) {
            if (($cm[$c] -lt 8) -or ($cm[$c] -gt 247)) { $curRun++ } else { if ($curRun -gt $maxRun) { $maxRun = $curRun }; $curRun = 0 }
        }
        if ($curRun -gt $maxRun) { $maxRun = $curRun }
        $gradOk = ([Math]::Abs($lq - $rq) -ge 40) -and ($maxRun -lt 8)
        $gradDetail = 'leftMean={0:N1} rightMean={1:N1} diff={2:N1} maxExtremeRun={3}' -f $lq, $rq, [Math]::Abs($lq - $rq), $maxRun
    }
    Check 'S4i-giant-gdi gradient survived (quarter means differ >= 40, no extreme run >= 8)' $gradOk $gradDetail
}
Kill-Riviv
Reset-Ini ''

$gwr = Run-Scene 'warp' 0 $giantExtra $giantPng 's4-giant-warp.png' 's4-giant-warp.err' $null $null $null 90000
$gwOk = ($gwr.Code -eq 0) -and (Test-Path $gwr.Out)
$gerr = $gwr.Err
$iBc = $gerr.IndexOf('renderer=warp backend=')
$iGate = $gerr.IndexOf('exceeds the D2D max bitmap')
$gOrder = ($iBc -ge 0) -and ($iGate -gt $iBc) -and $gerr.Contains('rendering it via gdi')
$gMaxLine = ''
if ($gerr -match 'exceeds the D2D max bitmap (\d+)') { $gMaxLine = 'device max=' + $Matches[1] }
Check 'S4j-giant-warp gate line present after the breadcrumb, fell back to gdi' $gOrder ("order=$gOrder $gMaxLine stderr=[$($gerr.Trim())]")
if ($gwOk) {
    $q = [Px]::Load($gwr.Out)
    $bb = [Px]::BBox($q.B, $q.W, $q.H)
    $bh = $bb[3] - $bb[1] + 1
    $bwd = $bb[2] - $bb[0] + 1
    $shapeOk = ($bh -ge 1) -and ($bh -le 2) -and ($bwd -eq $gwr.Vs[0])
    $gradOk = $false
    $gradDetail = 'bbox degenerate'
    if ($shapeOk) {
        $cm = [Px]::ColumnMeans($q.B, $q.W, $bb[0], $bb[1], $bwd, $bh)
        $q4 = [int]($bwd / 4)
        $lq = 0.0
        $rq = 0.0
        for ($c = 0; $c -lt $q4; $c++) { $lq += $cm[$c] }
        for ($c = ($bwd - $q4); $c -lt $bwd; $c++) { $rq += $cm[$c] }
        $lq = $lq / $q4
        $rq = $rq / $q4
        $gradOk = ([Math]::Abs($lq - $rq) -ge 40)
        $gradDetail = 'leftMean={0:N1} rightMean={1:N1} diff={2:N1}' -f $lq, $rq, [Math]::Abs($lq - $rq)
    }
    Check 'S4k-giant-warp dump content passes the same gradient assertions' ($shapeOk -and $gradOk) ("bbox ${bwd}x${bh} " + $gradDetail)
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4 boundary census: the #81 stitch trigger at 2^22. The gdi shrink arm
# cuts a source extent >= STRETCH_SOURCE_STITCH_TRIGGER (2^22) into 2^21
# source slices (stitch::stitch_tiles_sized), keeping the 32768..2^22 band
# on the single full-rect StretchBlt. Three widths at height 1 straddle the
# boundary; each runs renderer=gdi fit-dump with the same content
# assertions as the 16777217x1 extreme giant above (strip spans the
# viewport width, left/right quarter mean luminance differ >= 40, no run
# of >= 8 consecutive all-black/all-white columns) and the two sliced
# widths add a seam scan: over the strip's row, max adjacent-column
# |delta mean| <= 3*median + 2 (numbers recorded in the detail).
# ---------------------------------------------------------------------------
function Giant-Checks($q, $viewW) {
    # Strip shape + content stats for a 1px-tall gradient giant, mirroring
    # the inline assertions of the 16777217x1 scenario (no pipeline output).
    $r = @{}
    $bb = [Px]::BBox($q.B, $q.W, $q.H)
    $r.Bb = $bb
    $r.Bh = $bb[3] - $bb[1] + 1
    $r.Bw = $bb[2] - $bb[0] + 1
    $r.ShapeOk = ($r.Bh -ge 1) -and ($r.Bh -le 2) -and ($r.Bw -eq $viewW)
    $r.Lq = -1.0
    $r.Rq = -1.0
    $r.Diff = -1.0
    $r.MaxRun = -1
    $r.MaxDelta = -1.0
    $r.MedianDelta = -1.0
    if (-not $r.ShapeOk) { return $r }
    $cm = [Px]::ColumnMeans($q.B, $q.W, $bb[0], $bb[1], $r.Bw, $r.Bh)
    $q4 = [int]($r.Bw / 4)
    $lq = 0.0
    $rq = 0.0
    for ($c = 0; $c -lt $q4; $c++) { $lq += $cm[$c] }
    for ($c = ($r.Bw - $q4); $c -lt $r.Bw; $c++) { $rq += $cm[$c] }
    $r.Lq = $lq / $q4
    $r.Rq = $rq / $q4
    $r.Diff = [Math]::Abs($r.Lq - $r.Rq)
    $maxRun = 0
    $curRun = 0
    for ($c = 0; $c -lt $r.Bw; $c++) {
        if (($cm[$c] -lt 8) -or ($cm[$c] -gt 247)) { $curRun++ } else { if ($curRun -gt $maxRun) { $maxRun = $curRun }; $curRun = 0 }
    }
    if ($curRun -gt $maxRun) { $maxRun = $curRun }
    $r.MaxRun = $maxRun
    $deltas = New-Object System.Collections.Generic.List[double]
    $r.MaxDelta = 0.0
    for ($c = 1; $c -lt $r.Bw; $c++) {
        $d = [Math]::Abs($cm[$c] - $cm[$c - 1])
        if ($d -gt $r.MaxDelta) { $r.MaxDelta = $d }
        $deltas.Add($d)
    }
    if ($deltas.Count -gt 0) {
        $arr = $deltas.ToArray()
        [Array]::Sort($arr)
        $r.MedianDelta = $arr[[int]($arr.Length / 2)]
    }
    return $r
}
$giantCensus = @(
    @{ N = 'm'; W = 4000000; Png = $giant4mPng; Seam = $false },  # below the 2^22 trigger: single full-rect path must still render
    @{ N = 'n'; W = 4194304; Png = $giant4nPng; Seam = $true },   # exactly 2^22: the first sliced width
    @{ N = 'p'; W = 8388608; Png = $giant4pPng; Seam = $true }    # 2^23: the width that rendered black before the stitch fix
)
foreach ($cw in $giantCensus) {
    $tag = 'S4' + $cw.N
    if ($cw.Seam) { $pathTxt = 'sliced(2^21)' } else { $pathTxt = 'single-full-rect' }
    $crr = Run-Scene 'gdi' 0 $giantExtra $cw.Png ('s4-census-' + $cw.N + '-gdi.png') ('s4-census-' + $cw.N + '-gdi.err') $null $null $null 90000
    $crOk = ($crr.Code -eq 0) -and (Test-Path $crr.Out)
    Check ($tag + '-census-' + $cw.W + '-gdi ran clean (exit 0, dump exists, no gate on the gdi arm)') ($crOk -and (-not $crr.Err.Contains('exceeds the D2D max bitmap'))) ("exit=$($crr.Code) stderr=[$($crr.Err.Trim())]")
    if (-not $crOk) { Kill-Riviv; Reset-Ini ''; continue }
    $cq = [Px]::Load($crr.Out)
    $ck = Giant-Checks $cq $crr.Vs[0]
    Check ($tag + '-census-' + $cw.W + '-gdi bbox strip spans the viewport width, 1-2px tall') $ck.ShapeOk ("bbox l=$($ck.Bb[0]) t=$($ck.Bb[1]) $($ck.Bw)x$($ck.Bh) viewport=$($crr.Vs[0])x$($crr.Vs[1])")
    if ($ck.ShapeOk) {
        Check ($tag + '-census-' + $cw.W + '-gdi gradient survived (quarter means differ >= 40, no extreme run >= 8)') (($ck.Diff -ge 40) -and ($ck.MaxRun -lt 8)) ('leftMean={0:N1} rightMean={1:N1} diff={2:N1} maxExtremeRun={3}' -f $ck.Lq, $ck.Rq, $ck.Diff, $ck.MaxRun)
        if ($cw.Seam) {
            $seamBound = 3 * $ck.MedianDelta + 2
            Check ($tag + '-census-' + $cw.W + '-gdi sliced-path seam scan (max |dmean| <= 3*median + 2)') ($ck.MaxDelta -le $seamBound) ('medianDelta={0:N4} maxDelta={1:N4} bound={2:N4}' -f $ck.MedianDelta, $ck.MaxDelta, $seamBound)
        }
    }
    Write-Output ('  ' + $tag + ' census evidence: width=' + $cw.W + ' path=' + $pathTxt + ' strip=' + $ck.Bw + 'x' + $ck.Bh + ' leftMean=' + ('{0:N2}' -f $ck.Lq) + ' rightMean=' + ('{0:N2}' -f $ck.Rq) + ' maxExtremeRun=' + $ck.MaxRun + ' medianDelta=' + ('{0:N4}' -f $ck.MedianDelta) + ' maxDelta=' + ('{0:N4}' -f $ck.MaxDelta))
    Kill-Riviv
    Reset-Ini ''
}

# ---------------------------------------------------------------------------
# S5: dump failure = exit 2, stderr says so, no modal hang.
# ---------------------------------------------------------------------------
Reset-Ini (Scene-Ini 'warp' 0 800 600 $null)
$badOut = Join-Path $Stage 'no\such\dir\out.png'
$p = Start-Riv ("`"$png96`" -dump-viewport `"$badOut`"") 's5.err'
$main = Wait-Main $p
$adopted = Wait-Title $p 'hash96' 12000
$code = Close-Main $p $main
$err = Read-Err 's5.err'
Check 'S5 dump to a missing directory: exit 2, stderr mentions the failure, no hang' (($code -eq 2) -and $err.Contains('dump-viewport') -and $err.Contains('failed')) ("exit=$code adopted=$adopted stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S6: frame-time baseline (record-only). Launch->exit wall time of the dump
# run, 3 runs per scene per renderer. Includes process start, PS
# Start-Process overhead, adoption wait polls, (scenes b/c) calibration and
# the WM_CLOSE dump - a comparability baseline, not a product-only number.
# ---------------------------------------------------------------------------
$one2one = { param($m) [void][S81]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }
$s6scenes = @(
    @{ Name = 'o2o-800x600'; Img = $png800; Fill = 0; Extra = $null; TW = $null; TH = $null; Cmds = $one2one },
    @{ Name = 'mag2-96x64'; Img = $png96; Fill = 1; Extra = (@('mag_filter=0') + $BgMagenta); TW = 256; TH = 128; Cmds = $null },
    @{ Name = 'shrink2-640x480'; Img = $png640; Fill = 0; Extra = @('shrink_blit_mode=1'); TW = 320; TH = 240; Cmds = $null },
    @{ Name = 'bannerfit-40000x256'; Img = $pngBanner; Fill = 0; Extra = @('shrink_blit_mode=1'); TW = $null; TH = $null; Cmds = $null }
)
foreach ($sc in $s6scenes) {
    foreach ($rnd in @('warp', 'gdi')) {
        $times = @()
        for ($i = 0; $i -lt 3; $i++) {
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $r = Run-Scene $rnd $sc.Fill $sc.Extra $sc.Img ('s6-' + $sc.Name + '-' + $rnd + $i + '.png') ('s6-' + $rnd + $i + '.err') $sc.TW $sc.TH $sc.Cmds $null
            $sw.Stop()
            $times += $sw.ElapsedMilliseconds
            if ($r.Code -ne 0) { Write-Output ('  S6 note: ' + $sc.Name + '/' + $rnd + ' run ' + $i + ' exit=' + $r.Code) }
        }
        Write-Output ('S6 baseline: scene=' + $sc.Name + ' renderer=' + $rnd + ' ms1=' + $times[0] + ' ms2=' + $times[1] + ' ms3=' + $times[2])
    }
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# Golden summary + teardown: the staged ini must never outlive the run.
# ---------------------------------------------------------------------------
$gj = Golden-Judge $dumps
Write-Output ('GOLDENS: ' + (Golden-Summary $gj) + ' dir=' + $GoldenDir)
$iniCleaned = -not (Test-Path $Ini)
$leftover = Get-Process riviv -ErrorAction SilentlyContinue
if ($leftover) { $leftover | Stop-Process -Force }
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage }
else { Write-Output ('FAILURES: evidence kept in ' + $Stage) }
Write-Output ('SMOKE81 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip + ' iniCleaned=' + $iniCleaned)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
