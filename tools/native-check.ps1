# Native GUI check for the desktop build.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\native-check.ps1 [-Size 8] [-Seed 42]
#
# It launches the game with a fixed seed, finds the clue cells by looking at the
# board it actually drew, then posts real mouse messages and compares screenshots
# of the grid area:
#
#   * a plain left click paints exactly one cell,
#   * Alt + click steps the clue under the cursor (the path a mouse without a
#     middle button uses),
#   * Alt + click on a cell that is not a clue changes nothing on the board.
#
# Needs a desktop session: CopyFromScreen reads the real screen.

param(
    [int]$Size = 8,
    [string]$Seed = "42"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$exe = Join-Path $root "target\release\tapa.exe"
if (-not (Test-Path $exe)) { throw "build it first: cargo build --release" }

$settings = @("--size", "$Size", "--seed", "$Seed", "--set", "density=0.40-0.50")
# Keep the game's own remembered settings out of this: the slider calls Apply,
# which saves, so point it at a scratch file instead.
$scratch = Join-Path $env:TEMP "tapa-native-check.conf"
@("size = $Size", "seed = $Seed", "density = 0.40-0.50") | Set-Content -Path $scratch -Encoding ascii
$settings = @("--config", $scratch) + $settings

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class NativeCheck {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int h2, uint flags);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    // GetWindowTextW cannot read another process's edit control (no caption), so
    // WM_GETTEXT - which Windows marshals across processes - is used instead.
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")]
    public static extern IntPtr SendMessageText(IntPtr h, uint m, IntPtr w, System.Text.StringBuilder l);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x, y; }

    /// Screen grab as tightly packed BGRA rows.
    public static byte[] Grab(int x, int y, int w, int h) {
        using (var bmp = new Bitmap(w, h)) {
            using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
            var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var buf = new byte[w * 4 * h];
            for (int row = 0; row < h; row++) {
                Marshal.Copy(IntPtr.Add(data.Scan0, row * data.Stride), buf, row * w * 4, w * 4);
            }
            bmp.UnlockBits(data);
            return buf;
        }
    }

    public static int Diff(byte[] a, byte[] b) {
        int n = 0;
        for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) n++;
        return n;
    }

    /// Which cells are drawn as clue cells (cream background)? Returns one bool
    /// per cell, row major. The clue background is rgb(253,248,226); undecided
    /// cells are rgb(232,234,240) and empty cells are white, so the blue channel
    /// and the red channel tell all three apart.
    public static bool[] ClueCells(byte[] shot, int width, int cell, int cols, int rows) {
        var flags = new bool[cols * rows];
        for (int cy = 0; cy < rows; cy++) {
            for (int cx = 0; cx < cols; cx++) {
                int cream = 0, total = 0;
                for (int y = cy * cell + 3; y < (cy + 1) * cell - 3; y++) {
                    for (int x = cx * cell + 3; x < (cx + 1) * cell - 3; x++) {
                        int o = (y * width + x) * 4;
                        total++;
                        int b = shot[o], g = shot[o + 1], r = shot[o + 2];
                        if (r > 245 && g > 240 && b > 215 && b < 240) cream++;
                    }
                }
                flags[cy * cols + cx] = total > 0 && cream * 100 / total > 20;
            }
        }
        return flags;
    }

    /// Distance between the vertical grid lines on row `y`, i.e. the current
    /// cell size. `lines` receives how many lines were found. Returns 0 when no
    /// grid was found.
    public static int DetectCell(byte[] shot, int width, int y, int fromX, int toX, out int lines) {
        var xs = new System.Collections.Generic.List<int>();
        for (int x = fromX; x < toX; x++) {
            int o = (y * width + x) * 4;
            int b = shot[o], g = shot[o + 1], r = shot[o + 2];
            if (Math.Abs(r - 168) < 16 && Math.Abs(g - 174) < 16 && Math.Abs(b - 188) < 16) xs.Add(x);
        }
        lines = 0;
        if (xs.Count < 3) return 0;
        var centres = new System.Collections.Generic.List<int>();
        int start = xs[0], prev = xs[0];
        for (int i = 1; i < xs.Count; i++) {
            if (xs[i] != prev + 1) { centres.Add((start + prev) / 2); start = xs[i]; }
            prev = xs[i];
        }
        centres.Add((start + prev) / 2);
        lines = centres.Count;
        if (centres.Count < 2) return 0;
        var gaps = new System.Collections.Generic.List<int>();
        for (int i = 1; i < centres.Count; i++) gaps.Add(centres[i] - centres[i - 1]);
        gaps.Sort();
        return gaps[gaps.Count / 2];
    }

    /// How many pixels in the rectangle match rgb(r,g,b) within `tol`.
    public static int CountColor(byte[] shot, int width, int x0, int y0, int x1, int y1, int r, int g, int b, int tol) {
        int n = 0;
        for (int y = y0; y < y1; y++) {
            for (int x = x0; x < x1; x++) {
                int o = (y * width + x) * 4;
                if (Math.Abs(shot[o] - b) <= tol && Math.Abs(shot[o + 1] - g) <= tol && Math.Abs(shot[o + 2] - r) <= tol) n++;
            }
        }
        return n;
    }
}
'@

