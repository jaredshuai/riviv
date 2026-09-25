# smoke124 - #124 About box content through the real window (MessageBox path).
# ASCII-only source (PS 5.1 ANSI/param-binding trap, #79 lesson). Harness
# skeleton cribbed from smoke98/80: staged exe under %TEMP%\riviv-124-smoke
# (the staged exe's riviv.ini lands in the stage and leaves with it), poll-
# based waits (synthetic-message latency is unbounded), Add-Type C# probe
# helper, path-targeted process kills (R2-5: only $RunExe is ever killed -
# the developer's real viewer must survive everything).
#
# The About box opens from WM_COMMAND id 121 (menu.rs Cmd::HelpAbout.id()
# == 121; window.rs dispatches WM_COMMAND on the LOW word only), rendered
# by MessageBoxW with the main window as owner and the caption "riviv"
# (loc AppName). The body (text.rs about_text) carries two non-ASCII
# characters: U+00A9 (c) and U+2014 (em dash). This .ps1 itself stays pure
# ASCII - both characters are constructed at runtime via [char]0xA9 /
# [char]0x2014 and compared ORDINALLY, case-sensitive.
#
# Scenarios:
#   S0 staged exe copied, stage ini absent
#   S1 staged exe, no image arg, empty window: main window (class "riviv",
#      pid = staged pid) up within the poll cap
#   S2 WM_COMMAND 121 to the main window -> About MessageBox appears
#      (FindWindowW class #32770, caption "riviv", pid = staged pid)
#   S3 body text read via GetDlgItem(msgbox, 0xFFFF) + WM_GETTEXT:
#      version line first, license line (U+2014), credit line (U+00A9),
#      both URLs, "Renderer: " prefix (backend NOT pinned - hw/warp are
#      both legal), and the old "Upstream (MIT):" string GONE
#   S4 close the msgbox and keep the main window alive. Empirical ladder,
#   first winner is used and reported: (a) PostMessage WM_COMMAND
#   IDOK(=1); (b) SendMessage WM_COMMAND IDOK; (c) BM_CLICK SendMessage to
#   the OK button (GetDlgItem IDOK / first Button child) - smoke26's
#   verified dialog-close recipe. (a) did NOT close a MessageBox on the
#   dev machine (observed 2026-09-25); the ladder records what does.
#   S5 WM_CLOSE to the main window -> process exit 0; teardown kills only
#      path-filtered staged leftovers and removes the stage on success

param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public class S124 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMsg(IntPtr h, uint m, IntPtr w, IntPtr l);
    // CharSet.Unicode is LOAD-BEARING on the StringBuilder overload: the
    // default CharSet.Ansi marshals the buffer as LPStr, and win32k's
    // UTF-16 copy "r\0i\0v..." reads back as ANSI, truncating at the first
    // 0x00 byte - the first run of this script got exactly "r".
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)] public static extern IntPtr SendGetText(IntPtr h, uint m, IntPtr w, StringBuilder l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("kernel32.dll")] public static extern bool GetExitCodeProcess(IntPtr h, out int code);

    // WM_GETTEXT (0x000D) with a 4096-char StringBuilder. Cross-process
    // WM_GETTEXT is marshaled by user32 itself (the same channel
    // GetWindowText uses), so the pointer never crosses raw.
    public static string ReadText(IntPtr h) {
        if (h == IntPtr.Zero) { return ""; }
        StringBuilder sb = new StringBuilder(4096);
        SendGetText(h, 0x000D, (IntPtr)4096, sb);
        return sb.ToString();
    }
}
'@

[void][S124]::SetProcessDPIAware()
Write-Output ('PS: ' + $PSVersionTable.PSVersion.ToString())

$script:pass = 0
$script:fail = 0
$script:skip = 0
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
# Escape for evidence output: the two non-ASCII chars shown as code-point
# markers, CRLF shown as \n.
function Esc($s) {
    if ($s -eq $null) { return '' }
    return $s.Replace([string][char]0x2014, '<U+2014>').Replace([string][char]0xA9, '<U+00A9>').Replace("`r`n", '\n').Replace("`r", '\n').Replace("`n", '\n')
}
function Show-Escaped($text) {
    $lines = $text.Replace("`r`n", "`n").Replace("`r", "`n") -split "`n"
    foreach ($ln in $lines) { Write-Output ('  | ' + (Esc $ln)) }
}

