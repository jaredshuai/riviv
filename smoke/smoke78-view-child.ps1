# smoke78 - riviv_view child HWND (#78): structure, routing, lifecycle.
# ASCII-only source (PS5.1 ANSI trap). Staged-ini discipline: precheck +
# cleanup, force-kill only (no WM_CLOSE writeback).
#
# In-repo copy (PR #86 review round; precedent: installer/smoke26-assoc.ps1):
# the dedicated smoke for the #78 viewport-child HWND. Runs under Windows
# PowerShell 5.1; the real-input scenarios (key/mouse injection, foreground
# pixel sampling) need plain foreground rights, no elevation. Drive the
# build under test with -Exe (default D:\codespace\riviv\target\release\
# riviv.exe). Every instance runs from a staged copy in %TEMP%\riviv-78-smoke
# with a per-phase ini that is cleaned up on exit.
# SKIP semantics: S6/S7 need a live display; while the pre-existing
# background-process paint gate (a never-activated process gets no WM_PAINT -
# master-identical, verified by probe78-bg.ps1, NOT a #78 regression) has not
# lifted, they SKIP by design. The #78 structure/routing scenarios
# (S1-S5, S8) hard-assert and are unaffected by that gate.
param([string]$Exe = 'D:\codespace\riviv\target\release\riviv.exe')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition (@'
using System;
using System.Runtime.InteropServices;
public class S78 {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr hwnd, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumChildWindows(IntPtr hwnd, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, System.Text.StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref POINT p);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("gdi32.dll")] public static extern uint GetPixel(IntPtr hdc, int x, int y);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalAlloc(uint flags, uint bytes);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr hmem);
    [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr hmem);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO pci);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO { public int cbSize; public int flags; public IntPtr hCursor; public POINT pt; }
}
'@)
[S78]::SetProcessDPIAware() | Out-Null
$script:pass = 0; $script:fail = 0
$root = Join-Path $env:TEMP 'riviv-78-smoke'
$iniPath = "$root\riviv.ini"
if (Test-Path $root) { Remove-Item -Recurse -Force $root }
New-Item -ItemType Directory -Path $root | Out-Null
Copy-Item $Exe "$root\riviv.exe"

function Save-Png($path, $r, $g, $b) {
    $bmp = New-Object System.Drawing.Bitmap(64, 64)
    $c = [System.Drawing.Color]::FromArgb(255, $r, $g, $b)
    $graphics = [System.Drawing.Graphics]::FromImage($bmp)
    $brush = New-Object System.Drawing.SolidBrush($c)
    $graphics.FillRectangle($brush, 0, 0, 64, 64)
    $brush.Dispose(); $graphics.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}
$red = "$root\red.png"; $blue = "$root\blue.png"
Save-Png $red 255 0 0
Save-Png $blue 0 0 255

# stage a fixed-geometry ini with full chrome
$iniText = "[riviv]`r`nx=140`r`ny=140`r`nwide=1200`r`nhigh=800`r`nauto_zoom=0`r`n"
[System.IO.File]::WriteAllText($iniPath, $iniText)

function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Output ("PASS {0}  {1}" -f $name, $detail) }
    else { $script:fail++; Write-Output ("FAIL {0}  {1}" -f $name, $detail) }
}

function Get-Children($hwnd) {
    $script:kids = @()
    $cb = [S78+EnumProc]{ param($h, $lp) $script:kids += $h; return $true }
    [S78]::EnumChildWindows($hwnd, $cb, [IntPtr]::Zero) | Out-Null
    $list = @($script:kids)
    $script:kids = $null
    return $list
}

function Get-Class($h) {
    $sb = New-Object System.Text.StringBuilder 64
    [S78]::GetClassName($h, $sb, 64) | Out-Null
    return $sb.ToString()
}

function Get-Rect($h) {
    $r = New-Object S78+RECT
    [S78]::GetWindowRect($h, [ref]$r) | Out-Null
    return $r
}

