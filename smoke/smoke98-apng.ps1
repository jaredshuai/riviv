# smoke98 - #98 APNG animation through the existing frame pipeline.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Harness
# skeleton cribbed from smoke80/81/82: staged exe under
# %TEMP%\riviv-98-smoke, poll-based waits, Add-Type C# probe helper,
# SetProcessDPIAware, GetExitCodeProcess on the captured live handle.
#
# Fixtures are hand-built APNG bytes in C# (no .NET APNG encoder exists):
# PNG chunks with fresh CRC32, zlib streams as 0x78 0x01 + DeflateStream +
# adler32 (smoke81's verified zlib recipe), acTL/fcTL/IDAT/fdAT with the
# SHARED fcTL/fdAT sequence counter starting at 0 (the png crate hard-
# checks the ordering). The hostile variant patches the first fcTL's
# width beyond the IHDR canvas and re-CRCs the chunk - the same surgery
# as the loader unit tests, landing the failure BEFORE any frame decodes
# (png parse-time validate), which is what keeps the old display.
#
# Observable surface (#98):
#   - an acTL-bearing PNG loads as an animation: the status bar's frame
#     counter part reads "n / m" with m > 1 (SB_GETTEXT on the
#     msctls_statusbar32 child; the part is EMPTY while m <= 1, so the
#     poll itself filters loading/static states)
#   - a hostile fcTL over a loaded animation keeps the old display (the
#     counter stays live), adopts the failed file's title, shows the
#     failure verdict in the status main part, opens NO dialog, and the
#     window still closes with exit 0
#   - the stdin: pipe decodes the same animation (own window, #65)
#   - a STATIC PNG's -dump-viewport output is byte-identical between the
#     master parity exe and the branch exe (the zero-regression A/B the
#     issue's acceptance demands; both runs under the same pinned ini
#     rect so the letterboxed scene is geometrically identical)
#
# Scenarios:
#   S0 fixture self-checks: chunk walk of test.apng (acTL before IDAT,
#      fcTL before IDAT/fdAT, shared seq 0..N-1, 2x fdAT) + GDI+ decodes
#      the fixture (frame 0 red, 64x64 - the independent first-frame eye)
#   S1 file open -> frame counter "n / 3", title adopted, WM_CLOSE exit 0
#   S2 second-instance hostile open over the live animation -> old
#      display kept, failed title adopted, verdict text shown, no dialog,
#      exit 0
#   S3 stdin: pipe -> own window, frame counter "n / 3", exit 0
#   S4 static PNG A/B dump byte equality (branch vs master parity exe)

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')

$ErrorActionPreference = 'Stop'
[void][System.Reflection.Assembly]::LoadWithPartialName('System.Drawing')

Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Runtime.InteropServices;