# Non-ASCII fixture strings, constructed at runtime (the source stays ASCII).
$Em = [string][char]0x2014   # em dash, U+2014
$Co = [string][char]0xA9     # copyright, U+00A9
$WantLicense = 'License: MIT ' + $Em + ' same as upstream.'
$WantCredit  = 'Original C implementation ' + $Co + ' voidtools / David Carpenter.'

# ---------------------------------------------------------------------------
# Stage setup + baseline process discipline.
# ---------------------------------------------------------------------------
$Stage = Join-Path $env:TEMP 'riviv-124-smoke'
$Ini = Join-Path $Stage 'riviv.ini'
$RunExe = Join-Path $Stage 'riviv.exe'

# Any riviv process OUTSIDE the deterministic staged path may be the
# developer's real viewer (single-instance forwarding would hijack the run
# into it). Abort without touching it - that window is not ours to kill.
$foreign = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -ne $RunExe })
if ($foreign.Count -gt 0) {
    Write-Output 'ABORT: live riviv processes outside the stage (possible developer windows) - not touching them:'
    foreach ($f in $foreign) {
        $fp = '<unreadable>'
        try { if ($f.Path) { $fp = $f.Path } } catch {}
        Write-Output ('  pid=' + $f.Id + ' path=' + $fp)
    }
    exit 3
}
# Staged leftovers from a crashed earlier run: killed WITH the path filter.
$stale = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe })
if ($stale.Count -gt 0) { $stale | Stop-Process -Force; Start-Sleep -Milliseconds 300 }

if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
if (-not (Test-Path $Exe)) { Write-Output ('MISSING EXE: ' + $Exe); exit 2 }
Copy-Item $Exe $RunExe -Force
Check 'S0 staged exe copied, stage ini absent' ((Test-Path $RunExe) -and (-not (Test-Path $Ini))) ('exe=' + (Test-Path $RunExe) + ' ini=' + (Test-Path $Ini))

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$CMD_HELP_ABOUT = 121   # menu.rs Cmd::HelpAbout.id()
$IDOK = 1

# ---------------------------------------------------------------------------
# S1: launch the staged exe with no image arg; poll for the main window.
# ---------------------------------------------------------------------------
$err1 = Join-Path $Stage 's1.err'
if (Test-Path $err1) { Remove-Item $err1 -Force }
$p1 = Start-Process -FilePath $RunExe -PassThru -RedirectStandardError $err1
$script:Pid1 = $p1.Id
$script:RawH = $p1.Handle   # captured WHILE ALIVE (smoke98 lesson)
$script:main1 = [IntPtr]::Zero
$up1 = Wait-Until {
    $h = [S124]::FindWindowW('riviv', [NullString]::Value)
    if ($h -ne [IntPtr]::Zero) {
        $wpid = 0
        [void][S124]::GetWindowThreadProcessId($h, [ref]$wpid)
        if ($wpid -eq $script:Pid1) { $script:main1 = $h; return $true }
    }
    return $false
} 15000
Check 'S1 staged exe (no image) shows the main window (class riviv)' ($up1 -and ($script:main1 -ne [IntPtr]::Zero)) ('found=' + $up1 + ' hwnd=' + $script:main1 + ' exited=' + $p1.HasExited)

