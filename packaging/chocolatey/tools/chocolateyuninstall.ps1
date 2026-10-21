$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$exe = Join-Path $toolsDir 'tachyon.exe'

# chocolateybeforemodify.ps1 has already closed Tachyon; Chocolatey removes the package files and
# the shim. Remove what the install and the app added outside the package folder.

$shortcut = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'Tachyon.lnk'
Remove-Item -LiteralPath $shortcut -Force -ErrorAction SilentlyContinue

# `tachyon --autostart on` writes HKCU\...\Run "Tachyon" = "<path>\tachyon.exe" --background.
# Remove it for every signed-in user whose entry points at this package's tachyon.exe; an entry
# for another install (for example the MSIX build) is kept. Users who are not signed in keep a
# stale entry, which Windows skips because the file is gone.
$run = 'Software\Microsoft\Windows\CurrentVersion\Run'
$approved = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
foreach ($hive in Get-ChildItem -LiteralPath 'Registry::HKEY_USERS' -ErrorAction SilentlyContinue) {
    if ($hive.PSChildName -notmatch '^S-1-5-21-[\d-]+$') { continue }
    $runKey = "Registry::HKEY_USERS\$($hive.PSChildName)\$run"
    $command = (Get-ItemProperty -LiteralPath $runKey -Name 'Tachyon' -ErrorAction SilentlyContinue).Tachyon
    if ($command -and $command.IndexOf($exe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        Remove-ItemProperty -LiteralPath $runKey -Name 'Tachyon' -ErrorAction SilentlyContinue
        Remove-ItemProperty -LiteralPath "Registry::HKEY_USERS\$($hive.PSChildName)\$approved" `
            -Name 'Tachyon' -ErrorAction SilentlyContinue
        Write-Host "Removed Tachyon's sign-in autostart for $($hive.PSChildName)."
    }
}