public class S98 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int max);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    // ---- status bar reading: SB_GETTEXTLENGTH ONLY. Cross-process
    // SB_GETTEXT (a pointer-carrying message) returns empty on this
    // machine (#40's UIPI trap) - lengths are the reliable channel. Part
    // layout per status.rs: [main][preload?][pos][rgb][frame][dimension]
    // with dimension always LAST, so the frame counter sits at count-2
    // and its text is EMPTY while the frame total <= 1 (text.rs) - the
    // length alone separates animated from static.
    public static int GetPartCount(IntPtr sb) {
        return (int)SendMessageW(sb, 0x0406, IntPtr.Zero, IntPtr.Zero).ToInt64(); // SB_GETPARTS (WM_USER+6); lParam NULL -> returns the count
    }
    public static int GetPartLen(IntPtr sb, int part) {
        long lr = SendMessageW(sb, 0x040C, (IntPtr)part, IntPtr.Zero).ToInt64();
        return (int)(lr & 0xFFFF);
    }

    // ---- APNG fixture writing ----
    static void PutBe32(byte[] b, int o, uint v) {
        b[o] = (byte)(v >> 24); b[o+1] = (byte)(v >> 16); b[o+2] = (byte)(v >> 8); b[o+3] = (byte)v;
    }
    static void PutBe16(byte[] b, int o, ushort v) { b[o] = (byte)(v >> 8); b[o+1] = (byte)v; }
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
    static byte[] Solid(int w, int h, byte r, byte g, byte b) {
        byte[] px = new byte[w * h * 4];
        for (int i = 0; i < w * h; i++) { px[i*4] = r; px[i*4+1] = g; px[i*4+2] = b; px[i*4+3] = 255; }
        return px;
    }

    /// 3 full-canvas 64x64 frames (red/blue/green), 500 ms each: the
    /// standard animation fixture. Chunk order: IHDR acTL fcTL IDAT
    /// fcTL fdAT fcTL fdAT IEND with the shared sequence 0,1,2,3,4.
    public static void WriteApng3(string path, int w, int h) {
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 6; // RGBA8
        Chunk(ms, "IHDR", ihdr);
        byte[] actl = new byte[8];
        PutBe32(actl, 0, 3); PutBe32(actl, 4, 0); // 3 frames, infinite
        Chunk(ms, "acTL", actl);
        uint seq = 0;
        byte[][] frames = { Solid(w, h, 200, 60, 10), Solid(w, h, 10, 60, 200), Solid(w, h, 10, 200, 60) };
        for (int f = 0; f < 3; f++) {
            byte[] fctl = new byte[26];
            PutBe32(fctl, 0, seq++);
            PutBe32(fctl, 4, (uint)w); PutBe32(fctl, 8, (uint)h);
            PutBe32(fctl, 12, 0); PutBe32(fctl, 16, 0); // x/y offset
            PutBe16(fctl, 20, 50); PutBe16(fctl, 22, 100); // 50/100 s = 500 ms
            fctl[24] = 0; fctl[25] = 0; // dispose none, blend source
            Chunk(ms, "fcTL", fctl);
            byte[] payload = Zlib(Scanlines(w, h, frames[f]));
            if (f == 0) {
                Chunk(ms, "IDAT", payload);
            } else {
                byte[] fdat = new byte[4 + payload.Length];
                PutBe32(fdat, 0, seq++);
                Array.Copy(payload, 0, fdat, 4, payload.Length);
                Chunk(ms, "fdAT", fdat);
            }
        }
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }

    /// A plain static PNG (single IDAT, no animation chunks) - the A/B
    /// zero-regression fixture.
    public static void WriteStaticPng(string path, int w, int h) {
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 6;
        Chunk(ms, "IHDR", ihdr);
        Chunk(ms, "IDAT", Zlib(Scanlines(w, h, Solid(w, h, 200, 60, 10))));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }

    /// A static PNG carrying an iCCP profile (the #77 adobe-like fixture
    /// as base64, zlib per the smoke81 recipe) - the ICM-path A/B pair
    /// (external review R2-4: the plain pair alone leaves prepare_transform
    /// uncovered).
    public static void WriteStaticPngIccp(string path, int w, int h, string iccB64) {
        byte[] icc = Convert.FromBase64String(iccB64);
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 6;
        Chunk(ms, "IHDR", ihdr);
        byte[] kw = Encoding.ASCII.GetBytes("ICC profile");
        byte[] head = new byte[kw.Length + 2];
        kw.CopyTo(head, 0); head[kw.Length] = 0; head[kw.Length + 1] = 0;
        byte[] z = Zlib(icc);
        byte[] iccp = new byte[head.Length + z.Length];
        head.CopyTo(iccp, 0); z.CopyTo(iccp, head.Length);
        Chunk(ms, "iCCP", iccp);
        Chunk(ms, "IDAT", Zlib(Scanlines(w, h, Solid(w, h, 200, 60, 10))));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }

    /// A palette + tRNS static PNG (2 entries: 50%-transparent red and
    /// opaque blue, 8px checkerboard) - the alpha-composite + shrink-tier
    /// A/B pair (external review R2-4; 1200x1200 in the pinned 800x600
    /// window makes the fit a real shrink through the filter table).
    public static void WritePltePng(string path, int w, int h) {
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 3; // palette
        Chunk(ms, "IHDR", ihdr);
        Chunk(ms, "PLTE", new byte[] { 200, 60, 10, 10, 60, 200 });
        Chunk(ms, "tRNS", new byte[] { 128, 255 });
        byte[] idx = new byte[w * h];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) idx[y * w + x] = (byte)((((x / 8) + (y / 8)) % 2));
        byte[] raw = new byte[h * (1 + w)];
        for (int y = 0; y < h; y++) { raw[y * (1 + w)] = 0; Array.Copy(idx, y * w, raw, y * (1 + w) + 1, w); }
        Chunk(ms, "IDAT", Zlib(raw));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }

    /// Patch the FIRST fcTL's width (data offset 4) beyond the canvas and
    /// re-CRC the chunk - hostile subframe bounds, caught at png parse
    /// time (before any frame decodes, keeping the old display).
    public static void MakeHostile(string src, string dst) {
        byte[] all = File.ReadAllBytes(src);
        int pos = 8;
        while (pos + 12 <= all.Length) {
            int len = (all[pos] << 24) | (all[pos+1] << 16) | (all[pos+2] << 8) | all[pos+3];
            string kind = Encoding.ASCII.GetString(all, pos + 4, 4);
            if (kind == "fcTL") {
                PutBe32(all, pos + 8 + 4, 9999); // width field -> beyond IHDR
                byte[] crcIn = new byte[4 + len];
                Array.Copy(all, pos + 4, crcIn, 0, 4 + len);
                byte[] crc = new byte[4]; PutBe32(crc, 0, Crc32(crcIn));
                Array.Copy(crc, 0, all, pos + 8 + len, 4);
                File.WriteAllBytes(dst, all);
                return;
            }
            pos += 12 + len;
        }
        throw new Exception("no fcTL found to patch");
    }
}
'@

