# Foreground helpers for scripts that send keystrokes. Dot-source this file.
#
# Windows' foreground lock can keep another app (a browser, the agent's terminal) in front; keys
# sent with SendKeys then go to that app instead of Tachyon, and nothing reports it. Starting
# Tachyon from the harness is not enough: a new process may only take the foreground if the process
# that started it is the foreground process, and AllowSetForegroundWindow only works when called by
# the foreground process. What does work from the harness: a synthetic Alt tap makes this process
# the one that received the last input event, which allows SetForegroundWindow. Every batch of keys
# is preceded by Assert-Foreground, which throws if the foreground window is not Tachyon's.
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Fg {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    public static uint ForegroundPid() {
        uint pid;
        GetWindowThreadProcessId(GetForegroundWindow(), out pid);
        return pid;
    }
    public static string ForegroundTitle() {
        var s = new StringBuilder(256);
        GetWindowText(GetForegroundWindow(), s, 256);
        return s.ToString();
    }
}
"@

# Throws unless the foreground window belongs to process $ProcessId.
function Assert-Foreground([int]$ProcessId, [string]$Step) {
    $fg = [Fg]::ForegroundPid()
    if ($fg -ne $ProcessId) {
        $title = [Fg]::ForegroundTitle()
        throw "FOREGROUND CHECK FAILED before '$Step': the foreground window belongs to pid $fg ('$title'), not Tachyon (pid $ProcessId). No keys were sent."
    }
    Write-Output "foreground-ok step=$Step pid=$ProcessId"
}

# Brings $Hwnd (a window of process $ProcessId) to the foreground, then asserts it is there.
function Set-TachyonForeground([int]$ProcessId, [IntPtr]$Hwnd, [string]$Step) {
    if ($Hwnd -eq [IntPtr]::Zero) { throw "no Tachyon window handle for step '$Step'" }
    for ($attempt = 1; $attempt -le 5; $attempt++) {
        if ([Fg]::ForegroundPid() -eq $ProcessId) { break }
        if ([Fg]::IsIconic($Hwnd)) { [void][Fg]::ShowWindow($Hwnd, 9) }
        [Fg]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
        [Fg]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
        [void][Fg]::SetForegroundWindow($Hwnd)
        Start-Sleep -Milliseconds 200
    }
    Assert-Foreground $ProcessId $Step
}
