//! Optional notification-area icon; owned by the UI thread and removed on exit.
use crate::win::wide;
use std::{io, mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    System::LibraryLoader::GetModuleHandleW,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub const WM_TRAY: u32 = WM_APP + 20;

pub struct TrayIcon {
    data: NOTIFYICONDATAW,
    added: bool,
}
impl TrayIcon {
    pub fn new(host: HWND) -> Self {
        // SAFETY: LoadIcon returns a shared module icon, valid for process lifetime.
        let icon = unsafe {
            LoadIconW(
                GetModuleHandleW(ptr::null()),
                ptr::without_provenance::<u16>(1),
            )
        };
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: host,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        let tip = wide("BatterySentry · Open device panel");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        Self { data, added: false }
    }
    pub fn show(&mut self) -> io::Result<()> {
        if !self.added {
            // SAFETY: initialized structure with a live host and a shared icon.
            if unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) } == 0 {
                return Err(io::Error::other(
                    "Windows notification area is not available",
                ));
            }
            self.added = true;
        }
        Ok(())
    }
    pub fn hide(&mut self) {
        if self.added {
            // SAFETY: deleting this object's own icon; Explorer may already be gone.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data);
            }
            self.added = false;
        }
    }
    pub fn explorer_restarted(&mut self) {
        self.added = false;
    }
}
impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.hide();
    }
}

/// Tracks a menu without borrowing application state across its nested message loop.
pub fn context_menu(host: HWND) -> u32 {
    // SAFETY: live owner HWND; the temporary menu is destroyed after tracking.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(menu, MF_STRING, 1, wide("Open BatterySentry").as_ptr());
        AppendMenuW(menu, MF_STRING, 2, wide("Refresh").as_ptr());
        AppendMenuW(menu, MF_STRING, 4, wide("Settings").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, 3, wide("Exit").as_ptr());
        let mut position = POINT::default();
        GetCursorPos(&mut position);
        SetForegroundWindow(host);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            position.x,
            position.y,
            0,
            host,
            ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(host, WM_NULL, 0, 0);
        command as u32
    }
}
