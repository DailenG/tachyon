# One traced benchmark run: xtask report to <Name>.txt, gpui-trace lines to <Name>.trace.txt.
param(
    [Parameter(Mandatory = $true)] [string]$Repo,
    [Parameter(Mandatory = $true)] [string]$Exe,
    [Parameter(Mandatory = $true)] [string]$OutDir,
    [Parameter(Mandatory = $true)] [string]$Name,
    [string]$SetEnv = "",    # NAME=VALUE, several separated by ";"
    [int]$Runs = 10,
    [switch]$Warm
)
$ErrorActionPreference = "Stop"
$experimentVars = @("GPUI_TRACE_FONT_NO_UPDATE_CHECK", "GPUI_DISABLE_DIRECT_COMPOSITION", "GPUI_TRACE_PLACE_HIDDEN", "TACHYON_EXP_NO_TRANSITIONS", "TACHYON_FRAME_LOG")
foreach ($var in $experimentVars) { Remove-Item "Env:$var" -ErrorAction SilentlyContinue }
foreach ($assignment in ($SetEnv.Split(";") | Where-Object { $_ })) {
    $pair = $assignment.Split("=", 2)
    Set-Item -Path "Env:$($pair[0])" -Value $pair[1]
}
$cargoArgs = @("xtask", "bench-startup", "--no-build", "--bin", $Exe, "--runs", "$Runs")
if ($Warm) { $cargoArgs += "--warm" }
$proc = Start-Process -FilePath "cargo" -ArgumentList $cargoArgs -WorkingDirectory $Repo `
    -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput (Join-Path $OutDir "$Name.txt") `
    -RedirectStandardError (Join-Path $OutDir "$Name.trace.txt")
foreach ($var in $experimentVars) { Remove-Item "Env:$var" -ErrorAction SilentlyContinue }
$traceLines = (Select-String -Path (Join-Path $OutDir "$Name.trace.txt") -Pattern "^gpui-trace ").Count
Write-Output "$Name exit=$($proc.ExitCode) trace_lines=$traceLines"
