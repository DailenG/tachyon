# One scripted 5 MB paste with the frame overlay on. Replaces the clipboard.
# With -TypeAfter, types "ZQX" immediately after Ctrl+V (it must land after the paste).
# Fails (throws, exit code 1) if Tachyon is not the foreground window before any batch of keys.
# With -FrameLog, Tachyon writes its per-frame timing log there (TACHYON_FRAME_LOG).
param([string]$Exe, [string]$Doc, [string]$Dir, [int]$Run, [switch]$TypeAfter, [string]$FrameLog = "")
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "focus.ps1")
$empty = Join-Path $Dir "empty-$Run.md"
[IO.File]::WriteAllText($empty, "")
Set-Clipboard -Value ([IO.File]::ReadAllText($Doc, [Text.Encoding]::UTF8))
if ($FrameLog) { Remove-Item $FrameLog -ErrorAction SilentlyContinue; $env:TACHYON_FRAME_LOG = $FrameLog }
$p = Start-Process -FilePath $Exe -ArgumentList "--new-instance", "`"$empty`"" -PassThru
Remove-Item Env:TACHYON_FRAME_LOG -ErrorAction SilentlyContinue
try {
    $deadline = (Get-Date).AddSeconds(15)
    do {
        if ((Get-Date) -gt $deadline) { throw "Tachyon window did not appear" }
        Start-Sleep -Milliseconds 100
        $p.Refresh()
    } while ($p.MainWindowHandle -eq 0)
    Start-Sleep -Seconds 2
    $sh = New-Object -ComObject WScript.Shell
    $hwnd = $p.MainWindowHandle
    Set-TachyonForeground $p.Id $hwnd "overlay"
    $shot = Join-Path $PSScriptRoot "winshot.ps1"
    $sh.SendKeys("^%f")
    Start-Sleep -Seconds 1
    Assert-Foreground $p.Id "paste"
    if ($FrameLog) { Add-Content -Path $FrameLog -Value "marker paste" }
    if ($TypeAfter) { $sh.SendKeys("^vZQX") } else { $sh.SendKeys("^v") }
    Start-Sleep -Seconds 4
    & powershell -NoProfile -ExecutionPolicy Bypass -File $shot -ProcessId $p.Id -Out (Join-Path $Dir "paste-$Run-after-paste.png")
    Set-TachyonForeground $p.Id $hwnd "ctrl-home"
    if ($FrameLog) { Add-Content -Path $FrameLog -Value "marker ctrl-home" }
    $sh.SendKeys("^{HOME}")
    Start-Sleep -Seconds 1
    & powershell -NoProfile -ExecutionPolicy Bypass -File $shot -ProcessId $p.Id -Out (Join-Path $Dir "paste-$Run-ctrl-home.png")
    Set-TachyonForeground $p.Id $hwnd "ctrl-end"
    if ($FrameLog) { Add-Content -Path $FrameLog -Value "marker ctrl-end" }
    $sh.SendKeys("^{END}")
    Start-Sleep -Seconds 1
    & powershell -NoProfile -ExecutionPolicy Bypass -File $shot -ProcessId $p.Id -Out (Join-Path $Dir "paste-$Run-ctrl-end.png")
} finally {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id }
    Remove-Item $empty -ErrorAction SilentlyContinue
}