# ---------------------------------------------------------------------------
# S2: Help->About via the direct menu command. WM_COMMAND with
# wParam = MAKEWPARAM(121, 0) = 121; the wnd_proc dispatches on the low
# word. Poll for the modal MessageBox: class #32770, caption "riviv".
# ---------------------------------------------------------------------------
[void][S124]::PostMessage($script:main1, $WM_COMMAND, [IntPtr]$CMD_HELP_ABOUT, [IntPtr]::Zero)
$script:dlg = [IntPtr]::Zero
$up2 = Wait-Until {
    $h = [S124]::FindWindowW('#32770', 'riviv')
    if ($h -ne [IntPtr]::Zero) {
        $wpid = 0
        [void][S124]::GetWindowThreadProcessId($h, [ref]$wpid)
        if ($wpid -eq $script:Pid1) { $script:dlg = $h; return $true }
    }
    return $false
} 12000
if (-not $up2) {
    $anyDlg = [S124]::FindWindowW('#32770', [NullString]::Value)
    Write-Output ('  S2 diag: anyDlg=' + $anyDlg + ' mainAlive=' + [S124]::IsWindow($script:main1) + ' exited=' + $p1.HasExited)
}
Check 'S2 About box opens on WM_COMMAND 121 (class #32770, caption riviv)' ($up2 -and ($script:dlg -ne [IntPtr]::Zero)) ('found=' + $up2 + ' hwnd=' + $script:dlg)

# ---------------------------------------------------------------------------
# S3: read the message static (dialog item 0xFFFF) and assert the body
# verbatim, ordinally. The backend word is NOT pinned (d2d/hw and d2d/warp
# are both legal); only the "Renderer: " prefix is asserted.
# ---------------------------------------------------------------------------
$static3 = [IntPtr]::Zero
if ($script:dlg -ne [IntPtr]::Zero) { $static3 = [S124]::GetDlgItem($script:dlg, 0xFFFF) }
$script:text3 = ''
$read3 = Wait-Until {
    if ($static3 -eq [IntPtr]::Zero) { return $false }
    $t = [S124]::ReadText($static3)
    if ($t.Length -gt 0) { $script:text3 = $t; return $true }
    return $false
} 8000
Write-Output 'ABOUT TEXT (escaped; U+2014/U+00A9 as markers, \n per line):'
if ($read3) { Show-Escaped $script:text3 } else { Write-Output '  | <unreadable>' }

$rd = 'text unreadable (static=' + $static3 + ' dlg=' + $script:dlg + ' read=' + $read3 + ')'
$hasVersion = $read3 -and $script:text3.StartsWith('riviv 0.1.0', [StringComparison]::Ordinal)
$hasLicense = $read3 -and $script:text3.Contains($WantLicense)
$hasCredit  = $read3 -and $script:text3.Contains($WantCredit)
$hasUp      = $read3 -and $script:text3.Contains('Upstream: https://www.voidtools.com/voidimageviewer/')
$hasSrc     = $read3 -and $script:text3.Contains('Source: https://github.com/jaredshuai/riviv')
$hasRnd     = $read3 -and $script:text3.Contains('Renderer: ')
$noOld      = $read3 -and (-not $script:text3.Contains('Upstream (MIT):'))
$detail3 = $rd
if ($read3) { $detail3 = 'got=[' + (Esc $script:text3) + ']' }
Check 'S3a body opens with the version line "riviv 0.1.0"' $hasVersion $detail3
Check ('S3b license line verbatim [' + (Esc $WantLicense) + ']') $hasLicense $detail3
Check ('S3c credit line verbatim [' + (Esc $WantCredit) + ']') $hasCredit $detail3
Check 'S3d Upstream URL verbatim' $hasUp $detail3
Check 'S3e Source URL verbatim' $hasSrc $detail3
Check 'S3f "Renderer: " prefix present (backend not pinned)' $hasRnd $detail3
Check 'S3g old string "Upstream (MIT):" gone (dedup regression)' $noOld $detail3

