# Typing latency in a large document: opens -Doc with TACHYON_FRAME_LOG=-FrameLog, types -Chars
# characters -IntervalMs apart at the end of the document, then the same at its start. The frame
# log's key_to_paint_ms values are the result. Fails (throws, exit code 1) if Tachyon is not the
# foreground window before any batch of keys.
param(
    [Parameter(Mandatory = $true)] [string]$Exe,
    [Parameter(Mandatory = $true)] [string]$Doc,
    [Parameter(Mandatory = $true)] [string]$FrameLog,
    [int]$Chars = 120,
    [int]$IntervalMs = 60
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "focus.ps1")
Remove-Item $FrameLog -ErrorAction SilentlyContinue
$work = Join-Path (Split-Path $FrameLog) "typing-doc.md"
Copy-Item $Doc $work
$env:TACHYON_FRAME_LOG = $FrameLog
$p = Start-Process -FilePath $Exe -ArgumentList "--new-instance", "`"$work`"" -PassThru
Remove-Item Env:TACHYON_FRAME_LOG
try {
    $deadline = (Get-Date).AddSeconds(15)
    do {
        if ((Get-Date) -gt $deadline) { throw "Tachyon window did not appear" }
        Start-Sleep -Milliseconds 100
        $p.Refresh()
    } while ($p.MainWindowHandle -eq 0)
    # Loading and parsing the file happen in the background.
    Start-Sleep -Seconds 3
    $sh = New-Object -ComObject WScript.Shell
    $hwnd = $p.MainWindowHandle
    $text = ("the quick brown fox jumps over the lazy dog " * 10).Substring(0, $Chars)
    foreach ($place in @("end", "start")) {
        Set-TachyonForeground $p.Id $hwnd "typing-$place"
        if ($place -eq "end") { $sh.SendKeys("^{END}") } else { $sh.SendKeys("^{HOME}") }
        Start-Sleep -Seconds 1
        Add-Content -Path $FrameLog -Value "marker typing-$place begin"
        $i = 0
        foreach ($c in $text.ToCharArray()) {
            if ($i % 20 -eq 0) { Assert-Foreground $p.Id "typing-$place-$i" | Out-Null }
            $sh.SendKeys([string]$c)
            Start-Sleep -Milliseconds $IntervalMs
            $i++
        }
        Start-Sleep -Seconds 1
        Add-Content -Path $FrameLog -Value "marker typing-$place end"
    }
} finally {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id }
    Remove-Item $work -ErrorAction SilentlyContinue
}
