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
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
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

function Click-Cell([int]$cx, [int]$cy, [bool]$alt) {
    # WM_LBUTTONDOWN carries client coordinates, not screen coordinates
    $px = 24 + $cx * $script:cell + [math]::Floor($script:cell / 2)
    $py = 62 + $cy * $script:cell + [math]::Floor($script:cell / 2)
    $lp = [IntPtr][int](($py -shl 16) -bor ($px -band 0xFFFF))
    if ($alt) {
        [NativeCheck]::keybd_event(0x12, 0, 0, [IntPtr]::Zero)   # VK_MENU down
        Start-Sleep -Milliseconds 80
    }
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0201, [IntPtr]1, $lp)
    Start-Sleep -Milliseconds 80
    [void][NativeCheck]::PostMessageW($script:hwnd, 0x0202, [IntPtr]0, $lp)
    if ($alt) {
        Start-Sleep -Milliseconds 80
        [NativeCheck]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)   # VK_MENU up
    }
    Start-Sleep -Milliseconds 450
}

$script:gx = $gx; $script:gy = $gy; $script:gw = $gw; $script:gh = $gh

# ---- find the clue cells the game actually drew ---------------------------
$shot = Grab-Grid
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
$before = Grab-Grid
Click-Cell $freeCells[0][0] $freeCells[0][1] $false
$after = Grab-Grid
$paintDiff = [NativeCheck]::Diff($before, $after)
Report "a plain left click paints a cell" ($paintDiff -gt 0) "nothing changed"
Write-Host "       ($paintDiff bytes changed; one cell is about $((4 * $script:cell * $script:cell)))"

# ---- 2. Alt + click steps a clue ------------------------------------------
$altDiff = 0
$altCell = "(none)"
foreach ($c in $clueCells) {
    $before = Grab-Grid
    Click-Cell $c[0] $c[1] $true
    $after = Grab-Grid
    $d = [NativeCheck]::Diff($before, $after)
    if ($d -gt 0) { $altDiff = $d; $altCell = "$($c[0]),$($c[1])"; break }
}
Report "Alt + click steps the clue under the cursor" ($altDiff -gt 0) "no clue produced a deduction"
Write-Host "       (clue $altCell changed $altDiff bytes)"

# ---- 3. Alt + click on a non-clue cell paints nothing ---------------------
$before = Grab-Grid
Click-Cell $freeCells[$freeCells.Count - 1][0] $freeCells[$freeCells.Count - 1][1] $true
$after = Grab-Grid
$altFreeDiff = [NativeCheck]::Diff($before, $after)
Report "Alt + click on a non-clue cell paints nothing" ($altFreeDiff -eq 0) "$altFreeDiff bytes changed"

if (-not $proc.HasExited) { $proc.Kill() }
if ($failures -eq 0) { Write-Host "`nall native checks passed"; exit 0 }
Write-Host "`n$failures native check(s) failed"
exit 1
