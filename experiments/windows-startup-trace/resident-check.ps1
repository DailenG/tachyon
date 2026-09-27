# Functional check of resident mode with a ready window (normal build).
# Prints one line per step: "<step> expected=<x> got=<y> ok=<True|False>".
# Fails (throws, exit code 1) if Tachyon is not the foreground window before sending keys.
param([Parameter(Mandatory = $true)] [string]$Exe, [Parameter(Mandatory = $true)] [string]$Dir)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "focus.ps1")
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Wins {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public static List<string> Visible(uint pid) {
        var titles = new List<string>();
        EnumWindows((h, l) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero) {
                var s = new StringBuilder(256);
                GetWindowText(h, s, 256);
                titles.Add(s.ToString());
            }
            return true;
        }, IntPtr.Zero);
        return titles;
    }
    public static IntPtr FirstVisible(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero) {
                found = h;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void CloseVisible(uint pid) {
        EnumWindows((h, l) => {
            uint p;
            GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero) {
                PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero);
            }
            return true;
        }, IntPtr.Zero);
    }
}
"@
function Report([string]$Step, $Expected, $Got) {
    Write-Output ("{0} expected={1} got={2} ok={3}" -f $Step, $Expected, $Got, ($Expected -eq $Got))
}
$env:TACHYON_INSTANCE_ID = "tachyon-run3-check"
$file = Join-Path $Dir "resident-check.md"
[IO.File]::WriteAllText($file, "# Resident check`n")

$resident = $null
try {
    $resident = Start-Process -FilePath $Exe -ArgumentList "--resident" -PassThru
    Start-Sleep -Seconds 3
    Report "resident-start-visible-windows" 0 (@([Wins]::Visible([uint32]$resident.Id))).Count

    Start-Process -FilePath $Exe -ArgumentList "`"$file`"" -Wait
    Start-Sleep -Seconds 2
    $titles = @([Wins]::Visible([uint32]$resident.Id))
    Report "after-launch-visible-windows" 1 $titles.Count
    Report "after-launch-title-has-file" $true (($titles -join "|") -like "*resident-check.md*")
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "winshot.ps1") -ProcessId $resident.Id -Out (Join-Path $Dir "resident-after-launch.png")

    [Wins]::CloseVisible([uint32]$resident.Id)
    Start-Sleep -Seconds 2
    Report "after-close-visible-windows" 0 (@([Wins]::Visible([uint32]$resident.Id))).Count
    Report "after-close-process-alive" $true (-not $resident.HasExited)

    Start-Process -FilePath $Exe -Wait
    Start-Sleep -Seconds 2
    Report "second-launch-visible-windows" 1 (@([Wins]::Visible([uint32]$resident.Id))).Count

    $sh = New-Object -ComObject WScript.Shell
    # The resident process also owns the hidden ready window: target the visible one.
    Set-TachyonForeground $resident.Id ([Wins]::FirstVisible([uint32]$resident.Id)) "ctrl-q"
    $sh.SendKeys("^q")
    Start-Sleep -Seconds 3
    $resident.Refresh()
    Report "after-ctrl-q-process-exited" $true $resident.HasExited
} finally {
    if ($resident -and -not $resident.HasExited) { Stop-Process -Id $resident.Id }
    Remove-Item $file -ErrorAction SilentlyContinue
    Remove-Item Env:TACHYON_INSTANCE_ID -ErrorAction SilentlyContinue
}
