//! Opt-in native interaction check; never touches startup settings or real devices.
use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct TestWindow(HWND);
impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: test owns the window; App storage outlives this guard.
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

fn point(x: i32, y: i32) -> isize {
    ((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as isize
}

fn pump() {
    let deadline = Instant::now() + Duration::from_millis(170);
    while Instant::now() < deadline {
        // SAFETY: messages are processed on the owning test thread.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                // The fixture injects input explicitly; ignore unrelated desktop input.
                if matches!(msg.message, WM_MOUSEMOVE | WM_MOUSELEAVE | WM_ACTIVATE) {
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        thread::sleep(Duration::from_millis(8));
    }
}

fn click(hwnd: HWND, b: Bounds) {
    // SAFETY: no pointer payload; test window uses 96 DPI logical coordinates.
    unsafe {
        let p = point(b.x + b.w / 2, b.y + b.h / 2);
        SendMessageW(hwnd, WM_MOUSEMOVE, 0, p);
        SendMessageW(hwnd, WM_LBUTTONDOWN, 1, p);
        SendMessageW(hwnd, WM_LBUTTONUP, 0, p);
    }
}

fn snapshot(hwnd: HWND, path: &Path) {
    // SAFETY: bitmap is deselected before GetDIBits/deletion; all DCs are released.
    unsafe {
        UpdateWindow(hwnd);
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect);
        let screen = GetDC(hwnd);
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, rect.right, rect.bottom);
        let old = SelectObject(dc, bitmap);
        BitBlt(dc, 0, 0, rect.right, rect.bottom, screen, 0, 0, SRCCOPY);
        SelectObject(dc, old);
        let stride = (rect.right as usize * 3).div_ceil(4) * 4;
        let mut pixels = vec![0u8; stride * rect.bottom as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: rect.right,
                biHeight: -rect.bottom,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            rect.bottom as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        );
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(hwnd, screen);
        assert_eq!(lines, rect.bottom, "screenshot capture failed");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(54u32 + pixels.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&54u32.to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&rect.right.to_le_bytes());
        bytes.extend_from_slice(&(-rect.bottom).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&24u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes.extend_from_slice(&pixels);
        fs::write(path, bytes).expect("write native panel screenshot");
    }
}

#[test]
#[ignore = "requires LOCALAPPDATA under artifacts/ui-check; opens one temporary native window"]
fn native_resize_hover_dropdown_and_virtual_scroll() {
    let root = std::env::current_dir()
        .expect("crate directory")
        .join("artifacts/ui-check");
    let runtime = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("isolated LOCALAPPDATA"));
    assert!(
        runtime.starts_with(&root),
        "run with LOCALAPPDATA under artifacts/ui-check"
    );
    fs::create_dir_all(&root).expect("create check directory");
    let mut initial = App::new();
    initial.dpi = 96;
    initial.cfg = Config::default();
    initial.ui = Panel::new(460, 530, false);
    initial.layout_status.clear();
    initial.readings = (0..8)
        .map(|i| Reading {
            device_key: format!("fixture-{i}"),
            last_known: None,
            name: if i == 0 {
                "ATK A9 Plus NK — Long device name for resize alignment".into()
            } else if i == 1 {
                "AULA Device".into()
            } else {
                format!("ATK Device {}", i + 1)
            },
            identity: String::new(),
            value: Some(BatteryValue::Percent(83 - i * 9)),
            detail: "Connected via 2.4 GHz".into(),
        })
        .collect();
    let app = Box::new(RefCell::new(initial));
    app.borrow_mut().context = &*app;
    // SAFETY: registered class and App storage live until the test HWND is destroyed.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hinst = GetModuleHandleW(ptr::null());
        let class_name = wide("BatterySentry.PanelCheck");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(procedure),
            hInstance: hinst,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        assert_ne!(RegisterClassExW(&wc), 0, "register check class");
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide("BatterySentry.PanelCheck").as_ptr(),
            wide("BatterySentry panel check").as_ptr(),
            WS_POPUP | WS_THICKFRAME,
            50,
            50,
            460,
            530,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            (&*app as *const RefCell<App>).cast(),
        );
        assert!(!hwnd.is_null(), "create native check window");
        let _window = TestWindow(hwnd);
        app.borrow_mut().panel = hwnd;
        app.borrow_mut().host = hwnd;
        app.borrow_mut().tray = Some(tray::TrayIcon::new(hwnd));
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        pump();
        snapshot(hwnd, &root.join("default.bmp"));
        assert_eq!(app.borrow().ui.visible(8), 0..3);
        let mut wr = RECT::default();
        GetWindowRect(hwnd, &mut wr);
        assert_eq!(
            SendMessageW(hwnd, WM_NCHITTEST, 0, point(wr.left + 1, wr.top + 1)),
            HTTOPLEFT as isize
        );
        SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(440, 20));
        pump();
        let dc = GetDC(hwnd);
        let red = GetPixel(dc, 428, 10);
        ReleaseDC(hwnd, dc);
        assert!(
            (red & 255) > 150 && ((red >> 8) & 255) < 90,
            "close hover must be red: pixel={red:06x}, state={:?}",
            app.borrow().ui.hover
        );
        snapshot(hwnd, &root.join("hover.bmp"));
        SetWindowPos(
            hwnd,
            ptr::null_mut(),
            0,
            0,
            900,
            690,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        pump();
        assert_eq!(app.borrow().ui.width, 900);
        assert_eq!(app.borrow().ui.visible(8), 0..5);
        snapshot(hwnd, &root.join("wide.bmp"));
        let mut p = POINT { x: 400, y: 200 };
        ClientToScreen(hwnd, &mut p);
        SendMessageW(
            hwnd,
            WM_MOUSEWHEEL,
            (-240i16 as u16 as usize) << 16,
            point(p.x, p.y),
        );
        assert!(app.borrow().ui.scroll > 0);
        let interval = app.borrow().ui.bounds(Control::Interval);
        click(hwnd, interval);
        pump();
        assert!(app.borrow().ui.dropdown);
        snapshot(hwnd, &root.join("wide-dropdown.bmp"));
        let option = app.borrow().ui.bounds(Control::Option(4));
        click(hwnd, option);
        assert_eq!(app.borrow().cfg.interval, 600);
        assert!(!app.borrow().ui.dropdown);
        SendMessageW(hwnd, WM_MOUSEMOVE, 0, point(100, 250));
        pump();
        snapshot(hwnd, &root.join("wide-scrolled.bmp"));
        SendMessageW(hwnd, WM_KEYDOWN, VK_END as usize, 0);
        assert_eq!(app.borrow().ui.scroll, app.borrow().ui.max_scroll(8));
        app.borrow_mut().readings.truncate(1);
        app.borrow_mut().update_panel_size();
        assert_eq!(app.borrow().ui.scroll, 0);
        let close = app.borrow().ui.bounds(Control::Close);
        click(hwnd, close);
        assert_eq!(IsWindowVisible(hwnd), 0);
        let saved = fs::read_to_string(runtime.join("BatterySentry/settings.ini"))
            .expect("isolated settings");
        assert!(saved.contains("window_width=900"));
        assert!(saved.contains("window_height=690"));
        // The off transition only deletes our own icon and never needs Explorer.
        app.borrow_mut().open_panel();
        assert_ne!(
            IsWindowVisible(hwnd),
            0,
            "explicit open must restore a hidden panel"
        );
        let tray_toggle = app.borrow().ui.bounds(Control::Tray);
        click(hwnd, tray_toggle);
        assert!(!app.borrow().cfg.show_tray_icon);
        let saved = fs::read_to_string(runtime.join("BatterySentry/settings.ini"))
            .expect("isolated tray preference");
        assert!(saved.contains("show_tray_icon=0"));
    }
}
