# smoke81 - #81 filter tier remap + mip retirement + renderer default flip.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Harness
# skeleton cribbed verbatim from smoke80-d2d.ps1 (staged exe+ini under
# %TEMP%\riviv-81-smoke, WM_CLOSE dump channel, DPI-aware probe, poll-based
# waits, GetExitCodeProcess on the captured live handle).
#
# #90 removed the GDI render arm: renderer=gdi now maps to auto with a
# migration note on stderr, and every former warp-vs-gdi twin comparison is
# judged against the frozen golden corpora instead (both corpora were
# frozen from the GDI arm while it still existed, in the domains where
# warp == gdi was proven byte-identical).
#
# Scenarios:
#   S1 renderer keys: missing key -> "riviv: renderer=auto backend=...";
#      frobnicate -> "unrecognized renderer value" + "using auto" + the
#      auto line; gdi -> the #90 migration: exit 0, "riviv: renderer=gdi
#      was removed, using auto" AND "riviv: renderer=auto backend=d2d/"
#      on stderr, and "backend=gdi" never appears.
#   S2 L0/L1 integer magnify byte-exactness (core): 96x64 hash-pattern
#      source, fill_window=1 recipe, warp arm only (the gdi twin died
#      with #90). k=3,4 calibrate the riviv_view child to EXACTLY k*96 x
#      k*64 (fill renders edge to edge) -> the warp (NEAREST) dump is
#      compared byte-for-byte against the frozen golden81 dump, is exactly
#      k*96 x k*64, and is pixel-exact vs the replication model
#      expected[x,y] = src[x/k, y/k].
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
#      magenta, image box boundary +-0px, golden byte-compare): box
#      32..223 x 15..142, side margins 32, top/bottom margins 15.
#   S3 L2 filter tiers (filter domain: NOT byte-equal corpus, the gdi
#      cross-arm MAE/phase oracles died with the arm): mag=1 on a 256x256
#      smooth gradient at exact 2x (warp LINEAR; two runs must be
#      byte-identical - determinism is the remaining guard - content
#      stats recorded); shrink=1 on 640x480 at exact 2x down (warp
#      HIGH_QUALITY_CUBIC, same determinism treatment, no blown-out
#      pixels outside block halos); shrink=0 (warp NEAREST at exact 2x
#      down: two runs byte-identical, the phase census stays as failure
#      DIAGNOSTICS).
#   S4 giant correctness: 40000x256 banner (luminance gradient + a 4000px
#      2px-period stripe band at x=20000) at fit in a 1000x700 window ->
#      warp uploads 40000 (no D2D gate) and CUBIC-deep-shrinks it; the
#      warp content assertions stay (bbox shape, averaged stripes ~128,
#      monotone gradient, seam scan); the gdi-arm banner checks died with
#      the arm. 16777217x1 gradient giant via the hand-written PNG
#      writer: the warp run asserts no gate line + no gdi fallback + the
#      close-time stats line naming the giant form + the gradient
#      content. The renderer=gdi giant content runs and the 2^22 relief
#      boundary census (old S4g-S4i and S4m/n/p/q) are retired with the
#      GDI arm (#90: GiantRelief and the relief path no longer exist; the
#      mag >=2^22 KNOWN GAP went with them).
#   S5 dump failure: -dump-viewport into a missing directory -> exit 2,
#      stderr says so, no modal hang.
#   S6 frame-time baseline (record-only): launch->exit wall time of the
#      dump run, 3 runs per scene, warp renderer only (the gdi rows died
#      with the arm), printed as a table.
#   S10 golden90 corpus (#90): smoke/golden90/ froze five GDI-arm dumps at
#      commit 5996944 in the byte-equal NEAREST domain (1:1 exact, integer
#      magnify k=2/k=3, letterboxed both backgrounds, rotated 90 then 1:1).
#      Each scene runs under renderer=warp with a FRESH fixture
#      (HashSource + SaveRgba regenerated before EVERY instance - the
#      rot90 scene's EditRotate90 fires the shell rotate90 verb which
#      REWRITES the fixture file on disk, so one shared fixture across
#      instances poisons the second one) and must byte-match its golden:
#      5 hard assertions. The same 5 scenes also run once under
#      renderer=auto (hardware): all five byte-matching goldens promotes
#      them to hard assertions; any mismatch stays record-only with
#      byte-diff counts.
#
# Golden corpora: smoke/golden81/ holds the frozen gdi-arm dumps (S2 exact
# k=2,3,4 + padded), smoke/golden90/ the five-scene #90 corpus. -Regolden
# (re)creates golden81 from this run and -Regolden90 golden90 (both
# regenerate from the warp arm); by default the dumps are compared against
# them; a missing golden is a SKIP-with-note.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe', [switch]$Regolden, [switch]$Regolden90)
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
    // S10 diagnostic: does a decoded 256x192 white-bg frame exactly equal
    // the HashSource(96,64) fixture drawn 1:1 at (80,64), optionally
    // rotated 180 degrees? Used ONLY on a golden byte-mismatch to separate
    // "the golden was frozen from a rotated on-disk fixture" (a corpus
    // defect: regolden) from a real renderer content error.
    public static bool MatchesWhiteHashModel(byte[] b, int w, int h, bool rot) {
        if (w != 256 || h != 192) return false;
        int sw = 96, sh = 64;
        byte[] src = new byte[sw * sh * 4];
        HashSource(sw, sh, src);
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            int di = (y * w + x) * 4;
            byte r, g, b2;
            if (x >= 80 && x < 176 && y >= 64 && y < 128) {
                int lx = x - 80, ly = y - 64;
                int sx = rot ? (sw - 1 - lx) : lx;
                int sy = rot ? (sh - 1 - ly) : ly;
                int si = (sy * sw + sx) * 4;
                r = src[si]; g = src[si + 1]; b2 = src[si + 2];
            } else { r = 255; g = 255; b2 = 255; }
            if (b[di + 2] != r || b[di + 1] != g || b[di] != b2 || b[di + 3] != 255) return false;
        }
        return true;
    }
    public static void HashSource(int w, int h, byte[] a) {
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
Check 'S0 fixtures built' ((Test-Path $png96) -and (Test-Path $pngPad) -and (Test-Path $pngGrad) -and (Test-Path $png640) -and (Test-Path $png800) -and (Test-Path $pngBanner) -and (Test-Path $giantPng)) 'a fixture PNG is missing'

# ---------------------------------------------------------------------------
# S1: the default renderer flip (#81): missing key AND unrecognized value
# both land on auto; the legacy `gdi` word MIGRATES to auto (#90, S1c).
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
$bcLine = '(none)'
if ($err -match 'riviv: renderer=[^\r\n]+') { $bcLine = $Matches[0] }
Check 'S1c renderer=gdi -> #90 migration (removed note + auto backend line, never backend=gdi)' (($code -eq 0) -and $err.Contains('riviv: renderer=gdi was removed, using auto') -and $err.Contains('riviv: renderer=auto backend=d2d/') -and (-not $err.Contains('backend=gdi'))) ("exit=$code line=[$bcLine] stderr=[$($err.Trim())]")
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2 (core): integer magnify byte-exactness, k = 2, 3, 4, warp arm only.
# k=3/4: exact k*W x k*H viewport, fill renders edge to edge. k=2: the
# window min track width clamps the view to 256, where fill_window=1 still
# renders EXACTLY 2x (192x128) centered at (32,0) with 32px magenta sides.
# The byte oracle is the frozen golden81 dump (frozen from the GDI arm
# pre-#90, in the warp==gdi byte-equal domain).
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
    $dumps["k$k"] = $w.Out
    $ranOk = ($w.Code -eq 0) -and (Test-Path $w.Out)
    Check "S2a-k$k warp dump ran clean (exit 0, file exists)" $ranOk ("warp exit=$($w.Code) warpPng=$(Test-Path $w.Out) warpView=$($w.Vs[0])x$($w.Vs[1])")
    if (-not $ranOk) {
        Write-Output ("S2-k$k warp stderr: " + ($w.Err.Trim()))
        continue
    }
    $qw = [Px]::Load($w.Out)
    Check "S2d-k$k dump is the calibrated viewport $($geo.TW)x$($geo.TH)" (($qw.W -eq $geo.TW) -and ($qw.H -eq $geo.TH)) ("warp=$($qw.W)x$($qw.H) want=$($geo.TW)x$($geo.TH)")
    # Margins pure magenta where the clamped viewport leaves any (k=2 sides).
    $margOk2 = $true
    $margDet2 = ''
    if (($geo.BL -gt 0) -or ($geo.BT -gt 0)) {
        foreach ($q in @($qw)) {
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
        if ($margDet2 -eq '') { $margDet2 = 'all margin strips pure magenta' }
    } else {
        $margDet2 = 'exact fill - no margin strips by geometry'
    }
    Check "S2e-k$k margins pure background color" $margOk2 $margDet2
    $model = [Px]::UpsampleK($src96, 96, 64, $k)
    $mw = [Px]::CompareRgb($qw.B, $qw.W, $geo.BL, $geo.BT, $model, $geo.BW)
    Check "S2f-k$k image box == replication model src[x/k,y/k] at ($($geo.BL),$($geo.BT)) (exact RGB)" (($mw -eq $null)) "warp-first=$mw"
    $golden = Join-Path $GoldenDir ("s2-k$k-gdi.png")
    if (-not (Test-Path $golden)) {
        Skip-Scenario "S2g-k$k golden compare" "no golden yet (run -Regolden to freeze smoke\golden81\s2-k$k-gdi.png)"
    } else {
        $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($w.Out))
        Check "S2g-k$k warp dump matches frozen golden (the gdi-arm reference)" $same 'golden bytes differ from this run'
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
$dumps['pad'] = $padW2.Out
$padOk = ($padW2.Code -eq 0) -and (Test-Path $padW2.Out)
Check 'S2p-a padded warp dump ran clean' $padOk ("warp exit=$($padW2.Code) view=$($padW2.Vs[0])x$($padW2.Vs[1])")
if ($padOk) {
    $qw = [Px]::Load($padW2.Out)
    Check 'S2p-b padded dump is 256x158' (($qw.W -eq $PADW) -and ($qw.H -eq $PADH)) ("warp=$($qw.W)x$($qw.H)")
    # Margins pure magenta (the whole frame outside the box).
    $margsOk = $true
    $margDetail = ''
    foreach ($q in @($qw)) {
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
    if ($margDetail -eq '') { $margDetail = 'all four strips pure magenta' }
    Check 'S2p-d margins pure background color' $margsOk $margDetail
    # Image box == the 2x replica at ($padL,$padT): boundary +-0 by build.
    $bw2 = [Px]::CompareRgb($qw.B, $qw.W, $padL, $padT, $src192, 192)
    Check 'S2p-e image box == 2x replica at (32,15), boundary +-0px (exact RGB)' (($bw2 -eq $null)) "warp-first=$bw2"
    $golden = Join-Path $GoldenDir 's2-pad-gdi.png'
    if (-not (Test-Path $golden)) {
        Skip-Scenario 'S2p-f golden compare' 'no golden yet (run -Regolden to freeze smoke\golden81\s2-pad-gdi.png)'
    } else {
        $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($padW2.Out))
        Check 'S2p-f warp dump matches frozen golden (the gdi-arm reference)' $same 'golden bytes differ from this run'
    }
}
if ($Regolden) {
    # Review pre-3 P3-2: never bless a run that failed - a regressed build
    # must not overwrite the frozen goldens (the same run's independent
    # anchors - the src[x/k,y/k] replication model and the golden90 corpus
    # in S10 - narrow but do not close that hole).
    if ($script:fail -gt 0) {
        Write-Output ('GOLDENS: regolden REFUSED - run has ' + $script:fail + ' failing check(s); fix before freezing new goldens')
    } else {
        foreach ($key in @('k2', 'k3', 'k4', 'pad')) {
            $src = $dumps[$key]
            if (($src -ne $null) -and (Test-Path $src)) { Copy-Item $src (Join-Path $GoldenDir ('s2-' + $key + '-gdi.png')) -Force }
        }
        Write-Output 'GOLDENS: (re)written from this run (-Regolden)'
    }
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: the L2 filter tiers. Filter domain - NOT byte-equal corpus: the
# cross-arm MAE/phase oracles died with the GDI arm, so each tier runs the
# warp arm TWICE and asserts determinism (two runs byte-identical); the
# old cross-arm statistics are recorded as evidence only.
# ---------------------------------------------------------------------------
# S3a mag=1: 256x256 smooth gradient at exact 2x (fill recipe, 512x512):
# warp LINEAR, two runs.
$m1w = Run-Scene 'warp' 1 @('mag_filter=1') $pngGrad 's3a-mag1-warp.png' 's3a-warp.err' 512 512 $null $null
$m1v = Run-Scene 'warp' 1 @('mag_filter=1') $pngGrad 's3a-mag1-warp2.png' 's3a-warp2.err' 512 512 $null $null
$m1ok = ($m1w.Code -eq 0) -and ($m1v.Code -eq 0) -and (Test-Path $m1w.Out) -and (Test-Path $m1v.Out)
Check 'S3a-mag1 both warp runs ran clean (exit 0, files exist)' $m1ok ("warp exit=$($m1w.Code) warp2 exit=$($m1v.Code) views=$($m1w.Vs[0])x$($m1w.Vs[1])/$($m1v.Vs[0])x$($m1v.Vs[1])")
if ($m1ok) {
    $qw = [Px]::Load($m1w.Out)
    $qb = [Px]::Load($m1v.Out)
    Check 'S3a-mag1 both dumps are exactly 512x512 (exact-fill recipe held)' (($qw.W -eq 512) -and ($qw.H -eq 512) -and ($qb.W -eq 512) -and ($qb.H -eq 512)) ("warp=$($qw.W)x$($qw.H) warp2=$($qb.W)x$($qb.H)")
    $eqA = [Px]::BytesEqual([IO.File]::ReadAllBytes($m1w.Out), [IO.File]::ReadAllBytes($m1v.Out))
    Check 'S3a-mag1 warp LINEAR deterministic: two runs byte-identical (the HALFTONE cross-arm oracle died with the gdi arm)' $eqA 'run1 vs run2 file bytes differ'
    $s = [Px]::StatPair($qw.B, $qb.B, 512, 512, $null)
    $maeTxt = 'maeR={0:N3} maeG={1:N3} maeB={2:N3} max={3}' -f $s[0], $s[1], $s[2], [int]$s[3]
    Write-Output ('  S3a record (warp self-consistency, gdi arm gone): ' + $maeTxt)
}
Kill-Riviv
Reset-Ini ''

# S3b shrink=1 (default tier): 640x480 at exactly half size (320x240):
# warp HIGH_QUALITY_CUBIC, two runs (the HALFTONE cross-arm oracle died
# with the gdi arm).
$exclFlat = @(26, 26, 48, 38, 206, 41, 58, 48, 121, 156, 53, 43)  # blocks dilated 8 src px, /2
$s1w = Run-Scene 'warp' 0 @('shrink_blit_mode=1') $png640 's3b-shrink1-warp.png' 's3b-warp.err' 320 240 $null $null
$s1v = Run-Scene 'warp' 0 @('shrink_blit_mode=1') $png640 's3b-shrink1-warp2.png' 's3b-warp2.err' 320 240 $null $null
$s1ok = ($s1w.Code -eq 0) -and ($s1v.Code -eq 0) -and (Test-Path $s1w.Out) -and (Test-Path $s1v.Out)
Check 'S3b-shrink1 both warp runs ran clean (exit 0, files exist)' $s1ok ("warp exit=$($s1w.Code) warp2 exit=$($s1v.Code)")
if ($s1ok) {
    $qw = [Px]::Load($s1w.Out)
    $qb = [Px]::Load($s1v.Out)
    Check 'S3b-shrink1 both dumps are exactly 320x240' (($qw.W -eq 320) -and ($qw.H -eq 240) -and ($qb.W -eq 320) -and ($qb.H -eq 240)) ("warp=$($qw.W)x$($qw.H) warp2=$($qb.W)x$($qb.H)")
    $eqB = [Px]::BytesEqual([IO.File]::ReadAllBytes($s1w.Out), [IO.File]::ReadAllBytes($s1v.Out))
    Check 'S3b-shrink1 warp CUBIC deterministic: two runs byte-identical' $eqB 'run1 vs run2 file bytes differ'
    $blowW = [Px]::CountOutside($qw.B, $qw.W, 320, 240, 4, 251, $exclFlat)
    Check 'S3b-shrink1 no blown-out pixels outside block halos' ($blowW -eq 0) "warpOutside=$blowW"
    $s = [Px]::StatPair($qw.B, $qb.B, 320, 240, $exclFlat)
    $maeTxt = 'maeR={0:N3} maeG={1:N3} maeB={2:N3} max={3} maxOutsideBlocks={4} diffPixels={5}' -f $s[0], $s[1], $s[2], [int]$s[3], [int]$s[4], [int]$s[5]
    Write-Output ('  S3b record (warp self-consistency, gdi arm gone): ' + $maeTxt)
}
Kill-Riviv
Reset-Ini ''

# S3c shrink=0: warp NEAREST at exact 2x down, two runs. The old byte
# equality oracle was warp-vs-gdi (COLORONCOLOR) - that arm is gone, so
# determinism across two warp runs is the assertion and the uniform
# source-phase census stays as failure DIAGNOSTICS.
$s0w = Run-Scene 'warp' 0 @('shrink_blit_mode=0') $png640 's3c-shrink0-warp.png' 's3c-warp.err' 320 240 $null $null
$s0v = Run-Scene 'warp' 0 @('shrink_blit_mode=0') $png640 's3c-shrink0-warp2.png' 's3c-warp2.err' 320 240 $null $null
$s0ok = ($s0w.Code -eq 0) -and ($s0v.Code -eq 0) -and (Test-Path $s0w.Out) -and (Test-Path $s0v.Out)
Check 'S3c-shrink0 both warp runs ran clean (exit 0, files exist)' $s0ok ("warp exit=$($s0w.Code) warp2 exit=$($s0v.Code)")
if ($s0ok) {
    $eq0 = [Px]::BytesEqual([IO.File]::ReadAllBytes($s0w.Out), [IO.File]::ReadAllBytes($s0v.Out))
    $diag0 = ''
    if (-not $eq0) {
        $cw = [Px]::PhaseCensus([Px]::Load($s0w.Out).B, 320, 0, 0, 320, 240, $photo640, 640, 480)
        $cv = [Px]::PhaseCensus([Px]::Load($s0v.Out).B, 320, 0, 0, 320, 240, $photo640, 640, 480)
        $diag0 = " run1: $cw | run2: $cv"
    }
    Check 'S3c-shrink0 warp(NEAREST) deterministic: two runs byte-identical (the gdi(COLORONCOLOR) byte-equality oracle died with the arm)' $eq0 ("file bytes differ -$diag0")
}
Kill-Riviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: giant correctness. Banner 40000x256: warp uploads (no D2D gate) and
# CUBIC-deep-shrinks; the gdi-arm banner twin died with the arm (#90).
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
$baOk = ($baw.Code -eq 0) -and (Test-Path $baw.Out)
Check 'S4a-banner warp dump ran clean (exit 0)' $baOk ("warp exit=$($baw.Code) warpAlive=$($baw.Alive)")
if ($baOk) {
    $gateW = $baw.Err.Contains('exceeds the D2D max bitmap')
    Check 'S4a-banner-warp no D2D gate line (40000 uploaded, CUBIC deep shrink really ran)' (-not $gateW) ("stderr=[$($baw.Err.Trim())]")
    $qw = [Px]::Load($baw.Out)
    $rw = Banner-Checks $qw
    Check 'S4b-banner-warp bbox: height <= 8, width == viewport width' (($rw.Bh -ge 1) -and ($rw.Bh -le 8) -and ($rw.Bw -eq $baw.Vs[0])) ("bbox l=$($rw.Bb[0]) t=$($rw.Bb[1]) $($rw.Bw)x$($rw.Bh) viewport=$($baw.Vs[0])x$($baw.Vs[1])")
    Check 'S4c-banner-warp stripes band mean luminance in [96,160] (averaged, no aliasing garbage)' (($rw.BandMean -ge 96) -and ($rw.BandMean -le 160)) ('bandMean={0:N1}' -f $rw.BandMean)
    Check 'S4d-banner-warp left-half monotone (+-2) and seam scan max <= 3*median+2' ($rw.Monotone -and ($rw.MaxExcl -le (3 * $rw.Median + 2))) ('worstDrop={0:N2} medianDelta={1:N3} maxExcl={2:N2} maxFull={3:N2}' -f $rw.WorstDrop, $rw.Median, $rw.MaxExcl, $rw.MaxFull)
    Write-Output ('  S4 banner record (warp only, gdi arm gone): bandMean={0:N1} maxFull={1:N2}' -f $rw.BandMean, $rw.MaxFull)
}
Kill-Riviv
Reset-Ini ''

# Extreme giant 16777217x1 horizontal gradient. Since #82 the warp run
# asserts the OPPOSITE of the old gate: the D2D arm draws it itself (no
# gate line, no gdi fallback), with the stats line naming the form
# (S4j/S4j2). The former renderer=gdi content run (old S4g-S4i) is retired
# with the GDI arm (#90).
$giantExtra = @('shrink_blit_mode=1')

$gwr = Run-Scene 'warp' 0 $giantExtra $giantPng 's4-giant-warp.png' 's4-giant-warp.err' $null $null $null 90000
$gwOk = ($gwr.Code -eq 0) -and (Test-Path $gwr.Out)
$gerr = $gwr.Err
$gMaxLine = ''
if ($gerr -match 'max_bitmap=(\d+)') { $gMaxLine = 'device max=' + $Matches[1] }
# #82: the giant no longer trips a gate and no longer falls back to GDI - the
# D2D arm draws it itself (an overview level, or tiles). The old assertion
# (gate line + 'rendering it via gdi') is what #82 removed; what replaces it
# is the close-time stats line naming the form, plus the dump succeeding.
$gStatsLevel = -1
$gStatsTiles = -1
$gStatsSeen = $gerr -match 'riviv: tiles level=(\d+) tiles=(\d+) base=(\d+)'
if ($gStatsSeen) {
    $gStatsLevel = [int]$Matches[1]
    $gStatsTiles = [int]$Matches[2]
}
$gForm = $gStatsSeen -and (($gStatsLevel -ge 1) -or ($gStatsTiles -ge 1))
$gOk = $gwOk -and ($gerr -match 'backend=d2d') -and (-not $gerr.Contains('exceeds the D2D max bitmap')) -and (-not $gerr.Contains('rendering it via gdi')) -and (-not $gerr.Contains('trying the gdi channel'))
Check 'S4j-giant-warp drawn by the d2d arm itself (no gate line, no gdi fallback, exit 0)' $gOk ("exit=$($gwr.Code) out=$(Test-Path $gwr.Out) $gMaxLine stderr=[$($gerr.Trim())]")
Check 'S4j2-giant-warp stats line names the giant form (level>=1 or tiles>=1)' $gForm "seen=$gStatsSeen level=$gStatsLevel tiles=$gStatsTiles"

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
# S4 boundary census RETIRED with the GDI arm (#90): the old census ran
# renderer=gdi over widths straddling the 2^22 STRETCH_SOURCE_STITCH_
# TRIGGER relief boundary (old S4m/n/p/q, fed by the Giant-Checks helper
# and the 4M/2^22/2^23/6.29M census fixtures). GiantRelief, the relief
# path and the trigger are deleted; the mag >=2^22 KNOWN GAP went with
# them. Giant content on the D2D arms is covered by S4j/S4k here and by
# smoke82 S3/S4.
# ---------------------------------------------------------------------------

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
# run, 3 runs per scene, warp renderer only (the gdi rows died with the
# arm, #90). Includes process start, PS Start-Process overhead, adoption
# wait polls, (scenes b/c) calibration and the WM_CLOSE dump - a
# comparability baseline, not a product-only number.
# ---------------------------------------------------------------------------
$one2one = { param($m) [void][S81]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero) }
$s6scenes = @(
    @{ Name = 'o2o-800x600'; Img = $png800; Fill = 0; Extra = $null; TW = $null; TH = $null; Cmds = $one2one },
    @{ Name = 'mag2-96x64'; Img = $png96; Fill = 1; Extra = (@('mag_filter=0') + $BgMagenta); TW = 256; TH = 128; Cmds = $null },
    @{ Name = 'shrink2-640x480'; Img = $png640; Fill = 0; Extra = @('shrink_blit_mode=1'); TW = 320; TH = 240; Cmds = $null },
    @{ Name = 'bannerfit-40000x256'; Img = $pngBanner; Fill = 0; Extra = @('shrink_blit_mode=1'); TW = $null; TH = $null; Cmds = $null },
    # The giant fit dump itself (external review AI1 P2-4 row, now on the
    # D2D overview/tile path since #90 deleted the per-paint GDI relief
    # build): a >=2^24 fit dump pays the mip/overview machinery.
    @{ Name = 'relieffit-16777217x1'; Img = $giantPng; Fill = 0; Extra = @('shrink_blit_mode=1'); TW = $null; TH = $null; Cmds = $null }
)
foreach ($sc in $s6scenes) {
    foreach ($rnd in @('warp')) {
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
# S10: the golden90 corpus (#90). smoke/golden90/ froze five GDI-arm dumps
# at commit 5996944 in the byte-equal NEAREST domain (at freeze time each
# scene's gdi dump == its warp dump byte for byte), so after the #90
# deletion a warp dump must reproduce each golden byte-for-byte: the
# frozen file is the oracle for the removed arm. EVERY scene regenerates
# its fixture fresh right before its instance (HashSource + SaveRgba):
# the rot90 scene's EditRotate90 fires the shell rotate90 verb which
# REWRITES the fixture file on disk, so one shared fixture across
# instances poisons the second one. The same scenes also run once under
# renderer=auto (hardware): all five byte-matching goldens promotes them
# to hard assertions; any mismatch stays record-only with byte-diff
# counts (never forced green).
# ---------------------------------------------------------------------------
$GoldenDir90 = Join-Path $PSScriptRoot 'golden90'
$CMD_ROTATE90 = 23   # menu.rs Cmd::EditRotate90.id()
$rot90Cmds = { param($m)
    [void][S81]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ROTATE90, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    [void][S81]::PostMessage($m, $WM_COMMAND, [IntPtr]$CMD_ONE2ONE, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 400
}
function New-Hash96Fixture([string]$path) {
    $s = [Px]::HashSource(96, 64)
    [Px]::SaveRgba($path, $s, 96, 64)
}
$g90Fixture = Join-Path $Stage 'g90hash96.png'
$s10scenes = @(
    @{ Name = 's1-one2one-gdi.png';       Fill = 1; Extra = (@('mag_filter=0', 'shrink_blit_mode=0') + $BgMagenta); TW = 256; TH = 192; Cmds = $one2one },
    @{ Name = 's2-magk2-gdi.png';         Fill = 1; Extra = (@('mag_filter=0') + $BgMagenta); TW = 256; TH = 128; Cmds = $null },
    @{ Name = 's3-magk3-gdi.png';         Fill = 1; Extra = (@('mag_filter=0') + $BgMagenta); TW = 288; TH = 192; Cmds = $null },
    @{ Name = 's4-rot90-one2one-gdi.png'; Fill = 1; Extra = (@('mag_filter=0') + $BgMagenta); TW = 256; TH = 192; Cmds = $rot90Cmds },
    @{ Name = 's5-one2one-white-gdi.png'; Fill = 1; Extra = @('mag_filter=0', 'shrink_blit_mode=0'); TW = 256; TH = 192; Cmds = $one2one }
)
$g90WarpDumps = @{}
foreach ($sc in $s10scenes) {
    New-Hash96Fixture $g90Fixture
    $r = Run-Scene 'warp' $sc.Fill $sc.Extra $g90Fixture ('s10-warp-' + $sc.Name) ('s10-warp-' + $sc.Name + '.err') $sc.TW $sc.TH $sc.Cmds $null
    $g90WarpDumps[$sc.Name] = $r.Out
    $golden = Join-Path $GoldenDir90 $sc.Name
    $ranOk = ($r.Code -eq 0) -and (Test-Path $r.Out)
    Check ('S10a ' + $sc.Name + ' warp run clean (exit 0, dump exists)') $ranOk ("exit=$($r.Code) view=$($r.Vs[0])x$($r.Vs[1]) stderr=[$($r.Err.Trim())]")
    if ((-not $ranOk) -or (-not (Test-Path $golden))) {
        if (-not (Test-Path $golden)) { Skip-Scenario ('S10b ' + $sc.Name + ' golden byte-compare') 'golden90 file missing (broken checkout)' }
        Kill-Riviv
        Start-Sleep -Milliseconds 500   # settle before the next fixture rewrite (see the loop-end note)
        Reset-Ini ''
        continue
    }
    $same = [Px]::BytesEqual([IO.File]::ReadAllBytes($golden), [IO.File]::ReadAllBytes($r.Out))
    if ($same) {
        Check ('S10b ' + $sc.Name + ' warp dump byte-identical to the frozen golden90 reference') $true 'byte-identical'
    } else {
        # Mismatch diagnosis (S10 s5 finding): separate "the golden was
        # frozen from a rotated on-disk fixture" (a corpus defect -> regolden)
        # from a real renderer content error (STOP). Runs only on mismatch.
        $qd = [Px]::Load($r.Out)
        $qg2 = [Px]::Load($golden)
        $pxDiff = 0
        if (($qd.W -eq $qg2.W) -and ($qd.H -eq $qg2.H)) {
            for ($i = 0; $i -lt $qd.B.Length; $i += 4) {
                if ($qd.B[$i] -ne $qg2.B[$i] -or $qd.B[$i + 1] -ne $qg2.B[$i + 1] -or $qd.B[$i + 2] -ne $qg2.B[$i + 2] -or $qd.B[$i + 3] -ne $qg2.B[$i + 3]) { $pxDiff++ }
            }
        }
        $dumpUpright = [Px]::MatchesWhiteHashModel($qd.B, $qd.W, $qd.H, $false)
        $goldenRot = [Px]::MatchesWhiteHashModel($qg2.B, $qg2.W, $qg2.H, $true)
        $diag = ''
        if ($dumpUpright -and $goldenRot) {
            $diag = ' DIAGNOSIS: dump == fresh-HashSource model AND golden == rot180(fixture) model - the GOLDEN was frozen from a 180-degree-rotated on-disk fixture (freeze-harness shared-fixture artifact); the renderer is correct, regolden the mismatching scene'
        }
        Check ('S10b ' + $sc.Name + ' warp dump byte-identical to the frozen golden90 reference') $false ("bytes differ, pixelDiffs=$pxDiff$diag")
    }
    # The rot90 shell verb may finish writing the fixture AFTER the
    # process exit - settle INSIDE the loop, before the next iteration
    # rewrites the fixture path (AI1 P3-7: the poison window is the
    # cross-scene handoff s4->s5, not the loop's end; the first freeze
    # was poisoned exactly there).
    Start-Sleep -Milliseconds 500
    Kill-Riviv
    Reset-Ini ''
}
# Hardware arm: record-only unless ALL five dumps byte-match the goldens.
$g90AutoMatches = 0
$g90AutoDetail = ''
foreach ($sc in $s10scenes) {
    New-Hash96Fixture $g90Fixture
    $r = Run-Scene 'auto' $sc.Fill $sc.Extra $g90Fixture ('s10-auto-' + $sc.Name) ('s10-auto-' + $sc.Name + '.err') $sc.TW $sc.TH $sc.Cmds $null
    $golden = Join-Path $GoldenDir90 $sc.Name
    $verdict = 'nodump'
    if (($r.Code -eq 0) -and (Test-Path $r.Out) -and (Test-Path $golden)) {
        $ga = [IO.File]::ReadAllBytes($golden)
        $gb = [IO.File]::ReadAllBytes($r.Out)
        if ([Px]::BytesEqual($ga, $gb)) {
            $verdict = 'match'
            $g90AutoMatches++
        } else {
            $n = -1
            if ($ga.Length -eq $gb.Length) {
                $n = 0
                for ($i = 0; $i -lt $ga.Length; $i++) { if ($ga[$i] -ne $gb[$i]) { $n++ } }
            }
            $verdict = 'differs bytes=' + $n
        }
    }
    $g90AutoDetail += (' ' + $sc.Name + '=' + $verdict)
    Start-Sleep -Milliseconds 500   # same cross-scene settle as the warp loop
    Kill-Riviv
    Reset-Ini ''
}
Write-Output ('  S10 auto-vs-golden record:' + $g90AutoDetail)
if ($g90AutoMatches -eq 5) {
    Check 'S10c all five auto (hardware) dumps byte-identical to the frozen golden90 references' ($g90AutoMatches -eq 5) ('all-match;' + $g90AutoDetail)
} else {
    Write-Output ('  S10 RECORD-ONLY: the hardware auto arm does not byte-match the goldens on this machine (' + $g90AutoMatches + '/5) - kept as recorded evidence, not asserted')
}
if ($Regolden90) {
    if ($script:fail -gt 0) {
        Write-Output ('GOLDENS90: regolden REFUSED - run has ' + $script:fail + ' failing check(s); fix before freezing new goldens')
    } else {
        foreach ($sc in $s10scenes) {
            $src = $g90WarpDumps[$sc.Name]
            if (($src -ne $null) -and (Test-Path $src)) { Copy-Item $src (Join-Path $GoldenDir90 $sc.Name) -Force }
        }
        Write-Output 'GOLDENS90: (re)written from the warp arm of this run (-Regolden90)'
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
