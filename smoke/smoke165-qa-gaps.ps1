# smoke165 - #165 QA-gap machine coverage: five scenarios that had no
# machine coverage before this script. ASCII-only source (PS 5.1
# ANSI/param-binding trap, #79 lesson). Harness cribbed from:
#   smoke156: S0 staging (foreign-riviv FATAL -> Kill-StagedRiviv path
#      targeting -> wipe+copy -> ini pre-checks -> RIVIV_S156_ICC cargo
#      emitter -> C# PNG writer incl. the iCCP chunk), Run-Dump /
#      Reset-Ini / Close-Main / GetExitCodeProcess-on-the-live-handle,
#      Get-Lines comma-return, Png-Dims, Run-CargoTest, Read-Err
#      shared-read.
#   smoke78:  S3/S4 hand-built HDROP -> WM_DROPFILES posted straight to the
#      riviv_view child (the only in-repo drag-drop precedent).
#   smoke152: SVG fixtures, Px pixel reads over the dump PNG, Wait-Title,
#      pinned-ini letterbox discipline.
#
# Command channel facts (src/menu.rs Cmd::ALL index+1; HelpAbout=121
# cross-verified by smoke124): FileOpenFile(Ctrl+O)=1, ViewRefresh(F5)=41,
# NavNext=107, NavPrev=108. Posted as WM_COMMAND(wParam=id, lParam=0) to
# the owner main window. The Ctrl+O dialog is the comdlg32 GetOpenFileNameW
# dialog: class #32770 owned by the staged process; the filter combo
# carries control id 1136 (cmb13; possibly a ComboBoxEx32 wrapping an inner
# ComboBox - GetDlgItem does not recurse, so ComboBox-class descendants are
# enumerated as extra candidates); items read with CB_GETCOUNT /
# CB_GETLBTEXT (kernel-marshaled cross-process). Close ladder: BM_CLICK to
# GetDlgItem(dlg, 2), then WM_CLOSE as fallback (WM_COMMAND IDCANCEL
# straight to the dialog does not close it on this machine).
#
# Filter contract (src/text.rs dialog_filter test): SOME item of the filter
# dropdown contains *.svg (the full label is "All Image Files
# (*.bmp;...;*.svg)"; the list may carry "All Files (*.*)" after it).
#
# Title contract (src/text.rs title_wide): "<path or filename> - riviv" -
# the filename clause is what the S2/S3/S4 "ends with" assertions match.
#
# Scenarios:
#   S0 stage + fixtures:
#      S0a staged exe copied, stage ini absent, source exe dir ini absent
#      S0b cargo emitter wrote p3.icc (6680 bytes, test result: ok)
#      S0c client geometry probe (the dump of b.png IS the client area;
#          wide.png is then sized VW x VH, iCCP = p3.icc)
#      S0d all fixtures exist and are non-zero (b.png, wide.png, a.svg,
#          c.svg, trans.svg)
#   S1 Ctrl+O filter contains *.svg:
#      S1a dialog appeared; S1b some item contains *.svg; S1c dialog
#      closed via the cancel ladder; S1d WM_CLOSE -> exit 0
#   S2 playlist navigates the SVG neighbors (mix/ = a.svg, b.png, c.svg):
#      S2a adopted b.png; S2b NavNext -> title *.svg; S2c NavNext -> title
#      *.svg again; S2d NavPrev -> title *.svg or b.png; S2e exit 0
#   S3 SVG drag-drop swaps the image:
#      S3a riviv_view child found; S3b HDROP a.svg adopted; S3c HDROP
#      b.png adopted; S3d exit 0
#   S4 wide-ICC AC-arm storm (renderer=d2d, wide.png = client full size):
#      S4a adopted + output-surface=ac-scrgb >= 1; S4b/S4c/S4d ViewRefresh
#      x3 (alive, title unchanged); S4e Ctrl+O dialog cycle; S4f HDROP
#      b.png adopted; S4g HDROP wide.png adopted; S4h stderr clean (no
#      "display effect failure", no panic); S4i WM_CLOSE -> exit 0 and the
#      process gone within 15s
#   S5 SVG transparency composites over the pinned windowed background:
#      S5a run1 (8,8,8): transparent point +-12; S5b run1 center
#      green-dominant; S5c run2 (200,40,40): same point +-30 per channel;
#      S5d run2 center green-dominant; S5e the two transparent points
#      differ >= 100 on some channel
#   S6 teardown: stage ini removed unconditionally, no staged riviv left;
#      on PASS the whole stage (this log included) is removed.

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

