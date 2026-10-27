# smoke237 - #237 trusted update: download + ed25519 verify + install
# trust anchor (PR #238, ADR 0009). GUI smoke over the REAL GitHub channel.
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79/#124 lesson).
# Harness cribbed from smoke229: poll-based waits everywhere (posted-message
# latency is unbounded), BM_CLICK for message-box buttons (#124: a direct
# WM_COMMAND IDOK cannot close a MessageBox, BM_CLICK on the button does),
# WM_COMMAND posted async for menu ids, GetExitCodeProcess on handles
# captured WHILE ALIVE, path-targeted process handling, staged-exe-only
# runs, staged-ini precheck + cleanup.
#
# Source pins:
#   src/menu.rs:  Cmd::HelpCheckUpdates = WM_COMMAND 127 (unit-pinned,
#       "command_ids_stay_pinned_for_the_smoke_scripts").
#   src/copydata.rs: single-instance mutex name "RIVIV", created in run()
#       BEFORE the window exists; a second launch forwards its command line
#       to the owner window and exits 0 WITHOUT creating a window.
#   src/window.rs: run() order is install-pass -> HELPER SHORT-CIRCUIT
#       (self_replace::parse_helper_line, exits with the helper's code) ->
#       RIVIV mutex gate (forward+return) -> STARTUP SWEEP
#       (self_replace::sweep_staging beside the exe AND %TEMP%) -> window.
#       The sweep sits AFTER the gate: a forwarded launch exits BEFORE it
#       can sweep (see S1 adaptation note below).
#   src/self_replace.rs: helper switch `-update-self-to <orig>`; waits on
#       the PARENT handle inherited via STARTF_USESTDHANDLES/hStdInput
#       (read back through GetStdHandle(STD_INPUT_HANDLE)); dangling switch
#       -> empty path -> exit 2 before any file work; parent not gone within
#       60 s -> give up, exit 2; then read own image -> write <orig>.tmp ->
#       ReplaceFileW(orig, tmp, backup=<orig>.old) -> launch new orig ->
#       delete <orig>.old -> exit 0. The helper's own image
#       (riviv-update-*.exe) is deleted by the NEXT riviv startup's sweep.
#   src/update_install.rs: Available box is MB_YESNOCANCEL (YES=6
#       download+verify+install, NO=7 releases page, CANCEL=2 dismiss);
#       YES runs one flight: fetch 64 B .sig -> stream asset to
#       <staging>\riviv-update-<tag>.exe -> verify -> handoff. Any failure
#       -> the download-failed box (MB_OK), old version untouched, no
#       partial left. Flavor by Uninstall.exe adjacency (portable here).
#   Boxes: class #32770, caption "riviv"; body read via
#       GetDlgItem(dlg, 0xFFFF) + WM_GETTEXT (StringBuilder must declare
#       CharSet.Unicode, #124 lesson).
#
# Real-channel facts (verified 2026-10-27): GitHub latest = v0.8.0 with
# NO .sig assets -> for the 0.0.1 probe a reached channel fails the YES arm
# at the sig fetch (HTTP 404 body > 64 B) and shows the download-failed box.
# CAVEAT observed the same afternoon: the machine's egress IP had exhausted
# github's UNAUTHENTICATED API quota (curl: HTTP 403, body is a JSON object
# without tag_name) - the CHECK then fails at tag_name and the check-failed
# box shows instead of Available; S3/S5 are reported BLOCKED with the box
# text + the stderr check breadcrumb as evidence. That is the honest
# network-failure arm of S3, on the live channel.
#
# Scenario table (all sequential):
#   S0 gates: both exes present (new / baseline arm, same version, DIFFERENT
#       bytes - the self-replace assertion is SHA-based, never version-based);
#       living-process sweep (only riviv/riviv-update-* processes with >=1
#       thread under our paths are killed; 0-thread tombstones pass; a
#       foreign living riviv BLOCKS); staged dirs built under
#       %TEMP%\riviv-237-smoke (old\ has riviv.exe=BASELINE +
#       riviv-update-v0.9.0.exe=NEW - the staging name deliberately says
#       v0.9.0); staged-ini precheck (delete any riviv.ini first).
#   S1 helper self-replace end-to-end (THE core):
#       a) a REAL riviv window (new exe, staged elsewhere) is up and holds
#          the RIVIV mutex;
#       b) PS P/Invokes CreateProcessW MIRRORING THE PRODUCTION spawn_helper
#          SHAPE (ab51bcd, review #238 P1): lpApplicationName = NULL (the
#          module comes from the command line's first QUOTED word - a
#          quoted appname is a literal path WITH quotes = ERROR_INVALID_NAME
#          123; the pre-fix smoke passed an explicit unquoted appname and
#          silently bypassed the bug), no env/cwd override, lpCommandLine =
#          the full quoted line, STARTF_USESTDHANDLES and hStdInput = an
#          inherited PROCESS_SYNCHRONIZE handle to a sacrificial ping
#          process (bInheritHandles=TRUE); after 2 s the helper is still
#          alive, riviv.exe still has the BASELINE sha, and no .tmp/.old
#          exists (still waiting);
#       c) kill the sacrifice -> poll <=30 s: riviv.exe sha == NEW sha,
#          riviv.exe.old absent, .tmp absent, helper exit code 0, the
#          helper staging file still on disk (normal: only a future startup
#          sweeps it), and the mutex-owning window NEVER noticed;
#       d) contrast (adaptation, see below): launching the replaced
#          riviv.exe while the mutex owner is alive forwards and exits 0
#          fast WITHOUT sweeping (the helper staging file survives);
#       e) close the owner window -> launch the replaced riviv.exe again ->
#          it now reaches the startup sweep: the staging file AND a planted
#          riviv.exe.tmp residue (the ab51bcd predicate arm - a crash
#          between write-tmp and ReplaceFile would strand a half-build
#          forever) are deleted within 15 s and the new code opens a normal
#          window; close it.
#   S2 helper dangling switch: `riviv-update-v0.9.0.exe -update-self-to`
#       (no arg) from a fresh substage -> exit code 2, no MessageBox at any
#       point during its life, the replaced riviv.exe bytes untouched.
#   S3 three-button wiring + network-failure arm (real channel): 0.0.1
#       probe exe (one-off worktree build, version patched 0.8.0 -> 0.0.1,
#       worktree removed after) staged WITHOUT Uninstall.exe = portable;
#       WM_COMMAND 127 -> Available box (#32770): EXACTLY 3 button children
#       with ids 6/7/2, body contains the available text (zh or en);
#       BM_CLICK YES -> download-failed box within 60 s (sig 404 on the
#       real channel), body contains the failed text; BM_CLICK OK closes;
#       probe still alive; no riviv-update-* residue beside the exe or in
#       %TEMP%. Again 127 -> Available -> BM_CLICK IDCANCEL -> box gone and
#       NO follow-up box within 3 s. Probe window closes cleanly (exit 0).
#   S4 tamper arm (tool level): the cross-implementation lock vector
#       (seed 00..1f, asset 44 B, tag v0.1.0) verified OK with riviv-sign,
#       then the asset's LAST BYTE flipped (same file NAME, different dir -
#       the signed message binds the file name, so only the byte differs)
#       -> verify exits 1 with VERIFY-FAIL; the pristine asset re-verifies
#       exit 0 VERIFY-OK. Vectors rebuilt from scratch if swept.
#   S5 disk-failure arm: probe copied into a directory that icacls /deny
#       (W) makes unwritable (runnable: write denied only); 127 ->
#       Available -> BM_CLICK YES -> download-failed box; probe alive; no
#       riviv-update-* ANYWHERE (ro dir, probe dir, %TEMP%); the stderr
#       breadcrumb proves the install flight ran and targeted the ro dir.
#       HONEST LIMITATION (in the header and in the evidence): on today's
#       real channel the sig fetch 404s BEFORE File::create, so the
#       "sig-in-memory + create-fails" mechanism the ticket sketched is
#       shadowed; what this arm asserts is the user-level contract (box +
#       zero residue + old version untouched) with the deny verified at the
#       filesystem level. If icacls cannot produce the deny, S5 is BLOCKED.
#   S6 regression: grep smoke/*.ps1 for pins of the retired notify-only
#       Available copy ("Open the download page" / its zh rendering, built
#       from char codes to keep this file ASCII) - REPORT ONLY, never a
#       FAIL; then smoke78-view-child.ps1 is re-run against the probe exe
#       (it takes -Exe) and its DONE line is mapped 1:1; falling back to
#       the new exe is stated in the evidence if the probe is unavailable.
#   S7 install-flavor language arm: the probe runs INSTALLED (a dummy
#       Uninstall.exe beside it = the whole detect_flavor adjacency test),
#       127 -> Available -> BM_CLICK YES -> the flight targets the
#       LOCALIZED Setup asset (riviv-0.8.0.x64.<locale>-Setup.exe; locale
#       read off the box text's language), the breadcrumb pins
#       flavor=Installed and a staging path OUTSIDE the exe dir (Installed
#       staging = %TEMP%), the channel's missing Setup .sig lands the
#       download-failed box, zero staging residue anywhere, the dummy
#       marker untouched, clean exit.
#
# ADAPTATIONS vs the ticket sketch (all code-grounded, not behavior
# drift): (1) `git worktree add --detach` - the branch is checked out in
# the main tree, a plain add refuses; (2) S1 d/e ordering - the startup
# sweep sits AFTER the mutex gate (window.rs: the forwarded branch returns
# before the sweep), so the residual-deletion assertion needs the owner
# closed first; while the owner is alive the forwarded launch is itself
# asserted (exit 0, no sweep) as the contrast that proves the helper's
# short-circuit; (3) the probe build is done outside the script by default
# (pass -ProbeExe); without it the script builds its own worktree probe.
#
# Output contract: one line per scenario ("S<n> <name>: PASS/FAIL/BLOCKED
# <evidence>"), final line
#   "SMOKE237 RESULT: PASS=n FAIL=n BLOCKED=n log=<path>".
# Exit codes: 0 no FAIL, 1 any FAIL, 2 missing exe, 4 foreign living riviv.
# Full log: %TEMP%\riviv-237-smoke\smoke237.log (transcript tee; the stage
# SUBDIRECTORIES are removed at teardown, the root + log deliberately stay).

