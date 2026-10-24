# smoke229 - #229 ticket item 5: the Rename dialog (#227 spike boundary 7)
# joins the v6-visual-matrix family. The dialog (src/rename_dlg.rs, class
# "riviv_rename") is the fifth hand-built dialog; Common-Controls v6
# theming (manifest-only, #229) must leave its geometry untouched - this
# smoke pins the geometry against the SOURCE CONSTANTS and exercises the
# behavior, on ONE arm at a time (the master-vs-impl dual-arm equality was
# proven by a temp probe during #229; this script must not depend on a
# second exe being present). ASCII-only source (PS 5.1 ANSI/param-binding
# trap, #79 lesson). Harness cribbed from smoke178: poll-based waits
# everywhere (posted-message latency is unbounded), GetExitCodeProcess on
# the handle captured WHILE ALIVE, path-targeted process handling, and the
# C# chunk/zlib PNG writer (8x8 solid fixtures, mtimes staged c -> b -> a
# with 1.1s gaps so the default DateModified-descending nav order is a,
# b, c).
#
# Source pins (src/rename_dlg.rs):
#   class "riviv_rename" (line 48); client 186x51 dlu (lines 58-59);
#   edit id 1042 at dlu(7,7) size dlu(172,14), ES_AUTOHSCROLL +
#   WS_EX_CLIENTEDGE (lines 52, 242-256); OK id 1 BS_DEFPUSHBUTTON at
#   dlu(74,30) size dlu(50,14); Cancel id 2 BS_PUSHBUTTON at dlu(129,30)
#   size dlu(50,14) (lines 53-54, 299-300); no page containers - three
#   direct children; modal loop disables the owner (line 158); WM_CLOSE
#   collapses to Cancel (lines 383-387); OK = rename_target (stem + old
#   extension) -> shell FO_RENAME -> apply_rename (nav_current + playlist
#   row + title refresh, lines 309-357).
#   Command id: menu::Cmd::FileRename = 15 (unit-pinned, src/menu.rs
#   command_ids_stay_pinned_for_the_smoke_scripts); NavNext=107,
#   NavPrev=108.
#
# Geometry expectations are DERIVED, not hard-coded pixels: dlu(x,y) =
# (floor(x*bx/4), floor(y*by/8)) with (bx,by) = GetDialogBaseUnits of THIS
# machine (riviv is PerMonitorV2 via its embedded manifest, and this
# script claims the same awareness context first, so both sides read the
# same physical-pixel space; verified during #229 at 125% DPI: base units
# 10x20 -> client 465x127, edit 17,17 430x35, OK 185,75 125x35, Cancel
# 322,75 125x35). The OUTER window size is NOT pinned: DWM adds an
# extended frame beyond AdjustWindowRectEx's classic chrome, which is
# machine state, not app state (the dual-arm probe proved outer 491x198
# EQUAL on master and impl). Only a weak sanity check guards it.
#
# Scenarios (single launch, playlist a,b,c, current = a):
#   S0 gates: exe present; LIVING-PROCESS gate - only a riviv process with
#       at least one thread blocks (terminated remnants always show 0
#       threads and are let through, e.g. the known 0-thread riviv PIDs
#       some machines carry); stage prep (exe copied to
#       %TEMP%\riviv-229-smoke, the run BLOCKS if the stage cannot be
#       prepared).
#   S1 shape + geometry: WM_COMMAND 15 opens the dialog; class
#       riviv_rename; exactly 3 direct children with ids 1042/1/2; all
#       visible; client size == dlu(186,51); each control rect ==
#       its dlu expectation (relative to the client origin); style bits
#       (ES_AUTOHSCROLL + WS_EX_CLIENTEDGE on the edit, BS_DEFPUSHBUTTON
#       on OK, plain pushbutton on Cancel); outer sanity (client fits,
#       caption room exists); prefilled edit text == "a" (rename_stem);
#       owner disabled while the modal is up.
#   S2 rename OK: EM_SETSEL(0,-1) + WM_CHAR 'x','9' -> text "x9"; OK
#       (WM_COMMAND 1 posted to the DIALOG - #191 lesson: async posts
#       into a modal loop, never synchronous sends); a.png renamed to
#       x9.png ON DISK; title follows; owner re-enabled; dialog gone.
#   S3 playlist follows: NavNext -> b.png; NavPrev -> x9.png (the
#       renamed row - a stale "a.png" row would not land here).
#   S4 Cancel: on b.png open, type "z", Cancel (WM_COMMAND 2) -> dialog
#       gone, b.png intact, no z.png, owner re-enabled, title b.png.
#   S5 WM_CLOSE: on c.png open, WM_CLOSE to the dialog -> gone, c.png
#       intact (closes to Cancel, lines 383-387).
#   Teardown: WM_CLOSE to the main window, exit code 0 required; the ini
#       this run generated (staged next to the STAGED exe) is deleted;
#       no riviv left behind; failures tally FAIL, never a silent note.
#
# Output contract: one line per scenario ("S<n>: PASS/FAIL/SKIP
# <evidence>"), final line
#   "SMOKE229 RESULT: PASS=n FAIL=n SKIP=n iniCleaned=<True|False>".
# Exit codes: 0 all pass, 1 failures, 2 missing exe, 4 BLOCKED (foreign
# living riviv process / stage preparation failed).

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Text;
using System.Runtime.InteropServices;