public class S165 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumChildWindows(IntPtr hwnd, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dlg, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="FindWindowExW")] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="SendMessageTimeoutW")] public static extern IntPtr SendMessageTimeoutSb(IntPtr h, uint m, IntPtr w, StringBuilder l, uint flags, uint timeout, out IntPtr result);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalAlloc(uint flags, uint bytes);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr hmem);
    [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr hmem);

    // ---- PNG fixture writing (smoke156's verified chunk/zlib recipe) ----
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

    /// b.png: a plain 64x64 solid PNG with NO iCCP (the narrow fixture).
    public static void WriteBPng(string path) {
        WritePng(path, 64, 64, Solid(64, 64, 200, 60, 10), null);
    }

    /// wide.png: VW x VH RGBA8 with the iCCP profile bytes and a
    /// deterministic gradient (smoke156's WriteWidePng verbatim).
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

    /// Hand-built HDROP for one file (smoke78 S4 recipe): a DROPFILES
    /// header (pFiles=20, fWide=1) followed by the path, double-NUL ended.
    public static IntPtr BuildHdrop(string path) {
        byte[] bytes = Encoding.Unicode.GetBytes(path + "\0\0");
        int total = 20 + bytes.Length;
        IntPtr hmem = GlobalAlloc(0x0002, (uint)total);
        IntPtr mem = GlobalLock(hmem);
        byte[] off = BitConverter.GetBytes((uint)20);
        Marshal.Copy(off, 0, mem, 4);
        Marshal.WriteInt32(mem, 8, 1);
        Marshal.Copy(bytes, 0, (IntPtr)((long)mem + 20), bytes.Length);
        GlobalUnlock(hmem);
        return hmem;
    }
}
public class PngData { public int W; public int H; public byte[] B; }
public static class Px {
    // Dump PNG reader, BGRA byte order (smoke152's Load verbatim).
    public static PngData Load(string path) {
        using (System.Drawing.Bitmap bmp = new System.Drawing.Bitmap(path)) {
            PngData p = new PngData(); p.W = bmp.Width; p.H = bmp.Height;
            System.Drawing.Imaging.BitmapData d = bmp.LockBits(
                new System.Drawing.Rectangle(0, 0, p.W, p.H),
                System.Drawing.Imaging.ImageLockMode.ReadOnly,
                System.Drawing.Imaging.PixelFormat.Format32bppArgb);
            p.B = new byte[p.W * p.H * 4];
            Marshal.Copy(d.Scan0, p.B, 0, p.B.Length);
            bmp.UnlockBits(d);
            return p;
        }
    }
}
'@ -ReferencedAssemblies @('System.Drawing')

# ---------------------------------------------------------------------------
# Log: the script's own tee target is %TEMP%\riviv-s165\smoke165-run1.log
# (transcript). It lives inside the stage, so the stage wipe below spares
# this one file, and the S6 PASS cleanup removes it with the stage.
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-s165'
$LogPath = Join-Path $Stage 'smoke165-run1.log'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'
if (-not (Test-Path $Stage)) { New-Item -ItemType Directory -Path $Stage | Out-Null }
if (Test-Path $LogPath) { Remove-Item $LogPath -Force -ErrorAction SilentlyContinue }
$script:logOpen = $false
try {
    Start-Transcript -Path $LogPath | Out-Null
    $script:logOpen = $true
} catch {
    Write-Host ('WARN transcript failed, continuing without tee: ' + $_.Exception.Message)
}
function Stop-Log {
    if ($script:logOpen) {
        try { Stop-Transcript | Out-Null } catch { }
        $script:logOpen = $false
    }
}

# ---------------------------------------------------------------------------
# Harness (cribbed; detail lines go to the host, never into the pipeline)
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
    if ($ps) {
        Write-Host ('killing staged riviv pids: ' + (($ps | ForEach-Object { $_.Id }) -join ','))
        $ps | Stop-Process -Force
        Start-Sleep -Milliseconds 300
    }
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
function Get-Title($p) {
    $p.Refresh()
    return $p.MainWindowTitle
}
function Wait-Title($p, $leaf, $ms) {
    for ($i = 0; $i -lt [int]($ms / 100); $i++) {
        Start-Sleep -Milliseconds 100
        $t = Get-Title $p
        if ($t -like ('*' + $leaf + '*')) { return $true }
    }
    return $false
}
# The title is "<path or filename> - riviv"; these pollers match the end.
function Wait-TitleEnds($p, $leaf, $ms) {
    $suffix = $leaf + ' - riviv'
    for ($i = 0; $i -lt [int]($ms / 100); $i++) {
        Start-Sleep -Milliseconds 100
        $t = Get-Title $p
        if (($null -ne $t) -and $t.EndsWith($suffix)) { return $true }
    }
    return $false
}
function Wait-TitleChangedFrom($p, $old, $ms) {
    # Poll a background process's title until it differs from $old (posted
    # messages to a background process can service arbitrarily late).
    $t = $old
    for ($i = 0; $i -lt [int]($ms / 100); $i++) {
        Start-Sleep -Milliseconds 100
        $t = Get-Title $p
        if (($t -ne $old) -and ($t -ne '')) { return $t }
    }
    return $t
}
function Close-Main($p, $main, $waitMs) {
    if (($main -ne $null) -and ($main -ne [IntPtr]::Zero)) {
        [void][S165]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    if ($p.WaitForExit($waitMs)) {
        $code = 0
        if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
            [void][S165]::GetExitCodeProcess($script:RawH, [ref]$code)
        } else { return -2 }
        return $code
    }
    Write-Host ('process did not exit within ' + $waitMs + 'ms, killing pid ' + $p.Id)
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
    if ($adopted) { Start-Sleep -Milliseconds 1500 }   # paint settle
    $code = Close-Main $p $main 30000
    return @{ Out = $out; Code = $code; Main = $main; Adopted = $adopted; Err = (Read-Err $errName) }
}
# Every whole line of $err matching $pattern (anchored regexes; CRLF-safe).
# The leading comma keeps the ARRAY shape through PowerShell pipeline
# unroll (a single hit would otherwise degrade to a scalar string).
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

