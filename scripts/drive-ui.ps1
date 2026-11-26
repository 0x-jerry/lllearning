# Drives the running app: clicks a folder, opens a file, types, saves.
# Kept as a script because the same sequence is the manual pass in docs/plan.md §8.
param(
    [string]$OutDir = "$env:TEMP\ll-shots",
    [string]$Workspace = "E:\Codes\lllearning"
)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Native {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
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
Start-Sleep -Seconds 2

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
    Start-Sleep -Milliseconds 700
}

Save-Shot "01-start"

# Expand "src" — the third row under the root, at the default 22px row pitch.
Click-At 100 195
Start-Sleep -Seconds 1
Save-Shot "02-src-expanded"

# Open main.rs: the first file inside src.
Click-At 140 225
Start-Sleep -Seconds 2
Save-Shot "03-file-open"

Write-Host "done"