public class S229 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, StringBuilder lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dlg, int id);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsChild(IntPtr parent, IntPtr child);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hwnd, int idx);
    [DllImport("user32.dll")] public static extern uint GetDialogBaseUnits();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder sb, int max);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    public delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr hwnd, EnumProc cb, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int max);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }

    public static string TitleOf(IntPtr hwnd) {
        StringBuilder sb = new StringBuilder(512);
        GetWindowText(hwnd, sb, 512);
        return sb.ToString();
    }
    public static string ClassOf(IntPtr hwnd) {
        StringBuilder sb = new StringBuilder(256);
        GetClassName(hwnd, sb, 256);
        return sb.ToString();
    }
    // Cross-process edit text: WM_GETTEXTLENGTH + WM_GETTEXT via
    // SendMessageTimeout (the system marshals the buffer).
    public static string EditText(IntPtr edit) {
        IntPtr got = IntPtr.Zero;
        SendMessageTimeout(edit, 0x000E, IntPtr.Zero, IntPtr.Zero, 0x0002 | 0x0008, 4000, out got);
        int len = got.ToInt32();
        if (len < 0) return "(err)";
        StringBuilder sb = new StringBuilder(len + 2);
        SendMessageTimeout(edit, 0x000D, (IntPtr)(len + 1), sb, 0x0002 | 0x0008, 4000, out got);
        return sb.ToString();
    }
    public static List<IntPtr> TopWindows() {
        List<IntPtr> all = new List<IntPtr>();
        EnumProc cb = delegate(IntPtr h, IntPtr lp) { all.Add(h); return true; };
        EnumWindows(cb, IntPtr.Zero);
        return all;
    }
    public static List<IntPtr> ChildWindows(IntPtr hwnd) {
        List<IntPtr> all = new List<IntPtr>();
        EnumProc cb = delegate(IntPtr h, IntPtr lp) { all.Add(h); return true; };
        EnumChildWindows(hwnd, cb, IntPtr.Zero);
        return all;
    }
    // ---- PNG fixture writing (smoke178's verified recipe) ----
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
    // An 8x8 RGBA8 solid PNG with NO ancillary chunks.
    public static void WritePng8(string path, byte r, byte g, byte b) {
        int w = 8, h = 8;
        byte[] px = new byte[w * h * 4];
        for (int i = 0; i < w * h; i++) { px[i*4] = r; px[i*4+1] = g; px[i*4+2] = b; px[i*4+3] = 255; }
        MemoryStream ms = new MemoryStream();
        byte[] sig = { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A };
        ms.Write(sig, 0, 8);
        byte[] ihdr = new byte[13];
        PutBe32(ihdr, 0, (uint)w); PutBe32(ihdr, 4, (uint)h);
        ihdr[8] = 8; ihdr[9] = 6; // RGBA8
        Chunk(ms, "IHDR", ihdr);
        Chunk(ms, "IDAT", Zlib(Scanlines(w, h, px)));
        Chunk(ms, "IEND", new byte[0]);
        File.WriteAllBytes(path, ms.ToArray());
    }
}
'@

# ---------------------------------------------------------------------------
# DPI: claim PerMonitorV2 first (matches riviv's embedded manifest so rects
# and GetDialogBaseUnits land in the same physical-pixel space). The 2017
# API only exists on 1703+ - on the 1607-1702 floor the P/Invoke throws
# EntryPointNotFoundException, so guard it and fall back to system-aware.
# ---------------------------------------------------------------------------
$pmv2 = $false
try { $pmv2 = ([S229]::SetProcessDpiAwarenessContext([IntPtr](-4)) -ne [IntPtr]::Zero) } catch { $pmv2 = $false }
if (-not $pmv2) { [void][S229]::SetProcessDPIAware() }

# ---------------------------------------------------------------------------
# Stage paths. The exe under test is COPIED to %TEMP%\riviv-229-smoke and
# run from there, so the riviv.ini it writes (exe-relative) is the staged
# one this script owns and cleans - target\release is never touched.
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-229-smoke'
$StageExe = Join-Path $Stage 'riviv.exe'
$Fx = Join-Path $Stage 'fx'
$Afx = Join-Path $Fx 'a.png'
$Bfx = Join-Path $Fx 'b.png'
$Cfx = Join-Path $Fx 'c.png'
$Ini = Join-Path $Stage 'riviv.ini'
$LogPath = Join-Path $Stage 'smoke229-run.log'

# ---------------------------------------------------------------------------
# S0 gates - BEFORE anything else.
# ---------------------------------------------------------------------------
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); exit 2 }