# ---- window/dialog helpers -------------------------------------------------
function Get-Class($h) {
    $sb = New-Object System.Text.StringBuilder 256
    [void][S165]::GetClassName($h, $sb, 256)
    return $sb.ToString()
}
function Get-ChildrenOfClass($hwnd, $cls) {
    # EnumChildWindows walks ALL descendants (not only direct children).
    $script:enumHits = @()
    $cb = [S165+EnumProc]{ param($h, $lp) $script:enumHits += $h; return $true }
    [void][S165]::EnumChildWindows($hwnd, $cb, [IntPtr]::Zero)
    $list = @()
    foreach ($h in $script:enumHits) { if ((Get-Class $h) -eq $cls) { $list += $h } }
    $script:enumHits = $null
    return ,$list
}
function Find-OwnDialogs($procId) {
    # All top-level #32770 windows owned by the staged process (FindWindowW
    # alone could hit another process's dialog, so enumerate and filter).
    $script:topHits = @()
    $cb = [S165+EnumProc]{ param($h, $lp) $script:topHits += $h; return $true }
    [void][S165]::EnumWindows($cb, [IntPtr]::Zero)
    $found = @()
    foreach ($h in $script:topHits) {
        if ((Get-Class $h) -eq '#32770') {
            $wpid = 0
            [void][S165]::GetWindowThreadProcessId($h, [ref]$wpid)
            if ($wpid -eq $procId) { $found += $h }
        }
    }
    $script:topHits = $null
    return ,$found
}
function Wait-OwnDialog($procId, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        $d = Find-OwnDialogs $procId
        if ($d.Count -ge 1) { return $d[0] }
        Start-Sleep -Milliseconds 100
    }
    $d = Find-OwnDialogs $procId
    if ($d.Count -ge 1) { return $d[0] }
    return [IntPtr]::Zero
}
function Wait-DialogGone($dlg, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not [S165]::IsWindow($dlg)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (-not [S165]::IsWindow($dlg))
}
function Close-OpenDialog($dlg) {
    # Ladder (machine-verified): BM_CLICK to the Cancel button first,
    # WM_CLOSE as the fallback. Never WM_COMMAND IDCANCEL straight to the
    # dialog - it does not close it on this machine.
    $cancel = [S165]::GetDlgItem($dlg, 2)
    Write-Host ('close ladder: dlg=' + $dlg + ' cancelBtn=' + $cancel)
    if ($cancel -ne [IntPtr]::Zero) {
        [void][S165]::PostMessage($cancel, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)  # BM_CLICK
        if (Wait-DialogGone $dlg 10000) { return $true }
        Write-Host 'BM_CLICK did not close the dialog, falling back to WM_CLOSE'
    }
    [void][S165]::PostMessage($dlg, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CLOSE
    return (Wait-DialogGone $dlg 5000)
}
function Get-ComboCandidates($dlg) {
    # The filter combo: GetDlgItem(dlg, 1136) first (cmb13). If it is a
    # ComboBoxEx32, drill to the inner ComboBox (GetDlgItem does not
    # recurse). Then add every ComboBox-class descendant as a candidate.
    $cands = @()
    $c1136 = [S165]::GetDlgItem($dlg, 1136)
    if ($c1136 -ne [IntPtr]::Zero) {
        $cn = Get-Class $c1136
        Write-Host ('id1136 class: ' + $cn)
        if ($cn -eq 'ComboBoxEx32') {
            $inner = [S165]::FindWindowExW($c1136, [IntPtr]::Zero, 'ComboBox', [NullString]::Value)
            if ($inner -ne [IntPtr]::Zero) { $cands += $inner }
        } elseif ($cn -eq 'ComboBox') {
            $cands += $c1136
        }
    }
    foreach ($h in (Get-ChildrenOfClass $dlg 'ComboBox')) {
        if ($cands -notcontains $h) { $cands += $h }
    }
    return ,$cands
}
function Read-ComboItems($cb) {
    # CB_GETCOUNT(0x0146) / CB_GETLBTEXTLEN(0x0149) / CB_GETLBTEXT(0x0148)
    # via SendMessageTimeout (SMTO_ABORTIFHUNG); the kernel marshals the
    # CB_GETLBTEXT buffer across the process boundary.
    $items = @()
    $res = [IntPtr]::Zero
    $r = [S165]::SendMessageTimeout($cb, 0x0146, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$res)
    if ($r -eq [IntPtr]::Zero) { Write-Host 'CB_GETCOUNT timed out'; return ,$items }
    $count = [int]$res
    Write-Host ('combo ' + $cb + ' item count: ' + $count)
    for ($i = 0; $i -lt $count; $i++) {
        $rl = [S165]::SendMessageTimeout($cb, 0x0149, [IntPtr]$i, [IntPtr]::Zero, 2, 2000, [ref]$res)
        if ($rl -eq [IntPtr]::Zero) { continue }
        $len = [int]$res
        if ($len -le 0) { $items += ''; continue }
        $sb = New-Object System.Text.StringBuilder ($len + 2)
        $rt = [S165]::SendMessageTimeoutSb($cb, 0x0148, [IntPtr]$i, $sb, 2, 2000, [ref]$res)
        if ($rt -ne [IntPtr]::Zero) { $items += $sb.ToString() }
    }
    return ,$items
}
function Read-FilterItems($dlg) {
    # Poll: the combo is populated during WM_INITDIALOG, which may trail
    # the window's creation by a beat.
    $all = @()
    $deadline = [DateTime]::UtcNow.AddMilliseconds(4000)
    while ([DateTime]::UtcNow -lt $deadline) {
        $all = @()
        foreach ($cb in (Get-ComboCandidates $dlg)) {
            foreach ($it in (Read-ComboItems $cb)) {
                if ($all -notcontains $it) { $all += $it }
            }
        }
        if ($all.Count -ge 1) { return ,$all }
        Start-Sleep -Milliseconds 250
    }
    return ,$all
}
function Find-ViewChild($main) {
    $kids = Get-ChildrenOfClass $main 'riviv_view'
    if ($kids.Count -ge 1) { return $kids[0] }
    return [IntPtr]::Zero
}
function Post-Hdrop($view, $path) {
    $hmem = [S165]::BuildHdrop($path)
    [void][S165]::PostMessage($view, 0x0233, $hmem, [IntPtr]::Zero)  # WM_DROPFILES
}
# S3-DIAG (PERMANENT, failure-path only; cribbed from smoke78-view-child
# S4-DIAG): a failed drop adopt wait gets evidence instead of a bare FAIL.
# Prints the verbatim MainWindowTitle, the SMTO_ABORTIFHUNG WM_NULL
# SendMessageTimeout verdict for the main window and the view child (a
# wedged UI thread answers no SENT message -> quick-relaunch wedge
# family, same shape as smoke78), and every top-level window class of the pid (a
# stray owned #32770 modal would swallow WM_CLOSE and freeze the title).
# Pipeline output is ONLY the main-window responding bool (discipline:
# detail goes through Write-Host); callers branch the retry on it.
function Invoke-S3Diag($p, $main, $view) {
    $t = Get-Title $p
    Write-Host ('S3-DIAG pid=' + $p.Id + ' title=[' + $t + ']')
    $res = [IntPtr]::Zero
    $smMain = [S165]::SendMessageTimeout($main, 0x0000, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$res)
    Write-Host ('S3-DIAG main responding=' + ($smMain -ne [IntPtr]::Zero))
    if (($view -ne $null) -and ($view -ne [IntPtr]::Zero)) {
        $smView = [S165]::SendMessageTimeout($view, 0x0000, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$res)
        Write-Host ('S3-DIAG view responding=' + ($smView -ne [IntPtr]::Zero))
    }
    $script:s165DiagWins = @()
    $cb = [S165+EnumProc]{ param($h, $lp)
        $wpid = 0
        [void][S165]::GetWindowThreadProcessId($h, [ref]$wpid)
        if ($wpid -eq $p.Id) { $script:s165DiagWins += ((Get-Class $h) + ' ' + $h) }
        return $true }
    [void][S165]::EnumWindows($cb, [IntPtr]::Zero)
    Write-Host ('S3-DIAG top-level windows of pid: ' + (($script:s165DiagWins) -join ' | '))
    $script:s165DiagWins = $null
    return ($smMain -ne [IntPtr]::Zero)
}
# The single pixel assertions of S5 (dump bytes are BGRA, smoke152 style).
function Get-Pixel($q, $x, $y) {
    $i = (($y * $q.W) + $x) * 4
    return @([int]$q.B[$i + 2], [int]$q.B[$i + 1], [int]$q.B[$i])   # R,G,B
}
function Test-GreenDominant($rgb) {
    return (($rgb[1] -ge 120) -and ($rgb[1] -ge ($rgb[0] + 40)) -and ($rgb[1] -ge ($rgb[2] + 60)))
}

[void][S165]::SetProcessDPIAware()

# ---------------------------------------------------------------------------
# S0: the stage. The foreign-riviv check fires BEFORE anything else: a real
# viewer running would take the command line over (second-instance
# forwarding would destroy every assertion below).
# ---------------------------------------------------------------------------
$preExisting = @(Get-Process riviv -ErrorAction SilentlyContinue)
$foreign = @($preExisting | Where-Object { $_.Path -ne $RunExe })
if ($foreign.Count -gt 0) {
    $fxPids = ($foreign | ForEach-Object { $_.Id }) -join ','
    Write-Output ('FATAL foreign riviv running - close it and rerun (pids: ' + $fxPids + ')')
    Stop-Log
    exit 3
}
Kill-StagedRiviv
# Wipe the stage EXCEPT the run logs (smoke156 wipe+copy): the script's
# own transcript AND the prescribed outer Tee-Object target hold
# smoke165-run*.log open for the whole pipeline; deleting an open file
# here would abort the run under the Stop preference.
if (Test-Path $Stage) {
    Get-ChildItem -LiteralPath $Stage -Force |
        Where-Object { $_.Name -notlike 'smoke165-run*.log' } |
        Remove-Item -Recurse -Force
}
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); Stop-Log; exit 2 }
Copy-Item $Exe $RunExe -Force
# Leftover-ini pre-checks (cross-build false-regression discipline): the
# stage was just wiped, and the SOURCE exe dir must not carry an ini either.
$srcIni = Join-Path (Split-Path $Exe -Parent) 'riviv.ini'
Check 'S0a staged exe copied, stage ini absent, source exe dir ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini)) -and (-not (Test-Path $srcIni))) ('stageIni=' + (Test-Path $Ini) + ' srcIni=' + (Test-Path $srcIni))

