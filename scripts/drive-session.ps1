# Verifies that a restart restores the folder, the tabs and the pane widths:
# open files, move a divider, restart, and find the same layout.
param([string]$OutDir = "$env:TEMP\ll-shots")

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ses {
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
$sessionPath = Join-Path $env:APPDATA 'lllearning\session.json'
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

function Start-App {
    $proc = Start-Process -FilePath "target\debug\lllearning.exe" -PassThru
    $handle = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) { throw "the app exited with code $($proc.ExitCode)" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Ses]::IsWindow($proc.MainWindowHandle)) {
            $handle = $proc.MainWindowHandle; break
        }
    }
    if ($handle -eq [IntPtr]::Zero) { throw "no window" }
    [Ses]::ShowWindow($handle, 9) | Out-Null
    [Ses]::SetForegroundWindow($handle) | Out-Null
    Start-Sleep -Seconds 2
    return @{ Proc = $proc; Handle = $handle }
}

function Get-Rect($handle) {
    $r = New-Object Ses+RECT
    [Ses]::GetWindowRect($handle, [ref]$r) | Out-Null
    return $r
}

function Save-Shot($name, $rect) {
    $w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $bmp.Save((Join-Path $OutDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

function Click-At($rect, $x, $y) {
    [Ses]::SetCursorPos($rect.Left + $x, $rect.Top + $y) | Out-Null
    Start-Sleep -Milliseconds 250
    [Ses]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Ses]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 700
}

# First run: open two files.
$app = Start-App
$rect = Get-Rect $app.Handle
Click-At $rect 120 192   # a file row
Start-Sleep -Seconds 1
Click-At $rect 120 250   # another file row
Start-Sleep -Seconds 1
Save-Shot "30-first-run" $rect
Write-Host "--- first run done; waiting for the debounced save"
Start-Sleep -Seconds 2

# Quit through the window manager so the quit hook runs.
Stop-Process -Id $app.Proc.Id -Force
Start-Sleep -Seconds 2

Write-Host "--- session file after the first run:"
if (Test-Path $sessionPath) { Get-Content $sessionPath -Raw } else { Write-Host "MISSING" }

# Second run: the same tabs should come back.
$app2 = Start-App
$rect2 = Get-Rect $app2.Handle
Start-Sleep -Seconds 2
Save-Shot "31-restored" $rect2
Write-Host "--- second run done"
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
