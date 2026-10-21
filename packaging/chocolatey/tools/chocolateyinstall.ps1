$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$version = $env:ChocolateyPackageVersion
$zip = Join-Path $toolsDir "tachyon-$version-x86_64-windows.zip"

# The release zip holds one folder, tachyon-<version>-x86_64-windows\. Its contents are moved up
# into tools\, so tachyon.exe keeps the same path across upgrades: a login autostart entry
# (tachyon --autostart on) records that path and keeps working after choco upgrade.
$staging = Join-Path $toolsDir 'staging'
Get-ChocolateyUnzip -FileFullPath64 $zip -Destination $staging -PackageName $env:ChocolateyPackageName
Get-ChildItem -LiteralPath (Join-Path $staging "tachyon-$version-x86_64-windows") |
    Move-Item -Destination $toolsDir -Force
Remove-Item -LiteralPath $staging -Recurse -Force
Remove-Item -LiteralPath $zip -Force

$exe = Join-Path $toolsDir 'tachyon.exe'

# Chocolatey shims tachyon.exe onto the PATH. The .gui marker makes the shim start Tachyon and
# return at once: the first launch stays running in the notification area (resident mode), so a
# waiting shim would hold the terminal until Tachyon quits.
Set-Content -LiteralPath "$exe.gui" -Value '' -Encoding Ascii

$programs = [Environment]::GetFolderPath('CommonPrograms')
Install-ChocolateyShortcut -ShortcutFilePath (Join-Path $programs 'Tachyon.lnk') `
    -TargetPath $exe -Description 'Tachyon Markdown editor'