[NativeCheck]::SetProcessDPIAware() | Out-Null

$failures = 0
function Report([string]$label, [bool]$ok, [string]$detail) {
    if ($ok) { Write-Host "  ok   $label" }
    else { Write-Host "  FAIL $label ($detail)"; $script:failures += 1 }
}

# ---- the puzzle the game should be showing, from the CLI -------------------
$printed = & $exe --print 1 @settings
$rows = @()
$inGrid = $false
foreach ($line in $printed) {
    if ($line -match "^Tapa ") { $inGrid = $true; continue }
    if ($line -match "^solution:") { break }
    if ($inGrid -and $line.Trim().Length -gt 0) { $rows += $line }
}
if ($rows.Count -ne $Size) { throw "could not parse the printed puzzle" }
$expected = ""
for ($y = 0; $y -lt $Size; $y++) {
    $cells = @($rows[$y] -split '\s+' | Where-Object { $_ -ne "" })
    for ($x = 0; $x -lt $Size; $x++) { $expected += if ($cells[$x] -eq ".") { "." } else { "#" } }
}
Write-Host "cli puzzle: $expected"

# ---- launch ----------------------------------------------------------------
$proc = Start-Process -FilePath $exe -ArgumentList $settings -PassThru
Start-Sleep -Seconds 3

$script:hwnd = [IntPtr]::Zero
$cb = [NativeCheck+EnumProc]{
    param($h, $l)
    $owner = 0
    [void][NativeCheck]::GetWindowThreadProcessId($h, [ref]$owner)
    if ($owner -eq $proc.Id) {
        $name = New-Object System.Text.StringBuilder 128
        [void][NativeCheck]::GetClassNameW($h, $name, 128)
        if ($name.ToString() -eq "TapaWndClass" -and $script:hwnd -eq [IntPtr]::Zero) {
            $script:hwnd = $h
        }
    }
    return $true
}
[void][NativeCheck]::EnumWindows($cb, [IntPtr]::Zero)
if ($script:hwnd -eq [IntPtr]::Zero) { $proc.Kill(); throw "window not found" }

$rect = New-Object NativeCheck+RECT
[void][NativeCheck]::GetClientRect($script:hwnd, [ref]$rect)
$point = New-Object NativeCheck+POINT
[void][NativeCheck]::ClientToScreen($script:hwnd, [ref]$point)
$clientW = $rect.right - $rect.left
# render.rs: client_w = 2*24 + cell*cols + 18 + 320
$script:cell = [math]::Floor(($clientW - 48 - 18 - 320) / $Size)
$script:originX = $point.x
$script:originY = $point.y
$gx = $point.x + 24
$gy = $point.y + 62
$gw = $script:cell * $Size
$gh = $script:cell * $Size
Write-Host "client ${clientW}x$($rect.bottom - $rect.top) at $($point.x),$($point.y); cell=$($script:cell)"