# The LIVING-PROCESS gate: only a riviv process that is demonstrably alive
# blocks - at least one thread (the decisive tombstone marker: a terminated
# remnant always shows 0 threads). Path readability is NOT part of the
# tombstone test - an elevated or cross-bitness live instance can hide its
# path, and letting those through would only convert a true single-instance
# hijack into an honest S1 red; a process running from OUR stage root is
# still let through (it is staged, not foreign).
$foreign = @()
foreach ($p in (Get-Process riviv -ErrorAction SilentlyContinue)) {
    $path = $null
    try { $path = $p.Path } catch { $path = $null }
    # -1 = the thread count could not be READ (an inaccessible LIVE
    # process) - block on it, never tombstone it.
    $threads = -1
    try { $threads = $p.Threads.Count } catch { }
    $tombstone = ($threads -eq 0)
    # $Stage + '\' so a sibling like riviv-229-smoke-evil cannot pass as
    # staged (and get killed by the teardown filter).
    $staged = (($path -ne $null) -and ($path.StartsWith($Stage + '\', [StringComparison]::OrdinalIgnoreCase)))
    if ((-not $tombstone) -and (-not $staged)) { $foreign += ('pid=' + $p.Id + ' path=' + $path) }
}
if ($foreign.Count -gt 0) {
    Write-Output ('BLOCKED: living riviv process(es) not spawned by this script - close them and rerun (' + ($foreign -join '; ') + ').')
    exit 4
}

# Stage prep: wipe and rebuild. A stage that cannot be cleared (a staged
# exe still locked by an earlier crashed run) is a BLOCK, not a surprise;
# a prep failure (permissions, in-use destination) also routes through
# BLOCKED instead of an unstructured throw - $ErrorActionPreference is
# Stop up here, so New-Item/Copy-Item must be guarded.
if (Test-Path $Stage) {
    Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path (Join-Path $Stage 'riviv.exe')) {
        Write-Output ('BLOCKED: stage directory could not be cleared (' + $Stage + ' - a staged riviv.exe is still locked).')
        exit 4
    }
}
try {
    New-Item -ItemType Directory -Path $Stage -ErrorAction Stop | Out-Null
    Copy-Item $Exe $StageExe -Force -ErrorAction Stop
} catch {
    Write-Output ('BLOCKED: stage preparation failed: ' + $_.Exception.Message)
    Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
    exit 4
}
if (-not (Test-Path $StageExe)) { Write-Output 'BLOCKED: staged exe copy failed.'; exit 4 }
New-Item -ItemType Directory -Path $Fx | Out-Null

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
# Harness
# ---------------------------------------------------------------------------
$script:pass = 0
$script:fail = 0
$script:skip = 0
function Emit-S($id, $state, $ev) {
    # $state: 'PASS' | 'FAIL' | 'SKIP'
    if ($state -eq 'PASS') { $script:pass++ }
    elseif ($state -eq 'FAIL') { $script:fail++ }
    else { $script:skip++ }
    Write-Output ($id + ': ' + $state + ' ' + $ev)
}
function Emit-Gate($id, $gateOk, $ok, $ev) {
    # A scenario whose PRECONDITION (the prior scenario) went red is a SKIP,
    # not a FAIL - the real failure is already counted upstream; this keeps
    # FAIL focused on genuine assertion breaches.
    if (-not $gateOk) { Emit-S $id 'SKIP' $ev; return }
    Emit-S $id ($(if ($ok) { 'PASS' } else { 'FAIL' })) $ev
}
function Det($tag, $ok, $text) {
    $v = 'FAIL'
    if ($ok) { $v = 'PASS' }
    Write-Host ('  [' + $tag + '] ' + $v + ' ' + $text)
}
function Wait-Until($sb, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $sb) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (& $sb)
}
function Post-Cmd($hwnd, $id) {
    # WM_COMMAND(wParam = command id, lParam = 0), posted ASYNC - the #191
    # lesson: a modal loop plus a synchronous send deadlocks.
    [void][S229]::PostMessage($hwnd, 0x0111, [IntPtr]$id, [IntPtr]::Zero)
}
function Get-Title($main) {
    if (($main -eq $null) -or ($main -eq [IntPtr]::Zero)) { return '(no main window)' }
    return [S229]::TitleOf($main)
}
function Wait-TitleHas($main, $leaf, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Get-Title $main).Contains($leaf)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (Get-Title $main).Contains($leaf)
}
function Wait-Exit($p, $ms) {
    if ($p.WaitForExit($ms)) {
        $code = 0
        [void][S229]::GetExitCodeProcess($p.Handle, [ref]$code)
        return $code
    }
    return $null
}

