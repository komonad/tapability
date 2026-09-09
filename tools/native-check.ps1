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
    $beforeW = $rect.right - $rect.left
    [void][NativeCheck]::SendMessageW($script:slider, 0x0405, [IntPtr]1, [IntPtr]10)   # TBM_SETPOS
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0114, [IntPtr]8, $script:slider) # WM_HSCROLL / TB_ENDTRACK
    Start-Sleep -Seconds 3

    $rect2 = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetClientRect($script:hwnd, [ref]$rect2)
    $afterW = $rect2.right - $rect2.left
    $box = New-Object System.Text.StringBuilder 64
    [void][NativeCheck]::SendMessageText($script:firstEdit, 0x000D, [IntPtr]64, $box)
    Report "the slider writes into the size box" ($box.ToString() -eq "10") "size box says '$($box.ToString())'"

    # the window keeps its size; the grid is re-fitted inside it
    $pt2 = New-Object NativeCheck+POINT
    [void][NativeCheck]::ClientToScreen($script:hwnd, [ref]$pt2)
    $shot2 = [NativeCheck]::Grab($pt2.x, $pt2.y, $afterW, $rect2.bottom - $rect2.top)
    $lines = 0
    $cell10 = [NativeCheck]::DetectCell($shot2, $afterW, 67, 20, $afterW - 24 - 320 - 18, [ref]$lines)
    $availW = ($afterW - 24 - 320) - 18 - 24
    $availH = ($rect2.bottom - $rect2.top) - 62 - 118
    $expectedCell = [math]::Max(11, [math]::Min(96, [math]::Min([math]::Floor($availW / 10), [math]::Floor($availH / 10))))
    Report "a 10x10 board rescales inside the same window" ($cell10 -eq $expectedCell) "drew cells of $cell10, expected $expectedCell"
    Write-Host "       (client $afterW x $($rect2.bottom - $rect2.top), cell $cell10, $lines grid lines)"

    # ---- 5. resizing the window rescales the grid --------------------------
    $wr = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetWindowRect($script:hwnd, [ref]$wr)
    $bigW = $wr.right - $wr.left + 320
    $bigH = $wr.bottom - $wr.top + 260
    [void][NativeCheck]::SetWindowPos($script:hwnd, [IntPtr]::Zero, 0, 0, $bigW, $bigH, 0x0014) # SWP_NOMOVE|SWP_NOZORDER
    Start-Sleep -Seconds 2
    $rect3 = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetClientRect($script:hwnd, [ref]$rect3)
    $pt3 = New-Object NativeCheck+POINT
    [void][NativeCheck]::ClientToScreen($script:hwnd, [ref]$pt3)
    $w3 = $rect3.right - $rect3.left
    $h3 = $rect3.bottom - $rect3.top
    $shot3 = [NativeCheck]::Grab($pt3.x, $pt3.y, $w3, $h3)
    $lines3 = 0
    $cell3 = [NativeCheck]::DetectCell($shot3, $w3, 67, 20, $w3 - 24 - 320 - 18, [ref]$lines3)
    $availW3 = ($w3 - 24 - 320) - 18 - 24
    $availH3 = $h3 - 62 - 118
    $expected3 = [math]::Max(11, [math]::Min(96, [math]::Min([math]::Floor($availW3 / 10), [math]::Floor($availH3 / 10))))
    Report "a bigger window grows the grid" ($cell3 -eq $expected3 -and $cell3 -gt $cell10) "cells $cell10 -> $cell3, expected $expected3"
    Write-Host "       (client $w3 x $h3, cell $cell3)"

    # ---- 6. the footer text never runs into the control column -------------
    # the footer hints use rgb(112,128,136); the control column draws its own
    # text in black, so any of that colour right of the board is an overflow
    $boardRight = 24 + $cell3 * 10
    $spill = [NativeCheck]::CountColor($shot3, $w3, $boardRight + 2, 62 + $cell3 * 10 + 2, $w3, $h3, 112, 128, 136, 6)
    Report "board text stays out of the control column" ($spill -eq 0) "$spill footer pixels spilled into the panel"

    # and the same in a window squeezed to the minimum allowed size
    [void][NativeCheck]::SetWindowPos($script:hwnd, [IntPtr]::Zero, 0, 0, 300, 300, 0x0014)
    Start-Sleep -Seconds 2
    $rect4 = New-Object NativeCheck+RECT
    [void][NativeCheck]::GetClientRect($script:hwnd, [ref]$rect4)
    $pt4 = New-Object NativeCheck+POINT
    [void][NativeCheck]::ClientToScreen($script:hwnd, [ref]$pt4)
    $w4 = $rect4.right - $rect4.left
    $h4 = $rect4.bottom - $rect4.top
    $shot4 = [NativeCheck]::Grab($pt4.x, $pt4.y, $w4, $h4)
    $lines4 = 0
    $cell4 = [NativeCheck]::DetectCell($shot4, $w4, 67, 20, $w4 - 24 - 320 - 18, [ref]$lines4)
    $spill4 = [NativeCheck]::CountColor($shot4, $w4, 24 + $cell4 * 10 + 2, 62 + $cell4 * 10 + 2, $w4, $h4, 112, 128, 136, 6)
    Report "the minimum window still keeps the footer inside the board column" ($spill4 -eq 0) "$spill4 footer pixels spilled at the minimum size"
    Report "the grid stays at least 11 pixels per cell" ($cell4 -ge 11) "cells shrank to $cell4"
    Write-Host "       (minimum client $w4 x $h4, cell $cell4)"
}

if (-not $proc.HasExited) { $proc.Kill() }
if ($failures -eq 0) { Write-Host "`nall native checks passed"; exit 0 }
Write-Host "`n$failures native check(s) failed"
exit 1