function Grab-Grid {
    # the game must be on top: CopyFromScreen reads whatever is on the desktop
    [void][NativeCheck]::SetForegroundWindow($script:hwnd)
    [void][NativeCheck]::BringWindowToTop($script:hwnd)
    Start-Sleep -Milliseconds 250
    return [NativeCheck]::Grab($script:gx, $script:gy, $script:gw, $script:gh)
}

# Grab twice and only trust the shot when two consecutive frames agree, so a
# capture taken mid-repaint cannot masquerade as a change.
function Grab-Stable {
    $shot = Grab-Grid
    for ($i = 0; $i -lt 5; $i++) {
        $next = Grab-Grid
        if ([NativeCheck]::Diff($shot, $next) -eq 0) { return $shot }
        $shot = $next
    }
    return $shot
}

# Grab the whole client area, with the game raised first (CopyFromScreen reads
# whatever is on top of the desktop).
function Grab-Client {
    [void][NativeCheck]::SetForegroundWindow($script:hwnd)
    [void][NativeCheck]::BringWindowToTop($script:hwnd)
    Start-Sleep -Milliseconds 250
    $r = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetClientRect($script:hwnd, [ref]$r)
    $p = New-Object NativeCheck+POINT
    [void][NativeCheck]::ClientToScreen($script:hwnd, [ref]$p)
    return @{
        shot = [NativeCheck]::Grab($p.x, $p.y, $r.right, $r.bottom)
        w = $r.right
        h = $r.bottom
    }
}

function Click-Cell([int]$cx, [int]$cy, [bool]$alt) {
    # WM_LBUTTONDOWN carries client coordinates, not screen coordinates
    $px = 24 + $cx * $script:cell + [math]::Floor($script:cell / 2)
    $py = 62 + $cy * $script:cell + [math]::Floor($script:cell / 2)
    $lp = [IntPtr][int](($py -shl 16) -bor ($px -band 0xFFFF))
    if ($alt) {
        [NativeCheck]::keybd_event(0x12, 0, 0, [IntPtr]::Zero)   # VK_MENU down
        Start-Sleep -Milliseconds 120
        if ([NativeCheck]::GetAsyncKeyState(0x12) -ge 0) {
            Write-Host "  warn: Alt did not register before the click"
        }
    }
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0201, [IntPtr]1, $lp)
    Start-Sleep -Milliseconds 120
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0202, [IntPtr]0, $lp)
    if ($alt) {
        Start-Sleep -Milliseconds 120
        [NativeCheck]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)   # VK_MENU up
    }
    Start-Sleep -Milliseconds 500
}

$script:gx = $gx; $script:gy = $gy; $script:gw = $gw; $script:gh = $gh

# ---- find the clue cells the game actually drew ---------------------------
$shot = Grab-Stable
$flags = [NativeCheck]::ClueCells($shot, $gw, $script:cell, $Size, $Size)
$detected = ""
$clueCells = @()
$freeCells = @()
for ($y = 0; $y -lt $Size; $y++) {
    for ($x = 0; $x -lt $Size; $x++) {
        if ($flags[$y * $Size + $x]) { $detected += "#"; $clueCells += , @($x, $y) }
        else { $detected += "."; $freeCells += , @($x, $y) }
    }
}
Write-Host "drawn puzzle: $detected"
Report "the drawn board matches the CLI puzzle" ($detected -eq $expected) "cli=$expected drawn=$detected"
Report "the board has clues and free cells" ($clueCells.Count -gt 0 -and $freeCells.Count -gt 0) "clues=$($clueCells.Count)"

# ---- 1. a plain click paints one cell -------------------------------------
$before = Grab-Stable
Click-Cell $freeCells[0][0] $freeCells[0][1] $false
$after = Grab-Stable
$paintDiff = [NativeCheck]::Diff($before, $after)
Report "a plain left click paints a cell" ($paintDiff -gt 0) "nothing changed"
Write-Host "       ($paintDiff bytes changed; one cell is about $((4 * $script:cell * $script:cell)))"

