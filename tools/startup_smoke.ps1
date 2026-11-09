# One focused check: hidden background start -> ordinary second launch -> visible panel.
# Explicitly uses the current test desktop and isolated settings, never the user's desktop.
$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
$runtimeDir = Join-Path $projectDir 'artifacts/startup-check'
New-Item -ItemType Directory -Force (Join-Path $runtimeDir 'BatterySentry') | Out-Null
[IO.File]::WriteAllText((Join-Path $runtimeDir 'BatterySentry/settings.ini'), "interval=1800`nshow_tray_icon=0`n")
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public sealed class StartupCheckProcess : IDisposable {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    struct STARTUPINFO {
        public int cb; public string reserved; public string desktop; public string title;
        public uint x,y,width,height,charsX,charsY,fill,flags; public ushort show,reserved2;
        public IntPtr reserved3,input,output,error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESSINFO { public IntPtr process,thread; public uint pid,tid; }
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]
    static extern bool CreateProcess(string app,StringBuilder command,IntPtr pa,IntPtr ta,bool inherit,uint flags,IntPtr environment,string directory,ref STARTUPINFO start,out PROCESSINFO process);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr handle,uint code);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr handle,uint milliseconds);
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint id);
    [DllImport("user32.dll")] static extern IntPtr GetProcessWindowStation();
    [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)]
    static extern bool GetUserObjectInformation(IntPtr obj,int index,StringBuilder value,int capacity,out int needed);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls,IntPtr title);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd,uint msg,IntPtr wp,IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
    delegate bool WindowCallback(IntPtr hwnd,IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumThreadWindows(uint thread,WindowCallback callback,IntPtr parameter);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd,StringBuilder text,int size);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd,StringBuilder text,int size);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr handle,out uint code);
    readonly IntPtr handle; public readonly uint Pid;
    static string Name(IntPtr obj) {
        var b=new StringBuilder(512); int needed;
        if(!GetUserObjectInformation(obj,2,b,1024,out needed)) throw new System.ComponentModel.Win32Exception();
        return b.ToString();
    }
    public StartupCheckProcess(string exe,string arguments,bool hidden) {
        var start=new STARTUPINFO { cb=Marshal.SizeOf(typeof(STARTUPINFO)),
            desktop=Name(GetProcessWindowStation())+"\\"+Name(GetThreadDesktop(GetCurrentThreadId())),
            flags=hidden?1u:0u, show=hidden?(ushort)0:(ushort)1 };
        PROCESSINFO p;
        if(!CreateProcess(exe,new StringBuilder("\""+exe+"\" "+arguments),IntPtr.Zero,IntPtr.Zero,false,0,IntPtr.Zero,System.IO.Path.GetDirectoryName(exe),ref start,out p))
            throw new System.ComponentModel.Win32Exception();
        handle=p.process; Pid=p.pid; CloseHandle(p.thread);
    }
    public bool Wait(uint ms) { return WaitForSingleObject(handle,ms)==0; }
    public IntPtr Window(string cls) {
        if(Wait(0)) return IntPtr.Zero;
        IntPtr found=IntPtr.Zero;
        foreach(System.Diagnostics.ProcessThread thread in System.Diagnostics.Process.GetProcessById((int)Pid).Threads) {
            EnumThreadWindows((uint)thread.Id,delegate(IntPtr hwnd,IntPtr unused) {
                var name=new StringBuilder(256); GetClassName(hwnd,name,256);
                if(name.ToString()==cls) { found=hwnd; return false; } return true;
            },IntPtr.Zero);
            if(found!=IntPtr.Zero) break;
        }
        return found;
    }
    public string Describe() {
        uint code; GetExitCodeProcess(handle,out code);
        var result=new StringBuilder("pid="+Pid+" exit="+code);
        if(code!=259) return result.ToString();
        foreach(System.Diagnostics.ProcessThread thread in System.Diagnostics.Process.GetProcessById((int)Pid).Threads) {
            EnumThreadWindows((uint)thread.Id,delegate(IntPtr hwnd,IntPtr unused) {
                var cls=new StringBuilder(256); var text=new StringBuilder(512);
                GetClassName(hwnd,cls,256); GetWindowText(hwnd,text,512);
                result.Append(" ["+cls+": "+text+"]"); return true;
            },IntPtr.Zero);
        }
        return result.ToString();
    }
    public void Dispose() { if(!Wait(0)) TerminateProcess(handle,1); CloseHandle(handle); }
}
'@
function Wait-Check([scriptblock]$Condition) {
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do { if (& $Condition) { return }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    throw "Startup condition timed out ($script:stage): $($background.Describe())"
}
$previousLocalAppData = $env:LOCALAPPDATA
$children = [Collections.Generic.List[StartupCheckProcess]]::new()
try {
    $env:LOCALAPPDATA = $runtimeDir
    $exe = Join-Path $projectDir 'target/release/BatterySentry.exe'
    $background = [StartupCheckProcess]::new($exe, '--background', $true)
    $children.Add($background)
    $script:stage = 'background host creation'
    Wait-Check { $background.Window('BatterySentry.Host') -ne [IntPtr]::Zero }
    $hostWindow = $background.Window('BatterySentry.Host')
    [uint32]$ownerId = 0
    [StartupCheckProcess]::GetWindowThreadProcessId($hostWindow, [ref]$ownerId) | Out-Null
    if ($ownerId -ne $background.Pid) { throw 'The host does not belong to the check process' }
    if ($background.Window('BatterySentry.Panel') -ne [IntPtr]::Zero) { throw 'Background startup created a panel' }
    $ordinary = [StartupCheckProcess]::new($exe, '', $false)
    $children.Add($ordinary)
    if (-not $ordinary.Wait(5000)) { throw 'Second instance did not exit' }
    $script:stage = 'normal second launch opens panel'
    Wait-Check {
        $panelWindow = $background.Window('BatterySentry.Panel')
        $panelWindow -ne [IntPtr]::Zero -and [StartupCheckProcess]::IsWindowVisible($panelWindow)
    }
    $panelWindow = $background.Window('BatterySentry.Panel')
    [StartupCheckProcess]::SendMessage($panelWindow, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    [StartupCheckProcess]::SendMessage($hostWindow, 0x8014, [IntPtr]1, [IntPtr]0x202) | Out-Null
    if (-not [StartupCheckProcess]::IsWindowVisible($panelWindow)) { throw 'Tray callback did not reopen panel' }
    [StartupCheckProcess]::SendMessage($panelWindow, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $silent = [StartupCheckProcess]::new($exe, '--background', $true)
    $children.Add($silent)
    if (-not $silent.Wait(5000)) { throw 'Duplicate silent instance did not exit' }
    if ([StartupCheckProcess]::IsWindowVisible($panelWindow)) { throw 'Duplicate silent startup reopened the panel' }
    $reopen = [StartupCheckProcess]::new($exe, '', $false)
    $children.Add($reopen)
    if (-not $reopen.Wait(5000)) { throw 'Reopen instance did not exit' }
    $script:stage = 'normal reopen restores panel'
    Wait-Check { [StartupCheckProcess]::IsWindowVisible($panelWindow) }
    [StartupCheckProcess]::SendMessage($hostWindow, 0x8004, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $background.Wait(8000)) { throw 'Background instance did not exit cleanly' }
    Write-Output 'PASS: hidden start stays silent; ordinary relaunch opens panel; tray callback reopens panel; silent duplicate stays hidden; ordinary reopen succeeds.'
} finally {
    foreach ($child in $children) { $child.Dispose() }
    $env:LOCALAPPDATA = $previousLocalAppData
}