# The p3.icc emitter: riviv's own P3 destination profile via the same
# env-gated cargo test smoke156 reuses (RIVIV_S156_ICC names the output).
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
    Write-Output ('SUMMARY smoke165 pass=' + $script:pass + ' fail=' + ($script:fail + 1))
    Write-Output 'SMOKE165 RESULT: FAIL'
    Stop-Log
    exit 1
}

# b.png first: its dump probes the client area that wide.png must fill.
$BPng = Join-Path $Stage 'b.png'
[S165]::WriteBPng($BPng)
$IniText = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nicm=1`r`nrenderer=d2d`r`nshow_menu=0`r`n"
$r0 = Run-Dump $BPng 's0-probe.png' 's0.err' $IniText 'b.png'
$dims0 = Png-Dims $r0.Out
$VW = $dims0[0]
$VH = $dims0[1]
$geomOk = ($r0.Code -eq 0) -and (Test-Path $r0.Out) -and ($VW -ge 1800) -and ($VW -le 1920) -and ($VH -ge 1000) -and ($VH -le 1080)
Check 'S0c client geometry probe: exit 0, dump is the client area, VW in [1800,1920], VH in [1000,1080]' $geomOk ("exit=$($r0.Code) dump=$($r0.Out) VW=$VW VH=$VH")
if (-not $geomOk) {
    Write-Output 'FAILURES: S0c geometry probe failed - stderr follows.'
    Write-Output $r0.Err
    Kill-StagedRiviv
    Reset-Ini ''
    Write-Output ('SUMMARY smoke165 pass=' + $script:pass + ' fail=' + ($script:fail + 1))
    Write-Output 'SMOKE165 RESULT: FAIL'
    Stop-Log
    exit 1
}

