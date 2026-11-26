# Opens a plain-text file from the tree, types into the editor, saves with
# Ctrl+S, and reports whether the bytes on disk changed.
param([string]$OutDir = "$env:TEMP\ll-shots")

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ed {
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

# A scratch file whose name sorts early, so its row sits near the top of the
# root listing and is unambiguous to click.
$probe = "E:\Codes\lllearning\aaa-edit-probe.txt"
Set-Content -Path $probe -Value "alpha`nbeta`n" -Encoding UTF8
$before = (Get-FileHash $probe -Algorithm MD5).Hash
Write-Host "before md5=$before"

$proc = Start-Process -FilePath "target\debug\lllearning.exe" -PassThru
$handle = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $proc.Refresh()
    if ($proc.HasExited) { throw "the app exited with code $($proc.ExitCode)" }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Ed]::IsWindow($proc.MainWindowHandle)) {
        $handle = $proc.MainWindowHandle; break
    }
}
[Ed]::ShowWindow($handle, 9) | Out-Null
[Ed]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Seconds 2

$rect = New-Object Ed+RECT
[Ed]::GetWindowRect($handle, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
Write-Host "window ${w}x${h}"

function Save-Shot($name) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size $w, $h))
    $bmp.Save((Join-Path $OutDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

function Click-At($x, $y) {
    [Ed]::SetCursorPos($rect.Left + $x, $rect.Top + $y) | Out-Null
    Start-Sleep -Milliseconds 250
    [Ed]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 80
    [Ed]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 700
}

Start-Sleep -Milliseconds 600
Save-Shot "20-tree"

# The first file row under the root: root is row 1, then directories, then
# "aaa-edit-probe.txt" sorts before every other file.
Click-At 120 278
Start-Sleep -Seconds 2
Save-Shot "21-opened"

# Click into the editor's text area. The editor raises a word-completion
# popup as soon as typing starts, and it takes the keystrokes, so it is
# dismissed with Escape before the save is attempted.
Click-At 700 400
Start-Sleep -Milliseconds 500
[System.Windows.Forms.SendKeys]::SendWait("^a")
Start-Sleep -Milliseconds 300
[System.Windows.Forms.SendKeys]::SendWait("ZZZ")
Start-Sleep -Milliseconds 900
[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 500
Save-Shot "22-typed"

[System.Windows.Forms.SendKeys]::SendWait("^s")
Start-Sleep -Seconds 2
Save-Shot "23-saved"

$after = (Get-FileHash $probe -Algorithm MD5).Hash
Write-Host "after  md5=$after"
Write-Host "content: [$(Get-Content $probe -Raw)]"
if ($before -ne $after) { Write-Host "RESULT=CHANGED" } else { Write-Host "RESULT=UNCHANGED" }

Remove-Item $probe -ErrorAction SilentlyContinue
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
