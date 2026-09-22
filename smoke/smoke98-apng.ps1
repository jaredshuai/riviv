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
if (-not $parityReady) { Skip-Scenario 'S4 static PNG A/B (all)' ('master parity exe missing: ' + $ParitySrc) }

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
Check 'S0c bad98.apng is structurally identical (only fcTL width + CRC differ)' $badPatched ("kinds=[$($chBad.Kinds -join ',')]")

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
# S2 hostile open over the live animation (second-instance handoff)
# ---------------------------------------------------------------------------
Reset-Ini ''
$p2 = Start-Riv ('"' + $Apng + '"') 's2.err'
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
Start-Sleep -Milliseconds 800   # let the failure verdict settle

$afterLen2 = Get-FrameLen $main2
$mainLen2 = Get-MainLen $main2
$dialogs2 = Count-Dialogs $p2.Id
$alive2 = (-not $p2.HasExited)
$diag2 = ('p2bExited=' + $p2b.HasExited + ' rivivProcs=' + @(Get-Process riviv -ErrorAction SilentlyContinue).Count + ' title="' + $p2.MainWindowTitle + '"')
$code2 = Close-Main $p2 $main2
Check 'S2a forwarded instance adopted the failed file title (request-time)' ($titleOk2) $diag2
Check 'S2b old animation display kept (frame counter part still live, len == 5)' ($alive2 -and ($afterLen2 -eq 5)) ('framePartLen=' + $afterLen2 + ' before=' + $before2)
Check 'S2c failure verdict visible in the status main part (differential: 0 before, > 0 after)' (($main2pre -eq 0) -and ($mainLen2 -gt 0)) ('mainPartLen pre=' + $main2pre + ' post=' + $mainLen2)
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
$riv3 = $null
$null = Wait-Until { $script:r3 = @(Get-Process riviv -ErrorAction SilentlyContinue)[0]; ($script:r3 -ne $null) } 15000
$riv3 = @(Get-Process riviv -ErrorAction SilentlyContinue)[0]
$main3 = [IntPtr]::Zero
if ($riv3 -ne $null) { $main3 = Wait-Main $riv3 }
$titleOk3 = $false
if ($riv3 -ne $null) { $titleOk3 = Wait-Title $riv3 'stdin' 12000 }
# "n / 3" is exactly 5 chars (see S1b's channel note).
$found3 = Wait-Until { $script:len3 = Get-FrameLen $main3; ($script:len3 -eq 5) } 15000
$len3 = $script:len3
$code3 = -9
if ($riv3 -ne $null) {
    $h3 = $riv3.Handle
    if ($main3 -ne [IntPtr]::Zero) { [void][S98]::PostMessage($main3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    if ($riv3.WaitForExit(12000)) {
        $c3 = 0
        [void][S98]::GetExitCodeProcess($h3, [ref]$c3)
        $code3 = $c3
    } else { Stop-Process -Id $riv3.Id -Force -ErrorAction SilentlyContinue; $code3 = -1 }
}
Check 'S3a stdin: pipe launches its own window with the stdin title' (($riv3 -ne $null) -and ($main3 -ne [IntPtr]::Zero) -and $titleOk3) ('riv=' + $(if ($riv3) { 'yes' } else { 'no' }) + ' main=' + $main3 + ' title=' + $(if ($riv3) { $riv3.MainWindowTitle } else { '' }))
Check 'S3b stdin: animation decoded (frame counter part len == 5)' ($found3 -and ($len3 -eq 5)) ('framePartLen=' + $len3)
Check 'S3c stdin: WM_CLOSE exit code 0' ($code3 -eq 0) ('code=' + $code3)
Kill-Riviv

# ---------------------------------------------------------------------------
# S4 static PNG A/B dump byte equality (branch vs master parity exe)
# ---------------------------------------------------------------------------
if ($parityReady) {
    # Pinned rect so both runs letterbox identically; identical ini text
    # re-staged before each launch (WM_CLOSE writes the ini back).
    $iniText = "[riviv]`r`nx=40`r`ny=40`r`nwide=800`r`nhigh=600`r`nauto_zoom=0`r`nicm=0`r`n"
    $outA = Join-Path $Stage 's4-master.png'
    $outB = Join-Path $Stage 's4-branch.png'

    function Run-Dump($exePath, $img, $out, $errName) {
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
    $rA = Run-Dump $ParityExe $Still $outA 's4-master.err'
    $rB = Run-Dump $RunExe $Still $outB 's4-branch.err'
    $hashA = $null; $hashB = $null
    if ((Test-Path $outA) -and (Test-Path $outB)) {
        $hashA = (Get-FileHash $outA -Algorithm SHA256).Hash
        $hashB = (Get-FileHash $outB -Algorithm SHA256).Hash
    }
    Check 'S4a both A/B dumps exist and exited 0' ((Test-Path $outA) -and (Test-Path $outB) -and ($rA.Code -eq 0) -and ($rB.Code -eq 0)) ('a=' + (Test-Path $outA) + '/' + $rA.Code + ' b=' + (Test-Path $outB) + '/' + $rB.Code)
    Check 'S4b static PNG dump byte-identical: branch == master parity' (($hashA -ne $null) -and ($hashA -eq $hashB)) ('hashA=' + $hashA + ' hashB=' + $hashB)
    Reset-Ini ''
    Kill-Riviv
}

# ---------------------------------------------------------------------------
# Teardown. The ini reset is UNCONDITIONAL (every scenario's WM_CLOSE may
# have written one back, including when S4 was skipped for a missing
# parity exe - external review R1's false-red), and a leftover riviv
# process is a FAILURE, not something to sweep before asserting.
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue)
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S9 teardown: stage ini cleaned, no riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')
Write-Output ('RESULT: pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
if ($script:fail -eq 0) { Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue }
else { Write-Output ('FAILURES: evidence kept in ' + $Stage) }
if ($script:fail -gt 0) { exit 1 }
