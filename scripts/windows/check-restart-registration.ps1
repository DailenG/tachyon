# Reads back a running process's RegisterApplicationRestart registration with
# GetApplicationRestartSettings, for manual verification of issue #80 (winlab).
#
# Usage: pwsh -File check-restart-registration.ps1 [-ProcessId <id>]
#
# Without -ProcessId, the first running "tachyon" process is used. Requires no special
# privileges against a process owned by the same user: OpenProcess only asks for
# PROCESS_QUERY_LIMITED_INFORMATION.

param(
    [int]$ProcessId = 0
)

$ErrorActionPreference = 'Stop'

if ($ProcessId -eq 0) {
    $proc = Get-Process -Name 'tachyon' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $proc) {
        Write-Error 'No running tachyon.exe process found; pass -ProcessId or start Tachyon first.'
        exit 1
    }
    $ProcessId = $proc.Id
}

$signature = @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class TachyonRestartInfo
{
    public const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(uint dwDesiredAccess, bool bInheritHandle, int dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetApplicationRestartSettings(
        IntPtr hProcess,
        StringBuilder pwzCommandLine,
        ref uint pcchSize,
        out uint pdwFlags);
}
'@

Add-Type -TypeDefinition $signature -Language CSharp

$handle = [TachyonRestartInfo]::OpenProcess(
    [TachyonRestartInfo]::PROCESS_QUERY_LIMITED_INFORMATION, $false, $ProcessId)
if ($handle -eq [IntPtr]::Zero) {
    $lastError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Write-Error "Could not open process $ProcessId (Win32 error $lastError)."
    exit 1
}

try {
    $size = [uint32]1024
    $commandLine = New-Object System.Text.StringBuilder([int]$size)
    $flags = [uint32]0
    $result = [TachyonRestartInfo]::GetApplicationRestartSettings(
        $handle, $commandLine, [ref]$size, [ref]$flags)

    if ($result -ne 0) {
        Write-Host ("GetApplicationRestartSettings failed for PID {0}: HRESULT 0x{1:X8} " +
            "(this is the expected result if the process has not registered, or has hot " +
            "exit turned off)." -f $ProcessId, $result)
        exit 1
    }

    Write-Host "PID $ProcessId is registered for restart:"
    Write-Host "  Command line: $($commandLine.ToString())"
    Write-Host "  Flags: 0x$($flags.ToString('X'))"
    if (($flags -band 0x1) -ne 0) { Write-Host '    RESTART_NO_CRASH is set' }
    if (($flags -band 0x2) -ne 0) { Write-Host '    RESTART_NO_HANG is set' }
    if (($flags -band 0x4) -ne 0) { Write-Host '    RESTART_NO_PATCH is set' }
    if (($flags -band 0x8) -ne 0) { Write-Host '    RESTART_NO_REBOOT is set' }
}
finally {
    [TachyonRestartInfo]::CloseHandle($handle) | Out-Null
}
