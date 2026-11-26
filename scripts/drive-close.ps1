# Verifies the dirty-close flow: edit a file, press the tab's close button, and
# check that the Save / Discard / Cancel prompt appears and that Save writes.
param([string]$OutDir = "$env:TEMP\ll-shots")

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Cl {
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

$probe = "E:\Codes\lllearning\aaa-close-probe.txt"
Set-Content -Path $probe -Value "original`n" -Encoding UTF8
$before = (Get-FileHash $probe -Algorithm MD5).Hash
Write-Host "before md5=$before"

# Fresh session pointing at the workspace, so the probe's row is predictable.
$cfg = Join-Path $env:APPDATA 'lllearning'
New-Item -ItemType Directory -Force -Path $cfg | Out-Null
@{ version = 1; workspace_root = "E:\Codes\lllearning" } | ConvertTo-Json | Set-Content (Join-Path $cfg 'session.json') -Encoding UTF8

$proc = Start-Process -FilePath "target\debug\lllearning.exe" -PassThru
$handle = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) { throw "the app exited with code $($proc.ExitCode)" }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Cl]::IsWindow($proc.MainWindowHandle)) {
        $handle = $proc.MainWindowHandle; break
    }
}
[Cl]::ShowWindow($handle, 9) | Out-Null
[Cl]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Seconds 2

$rect = New-Object Cl+RECT
[Cl]::GetWindowRect($handle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top

function Save-Shot($name) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $bmp.Save((Join-Path $OutDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

function Click-At($x, $y) {
    [Cl]::SetCursorPos($rect.Left + $x, $rect.Top + $y) | Out-Null
    Start-Sleep -Milliseconds 250
    [Cl]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Cl]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 700
}

# Open the probe: it sorts before every other file in the root listing.
Click-At 130 278
Start-Sleep -Seconds 2
Save-Shot "60-opened"

# Make it dirty.
Click-At 700 400
Start-Sleep -Milliseconds 400
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait("QQQ")
Start-Sleep -Milliseconds 900
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 400
Save-Shot "61-dirty"

# Press the tab's close button. The tab strip starts at x≈330; the close X of a
# single tab sits just right of its label.
Click-At 455 46
Start-Sleep -Seconds 2
Save-Shot "62-close-prompt"

# Answer Save, if the prompt is up: the primary button is at the dialog's foot.
Click-At 698 202
Start-Sleep -Seconds 2
Save-Shot "63-after-save"

$after = (Get-FileHash $probe -Algorithm MD5).Hash
Write-Host "after  md5=$after"
Write-Host "content: [$(Get-Content $probe -Raw)]"
if ($before -ne $after) { Write-Host "RESULT=SAVED" } else { Write-Host "RESULT=UNCHANGED" }

Remove-Item $probe -ErrorAction SilentlyContinue
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
