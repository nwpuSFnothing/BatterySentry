//! Startup-folder shortcut. Ordinary state checks never repair user settings.
use std::{io, path::PathBuf, ptr};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize, IPersistFile,
        },
        UI::Shell::{
            FOLDERID_Startup, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
        },
    },
    core::{Interface, PCWSTR},
};
use windows_sys::Win32::System::Registry::*;
fn path() -> io::Result<PathBuf> {
    // SAFETY: known folder GUID; returned allocation is copied then freed.
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DEFAULT, None)
            .map_err(io::Error::other)?;
        let value = p.to_string().map_err(io::Error::other);
        CoTaskMemFree(Some(p.0.cast()));
        Ok(PathBuf::from(value?).join("BatterySentry.lnk"))
    }
}
pub fn enabled() -> bool {
    let Ok(path) = path() else {
        return false;
    };
    if !path.is_file() {
        return false;
    }
    // SAFETY: valid registry strings and byte output buffer; no writes.
    unsafe {
        let key = crate::win::wide(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder",
        );
        let name = crate::win::wide("BatterySentry.lnk");
        let mut data = [0u8; 12];
        let mut len = data.len() as u32;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_BINARY,
            ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut len,
        );
        status != 0 || len == 0 || !matches!(data[0], 3 | 7)
    }
}
struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: paired with successful CoInitializeEx on this thread.
        unsafe { CoUninitialize() };
    }
}
pub fn set_enabled(enable: bool) -> io::Result<()> {
    let path = path()?;
    if !enable {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    create_shortcut(&path, &std::env::current_exe()?)?;
    // SAFETY: explicit user enable clears the disabled record with a valid key handle.
    unsafe {
        let name = crate::win::wide(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder",
        );
        let mut key = ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, KEY_SET_VALUE, &mut key) == 0 {
            let status = RegDeleteValueW(key, crate::win::wide("BatterySentry.lnk").as_ptr());
            RegCloseKey(key);
            if status != 0 && status != 2 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
        }
    }
    if !enabled() {
        return Err(io::Error::other(
            "Windows still reports startup as disabled",
        ));
    }
    Ok(())
}

fn create_shortcut(path: &std::path::Path, exe: &std::path::Path) -> io::Result<()> {
    // SAFETY: COM interfaces used only on initialized thread; UTF-16 arguments
    // live through each call and windows bindings release owned COM references.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(io::Error::other)?;
        let _com = Com;
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io::Error::other)?;
        link.SetPath(PCWSTR(crate::win::wide(&exe.to_string_lossy()).as_ptr()))
            .map_err(io::Error::other)?;
        link.SetArguments(PCWSTR(crate::win::wide("--background").as_ptr()))
            .map_err(io::Error::other)?;
        if let Some(parent) = exe.parent() {
            link.SetWorkingDirectory(PCWSTR(crate::win::wide(&parent.to_string_lossy()).as_ptr()))
                .map_err(io::Error::other)?;
        }
        link.SetDescription(PCWSTR(
            crate::win::wide("BatterySentry - wireless peripheral batteries").as_ptr(),
        ))
        .map_err(io::Error::other)?;
        let persist: IPersistFile = link.cast().map_err(io::Error::other)?;
        persist
            .Save(
                PCWSTR(crate::win::wide(&path.to_string_lossy()).as_ptr()),
                true,
            )
            .map_err(io::Error::other)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_has_background_argument_and_correct_target() {
        let path =
            std::env::temp_dir().join(format!("BatterySentry-test-{}.lnk", std::process::id()));
        let exe = std::env::current_exe().expect("test executable");
        create_shortcut(&path, &exe).expect("create test shortcut");
        // SAFETY: COM initialized on this thread; shortcut is loaded into an owned
        // interface and copied into bounded output buffers before uninitializing.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .expect("COM initialization");
            let _com = Com;
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).expect("ShellLink");
            let persist: IPersistFile = link.cast().expect("IPersistFile");
            persist
                .Load(
                    PCWSTR(crate::win::wide(&path.to_string_lossy()).as_ptr()),
                    windows::Win32::System::Com::STGM_READ,
                )
                .expect("load shortcut");
            let mut args = [0u16; 256];
            link.GetArguments(&mut args).expect("shortcut arguments");
            let len = args.iter().position(|c| *c == 0).unwrap_or(args.len());
            assert_eq!(String::from_utf16_lossy(&args[..len]), "--background");
            let mut target = [0u16; 1024];
            link.GetPath(&mut target, ptr::null_mut(), 0)
                .expect("shortcut target");
            let len = target.iter().position(|c| *c == 0).unwrap_or(target.len());
            assert_eq!(PathBuf::from(String::from_utf16_lossy(&target[..len])), exe);
        }
        std::fs::remove_file(path).expect("remove test shortcut");
    }
}