# ---------------------------------------------------------------------------
# Harness (smoke82 skeleton)
# ---------------------------------------------------------------------------
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

[void][S98]::SetProcessDPIAware()
Kill-Riviv

$Stage = Join-Path $env:TEMP 'riviv-98-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); exit 2 }
$RunExe = Join-Path $Stage 'riviv.exe'
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied, stage ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini))) 'Copy-Item failed or leftover ini'

$ParitySrc = Join-Path $env:TEMP 'riviv-98-parity\riviv-master.exe'
$ParityExe = Join-Path $Stage 'riviv-master.exe'
$parityReady = Test-Path $ParitySrc
if ($parityReady) { Copy-Item $ParitySrc $ParityExe -Force }
$script:gateIncomplete = $false   # set by S4 when its REQUIRED parity arm is missing

function Reset-Ini($text) {
    if (Test-Path $Ini) { Remove-Item $Ini -Force }
    if ($text -ne '') { [IO.File]::WriteAllText($Ini, $text) }
}
function Start-Riv($argStr, $errName) {
    $errPath = Join-Path $Stage $errName
    if (Test-Path $errPath) { Remove-Item $errPath -Force }
    $script:RawP = Start-Process -FilePath $RunExe -ArgumentList $argStr -PassThru -RedirectStandardError $errPath
    $script:RawH = $script:RawP.Handle
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
        [void][S98]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit(12000)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S98]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else { return -2 }
        return $code
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    return -1
}
function Sb-Of($main) {
    if (($main -eq $null) -or ($main -eq [IntPtr]::Zero)) { return [IntPtr]::Zero }
    return [S98]::FindWindowExW($main, [IntPtr]::Zero, 'msctls_statusbar32', [NullString]::Value)
}
# The frame-counter part's TEXT LENGTH (SB_GETTEXTLENGTH at part count-2;
# the text is "n / m" only while the loaded total m > 1, empty otherwise).
# -1 = no status bar / no parts.
function Get-FrameLen($main) {
    $sb = Sb-Of $main
    if ($sb -eq [IntPtr]::Zero) { return -1 }
    $count = [S98]::GetPartCount($sb)
    if ($count -lt 2) { return -1 }
    return [S98]::GetPartLen($sb, $count - 2)
}
# The status main part's text length (the verdict/verdict-clear channel).
function Get-MainLen($main) {
    $sb = Sb-Of $main
    if ($sb -eq [IntPtr]::Zero) { return -1 }
    return [S98]::GetPartLen($sb, 0)
}
# Any #32770 dialog owned by pid (the "no popup on user-level failure" arm).
function Count-Dialogs($procId) {
    $script:DlgHits = 0
    $cb = {
        param($h, $lp)
        $owner = 0
        [void][S98]::GetWindowThreadProcessId($h, [ref]$owner)
        if ($owner -eq $script:DlgPid) {
            $cn = New-Object System.Text.StringBuilder 64
            [void][S98]::GetClassNameW($h, $cn, 64)
            if ($cn.ToString() -eq '#32770') { $script:DlgHits = $script:DlgHits + 1 }
        }
        return $true
    }
    $script:DlgPid = $procId
    [void][S98]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:DlgHits
}

# ---------------------------------------------------------------------------
# S0 fixtures
# ---------------------------------------------------------------------------
$Apng = Join-Path $Stage 'test.apng'
[S98]::WriteApng3($Apng, 64, 64)
$Bad = Join-Path $Stage 'bad98.apng'
[S98]::MakeHostile($Apng, $Bad)
$Still = Join-Path $Stage 'still98.png'
[S98]::WriteStaticPng($Still, 64, 64)

