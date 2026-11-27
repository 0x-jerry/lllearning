# Measures whether the browser pane starts its WebView2 without stalling frames.
#
# A synchronous build waits for WebView2 inside a nested Win32 pump, so no frame
# and no GPUI task runs until it returns; the platform still dispatches window
# messages there, which is why a hung-window probe cannot see it. The signal is
# the startup log instead: the first frame must be presented *before* the page is
# ready, or the build was holding the render thread.
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Start {
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
}
"@

Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

$config = Join-Path $env:APPDATA 'lllearning'
$session = Join-Path $config 'session.json'
$backup = Join-Path $env:TEMP 'll-session-backup.json'
$log = Join-Path $env:TEMP 'll-start.log'
$had = Test-Path $session
if ($had) { Copy-Item $session $backup -Force }
Remove-Item $log -ErrorAction SilentlyContinue

# The build has to happen inside the run, and a collapsed dock builds nothing,
# so the right dock is forced open.
New-Item -ItemType Directory -Force -Path $config | Out-Null
Set-Content -Path $session -Encoding ascii -Value (ConvertTo-Json -Depth 4 @{
    version = 2
    workspace_root = (Get-Location).Path
    left = @{ active = 'explorer'; open = $true }
    right = @{ active = 'browser'; open = $true }
})

$proc = Start-Process -FilePath "target\debug\lllearning.exe" -PassThru -RedirectStandardOutput $log
$handle = [IntPtr]::Zero
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 100
    $proc.Refresh()
    if ($proc.HasExited) { throw "the app exited with code $($proc.ExitCode)" }
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Start]::IsWindow($proc.MainWindowHandle)) {
        $handle = $proc.MainWindowHandle
        break
    }
}
if ($handle -eq [IntPtr]::Zero) { throw "the app never produced a window" }

for ($i = 0; $i -lt 100; $i++) {
    Start-Sleep -Milliseconds 100
    if ((Get-Content $log -Raw -ErrorAction SilentlyContinue) -match 'webview ready') { break }
}
Start-Sleep -Milliseconds 300
$text = Get-Content $log -Raw -ErrorAction SilentlyContinue
Get-Process lllearning -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
if ($had) { Copy-Item $backup $session -Force } else { Remove-Item $session -ErrorAction SilentlyContinue }

function Mark($name) {
    if ($text -match "$name at ([0-9.]+) ms") { return [double]$Matches[1] }
    throw "the log has no `"$name`" mark: $log"
}

$build = Mark 'webview build started'
$ready = Mark 'webview ready'
$frame = Mark 'first frame presented'
$gap = $frame - $build
Write-Host ("build={0:N1} ms ready={1:N1} ms first-frame={2:N1} ms gap={3:N1} ms" -f $build, $ready, $frame, $gap)

if ($ready -le $build) { throw "the page became ready before the build started" }
if ($gap -ge 150) { throw "the render thread stalled $gap ms while the page started" }

Write-Host "done"
