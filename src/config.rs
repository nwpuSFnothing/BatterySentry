use crate::theme::{DEFAULT_HEIGHT, DEFAULT_WIDTH, MIN_HEIGHT, MIN_WIDTH};
use std::{fs, io, path::PathBuf};
pub const INTERVALS: [u32; 6] = [30, 60, 120, 300, 600, 1800];
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub interval: u32,
    pub window_width: i32,
    pub window_height: i32,
    pub show_tray_icon: bool,
    pub window_size_manual: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            interval: 60,
            window_width: DEFAULT_WIDTH,
            window_height: DEFAULT_HEIGHT,
            show_tray_icon: true,
            window_size_manual: false,
        }
    }
}
fn path() -> io::Result<PathBuf> {
    let base =
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| io::Error::other("LOCALAPPDATA not set"))?;
    Ok(PathBuf::from(base)
        .join("BatterySentry")
        .join("settings.ini"))
}
impl Config {
    fn parse(text: &str) -> Self {
        let interval = text
            .lines()
            .find_map(|l| {
                l.strip_prefix("interval=")
                    .and_then(|v| v.trim().parse::<u32>().ok())
            })
            .filter(|n| INTERVALS.contains(n))
            .unwrap_or(60);
        let dimension = |key: &str, fallback, minimum| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.trim().parse::<i32>().ok())
                .filter(|n| (minimum..=4096).contains(n))
                .unwrap_or(fallback)
        };
        let width = dimension("window_width=", DEFAULT_WIDTH, MIN_WIDTH);
        let height = dimension("window_height=", DEFAULT_HEIGHT, MIN_HEIGHT);
        let legacy_default = width == 460 && matches!(height, 504 | 530);
        let window_size_manual = text
            .lines()
            .find_map(|line| line.strip_prefix("window_size_manual="))
            .map(|value| matches!(value.trim(), "1" | "true"))
            .unwrap_or(!legacy_default && (width != DEFAULT_WIDTH || height != DEFAULT_HEIGHT));
        Self {
            interval,
            window_width: if legacy_default && !window_size_manual {
                DEFAULT_WIDTH
            } else {
                width
            },
            window_height: if legacy_default && !window_size_manual {
                DEFAULT_HEIGHT
            } else {
                height
            },
            window_size_manual,
            show_tray_icon: text
                .lines()
                .find_map(|line| line.strip_prefix("show_tray_icon="))
                .map(str::trim)
                .is_none_or(|value| !matches!(value, "0" | "false")),
        }
    }
    pub fn load() -> Self {
        path()
            .ok()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }
    pub fn save(self) -> io::Result<()> {
        let path = path()?;
        fs::create_dir_all(path.parent().expect("config parent"))?;
        let temp = path.with_extension("tmp");
        fs::write(
            &temp,
            format!(
                "interval={}\nwindow_width={}\nwindow_height={}\nshow_tray_icon={}\nwindow_size_manual={}\n",
                self.interval,
                self.window_width,
                self.window_height,
                u8::from(self.show_tray_icon),
                u8::from(self.window_size_manual)
            ),
        )?;
        // SAFETY: terminated UTF-16 paths remain valid for the call.
        unsafe {
            if windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                crate::win::wide(&temp.to_string_lossy()).as_ptr(),
                crate::win::wide(&path.to_string_lossy()).as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(crate::win::error("Save settings"));
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_validate_intervals() {
        assert!(Config::parse("interval=60").show_tray_icon);
        assert!(!Config::parse("show_tray_icon=0").show_tray_icon);
        assert!(!Config::parse("show_tray_icon=false").show_tray_icon);
        for s in [
            "",
            "interval=0",
            "interval=-1",
            "interval=abc",
            "interval=4294967295",
        ] {
            assert_eq!(Config::parse(s).interval, 60);
        }
        for n in INTERVALS {
            assert_eq!(Config::parse(&format!("interval={n}\r\n")).interval, n);
        }
    }
}