function Get-ChunkKinds($path) {
    $b = [IO.File]::ReadAllBytes($path)
    $kinds = @()
    $seqs = @()
    $pos = 8
    while ($pos + 12 -le $b.Length) {
        $len = ($b[$pos] -shl 24) -bor ($b[$pos+1] -shl 16) -bor ($b[$pos+2] -shl 8) -bor $b[$pos+3]
        if ($pos + 12 + $len -gt $b.Length) { break }
        $kind = [Text.Encoding]::ASCII.GetString($b, $pos + 4, 4)
        $kinds += $kind
        if (($kind -eq 'fcTL') -or ($kind -eq 'fdAT')) {
            $seq = ($b[$pos+8] -shl 24) -bor ($b[$pos+9] -shl 16) -bor ($b[$pos+10] -shl 8) -bor $b[$pos+11]
            $seqs += $seq
        }
        $pos += 12 + $len
    }
    return @{ Kinds = $kinds; Seqs = $seqs }
}
$ch = Get-ChunkKinds $Apng
$wantKinds = @('IHDR','acTL','fcTL','IDAT','fcTL','fdAT','fcTL','fdAT','IEND')
$kindsOk = (($ch.Kinds -join ',') -eq ($wantKinds -join ','))
$seqsOk = (($ch.Seqs -join ',') -eq '0,1,2,3,4')
Check 'S0a test.apng chunk structure: acTL-before-IDAT, fcTL gates every image, shared seq 0-4' ($kindsOk -and $seqsOk) ("kinds=[$($ch.Kinds -join ',')] seqs=[$($ch.Seqs -join ',')]")

$gdiOk = $false; $gdiDetail = 'unreadable'
try {
    $img = [Drawing.Image]::FromFile($Apng)
    $c = $img.GetPixel(32, 32)
    $gdiOk = ($img.Width -eq 64) -and ($img.Height -eq 64) -and ($c.R -gt 150) -and ($c.G -lt 110) -and ($c.B -lt 60)
    $gdiDetail = ("{0}x{1} center=({2},{3},{4})" -f $img.Width, $img.Height, $c.R, $c.G, $c.B)
    $img.Dispose()
} catch { $gdiDetail = 'exception: ' + $_.Exception.Message }
Check 'S0b GDI+ independently decodes the fixture (64x64, frame 0 red)' $gdiOk $gdiDetail

$chBad = Get-ChunkKinds $Bad
$badPatched = (($chBad.Kinds -join ',') -eq ($wantKinds -join ','))
# The patch must have LANDED (external review R4: a chunk-kind walk alone
# cannot tell a hostile file from the intact animation - if the width
# surgery silently missed, every S2 assertion would pass vacuously).
function Get-FirstFctlWidth($path) {
    $b = [IO.File]::ReadAllBytes($path)
    $pos = 8
    while ($pos + 12 -le $b.Length) {
        $len = ($b[$pos] -shl 24) -bor ($b[$pos+1] -shl 16) -bor ($b[$pos+2] -shl 8) -bor $b[$pos+3]
        if ($pos + 12 + $len -gt $b.Length) { break }
        $kind = [Text.Encoding]::ASCII.GetString($b, $pos + 4, 4)
        if ($kind -eq 'fcTL') {
            # [int] casts: PS byte-typed -shl wraps back to a byte and the
            # high bytes vanish (9999 = 0x270F read back as 0x0F = 15).
            return (([int]$b[$pos+12] -shl 24) -bor ([int]$b[$pos+13] -shl 16) -bor ([int]$b[$pos+14] -shl 8) -bor [int]$b[$pos+15])
        }
        $pos += 12 + $len
    }
    return -1
}
$badWidth = Get-FirstFctlWidth $Bad
Check 'S0c bad98.apng structurally identical AND the hostile fcTL width really patched to 9999' ($badPatched -and ($badWidth -eq 9999)) ("kinds=[$($chBad.Kinds -join ',')] firstFctlWidth=$badWidth")

