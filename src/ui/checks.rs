//! Controlled native-window rendering checks and process metrics, using synthetic readings.
use super::*;
use crate::win::wide;
use std::{fs, mem::size_of, ptr, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, ProcessStatus::*, Threading::*},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the fixture owns the window and renderer has already been dropped.
        unsafe { DestroyWindow(self.0) };
    }
}
fn readings(count: usize) -> Vec<Reading> {
    (0..count)
        .map(|i| Reading {
            device_key: format!("fixture-{i}"),
            identity: String::new(),
            name: if i == 0 {
                "ATK A9 Plus NK".into()
            } else {
                "AULA F87 Pro".into()
            },
            value: match i {
                1 => Some(BatteryValue::Charging),
                2 => None,
                _ => Some(BatteryValue::Percent(83)),
            },
            last_known: (i == 2).then_some(BatteryValue::Percent(65)),
            detail: if i == 2 {
                "Last read 32s ago".into()
            } else {
                "Connected via 2.4 GHz".into()
            },
        })
        .collect()
}
fn cpu_time() -> u64 {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: current process handle is borrowed and all four output values are writable.
    unsafe {
        assert_ne!(
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user
            ),
            0
        );
    }
    let value = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    value(kernel) + value(user)
}
thread_local! {
    static PREVIOUS_SAMPLE: std::cell::Cell<Option<(u64, Instant)>> = const { std::cell::Cell::new(None) };
}
fn sample(phase: &str) {
    let (cpu, start) = PREVIOUS_SAMPLE.with(|previous| {
        previous
            .get()
            .unwrap_or_else(|| (cpu_time(), Instant::now()))
    });
    thread::sleep(Duration::from_millis(250));
    let now = Instant::now();
    let current_cpu = cpu_time();
    let percent =
        (current_cpu - cpu) as f64 / 10_000_000.0 / now.duration_since(start).as_secs_f64() * 100.0;
    PREVIOUS_SAMPLE.with(|previous| previous.set(Some((current_cpu, now))));
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: extended process counter buffer has its exact byte size and current process access.
    unsafe {
        assert_ne!(
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                memory.cb
            ),
            0
        );
        eprintln!(
            "METRIC {phase} WS={} PRIVATE={} CPU_ONE_CORE={percent:.2}% GDI={} USER={}",
            memory.WorkingSetSize,
            memory.PrivateUsage,
            GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS),
            GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS)
        );
    }
}
fn suspend(renderer: &mut renderer::Renderer) {
    renderer.release_target();
}
fn paint(
    renderer: &mut renderer::Renderer,
    hwnd: HWND,
    panel: &Panel,
    devices: &[Reading],
    dpi: u32,
) {
    let config = Config::default();
    let view = View {
        ui: panel,
        readings: devices,
        cfg: &config,
        busy: false,
        last: None,
        scan_error: "",
        layout_status: "",
    };
    // SAFETY: mark the owned test window dirty before explicitly serving its paint.
    unsafe {
        InvalidateRect(hwnd, ptr::null(), 0);
    }
    renderer
        .paint(hwnd, dpi, |c| view.paint(c))
        .expect("paint native fixture");
}
#[test]
fn native_rendering_lifecycle() {
    let root = std::env::current_dir()
        .unwrap()
        .join("artifacts/renderer-check");
    fs::create_dir_all(&root).unwrap();
    let prefix = std::env::var("BATTERYSENTRY_RENDER_LABEL").unwrap_or_else(|_| "d2d".into());
    // SAFETY: all fixture windows, resources and message operations belong to this test thread.
    unsafe {
        SetThreadDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let instance = GetModuleHandleW(ptr::null());
        let class = wide("BatterySentry.RendererCheck");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            hInstance: instance,
            lpfnWndProc: Some(DefWindowProcW),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        assert_ne!(RegisterClassExW(&wc), 0);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            40,
            40,
            420,
            340,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        assert!(!hwnd.is_null());
        let window = Window(hwnd);
        let mut renderer = renderer::Renderer::default();
        sample("startup-idle");
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        let mut panel = Panel::new(420, 340, false);
        let two = readings(2);
        paint(&mut renderer, hwnd, &panel, &two, 96);
        sample("devices");
        panel.show_page(Page::Settings);
        panel.height = layout::preferred_height(Page::Settings, 2);
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            420,
            panel.height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        paint(&mut renderer, hwnd, &panel, &two, 96);
        sample("settings");
        panel.show_page(Page::Devices);
        panel.width = 600;
        panel.height = 460;
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            600,
            460,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        paint(&mut renderer, hwnd, &panel, &two, 96);
        sample("resize");
        paint(&mut renderer, hwnd, &panel, &readings(3), 96);
        sample("refresh-fixture");
        ShowWindow(hwnd, SW_MINIMIZE);
        suspend(&mut renderer);
        sample("minimized");
        ShowWindow(hwnd, SW_HIDE);
        suspend(&mut renderer);
        sample("hidden");
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        ShowWindow(hwnd, SW_RESTORE);
        for _ in 0..12 {
            paint(&mut renderer, hwnd, &panel, &two, 96);
            ShowWindow(hwnd, SW_HIDE);
            suspend(&mut renderer);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        paint(&mut renderer, hwnd, &panel, &two, 96);
        sample("reopened-12");
        let gdi_before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        let user_before = GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS);
        paint(&mut renderer, hwnd, &panel, &two, 96);
        for _ in 0..24 {
            ShowWindow(hwnd, SW_HIDE);
            suspend(&mut renderer);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            paint(&mut renderer, hwnd, &panel, &two, 96);
        }
        sample("reopened-36");
        assert!(GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) <= gdi_before + 1);
        assert!(GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS) <= user_before + 1);
        renderer.check_device_loss();
        paint(&mut renderer, hwnd, &panel, &two, 96);
        for dpi in [96, 120, 144, 192] {
            for count in [0, 1, 2, 8] {
                let height = layout::preferred_height(Page::Devices, count);
                panel = Panel::new(420, height, false);
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    40,
                    40,
                    420 * dpi as i32 / 96,
                    height * dpi as i32 / 96,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                paint(&mut renderer, hwnd, &panel, &readings(count), dpi);
            }
        }
        drop(renderer);
        sample("renderer-released");
        drop(window);
        sample("window-destroyed");
        let widget_parent = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            240,
            48,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        assert!(!widget_parent.is_null());
        let _parent = Window(widget_parent);
        let child = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            200,
            42,
            widget_parent,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        assert!(!child.is_null());
        let _child = Window(child);
        assert_ne!(
            SetLayeredWindowAttributes(child, rgb(210, 210, 211), 255, LWA_COLORKEY),
            0
        );
        let mut widget = widget::WidgetRenderer::default();
        InvalidateRect(child, ptr::null(), 0);
        widget
            .paint(child, 96, |c| {
                widget::paint_widget(c, &two, THEME, rgb(210, 210, 211))
            })
            .unwrap();
        drop(widget);
        drop(_child);
        drop(_parent);
        for dpi in [96, 120, 144, 192] {
            for count in [0, 1, 2, 8] {
                let panel = Panel::new(420, layout::preferred_height(Page::Devices, count), false);
                let devices = readings(count);
                let view = View {
                    ui: &panel,
                    readings: &devices,
                    cfg: &Config::default(),
                    busy: false,
                    last: None,
                    scan_error: "",
                    layout_status: "",
                };
                renderer::write_snapshot(
                    &root.join(format!("{prefix}-{count}-{dpi}.bmp")),
                    &panel,
                    dpi,
                    false,
                    |c| view.paint(c),
                )
                .unwrap();
            }
            let mut panel = Panel::new(420, layout::preferred_height(Page::Settings, 2), false);
            panel.show_page(Page::Settings);
            let devices = readings(2);
            let config = Config::default();
            let view = View {
                ui: &panel,
                readings: &devices,
                cfg: &config,
                busy: false,
                last: None,
                scan_error: "",
                layout_status: "",
            };
            renderer::write_snapshot(
                &root.join(format!("{prefix}-settings-{dpi}.bmp")),
                &panel,
                dpi,
                false,
                |c| view.paint(c),
            )
            .unwrap();
        }
        let panel = Panel::new(420, 340, false);
        let devices = readings(2);
        let config = Config::default();
        let view = View {
            ui: &panel,
            readings: &devices,
            cfg: &config,
            busy: false,
            last: None,
            scan_error: "",
            layout_status: "",
        };
        renderer::write_snapshot(
            &root.join(format!("{prefix}-grayscale.bmp")),
            &panel,
            120,
            true,
            |c| view.paint(c),
        )
        .unwrap();
    }
}
