//! Native rendering lifecycle checks, without screenshots or performance sampling.
use super::*;
use crate::win::wide;
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources},
    },
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
                3 => Some(BatteryValue::PercentCharging(65)),
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

        paint(&mut renderer, hwnd, &panel, &readings(3), 96);

        ShowWindow(hwnd, SW_MINIMIZE);
        renderer.release_target();

        ShowWindow(hwnd, SW_HIDE);
        renderer.release_target();

        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        ShowWindow(hwnd, SW_RESTORE);
        let gdi_before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        let user_before = GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS);
        paint(&mut renderer, hwnd, &panel, &two, 96);
        for _ in 0..3 {
            ShowWindow(hwnd, SW_HIDE);
            renderer.release_target();
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            paint(&mut renderer, hwnd, &panel, &two, 96);
        }

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
        panel.show_page(Page::Settings);
        for dpi in [96, 120, 144, 192] {
            paint(&mut renderer, hwnd, &panel, &two, dpi);
        }
        drop(renderer);

        drop(window);

        assert_ne!(UnregisterClassW(class.as_ptr(), instance), 0);
    }
}