# ---------------------------------------------------------------------------
# S1 file open -> animation
# ---------------------------------------------------------------------------
Reset-Ini ''
$p1 = Start-Riv ('"' + $Apng + '"') 's1.err'
$main1 = Wait-Main $p1
$adopted1 = Wait-Title $p1 'test.apng' 12000
# "n / 3" is exactly 5 chars. The length-only channel (SB_GETTEXT cross-
# process returns empty on this machine) cannot distinguish a dropped
# third frame ("1 / 2" is also 5) - that residual is accepted here and
# covered by the unit suite instead; 5 still separates animated from
# static/loading (0 or the verdict text in the MAIN part).
$found1 = Wait-Until { $script:len1 = Get-FrameLen $main1; ($script:len1 -eq 5) } 15000
$len1 = $script:len1
$code1 = Close-Main $p1 $main1
Check 'S1a window + title adopted for test.apng' (($main1 -ne [IntPtr]::Zero) -and $adopted1) ('main=' + $main1 + ' adopted=' + $adopted1)
Check 'S1b frame counter part carries "n / m" text (len == 5; empty while static)' ($found1 -and ($len1 -eq 5)) ('framePartLen=' + $len1)
Check 'S1c WM_CLOSE exit code 0' ($code1 -eq 0) ('code=' + $code1)
Kill-Riviv

# ---------------------------------------------------------------------------
# S1b3 (external review R5): the animation actually ADVANCES over time -
# the one assertion the frame-counter length channel is structurally blind
# to ("1 / 3" is 5 chars for every n; a build whose timer never fires
# passes S1b/S2b/S3b). Two timed WM_CLOSE dumps of the same fixture:
# ~200 ms lands in frame 0 [0,500), ~1200 ms in frame 2 [1000,1500) -
# launch overhead only ever ADDS time and both windows keep >=300 ms of
# margin to their interval edges.
# ---------------------------------------------------------------------------
function Timed-Dump($ms, $outName, $errName) {
    $out = Join-Path $Stage $outName
    if (Test-Path $out) { Remove-Item $out -Force }
    Reset-Ini ''
    $p = Start-Riv ('"' + $Apng + '" -dump-viewport "' + $out + '"') $errName
    $main = Wait-Main $p
    $null = Wait-Title $p 'test.apng' 12000
    Start-Sleep -Milliseconds $ms
    $code = Close-Main $p $main
    return @{ Out = $out; Code = $code }
}
function Dump-CenterColor($path) {
    $bmp = [Drawing.Bitmap]::FromFile($path)
    $c = $bmp.GetPixel([int]($bmp.Width / 2), [int]($bmp.Height / 2))
    $bmp.Dispose()
    return $c
}
$tA = Timed-Dump 200 's1b3-f0.png' 's1b3-a.err'
$tB = Timed-Dump 1200 's1b3-f2.png' 's1b3-b.err'
$advOk = $false
$advDetail = 'no dumps'
if ((Test-Path $tA.Out) -and (Test-Path $tB.Out)) {
    $cA = Dump-CenterColor $tA.Out
    $cB = Dump-CenterColor $tB.Out
    $hA = (Get-FileHash $tA.Out -Algorithm SHA256).Hash
    $hB = (Get-FileHash $tB.Out -Algorithm SHA256).Hash
    $redHit = ([math]::Abs($cA.R - 200) -le 30) -and ([math]::Abs($cA.G - 60) -le 30) -and ([math]::Abs($cA.B - 10) -le 30)
    $greenHit = ([math]::Abs($cB.R - 10) -le 30) -and ([math]::Abs($cB.G - 200) -le 30) -and ([math]::Abs($cB.B - 60) -le 30)
    $advOk = ($tA.Code -eq 0) -and ($tB.Code -eq 0) -and $redHit -and $greenHit -and ($hA -ne $hB)
    $advDetail = ('f0=(' + $cA.R + ',' + $cA.G + ',' + $cA.B + ') f2=(' + $cB.R + ',' + $cB.G + ',' + $cB.B + ') hashEq=' + ($hA -eq $hB) + ' codes=' + $tA.Code + '/' + $tB.Code)
} else { $advDetail = 'dumpA=' + (Test-Path $tA.Out) + ' dumpB=' + (Test-Path $tB.Out) }
Check 'S1b3 animation advances: @200ms dump center is frame-0 red, @1200ms is frame-2 green, hashes differ' $advOk $advDetail
Kill-Riviv

