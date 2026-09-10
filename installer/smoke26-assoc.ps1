# smoke26-assoc.ps1 - issue #26 file associations + install CLI smoke.
# Runs UN-ELEVATED: elevation-demanding paths are driven through /isrunas
# (the exact code path the elevated child runs, minus the UAC prompt); the
# truly elevation-only scenarios (start-menu folder under CommonPrograms,
# NSIS full install/uninstall) are SKIPped for manual QA.
# Machine-state discipline: the whole body runs in try/finally; the finally
# restores the touched associations (png/ico/bmp) and the %APPDATA%\riviv
# directory from a pre-run snapshot, so a pre-existing riviv install
# survives the run (cubic round 1).
# ASCII-only source (PS 5.1 GBK pitfall, #20/#25).
param(
    [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\riviv.exe")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type '
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
}';
[W]::SetProcessDPIAware() | Out-Null

$script:results = @()
function Pass($n) { $script:results += "PASS $n"; Write-Host "PASS $n" -ForegroundColor Green }
function Fail($n, $why) { $script:results += "FAIL $n : $why"; Write-Host "FAIL $n : $why" -ForegroundColor Red }
function Skip($n, $why) { $script:results += "SKIP $n : $why"; Write-Host "SKIP $n : $why" -ForegroundColor Yellow }
function Check($n, $cond, $why) { if ($cond) { Pass $n } else { Fail $n $why } }

function Get-RegDefault($key) {
    $v = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).'(default)'
    return [string]$v
}
function Wait-Window($procId, [int]$timeoutMs) {
    $deadline = [Environment]::TickCount + $timeoutMs
    while ([Environment]::TickCount -lt $deadline) {
        $script:found = [IntPtr]::Zero
        $cb = {
            param($h, $l)
            $pid2 = 0
            [W]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
            if ($pid2 -eq $procId) {
                $sb = New-Object System.Text.StringBuilder 256
                [W]::GetClassNameW($h, $sb, 256) | Out-Null
                if ($sb.ToString() -eq 'riviv') { $script:found = $h; return $false }
            }
            return $true
        }
        [W]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
        if ($script:found -ne [IntPtr]::Zero) { return $script:found }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Wait-Title($hwnd, [string]$contains, [int]$timeoutMs) {
    $deadline = [Environment]::TickCount + $timeoutMs
    while ([Environment]::TickCount -lt $deadline) {
        $sb = New-Object System.Text.StringBuilder 256
        [W]::GetWindowTextW($hwnd, $sb, 256) | Out-Null
        if ($sb.ToString().Contains($contains)) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

# ---- setup: isolated exe dir + dummy Uninstall.exe + a test png ----
$dir = Join-Path $env:TEMP "riviv-test\s26"
if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
New-Item -ItemType Directory -Path $dir -Force | Out-Null
$exeCopy = Join-Path $dir "riviv.exe"
Copy-Item $Exe $exeCopy
# a stand-in Uninstall.exe (NSIS's WriteUninstaller product) to verify the copy
Copy-Item $Exe (Join-Path $dir "Uninstall.exe")
$png = Join-Path $dir "s7test.png"
$bmp = New-Object System.Drawing.Bitmap 32, 24
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear([System.Drawing.Color]::FromArgb(255, 30, 120, 200))
$g.Dispose()
$bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

# ---- machine-state snapshot: only what this run touches ----
$appdataRiviv = Join-Path $env:APPDATA "riviv"
$appdataExisted = Test-Path $appdataRiviv
$appdataBackup = Join-Path $env:TEMP "riviv-test\s26-appdata-backup"
if ($appdataExisted) { Move-Item $appdataRiviv $appdataBackup }
# extensions this script installs (S1/S4/S5/S6); the restore step only
# uninstalls those that were NOT riviv-owned before the run.
$touched = @('png', 'ico', 'bmp')
$ownedBefore = @{}
foreach ($ext in $touched) {
    $ownedBefore[$ext] = ((Get-RegDefault ('HKCU:\SOFTWARE\Classes\.' + $ext)) -eq ('riviv.' + $ext))
}
$pngBefore = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.png'

# kill stray riviv from earlier scenarios (mutex discipline)
Get-Process riviv -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300

try {

Write-Host "== S1: CLI association round trip (/png, /nopng)"
& $exeCopy /png
Start-Sleep -Milliseconds 700
$dot = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.png'
Check "S1a /png sets .png default to progid" ($dot -eq 'riviv.png') "got [$dot]"
$cmdKey = Get-RegDefault 'HKCU:\SOFTWARE\Classes\riviv.png\shell\open\command'
Check "S1b open command quotes the exe" ($cmdKey -eq ('"{0}" "%1"' -f $exeCopy)) "got [$cmdKey]"
$iconKey = Get-RegDefault 'HKCU:\SOFTWARE\Classes\riviv.png\DefaultIcon'
Check "S1c DefaultIcon is exe,0" ($iconKey -eq ("{0},0" -f $exeCopy)) "got [$iconKey]"
$descKey = Get-RegDefault 'HKCU:\SOFTWARE\Classes\riviv.png'
Check "S1d progid description" ($descKey -eq 'PNG Image') "got [$descKey]"
$backup = (Get-ItemProperty 'HKCU:\SOFTWARE\Classes\.png' -ErrorAction SilentlyContinue).PSObject.Properties.Name -contains 'riviv.Backup'
Check "S1e Backup value exists" $backup "riviv.Backup missing"
# .ico uses %1 as its own icon
& $exeCopy /ico
Start-Sleep -Milliseconds 700
$icoIcon = Get-RegDefault 'HKCU:\SOFTWARE\Classes\riviv.ico\DefaultIcon'
Check "S1f ico DefaultIcon is %1" ($icoIcon -eq '%1') "got [$icoIcon]"
& $exeCopy /noico
& $exeCopy /nopng
Start-Sleep -Milliseconds 700
$dot2 = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.png'
Check "S1g /nopng restores the original default" ($dot2 -eq $pngBefore) "got [$dot2] (before=[$pngBefore])"
$backupGone = -not ((Get-ItemProperty 'HKCU:\SOFTWARE\Classes\.png' -ErrorAction SilentlyContinue).PSObject.Properties.Name -contains 'riviv.Backup')
Check "S1h Backup value removed" $backupGone "riviv.Backup still present"
Check "S1i progid tree removed (RegDeleteTree deviation)" (-not (Test-Path 'HKCU:\SOFTWARE\Classes\riviv.png')) "progid key still exists"
Check "S1j ico progid removed" (-not (Test-Path 'HKCU:\SOFTWARE\Classes\riviv.ico')) "ico progid key still exists"

Write-Host "== S2: /appdata flip via /isrunas (the elevated child's path)"
& $exeCopy /appdata /isrunas
Start-Sleep -Milliseconds 700
$exeIni = Join-Path $dir "riviv.ini"
$appdataIni = Join-Path $env:APPDATA "riviv\riviv.ini"
Check "S2a appdata ini written" (Test-Path $appdataIni) "no $appdataIni"
Check "S2b exe-dir marker written" ((Test-Path $exeIni) -and ((Get-Content $exeIni -Raw -ErrorAction SilentlyContinue) -match '(?m)^appdata\s*=\s*1\s*$')) "exe ini missing appdata=1 marker"
& $exeCopy /noappdata /isrunas
Start-Sleep -Milliseconds 700
# Upstream /noappdata = config_save_settings(0): the exe-dir ini becomes the
# FULL table whose keys do NOT include appdata (config.c:283-289 writes the
# marker only in the is_root&&appdata branch); the dormant appdata file is
# left in place (only /uninstall deletes it, viv.c:4720-4726).
$exeRaw = Get-Content $exeIni -Raw -ErrorAction SilentlyContinue
Check "S2c exe-dir ini is the full table again" ($exeRaw -match '(?m)^x\s*=' -and $exeRaw -notmatch '(?m)^appdata\s*=') "exe ini is not the full table"
Check "S2d no appdata line remains (absent = 0, upstream semantics)" ($exeRaw -notmatch '(?m)^appdata\s*=\s*1\s*$') "appdata=1 still present"

Write-Host "== S3: /install copy via /isrunas"
$target = Join-Path $dir "install-target"
& $exeCopy /install $target /isrunas
Start-Sleep -Milliseconds 900
Check "S3a target dir created" (Test-Path $target) "no target dir"
Check "S3b riviv.exe copied" (Test-Path (Join-Path $target 'riviv.exe')) "exe not copied"
Check "S3c Uninstall.exe copied" (Test-Path (Join-Path $target 'Uninstall.exe')) "Uninstall.exe not copied"

Write-Host "== S4: Options checkbox wiring (BMP check -> OK -> registry -> reread)"
# Normalize a pre-existing riviv-owned .bmp so the click flow below starts
# from a known unchecked state (the pre-state is re-installed in finally).
if ($ownedBefore['bmp']) { & $exeCopy /nobmp; Start-Sleep -Milliseconds 700 }
$p = Start-Process -FilePath $exeCopy -PassThru
$hwnd = Wait-Window $p.Id 10000
Check "S4a app window appears" ($hwnd -ne [IntPtr]::Zero) "no riviv window"
if ($hwnd -ne [IntPtr]::Zero) {
    [W]::PostMessageW($hwnd, 0x0100, [IntPtr]0x4F, [IntPtr]::Zero) | Out-Null   # WM_KEYDOWN 'O' -> Options
    Start-Sleep -Milliseconds 900
    $script:dlg = [IntPtr]::Zero
    $cb2 = {
        param($h, $l)
        $pid2 = 0
        [W]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
        if ($pid2 -eq $script:p.Id) {
            $sb = New-Object System.Text.StringBuilder 256
            [W]::GetClassNameW($h, $sb, 256) | Out-Null
            if ($sb.ToString() -eq 'riviv_options') { $script:dlg = $h; return $false }
        }
        return $true
    }
    [W]::EnumWindows($cb2, [IntPtr]::Zero) | Out-Null
    Check "S4b options dialog opens" ($script:dlg -ne [IntPtr]::Zero) "no riviv_options window"
    if ($script:dlg -ne [IntPtr]::Zero) {
        $page0 = [W]::GetDlgItem($script:dlg, 5)
        $bmpChk = [W]::GetDlgItem($page0, 271)
        Check "S4c BMP checkbox exists" ($bmpChk -ne [IntPtr]::Zero) "ctrl 271 missing"
        $chk0 = [W]::SendMessage($bmpChk, 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero)
        Check "S4d unchecked after normalize" ($chk0 -eq [IntPtr]::Zero) "initial state = $chk0"
        [W]::SendMessage($bmpChk, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # BM_CLICK
        Start-Sleep -Milliseconds 200
        $ok = [W]::GetDlgItem($script:dlg, 1)
        [W]::SendMessage($ok, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null       # BM_CLICK OK
        Start-Sleep -Milliseconds 900
        $bmpDot = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.bmp'
        Check "S4e OK installed the association" ($bmpDot -eq 'riviv.bmp') "got [$bmpDot]"
        # reopen: checkbox must now read as checked from the registry
        [W]::PostMessageW($hwnd, 0x0100, [IntPtr]0x4F, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 900
        [W]::EnumWindows($cb2, [IntPtr]::Zero) | Out-Null
        if ($script:dlg -ne [IntPtr]::Zero) {
            $page0 = [W]::GetDlgItem($script:dlg, 5)
            $bmpChk = [W]::GetDlgItem($page0, 271)
            $chk1 = [W]::SendMessage($bmpChk, 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero)
            Check "S4f reopen reads checked state" ($chk1 -eq [IntPtr]1) "reopen state = $chk1"
            # Check None button clears all nine
            $noneBtn = [W]::GetDlgItem($page0, 281)
            [W]::SendMessage($noneBtn, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 200
            $chk2 = [W]::SendMessage($bmpChk, 0x00F0, [IntPtr]::Zero, [IntPtr]::Zero)
            Check "S4g Check None clears" ($chk2 -eq [IntPtr]::Zero) "after none = $chk2"
            $ok = [W]::GetDlgItem($script:dlg, 1)
            [W]::SendMessage($ok, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 900
            $bmpDot2 = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.bmp'
            Check "S4h uncheck -> OK uninstalls" ($bmpDot2 -ne 'riviv.bmp') "still [$bmpDot2]"
        } else { Fail "S4f" "options dialog did not reopen" }
    }
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

Write-Host "== S5: associated-file launch (the Explorer double-click path)"
# Windows 8+ resolves double-clicks through FileExts\<ext>\UserChoice (hash
# protected); the Classes-default association only applies where NO
# UserChoice exists. Probe for one extension without a UserChoice first;
# with all nine claimed, the launch path cannot be driven here without
# faking the Explorer hash (same limitation upstream voidImageViewer has).
$freeExt = $null
foreach ($ext in @('png', 'bmp')) {   # System.Drawing can write both
    $uc = Test-Path ('HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.' + $ext + '\UserChoice')
    if (-not $uc) { $freeExt = $ext; break }
}
if ($freeExt) {
    & $exeCopy ('/' + $freeExt)
    Start-Sleep -Milliseconds 700
    # a test file OF the free extension, so the ShellExecute resolution
    # actually goes through it
    $img = New-Object System.Drawing.Bitmap 32, 24
    $g2 = [System.Drawing.Graphics]::FromImage($img)
    $g2.Clear([System.Drawing.Color]::FromArgb(255, 30, 120, 200))
    $g2.Dispose()
    $freeFile = Join-Path $dir ("s7test." + $freeExt)
    $fmt = if ($freeExt -eq 'png') { [System.Drawing.Imaging.ImageFormat]::Png } else { [System.Drawing.Imaging.ImageFormat]::Bmp }
    $img.Save($freeFile, $fmt)
    $img.Dispose()
    Invoke-Item $freeFile
    $launched = $null
    $deadline = [Environment]::TickCount + 10000
    while ([Environment]::TickCount -lt $deadline -and -not $launched) {
        $procs = @(Get-Process riviv -ErrorAction SilentlyContinue)
        if ($procs.Count -gt 0) { $launched = $procs | Sort-Object StartTime -ErrorAction SilentlyContinue | Select-Object -Last 1 }
        if (-not $launched) { Start-Sleep -Milliseconds 150 }
    }
    if ($launched) {
        $h2 = Wait-Window $launched.Id 8000
        $want = ('s7test.' + $freeExt + ' - riviv')
        if ($h2 -ne [IntPtr]::Zero -and (Wait-Title $h2 $want 5000)) {
            Pass "S5a associated file opens in riviv (title match, free ext $freeExt)"
        } else {
            Fail "S5a" "window/title not found (hwnd=$h2, wanted [$want])"
        }
        Stop-Process -Id $launched.Id -Force -ErrorAction SilentlyContinue
    } else {
        Fail "S5a" "no new riviv process after Invoke-Item"
    }
    Start-Sleep -Milliseconds 400
} else {
    Skip "S5a associated file opens in riviv" "all extensions carry an Explorer UserChoice on this machine - Classes-default launch needs a clean ext or manual 'open with'; QA checklist"
}

Write-Host "== S6: /uninstall via /isrunas cleans everything"
& $exeCopy /png
Start-Sleep -Milliseconds 700
& $exeCopy /uninstall $target /isrunas
Start-Sleep -Milliseconds 1200
Check "S6a target files removed" (-not (Test-Path (Join-Path $target 'riviv.exe'))) "target exe still present"
Check "S6b target dir removed" (-not (Test-Path $target)) "target dir still present"
# The is_runas gate skips the STANDARD-USER association pass (viv.c:4624) —
# the association leg of /uninstall is the parent's job (it re-execs
# elevated); assert the gate instead: the .png association survives an
# isrunas-only uninstall.
$pngAfter = Get-RegDefault 'HKCU:\SOFTWARE\Classes\.png'
Check "S6c isrunas skips association uninstall (upstream gate)" ($pngAfter -eq 'riviv.png') "got [$pngAfter]"
Check "S6d progid tree intact under the gate" (Test-Path 'HKCU:\SOFTWARE\Classes\riviv.png') "progid key gone"

Write-Host "== S7: start-menu shortcuts — needs elevation (CommonPrograms)"
Skip "S7a /startmenu + /nostartmenu" "CSIDL_COMMON_PROGRAMS write needs UAC; unattended run would hang on the prompt - manual QA"

Write-Host "== S8: NSIS installer — build verified, full flow needs elevation"
$zh = @(Get-ChildItem (Join-Path $PSScriptRoot "nsis") -Filter 'riviv-*.zh-CN-Setup.exe' -ErrorAction SilentlyContinue)
$en = @(Get-ChildItem (Join-Path $PSScriptRoot "nsis") -Filter 'riviv-*.en-US-Setup.exe' -ErrorAction SilentlyContinue)
Check "S8a both language setups built" ($zh.Count -ge 1 -and $en.Count -ge 1) "zh=$($zh.Count) en=$($en.Count)"
Skip "S8b silent install + uninstall" "NSIS /install elevates via UAC - manual QA"

} finally {
    # ---- restore machine state (runs even on a terminating error) ----
    foreach ($ext in $touched) {
        $now = ((Get-RegDefault ('HKCU:\SOFTWARE\Classes\.' + $ext)) -eq ('riviv.' + $ext))
        if ($now -and -not $ownedBefore[$ext]) {
            & $exeCopy ('/no' + $ext) 2>$null
        } elseif ($ownedBefore[$ext] -and -not $now) {
            & $exeCopy ('/' + $ext) 2>$null
        }
    }
    Start-Sleep -Milliseconds 800
    Get-Process riviv -ErrorAction SilentlyContinue | Stop-Process -Force
    if ($appdataExisted) {
        if (Test-Path $appdataRiviv) { Remove-Item $appdataRiviv -Recurse -Force }
        Move-Item $appdataBackup $appdataRiviv
    } elseif (Test-Path $appdataRiviv) {
        Remove-Item $appdataRiviv -Recurse -Force
    }
    Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "`n==== summary ===="
$script:results
$fails = @($script:results | Where-Object { $_ -like 'FAIL*' })
$skips = @($script:results | Where-Object { $_ -like 'SKIP*' })
Write-Host ("total={0} pass={1} fail={2} skip={3}" -f $script:results.Count, ($script:results.Count - $fails.Count - $skips.Count), $fails.Count, $skips.Count)
if ($fails.Count -gt 0) { exit 1 }