# The remaining fixtures: wide.png (client size + iCCP) and three SVGs.
$Wide = Join-Path $Stage 'wide.png'
[S165]::WriteWidePng($Wide, $VW, $VH, [IO.File]::ReadAllBytes($P3Icc))
$ASvg = Join-Path $Stage 'a.svg'
[IO.File]::WriteAllText($ASvg, '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="#00AA00"/></svg>')
$CSvg = Join-Path $Stage 'c.svg'
[IO.File]::WriteAllText($CSvg, '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="#00AA00"/></svg>')
$TransSvg = Join-Path $Stage 'trans.svg'
[IO.File]::WriteAllText($TransSvg, "<svg xmlns='http://www.w3.org/2000/svg' width='400' height='400'><rect x='160' y='160' width='80' height='80' fill='#00aa00'/></svg>")
$fixtureOk = $true
$fixtureDetail = ''
foreach ($f in @($BPng, $Wide, $ASvg, $CSvg, $TransSvg)) {
    $sz = 0
    if (Test-Path $f) { $sz = (Get-Item $f).Length }
    if ($sz -le 0) { $fixtureOk = $false; $fixtureDetail += ($f + ' missing/empty; ') }
}
Check 'S0d all fixtures exist and are non-zero (b.png, wide.png, a.svg, c.svg, trans.svg)' $fixtureOk ($fixtureDetail + 'wide=' + (Get-Item $Wide).Length + 'B')
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S1: Ctrl+O. The GetOpenFileNameW dialog must come up on WM_COMMAND(1) and
# its filter list must carry *.svg (the #152 admission surfaced in the UI).
# ---------------------------------------------------------------------------
Reset-Ini ''
$p1 = Start-Riv ('"' + $BPng + '"') 's1.err'
$main1 = Wait-Main $p1
$adopt1 = Wait-Title $p1 'b.png' 15000
$dlg1 = [IntPtr]::Zero
if ($adopt1 -and ($main1 -ne [IntPtr]::Zero) -and (-not $p1.HasExited)) {
    [void][S165]::PostMessage($main1, 0x0111, [IntPtr]1, [IntPtr]::Zero)  # WM_COMMAND FileOpenFile
    $dlg1 = Wait-OwnDialog $p1.Id 10000
}
Check 'S1a Ctrl+O opened the #32770 GetOpenFileNameW dialog' ($dlg1 -ne [IntPtr]::Zero) ("adopted=$adopt1 dlg=$dlg1 pid=$($p1.Id)")
$items1 = @()
if ($dlg1 -ne [IntPtr]::Zero) { $items1 = Read-FilterItems $dlg1 }
$hasSvg1 = $false
foreach ($it in $items1) { if (($null -ne $it) -and $it.Contains('*.svg')) { $hasSvg1 = $true } }
Check 'S1b some filter item contains *.svg' $hasSvg1 ("items=[" + ($items1 -join ' | ') + "]")
$dlgClosed1 = $false
if ($dlg1 -ne [IntPtr]::Zero) { $dlgClosed1 = Close-OpenDialog $dlg1 }
# Vacuous when no dialog ever appeared (S1a already carries that failure).
Check 'S1c dialog closed via the cancel ladder' (($dlg1 -eq [IntPtr]::Zero) -or $dlgClosed1) ("dlg=$dlg1 closed=$dlgClosed1")
$code1 = -9
if ($dlgClosed1 -or ($dlg1 -eq [IntPtr]::Zero)) {
    if ($main1 -ne [IntPtr]::Zero) { $code1 = Close-Main $p1 $main1 15000 }
} else {
    Kill-StagedRiviv
}
Check 'S1d WM_CLOSE -> exit 0' ($code1 -eq 0) "exit=$code1"
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S2: the playlist. mix/ = a.svg + b.png + c.svg; in ANY total order of
# three files the two NavNext steps from b.png land on the two SVGs and a
# NavPrev lands on an SVG or back on b.png.
# ---------------------------------------------------------------------------
$Mix = Join-Path $Stage 'mix'
New-Item -ItemType Directory -Path $Mix | Out-Null
Copy-Item $ASvg (Join-Path $Mix 'a.svg') -Force
Copy-Item $BPng (Join-Path $Mix 'b.png') -Force
Copy-Item $CSvg (Join-Path $Mix 'c.svg') -Force
Reset-Ini ''
$p2 = Start-Riv ('"' + (Join-Path $Mix 'b.png') + '"') 's2.err'
$main2 = Wait-Main $p2
$adopt2 = Wait-Title $p2 'b.png' 15000
$t2a = Get-Title $p2
$navOk1 = $false; $t2b = ''
$navOk2 = $false; $t2c = ''
$navOk3 = $false; $t2d = ''
if ($adopt2 -and ($main2 -ne [IntPtr]::Zero) -and (-not $p2.HasExited)) {
    [void][S165]::PostMessage($main2, 0x0111, [IntPtr]107, [IntPtr]::Zero)  # NavNext
    $t2b = Wait-TitleChangedFrom $p2 $t2a 10000
    $navOk1 = ($t2b -ne $t2a) -and ($t2b.EndsWith('.svg - riviv'))
    [void][S165]::PostMessage($main2, 0x0111, [IntPtr]107, [IntPtr]::Zero)  # NavNext
    $t2c = Wait-TitleChangedFrom $p2 $t2b 10000
    $navOk2 = ($t2c -ne $t2b) -and ($t2c.EndsWith('.svg - riviv'))
    [void][S165]::PostMessage($main2, 0x0111, [IntPtr]108, [IntPtr]::Zero)  # NavPrev
    $t2d = Wait-TitleChangedFrom $p2 $t2c 10000
    $navOk3 = ($t2d -ne $t2c) -and (($t2d.EndsWith('.svg - riviv')) -or ($t2d.EndsWith('b.png - riviv')))
}
$code2 = -9
if ($main2 -ne [IntPtr]::Zero) { $code2 = Close-Main $p2 $main2 15000 }
Check 'S2a playlist launch adopted b.png' ($adopt2 -and ($t2a -ne '')) "adopted=$adopt2 title=[$t2a]"
Check 'S2b NavNext: title ends *.svg (!= b.png)' $navOk1 ("$t2a -> $t2b")
Check 'S2c NavNext again: title ends *.svg again' $navOk2 ("$t2b -> $t2c")
Check 'S2d NavPrev: title ends *.svg or b.png' $navOk3 ("$t2c -> $t2d")
Check 'S2e WM_CLOSE -> exit 0' ($code2 -eq 0) "exit=$code2"
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S3: SVG drag-drop. The hand-built HDROP goes straight to the riviv_view
# child (smoke78 S4 precedent); the title must adopt the dropped image both
# ways (SVG in, PNG back).
# ---------------------------------------------------------------------------
Reset-Ini ''
$p3 = Start-Riv ('"' + $BPng + '"') 's3.err'
$main3 = Wait-Main $p3
$adopt3 = Wait-Title $p3 'b.png' 15000
$view3 = [IntPtr]::Zero
if ($main3 -ne [IntPtr]::Zero) { $view3 = Find-ViewChild $main3 }
Check 'S3a riviv_view child found' (($view3 -ne [IntPtr]::Zero) -and $adopt3) ("view=$view3 adopted=$adopt3")
$dropOk1 = $false
$dropOk2 = $false
$s3MainLive1 = $true
$s3MainLive2 = $true
if (($view3 -ne [IntPtr]::Zero) -and (-not $p3.HasExited)) {
    Post-Hdrop $view3 $ASvg
    $dropOk1 = Wait-TitleEnds $p3 'a.svg' 10000
    if (-not $dropOk1) { $s3MainLive1 = Invoke-S3Diag $p3 $main3 $view3 }
    Post-Hdrop $view3 $BPng
    $dropOk2 = Wait-TitleEnds $p3 'b.png' 10000
    if (-not $dropOk2) { $s3MainLive2 = Invoke-S3Diag $p3 $main3 $view3 }
}
$code3 = -9
if ($main3 -ne [IntPtr]::Zero) { $code3 = Close-Main $p3 $main3 15000 }
Check 'S3b HDROP a.svg onto the view child: title adopted a.svg' $dropOk1 ("title ends a.svg - riviv: $dropOk1")
Check 'S3c HDROP b.png onto the view child: title adopted b.png' $dropOk2 ''
Check 'S3d WM_CLOSE -> exit 0' ($code3 -eq 0) "exit=$code3"
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S4: the wide-ICC AC-arm storm. The AC face comes up on the live wide
# image; refresh x3, an open-dialog cycle, and narrow/wide drop swaps must
# all leave stderr clean and the process exitable.
# ---------------------------------------------------------------------------
Reset-Ini $IniText
$p4 = Start-Riv ('"' + $Wide + '"') 's4.err'
$main4 = Wait-Main $p4
$adopt4 = Wait-Title $p4 'wide.png' 15000
$surface4 = $false
if ($adopt4) {
    $surface4 = Wait-Until { ((Get-Lines (Read-Err 's4.err') 'output-surface=ac-scrgb').Count -ge 1) } 10000
}
Check 'S4a wide.png adopted, stderr has output-surface=ac-scrgb >= 1' ($adopt4 -and $surface4) ("adopted=$adopt4 surface=$surface4 alive=$(-not $p4.HasExited)")
$stormDead = $false
$refreshOk = @($true, $true, $true)
for ($ri = 0; $ri -lt 3; $ri++) {
    if ((-not $p4.HasExited) -and ($main4 -ne [IntPtr]::Zero)) {
        [void][S165]::PostMessage($main4, 0x0111, [IntPtr]41, [IntPtr]::Zero)  # ViewRefresh (F5)
        Start-Sleep -Milliseconds 900
        $p4.Refresh()
        $t = Get-Title $p4
        $refreshOk[$ri] = ((-not $p4.HasExited) -and ($null -ne $t) -and $t.EndsWith('wide.png - riviv'))
    } else {
        $refreshOk[$ri] = $false
        $stormDead = $true
    }
}
Check 'S4b ViewRefresh #1: process alive, title unchanged' $refreshOk[0] ("alive=$(-not $p4.HasExited) title=[$(Get-Title $p4)]")
Check 'S4c ViewRefresh #2: process alive, title unchanged' $refreshOk[1] ("alive=$(-not $p4.HasExited) title=[$(Get-Title $p4)]")
Check 'S4d ViewRefresh #3: process alive, title unchanged' $refreshOk[2] ("alive=$(-not $p4.HasExited) title=[$(Get-Title $p4)]")
$dlgOk4 = $false
if ((-not $stormDead) -and (-not $p4.HasExited) -and ($main4 -ne [IntPtr]::Zero)) {
    [void][S165]::PostMessage($main4, 0x0111, [IntPtr]1, [IntPtr]::Zero)  # FileOpenFile
    $dlg4 = Wait-OwnDialog $p4.Id 10000
    if ($dlg4 -ne [IntPtr]::Zero) {
        $dlgOk4 = Close-OpenDialog $dlg4
    }
    if (-not $dlgOk4) { Write-Host ('S4 dialog cycle failed, dlg=' + $dlg4) }
}
Check 'S4e Ctrl+O dialog appeared and closed' $dlgOk4 ''
$dropOk4a = $false
$dropOk4b = $false
if ((-not $stormDead) -and (-not $p4.HasExited)) {
    $view4 = Find-ViewChild $main4
    if ($view4 -ne [IntPtr]::Zero) {
        Post-Hdrop $view4 $BPng
        $dropOk4a = Wait-TitleEnds $p4 'b.png' 10000
        Post-Hdrop $view4 $Wide
        $dropOk4b = Wait-TitleEnds $p4 'wide.png' 10000
    } else {
        Write-Host 'S4: riviv_view child not found for the drops'
    }
}
Check 'S4f HDROP b.png adopted (wide -> narrow face rebuild)' $dropOk4a ''
Check 'S4g HDROP wide.png adopted (narrow -> wide face rebuild)' $dropOk4b ''
$err4 = Read-Err 's4.err'
$badLines4 = Get-Lines $err4 'display effect failure|panic'
Check 'S4h stderr clean: no display effect failure, no panic' ($badLines4.Count -eq 0) ("badLines=$($badLines4.Count) lines=[$($badLines4 -join ' | ')]")
$code4 = -9
if ($main4 -ne [IntPtr]::Zero) { $code4 = Close-Main $p4 $main4 15000 }
Check 'S4i WM_CLOSE -> exit 0 and the process gone within 15s' ($code4 -eq 0) "exit=$code4"
if ($code4 -eq -1) { Write-Output ('FAILURES: S4 stderr dump follows.'); Write-Output $err4 }
Kill-StagedRiviv
Reset-Ini ''