param(
    [string]$NewExe   = 'D:\codespace\riviv\target\release\riviv.exe',
    [string]$OldExe   = 'D:\codespace\riviv-armA-237\target\release\riviv.exe',
    [string]$SignExe  = 'D:\codespace\riviv\tools\riviv-sign\target\release\riviv-sign.exe',
    [string]$Repo     = 'D:\codespace\riviv',
    [string]$Branch   = 'feat-237-trusted-install',
    [string]$ProbeExe = ''
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class S237 {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr hwnd, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, StringBuilder sb, uint flags, uint timeout, out IntPtr result);

    // CharSet.Unicode is REQUIRED: without it the StringBuilder command
    // line marshals as ANSI bytes and CreateProcessW reads them as UTF-16
    // garbage (the W entry point). applicationName/currentDirectory are
    // IntPtr - NOT string: a literal $null string param is bound by PS as
    // an EMPTY-STRING buffer and lpApplicationName="" is ERROR_INVALID_NAME
    // (the diag237 lesson); IntPtr Zero is a REAL NULL. This is exactly the
    // production spawn_helper shape (ab51bcd, review #238 P1): appname NULL
    // (module from the command line's first QUOTED word), no env/cwd
    // override.
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool CreateProcessW(IntPtr applicationName, StringBuilder commandLine, IntPtr procAttrs, IntPtr threadAttrs, bool inheritHandles, uint creationFlags, IntPtr environment, IntPtr currentDirectory, ref STARTUPINFOW si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll")] public static extern bool SetHandleInformation(IntPtr h, uint mask, uint flags);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] public static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    [StructLayout(LayoutKind.Sequential)]
    public struct STARTUPINFOW {
        public uint cb;
        public IntPtr lpReserved;
        public IntPtr lpDesktop;
        public IntPtr lpTitle;
        public uint dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute;
        public uint dwFlags;   // STARTF_* bits; field ORDER matters (x64 layout)
        public ushort wShowWindow, cbReserved2;
        public IntPtr lpReserved2;
        public IntPtr hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION {
        public IntPtr hProcess;
        public IntPtr hThread;
        public uint dwProcessId;
        public uint dwThreadId;
    }

    public static string ClassOf(IntPtr h) {
        StringBuilder sb = new StringBuilder(256);
        GetClassName(h, sb, 256);
        return sb.ToString();
    }
    public static List<IntPtr> TopWindows() {
        List<IntPtr> all = new List<IntPtr>();
        EnumProc cb = delegate(IntPtr h, IntPtr lp) { all.Add(h); return true; };
        EnumWindows(cb, IntPtr.Zero);
        return all;
    }
    public static List<IntPtr> Children(IntPtr h) {
        List<IntPtr> all = new List<IntPtr>();
        EnumProc cb = delegate(IntPtr c, IntPtr lp) { all.Add(c); return true; };
        EnumChildWindows(h, cb, IntPtr.Zero);
        return all;
    }
    // Box body text: the MessageBox static (id 0xFFFF) via WM_GETTEXT
    // (0x000D); the StringBuilder marshaling NEEDS CharSet.Unicode on the
    // SendMessageTimeout overload (the #124 lesson - an ANSI declaration
    // reads garbage from a Unicode class).
    public static string BoxText(IntPtr dlg) {
        IntPtr st = GetDlgItem(dlg, 0xFFFF);
        if (st == IntPtr.Zero) return "(nostatic)";
        StringBuilder sb = new StringBuilder(4096);
        IntPtr got;
        SendMessageTimeout(st, 0x000D, (IntPtr)4096, sb, 0x0002 | 0x0008, 4000, out got);
        return sb.ToString();
    }
}

'@

# ---------------------------------------------------------------------------
# Stage paths. Every riviv under test runs from a staged copy under
# %TEMP%\riviv-237-smoke; the repo target dirs are never executed in place
# and never hold a riviv.ini from these runs.
# ---------------------------------------------------------------------------
$Stage    = Join-Path $env:TEMP 'riviv-237-smoke'
$OldDir   = Join-Path $Stage 'old'      # baseline install dir + staged helper
$WinDir   = Join-Path $Stage 'win'      # the mutex-owner window's dir
$S2Dir    = Join-Path $Stage 's2'       # dangling-switch helper copy
$ProbeDir = Join-Path $Stage 'probe'    # 0.0.1 probe (portable)
$RoDir    = Join-Path $Stage 'ro'       # write-denied copy for S5
$TampDir  = Join-Path $Stage 'tamper'   # S4 same-name tampered asset
$S7Dir    = Join-Path $Stage 's7'       # S7 probe + dummy Uninstall.exe (Installed)
$LogPath  = Join-Path $Stage 'smoke237.log'
$OldTarget = Join-Path $OldDir 'riviv.exe'
$HelperFile = Join-Path $OldDir 'riviv-update-v0.9.0.exe'
$OldIni  = Join-Path $OldDir 'riviv.ini'
$WinIni  = Join-Path $WinDir 'riviv.ini'
$ErrNew  = Join-Path $Stage 'probe-stderr.log'
$ErrRo   = Join-Path $Stage 'ro-stderr.log'
$ErrS7   = Join-Path $Stage 's7-stderr.log'

# Box-text match strings, built from char codes so THIS FILE STAYS PURE
# ASCII (the PS 5.1 param-binding trap). The disjunctions cover the en-US,
# zh-CN and zh-Hant renderings of the two #237 boxes as pinned in loc.rs
# (Id::UpdateNewVersionText / Id::UpdateDownloadFailedText at loc.rs 1012-1021
# en, 1360-1369 zh-CN, 1710-1719 zh-Hant).
$zhAvail = -join @([char]0x5DF2, [char]0x53EF, [char]0x7528)   # zh-CN: available
$zhAvailTw = -join @([char]0x5DF2, [char]0x53EF, [char]0x4F7F, [char]0x7528)  # zh-Hant
$zhFail = -join @([char]0x4E0B, [char]0x8F7D, [char]0x5931, [char]0x8D25)     # zh-CN: dl failed
$zhFailTw = -join @([char]0x4E0B, [char]0x8F09, [char]0x5931, [char]0x6557)   # zh-Hant
function Test-AvailText($s) {
    return $s.Contains('is available') -or $s.Contains($zhAvail) -or $s.Contains($zhAvailTw)
}
function Test-FailText($s) {
    return $s.Contains('Download failed') -or $s.Contains($zhFail) -or $s.Contains($zhFailTw)
}
# The #210 check-failed box ("Update check failed..." / zh / zh-TW): the
# honest BLOCKED marker for the network arms - seeing it means the real
# channel is unreachable, not that the wiring is wrong.
$zhCheck = -join @([char]0x66F4, [char]0x65B0, [char]0x68C0, [char]0x67E5, [char]0x5931, [char]0x8D25)      # zh-CN
$zhCheckTw = -join @([char]0x66F4, [char]0x65B0, [char]0x6AA2, [char]0x67E5, [char]0x5931, [char]0x8D25)   # zh-Hant
function Test-CheckFailText($s) {
    return $s.Contains('Update check failed') -or $s.Contains($zhCheck) -or $s.Contains($zhCheckTw)
}
# Poll for the Available box OR the check-failed box. Returns a hashtable:
#   dlg     - the box hwnd (Available: the 3-button box; netFail: the
#             check-failed box; none: Zero)
#   netFail - $true when the single-button check-failed box showed
#   text    - the text of whichever box was seen
# Button-id note (diag237g, this machine's build 26300): a single-button
# MessageBoxW(MB_OK) box's button enumerates as control id 2, NOT the
# classic IDOK 1 - so the check-failed box is classified by its ONE Button
# child + the body text, and clicked by enumeration (Click-FirstButton).
# The Available box keeps the classic id assertion (6/7/2, per ticket).
function Wait-Avail-Or-NetFail($owner, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        $b = Find-Box $owner
        if ($b -ne [IntPtr]::Zero) {
            $t = [S237]::BoxText($b)
            if ([S237]::GetDlgItem($b, 6) -ne [IntPtr]::Zero) {
                # Available box: settle until all three buttons exist.
                $r = Wait-BoxReady $owner @(6, 7, 2) 5000
                return @{ dlg = $r; netFail = $false; text = $t }
            }
            $btns = Box-Buttons $b
            if (($btns.Count -eq 1) -and (Test-CheckFailText $t)) {
                return @{ dlg = $b; netFail = $true; text = $t }
            }
        }
        Start-Sleep -Milliseconds 200
    }
    return @{ dlg = [IntPtr]::Zero; netFail = $false; text = '' }
}

$script:pass = 0
$script:fail = 0
$script:blocked = 0
function Emit-S($id, $name, $state, $ev) {
    if ($state -eq 'PASS') { $script:pass++ }
    elseif ($state -eq 'FAIL') { $script:fail++ }
    elseif ($state -eq 'BLOCKED') { $script:blocked++ }
    Write-Output ($id + ' ' + $name + ': ' + $state + ' ' + $ev)
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
function Invoke-Quiet($sb) {
    # PS 5.1: under EAP=Stop a redirected native-stderr line becomes a
    # terminating record (cargo/git/icacls all write stderr); controlled
    # native calls run under Continue and are judged by $LASTEXITCODE.
    $eap = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $sb } finally { $ErrorActionPreference = $eap }
}
function Sha256($path) { return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
function Sha8($path) { return (Sha256 $path).Substring(0, 8) }
function Clear-Ini($dir) {
    $ini = Join-Path $dir 'riviv.ini'
    if (Test-Path $ini) {
        Remove-Item $ini -Force -ErrorAction SilentlyContinue
        return $true
    }
    return $false
}
function Get-Residue($dir) {
    @(Get-ChildItem -LiteralPath $dir -Filter 'riviv-update-*.exe' -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })
}
function Wait-MainWindow($p, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        $p.Refresh()
        $h = $p.MainWindowHandle
        if (($h -ne $null) -and ($h -ne [IntPtr]::Zero)) { return $h }
        if ($p.HasExited) { return [IntPtr]::Zero }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Wait-Exit($p, $ms) {
    # The handle must have been captured WHILE ALIVE (smoke80 lesson).
    if ($p.WaitForExit($ms)) {
        $code = 0
        [void][S237]::GetExitCodeProcess($p.Handle, [ref]$code)
        return $code
    }
    return $null
}
function Post-Cmd($hwnd, $id) {
    # WM_COMMAND(wParam = id), posted ASYNC (smoke229 #191 lesson).
    [void][S237]::PostMessage($hwnd, 0x0111, [IntPtr]$id, [IntPtr]::Zero)
}
function Find-Box($owner) {
    foreach ($h in [S237]::TopWindows()) {
        $wpid = 0
        [void][S237]::GetWindowThreadProcessId($h, [ref]$wpid)
        if (($wpid -eq $owner) -and ([S237]::ClassOf($h) -eq '#32770')) { return $h }
    }
    return [IntPtr]::Zero
}
function Wait-BoxReady($owner, $ids, $ms) {
    # A message box is enumerable between CreateWindowExW and control
    # build: wait until the body static AND every expected button id exist,
    # then settle. $ids is an array of button control ids.
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        $d = Find-Box $owner
        if ($d -ne [IntPtr]::Zero) {
            $ready = ([S237]::GetDlgItem($d, 0xFFFF) -ne [IntPtr]::Zero)
            foreach ($i in $ids) { if ([S237]::GetDlgItem($d, $i) -eq [IntPtr]::Zero) { $ready = $false } }
            if ($ready) { Start-Sleep -Milliseconds 200; return (Find-Box $owner) }
        }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Wait-BoxGone($dlg, $ms) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not [S237]::IsWindow($dlg)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return (-not [S237]::IsWindow($dlg))
}
function Click-Button($dlg, $id) {
    # BM_CLICK (0x00F5) on the button control - the #124 lesson: a direct
    # WM_COMMAND IDOK cannot close a MessageBox, the button walk does.
    $btn = [S237]::GetDlgItem($dlg, $id)
    if ($btn -eq [IntPtr]::Zero) { return $false }
    [void][S237]::SendMessage($btn, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    return $true
}
function Click-FirstButton($dlg) {
    # BM_CLICK the box's enumerated-first Button child - the diag237g note:
    # this build's single-button boxes do NOT carry the classic IDOK 1.
    $btns = Box-Buttons $dlg
    if ($btns.Count -eq 0) { return $false }
    $btn = [S237]::GetDlgItem($dlg, $btns[0])
    if ($btn -eq [IntPtr]::Zero) { return $false }
    [void][S237]::SendMessage($btn, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    return $true
}
function Wait-FailedBox($owner, $ms) {
    # The download-failed box (MB_OK): ONE Button child + the failed body
    # text; control ids are not assumed (diag237g note).
    $deadline = [DateTime]::UtcNow.AddMilliseconds($ms)
    while ([DateTime]::UtcNow -lt $deadline) {
        $b = Find-Box $owner
        if ($b -ne [IntPtr]::Zero) {
            $btns = Box-Buttons $b
            if (($btns.Count -eq 1) -and (Test-FailText ([S237]::BoxText($b)))) {
                Start-Sleep -Milliseconds 200
                return (Find-Box $owner)
            }
        }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Box-Buttons($dlg) {
    $ids = @()
    foreach ($c in [S237]::Children($dlg)) {
        if ([S237]::ClassOf($c) -eq 'Button') { $ids += [S237]::GetDlgCtrlID($c) }
    }
    return $ids
}
function Fmt20($s) {
    if ($s.Length -gt 20) { return $s.Substring(0, 20) }
    return $s
}
function Test-AliveRaw($h) { return ([S237]::WaitForSingleObject($h, 0) -eq 258) }   # 258 = WAIT_TIMEOUT (0x102): handle not signaled = alive. 259 (0x103) is GetExitCodeProcess's STILL_ACTIVE - a DIFFERENT constant (first-run lesson: the wait that never returns was comparing 259 and read a live helper as dead).
function Read-ExitRaw($h) {
    $c = 0
    [void][S237]::GetExitCodeProcess($h, [ref]$c)
    return $c
}

# ---------------------------------------------------------------------------
# S0 gates - BEFORE anything else.
# ---------------------------------------------------------------------------
if (-not (Test-Path $NewExe)) { Write-Output ('MISSING EXE: ' + $NewExe); exit 2 }
if (-not (Test-Path $OldExe)) { Write-Output ('MISSING EXE: ' + $OldExe); exit 2 }

# Fresh stage root (the log is created after this wipe, then kept).
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force -ErrorAction SilentlyContinue }
if (Test-Path (Join-Path $Stage 'riviv.exe')) {
    Write-Output ('BLOCKED: stage root could not be cleared (' + $Stage + ').')
    exit 4
}
New-Item -ItemType Directory -Path $Stage -ErrorAction Stop | Out-Null

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

# The premise: same version, DIFFERENT bytes. The self-replace assertion is
# SHA-based (both arms report 0.8.0), so equal bytes would void S1.
$newSha = Sha256 $NewExe
$oldSha = Sha256 $OldExe
Det 'S0.exes' ($newSha -ne $oldSha) ('new=' + $newSha.Substring(0, 8) + ' baseline=' + $oldSha.Substring(0, 8) + ' (must differ)')

# LIVING-PROCESS sweep, path-targeted: kill riviv / riviv-update-* processes
# with at least one thread whose path is under OUR territory (repo, the
# baseline worktree, the stage); 0-thread tombstones always pass (.HasExited
# lies); a living foreign riviv BLOCKS (it would own the RIVIV mutex and
# hijack every launch here).
$oldRoot = Split-Path (Split-Path $OldExe)
$territory = @(($Repo.TrimEnd('\') + '\'), ($oldRoot.TrimEnd('\') + '\'), ($Stage.TrimEnd('\') + '\'))
$killed = @()
$foreign = @()
foreach ($p in (Get-Process | Where-Object { ($_.Name -eq 'riviv') -or ($_.Name -like 'riviv-update-*') })) {
    $path = $null
    try { $path = $p.Path } catch { $path = $null }
    $threads = -1
    try { $threads = $p.Threads.Count } catch { }
    if ($threads -eq 0) { continue }
    $ours = $false
    foreach ($t in $territory) {
        if ($path -and ($path.StartsWith($t, [StringComparison]::OrdinalIgnoreCase))) { $ours = $true; break }
    }
    if ($ours) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        $killed += ('pid=' + $p.Id + ' ' + $path)
    } else {
        $foreign += ('pid=' + $p.Id + ' path=' + $path + ' threads=' + $threads)
    }
}
if ($foreign.Count -gt 0) {
    Stop-Log
    Write-Output ('BLOCKED: living riviv process(es) not under this script''s paths - close them and rerun (' + ($foreign -join '; ') + ').')
    exit 4
}
Start-Sleep -Milliseconds 800   # let the kills settle before staging copies
$still = @(Get-Process | Where-Object { ($_.Name -eq 'riviv') -or ($_.Name -like 'riviv-update-*') } | Where-Object {
    $t2 = -1
    try { $t2 = $_.Threads.Count } catch { }
    $t2 -gt 0
})
Det 'S0.sweep' ($still.Count -eq 0) ('killed=[' + ($killed -join ';') + '] still-living=' + $still.Count)
if ($still.Count -gt 0) {
    Stop-Log
    Write-Output 'BLOCKED: territory riviv process(es) survived the sweep (locked exe would fake the staging).'
    exit 4
}

# Stage prep: substage dirs. old\ carries the BASELINE bytes as riviv.exe
# and the NEW bytes under the helper staging name riviv-update-v0.9.0.exe
# (the name deliberately says v0.9.0 - the sweep matches the prefix, and no
# assertion may read the name as truth).
$prepOk = $true
try {
    foreach ($d in @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $TampDir, $S7Dir)) {
        New-Item -ItemType Directory -Path $d -Force -ErrorAction Stop | Out-Null
    }
    Copy-Item $OldExe $OldTarget -Force -ErrorAction Stop
    Copy-Item $NewExe $HelperFile -Force -ErrorAction Stop
    Copy-Item $NewExe (Join-Path $WinDir 'riviv.exe') -Force -ErrorAction Stop
    Copy-Item $NewExe (Join-Path $S2Dir 'riviv-update-v0.9.0.exe') -Force -ErrorAction Stop
} catch {
    Det 'S0.stage' $false ('stage preparation failed: ' + $_.Exception.Message)
    $prepOk = $false
}
$iniPre = @()
foreach ($d in @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $S7Dir)) {
    if (Clear-Ini $d) { $iniPre += (Split-Path $d -Leaf) }
}
Det 'S0.stage' $prepOk ('staged: old\riviv.exe=' + (Sha8 $OldTarget) + ' helper(new bytes)=' + (Sha8 $HelperFile) + ' iniPrecheckCleared=[' + ($iniPre -join ',') + ']')

$gate0 = $prepOk -and ($newSha -ne $oldSha) -and ($still.Count -eq 0)
Emit-S 'S0' 'gates' ($(if ($gate0) { 'PASS' } else { 'FAIL' })) ('new=' + $newSha.Substring(0, 8) + ' baseline=' + $oldSha.Substring(0, 8) + ' killed=' + $killed.Count)

try {

# ===========================================================================
# S1: helper self-replace end-to-end (the core proposition: the -update-
# self-to short-circuit sits BEFORE the RIVIV mutex, so a helper launch is
# never forwarded to the running old instance).
# ===========================================================================
$ok1 = $gate0
$ev1 = 'cascade: S0 red'
if ($ok1) {
    # (a) the mutex owner: a REAL window from the NEW exe, another dir.
    Clear-Ini $WinDir
    $script:pa = Start-Process -FilePath (Join-Path $WinDir 'riviv.exe') -PassThru -WorkingDirectory $WinDir
    $script:pa.Handle | Out-Null
    $wa = Wait-MainWindow $script:pa 15000
    $ok1 = ($wa -ne [IntPtr]::Zero) -and (-not $script:pa.HasExited)
    Det 'S1.a' $ok1 ('owner window pid=' + $script:pa.Id + ' hwnd=' + $wa + ' alive=' + (-not $script:pa.HasExited))
    if (-not $ok1) { $ev1 = 'owner window never came up (alive=' + (-not $script:pa.HasExited) + ')' }
}
$sac = $null
$hHelper = [IntPtr]::Zero
$helperPid = 0
if ($ok1) {
    # (b) the sacrificial parent: a long ping. The helper waits on an
    # inherited PROCESS_SYNCHRONIZE handle to IT (riviv's own flow hands
    # the same kind of handle to its parent; here we control the moment).
    Clear-Ini $OldDir
    $sac = Start-Process -FilePath (Join-Path $env:windir 'System32\ping.exe') -ArgumentList '-n', '300', '127.0.0.1' -PassThru -WindowStyle Hidden
    $sac.Handle | Out-Null
    $sacH = [S237]::OpenProcess(0x00100000, $false, [uint32]$sac.Id)   # PROCESS_SYNCHRONIZE
    if ($sacH -eq [IntPtr]::Zero) {
        $ok1 = $false; $ev1 = 'OpenProcess(sacrifice) failed'
    }
    if ($ok1) {
        # HANDLE_FLAG_INHERIT (0x1) so the CreateProcessW(TRUE) child gets it.
        [void][S237]::SetHandleInformation($sacH, 1, 1)
        $cmd = New-Object System.Text.StringBuilder
        [void]$cmd.Append(('"' + $HelperFile + '" -update-self-to "' + $OldTarget + '"'))
        $cmd.Capacity = 2048
        $si = New-Object 'S237+STARTUPINFOW'
        $si.cb = [uint32][System.Runtime.InteropServices.Marshal]::SizeOf([type]'S237+STARTUPINFOW')
        $si.dwFlags = [uint32]0x00000100   # STARTF_USESTDHANDLES
        $si.hStdInput = $sacH
        $pi = New-Object 'S237+PROCESS_INFORMATION'
        # MIRRORS the production spawn_helper call shape (ab51bcd, review
        # #238 P1): lpApplicationName = NULL - the module comes from the
        # command line's first QUOTED word; a QUOTED lpApplicationName is a
        # literal path WITH the quotes in it = ERROR_INVALID_NAME (123), the
        # on-machine P/Invoke proof behind the fix. No env/cwd override,
        # like production. The IntPtr-typed params pass a REAL NULL (a
        # literal $null string param would be bound by PS as an
        # EMPTY-STRING buffer and fail with the same 123 - the diag237
        # lesson). The first smoke run passed an explicit UNQUOTED appname,
        # which silently bypassed the production bug - this harness now
        # reproduces the call verbatim.
        $spawned = [S237]::CreateProcessW([IntPtr]::Zero, $cmd, [IntPtr]::Zero, [IntPtr]::Zero, $true, [uint32]0, [IntPtr]::Zero, [IntPtr]::Zero, [ref]$si, [ref]$pi)
        [void][S237]::CloseHandle($sacH)   # the child holds its own copy
        if (-not $spawned) {
            $ok1 = $false; $ev1 = 'CreateProcessW(helper) failed'
        } else {
            $hHelper = $pi.hProcess
            $helperPid = $pi.dwProcessId
            [void][S237]::CloseHandle($pi.hThread)
            Write-Host ('  [S1.b.spawn] PASS helper pid=' + $helperPid + ' waits on sacrifice pid=' + $sac.Id)
        }
    }
}
if ($ok1) {
    Start-Sleep -Milliseconds 2000
    $waiting = Test-AliveRaw $hHelper
    $shaNow = Sha256 $OldTarget
    $tmpThere = Test-Path (Join-Path $OldDir 'riviv.exe.tmp')
    $oldThere = Test-Path (Join-Path $OldDir 'riviv.exe.old')
    $ok1 = $waiting -and ($shaNow -eq $oldSha) -and (-not $tmpThere) -and (-not $oldThere)
    Det 'S1.b.wait' $ok1 ('helper alive=' + $waiting + ' sha=' + $shaNow.Substring(0, 8) + ' (baseline ' + $oldSha.Substring(0, 8) + ') tmp=' + $tmpThere + ' old=' + $oldThere)
    if (-not $ok1) {
        $ev1 = 'helper did NOT stay waiting: alive=' + $waiting + ' sha-moved=' + ($shaNow -ne $oldSha)
        if (-not $waiting) { $ev1 += ' exitCode=' + (Read-ExitRaw $hHelper) }
    }
}
if ($ok1) {
    # (c) the parent dies -> the swap. Poll the helper's exit <=30 s.
    Stop-Process -Id $sac.Id -Force -ErrorAction SilentlyContinue
    $exited = Wait-Until { -not (Test-AliveRaw $hHelper) } 30000
    $code = 0
    if ($exited) { $code = Read-ExitRaw $hHelper }
    $shaNow = Sha256 $OldTarget
    $tmpThere = Test-Path (Join-Path $OldDir 'riviv.exe.tmp')
    $oldThere = Test-Path (Join-Path $OldDir 'riviv.exe.old')
    $helperThere = Test-Path $HelperFile
    $ownerAlive = (-not $script:pa.HasExited)
    $ok1 = $exited -and ($code -eq 0) -and ($shaNow -eq $newSha) -and (-not $oldThere) -and (-not $tmpThere) -and $helperThere -and $ownerAlive
    Det 'S1.c.swap' $ok1 ('helper exited=' + $exited + ' code=' + $code + ' sha=' + $shaNow.Substring(0, 8) + ' (new ' + $newSha.Substring(0, 8) + ') .old=' + $oldThere + ' .tmp=' + $tmpThere + ' stagingFileKept=' + $helperThere + ' ownerStillUp=' + $ownerAlive)
    if (-not $ok1) {
        $ev1 = 'swap failed: exited=' + $exited + ' code=' + $code + ' sha-ok=' + ($shaNow -eq $newSha) + ' .old=' + $oldThere + ' ownerAlive=' + $ownerAlive
    } else {
        $ev1 = 'sha ' + $oldSha.Substring(0, 8) + ' -> ' + $newSha.Substring(0, 8) + ' exit=0 .old gone, helper staging file kept (sweep is a future startup''s job), mutex owner never noticed'
    }
}
if ($ok1) {
    # (d) the contrast, code-grounded: a NORMAL launch while the owner
    # holds the mutex forwards and exits 0 BEFORE the startup sweep - so
    # the helper staging file must survive this launch too (gate-before-
    # sweep ordering, window.rs).
    Clear-Ini $OldDir
    $pf = Start-Process -FilePath $OldTarget -PassThru -WorkingDirectory $OldDir
    $pf.Handle | Out-Null
    $fcode = Wait-Exit $pf 10000
    $fFast = ($fcode -ne $null)
    $helperStill = Test-Path $HelperFile
    $ownerAlive = (-not $script:pa.HasExited)
    $ok1 = $fFast -and ($fcode -eq 0) -and $helperStill -and $ownerAlive
    Det 'S1.d.forward' $ok1 ('normal launch forwarded: exitedFast=' + $fFast + ' code=' + $fcode + ' stagingFileSurvived=' + $helperStill + ' ownerAlive=' + $ownerAlive)
    if (-not $ok1) { $ev1 = 'forwarded-launch contrast failed: code=' + $fcode + ' helperFile=' + $helperStill }
}
if ($ok1) {
    # (e) owner closes (mutex released) -> the NEXT start reaches the sweep:
    # the staging file is deleted and the replaced exe opens a normal window.
    # ab51bcd widened the sweep predicate with <exe>.tmp (a crash between
    # write-tmp and ReplaceFile would strand a half-built file forever) -
    # plant a LIVE residue and demand its removal alongside the staging file.
    if ($script:pa.MainWindowHandle -ne [IntPtr]::Zero) {
        [void][S237]::PostMessage($script:pa.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    $acode = Wait-Exit $script:pa 30000
    Det 'S1.e.closeOwner' ($acode -eq 0) ('owner closed code=' + $acode)
    Clear-Ini $OldDir
    $tmpResidue = Join-Path $OldDir 'riviv.exe.tmp'
    [IO.File]::WriteAllBytes($tmpResidue, [byte[]](1, 2, 3))
    $pr = Start-Process -FilePath $OldTarget -PassThru -WorkingDirectory $OldDir
    $pr.Handle | Out-Null
    $swept = Wait-Until { (-not (Test-Path $HelperFile)) -and (-not (Test-Path $tmpResidue)) } 15000
    $wr = Wait-MainWindow $pr 15000
    $tmpGone = -not (Test-Path $tmpResidue)
    $ok1 = $swept -and ($wr -ne [IntPtr]::Zero) -and $tmpGone
    Det 'S1.e.sweep' $ok1 ('staging file swept=' + (-not (Test-Path $HelperFile)) + ' planted .tmp residue swept=' + $tmpGone + ' replaced exe window=' + ($wr -ne [IntPtr]::Zero))
    if ($ok1) {
        [void][S237]::PostMessage($wr, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        $rcode = Wait-Exit $pr 30000
        Det 'S1.e.exit' ($rcode -eq 0) ('replaced exe clean exit code=' + $rcode)
        if ($rcode -ne 0) { $ok1 = $false; $ev1 = 'replaced exe exit code ' + $rcode }
        else { $ev1 += '; next start swept the helper file and the replaced exe opened + exited cleanly' }
    } else {
        $ev1 = 'startup sweep failed: swept=' + $swept + ' window=' + ($wr -ne [IntPtr]::Zero)
        if ($wr -ne [IntPtr]::Zero) { [void][S237]::PostMessage($wr, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
        if (-not $pr.HasExited) { Stop-Process -Id $pr.Id -Force -ErrorAction SilentlyContinue }
    }
}
Emit-S 'S1' 'helper-self-replace' ($(if ($ok1) { 'PASS' } else { 'FAIL' })) $ev1
# Owner hygiene: S1.e normally closed the mutex owner; if S1 went red
# earlier, close it here no matter what - a leaked owner forwards every
# later launch and poisons S3/S5/S6 (the first-run lesson: everything
# downstream failed as "window never came up" because the forwarded
# launches exited 0 into the stale owner).
if (($script:pa -ne $null) -and (-not $script:pa.HasExited)) {
    $mh = $script:pa.MainWindowHandle
    if (($mh -ne $null) -and ($mh -ne [IntPtr]::Zero)) {
        [void][S237]::PostMessage($mh, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    $null = Wait-Exit $script:pa 15000
    if (-not $script:pa.HasExited) { Stop-Process -Id $script:pa.Id -Force -ErrorAction SilentlyContinue }
    Write-Host ('  [S1.hygiene] owner still up after S1 -> closed: ' + $script:pa.HasExited)
}
if ($hHelper -ne [IntPtr]::Zero) { [void][S237]::CloseHandle($hHelper) }
if (($sac -ne $null) -and (-not $sac.HasExited)) { Stop-Process -Id $sac.Id -Force -ErrorAction SilentlyContinue }

# ===========================================================================
# S2: helper dangling switch -> exit 2, no box, staged bytes untouched.
# ===========================================================================
$ok2 = $gate0
$ev2 = 'cascade: S0 red'
if ($ok2) {
    Clear-Ini $S2Dir
    $sha2Before = Sha256 $OldTarget   # captured AFTER S1 - the assertion is "the dangling run moved nothing", whatever S1 left behind
    $helper2 = Join-Path $S2Dir 'riviv-update-v0.9.0.exe'
    $p2 = Start-Process -FilePath $helper2 -ArgumentList '-update-self-to' -PassThru -WorkingDirectory $S2Dir
    $p2.Handle | Out-Null
    # Poll exit AND box-presence together: the malformed-argument path must
    # never surface a fail-soft MessageBox (that one is only for swap I/O).
    $sawBox = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Find-Box $p2.Id) -ne [IntPtr]::Zero) { $sawBox = $true }
        if ($p2.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    $code2 = Wait-Exit $p2 5000
    $sha2 = Sha256 $OldTarget
    $ok2 = ($code2 -eq 2) -and (-not $sawBox) -and ($sha2 -eq $sha2Before)
    Det 'S2' $ok2 ('exit=' + $code2 + ' boxShown=' + $sawBox + ' targetShaUnchanged=' + ($sha2 -eq $sha2Before))
    if ($ok2) { $ev2 = 'dangling -update-self-to: exit 2, no MessageBox, staged bytes untouched' }
    else { $ev2 = 'exit=' + $code2 + ' (expect 2) box=' + $sawBox }
}
Emit-S 'S2' 'helper-dangling-arg' $(if ($ok2) { 'PASS' } else { 'FAIL' }) $ev2

# ===========================================================================
# The 0.0.1 probe (gates S3/S5/S6b): a one-off worktree with the version
# patched to 0.0.1, built and copied out; the worktree is removed.
# ===========================================================================
$script:probeStage = Join-Path $ProbeDir 'riviv.exe'
$probeOk = $false
$probeEv = ''
if ($ProbeExe -and (Test-Path $ProbeExe)) {
    Copy-Item $ProbeExe $script:probeStage -Force
    $probeOk = $true
    $probeEv = 'prebuilt probe staged from ' + $ProbeExe
} else {
    $wt = Join-Path $env:TEMP 'riviv-237-probe'
    try {
        Invoke-Quiet { & git -C $Repo worktree remove --force $wt 2>$null }
        if (Test-Path $wt) { Remove-Item $wt -Recurse -Force -ErrorAction SilentlyContinue }
        Invoke-Quiet { & git -C $Repo worktree prune }
        Invoke-Quiet { & git -C $Repo worktree add --detach $wt $Branch | Out-Null }
        if ($LASTEXITCODE -ne 0) { throw ('worktree add failed, rc=' + $LASTEXITCODE) }
        # The main tree owns the branch checkout, hence --detach above. The
        # patch round-trips the file AS UTF-8 BYTES (ReadAllText/WriteAllText
        # with an explicit no-BOM UTF8): a Get-Content/Set-Content dance
        # reads the Chinese comments in the ANSI codepage and the GBK decode
        # swallows newline bytes - `resvg` / `oxideav-avif` merge into
        # comment lines, cargo resolves a dependency-less manifest, and the
        # build fails with "unlinked crate" for every leaf dep.
        $utf8 = New-Object System.Text.UTF8Encoding($false)
        $toml = Join-Path $wt 'Cargo.toml'
        $text = [IO.File]::ReadAllText($toml, [Text.Encoding]::UTF8)
        $patched = $text.Replace('version = "0.8.0"', 'version = "0.0.1"')
        if ($patched -eq $text) { throw 'Cargo.toml version patch did not land' }
        [IO.File]::WriteAllText($toml, $patched, $utf8)
        # Structural sanity: the manifest still resolves the pinned leaves.
        $treeLine = Invoke-Quiet { & cargo tree -e normal --depth 1 --manifest-path $toml 2>$null | Select-String -Pattern 'oxideav-avif|resvg' }
        if (-not $treeLine) { throw 'normal dep tree lost oxideav-avif/resvg (encoding corruption)' }
        $log = Join-Path $wt 'probe-build.log'
        Invoke-Quiet { Push-Location $wt; try { & cargo build --release *> $log } finally { Pop-Location } }
        $brc = $LASTEXITCODE
        if ($brc -ne 0) {
            Get-Content $log -Tail 30 | ForEach-Object { Write-Host ('  [probe-build] ' + $_) }
            throw ('cargo build failed, rc=' + $brc)
        }
        Copy-Item (Join-Path $wt 'target\release\riviv.exe') $script:probeStage -Force
        $probeOk = $true
        $probeEv = 'worktree probe built and staged (sha ' + (Sha8 $script:probeStage) + ')'
    } catch {
        $probeEv = 'probe build failed: ' + $_.Exception.Message
    } finally {
        Invoke-Quiet { & git -C $Repo worktree remove --force $wt 2>$null }
        if (Test-Path $wt) { Remove-Item $wt -Recurse -Force -ErrorAction SilentlyContinue }
        Invoke-Quiet { & git -C $Repo worktree prune }
    }
}
if ($probeOk) {
    Clear-Ini $ProbeDir
    $pd = Sha256 $script:probeStage
    Det 'probe' ($pd -ne $newSha) ('probe sha=' + $pd.Substring(0, 8) + ' differs from new exe=' + ($pd -ne $newSha) + ' - ' + $probeEv)
}
Write-Host ('  [probe.gate] ' + $(if ($probeOk) { 'PASS' } else { 'BLOCKED' }) + ' ' + $probeEv)

# ===========================================================================
# S3: three-button Available wiring + the network-failure arm on the REAL
# channel (today: v0.8.0 latest, no .sig asset -> YES fails at the sig
# fetch with the download-failed box).
# ===========================================================================
$ok3 = $probeOk
$ev3 = 'BLOCKED: probe exe unavailable (' + $probeEv + ')'
if ($ok3) {
    $pp = Start-Process -FilePath $script:probeStage -PassThru -WorkingDirectory $ProbeDir -RedirectStandardError $ErrNew
    $pp.Handle | Out-Null
    $wp3 = Wait-MainWindow $pp 15000
    $ok3 = ($wp3 -ne [IntPtr]::Zero) -and (-not $pp.HasExited)
    if (-not $ok3) { $ev3 = 'probe window never came up' }
}
$dlg3 = [IntPtr]::Zero
$netFail3 = $false
$script:s3Done = $false
if ($ok3) {
    Post-Cmd $wp3 127
    # 120 s: the box must appear whether the channel answers fast (a 403
    # body in <1 s) or hangs through WinHTTP's 5/5/5/10 timeouts plus
    # resolver time; the first runs missed a slow-failing channel at 45 s.
    $res3 = Wait-Avail-Or-NetFail $pp.Id 120000
    $dlg3 = $res3.dlg
    $netFail3 = $res3.netFail
    if ($netFail3) {
        # The real channel gave no usable verdict (today: the unauthenticated
        # github API quota of this egress IP is exhausted - 403, body has no
        # tag_name - verified by hand with curl). The wiring is not on trial;
        # what IS asserted: 127 ran the check (box + breadcrumb) and the app
        # failed soft with the check-failed box.
        [void](Click-FirstButton $dlg3)
        [void](Wait-BoxGone $dlg3 8000)
        [void][S237]::PostMessage($wp3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pp 15000)
        if (-not $pp.HasExited) { Stop-Process -Id $pp.Id -Force -ErrorAction SilentlyContinue }
        $crumb3 = ''
        try { $crumb3 = (Get-Content $ErrNew -Raw -ErrorAction Stop) } catch { $crumb3 = '' }
        $crumbLine3 = ''
        foreach ($ln in ($crumb3 -split "`n")) { if ($ln -match 'update check:') { $crumbLine3 = $ln.Trim() } }
        $ev3 = 'BLOCKED: real channel unusable - check-failed box showed ([' + (Fmt20 $res3.text) + ']) check breadcrumb=[' + $crumbLine3 + '] (probe closed)'
        Emit-S 'S3' 'three-buttons-network-arm' 'BLOCKED' $ev3
        $script:s3Done = $true
        $ok3 = $false
    } elseif ($dlg3 -eq [IntPtr]::Zero) {
        # No box of either kind within the window: close the probe BEFORE
        # reporting - a leaked mutex owner forward-poisons S5/S6/T (the
        # first-run lesson).
        [void][S237]::PostMessage($wp3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pp 15000)
        if (-not $pp.HasExited) { Stop-Process -Id $pp.Id -Force -ErrorAction SilentlyContinue }
        $ok3 = $false
        $ev3 = 'no Available and no check-failed box within 120s (probe closed; check hung? see stderr in the log)'
    }
}
$btnIds3 = @()
$txt3 = ''
if ($ok3) {
    $btnIds3 = Box-Buttons $dlg3
    $txt3 = [S237]::BoxText($dlg3)
    $idsOk = (($btnIds3.Count -eq 3) -and ($btnIds3 -contains 6) -and ($btnIds3 -contains 7) -and ($btnIds3 -contains 2))
    $txtOk = Test-AvailText $txt3
    $ok3 = $idsOk -and $txtOk
    Det 'S3.box' $ok3 ('buttons=' + $btnIds3.Count + ' ids=[' + (($btnIds3 | Sort-Object) -join ',') + '] (expect 3: 2,6,7) text=[' + (Fmt20 $txt3) + ']')
    if (-not $ok3) { $ev3 = 'Available box wrong: buttons=' + (($btnIds3 | Sort-Object) -join ',') + ' text=[' + (Fmt20 $txt3) + ']' }
}
$dlg3b = [IntPtr]::Zero
if ($ok3) {
    # YES -> the install flight. The real channel 404s the .sig (no such
    # asset): the download-failed box, within the transport's timeouts.
    [void](Click-Button $dlg3 6)
    [void](Wait-BoxGone $dlg3 8000)
    $dlg3b = Wait-FailedBox $pp.Id 60000
    $ok3 = ($dlg3b -ne [IntPtr]::Zero)
    if (-not $ok3) { $ev3 = 'download-failed box never appeared within 60s' }
}
$txt3b = ''
if ($ok3) {
    $txt3b = [S237]::BoxText($dlg3b)
    $ok3 = Test-FailText $txt3b
    Det 'S3.failed' $ok3 ('failed-box text=[' + (Fmt20 $txt3b) + ']')
    if (-not $ok3) { $ev3 = 'failed-box text mismatch: [' + (Fmt20 $txt3b) + ']' }
}
if ($ok3) {
    [void](Click-FirstButton $dlg3b)
    $gone = Wait-BoxGone $dlg3b 8000
    $alive = -not $pp.HasExited
    $res = @(Get-Residue $ProbeDir) + @(Get-Residue $env:TEMP)
    $ok3 = $gone -and $alive -and ($res.Count -eq 0)
    Det 'S3.afterYes' $ok3 ('box closed=' + $gone + ' probe alive=' + $alive + ' residue=' + ($res -join ';'))
    if (-not $ok3) { $ev3 = 'after YES: gone=' + $gone + ' alive=' + $alive + ' residue=' + ($res -join ';') }
}
if ($ok3) {
    # Round 2: CANCEL must dismiss with NO follow-up box.
    Post-Cmd $wp3 127
    $dlg3c = Wait-BoxReady $pp.Id @(6, 7, 2) 30000
    $up = ($dlg3c -ne [IntPtr]::Zero)
    if ($up) {
        [void](Click-Button $dlg3c 2)
        $goneC = Wait-BoxGone $dlg3c 8000
        Start-Sleep -Milliseconds 3000
        $noFollow = ((Find-Box $pp.Id) -eq [IntPtr]::Zero)
        $ok3 = $goneC -and $noFollow
        Det 'S3.cancel' $ok3 ('cancel closed=' + $goneC + ' no follow-up box=' + $noFollow)
        if (-not $ok3) { $ev3 = 'CANCEL left a follow-up box or stayed open' }
    } else {
        $ok3 = $false
        $ev3 = 'second 127 never re-presented the Available box (single-flight wedge?)'
    }
}
$code3 = $null
if ($ok3) {
    [void][S237]::PostMessage($wp3, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    $code3 = Wait-Exit $pp 30000
    if ($code3 -eq $null) { Stop-Process -Id $pp.Id -Force -ErrorAction SilentlyContinue }
    # The install flight's own breadcrumb: YES must have run the fetch
    # pipeline for the real portable asset (stderr survives the GUI exe
    # only through the redirect file, read AFTER exit).
    $crumb3 = ''
    try { $crumb3 = Get-Content $ErrNew -Raw -ErrorAction Stop } catch { $crumb3 = '' }
    $crumb3Ok = $crumb3.Contains('update install: fetching riviv-0.8.0-portable.exe')
    $ok3 = ($code3 -eq 0) -and $crumb3Ok
    Det 'S3.exit' $ok3 ('probe exit code=' + $code3 + ' install-flight breadcrumb=' + $crumb3Ok)
    if (-not $ok3) { $ev3 = 'probe exit code ' + $code3 + ' breadcrumb=' + $crumb3Ok }
    else { $ev3 = '3 buttons 2/6/7, yes->failed box [' + (Fmt20 $txt3b) + '], cancel round clean, exit 0, zero residue' }
}
if (-not $script:s3Done) {
    Emit-S 'S3' 'three-buttons-network-arm' $(if ($probeOk) { $(if ($ok3) { 'PASS' } else { 'FAIL' }) } else { 'BLOCKED' }) $ev3
}

# ===========================================================================
# S4: tamper arm at the TOOL level (riviv-sign verify against the cross-
# implementation lock vector; the tampered copy keeps the FILE NAME so the
# only difference is the flipped byte).
# ===========================================================================
$PubHex = '03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8'
$seedF  = Join-Path $env:TEMP 'vec-seed.bin'
$assetF = Join-Path $env:TEMP 'riviv-0.2.0-portable.exe'
$sigF   = Join-Path $env:TEMP 'riviv-0.2.0-portable.exe.sig'
$tampF  = Join-Path $TampDir 'riviv-0.2.0-portable.exe'
$ok4 = $gate0 -and (Test-Path $SignExe)
$ev4 = 'cascade'
if (-not (Test-Path $SignExe)) { $ok4 = $false; $ev4 = 'BLOCKED: riviv-sign tool missing at ' + $SignExe }
if ($ok4) {
    # Vector self-heal (the %TEMP% copies are swept by real riviv startups).
    if (-not (Test-Path $seedF)) { [IO.File]::WriteAllBytes($seedF, [byte[]](0..31)) }
    if (-not (Test-Path $assetF)) {
        [IO.File]::WriteAllBytes($assetF, [Text.Encoding]::ASCII.GetBytes('riviv #237 cross-implementation lock vector' + [char]10))
    }
    if (-not (Test-Path $sigF)) {
        $so = & $SignExe sign $seedF 'v0.1.0' $assetF 2>&1
        if ($LASTEXITCODE -ne 0) { $ok4 = $false; $ev4 = 'vector re-sign failed: ' + ($so -join ' ') }
    }
    $ab = [IO.File]::ReadAllBytes($assetF)
    Det 'S4.vector' $ok4 ('seed 32B asset ' + $ab.Length + 'B (expect 44) sig=' + (Test-Path $sigF))
    if ($ab.Length -ne 44) { $ok4 = $false; $ev4 = 'vector asset is ' + $ab.Length + ' B, expect 44' }
}
$vo = ''
if ($ok4) {
    $vo = (& $SignExe verify $PubHex 'v0.1.0' $assetF $sigF 2>&1 | Select-Object -Last 1)
    $vok = ($LASTEXITCODE -eq 0) -and (($vo -join '') -like 'VERIFY-OK*')
    Det 'S4.orig' $vok ('pristine verify rc=' + $LASTEXITCODE + ' [' + ($vo -join '') + ']')
    if (-not $vok) { $ok4 = $false; $ev4 = 'pristine verify failed rc=' + $LASTEXITCODE }
}
if ($ok4) {
    # Flip the LAST byte only; same file name in the tamper dir so the
    # signed message's name field is identical and the flip is isolated.
    $tb = [IO.File]::ReadAllBytes($assetF)
    $tb[$tb.Length - 1] = $tb[$tb.Length - 1] -bxor 0xFF
    [IO.File]::WriteAllBytes($tampF, $tb)
    $to = (& $SignExe verify $PubHex 'v0.1.0' $tampF $sigF 2>&1 | Select-Object -Last 1)
    $tfail = ($LASTEXITCODE -eq 1) -and (($to -join '') -like 'VERIFY-FAIL*')
    Det 'S4.tamper' $tfail ('tampered verify rc=' + $LASTEXITCODE + ' [' + ($to -join '') + ']')
    if ($tfail) { $ev4 = 'tampered last byte -> rc=1 VERIFY-FAIL; pristine -> rc=0 VERIFY-OK' }
    else { $ok4 = $false; $ev4 = 'tampered verify rc=' + $LASTEXITCODE + ' [' + ($to -join '') + ']' }
}
$state4 = 'FAIL'
if ($ok4) { $state4 = 'PASS' }
elseif ($ev4 -like 'BLOCKED*') { $state4 = 'BLOCKED' }
Emit-S 'S4' 'sign-tool-tamper' $state4 $ev4

# ===========================================================================
# S5: disk-failure arm - the probe runs from a write-denied directory.
# The deny is REAL (verified at the filesystem level); on today's channel
# the sig fetch 404s before File::create, so the box is the same as S3's -
# the asserted contract is: box + probe alive + ZERO residue + the install
# flight's breadcrumb targeting the ro dir. See the header note.
# ===========================================================================
$ok5 = $probeOk
$ev5 = 'BLOCKED: probe exe unavailable'
if ($ok5) {
    Clear-Ini $RoDir
    Copy-Item $script:probeStage (Join-Path $RoDir 'riviv.exe') -Force
    Invoke-Quiet { & icacls.exe $RoDir /deny "$($env:USERNAME):(W)" | Out-Null }
    $denyRc = $LASTEXITCODE
    $denyWorks = $false
    if ($denyRc -eq 0) {
        try {
            [IO.File]::WriteAllText((Join-Path $RoDir 'wtest.tmp'), 'x')
            $denyWorks = $false   # the write SUCCEEDED: the deny is a no-op
        } catch { $denyWorks = $true }
    }
    $ok5 = $denyWorks
    Det 'S5.deny' $ok5 ('icacls deny rc=' + $denyRc + ' write-blocked=' + $denyWorks)
    if (-not $ok5) { $ev5 = 'BLOCKED: icacls could not produce a write-deny (rc=' + $denyRc + ')' }
}
$pr5 = $null
$wp5 = [IntPtr]::Zero
if ($ok5) {
    $pr5 = Start-Process -FilePath (Join-Path $RoDir 'riviv.exe') -PassThru -WorkingDirectory $Stage -RedirectStandardError $ErrRo
    $pr5.Handle | Out-Null
    $wp5 = Wait-MainWindow $pr5 15000
    $ok5 = ($wp5 -ne [IntPtr]::Zero) -and (-not $pr5.HasExited)
    if (-not $ok5) { $ev5 = 'probe did not start from the write-denied dir' }
}
$dlg5 = [IntPtr]::Zero
if ($ok5) {
    Post-Cmd $wp5 127
    $res5a = Wait-Avail-Or-NetFail $pr5.Id 120000
    $dlg5 = $res5a.dlg
    if ($res5a.netFail) {
        [void](Click-FirstButton $res5a.dlg)
        [void](Wait-BoxGone $res5a.dlg 8000)
        [void][S237]::PostMessage($wp5, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pr5 15000)
        if (-not $pr5.HasExited) { Stop-Process -Id $pr5.Id -Force -ErrorAction SilentlyContinue }
        $crumb5n = ''
        try { $crumb5n = (Get-Content $ErrRo -Raw -ErrorAction Stop) } catch { $crumb5n = '' }
        $crumbLine5n = ''
        foreach ($ln in ($crumb5n -split "`n")) { if ($ln -match 'update check:') { $crumbLine5n = $ln.Trim() } }
        $ev5 = 'BLOCKED: real channel unusable - check-failed box showed ([' + (Fmt20 $res5a.text) + ']) breadcrumb=[' + $crumbLine5n + ']; the deny mechanics were still proven (see S5.deny/S5.restore)'
        $ok5 = $false
    } elseif ($dlg5 -eq [IntPtr]::Zero) {
        [void][S237]::PostMessage($wp5, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pr5 15000)
        if (-not $pr5.HasExited) { Stop-Process -Id $pr5.Id -Force -ErrorAction SilentlyContinue }
        $ok5 = $false
        $ev5 = 'no Available and no check-failed box within 120s from the write-denied dir (probe closed)'
    }
}
$txt5b = ''
if ($ok5) {
    [void](Click-Button $dlg5 6)
    [void](Wait-BoxGone $dlg5 8000)
    $dlg5b = Wait-FailedBox $pr5.Id 60000
    $up = ($dlg5b -ne [IntPtr]::Zero)
    $txtOk = $false
    if ($up) {
        $txt5b = [S237]::BoxText($dlg5b)
        $txtOk = Test-FailText $txt5b
        [void](Click-FirstButton $dlg5b)
        [void](Wait-BoxGone $dlg5b 8000)
    }
    $alive5 = ($pr5 -ne $null) -and (-not $pr5.HasExited)
    $res5 = @(Get-Residue $RoDir) + @(Get-Residue $ProbeDir) + @(Get-Residue $env:TEMP)
    # Close FIRST, then read the breadcrumbs (PS holds the redirect file
    # open while the process runs - a mid-flight read can fail).
    if ($wp5 -ne [IntPtr]::Zero) { [void][S237]::PostMessage($wp5, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    $code5 = Wait-Exit $pr5 15000
    if ($code5 -eq $null) {
        Stop-Process -Id $pr5.Id -Force -ErrorAction SilentlyContinue
        Write-Host '  [S5.close] forced kill (exit code not asserted under the write-deny)'
    } else {
        Write-Host ('  [S5.close] exit code=' + $code5 + ' (not asserted under the write-deny)')
    }
    $crumb = ''
    try { $crumb = (Get-Content $ErrRo -Raw -ErrorAction Stop) } catch { $crumb = '' }
    $crumbOk = $crumb.Contains('update install: fetching') -and $crumb.Contains($RoDir)
    $ok5 = $up -and $txtOk -and $alive5 -and ($res5.Count -eq 0) -and $crumbOk
    Det 'S5.arm' $ok5 ('failed-box=[' + (Fmt20 $txt5b) + '] aliveAtBox=' + $alive5 + ' residue=' + ($res5 -join ';') + ' breadcrumbTargetedRoDir=' + $crumbOk)
    if ($ok5) {
        $ev5 = 'write-denied dir: failed box shown, probe alive, zero residue; NOTE the sig fetch 404s before File::create on today''s channel (header note)'
    } else {
        $ev5 = 'ro-dir arm: box=' + $up + ' textOk=' + $txtOk + ' alive=' + $alive5 + ' residue=' + ($res5.Count -eq 0) + ' crumb=' + $crumbOk
    }
}
$restoreOk = $false
if (Test-Path $RoDir) {
    Invoke-Quiet { & icacls.exe $RoDir /remove:d "$($env:USERNAME)" | Out-Null }
    try {
        [IO.File]::WriteAllText((Join-Path $RoDir 'wtest2.tmp'), 'y')
        Remove-Item (Join-Path $RoDir 'wtest2.tmp') -Force -ErrorAction SilentlyContinue
        $restoreOk = $true
    } catch { $restoreOk = $false }
    Det 'S5.restore' $restoreOk ('write-deny removed, dir writable again=' + $restoreOk)
}
$state5 = 'FAIL'
if (-not $probeOk) { $state5 = 'BLOCKED' }
elseif ($ok5 -and $restoreOk) { $state5 = 'PASS' }
elseif ($ev5 -like 'BLOCKED*') { $state5 = 'BLOCKED' }
Emit-S 'S5' 'disk-failure-arm' $state5 $ev5

# ===========================================================================
# S7: the INSTALL-flavor language arm. The probe runs INSTALLED - a dummy
# Uninstall.exe sits beside it, which is the whole adjacency test
# (detect_flavor) - so the update targets the LOCALIZED Setup asset
# (riviv-0.8.0.x64.<locale>-Setup.exe; the locale follows the box text's
# language) and, unlike the portable S3, the staging dir is %TEMP%, never
# beside the exe.
# ===========================================================================
$ok7 = $probeOk
$ev7 = 'BLOCKED: probe exe unavailable'
$script:s7Done = $false
$setupName = ''
if ($ok7) {
    Clear-Ini $S7Dir
    Copy-Item $script:probeStage (Join-Path $S7Dir 'riviv.exe') -Force
    # The adjacency marker itself (existence is the whole test; the bytes
    # are a MZ header for plausibility only).
    [IO.File]::WriteAllBytes((Join-Path $S7Dir 'Uninstall.exe'), [byte[]](0x4D, 0x5A, 0x90, 0x00))
    $pp7 = Start-Process -FilePath (Join-Path $S7Dir 'riviv.exe') -PassThru -WorkingDirectory $S7Dir -RedirectStandardError $ErrS7
    $pp7.Handle | Out-Null
    $wp7 = Wait-MainWindow $pp7 15000
    $ok7 = ($wp7 -ne [IntPtr]::Zero) -and (-not $pp7.HasExited)
    if (-not $ok7) { $ev7 = 'probe window never came up (S7 Installed staging)' }
}
if ($ok7) {
    Post-Cmd $wp7 127
    $res7 = Wait-Avail-Or-NetFail $pp7.Id 120000
    if ($res7.netFail) {
        # Channel unusable at the CHECK stage (quota / reachability).
        [void](Click-FirstButton $res7.dlg)
        [void](Wait-BoxGone $res7.dlg 8000)
        [void][S237]::PostMessage($wp7, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pp7 15000)
        if (-not $pp7.HasExited) { Stop-Process -Id $pp7.Id -Force -ErrorAction SilentlyContinue }
        $ev7 = 'BLOCKED: real channel unusable - check-failed box showed ([' + (Fmt20 $res7.text) + ']) (probe closed)'
        Emit-S 'S7' 'install-flavor-language-arm' 'BLOCKED' $ev7
        $script:s7Done = $true
        $ok7 = $false
    } elseif ($res7.dlg -eq [IntPtr]::Zero) {
        [void][S237]::PostMessage($wp7, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        [void](Wait-Exit $pp7 15000)
        if (-not $pp7.HasExited) { Stop-Process -Id $pp7.Id -Force -ErrorAction SilentlyContinue }
        $ok7 = $false
        $ev7 = 'no Available and no check-failed box within 120s (S7; probe closed)'
    } else {
        $dlg7 = $res7.dlg
    }
}
if ($ok7) {
    $btnIds7 = Box-Buttons $dlg7
    $txt7 = [S237]::BoxText($dlg7)
    $idsOk = (($btnIds7.Count -eq 3) -and ($btnIds7 -contains 6) -and ($btnIds7 -contains 7) -and ($btnIds7 -contains 2))
    $txtOk = Test-AvailText $txt7
    # The Setup asset's locale follows the UI language the box text shows.
    if ($txt7.Contains($zhAvail)) { $setupName = 'riviv-0.8.0.x64.zh-CN-Setup.exe' }
    elseif ($txt7.Contains($zhAvailTw)) { $setupName = 'riviv-0.8.0.x64.zh-TW-Setup.exe' }
    elseif ($txt7.Contains('is available')) { $setupName = 'riviv-0.8.0.x64.en-US-Setup.exe' }
    $ok7 = $idsOk -and $txtOk -and ($setupName -ne '')
    Det 'S7.box' $ok7 ('buttons=' + $btnIds7.Count + ' ids=[' + (($btnIds7 | Sort-Object) -join ',') + '] (expect 3: 2,6,7) text=[' + (Fmt20 $txt7) + '] setupAsset=' + $setupName)
    if (-not $ok7) { $ev7 = 'Available box wrong (S7): buttons=' + (($btnIds7 | Sort-Object) -join ',') + ' text=[' + (Fmt20 $txt7) + ']' }
}
if ($ok7) {
    # YES -> the install flight targets the LOCALIZED Setup asset; today's
    # channel ships no .sig beside the Setup -> the sig fetch 404s and the
    # download-failed box shows. The flight's breadcrumbs land in the
    # stderr REDIRECT FILE (no unread-pipe 4K hang); it is read only after
    # the probe has closed and released the handle.
    [void](Click-Button $dlg7 6)
    [void](Wait-BoxGone $dlg7 8000)
    $dlg7b = Wait-FailedBox $pp7.Id 60000
    $up7 = ($dlg7b -ne [IntPtr]::Zero)
    $txtOk7 = $false
    $txt7b = ''
    if ($up7) {
        $txt7b = [S237]::BoxText($dlg7b)
        $txtOk7 = Test-FailText $txt7b
        [void](Click-FirstButton $dlg7b)
        [void](Wait-BoxGone $dlg7b 8000)
    }
    [void][S237]::PostMessage($wp7, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    $code7 = Wait-Exit $pp7 30000
    if ($code7 -eq $null) { Stop-Process -Id $pp7.Id -Force -ErrorAction SilentlyContinue }
    $crumb7 = ''
    try { $crumb7 = (Get-Content $ErrS7 -Raw -ErrorAction Stop) } catch { $crumb7 = '' }
    $fetchOk = ($setupName -ne '') -and $crumb7.Contains('update install: fetching ' + $setupName)
    $flavorOk = $crumb7.Contains('flavor=Installed')
    # Installed staging lives in %TEMP% - the exe dir must never see a
    # staging path in the breadcrumb.
    $stagedOutsideExeDir = ($crumb7.Contains('update install: fetching')) -and (-not $crumb7.Contains($S7Dir))
    $res7left = @(Get-Residue $env:TEMP) + @(Get-Residue $S7Dir)
    $dummyOk = (Test-Path (Join-Path $S7Dir 'Uninstall.exe'))
    $ok7 = $up7 -and $txtOk7 -and ($code7 -eq 0) -and $fetchOk -and $flavorOk -and ($res7left.Count -eq 0) -and $dummyOk
    Det 'S7.arm' $ok7 ('failedBox=[' + (Fmt20 $txt7b) + '] exit=' + $code7 + ' fetchBreadcrumb=' + $fetchOk + ' flavorInstalled=' + $flavorOk + ' stagingNotBesideExe=' + $stagedOutsideExeDir + ' residue=' + ($res7left -join ';') + ' dummyUninstallIntact=' + $dummyOk)
    if ($ok7) {
        $ev7 = 'Installed probe: yes -> localized Setup fetch [' + $setupName + '] -> failed box; zero staging residue (%TEMP% + exe dir), dummy Uninstall.exe untouched, exit 0'
    } else {
        $ev7 = 'S7 arm: box=' + $up7 + ' textOk=' + $txtOk7 + ' exit=' + $code7 + ' fetch=' + $fetchOk + ' flavor=' + $flavorOk + ' residueClean=' + ($res7left.Count -eq 0) + ' dummy=' + $dummyOk + ' stagedOutside=' + $stagedOutsideExeDir
    }
}
if (-not $script:s7Done) {
    $state7 = 'FAIL'
    if (-not $probeOk) { $state7 = 'BLOCKED' }
    elseif ($ok7) { $state7 = 'PASS' }
    elseif ($ev7 -like 'BLOCKED*') { $state7 = 'BLOCKED' }
    Emit-S 'S7' 'install-flavor-language-arm' $state7 $ev7
}

# ===========================================================================
# S6: regression - (a) grep the smoke family for pins of the retired
# notify-only Available copy (report-only, never FAILs); (b) re-run
# smoke78-view-child.ps1 against the probe exe (it takes -Exe).
# ===========================================================================
$zhOpen = (-join @([char]0x6253, [char]0x5F00, [char]0x4E0B, [char]0x8F7D, [char]0x9875, [char]0x9762))
$hits = @(Get-ChildItem (Join-Path $Repo 'smoke') -Filter '*.ps1' |
    Where-Object { $_.Name -ne 'smoke237-trusted-update.ps1' } |
    Select-String -SimpleMatch -Pattern 'Open the download page', $zhOpen)
foreach ($h in $hits) { Write-Host ('  [S6a.pin] ' + $h.Path + ':' + $h.LineNumber + ': ' + $h.Line.Trim()) }
Det 'S6a' $true ('retired-copy pins in smoke/*.ps1: ' + $hits.Count + ' (report-only)')

$ok6 = $false
$ev6 = ''
if ($probeOk) { $s6exe = $script:probeStage } else { $s6exe = $NewExe }
Write-Host ('  [S6b] running smoke78-view-child.ps1 -Exe ' + $s6exe)
# The child writes its own stderr (Add-Type et al); under EAP=Stop PS 5.1
# would turn that into a terminating record - run it under Continue and
# judge it by its exit code + DONE line.
$out78 = Invoke-Quiet { & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Repo 'smoke\smoke78-view-child.ps1') -Exe $s6exe }
$rc78 = $LASTEXITCODE
$out78 | ForEach-Object { Write-Host ('  [s78] ' + $_) }
$done78 = ($out78 | Where-Object { $_ -like 'SMOKE78 DONE*' } | Select-Object -Last 1)
if ($rc78 -eq 0) { $ok6 = $true; $ev6 = 'smoke78 clean (' + $done78 + ', exe=' + $(if ($probeOk) { 'probe' } else { 'new exe (probe unavailable)' }) + ')' }
else { $ev6 = 'smoke78 rc=' + $rc78 + ' (' + $done78 + ')' }
Emit-S 'S6' 'regression' $(if ($ok6) { 'PASS' } else { 'FAIL' }) $ev6

# ===========================================================================
# Teardown: kill anything we staged, remove staged ini files + substage
# dirs, report residue. The stage ROOT + log stay (the log is the tee).
# ===========================================================================
foreach ($p in (Get-Process | Where-Object { ($_.Name -eq 'riviv') -or ($_.Name -like 'riviv-update-*') })) {
    $path = $null
    try { $path = $p.Path } catch { $path = $null }
    if ($path -and ($path.StartsWith($Stage.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase))) {
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        Emit-S 'T' 'stray-process' 'FAIL' ('forced kill of staged pid ' + $p.Id)
    }
}
Start-Sleep -Milliseconds 800   # let force-killed images release their file locks before the removal
foreach ($d in @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $TampDir, $S7Dir)) {
    Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
}
foreach ($d in @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $TampDir, $S7Dir)) {
    if (Test-Path $d) {
        Start-Sleep -Milliseconds 800   # lock release latency, one retry
        Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
    }
}
$stageSubLeft = @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $TampDir, $S7Dir | Where-Object { Test-Path $_ })
Det 'T.stage' ($stageSubLeft.Count -eq 0) ('staged subdirs removed: ' + ($stageSubLeft.Count -eq 0))
if ($stageSubLeft.Count -gt 0) { $script:fail++ }
$tempRes = @(Get-Residue $env:TEMP)
Det 'T.temp' ($tempRes.Count -eq 0) ('%TEMP% riviv-update-* residue: ' + $tempRes.Count)
if ($tempRes.Count -gt 0) { $script:fail++ }

Write-Output ('SMOKE237 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' BLOCKED=' + $script:blocked + ' log=' + $LogPath)
Stop-Log
if ($script:fail -gt 0) { exit 1 }
exit 0

} catch {
    Write-Output ('ERROR: unexpected exception mid-matrix: ' + $_.Exception.Message)
    Write-Output $_.ScriptStackTrace
    foreach ($p in (Get-Process | Where-Object { ($_.Name -eq 'riviv') -or ($_.Name -like 'riviv-update-*') })) {
        $path = $null
        try { $path = $p.Path } catch { $path = $null }
        if ($path -and ($path.StartsWith($Stage.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase))) {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        }
    }
    Start-Sleep -Milliseconds 800   # lock release latency before the removal
    foreach ($d in @($OldDir, $WinDir, $S2Dir, $ProbeDir, $RoDir, $TampDir, $S7Dir)) {
        Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
    }
    $script:fail++
    Write-Output ('SMOKE237 RESULT: PASS=' + $script:pass + ' FAIL=' + $script:fail + ' BLOCKED=' + $script:blocked + ' log=' + $LogPath)
    Stop-Log
    exit 1
}