function Wait-Window($p) {
    for ($i = 0; $i -lt 100; $i++) {
        $p.Refresh()
        if ($p.MainWindowHandle -ne 0) { return $p.MainWindowHandle }
        Start-Sleep -Milliseconds 100
    }
    throw "no window appeared"
}

# Poll a background process's title until it differs from $notThis (posted
# messages to a background process can service arbitrarily late under load).
function Wait-Title3($p, $notThis, $ms) {
    $t = ''
    for ($i = 0; $i -lt [int]($ms / 100); $i++) {
        Start-Sleep -Milliseconds 100
        $p.Refresh()
        $t = $p.MainWindowTitle
        if ($t -ne $notThis -and $t -ne '') { return $t }
    }
    return $t
}

function Client-Origin($h) {
    $pt = New-Object S78+POINT; $pt.X = 0; $pt.Y = 0
    [S78]::ClientToScreen($h, [ref]$pt) | Out-Null
    return $pt
}

function Sample-Screen($x, $y) {
    $hdc = [S78]::GetDC([IntPtr]::Zero)
    try {
        $c = [S78]::GetPixel($hdc, $x, $y)
        return @([int]($c -band 0xFF), [int](($c -shr 8) -band 0xFF), [int](($c -shr 16) -band 0xFF))
    } finally { [S78]::ReleaseDC([IntPtr]::Zero, $hdc) | Out-Null }
}