# ---- 2. Alt + click steps a clue ------------------------------------------
$altDiff = 0
$altCell = "(none)"
foreach ($c in $clueCells) {
    $before = Grab-Stable
    Click-Cell $c[0] $c[1] $true
    $after = Grab-Stable
    $d = [NativeCheck]::Diff($before, $after)
    if ($d -gt 0) { $altDiff = $d; $altCell = "$($c[0]),$($c[1])"; break }
}
Report "Alt + click steps the clue under the cursor" ($altDiff -gt 0) "no clue produced a deduction"
Write-Host "       (clue $altCell changed $altDiff bytes)"

# ---- 3. Alt + click on a non-clue cell paints nothing ---------------------
$before = Grab-Stable
Click-Cell $freeCells[$freeCells.Count - 1][0] $freeCells[$freeCells.Count - 1][1] $true
$after = Grab-Stable
$altFreeDiff = [NativeCheck]::Diff($before, $after)
Report "Alt + click on a non-clue cell paints nothing" ($altFreeDiff -eq 0) "$altFreeDiff bytes changed"

# ---- 4. the board-size slider rebuilds the board --------------------------
# find the trackbar and the first edit box among the children
$script:slider = [IntPtr]::Zero
$script:firstEdit = [IntPtr]::Zero
$script:editTexts = @()
$childCb = [NativeCheck+EnumProc]{
    param($h, $l)
    $name = New-Object System.Text.StringBuilder 128
    [void][NativeCheck]::GetClassNameW($h, $name, 128)
    if ($name.ToString() -eq "msctls_trackbar32" -and $script:slider -eq [IntPtr]::Zero) { $script:slider = $h }
    if ($name.ToString() -eq "Edit") {
        $text = New-Object System.Text.StringBuilder 128
        [void][NativeCheck]::SendMessageText($h, 0x000D, [IntPtr]128, $text)
        $script:editTexts += $text.ToString()
        if ($script:firstEdit -eq [IntPtr]::Zero) { $script:firstEdit = $h }
    }
    return $true
}
[void][NativeCheck]::EnumChildWindows($script:hwnd, $childCb, [IntPtr]::Zero)
Report "the control column has a size slider" ($script:slider -ne [IntPtr]::Zero) "no trackbar found"

