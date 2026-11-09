"""One short integration test on an isolated mock taskbar, not the user's Explorer."""
import ctypes as c
from ctypes import wintypes as w
from pathlib import Path
import subprocess
import sys
import time
import os
import smoke

u=smoke.user
u.SetProcessDpiAwarenessContext.argtypes=[c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))
k=smoke.kernel
k.GetModuleHandleW.argtypes=[w.LPCWSTR];k.GetModuleHandleW.restype=w.HMODULE
u.DefWindowProcW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM];u.DefWindowProcW.restype=c.c_ssize_t
u.CreateWindowExW.argtypes=[w.DWORD,w.LPCWSTR,w.LPCWSTR,w.DWORD,c.c_int,c.c_int,c.c_int,c.c_int,w.HWND,w.HMENU,w.HINSTANCE,c.c_void_p];u.CreateWindowExW.restype=w.HWND
u.FindWindowExW.argtypes=[w.HWND,w.HWND,w.LPCWSTR,w.LPCWSTR];u.FindWindowExW.restype=w.HWND
u.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT]
u.DestroyWindow.argtypes=[w.HWND]
PROC=c.WINFUNCTYPE(c.c_ssize_t,w.HWND,w.UINT,w.WPARAM,w.LPARAM)

@PROC
def procedure(h,msg,wp,lp):
    if msg==2:u.PostQuitMessage(0);return 0
    return u.DefWindowProcW(h,msg,wp,lp)

class Class(c.Structure):
    _fields_=[('style',w.UINT),('proc',PROC),('cls_extra',c.c_int),('wnd_extra',c.c_int),('instance',w.HINSTANCE),('icon',w.HICON),('cursor',w.HANDLE),('brush',w.HBRUSH),('menu',w.LPCWSTR),('name',w.LPCWSTR)]

def create(cls,title,x,y,width,height,parent=None):
    instance=k.GetModuleHandleW(None)
    wc=Class(0,procedure,0,0,instance,None,None,w.HBRUSH(16),None,cls)
    u.RegisterClassW(c.byref(wc))
    hwnd=u.CreateWindowExW(0x08000080,cls,title,(0x40000000 if parent else 0x80000000)|0x10000000,x,y,width,height,parent,None,instance,None)
    assert hwnd, c.get_last_error()
    return hwnd

def rect(h):
    r=w.RECT();assert u.GetWindowRect(h,c.byref(r));return r

def pump_until(predicate,seconds=12):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        msg=w.MSG()
        while u.PeekMessageW(c.byref(msg),None,0,0,1):u.TranslateMessage(c.byref(msg));u.DispatchMessageW(c.byref(msg))
        result=predicate()
        if result:return result
        time.sleep(.03)
    raise AssertionError('Mock taskbar state timeout')

if '--monitor' in sys.argv:
    create('LiteMonitor.Fixture','LiteMonitor',1000,0,120,48,int(sys.argv[-1]))
    msg=w.MSG()
    while u.GetMessageW(c.byref(msg),None,0,0)>0:u.TranslateMessage(c.byref(msg));u.DispatchMessageW(c.byref(msg))