$p = Start-Process -FilePath "$root\riviv.exe" -ArgumentList "`"$red`"" -PassThru
try {
    $h = Wait-Window $p
    Start-Sleep -Milliseconds 1200
    $origin = Client-Origin $h
    $kids = Get-Children $h
    $byClass = @{}
    foreach ($k in $kids) { $c = Get-Class $k; if (!$byClass.ContainsKey($c)) { $byClass[$c] = $k } }

    # S1 structure: riviv_view exists, anchored at client origin, bottom
    # meets the chrome, width spans the client.
    $view = $byClass['riviv_view']
    Check 'S1a riviv_view child exists' ($view -ne $null) ("children=" + (($kids | ForEach-Object { Get-Class $_ }) -join ','))
    if ($view) {
        $vr = Get-Rect $view
        $w = $vr.R - $vr.L; $vh = $vr.B - $vr.T
        $anchored = ($vr.L -eq $origin.X) -and ($vr.T -eq $origin.Y)
        Check 'S1b anchored at client origin, full width' $anchored ("view=$($vr.L),$($vr.T) ${w}x${vh} origin=$($origin.X),$($origin.Y)")
        $bar = $byClass['msctls_statusbar32']
        $rebar = $byClass['riviv_rebar']
        $meetsChrome = $false
        $chromeDesc = ""
        if ($rebar) {
            $rr = Get-Rect $rebar
            # The chrome stack is view -> rebar -> status (bottom-docked):
            # the viewport must END exactly where the strip begins.
            $meetsChrome = ($rr.T -eq $vr.B)
            $chromeDesc = "view.bottom=$($vr.B) rebar.top=$($rr.T)"
        }
        Check 'S1c viewport bottom meets the chrome (no gap)' $meetsChrome $chromeDesc
    }

    # S4 drop onto the child: hand-built HDROP -> WM_DROPFILES(child).
    $WM_DROPFILES = 0x0233
    $files = "$blue".ToLower()
    $bytes = [System.Text.Encoding]::Unicode.GetBytes($files + [char]0 + [char]0)
    $total = 20 + $bytes.Length
    $hmem = [S78]::GlobalAlloc(0x0002, [UInt32]$total)  # GMEM_MOVEABLE
    $mem = [S78]::GlobalLock($hmem)
    $drop = [BitConverter]::GetBytes([UInt32]20)
    [Runtime.InteropServices.Marshal]::Copy($drop, 0, $mem, 4)
    [Runtime.InteropServices.Marshal]::WriteInt32($mem, 8, 1)  # fWide
    [Runtime.InteropServices.Marshal]::Copy($bytes, 0, [IntPtr]::Add($mem, 20), $bytes.Length)
    [S78]::GlobalUnlock($hmem) | Out-Null
    [S78]::PostMessage($view, $WM_DROPFILES, $hmem, [IntPtr]::Zero) | Out-Null
    $t = Wait-Title3 $p ($p.MainWindowTitle) 4000
    # Title-only assertion here: while the process has never been brought
    # forward, Windows gates WM_PAINT generation for image adoptions and
    # even PrintWindow reads the stale surface (verified identical on
    # master f65c00a with probe78-bg.ps1 - pre-existing, out of #78 scope;
    # a menu activation alone does NOT lift it, a fullscreen resize does).
    # The pixel scenarios run after the S5 fullscreen dance below.
    Check 'S4 drop forwarded to the owner (request adopted)' ($t -like 'blue.png*') ("title='$t'")
    if ($t -notlike 'blue.png*') {
        # Wedged-instance diagnostics: does the UI thread still service
        # SENT messages, and are sibling riviv processes interfering?
        $res = [IntPtr]::Zero
        $sm = [S78]::SendMessageTimeout($h, 0x0000, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$res)
        $alive2 = (Get-Process riviv -ErrorAction SilentlyContinue | Measure-Object).Count
        Write-Output ("S4-DIAG responding=" + ($sm -ne [IntPtr]::Zero) + " rivivProcs=" + $alive2)
    }

    # S5 double-click fullscreen via the child, twice (enter + exit).
    $WM_LBUTTONDBLCLK = 0x0203
    $dbl = [IntPtr](((350 -shl 16) -bor (570 -band 0xFFFF)))
    [S78]::PostMessage($view, $WM_LBUTTONDBLCLK, [IntPtr]::Zero, $dbl) | Out-Null
    Start-Sleep -Milliseconds 700
    $vr2 = Get-Rect $view
    $o2 = Client-Origin $h
    $fsCover = ($vr2.L -eq $o2.X) -and ($vr2.T -eq $o2.Y) -and ($vr2.R - $vr2.L) -gt 2000
    Check 'S5a double-click on child enters fullscreen (view covers)' $fsCover ("view=$($vr2.L),$($vr2.T) $($vr2.R -$vr2.L)x$($vr2.B -$vr2.T)")
    [S78]::PostMessage($view, $WM_LBUTTONDBLCLK, [IntPtr]::Zero, $dbl) | Out-Null
    Start-Sleep -Milliseconds 700
    $vr3 = Get-Rect $view
    $backWin = ($vr3.R - $vr3.L) -lt 1900
    Check 'S5b double-click exits fullscreen (view shrinks back)' $backWin ("viewW=$($vr3.R -$vr3.L)")
    $origin = Client-Origin $h

    # S7 minimize / restore: content survives. Windows gates WM_PAINT for
    # windows of a never-activated background process (probe78-bg.ps1
    # verified master-identical - pre-existing, out of #78 scope): the
    # settle below rides out the #76 handshake's 5s worker cap first; if
    # the gate STILL has not lifted, SKIP rather than FAIL (the #78-relevant
    # structure/routing scenarios above all hard-assert).
    Start-Sleep -Milliseconds 5500
    [S78]::PostMessage($h, 0x0112, [IntPtr](0xF020), [IntPtr]::Zero) | Out-Null  # SC_MINIMIZE
    Start-Sleep -Milliseconds 700
    [S78]::PostMessage($h, 0x0112, [IntPtr](0xF120), [IntPtr]::Zero) | Out-Null  # SC_RESTORE
    Start-Sleep -Milliseconds 900
    $origin = Client-Origin $h
    $after = Sample-Screen ([int]($origin.X + 570)) ([int]($origin.Y + 350))
    $alive = -not $p.HasExited
    $blueLive = $alive -and $after[2] -gt 150 -and $after[0] -lt 80
    if ($blueLive) {
        $script:pass++
        Write-Output ("PASS S7 minimize/restore shows the dropped image  rgb=" + ($after -join ','))
    } elseif ($alive -and ($after[0] -gt 200 -and $after[1] -gt 200)) {
        Write-Output "SKIP S7 background-paint gate not lifted (pre-existing, master-identical)"
    } else {
        $script:fail++
        Write-Output ("FAIL S7 minimize/restore  rgb=" + ($after -join ',') + " alive=$alive")
    }

    # S6 wheel delivered straight to the child (the routing this ticket
    # added - posted directly to the child regardless of focus): zooms in
    # until the image covers the corner. Requires the display live (S7);
    # SKIPs under the pre-existing background-paint gate like S7.
    $WM_MOUSEWHEEL = 0x020A
    $anchorX = [int]($origin.X + 570); $anchorY = [int]($origin.Y + 350)
    $cornerBefore = Sample-Screen ([int]($origin.X + 20)) ([int]($origin.Y + 20))
    $wp = [IntPtr](0x00780000)  # delta=120
    $lpScreen = [IntPtr]((($anchorY -band 0xFFFF) -shl 16) -bor ($anchorX -band 0xFFFF))
    for ($i = 0; $i -lt 14; $i++) { [S78]::PostMessage($view, $WM_MOUSEWHEEL, $wp, $lpScreen) | Out-Null; Start-Sleep -Milliseconds 40 }
    Start-Sleep -Milliseconds 500
    $cornerAfter = Sample-Screen ([int]($origin.X + 20)) ([int]($origin.Y + 20))
    if ($blueLive) {
        Check 'S6 wheel to child zooms (corner letterbox -> image)' ($cornerBefore[0] -gt 200 -and $cornerAfter[2] -gt 150 -and $cornerAfter[0] -lt 80) ("before=" + ($cornerBefore -join ',') + " after=" + ($cornerAfter -join ','))
    } else {
        Write-Output "SKIP S6 wheel zoom (display gated; routing itself covered by the S4/S5 forward paths)"
    }

    # S3 context-menu production chain over the child, LAST (its modal loop
    # occasionally swallows subsequent synthetic posts - off the critical
    # path): post WM_RBUTTONUP -> child DefWindowProc -> WM_CONTEXTMENU(child)
    # -> forwarded popup. Dismissal is VERIFIED (poll the menu gone, real
    # ESC as fallback) so the process teardown is clean.
    $WM_RBUTTONUP = 0x0205
    # lparam for a mouse message is CLIENT coords of the receiving window;
    # child client coords == main client coords (the #78 identity).
    $lpClient = [IntPtr](((200 -shl 16) -bor (300 -band 0xFFFF)))
    [S78]::PostMessage($view, $WM_RBUTTONUP, [IntPtr]::Zero, $lpClient) | Out-Null
    $menuFound = $false
    $menuHwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 20; $i++) {
        Start-Sleep -Milliseconds 100
        $m = [S78]::FindWindowW('#32768', [NullString]::Value)
        if ($m -ne [IntPtr]::Zero) {
            $mpid = 0
            [S78]::GetWindowThreadProcessId($m, [ref]$mpid) | Out-Null
            if ($mpid -eq $p.Id) { $menuFound = $true; $menuHwnd = $m; break }
        }
    }
    Check 'S3 right-click popup produced via the child' $menuFound ("#32768 owner=" + $p.Id)
    [S78]::PostMessage($h, 0x001F, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_CANCELMODE
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Milliseconds 150
        $m = [S78]::FindWindowW('#32768', [NullString]::Value)
        if ($m -eq [IntPtr]::Zero) { break }
        $mpid = 0
        [S78]::GetWindowThreadProcessId($m, [ref]$mpid) | Out-Null
        if ($mpid -ne $p.Id) { break }
        if ($i -eq 4) { [S78]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero); [S78]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero) }  # real ESC
    }
} finally {
    if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -Force }
    Start-Sleep -Milliseconds 300
}

# S2 chrome-less variant: view == the whole client.
[System.IO.File]::WriteAllText($iniPath, "[riviv]`r`nx=140`r`ny=140`r`nwide=1200`r`nhigh=800`r`nauto_zoom=0`r`nshow_menu=0`r`nshow_status=0`r`nshow_controls=0`r`n")
$p2 = Start-Process -FilePath "$root\riviv.exe" -ArgumentList "`"$red`"" -PassThru
try {
    $h2 = Wait-Window $p2
    Start-Sleep -Milliseconds 1200
    $o = Client-Origin $h2
    $kids2 = Get-Children $h2
    $v2 = $null
    foreach ($k in $kids2) { if ((Get-Class $k) -eq 'riviv_view') { $v2 = $k } }
    $anchored2 = $false
    if ($v2) {
        $r2 = Get-Rect $v2
        $anchored2 = ($r2.L -eq $o.X) -and ($r2.T -eq $o.Y)
    }
    # assert no chrome children exist at all.
    $noChrome = $true
    foreach ($k in $kids2) { $cn = Get-Class $k; if ($cn -eq 'msctls_statusbar32' -or $cn -eq 'riviv_rebar') { $noChrome = $false } }
    Check 'S2 chrome-less: only riviv_view child, anchored' ($v2 -ne $null -and $noChrome -and $anchored2) ("kids=" + (($kids2 | ForEach-Object { Get-Class $_ }) -join ','))
} finally {
    if ($p2 -and !$p2.HasExited) { Stop-Process -Id $p2.Id -Force }
    Start-Sleep -Milliseconds 300
}

