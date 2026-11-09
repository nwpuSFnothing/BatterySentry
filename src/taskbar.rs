//! Read-only taskbar discovery. Never resizes Explorer or another application's window.
use crate::win::wide;
use std::{
    io, ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};
use windows::Win32::{
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
    UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomation2, TreeScope_Descendants},
};
use windows::core::Interface;
use windows_sys::Win32::{
    Foundation::*,
    UI::{Accessibility::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

pub const WM_LAYOUT: u32 = WM_APP + 10;
pub const WM_LAYOUT_DIRTY: u32 = WM_APP + 11;
pub const WIDGET_CLASS: &str = "BatterySentry.Taskbar";
static HOST: AtomicUsize = AtomicUsize::new(0);
static DIRTY: AtomicBool = AtomicBool::new(false);
static FULLSCREEN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub left: i32,
    pub right: i32,
}

/// Chooses the rightmost sufficiently wide gap. Occupied spans include padding.
pub fn rightmost_gap(
    bar: Span,
    occupied: &[Span],
    preferred: i32,
    minimum: i32,
    pad: i32,
) -> Option<Span> {
    let mut spans: Vec<_> = occupied
        .iter()
        .map(|s| Span {
            left: (s.left - pad).max(bar.left),
            right: (s.right + pad).min(bar.right),
        })
        .filter(|s| s.right > s.left)
        .collect();
    spans.sort_by_key(|s| s.left);
    let mut gaps = Vec::new();
    let mut cursor = bar.left + pad;
    for s in spans {
        if s.left > cursor {
            gaps.push(Span {
                left: cursor,
                right: s.left,
            });
        }
        cursor = cursor.max(s.right);
    }
    if cursor < bar.right - pad {
        gaps.push(Span {
            left: cursor,
            right: bar.right - pad,
        });
    }
    gaps.into_iter()
        .rev()
        .find(|g| g.right - g.left >= minimum)
        .map(|g| Span {
            left: (g.right - preferred).max(g.left),
            right: g.right,
        })
}

pub struct Layout {
    pub generation: u64,
    pub provisional: bool,
    pub occupied: Vec<Span>,
    pub parent: usize,
    pub rect: Option<RECT>,
    pub dpi: u32,
    pub reason: String,
}
struct NativeScan {
    bar: RECT,
    explorer: u32,
    own: u32,
    occupied: Vec<Span>,
}
fn intersects(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
}
unsafe extern "system" fn collect_native(hwnd: HWND, context: LPARAM) -> i32 {
    // SAFETY: Enum callbacks are synchronous; context points to its live scan record.
    unsafe {
        let s = &mut *(context as *mut NativeScan);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == s.own {
            return 1;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r) == 0
            || !intersects(&s.bar, &r)
            || r.bottom - r.top > (s.bar.bottom - s.bar.top) * 2
        {
            return 1;
        }
        let mut class = [0u16; 128];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), 128);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        // Foreign children include LiteMonitor and other taskbar widgets. Shell
        // containers are not occupied in their entirety: UIA measures their buttons.
        if pid != s.explorer
            || matches!(
                class.as_str(),
                "TrayNotifyWnd" | "Start" | "TrayClockWClass" | "TrayShowDesktopButtonWClass"
            )
        {
            s.occupied.push(Span {
                left: r.left,
                right: r.right,
            });
        }
        1
    }
}

