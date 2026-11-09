use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn error(context: &str) -> io::Error {
    let source = io::Error::last_os_error();
    io::Error::new(source.kind(), format!("{context}: {source}"))
}

/// Windows mutexes are session-wide; windows themselves belong to a desktop.
/// Scope the singleton to the desktop so a background/test desktop cannot block Explorer.
pub fn desktop_instance_name() -> io::Result<String> {
    use windows_sys::Win32::System::{StationsAndDesktops::*, Threading::GetCurrentThreadId};
    fn object_name(handle: HANDLE) -> io::Result<String> {
        let mut name = [0u16; 256];
        let mut needed = 0;
        // SAFETY: borrowed station/desktop handle, correctly sized writable UTF-16 buffer.
        if unsafe {
            GetUserObjectInformationW(
                handle,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                std::mem::size_of_val(&name) as u32,
                &mut needed,
            )
        } == 0
        {
            return Err(error("Read window desktop name"));
        }
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        Ok(name[..end].iter().map(|c| format!("{c:04x}")).collect())
    }
    // SAFETY: these are borrowed handles; no ownership transfer and no close is required.
    unsafe {
        let station = object_name(GetProcessWindowStation())?;
        let desktop = object_name(GetThreadDesktop(GetCurrentThreadId()))?;
        Ok(instance_name_for(&station, &desktop))
    }
}

fn instance_name_for(station: &str, desktop: &str) -> String {
    // Fixed FNV-1a, not a randomized hasher: separate processes must agree.
    // Keep below MAX_PATH even for long sandbox desktop/window-station names.
    let hash = station
        .bytes()
        .chain([b'/'])
        .chain(desktop.bytes())
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    format!("Local\\BatterySentry.SingleInstance.{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn singleton_names_are_short_stable_and_desktop_specific() {
        let long = "desktop".repeat(200);
        let name = instance_name_for("WinSta0", &long);
        assert!(name.len() < 260);
        assert_eq!(name, instance_name_for("WinSta0", &long));
        assert_ne!(name, instance_name_for("WinSta0", "Default"));
        assert_ne!(name, instance_name_for("OtherStation", &long));
    }
}
#[derive(Debug)]
pub struct Handle(HANDLE);
impl Handle {
    pub fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(error("Open Windows handle"))
        } else {
            Ok(Self(raw))
        }
    }
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns a valid kernel handle.
        unsafe { CloseHandle(self.0) };
    }
}