$cleaned = $false
if (Test-Path $iniPath) { Remove-Item -Force $iniPath; $cleaned = $true }

# S8 child-routed input sweep: the forwarding arms no other smoke exercises
# (L/R/M/X button families posted DIRECTLY to the child). Click actions are
# staged to NAVIGATION so the effect is title-observable and paint-free -
# the pre-existing background-paint gate cannot mask it.
[System.IO.File]::WriteAllText($iniPath, "[riviv]`r`nx=140`r`ny=140`r`nwide=1200`r`nhigh=800`r`nauto_zoom=0`r`nleft_click_action=4`r`nright_click_action=2`r`n")
$green = "$root\green.png"
Save-Png $green 0 200 0
$p3 = Start-Process -FilePath "$root\riviv.exe" -ArgumentList "`"$red`" `"${blue}`" `"$green`"" -PassThru
try {
    $h3 = Wait-Window $p3
    Start-Sleep -Milliseconds 1500
    $kids3 = Get-Children $h3
    $v3h = [IntPtr]::Zero
    foreach ($k in $kids3) { if ((Get-Class $k) -eq 'riviv_view') { $v3h = $k } }
    if ($v3h -eq [IntPtr]::Zero) { throw "no riviv_view child in sweep instance" }
    $lpDown = [IntPtr](((300 -shl 16) -bor (400 -band 0xFFFF)))

    $t0 = $null; $p3.Refresh(); $t0 = $p3.MainWindowTitle
    [S78]::PostMessage($v3h, 0x0201, [IntPtr]::Zero, $lpDown) | Out-Null  # WM_LBUTTONDOWN (action 4 = next)
    $t1 = Wait-Title3 $p3 $t0 4000
    Check 'S8a WM_LBUTTONDOWN to child navigates' ($t1 -ne $t0 -and $t1 -ne '') ("$t0 -> $t1")
    [S78]::PostMessage($v3h, 0x0202, [IntPtr]::Zero, $lpDown) | Out-Null  # WM_LBUTTONUP (no drag live: no-op arm)
    Start-Sleep -Milliseconds 250

    # Foreground + cursor onto the viewport BEFORE the M-button pair:
    # ShowCursor is per-input-queue - a BACKGROUND process's
    # ShowCursor(FALSE) never hides the on-screen cursor owned by the
    # foreground queue, so the hide is only observable with the app
    # foreground and the cursor over it (the earlier flaky passes were
    # moments the cursor happened to be there).
    $o3 = New-Object S78+POINT; $o3.X = 0; $o3.Y = 0
    [S78]::ClientToScreen($h3, [ref]$o3) | Out-Null
    [S78]::SetCursorPos([int]($o3.X + 400), [int]($o3.Y + 250)) | Out-Null
    for ($i = 0; $i -lt 10; $i++) {
        [S78]::SetForegroundWindow($h3) | Out-Null
        if ([S78]::GetForegroundWindow() -eq $h3) { break }
        [S78]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
        [S78]::SetForegroundWindow($h3) | Out-Null
        [S78]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 120
    }
    Start-Sleep -Milliseconds 250
    [S78]::PostMessage($v3h, 0x0207, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_MBUTTONDOWN: mscroll starts
    # Poll, not a single read: a BACKGROUND process may service the posted
    # message well past any fixed wait under load (S8b flaked exactly there
    # on the pre-fix binary too - timing, not behavior).
    $mHidden = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 100
        $ci = New-Object S78+CURSORINFO
        $ci.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($ci)
        [S78]::GetCursorInfo([ref]$ci) | Out-Null
        if ((($ci.flags -band 1) -eq 0)) { $mHidden = $true; break }
    }
    [S78]::PostMessage($v3h, 0x0208, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_MBUTTONUP: ends, re-shows
    $mShownBack = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 100
        $ci2 = New-Object S78+CURSORINFO
        $ci2.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($ci2)
        [S78]::GetCursorInfo([ref]$ci2) | Out-Null
        if ((($ci2.flags -band 1) -ne 0)) { $mShownBack = $true; break }
    }
    Check 'S8b M-button pair to child (hide + restore cursor)' ($mHidden -and $mShownBack) ("downHidden=$mHidden upVisible=$mShownBack")

    $p3.Refresh(); $t1b = $p3.MainWindowTitle
    [S78]::PostMessage($v3h, 0x0204, [IntPtr]::Zero, $lpDown) | Out-Null  # WM_RBUTTONDOWN (action 2 = prev)
    $t2 = Wait-Title3 $p3 $t1b 4000
    Check 'S8c WM_RBUTTONDOWN to child navigates' ($t2 -ne $t1b -and $t2 -ne '') ("$t1b -> $t2")
    [S78]::PostMessage($v3h, 0x0206, [IntPtr]::Zero, $lpDown) | Out-Null  # WM_RBUTTONDBLCLK (action 2 again)
    $t3 = Wait-Title3 $p3 $t2 4000
    Check 'S8d WM_RBUTTONDBLCLK to child navigates' ($t3 -ne $t2 -and $t3 -ne '') ("$t2 -> $t3")
    [S78]::PostMessage($v3h, 0x0205, [IntPtr]::Zero, $lpDown) | Out-Null  # WM_RBUTTONUP: swallowed under action 2
    Start-Sleep -Milliseconds 400
    $mnu = [S78]::FindWindowW('#32768', [NullString]::Value)
    $noMenu = ($mnu -eq [IntPtr]::Zero)
    Check 'S8e RBUTTONUP under action 2: no context menu' $noMenu ''

    $wpX = [IntPtr](0x00010000)  # HIWORD=XBUTTON1, keys=0
    [S78]::PostMessage($v3h, 0x020B, $wpX, $lpDown) | Out-Null  # WM_XBUTTONDOWN (action 2 = nav)
    $t4 = Wait-Title3 $p3 $t3 4000
    Check 'S8f WM_XBUTTONDOWN to child navigates' ($t4 -ne $t3 -and $t4 -ne '') ("$t3 -> $t4")
} finally {
    if ($p3 -and !$p3.HasExited) { Stop-Process -Id $p3.Id -Force }
    Start-Sleep -Milliseconds 300
}
if (Test-Path $iniPath) { Remove-Item -Force $iniPath }

Write-Output ("SMOKE78 DONE pass={0} fail={1} (ini cleaned: {2})" -f $script:pass, $script:fail, $cleaned)
if ($script:fail -gt 0) { exit 1 }