if ($script:slider -ne [IntPtr]::Zero) {
    $lines0 = 0
    $cell0 = [NativeCheck]::DetectCell($shot, $gw, 5, 0, $gw, [ref]$lines0)
    Write-Host "       (board $Size cells, cell $cell0 px, $lines0 grid lines)"

    # ---- 4. the cell-size slider zooms the board --------------------------
    $want = [math]::Min(96, $cell0 + 14)
    [void][NativeCheck]::SendMessageW($script:slider, 0x0405, [IntPtr]1, [IntPtr]$want)   # TBM_SETPOS
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0114, [IntPtr]8, $script:slider)    # WM_HSCROLL / TB_ENDTRACK
    Start-Sleep -Seconds 2

    $client2 = Grab-Client
    $shot2 = $client2.shot
    $w2 = $client2.w
    $h2 = $client2.h
    $lines2 = 0
    $cell2 = [NativeCheck]::DetectCell($shot2, $w2, 67, 20, $w2 - 24 - 320 - 18, [ref]$lines2)
    $box = New-Object System.Text.StringBuilder 64
    [void][NativeCheck]::SendMessageText($script:firstEdit, 0x000D, [IntPtr]64, $box)
    Report "the zoom slider enlarges the cells" ($cell2 -eq $want) "asked for $want, drew $cell2"
    Report "zooming keeps the number of cells" ($lines2 -eq $lines0) "grid lines $lines0 -> $lines2"
    Report "zooming leaves the board size field alone" ($box.ToString() -eq "$Size") "size box says '$($box.ToString())'"
    Write-Host "       (client $w2 x $h2, cell $cell2)"

    # zoom back out to a fixed small size
    [void][NativeCheck]::SendMessageW($script:slider, 0x0405, [IntPtr]1, [IntPtr]16)      # TBM_SETPOS
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0114, [IntPtr]8, $script:slider)    # WM_HSCROLL / TB_ENDTRACK
    Start-Sleep -Seconds 2
    $client4 = Grab-Client
    $shot4 = $client4.shot
    $w4 = $client4.w
    $lines4 = 0
    $cell4 = [NativeCheck]::DetectCell($shot4, $w4, 67, 20, $w4 - 24 - 320 - 18, [ref]$lines4)
    Report "the zoom slider shrinks the cells" ($cell4 -eq 16) "asked for 16, drew $cell4"
    Report "shrinking keeps the number of cells" ($lines4 -eq $lines0) "grid lines $lines0 -> $lines4"

    # ---- 5. resizing the window rescales the grid --------------------------
    $wr = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetWindowRect($script:hwnd, [ref]$wr)
    $bigW = $wr.right - $wr.left + 320
    $bigH = $wr.bottom - $wr.top + 260
    [void][NativeCheck]::SetWindowPos($script:hwnd, [IntPtr]::Zero, 0, 0, $bigW, $bigH, 0x0014) # SWP_NOMOVE|SWP_NOZORDER
    Start-Sleep -Seconds 2
    $client3 = Grab-Client
    $shot3 = $client3.shot
    $w3 = $client3.w
    $h3 = $client3.h
    $lines3 = 0
    $cell3 = [NativeCheck]::DetectCell($shot3, $w3, 67, 20, $w3 - 24 - 320 - 18, [ref]$lines3)
    $availW3 = ($w3 - 24 - 320) - 18 - 24
    $availH3 = $h3 - 62 - 118
    $expected3 = [math]::Max(11, [math]::Min(96, [math]::Min([math]::Floor($availW3 / $Size), [math]::Floor($availH3 / $Size))))
    Report "a bigger window grows the grid" ($cell3 -eq $expected3 -and $cell3 -gt $cell4) "cells $cell4 -> $cell3, expected $expected3"
    Report "resizing keeps the number of cells" ($lines3 -eq $lines0) "grid lines $lines0 -> $lines3"
    Write-Host "       (client $w3 x $h3, cell $cell3)"

    # ---- 6. the footer text never runs into the control column -------------
    # the footer hints use rgb(112,128,136); the control column draws its own
    # text in black, so any of that colour right of the board is an overflow
    $boardRight = 24 + $cell3 * $Size
    $spill = [NativeCheck]::CountColor($shot3, $w3, $boardRight + 2, 62 + $cell3 * $Size + 2, $w3, $h3, 112, 128, 136, 6)
    Report "board text stays out of the control column" ($spill -eq 0) "$spill footer pixels spilled into the panel"

    # and the same in a window squeezed to the minimum allowed size
    [void][NativeCheck]::SetWindowPos($script:hwnd, [IntPtr]::Zero, 0, 0, 300, 300, 0x0014)
    Start-Sleep -Seconds 2
    $client5 = Grab-Client
    $shot5 = $client5.shot
    $w5 = $client5.w
    $h5 = $client5.h
    $lines5 = 0
    $cell5 = [NativeCheck]::DetectCell($shot5, $w5, 67, 20, $w5 - 24 - 320 - 18, [ref]$lines5)
    $spill5 = [NativeCheck]::CountColor($shot5, $w5, 24 + $cell5 * $Size + 2, 62 + $cell5 * $Size + 2, $w5, $h5, 112, 128, 136, 6)
    Report "the minimum window still keeps the footer inside the board column" ($spill5 -eq 0) "$spill5 footer pixels spilled at the minimum size"
    Report "the grid stays at least 11 pixels per cell" ($cell5 -ge 11) "cells shrank to $cell5"
    Report "the minimum window keeps all the cells" ($lines5 -eq $lines0) "grid lines $lines0 -> $lines5"
    Write-Host "       (minimum client $w5 x $h5, cell $cell5)"
}

if (-not $proc.HasExited) { $proc.Kill() }
if ($failures -eq 0) { Write-Host "`nall native checks passed"; exit 0 }
Write-Host "`n$failures native check(s) failed"
exit 1
