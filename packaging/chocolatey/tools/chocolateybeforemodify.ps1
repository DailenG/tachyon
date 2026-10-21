$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$exe = Join-Path $toolsDir 'tachyon.exe'

# Runs before an upgrade or uninstall: Tachyon must not be running from this package, or its
# files are locked. Only processes started from this package's tachyon.exe are touched; a
# Tachyon from another install (for example the MSIX build) is left alone.
function Get-PackageProcess {
    @(Get-Process -Name 'tachyon' -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and ($_.Path -ieq $exe) })
}

$ours = Get-PackageProcess
if ($ours.Count -eq 0) { return }

# Quit gracefully first, so unsaved documents are kept for the next start (hot exit). `--quit`
# goes to the running instance of the calling user, so it is sent only when every Tachyon in
# this session is ours; otherwise it could reach a Tachyon from another install.
$session = (Get-Process -Id $PID).SessionId
$inSession = @(Get-Process -Name 'tachyon' -ErrorAction SilentlyContinue |
    Where-Object { $_.SessionId -eq $session })
$oursInSession = @($ours | Where-Object { $_.SessionId -eq $session })
if ($oursInSession.Count -gt 0 -and $oursInSession.Count -eq $inSession.Count) {
    Write-Host 'Asking Tachyon to quit (unsaved documents are kept)...'
    Start-Process -FilePath $exe -ArgumentList '--quit' -WindowStyle Hidden -Wait
    $oursInSession | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

$left = Get-PackageProcess
if ($left.Count -gt 0) {
    Write-Warning "Closing $($left.Count) Tachyon process(es) still running from this package."
    $left | Stop-Process -Force -ErrorAction SilentlyContinue
    $left | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}