fn snapshot(automation: Option<&IUIAutomation>) -> Layout {
    // SAFETY: read-only enumeration with bounded stack buffers and a synchronous
    // callback. COM interfaces are used on their dedicated initialized thread.
    unsafe {
        let bar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), ptr::null());
        let mut bounds = RECT::default();
        if bar.is_null() || GetWindowRect(bar, &mut bounds) == 0 || IsWindowVisible(bar) == 0 {
            return Layout {
                generation: 0,
                provisional: false,
                occupied: Vec::new(),
                parent: 0,
                rect: None,
                dpi: 96,
                reason: "Waiting for Windows taskbar".into(),
            };
        }
        let dpi = GetDpiForWindow(bar).max(96);
        let scale = |n| n * dpi as i32 / 96;
        if bounds.bottom - bounds.top < scale(24)
            || bounds.right - bounds.left < bounds.bottom - bounds.top
        {
            return Layout {
                generation: 0,
                provisional: false,
                occupied: Vec::new(),
                parent: bar as usize,
                rect: None,
                dpi,
                reason: "Taskbar is hidden or vertical".into(),
            };
        }
        let mut explorer = 0;
        GetWindowThreadProcessId(bar, &mut explorer);
        let foreground = GetForegroundWindow();
        let mut fg = RECT::default();
        let mut fg_pid = 0;
        GetWindowThreadProcessId(foreground, &mut fg_pid);
        if fg_pid != explorer
            && fg_pid != std::process::id()
            && GetWindowRect(foreground, &mut fg) != 0
            && fg.left <= bounds.left
            && fg.right >= bounds.right
            && fg.top <= bounds.top
            && fg.bottom >= bounds.bottom
        {
            return Layout {
                generation: 0,
                provisional: false,
                occupied: Vec::new(),
                parent: bar as usize,
                rect: None,
                dpi,
                reason: "Taskbar display hidden for fullscreen application".into(),
            };
        }
        let mut native = NativeScan {
            bar: bounds,
            explorer,
            own: std::process::id(),
            occupied: Vec::new(),
        };
        EnumChildWindows(
            bar,
            Some(collect_native),
            (&mut native as *mut NativeScan) as isize,
        );
        // Also catches third-party taskbar-aligned top-level windows.
        EnumWindows(
            Some(collect_native),
            (&mut native as *mut NativeScan) as isize,
        );
        debug_layout("native enumeration complete");
        let mut uia_ok = false;
        if let Some(automation) = automation
            && let Ok(root) = automation.ElementFromHandle(windows::Win32::Foundation::HWND(bar))
            && let Ok(condition) = automation.CreateTrueCondition()
            && let Ok(elements) = root.FindAll(TreeScope_Descendants, &condition)
            && let Ok(count) = elements.Length()
        {
            for i in 0..count.min(512) {
                let Ok(e) = elements.GetElement(i) else {
                    continue;
                };
                if e.CurrentProcessId().ok() == Some(std::process::id() as i32)
                    || e.CurrentIsOffscreen().map(|v| v.as_bool()).unwrap_or(true)
                {
                    continue;
                }
                let kind = e.CurrentControlType().map(|v| v.0).unwrap_or(0);
                // Buttons, images, list/menu/tab items and text; exclude large
                // containers and blank XAML hosts so their gaps remain usable.
                if !matches!(kind, 50000 | 50006 | 50007 | 50011 | 50019 | 50020 | 50023) {
                    continue;
                }
                if let Ok(r) = e.CurrentBoundingRectangle() {
                    let r = RECT {
                        left: r.left,
                        top: r.top,
                        right: r.right,
                        bottom: r.bottom,
                    };
                    if intersects(&bounds, &r) && r.right > r.left {
                        native.occupied.push(Span {
                            left: r.left,
                            right: r.right,
                        });
                        uia_ok = true;
                    }
                }
            }
        }
        debug_layout("UIA enumeration complete");
        let trusted_occupied = native.occupied.clone();
        let mut provisional = false;
        if !uia_ok {
            // Conservatively reserve the complete task-list region when UIA is
            // unavailable. Hiding is preferable to covering application buttons.
            for class in [
                "MSTaskSwWClass",
                "Windows.UI.Composition.DesktopWindowContentBridge",
            ] {
                let list = FindWindowExW(bar, ptr::null_mut(), wide(class).as_ptr(), ptr::null());
                let mut r = RECT::default();
                if !list.is_null() && GetWindowRect(list, &mut r) != 0 {
                    provisional = true;
                    native.occupied.push(Span {
                        left: r.left,
                        right: r.right,
                    });
                }
            }
        }
        if std::env::var_os("BATTERYSENTRY_LAYOUT_DEBUG").is_some() {
            eprintln!("occupied: {:?}", native.occupied);
        }
        let gap = rightmost_gap(
            Span {
                left: bounds.left,
                right: bounds.right,
            },
            &native.occupied,
            scale(200),
            scale(150),
            0,
        );
        let rect = gap.map(|g| RECT {
            left: g.left - bounds.left,
            top: scale(3),
            right: g.right - bounds.left,
            bottom: (bounds.bottom - bounds.top - scale(3)).min(scale(48)),
        });
        Layout {
            generation: 0,
            provisional,
            occupied: trusted_occupied,
            parent: bar as usize,
            rect,
            dpi,
            reason: if rect.is_some() {
                String::new()
            } else {
                "No free taskbar space; existing items preserved".into()
            },
        }
    }
}