# ---------------------------------------------------------------------------
# S2 hostile open over the live animation (second-instance handoff).
# Instance 1 launches with -dump-viewport so the WM_CLOSE dump becomes
# PIXEL evidence of the kept display (external review R3-6: the frame
# counter is only a proxy; the center of the dumped scene must be one of
# the animation's colors - a cleared display would letterbox white there).
# ---------------------------------------------------------------------------
Reset-Ini ''
$S2Dump = Join-Path $Stage 's2-keep.png'
if (Test-Path $S2Dump) { Remove-Item $S2Dump -Force }
$p2 = Start-Riv ('"' + $Apng + '" -dump-viewport "' + $S2Dump + '"') 's2.err'
$main2 = Wait-Main $p2
$null = Wait-Until { $script:len2a = Get-FrameLen $main2; ($script:len2a -eq 5) } 15000
$before2 = $script:len2a
# Differential baseline: after a COMPLETED load the status main part is
# empty (no Loading indicator, no verdict) - so the post-failure mainLen
# > 0 below can only be the failure verdict, not a leftover indicator.
$null = Wait-Until { $script:main2pre = Get-MainLen $main2; ($script:main2pre -eq 0) } 10000
$main2pre = $script:main2pre
# The #21 trap: a handoff arriving within add_command_line_timeout (500 ms
# default) of instance 1's own command-line processing is APPEND mode - the
# bad file would join the playlist instead of replacing the display. Sleep
# past the window so the forwarded open takes the replace path.
Start-Sleep -Milliseconds 900

# The second instance hands its command line to the running one and exits.
$errPath2 = Join-Path $Stage 's2-fwd.err'
if (Test-Path $errPath2) { Remove-Item $errPath2 -Force }
$p2b = Start-Process -FilePath $RunExe -ArgumentList ('"' + $Bad + '"') -PassThru -RedirectStandardError $errPath2
$null = $p2b.WaitForExit(12000)
$titleOk2 = Wait-Title $p2 'bad98' 12000
# Poll the verdict with PERSISTENCE (external review R4: a single nonzero
# main-part length is also true while the transient "Loading..." text is
# up - a snapshot taken there would green the scenario before the failure
# lands). The verdict PERSISTS until the next open; two equal nonzero
# samples 500 ms apart can only be it. The frame counter must stay live
# throughout (the old animation is the display).
$verdictOk2 = $false
$mainLen2 = -1
$deadline = [DateTime]::UtcNow.AddSeconds(12)
while ([DateTime]::UtcNow -lt $deadline) {
    $l1 = Get-MainLen $main2
    if ($l1 -gt 0) {
        Start-Sleep -Milliseconds 500
        $l2 = Get-MainLen $main2
        $cnt = Get-FrameLen $main2
        if (($l2 -eq $l1) -and ($l2 -gt 0) -and ($cnt -eq 5)) { $verdictOk2 = $true; $mainLen2 = $l2; break }
    } else {
        Start-Sleep -Milliseconds 100
    }
}

$afterLen2 = Get-FrameLen $main2
$dialogs2 = Count-Dialogs $p2.Id
$alive2 = (-not $p2.HasExited)
$diag2 = ('p2bExited=' + $p2b.HasExited + ' rivivProcs=' + @(Get-Process riviv -ErrorAction SilentlyContinue).Count + ' title="' + $p2.MainWindowTitle + '"')
$code2 = Close-Main $p2 $main2
# Pixel evidence: the dump's center (the fitted 64x64 image's spot) must
# still carry one of the animation's solid colors; the surrounding
# letterbox stays white. Any mid-play frame is accepted.
$keepOk2 = $false
$keepDetail2 = 'no dump'
if (Test-Path $S2Dump) {
    try {
        $bmp = [Drawing.Bitmap]::FromFile($S2Dump)
        $cx = [int]($bmp.Width / 2)
        $cy = [int]($bmp.Height / 2)
        $c = $bmp.GetPixel($cx, $cy)
        $bmp.Dispose()
        foreach ($want in @(@(200, 60, 10), @(10, 60, 200), @(10, 200, 60))) {
            if (([math]::Abs($c.R - $want[0]) -le 30) -and ([math]::Abs($c.G - $want[1]) -le 30) -and ([math]::Abs($c.B - $want[2]) -le 30)) { $keepOk2 = $true }
        }
        $keepDetail2 = ('center=(' + $c.R + ',' + $c.G + ',' + $c.B + ') at ' + $cx + ',' + $cy)
    } catch { $keepDetail2 = 'exception: ' + $_.Exception.Message }
}
Check 'S2a forwarded instance adopted the failed file title (request-time)' ($titleOk2) $diag2
Check 'S2b old animation display kept (frame counter part still live, len == 5)' ($alive2 -and ($afterLen2 -eq 5)) ('framePartLen=' + $afterLen2 + ' before=' + $before2)
Check 'S2b2 pixel evidence: the dumped viewport center is one of the animation colors' $keepOk2 $keepDetail2
Check 'S2c failure verdict visible (persistent nonzero main part; Loading is transient, two equal samples exclude it)' (($main2pre -eq 0) -and $verdictOk2) ('mainPartLen pre=' + $main2pre + ' post=' + $mainLen2 + ' verdictOk=' + $verdictOk2)
Check 'S2d no dialog popup owned by the window' ($dialogs2 -eq 0) ('dialogs=' + $dialogs2)
Check 'S2e WM_CLOSE exit code 0' ($code2 -eq 0) ('code=' + $code2)
Kill-Riviv