# ---------------------------------------------------------------------------
# S5: SVG transparency composites over the pinned windowed background.
# Two independent trans.svg dump runs differing ONLY in the ini background
# color; the transparent sample sits near the image's top-left corner
# (image origin = dump center - (200,200); the 400x400 SVG displays 1:1),
# the center sample sits on the #00aa00 rect.
# ---------------------------------------------------------------------------
$BGIni1 = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nshow_menu=0`r`nwindowed_background_color_r=8`r`nwindowed_background_color_g=8`r`nwindowed_background_color_b=8`r`n"
$BGIni2 = "[riviv]`r`nx=40`r`ny=40`r`nwide=1920`r`nhigh=1200`r`nauto_zoom=0`r`nshow_menu=0`r`nwindowed_background_color_r=200`r`nwindowed_background_color_g=40`r`nwindowed_background_color_b=40`r`n"
$r5a = Run-Dump $TransSvg 's5-run1.png' 's5a.err' $BGIni1 'trans'
$r5b = Run-Dump $TransSvg 's5-run2.png' 's5b.err' $BGIni2 'trans'
Kill-StagedRiviv
$s5aOk = $false; $s5bOk = $false; $s5cOk = $false; $s5dOk = $false; $s5eOk = $false
$s5Detail = 'run1/run2 unusable'
if ((Test-Path $r5a.Out) -and (Test-Path $r5b.Out)) {
    $qA = [Px]::Load($r5a.Out)
    $qB = [Px]::Load($r5b.Out)
    $sameDims = ($qA.W -eq $qB.W) -and ($qA.H -eq $qB.H)
    $ox = ([int][Math]::Floor($qA.W / 2)) - 200
    $oy = ([int][Math]::Floor($qA.H / 2)) - 200
    $tx = $ox + 40
    $ty = $oy + 40
    $inImg = ($sameDims -and ($ox -ge 0) -and ($oy -ge 0) -and (($ox + 400) -le $qA.W) -and (($oy + 400) -le $qA.H))
    if ($inImg) {
        $bgA = Get-Pixel $qA $tx $ty
        $bgB = Get-Pixel $qB $tx $ty
        $ctrA = Get-Pixel $qA ([int][Math]::Floor($qA.W / 2)) ([int][Math]::Floor($qA.H / 2))
        $ctrB = Get-Pixel $qB ([int][Math]::Floor($qB.W / 2)) ([int][Math]::Floor($qB.H / 2))
        $s5aOk = ($r5a.Code -eq 0) -and $r5a.Adopted -and $sameDims -and ([Math]::Abs($bgA[0] - 8) -le 12) -and ([Math]::Abs($bgA[1] - 8) -le 12) -and ([Math]::Abs($bgA[2] - 8) -le 12)
        $s5bOk = Test-GreenDominant $ctrA
        $s5cOk = ($r5b.Code -eq 0) -and $r5b.Adopted -and ([Math]::Abs($bgB[0] - 200) -le 30) -and ([Math]::Abs($bgB[1] - 40) -le 30) -and ([Math]::Abs($bgB[2] - 40) -le 30)
        $s5dOk = Test-GreenDominant $ctrB
        $dR = [Math]::Abs($bgA[0] - $bgB[0])
        $dG = [Math]::Abs($bgA[1] - $bgB[1])
        $dB = [Math]::Abs($bgA[2] - $bgB[2])
        $s5eOk = (($dR -ge 100) -or ($dG -ge 100) -or ($dB -ge 100))
        $s5Detail = "dims=$($qA.W)x$($qA.H)/$($qB.W)x$($qB.H) origin=$ox,$oy sample=$tx,$ty run1bg=[$($bgA -join ',')] run2bg=[$($bgB -join ',')] run1ctr=[$($ctrA -join ',')] run2ctr=[$($ctrB -join ',')] maxChanDiff=$([Math]::Max($dR, [Math]::Max($dG, $dB)))"
    } else {
        $s5Detail = "dims mismatch or image origin out of dump: $($qA.W)x$($qA.H) vs $($qB.W)x$($qB.H)"
    }
}
Check 'S5a run1 (8,8,8): transparent point ~= (8,8,8) +-12 per channel' $s5aOk ("exit=$($r5a.Code) adopted=$($r5a.Adopted) $s5Detail")
Check 'S5b run1 center green-dominant (G>=120, G>=R+40, G>=B+60)' $s5bOk $s5Detail
Check 'S5c run2 (200,40,40): same point ~= (200,40,40) +-30 per channel' $s5cOk ("exit=$($r5b.Code) adopted=$($r5b.Adopted) $s5Detail")
Check 'S5d run2 center green-dominant' $s5dOk $s5Detail
Check 'S5e the two transparent points differ >= 100 on some channel' $s5eOk $s5Detail