unsafe extern "system" fn changed(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    // SAFETY: callback only inspects window geometry and posts a pointer-free
    // notification. Atomic coalescing bounds messages during Explorer layout bursts.
    unsafe {
        let host = HOST.load(Ordering::Relaxed);
        if host == 0 || hwnd.is_null() {
            return;
        }
        let bar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), ptr::null());
        let mut b = RECT::default();
        let mut r = RECT::default();
        let relevant = if event == EVENT_SYSTEM_FOREGROUND {
            let covers = GetAncestor(hwnd, GA_ROOT) != bar
                && GetWindowRect(bar, &mut b) != 0
                && GetWindowRect(hwnd, &mut r) != 0
                && r.left <= b.left
                && r.right >= b.right
                && r.top <= b.top
                && r.bottom >= b.bottom;
            FULLSCREEN.swap(covers, Ordering::Relaxed) != covers
        } else {
            // Focus, selection and caret notifications do not change placement.
            if !matches!(
                event,
                EVENT_OBJECT_CREATE
                    | EVENT_OBJECT_DESTROY
                    | EVENT_OBJECT_SHOW
                    | EVENT_OBJECT_HIDE
                    | EVENT_OBJECT_LOCATIONCHANGE
            ) || !matches!(object, 0 | -4)
            {
                return;
            }
            bar.is_null()
                || GetAncestor(hwnd, GA_ROOT) == bar
                || (GetWindowRect(bar, &mut b) != 0
                    && GetWindowRect(hwnd, &mut r) != 0
                    && intersects(&b, &r))
        };
        if relevant && !DIRTY.swap(true, Ordering::Relaxed) {
            PostMessageW(host as HWND, WM_LAYOUT_DIRTY, 0, 0);
        }
    }
}