else:
    assert not u.FindWindowW('Shell_TrayWnd',None),'Run only on isolated desktop without a real taskbar'
    bar=create('Shell_TrayWnd','Mock taskbar',0,0,1280,48)
    create('Start','Start',0,0,120,48,bar)
    create('TrayNotifyWnd','Tray',1120,0,160,48,bar)
    monitor=subprocess.Popen([sys.executable,__file__,'--monitor',str(bar)],creationflags=0x08000000)
    app=None
    try:
        lite=pump_until(lambda:u.FindWindowExW(bar,None,'LiteMonitor.Fixture',None))
        env=dict(os.environ,BATTERYSENTRY_LAYOUT_DEBUG='1')
        if '--uia' not in sys.argv:env['BATTERYSENTRY_LAYOUT_NO_UIA']='1'
        app=subprocess.Popen([str(smoke.EXE),'--background'],env=env,stderr=subprocess.PIPE,creationflags=0x08000000)
        host=pump_until(lambda:u.FindWindowW('BatterySentry.Host',None))
        assert not u.FindWindowW('BatterySentry.Panel',None),'Background startup opened panel'
        subprocess.run([str(smoke.EXE),'--background'],env=env,timeout=4,check=True,creationflags=0x08000000)
        assert not u.FindWindowW('BatterySentry.Panel',None),'Duplicate background startup opened panel'
        widget=pump_until(lambda:u.FindWindowExW(bar,None,'BatterySentry.Taskbar',None) or u.FindWindowW('BatterySentry.Taskbar',None))
        pump_until(lambda:u.IsWindowVisible(widget))
        style=u.GetWindowLongPtrW(widget,-16)
        assert style & 0x40000000, f'Taskbar window is not attached as child: {style:x}'
        initial=rect(widget);reserved=rect(lite)
        assert initial.right==reserved.left,(initial.right,reserved.left)
        assert initial.right-initial.left>=150
        u.SetWindowPos(lite,None,900,0,220,48,0x14)
        pump_until(lambda:rect(widget).right==900)
        moved=rect(widget)
        focus_a=create('FocusFixture.A','A',100,100,200,100)
        focus_b=create('FocusFixture.B','B',400,100,200,100)
        EVENTS=[]
        EVENTPROC=c.WINFUNCTYPE(None,w.HANDLE,w.DWORD,w.HWND,c.c_long,c.c_long,w.DWORD,w.DWORD)
        @EVENTPROC
        def observed(h,event,window,obj,child,tid,tick):
            if window==widget and obj==0:EVENTS.append(event)
        u.SetWinEventHook.argtypes=[w.DWORD,w.DWORD,w.HMODULE,EVENTPROC,w.DWORD,w.DWORD,w.DWORD]
        u.SetWinEventHook.restype=w.HANDLE
        u.UnhookWinEvent.argtypes=[w.HANDLE]
        hook=u.SetWinEventHook(0x8002,0x8003,None,observed,0,0,0)
        for window in [focus_a,focus_b,focus_a,focus_b]:
            u.SetWindowPos(window,None,0,0,0,0,3)
            until=time.monotonic()+.2
            pump_until(lambda:time.monotonic()>=until)
        assert u.IsWindowVisible(widget), 'Taskbar widget hidden on focus switch'
        assert not EVENTS, f'Widget visibility flickered: {EVENTS}'
        u.UnhookWinEvent(hook)
        print('PASS: taskbar child attached; four normal focus switches cause no show/hide events')
        u.DestroyWindow(focus_a);u.DestroyWindow(focus_b)
        smoke.screenshot(widget)
        Path(smoke.ARTIFACTS/'panel.png').replace(smoke.ARTIFACTS/'taskbar-widget.png')
        host=u.FindWindowW('BatterySentry.Host',None)
        u.PostMessageW(host,0x8003,0,0)
        panel=pump_until(lambda:u.FindWindowW('BatterySentry.Panel',None))
        pump_until(lambda:u.IsWindowVisible(panel))
        smoke.screenshot(panel)
        Path(smoke.ARTIFACTS/'panel.png').replace(smoke.ARTIFACTS/'panel-aligned.png')
        print(f'PASS: widget [{initial.left},{initial.right}] before LiteMonitor {reserved.left}; moves to [{moved.left},{moved.right}] after LiteMonitor expands to x=900')
    finally:
        if app:
            host=u.FindWindowW('BatterySentry.Host',None)
            if host:u.PostMessageW(host,0x8004,0,0)
            try:pump_until(lambda:app.poll() is not None,seconds=5)
            except AssertionError:app.kill();app.wait(timeout=3)
            print('layout trace:',app.stderr.read().decode(errors='replace'))
        u.PostMessageW(lite,0x10,0,0)
        try:pump_until(lambda:monitor.poll() is not None,seconds=3)
        except AssertionError:monitor.kill();monitor.wait(timeout=3)
        u.DestroyWindow(bar)