# ---------------------------------------------------------------------------
# S3 stdin: pipe
# ---------------------------------------------------------------------------
Reset-Ini ''
$errPath3 = Join-Path $Stage 's3.err'
if (Test-Path $errPath3) { Remove-Item $errPath3 -Force }
# cmd type pumps raw bytes (PS piping would re-encode); #65's recipe.
$null = Start-Process -FilePath 'cmd.exe' -ArgumentList ('/c', ('type "' + $Apng + '" | "' + $RunExe + '" stdin:')) -PassThru -WindowStyle Hidden
# Target by EXE PATH, never by name: riviv is the developer's daily
# viewer, and @(Get-Process riviv)[0] would happily WM_CLOSE a real
# browsing window (external review R2-5). Exactly one staged instance
# must exist - a developer instance is ignored, two staged ones fail.
$null = Wait-Until { $script:r3 = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe }); ($script:r3.Count -eq 1) } 15000
$riv3 = $null
if ($script:r3 -ne $null -and $script:r3.Count -eq 1) { $riv3 = $script:r3[0] }
$main3 = [IntPtr]::Zero
if ($riv3 -ne $null) { $main3 = Wait-Main $riv3 }
$devInstances = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -ne $RunExe }).Count
$titleOk3 = $false
if ($riv3 -ne $null) { $titleOk3 = Wait-Title $riv3 'stdin' 12000 }
# "n / 3" is exactly 5 chars (see S1b's channel note).
$found3 = Wait-Until { $script:len3 = Get-FrameLen $main3; ($script:len3 -eq 5) } 15000
$len3 = $script:len3
$code3 = -9
if ($riv3 -ne $null) {
    # Handle capture guarded: an early-dead process would throw on .Handle
    # here (external review R3-6) - the scenario must FAIL cleanly, not
    # abort the script; GetExitCodeProcess is skipped without the handle.
    $h3 = [IntPtr]::Zero
    if (-not $riv3.HasExited) { $h3 = $riv3.Handle }
    if ($main3 -ne [IntPtr]::Zero) { [void][S98]::PostMessage($main3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    if ($riv3.WaitForExit(12000) -and ($h3 -ne [IntPtr]::Zero)) {
        $c3 = 0
        [void][S98]::GetExitCodeProcess($h3, [ref]$c3)
        $code3 = $c3
    } else { Stop-Process -Id $riv3.Id -Force -ErrorAction SilentlyContinue; $code3 = -1 }
}
Check 'S3a stdin: pipe launches exactly one staged window with the stdin title' (($riv3 -ne $null) -and ($main3 -ne [IntPtr]::Zero) -and $titleOk3) ('staged=' + $(if ($riv3) { '1' } else { '0' }) + ' main=' + $main3 + ' title=' + $(if ($riv3) { $riv3.MainWindowTitle } else { '' }) + ' devInstancesIgnored=' + $devInstances)
Check 'S3b stdin: animation decoded (frame counter part len == 5)' ($found3 -and ($len3 -eq 5)) ('framePartLen=' + $len3)
Check 'S3c stdin: WM_CLOSE exit code 0' ($code3 -eq 0) ('code=' + $code3)
Kill-Riviv

# ---------------------------------------------------------------------------
# S4 static PNG A/B dump byte equality (branch vs master parity exe).
# REQUIRED gate (external review R2-3): a missing parity exe is NOT a
# silent skip - the run exits 2 with the stage kept.
# Three pairs (external review R2-4: the plain pair alone left the arm
# migration's risk faces uncovered): (a) plain RGBA 1:1; (b) iCCP-bearing
# PNG under icm=1 - the prepare_transform path; (c) palette+tRNS 1200x1200
# - the alpha composite AND a real shrink through the filter table.
# ---------------------------------------------------------------------------
$IccB64 = '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'
if ($parityReady) {
    $StillIcc = Join-Path $Stage 'still-icc98.png'
    [S98]::WriteStaticPngIccp($StillIcc, 64, 64, $IccB64)
    $Plte = Join-Path $Stage 'plte98.png'
    [S98]::WritePltePng($Plte, 1200, 1200)

    # Pinned rect so both runs letterbox identically; identical ini text
    # re-staged before each launch (WM_CLOSE writes the ini back).
    $rectIni = "x=40`r`ny=40`r`nwide=800`r`nhigh=600`r`nauto_zoom=0`r`n"
    $iniPlain = "[riviv]`r`n" + $rectIni + "icm=0`r`n"
    $iniIcm = "[riviv]`r`n" + $rectIni + "icm=1`r`n"

    function Run-Dump($exePath, $img, $out, $errName, $iniText) {
        Reset-Ini $iniText
        if (Test-Path $out) { Remove-Item $out -Force }
        $errPath = Join-Path $Stage $errName
        if (Test-Path $errPath) { Remove-Item $errPath -Force }
        $pp = Start-Process -FilePath $exePath -ArgumentList ('"' + $img + '" -dump-viewport "' + $out + '"') -PassThru -RedirectStandardError $errPath
        $ph = $pp.Handle   # capture WHILE ALIVE (smoke80 lesson)
        $mm = Wait-Main $pp
        Start-Sleep -Milliseconds 1200   # paint settle before the WM_CLOSE dump
        if (($mm -ne $null) -and ($mm -ne [IntPtr]::Zero)) {
            [void][S98]::PostMessage($mm, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        $cc = -1
        if ($pp.WaitForExit(12000)) {
            $c = 0
            [void][S98]::GetExitCodeProcess($ph, [ref]$c)
            $cc = $c
        } else {
            Stop-Process -Id $pp.Id -Force -ErrorAction SilentlyContinue
        }
        return @{ Out = $out; Code = $cc; Main = $mm }
    }
    function Compare-Pair($name, $img, $iniText) {
        $outA = Join-Path $Stage ($name + '-master.png')
        $outB = Join-Path $Stage ($name + '-branch.png')
        $rA = Run-Dump $ParityExe $img $outA ($name + '-master.err') $iniText
        $rB = Run-Dump $RunExe $img $outB ($name + '-branch.err') $iniText
        $hashA = $null; $hashB = $null
        if ((Test-Path $outA) -and (Test-Path $outB)) {
            $hashA = (Get-FileHash $outA -Algorithm SHA256).Hash
            $hashB = (Get-FileHash $outB -Algorithm SHA256).Hash
        }
        Check ('S4 ' + $name + ': both dumps exist, exit 0, byte-identical branch == master parity') ((Test-Path $outA) -and (Test-Path $outB) -and ($rA.Code -eq 0) -and ($rB.Code -eq 0) -and ($hashA -eq $hashB)) ('a=' + (Test-Path $outA) + '/' + $rA.Code + ' b=' + (Test-Path $outB) + '/' + $rB.Code + ' hashEq=' + ($hashA -eq $hashB))
    }
    Compare-Pair 's4a-plain' $Still $iniPlain
    Compare-Pair 's4b-icc' $StillIcc $iniIcm
    Compare-Pair 's4c-plte-shrink' $Plte $iniPlain
    Reset-Ini ''
    Kill-Riviv
} else {
    $script:gateIncomplete = $true
    Skip-Scenario 'S4 static A/B (all three pairs)' ('REQUIRED gate: master parity exe missing at ' + $ParitySrc + ' - build it (git worktree add <tmp> 60c9f0f; cargo build --release inside; copy the exe) and rerun; exiting 2')
}

# ---------------------------------------------------------------------------
# Teardown. The ini reset is UNCONDITIONAL (every scenario's WM_CLOSE may
# have written one back, including when S4 was skipped for a missing
# parity exe - external review R1's false-red), and a leftover riviv
# process is a FAILURE, not something to sweep before asserting.
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
# Leftover STAGED instances are a failure; the developer's own riviv
# windows (a different exe path) are exempt - same targeting rule as S3.
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { ($_.Path -eq $RunExe) -or ($_.Path -eq $ParityExe) })
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S9 teardown: stage ini cleaned, no staged riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')
Write-Output ('RESULT: pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip + ' gateIncomplete=' + $script:gateIncomplete)
if ($script:fail -gt 0) { Write-Output ('FAILURES: evidence kept in ' + $Stage); exit 1 }
if ($script:gateIncomplete) { Write-Output ('GATE INCOMPLETE: evidence kept in ' + $Stage); exit 2 }
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