# ---- rename-dialog helpers -------------------------------------------------
function Find-RenameDlg($procId) {
    foreach ($h in [S229]::TopWindows()) {
        $wpid = 0
        [void][S229]::GetWindowThreadProcessId($h, [ref]$wpid)
        if (($wpid -eq $procId) -and ([S229]::ClassOf($h) -eq 'riviv_rename')) { return $h }
    }
    return [IntPtr]::Zero
}
function Wait-RenameDlgReady($procId) {
    # The dialog becomes enumerable between CreateWindowExW and
    # build_controls (WS_VISIBLE at creation): wait until the edit child
    # EXISTS, or reads/typing hit a control-less frame (probe lesson).
    # Own loop - a scriptblock invoked with & runs in a child scope, so an
    # out-assignment inside Wait-Until would not propagate.
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        $d = Find-RenameDlg $procId
        if (($d -ne [IntPtr]::Zero) -and ([S229]::GetDlgItem($d, 1042) -ne [IntPtr]::Zero)) {
            Start-Sleep -Milliseconds 150   # let creation settle
            return $d
        }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Wait-DlgGone($dlg, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not [S229]::IsWindow($dlg)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (-not [S229]::IsWindow($dlg))
}
function Type-IntoEdit($edit, $text) {
    # EM_SETSEL(0,-1) selects the prefilled stem; WM_CHAR then replaces it
    # (the dialog preselects the whole stem for exactly this flow).
    [void][S229]::PostMessage($edit, 0x00B1, [IntPtr]0, [IntPtr](-1))
    foreach ($ch in $text.ToCharArray()) {
        [void][S229]::PostMessage($edit, 0x0102, [IntPtr][int]$ch, [IntPtr]0)
    }
}

# ---------------------------------------------------------------------------
# Geometry expectations DERIVED from the source constants (no pixels in
# this file): dlu(x,y) = (floor(x*bx/4), floor(y*by/8)).
# ---------------------------------------------------------------------------
$bu = [S229]::GetDialogBaseUnits()
$bx = [int]($bu -band 0xFFFF)
$by = [int](($bu -shr 16) -band 0xFFFF)
function Conv-X($x) { [math]::Floor($x * $bx / 4) }
function Conv-Y($y) { [math]::Floor($y * $by / 8) }
$ExpCli = @{ w = (Conv-X 186); h = (Conv-Y 51) }
$ExpEdit = @{ x = (Conv-X 7); y = (Conv-Y 7); w = (Conv-X 172); h = (Conv-Y 14) }
$ExpOk = @{ x = (Conv-X 74); y = (Conv-Y 30); w = (Conv-X 50); h = (Conv-Y 14) }
$ExpCan = @{ x = (Conv-X 129); y = (Conv-Y 30); w = (Conv-X 50); h = (Conv-Y 14) }
Write-Host ('  [derive] baseUnits=' + $bx + 'x' + $by + ' client=' + $ExpCli.w + 'x' + $ExpCli.h +
    ' edit=' + $ExpEdit.x + ',' + $ExpEdit.y + ' ' + $ExpEdit.w + 'x' + $ExpEdit.h +
    ' ok=' + $ExpOk.x + ',' + $ExpOk.y + ' ' + $ExpOk.w + 'x' + $ExpOk.h +
    ' cancel=' + $ExpCan.x + ',' + $ExpCan.y + ' ' + $ExpCan.w + 'x' + $ExpCan.h)

# ---------------------------------------------------------------------------
# Fixtures: 8x8 solid PNGs, three colors, mtimes staged c -> b -> a so the
# DateModified-descending nav order over the launched a,b,c is a, b, c.
# ---------------------------------------------------------------------------
[S229]::WritePng8($Cfx, 30, 30, 220)   # blue
Start-Sleep -Milliseconds 1100
[S229]::WritePng8($Bfx, 30, 220, 30)   # green
Start-Sleep -Milliseconds 1100
[S229]::WritePng8($Afx, 220, 30, 30)   # red

$exitNote = ''
$ErrPath = Join-Path $env:TEMP 'riviv-229-last-stderr.txt'
if (Test-Path $ErrPath) { Remove-Item $ErrPath -Force -ErrorAction SilentlyContinue }

try {

# ===========================================================================
# LAUNCH: playlist a,b,c, current = a.
# ===========================================================================
$p = Start-Process -FilePath $StageExe -ArgumentList ('"' + $Afx + '" "' + $Bfx + '" "' + $Cfx + '"') -PassThru -WorkingDirectory $Stage -RedirectStandardError $ErrPath
$p.Handle | Out-Null   # capture the handle WHILE ALIVE (smoke80 lesson)
$main = [IntPtr]::Zero
for ($i = 0; $i -lt 150; $i++) {
    Start-Sleep -Milliseconds 100
    $p.Refresh()
    $h = $p.MainWindowHandle
    if (($h -ne $null) -and ($h -ne [IntPtr]::Zero)) { $main = $h; break }
    if ($p.HasExited) { break }
}
$adopt = Wait-TitleHas $main 'a.png' 15000
Start-Sleep -Milliseconds 800   # settle
$alive = -not $p.HasExited
Write-Host ('  [launch] pid=' + $p.Id + ' main=' + $main + ' adopted=' + $adopt + ' title=[' + (Get-Title $main) + ']')

# ---------------------------------------------------------------------------
# S1: shape + geometry + prefill + modality.
# ---------------------------------------------------------------------------
$ok1 = $alive -and $adopt
$ev1 = ''
if (-not $ok1) { $ev1 = 'launch/adopt failed: alive=' + $alive + ' adopted=' + $adopt + ' title=[' + (Get-Title $main) + ']' }
$dlg = [IntPtr]::Zero
if ($ok1) {
    Post-Cmd $main 15   # FileRename
    $dlg = Wait-RenameDlgReady $p.Id
    if ($dlg -eq [IntPtr]::Zero) { $ok1 = $false; $ev1 = 'rename dialog (class riviv_rename with edit 1042) never appeared within 10s' }
}
$edit = [IntPtr]::Zero; $okBtn = [IntPtr]::Zero; $canBtn = [IntPtr]::Zero
if ($ok1) {
    $edit = [S229]::GetDlgItem($dlg, 1042)
    $okBtn = [S229]::GetDlgItem($dlg, 1)
    $canBtn = [S229]::GetDlgItem($dlg, 2)
    $cls = [S229]::ClassOf($dlg)
    Det 'S1.class' ($cls -eq 'riviv_rename') ('dialog class=[' + $cls + ']')
    if ($cls -ne 'riviv_rename') { $ok1 = $false; $ev1 = 'unexpected dialog class ' + $cls }
}
if ($ok1) {
    $children = [S229]::ChildWindows($dlg)
    $ids = @()
    foreach ($c in $children) { $ids += [S229]::GetDlgCtrlID($c) }
    $shapeOk = ($children.Count -eq 3) -and ($ids -contains 1042) -and ($ids -contains 1) -and ($ids -contains 2) -and
        ($edit -ne [IntPtr]::Zero) -and ($okBtn -ne [IntPtr]::Zero) -and ($canBtn -ne [IntPtr]::Zero) -and
        ([S229]::IsChild($dlg, $edit)) -and ([S229]::IsChild($dlg, $okBtn)) -and ([S229]::IsChild($dlg, $canBtn))
    Det 'S1.controls' $shapeOk ('direct children=' + $children.Count + ' ids=[' + (($ids | Sort-Object) -join ',') + '] (expect 3: 1,2,1042, no page containers)')
    if (-not $shapeOk) { $ok1 = $false; $ev1 = 'control shape wrong: count=' + $children.Count + ' ids=[' + (($ids | Sort-Object) -join ',') + ']' }
}
function Rect-Rel($hwnd, $ox, $oy) {
    $r = New-Object S229+RECT
    [void][S229]::GetWindowRect($hwnd, [ref]$r)
    return @{ x = $r.L - $ox; y = $r.T - $oy; w = $r.R - $r.L; h = $r.B - $r.T }
}
function Compare-Rect($tag, $got, $exp) {
    $ok = ($got.x -eq $exp.x) -and ($got.y -eq $exp.y) -and ($got.w -eq $exp.w) -and ($got.h -eq $exp.h)
    Det $tag $ok ('got ' + $got.x + ',' + $got.y + ' ' + $got.w + 'x' + $got.h + ' expect ' + $exp.x + ',' + $exp.y + ' ' + $exp.w + 'x' + $exp.h)
    return $ok
}
if ($ok1) {
    $vis = [int][S229]::IsWindowVisible($dlg) + [int][S229]::IsWindowVisible($edit) + [int][S229]::IsWindowVisible($okBtn) + [int][S229]::IsWindowVisible($canBtn)
    Det 'S1.visible' ($vis -eq 4) ('dlg/edit/ok/cancel all visible: ' + $vis + '/4')
    if ($vis -ne 4) { $ok1 = $false; $ev1 = 'a control is not visible (' + $vis + '/4)' }
}
if ($ok1) {
    $cli = New-Object S229+RECT
    [void][S229]::GetClientRect($dlg, [ref]$cli)
    $cliW = $cli.R - $cli.L; $cliH = $cli.B - $cli.T
    $cliOk = ($cliW -eq $ExpCli.w) -and ($cliH -eq $ExpCli.h)
    Det 'S1.client' $cliOk ('client ' + $cliW + 'x' + $cliH + ' expect ' + $ExpCli.w + 'x' + $ExpCli.h + ' (dlu 186x51)')
    if (-not $cliOk) { $ok1 = $false; $ev1 = 'client size ' + $cliW + 'x' + $cliH + ' != dlu-derived ' + $ExpCli.w + 'x' + $ExpCli.h }
}
if ($ok1) {
    $po = New-Object S229+POINT
    $po.X = 0; $po.Y = 0
    [void][S229]::ClientToScreen($dlg, [ref]$po)
    $rectsOk = (Compare-Rect 'S1.edit' (Rect-Rel $edit $po.X $po.Y) $ExpEdit) -and
               (Compare-Rect 'S1.ok' (Rect-Rel $okBtn $po.X $po.Y) $ExpOk) -and
               (Compare-Rect 'S1.cancel' (Rect-Rel $canBtn $po.X $po.Y) $ExpCan)
    if (-not $rectsOk) { $ok1 = $false; $ev1 = 'control rects deviate from the dlu-derived layout (see S1.* detail lines)' }
}
if ($ok1) {
    # Style pins, source-controlled bits only: WS_CHILD(0x40000000) +
    # WS_VISIBLE(0x10000000) + WS_TABSTOP(0x00010000) + ES_AUTOHSCROLL(0x80)
    # on the edit, WS_EX_CLIENTEDGE(0x200) on the edit's exstyle;
    # BS_DEFPUSHBUTTON(0x1) set on OK, clear on Cancel. Bits the system
    # manages (WS_EX_WINDOWEDGE and friends fluctuate with activation
    # state) are NOT asserted - the dual-arm probe owns those.
    $styE = [S229]::GetWindowLong($edit, -16)
    $exE = [S229]::GetWindowLong($edit, -20)
    $styO = [S229]::GetWindowLong($okBtn, -16)
    $styC = [S229]::GetWindowLong($canBtn, -16)
    $styOk = ((($styE -band 0x40000000) -ne 0) -and (($styE -band 0x10000000) -ne 0) -and
        (($styE -band 0x00010000) -ne 0) -and (($styE -band 0x80) -ne 0) -and
        (($exE -band 0x200) -ne 0) -and (($styO -band 0x1) -ne 0) -and (($styC -band 0x1) -eq 0))
    $styTxt = 'edit sty=0x{0:X} ex=0x{1:X} (CHILD|VISIBLE|TABSTOP|AUTOHSCROLL + CLIENTEDGE) ok=0x{2:X} (DEFPUSH) cancel=0x{3:X}' -f $styE, $exE, $styO, $styC
    Det 'S1.styles' $styOk $styTxt
    if (-not $styOk) { $ok1 = $false; $ev1 = 'control style bits deviate (' + $styTxt + ')' }
}
if ($ok1) {
    # Outer sanity only (DWM extended frame is machine state, see header).
    $out = New-Object S229+RECT
    [void][S229]::GetWindowRect($dlg, [ref]$out)
    $outW = $out.R - $out.L; $outH = $out.B - $out.T
    $sanity = ($outW -ge ($ExpCli.w + 4)) -and ($outH -ge ($ExpCli.h + 20))
    Det 'S1.outer' $sanity ('outer ' + $outW + 'x' + $outH + ' frames client ' + $ExpCli.w + 'x' + $ExpCli.h + ' with caption room (weak sanity; DWM frame is machine state)')
    if (-not $sanity) { $ok1 = $false; $ev1 = 'outer ' + $outW + 'x' + $outH + ' cannot contain client ' + $ExpCli.w + 'x' + $ExpCli.h }
}
if ($ok1) {
    $pre = [S229]::EditText($edit)
    Det 'S1.prefill' ($pre -eq 'a') ('edit text=[' + $pre + '] expect the rename stem of a.png ("a")')
    if ($pre -ne 'a') { $ok1 = $false; $ev1 = 'edit prefill [' + $pre + '] != "a"' }
}
if ($ok1) {
    $enOwner = [S229]::IsWindowEnabled($main)
    Det 'S1.modal' (-not $enOwner) ('owner disabled while the modal is up: ' + ((-not $enOwner)))
    if ($enOwner) { $ok1 = $false; $ev1 = 'owner NOT disabled while the rename modal is up' }
}
if ($ok1) {
    Post-Cmd $dlg 2   # Cancel - close it, geometry work done
    $gone = Wait-DlgGone $dlg 5000
    Det 'S1.close' $gone ('cancel dismissed the dialog')
    if (-not $gone) { $ok1 = $false; $ev1 = 'cancel did not dismiss the dialog' }
}
if ($ok1) { $ev1 = 'class/controls/client/rects/styles/prefill/modal all pinned (client ' + $ExpCli.w + 'x' + $ExpCli.h + ')' }
Emit-S 'S1' ($(if ($ok1) { 'PASS' } else { 'FAIL' })) $ev1

# ---------------------------------------------------------------------------
# S2: rename OK - type x9 over the prefilled stem, OK, disk + title.
# ---------------------------------------------------------------------------
$gate2 = $ok1
$ok2 = $ok1
$ev2 = 'cascade: S1 red (dialog machinery not proven)'
if ($ok2) {
    Post-Cmd $main 15
    $dlg = Wait-RenameDlgReady $p.Id
    if ($dlg -eq [IntPtr]::Zero) { $ok2 = $false; $ev2 = 'rename dialog never reappeared for the OK run' }
}
if ($ok2) {
    $edit = [S229]::GetDlgItem($dlg, 1042)
    Type-IntoEdit $edit 'x9'
    Start-Sleep -Milliseconds 300
    $typed = [S229]::EditText($edit)
    Det 'S2.typed' ($typed -eq 'x9') ('edit text=[' + $typed + '] expect [x9]')
    if ($typed -ne 'x9') { $ok2 = $false; $ev2 = 'typing did not land: [' + $typed + '] != [x9]' }
}
if ($ok2) {
    Post-Cmd $dlg 1   # OK (posted to the DIALOG, async - #191 lesson)
    $x9 = Join-Path $Fx 'x9.png'
    $diskOk = Wait-Until { (-not (Test-Path $Afx)) -and (Test-Path $x9) } 15000
    Det 'S2.disk' $diskOk ('a.png gone + x9.png on disk: a=' + (Test-Path $Afx) + ' x9=' + (Test-Path $x9))
    if (-not $diskOk) { $ok2 = $false; $ev2 = 'rename not on disk within 15s (a exists=' + (Test-Path $Afx) + ' x9 exists=' + (Test-Path $x9) + ')' }
}
if ($ok2) {
    $tOk = Wait-TitleHas $main 'x9.png' 15000
    Det 'S2.title' $tOk ('title=[' + (Get-Title $main) + ']')
    if (-not $tOk) { $ok2 = $false; $ev2 = 'title never followed the rename (actual: [' + (Get-Title $main) + '])' }
}
if ($ok2) {
    $dlgGone = Wait-DlgGone $dlg 5000
    $ownerBack = Wait-Until { [S229]::IsWindowEnabled($main) } 5000
    Det 'S2.end' ($dlgGone -and $ownerBack) ('dialog gone=' + $dlgGone + ' owner re-enabled=' + $ownerBack)
    if (-not ($dlgGone -and $ownerBack)) { $ok2 = $false; $ev2 = 'modal did not end cleanly (gone=' + $dlgGone + ' ownerEnabled=' + $ownerBack + ')' }
}
if ($ok2) { $ev2 = 'a.png -> x9.png on disk, title=[' + (Get-Title $main) + '], modal ended, owner back' }
Emit-Gate 'S2' $gate2 $ok2 $ev2

# ---------------------------------------------------------------------------
# S3: the playlist row followed: NavNext -> b, NavPrev -> x9 (a's slot).
# ---------------------------------------------------------------------------
$gate3 = $ok2
$ok3 = $ok2
$ev3 = 'cascade: S2 red (rename did not land, playlist state unknown)'
if ($ok3) {
    Post-Cmd $main 107   # NavNext
    $n3a = Wait-TitleHas $main 'b.png' 15000
    Det 'S3.next' $n3a ('after NavNext title=[' + (Get-Title $main) + ']')
    if (-not $n3a) { $ok3 = $false; $ev3 = 'NavNext did not reach b.png (actual: [' + (Get-Title $main) + '])' }
}
if ($ok3) {
    Post-Cmd $main 108   # NavPrev -> the renamed row
    $n3b = Wait-TitleHas $main 'x9.png' 15000
    Det 'S3.prev' $n3b ('after NavPrev title=[' + (Get-Title $main) + ']')
    if (-not $n3b) { $ok3 = $false; $ev3 = 'NavPrev did not land on the RENAMED row x9.png (actual: [' + (Get-Title $main) + '])' }
}
if ($ok3) { $ev3 = 'playlist row renamed (next->b, prev->x9 round-trip)' }
Emit-Gate 'S3' $gate3 $ok3 $ev3

# ---------------------------------------------------------------------------
# S4: Cancel path - on b.png, type z, Cancel: file untouched.
# ---------------------------------------------------------------------------
$gate4 = $ok3
$ok4 = $ok3
$ev4 = 'cascade: S3 red (navigation state unknown)'
if ($ok4) {
    Post-Cmd $main 107   # -> b.png
    $onb = Wait-TitleHas $main 'b.png' 15000
    Det 'S4.nav' $onb ('on b.png: title=[' + (Get-Title $main) + ']')
    if (-not $onb) { $ok4 = $false; $ev4 = 'did not reach b.png (actual: [' + (Get-Title $main) + '])' }
}
if ($ok4) {
    Post-Cmd $main 15
    $dlg = Wait-RenameDlgReady $p.Id
    if ($dlg -eq [IntPtr]::Zero) { $ok4 = $false; $ev4 = 'rename dialog never appeared for the cancel run' }
}
if ($ok4) {
    $edit = [S229]::GetDlgItem($dlg, 1042)
    Type-IntoEdit $edit 'z'
    Start-Sleep -Milliseconds 300
    $tz = [S229]::EditText($edit)
    Det 'S4.typed' ($tz -eq 'z') ('edit text=[' + $tz + '] expect [z]')
    if ($tz -ne 'z') { $ok4 = $false; $ev4 = 'typing did not land: [' + $tz + '] != [z]' }
}
if ($ok4) {
    Post-Cmd $dlg 2   # Cancel
    $gone = Wait-DlgGone $dlg 5000
    Start-Sleep -Milliseconds 500
    $bIntact = (Test-Path $Bfx) -and (-not (Test-Path (Join-Path $Fx 'z.png')))
    $ownerBack = Wait-Until { [S229]::IsWindowEnabled($main) } 5000
    Det 'S4.intact' ($gone -and $bIntact -and $ownerBack) ('dlg gone=' + $gone + ' b.png intact=' + $bIntact + ' owner back=' + $ownerBack + ' title=[' + (Get-Title $main) + ']')
    if (-not ($gone -and $bIntact -and $ownerBack)) { $ok4 = $false; $ev4 = 'cancel left damage: gone=' + $gone + ' bIntact=' + $bIntact + ' ownerEnabled=' + $ownerBack }
}
if ($ok4) { $ev4 = 'cancel: b.png untouched (no z.png), title=[' + (Get-Title $main) + ']' }
Emit-Gate 'S4' $gate4 $ok4 $ev4

# ---------------------------------------------------------------------------
# S5: WM_CLOSE collapses to Cancel - on c.png, close via WM_CLOSE.
# ---------------------------------------------------------------------------
$gate5 = $ok4
$ok5 = $ok4
$ev5 = 'cascade: S4 red (navigation state unknown)'
if ($ok5) {
    Post-Cmd $main 107   # -> c.png
    $onc = Wait-TitleHas $main 'c.png' 15000
    Det 'S5.nav' $onc ('on c.png: title=[' + (Get-Title $main) + ']')
    if (-not $onc) { $ok5 = $false; $ev5 = 'did not reach c.png (actual: [' + (Get-Title $main) + '])' }
}
if ($ok5) {
    Post-Cmd $main 15
    $dlg = Wait-RenameDlgReady $p.Id
    if ($dlg -eq [IntPtr]::Zero) { $ok5 = $false; $ev5 = 'rename dialog never appeared for the WM_CLOSE run' }
}
if ($ok5) {
    # Type a DIFFERENT name first (cubic P2): with the prefilled "c" the
    # check cannot tell Cancel from OK - an OK would leave c.png on disk
    # too. With "z" typed, only a cancel-collapse keeps z.png absent.
    $edit = [S229]::GetDlgItem($dlg, 1042)
    Type-IntoEdit $edit 'z'
    Start-Sleep -Milliseconds 300
    $tz5 = [S229]::EditText($edit)
    Det 'S5.typed' ($tz5 -eq 'z') ('edit text=[' + $tz5 + '] expect [z]')
    if ($tz5 -ne 'z') { $ok5 = $false; $ev5 = 'typing did not land: [' + $tz5 + '] != [z]' }
}
if ($ok5) {
    [void][S229]::PostMessage($dlg, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_CLOSE
    $gone = Wait-DlgGone $dlg 5000
    Start-Sleep -Milliseconds 300
    $cIntact = (Test-Path $Cfx) -and (-not (Test-Path (Join-Path $Fx 'z.png')))
    Det 'S5.intact' ($gone -and $cIntact) ('dlg gone=' + $gone + ' c.png intact + no z.png=' + $cIntact + ' title=[' + (Get-Title $main) + ']')
    if (-not ($gone -and $cIntact)) { $ok5 = $false; $ev5 = 'WM_CLOSE did not collapse to a harmless cancel (gone=' + $gone + ' cIntact=' + $cIntact + ')' }
}
if ($ok5) { $ev5 = 'WM_CLOSE collapsed to cancel, c.png untouched' }
Emit-Gate 'S5' $gate5 $ok5 $ev5

# ---------------------------------------------------------------------------
# Teardown: WM_CLOSE, poll exit, ini discipline. Failures tally FAIL.
# ---------------------------------------------------------------------------
if ($main -ne [IntPtr]::Zero) { [void][S229]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
$code = Wait-Exit $p 30000
if ($code -eq $null) {
    Write-Host ('  [teardown] did not exit within 30s, killing pid ' + $p.Id)
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    $p.WaitForExit(3000) | Out-Null
    $exitNote = 'FORCE-KILLED'
    Emit-S 'T.exit' 'FAIL' 'forced kill after 30s'
} elseif ($code -ne 0) {
    $exitNote = 'exit=' + $code
    Emit-S 'T.exit' 'FAIL' ('abnormal exit code ' + $code)
} else {
    $exitNote = 'exit=0'
    Emit-S 'T.exit' 'PASS' 'clean exit 0'
}
Write-Host ('  [teardown] app closed: ' + $exitNote)

$iniCleaned = $true
if (Test-Path $Ini) {
    Remove-Item $Ini -Force -ErrorAction SilentlyContinue
    $iniCleaned = -not (Test-Path $Ini)
}
Det 'T.ini' $iniCleaned ('this-run staged ini removed: ' + $iniCleaned)
if (-not $iniCleaned) { $script:fail++ }
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object {
    $lp = $null
    try { $lp = $_.Path } catch { $lp = $null }
    ($lp -ne $null) -and ($lp.StartsWith($Stage + '\', [StringComparison]::OrdinalIgnoreCase))
})
Det 'T.procs' ($leftover.Count -eq 0) ('staged riviv processes left: ' + (($leftover | ForEach-Object { $_.Id }) -join ','))
if ($leftover.Count -gt 0) {
    # Ours by construction (stage-path filter above) - safe to clear by pid.
    $leftover | Stop-Process -Force -ErrorAction SilentlyContinue
    $script:fail++
}
# Stop the transcript BEFORE removing the stage: the log file lives in the
# stage and an open transcript handle would make the directory delete fail
# partially (P3 from the cubic review).
Stop-Log
Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
$stageClean = -not (Test-Path $StageExe)
Det 'T.stage' $stageClean ('stage cleaned: ' + $stageClean)
if (-not $stageClean) { $script:fail++ }   # a leftover stage is a FAIL, not a note (codex P2)

if ($script:fail -gt 0) {
    Write-Output 'FAILURES: riviv stderr follows (if any).'
    if (Test-Path $ErrPath) {
        $body = ''
        try { $body = (Get-Content $ErrPath -Raw -ErrorAction SilentlyContinue) } catch { $body = '(stderr unreadable)' }
        if ($body -and ($body.Trim().Length -gt 0)) { Write-Output $body }
    }
}
if (Test-Path $ErrPath) { Remove-Item $ErrPath -Force -ErrorAction SilentlyContinue }

Write-Output ('SMOKE229 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip + ' iniCleaned=' + $iniCleaned)
Stop-Log
if ($script:fail -gt 0) { exit 1 }
exit 0

} catch {
    Write-Output ('ERROR: unexpected exception mid-matrix: ' + $_.Exception.Message)
    Write-Output $_.ScriptStackTrace
    if (($p -ne $null) -and (-not $p.HasExited)) {
        Write-Host ('killing our pid ' + $p.Id)
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path $Ini) { Remove-Item $Ini -Force -ErrorAction SilentlyContinue }
    Stop-Log
    Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path $ErrPath) {
        # Emit the redirected stderr before deleting it (cubic P3): the
        # normal failure path prints it; an unexpected-exception path that
        # silently discards it hides the app's own diagnostics.
        $body = ''
        try { $body = (Get-Content $ErrPath -Raw -ErrorAction SilentlyContinue) } catch { $body = '(stderr unreadable)' }
        if ($body -and ($body.Trim().Length -gt 0)) { Write-Output $body }
        Remove-Item $ErrPath -Force -ErrorAction SilentlyContinue
    }
    $script:fail++
    Write-Output ('SMOKE229 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' SKIP=' + $script:skip + ' iniCleaned=False')
    exit 1
}