# ---------------------------------------------------------------------------
# S4: close the modal, keeping the main window alive. The task's nominal
# recipe is the direct WM_COMMAND IDOK; the empirically verified repo
# recipe (smoke26) is BM_CLICK on the OK button. Try in order, first
# winner wins; the used path is printed for the smoke report.
# ---------------------------------------------------------------------------
$closePath4 = '<none>'
$okBtn4 = [IntPtr]::Zero
if ($script:dlg -ne [IntPtr]::Zero) {
    $okBtn4 = [S124]::GetDlgItem($script:dlg, $IDOK)
    if ($okBtn4 -eq [IntPtr]::Zero) { $okBtn4 = [S124]::FindWindowExW($script:dlg, [IntPtr]::Zero, 'Button', [NullString]::Value) }
}
if (($script:dlg -ne [IntPtr]::Zero) -and ([S124]::IsWindow($script:dlg))) {
    # (a) PostMessage WM_COMMAND IDOK (2.5 s poll).
    [void][S124]::PostMessage($script:dlg, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    if (Wait-Until { [S124]::FindWindowW('#32770', 'riviv') -eq [IntPtr]::Zero } 2500) { $closePath4 = 'PostMessage WM_COMMAND IDOK' }
    # (b) SendMessage WM_COMMAND IDOK (2.5 s poll).
    if ($closePath4 -eq '<none>') {
        if ([S124]::IsWindow($script:dlg)) {
            [void][S124]::SendMsg($script:dlg, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
            if (Wait-Until { [S124]::FindWindowW('#32770', 'riviv') -eq [IntPtr]::Zero } 2500) { $closePath4 = 'SendMessage WM_COMMAND IDOK' }
        }
    }
    # (c) BM_CLICK (0x00F5) to the OK button - smoke26's verified recipe.
    if ($closePath4 -eq '<none>') {
        if (($okBtn4 -ne [IntPtr]::Zero) -and ([S124]::IsWindow($okBtn4))) {
            [void][S124]::SendMsg($okBtn4, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
            if (Wait-Until { [S124]::FindWindowW('#32770', 'riviv') -eq [IntPtr]::Zero } 8000) { $closePath4 = 'BM_CLICK OK button' }
        }
    }
}
$gone4 = ($closePath4 -ne '<none>')
$mainAlive4 = [S124]::IsWindow($script:main1)
Write-Output ('S4 close path: ' + $closePath4)
Check 'S4a About box closes (empirical ladder; path printed above)' $gone4 ('path=' + $closePath4 + ' okBtn=' + $okBtn4)
Check 'S4b main window still alive after About close' $mainAlive4 ('alive=' + $mainAlive4)

# ---------------------------------------------------------------------------
# S5: WM_CLOSE to the main window -> staged process exits 0.
# ---------------------------------------------------------------------------
[void][S124]::PostMessage($script:main1, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
$code5 = -1
if ($p1.WaitForExit(12000)) {
    if ($script:RawH -ne $null -and $script:RawH -ne [IntPtr]::Zero) {
        $c5 = 0
        [void][S124]::GetExitCodeProcess($script:RawH, [ref]$c5)
        $code5 = $c5
    } else { $code5 = -2 }
} else {
    Stop-Process -Id $p1.Id -Force -ErrorAction SilentlyContinue
    $p1.WaitForExit(3000) | Out-Null
}
Check 'S5 WM_CLOSE exits 0' ($code5 -eq 0) ('code=' + $code5)

# ---------------------------------------------------------------------------
# Teardown: only path-filtered staged leftovers are killed; the stage (with
# the WM_CLOSE-written ini inside) is removed on success, kept as evidence
# on failure.
# ---------------------------------------------------------------------------
$leftover = @(Get-Process riviv -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $RunExe })
$leftoverIds = ($leftover | ForEach-Object { $_.Id }) -join ','
if ($leftover) { $leftover | Stop-Process -Force }
Check 'S9 teardown: no staged riviv left' ($leftover.Count -eq 0) ('leftoverPids=[' + $leftoverIds + ']')

if ($script:fail -eq 0) {
    Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
    $stageGone = -not (Test-Path $Stage)
    Check 'S9b stage removed (staged ini went with it)' $stageGone ('stage=' + (Test-Path $Stage))
} else {
    Write-Output ('FAILURES: evidence kept in ' + $Stage)
}
Write-Output ('RESULT: pass=' + $script:pass + ' fail=' + $script:fail + ' skip=' + $script:skip)
Write-Output ('SMOKE124: ' + $script:pass + ' PASS / ' + $script:fail + ' FAIL')
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
