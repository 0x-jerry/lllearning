# Drives the right dock: collapsing it must take the native webview with it.
# The webview is a child HWND, not something GPUI paints, so the check is on the
# child window's own visibility rather than on pixels -- a screenshot can be
# occluded by any other window and would prove nothing.
param(
    [string]$OutDir = "$env:TEMP\ll-shots"
)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class Native {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc proc, IntPtr param);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    // The webview's host window: wry creates it with a known class name.
    public static IntPtr FindWebView(IntPtr parent) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (h, p) => {
            var name = new StringBuilder(256);
            GetClassName(h, name, 256);
            if (name.ToString() == "WRY_WEBVIEW") {
                found = h;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

$proc = Start-Process -FilePath "target\debug\lllearning.exe" -PassThru
$handle = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) { throw "the app exited with code $($proc.ExitCode)" }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Native]::IsWindow($proc.MainWindowHandle)) {
        $handle = $proc.MainWindowHandle
        break
    }
}
if ($handle -eq [IntPtr]::Zero) { throw "the app never produced a window" }

[Native]::ShowWindow($handle, 9) | Out-Null
[Native]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Seconds 3

$rect = New-Object Native+RECT
[Native]::GetWindowRect($handle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left
$h = $rect.Bottom - $rect.Top
if ($w -le 0 -or $h -le 0) { throw "the window has no size: ${w}x${h}" }
Write-Host "window ${w}x${h} at $($rect.Left),$($rect.Top)"

function Save-Shot($name) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $path = Join-Path $OutDir "$name.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "shot $path"
}

function Click-At($x, $y) {
    [Native]::SetCursorPos($rect.Left + $x, $rect.Top + $y) | Out-Null
    Start-Sleep -Milliseconds 250
    [Native]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Native]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 900
}

$webview = [Native]::FindWebView($handle)
if ($webview -eq [IntPtr]::Zero) { throw "no webview child window was found" }
Write-Host ("webview hwnd 0x{0:x}" -f [int64]$webview)

function Assert-Visible($name, $expected) {
    $visible = [Native]::IsWindowVisible($webview)
    Write-Host "$name webview visible: $visible"
    if ($visible -ne $expected) {
        throw "$name expected webview visible=$expected, got $visible"
    }
}

Save-Shot "01-start"

# The right strip's browser button, then the same button again to collapse.
$stripX = $w - 18
$browserY = 48

# A restored session may leave the dock collapsed, and a collapsed dock never
# renders the pane, so the first click only normalises the starting state.
if (-not [Native]::IsWindowVisible($webview)) {
    Click-At $stripX $browserY
}
Assert-Visible "01-open" $true

Click-At $stripX $browserY
Save-Shot "02-collapsed"
$collapsed = [Native]::IsWindowVisible($webview)
Write-Host "02-collapsed webview visible: $collapsed"

Click-At $stripX $browserY
Save-Shot "03-restored"
$restored = [Native]::IsWindowVisible($webview)
Write-Host "03-restored webview visible: $restored"

if (-not $collapsed -eq $false) { throw "the webview is still shown while the dock is collapsed" }
if (-not $restored) { throw "the webview did not come back when the dock reopened" }

Write-Host "done"