# ---------------------------------------------------------------------------
# S6 + teardown. The ini removal is UNCONDITIONAL (a leftover ini fakes
# cross-build regressions); a leftover staged riviv process is a failure,
# not something to sweep silently. On failure the rest of the stage is
# KEPT (stderr captures + dumps + this log are the evidence); on PASS the
# whole stage goes (this log with it).
# ---------------------------------------------------------------------------
Reset-Ini ''
$iniCleaned = -not (Test-Path $Ini)
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe })
$leftoverNames = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S6 teardown: stage ini removed, no staged riviv left' ($iniCleaned -and ($leftover.Count -eq 0)) ('ini exists: ' + (Test-Path $Ini) + ' leftoverPids: [' + $leftoverNames + ']')

Write-Output ('SUMMARY smoke165 pass=' + $script:pass + ' fail=' + $script:fail)
if ($script:fail -gt 0) {
    Write-Output ('FAILURES: per-scenario stderr follows (stage kept: ' + $Stage + ').')
    foreach ($e in @(@('s0', 's0.err'), @('s1', 's1.err'), @('s2', 's2.err'), @('s3', 's3.err'), @('s4', 's4.err'), @('s5a', 's5a.err'), @('s5b', 's5b.err'))) {
        $body = Read-Err $e[1]
        if ($body -ne '') {
            Write-Output ('--- ' + $e[0] + ' stderr ---')
            Write-Output $body
        }
    }
    Write-Output 'SMOKE165 RESULT: FAIL'
    Stop-Log
    exit 1
}
Write-Output 'SMOKE165 RESULT: PASS'
Stop-Log
# Stage teardown on PASS: everything goes EXCEPT the run logs (the
# transcript and the outer Tee-Object target, same spare set as the S0
# wipe) - they are the report's evidence and the tee may still hold the
# last one open while this line runs.
if (Test-Path $Stage) {
    Get-ChildItem -LiteralPath $Stage -Force |
        Where-Object { $_.Name -notlike 'smoke165-run*.log' } |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
