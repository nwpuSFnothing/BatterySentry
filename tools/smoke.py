"""Windows integration checks. Standard-library Python is only a developer tool.

Run: python tools/smoke.py [--sample-seconds 65]
Never changes startup settings. Settings writes, if any, stay under artifacts/.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import struct
import subprocess
import time
import zlib

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target/release/BatterySentry.exe"
ARTIFACTS = ROOT / "artifacts"
ARTIFACTS.mkdir(exist_ok=True)
user = c.WinDLL("user32", use_last_error=True)
kernel = c.WinDLL("kernel32", use_last_error=True)
gdi = c.WinDLL("gdi32", use_last_error=True)
user.FindWindowW.argtypes = [w.LPCWSTR, w.LPCWSTR]
user.FindWindowW.restype = w.HWND
user.IsWindowVisible.argtypes = [w.HWND]
user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user.GetWindowLongPtrW.argtypes = [w.HWND, c.c_int]
user.GetWindowLongPtrW.restype = c.c_ssize_t
user.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
user.PrintWindow.argtypes = [w.HWND, w.HDC, w.UINT]
user.GetDC.argtypes = [w.HWND]
user.GetDC.restype = w.HDC
user.ReleaseDC.argtypes = [w.HWND, w.HDC]
gdi.CreateCompatibleDC.argtypes = [w.HDC]
gdi.CreateCompatibleDC.restype = w.HDC
gdi.CreateCompatibleBitmap.argtypes = [w.HDC, c.c_int, c.c_int]
gdi.CreateCompatibleBitmap.restype = w.HBITMAP
gdi.SelectObject.argtypes = [w.HDC, w.HGDIOBJ]
gdi.SelectObject.restype = w.HGDIOBJ
gdi.GetDIBits.argtypes = [w.HDC, w.HBITMAP, w.UINT, w.UINT, c.c_void_p, c.c_void_p, w.UINT]
gdi.DeleteObject.argtypes = [w.HGDIOBJ]
gdi.DeleteDC.argtypes = [w.HDC]
kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
kernel.OpenProcess.restype = w.HANDLE
kernel.CloseHandle.argtypes = [w.HANDLE]
kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4


class Memory(c.Structure):
    _fields_ = [("cb", w.DWORD), ("PageFaultCount", w.DWORD)] + [
        (name, c.c_size_t) for name in (
            "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
            "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
            "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage",
        )
    ]


kernel.K32GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]
user.GetGuiResources.argtypes = [w.HANDLE, w.DWORD]


def stats(pid):
    handle = kernel.OpenProcess(0x410, False, pid)
    assert handle, "OpenProcess failed"
    try:
        memory = Memory(cb=c.sizeof(Memory))
        assert kernel.K32GetProcessMemoryInfo(handle, c.byref(memory), c.sizeof(memory))
        times = [w.FILETIME() for _ in range(4)]
        assert kernel.GetProcessTimes(handle, *(c.byref(v) for v in times))
        cpu = sum((v.dwHighDateTime << 32) + v.dwLowDateTime for v in times[2:]) / 1e7
        return {"working_set_bytes": memory.WorkingSetSize, "private_bytes": memory.PrivateUsage,
                "cpu_seconds": cpu, "gdi_objects": user.GetGuiResources(handle, 0),
                "user_objects": user.GetGuiResources(handle, 1)}
    finally:
        kernel.CloseHandle(handle)


def screenshot(hwnd):
    rect = w.RECT()
    assert user.GetWindowRect(hwnd, c.byref(rect))
    width, height = rect.right - rect.left, rect.bottom - rect.top
    screen = user.GetDC(None)
    dc = gdi.CreateCompatibleDC(screen)
    bitmap = gdi.CreateCompatibleBitmap(screen, width, height)
    old = gdi.SelectObject(dc, bitmap)
    try:
        assert user.PrintWindow(hwnd, dc, 2)
        gdi.SelectObject(dc, old)
        info = struct.pack("<IiiHHIIiiII", 40, width, -height, 1, 32, 0, width * height * 4, 0, 0, 0, 0)
        buffer = c.create_string_buffer(width * height * 4)
        assert gdi.GetDIBits(dc, bitmap, 0, height, buffer, info, 0)
        pixels = buffer.raw
        rows = bytearray()
        for y in range(height):
            rows.append(0)
            for x in range(width):
                b, g, r, _ = pixels[(y * width + x) * 4: (y * width + x) * 4 + 4]
                rows.extend((r, g, b))

        def chunk(tag, data):
            return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)

        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
        (ARTIFACTS / "panel.png").write_bytes(png)
    finally:
        gdi.SelectObject(dc, old)
        gdi.DeleteObject(bitmap)
        gdi.DeleteDC(dc)
        user.ReleaseDC(None, screen)


def wait_for(predicate, seconds=5):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError("Timed out waiting for window state")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sample-seconds", type=float, default=0)
    args = parser.parse_args()
    assert not user.FindWindowW("BatterySentry.Host", None), "Stop the existing BatterySentry first"
    report = {"binary_bytes": EXE.stat().st_size, "cycles": []}
    env = dict(os.environ, LOCALAPPDATA=str(ARTIFACTS / "runtime"))
    for cycle in range(3):
        process = subprocess.Popen([str(EXE), "--background"], env=env, creationflags=0x08000000)
        try:
            host = wait_for(lambda: user.FindWindowW("BatterySentry.Host", None))
            assert not user.FindWindowW("BatterySentry.Panel", None), "Panel must be lazy"
            time.sleep(3)
            before = stats(process.pid)
            if cycle == 0 and args.sample_seconds:
                print(f"Sampling tray-only process for {args.sample_seconds:g}s...", flush=True)
                time.sleep(args.sample_seconds)
                after = stats(process.pid)
                report["sample_seconds"] = args.sample_seconds
                report["tray_before"] = before
                report["tray_after"] = after
                report["sample_cpu_seconds"] = after["cpu_seconds"] - before["cpu_seconds"]
            subprocess.run([str(EXE)], env=env, timeout=5, check=True, creationflags=0x08000000)
            panel = wait_for(lambda: user.FindWindowW("BatterySentry.Panel", None))
            wait_for(lambda: user.IsWindowVisible(panel))
            assert user.GetWindowLongPtrW(panel, -16) & 0x00c00000 == 0, "Panel has a caption/border"
            subprocess.run([str(EXE)], env=env, timeout=5, check=True, creationflags=0x08000000)
            assert user.FindWindowW("BatterySentry.Panel", None) == panel, "Duplicate panel created"
            user.PostMessageW(host, 0x8001, 1, 0x0202)  # tray click -> refresh
            time.sleep(3)
            if cycle == 0:
                screenshot(panel)
                report["panel"] = stats(process.pid)
            user.PostMessageW(panel, 0x0010, 0, 0)  # close only hides
            wait_for(lambda: not user.IsWindowVisible(panel))
            assert process.poll() is None, "Closing panel terminated tray"
            user.PostMessageW(host, 0x8003, 0, 0)
            wait_for(lambda: user.IsWindowVisible(panel))
            report["cycles"].append({"cycle": cycle + 1, "single_instance": True, "borderless": True, "close_hides": True})
        finally:
            subprocess.run([str(EXE), "--quit"], env=env, timeout=5, check=True, creationflags=0x08000000)
            process.wait(timeout=10)
        assert process.returncode == 0, f"Exit code {process.returncode}"
        assert not user.FindWindowW("BatterySentry.Host", None), "Host remained after exit"
        assert not user.FindWindowW("BatterySentry.Panel", None), "Panel remained after exit"
    (ARTIFACTS / "smoke.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
