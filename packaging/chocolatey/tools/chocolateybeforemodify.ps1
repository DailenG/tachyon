$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$exe = Join-Path $toolsDir 'tachyon.exe'

# Runs before an upgrade or uninstall: Tachyon must not be running from this package, or its
# files are locked. Only processes started from this package's tachyon.exe are touched; a
# Tachyon from another install (for example the MSIX build) is left alone.
#
# Paths come from Win32_Process, which reports them for every user's processes when elevated
# (Get-Process cannot read them for another user's process). A Tachyon whose path cannot be read
# might be ours, so the change is refused rather than half-replacing locked files.
function Get-TachyonProcess {
    @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'tachyon.exe'" -ErrorAction SilentlyContinue)
}

$all = Get-TachyonProcess
$unknown = @($all | Where-Object { -not $_.ExecutablePath })
if ($unknown.Count -gt 0) {
    throw ("Cannot tell whether Tachyon process(es) $(($unknown.ProcessId) -join ', ') run from this " +
        "package. Close Tachyon (tray icon > Quit Tachyon, or 'tachyon --quit') and try again.")
}
$ours = @($all | Where-Object { $_.ExecutablePath -ieq $exe })
if ($ours.Count -eq 0) { return }

# Quit gracefully first, so unsaved documents are kept for the next start (hot exit). `--quit`
# goes to the running instance of the calling user, so it is sent only when every Tachyon in
# this session is ours; otherwise it could reach a Tachyon from another install.
$session = (Get-Process -Id $PID).SessionId
$inSession = @($all | Where-Object { $_.SessionId -eq $session })
$oursInSession = @($ours | Where-Object { $_.SessionId -eq $session })
if ($oursInSession.Count -gt 0 -and $oursInSession.Count -eq $inSession.Count) {
    Write-Host 'Asking Tachyon to quit (unsaved documents are kept)...'
    Start-Process -FilePath $exe -ArgumentList '--quit' -WindowStyle Hidden -Wait
    Wait-Process -Id $oursInSession.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
}

$left = @(Get-TachyonProcess | Where-Object { $_.ExecutablePath -ieq $exe })
if ($left.Count -gt 0) {
    Write-Warning "Closing $($left.Count) Tachyon process(es) still running from this package."
    Stop-Process -Id $left.ProcessId -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $left.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
}