pub struct Tracker {
    tx: Sender<u64>,
    pub results: Receiver<Layout>,
    join: Option<JoinHandle<()>>,
    hook: HWINEVENTHOOK,
    foreground_hook: HWINEVENTHOOK,
    generation: AtomicU64,
}
impl Tracker {
    pub fn new(host: HWND) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (out, results) = mpsc::channel();
        let address = host as usize;
        // Keep native geometry responsive even if a UIA provider stalls. UIA
        // owns its own COM apartment and publishes a refined layout separately.
        let (uia_tx, uia_rx) = mpsc::channel::<u64>();
        if std::env::var_os("BATTERYSENTRY_LAYOUT_NO_UIA").is_none() {
            let refined = out.clone();
            let _ = thread::Builder::new()
                .name("taskbar-accessibility".into())
                .spawn(move || {
                    // SAFETY: every COM interface stays on this initialized MTA thread.
                    unsafe {
                        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                            return;
                        }
                        let automation: Option<IUIAutomation> =
                            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
                        if let Some(ref a) = automation
                            && let Ok(a2) = a.cast::<IUIAutomation2>()
                        {
                            let _ = a2.SetConnectionTimeout(500);
                            let _ = a2.SetTransactionTimeout(500);
                        }
                        while let Ok(mut generation) = uia_rx.recv() {
                            // Discard superseded requests after a slow provider call.
                            for newer in uia_rx.try_iter() {
                                generation = newer;
                            }
                            if generation == 0 {
                                break;
                            }
                            if automation.is_none() {
                                break;
                            }
                            let mut layout = snapshot(automation.as_ref());
                            layout.generation = generation;
                            if refined.send(layout).is_err() {
                                break;
                            }
                            PostMessageW(address as HWND, WM_LAYOUT, 0, 0);
                        }
                        drop(automation);
                        CoUninitialize();
                    }
                });
        }
        let join = thread::Builder::new()
            .name("taskbar-layout".into())
            .spawn(move || {
                while let Ok(generation) = rx.recv() {
                    if generation == 0 {
                        break;
                    }
                    let mut layout = snapshot(None);
                    layout.generation = generation;
                    if out.send(layout).is_err() {
                        break;
                    }
                    // SAFETY: pointer-free notification; no App references cross threads.
                    unsafe {
                        PostMessageW(address as HWND, WM_LAYOUT, 0, 0);
                    }
                    let _ = uia_tx.send(generation);
                }
            })?;
        HOST.store(address, Ordering::Relaxed);
        // SAFETY: out-of-context hook, delivered on the UI thread, no injected DLL.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_CREATE,
                EVENT_OBJECT_LOCATIONCHANGE,
                ptr::null_mut(),
                Some(changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        Ok(Self {
            tx,
            results,
            join: Some(join),
            hook,
            // SAFETY: same out-of-context, non-injecting callback as the layout hook.
            foreground_hook: unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    ptr::null_mut(),
                    Some(changed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            },
            generation: AtomicU64::new(0),
        })
    }
    pub fn request(&self) {
        DIRTY.store(false, Ordering::Relaxed);
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(generation);
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }
    pub fn shutdown(mut self) {
        let _ = self.tx.send(0);
        if let Some(join) = self.join.take() {
            // UIA providers may be unresponsive during Explorer shutdown. No App
            // references cross this thread boundary; process exit reclaims it.
            if join.is_finished() {
                let _ = join.join();
            }
        }
    }
}
impl Drop for Tracker {
    fn drop(&mut self) {
        HOST.store(0, Ordering::Relaxed);
        let _ = self.tx.send(0);
        // SAFETY: this tracker owns the hook; callback contains no borrowed data.
        unsafe {
            if !self.foreground_hook.is_null() {
                UnhookWinEvent(self.foreground_hook);
            }
            if !self.hook.is_null() {
                UnhookWinEvent(self.hook);
            }
        }
    }
}

fn debug_layout(message: &str) {
    if std::env::var_os("BATTERYSENTRY_LAYOUT_DEBUG").is_some() {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn places_left_of_litemonitor_and_tray() {
        let occupied = [
            Span {
                left: 0,
                right: 200,
            },
            Span {
                left: 650,
                right: 1120,
            },
            Span {
                left: 1600,
                right: 1740,
            },
            Span {
                left: 1740,
                right: 1920,
            },
        ];
        assert_eq!(
            rightmost_gap(
                Span {
                    left: 0,
                    right: 1920
                },
                &occupied,
                230,
                150,
                6
            ),
            Some(Span {
                left: 1364,
                right: 1594
            })
        );
    }
    #[test]
    fn chooses_next_gap_and_never_overlaps_when_crowded() {
        let b = Span {
            left: -1920,
            right: 0,
        };
        let occupied = [
            Span {
                left: -500,
                right: 0,
            },
            Span {
                left: -1800,
                right: -650,
            },
        ];
        assert_eq!(rightmost_gap(b, &occupied, 230, 150, 6), None);
        assert_eq!(
            rightmost_gap(
                Span {
                    left: 0,
                    right: 1000
                },
                &[Span {
                    left: 400,
                    right: 1000
                }],
                230,
                150,
                6
            ),
            Some(Span {
                left: 164,
                right: 394
            })
        );
    }
}
