//! Native message-driven tray and lazily-created borderless panel, in one process.
use crate::{
    config::{Config, INTERVALS},
    drivers::{self, Reading},
    panel::{self, Control, Page, Panel},
    startup, taskbar,
    theme::{Theme, rgb},
    tray,
    ui::{self, renderer::Renderer, widget::WidgetRenderer},
    win::{Handle, desktop_instance_name, error, wide},
};
use std::{
    cell::RefCell,
    io,
    mem::size_of,
    ptr,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{Console::*, LibraryLoader::*, Threading::*},
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::{NIN_SELECT, NINF_KEY},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "BatterySentry.Host";
const PANEL_CLASS: &str = "BatterySentry.Panel";
const WM_RESULTS: u32 = WM_APP + 2;
const WM_OPEN: u32 = WM_APP + 3;
const WM_EXIT: u32 = WM_APP + 4;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

pub fn console() {
    // SAFETY: attaching to the parent console has no borrowed pointers.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
pub fn show_error(message: &str) {
    // SAFETY: both strings are terminated and live for this modal call.
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("BatterySentry").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

enum Work {
    Refresh,
    Rescan,
    Stop,
}
struct Worker {
    commands: Sender<Work>,
    results: Receiver<io::Result<Vec<Reading>>>,
    join: Option<JoinHandle<()>>,
}
impl Worker {
    fn new(hwnd: HWND) -> io::Result<Self> {
        let (commands, rx) = mpsc::channel();
        let (tx, results) = mpsc::channel();
        let address = hwnd as usize;
        let join = thread::Builder::new()
            .name("battery-reader".into())
            .spawn(move || {
                let mut cache = drivers::ReadingCache::default();
                let mut scanner = drivers::Scanner::default();
                while let Ok(work) = rx.recv() {
                    match work {
                        Work::Stop => break,
                        Work::Rescan => scanner.invalidate_identities(),
                        Work::Refresh => {}
                    }
                    let values = scanner.scan().map(|mut values| {
                        cache.apply(&mut values, Instant::now());
                        values
                    });
                    if tx.send(values).is_err() {
                        break;
                    }
                    // SAFETY: no pointer payload is sent. A destroyed HWND simply rejects it.
                    unsafe {
                        PostMessageW(address as HWND, WM_RESULTS, 0, 0);
                    }
                }
            })?;
        Ok(Self {
            commands,
            results,
            join: Some(join),
        })
    }
    fn shutdown(mut self) {
        let _ = self.commands.send(Work::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.commands.send(Work::Stop);
    }
}

struct App {
    context: *const RefCell<App>,
    host: HWND,
    panel: HWND,
    worker: Option<Worker>,
    readings: Vec<Reading>,
    cfg: Config,
    renderer: Renderer,
    widget_renderer: WidgetRenderer,
    widget: HWND,
    widget_parent: HWND,
    widget_key: u32,
    tracker: Option<taskbar::Tracker>,
    layout_busy: bool,
    layout_again: bool,
    layout_retry_pending: bool,
    layout_failures: u8,
    layout_intentionally_hidden: bool,
    widget_dpi: u32,
    layout_status: String,
    busy: bool,
    last: Option<Instant>,
    startup: bool,
    taskbar: u32,
    dpi: u32,
    ui: Panel,
    scan_error: String,
    rescan: bool,
    refresh_again: bool,
    tray: Option<tray::TrayIcon>,
}
impl App {
    fn layout_failed(&mut self) {
        if !self.layout_retry_pending {
            // Allow native discovery and UIA refinement to finish the same attempt.
            // Repeated window events must not postpone the retry deadline.
            // SAFETY: this UI thread owns the live host and its retry timer.
            unsafe { SetTimer(self.host, 4, 3000, None) };
            self.layout_retry_pending = true;
        }
        self.invalidate();
    }
    fn request_layout(&mut self) {
        if self.layout_busy {
            self.layout_again = true;
            return;
        }
        if !self.layout_busy
            && let Some(tracker) = &self.tracker
        {
            self.layout_busy = true;
            tracker.request();
        }
    }
    fn destroy_widget(&mut self) {
        // SAFETY: handle is owned by this UI thread. Explorer may already have
        // destroyed the child, so validate it before any destruction attempt.
        unsafe {
            if !self.widget.is_null() && IsWindow(self.widget) != 0 {
                DestroyWindow(self.widget);
            }
        }
        self.widget = ptr::null_mut();
    }
    fn invalidate_widget(&self) {
        // SAFETY: invalidation only; no pointer payload and no borrowed buffers.
        unsafe {
            if !self.widget.is_null() {
                InvalidateRect(self.widget, ptr::null(), 0);
            }
        }
    }
    fn apply_layout(&mut self) {
        let layout = self
            .tracker
            .as_ref()
            .and_then(|t| t.results.try_recv().ok());
        let Some(layout) = layout else {
            return;
        };
        if !self
            .tracker
            .as_ref()
            .is_some_and(|t| t.is_current(layout.generation))
        {
            return;
        }
        self.layout_busy = false;
        if self.layout_again {
            self.layout_again = false;
            self.request_layout();
        }
        self.layout_intentionally_hidden = layout.intentionally_hidden;
        if self.layout_intentionally_hidden {
            self.layout_failures = 0;
        }
        self.layout_status = layout.reason;
        let dpi_changed = self.widget_dpi != layout.dpi;
        self.widget_dpi = layout.dpi;
        // SAFETY: all HWND/GDI mutations run on the owning UI thread. Like
        // LiteMonitor, attach the non-activating window to Explorer and preserve
        // its style bits when converting WS_POPUP to WS_CHILD.
        unsafe {
            let parent = layout.parent as HWND;
            let mut bar = RECT::default();
            if !parent.is_null() {
                GetWindowRect(parent, &mut bar);
            }
            if layout.provisional
                && IsWindow(self.widget) != 0
                && IsWindowVisible(self.widget) != 0
                && self.widget_parent == parent
            {
                let mut current = RECT::default();
                GetWindowRect(self.widget, &mut current);
                if !layout
                    .occupied
                    .iter()
                    .any(|s| s.left < current.right && s.right > current.left)
                {
                    // A conservative native container bound is not a new layout.
                    // Keep the last verified UIA placement while refinement runs.
                    KillTimer(self.host, 4);
                    self.layout_retry_pending = false;
                    self.layout_failures = 0;
                    self.layout_status.clear();
                    return;
                }
            }
            let Some(r) = layout.rect.filter(|_| !parent.is_null()) else {
                if !self.widget.is_null() && IsWindowVisible(self.widget) != 0 {
                    ShowWindow(self.widget, SW_HIDE);
                }
                self.layout_failed();
                return;
            };
            let mut created = false;
            if IsWindow(self.widget) == 0 || self.widget_parent != parent {
                self.destroy_widget();
                self.widget = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                    wide(taskbar::WIDGET_CLASS).as_ptr(),
                    wide("BatterySentry Taskbar 0.2.3").as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    self.context.cast(),
                );
                if self.widget.is_null() {
                    self.layout_status = error("Create taskbar display").to_string();
                    self.layout_failed();
                    return;
                }
                let style = GetWindowLongPtrW(self.widget, GWL_STYLE) as u32;
                SetWindowLongPtrW(
                    self.widget,
                    GWL_STYLE,
                    ((style & !WS_POPUP) | WS_CHILD | WS_CLIPSIBLINGS) as isize,
                );
                SetParent(self.widget, parent);
                if GetParent(self.widget) != parent {
                    // Preserve a usable non-activating fallback if Windows blocks
                    // cross-process parenting (e.g. mixed integrity/DPI contexts).
                    SetWindowLongPtrW(self.widget, GWL_STYLE, style as isize);
                }
                if std::env::var_os("BATTERYSENTRY_LAYOUT_DEBUG").is_some() {
                    eprintln!("attached={}", GetParent(self.widget) == parent);
                }
                self.widget_parent = parent;
                created = true;
            }
            let key = taskbar_key();
            let theme_changed = key != self.widget_key;
            if created || theme_changed {
                if SetLayeredWindowAttributes(self.widget, key, 255, LWA_COLORKEY) == 0 {
                    self.layout_status = error("Set taskbar display transparency").to_string();
                    self.destroy_widget();
                    self.layout_failed();
                    return;
                }
                self.widget_key = key;
            }
            let child = GetParent(self.widget) == parent
                && GetWindowLongPtrW(self.widget, GWL_STYLE) as u32 & WS_CHILD != 0;
            let desired = RECT {
                left: r.left + bar.left,
                top: r.top + bar.top,
                right: r.right + bar.left,
                bottom: r.bottom + bar.top,
            };
            let mut current = RECT::default();
            GetWindowRect(self.widget, &mut current);
            let moved = (current.left, current.top, current.right, current.bottom)
                != (desired.left, desired.top, desired.right, desired.bottom);
            let hidden = IsWindowVisible(self.widget) == 0;
            let covered = child && GetTopWindow(parent) != self.widget;
            if (created || moved || hidden || covered)
                && SetWindowPos(
                    self.widget,
                    if child { HWND_TOP } else { HWND_TOPMOST },
                    if child { r.left } else { desired.left },
                    if child { r.top } else { desired.top },
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOACTIVATE,
                ) == 0
            {
                self.layout_status = error("Position taskbar display").to_string();
                self.destroy_widget();
                self.layout_failed();
                return;
            }
            if IsWindowVisible(self.widget) == 0 {
                ShowWindow(self.widget, SW_SHOWNOACTIVATE);
            }
            KillTimer(self.host, 4);
            self.layout_retry_pending = false;
            self.layout_failures = 0;
            if created || moved || dpi_changed || theme_changed {
                self.invalidate_widget();
            }
            self.invalidate();
        }
    }
    fn paint_widget(&mut self, hwnd: HWND) {
        let readings = &self.readings;
        let theme = if light_taskbar() {
            Theme::LIGHT
        } else {
            Theme::DARK
        };
        let key = taskbar_key();
        let _ = self.widget_renderer.paint(hwnd, self.widget_dpi, |c| {
            ui::widget::paint_widget(c, readings, theme, key);
        });
    }
    fn new() -> Self {
        let cfg = Config::load();
        let startup = startup::enabled();
        let mut ui = Panel::new(cfg.window_width, cfg.window_height, startup);
        ui.tray_target = cfg.show_tray_icon;
        ui.tray_switch = if cfg.show_tray_icon { 1.0 } else { 0.0 };
        Self {
            context: ptr::null(),
            host: ptr::null_mut(),
            panel: ptr::null_mut(),
            worker: None,
            readings: Vec::new(),
            cfg,
            renderer: Renderer::default(),
            widget_renderer: WidgetRenderer::default(),
            widget: ptr::null_mut(),
            widget_parent: ptr::null_mut(),
            widget_key: 0,
            tracker: None,
            layout_busy: false,
            layout_again: false,
            layout_retry_pending: false,
            layout_failures: 0,
            layout_intentionally_hidden: false,
            widget_dpi: 96,
            layout_status: "Waiting for taskbar".into(),
            busy: false,
            last: None,
            startup,
            taskbar: 0,
            dpi: 96,
            ui,
            scan_error: String::new(),
            rescan: false,
            refresh_again: false,
            tray: None,
        }
    }
    fn refresh(&mut self) {
        if self.busy {
            self.refresh_again |= self.rescan;
            return;
        }
        let command = if self.rescan {
            Work::Rescan
        } else {
            Work::Refresh
        };
        if let Some(worker) = &self.worker
            && worker.commands.send(command).is_ok()
        {
            self.rescan = false;
            self.busy = true;
            self.invalidate();
        }
    }
    fn invalidate(&self) {
        // SAFETY: invalidation only, and all window state belongs to the UI thread.
        unsafe {
            if !self.panel.is_null() {
                InvalidateRect(self.panel, ptr::null(), 0);
            }
        }
    }
    fn results(&mut self) {
        let result = self.worker.as_ref().and_then(|w| w.results.try_recv().ok());
        if let Some(result) = result {
            let first = (self.ui.scroll / panel::ROW_HEIGHT) as usize;
            let anchor = self.readings.get(first).map(|r| r.device_key.clone());
            match result {
                Ok(values) => {
                    self.readings = values;
                    self.scan_error.clear();
                    if let Some(key) = anchor
                        && let Some(index) = self.readings.iter().position(|r| r.device_key == key)
                    {
                        self.ui.scroll =
                            index as i32 * panel::ROW_HEIGHT + self.ui.scroll % panel::ROW_HEIGHT;
                    }
                }
                Err(e) => {
                    self.readings.clear();
                    self.scan_error = e.to_string();
                }
            }
            self.ui.clamp_scroll(self.readings.len());
            self.busy = false;
            self.last = Some(Instant::now());
            self.fit_panel();
            self.invalidate_widget();
            self.invalidate();
            if self.refresh_again {
                self.refresh_again = false;
                self.refresh();
            }
        }
    }
    fn update_panel_size(&mut self) {
        // SAFETY: panel is a live HWND, dimensions are only read on the UI thread.
        unsafe {
            let mut rect = RECT::default();
            if GetClientRect(self.panel, &mut rect) != 0 && rect.right > 0 && rect.bottom > 0 {
                self.ui.width = rect.right * 96 / self.dpi.max(1) as i32;
                self.ui.height = rect.bottom * 96 / self.dpi.max(1) as i32;
                self.ui.clamp_scroll(self.readings.len());
            }
        }
        self.invalidate();
    }
    fn show_page(&mut self, page: Page) {
        self.ui.show_page(page);
        self.fit_panel();
        self.invalidate();
    }
    fn fit_panel(&mut self) {
        if self.cfg.window_size_manual {
            return;
        }
        let count = if self.last.is_none() {
            2
        } else {
            self.readings.len()
        };
        let height = ui::layout::preferred_height(self.ui.page, count);
        if self.panel.is_null() {
            self.cfg.window_height = height;
            self.ui.height = height;
            return;
        }
        // SAFETY: geometry changes stay on the owning thread and within the current monitor.
        unsafe {
            if IsIconic(self.panel) != 0 {
                return;
            }
            let mut rect = RECT::default();
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetWindowRect(self.panel, &mut rect) == 0
                || GetMonitorInfoW(
                    MonitorFromWindow(self.panel, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                ) == 0
            {
                return;
            }
            let area = monitor.rcWork;
            let h = scale(height, self.dpi).min(area.bottom - area.top);
            if rect.bottom - rect.top != h {
                SetWindowPos(
                    self.panel,
                    ptr::null_mut(),
                    rect.left,
                    rect.top.min(area.bottom - h).max(area.top),
                    rect.right - rect.left,
                    h,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                self.update_panel_size();
            }
        }
        self.cfg.window_width = self.ui.width;
        self.cfg.window_height = self.ui.height;
    }
    fn save_panel_size(&mut self) {
        self.cfg.window_width = self.ui.width.clamp(panel::MIN_WIDTH, 4096);
        self.cfg.window_height = self.ui.height.clamp(panel::MIN_HEIGHT, 4096);
        if let Err(e) = self.cfg.save() {
            self.scan_error = format!("Could not save window size: {e}");
        }
    }
    fn update_animation_setting(&mut self) {
        // SAFETY: writes one BOOL to a correctly sized local variable.
        unsafe {
            let mut enabled = 1i32;
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut enabled as *mut i32).cast(),
                0,
            );
            self.ui.animations = enabled != 0;
        }
    }
    fn animate(&mut self) {
        self.ui.restart_animation();
        if !self.ui.animations {
            self.ui.tick(self.startup);
        }
        // SAFETY: timer targets this panel; it is killed when transitions finish.
        unsafe {
            if self.ui.animations {
                SetTimer(self.panel, panel::ANIMATION_TIMER, 16, None);
            }
        }
        self.invalidate();
    }
    fn hide_panel(&mut self) {
        self.save_panel_size();
        self.ui.hovered = None;
        self.ui.pressed = None;
        self.ui.focused = None;
        self.ui.drag_offset = None;
        self.ui.tick(self.startup);
        // SAFETY: no destruction; close retains the running background application.
        unsafe {
            KillTimer(self.panel, panel::ANIMATION_TIMER);
            KillTimer(self.panel, 6);
            self.renderer.release_target();
            ReleaseCapture();
            ShowWindow(self.panel, SW_HIDE);
        }
    }
    fn open_panel(&mut self) {
        self.ui.show_page(Page::Devices);
        if self.panel.is_null() {
            self.fit_panel();
        }
        // SAFETY: panel allocation and HWND mutations all run on this thread.
        unsafe {
            self.startup = startup::enabled();
            self.update_animation_setting();
            if self.panel.is_null() {
                let hinst = GetModuleHandleW(ptr::null());
                self.dpi = GetDpiForSystem().max(96);
                let mut area = RECT::default();
                SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0);
                let w = scale(self.cfg.window_width, self.dpi).min(area.right - area.left);
                let h = scale(self.cfg.window_height, self.dpi).min(area.bottom - area.top);
                self.panel = CreateWindowExW(
                    WS_EX_APPWINDOW,
                    wide(PANEL_CLASS).as_ptr(),
                    wide("BatterySentry").as_ptr(),
                    WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX,
                    area.left + (area.right - area.left - w) / 2,
                    area.top + (area.bottom - area.top - h) / 2,
                    w,
                    h,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    hinst,
                    self.context.cast(),
                );
                if self.panel.is_null() {
                    show_error(&error("Create panel").to_string());
                    return;
                }
                let corner = 2i32;
                DwmSetWindowAttribute(
                    self.panel,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    (&corner as *const i32).cast(),
                    4,
                );
                let border = Theme::LIGHT.border;
                DwmSetWindowAttribute(
                    self.panel,
                    DWMWA_BORDER_COLOR as u32,
                    (&border as *const u32).cast(),
                    4,
                );
                self.dpi = GetDpiForWindow(self.panel).max(96);
                self.update_panel_size();
            }
            self.ui.switch = if self.startup { 1.0 } else { 0.0 };
            // The first ShowWindow may inherit SW_HIDE from a silent launcher.
            // Consume that startup setting, then explicitly restore the user-requested panel.
            ShowWindow(self.panel, SW_SHOW);
            ShowWindow(self.panel, SW_RESTORE);
            self.fit_panel();
            SetTimer(self.panel, 6, 1000, None);
            SetForegroundWindow(self.panel);
            self.invalidate();
        }
    }
    fn activate_control(&mut self, control: Control) {
        match control {
            Control::Close => self.hide_panel(),
            Control::Minimize => {
                // SAFETY: live panel HWND; minimizing does not stop the worker.
                unsafe {
                    ShowWindow(self.panel, SW_MINIMIZE);
                }
            }
            Control::Startup => {
                if let Err(e) = startup::set_enabled(!self.startup) {
                    show_error(&e.to_string());
                }
                self.startup = startup::enabled();
                self.animate();
            }
            Control::Tray => {
                let next = Config {
                    show_tray_icon: !self.cfg.show_tray_icon,
                    ..self.cfg
                };
                let Some(tray) = self.tray.as_mut() else {
                    return;
                };
                if next.show_tray_icon {
                    if let Err(e) = tray.show() {
                        show_error(&e.to_string());
                        return;
                    }
                } else {
                    tray.hide();
                }
                if let Err(e) = next.save() {
                    if self.cfg.show_tray_icon {
                        let _ = tray.show();
                    } else {
                        tray.hide();
                    }
                    show_error(&e.to_string());
                    return;
                }
                self.cfg = next;
                self.ui.tray_target = next.show_tray_icon;
                self.animate();
            }
            Control::Option(index) => {
                if let Some(interval) = INTERVALS.get(index) {
                    let next = Config {
                        interval: *interval,
                        ..self.cfg
                    };
                    if let Err(e) = next.save() {
                        show_error(&e.to_string());
                        return;
                    }
                    self.cfg = next;
                    // SAFETY: replaces the existing host refresh timer.
                    unsafe {
                        SetTimer(self.host, 1, *interval * 1000, None);
                    }

                    self.ui.focused = Some(Control::Option(index));
                }
            }
            Control::Refresh => {
                if !self.busy {
                    self.refresh();
                }
            }
            Control::Exit => {
                self.save_panel_size();
                // SAFETY: asynchronous exit message, with no pointer payload.
                unsafe {
                    PostMessageW(self.host, WM_EXIT, 0, 0);
                }
            }
            Control::Settings => self.show_page(Page::Settings),
            Control::Back => self.show_page(Page::Devices),
            Control::Scrollbar => {}
        }
        self.invalidate();
    }
    fn sync_tray(&mut self) {
        if let Some(icon) = &mut self.tray {
            if self.cfg.show_tray_icon {
                if let Err(e) = icon.show() {
                    self.scan_error = e.to_string();
                }
            } else {
                icon.hide();
            }
        }
    }
    fn pointer_move(&mut self, x: i32, y: i32) {
        self.ui.drag_scrollbar(y, self.readings.len());
        if self.ui.drag_offset.is_some() {
            self.invalidate();
        }
        let hovered = self.ui.hit(x, y, self.readings.len());
        if self.ui.hovered != hovered {
            self.ui.hovered = hovered;
            self.animate();
        }
        // SAFETY: request a leave notification for this live panel window.
        unsafe {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.panel,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
        }
    }
    fn pointer_down(&mut self, x: i32, y: i32) {
        let control = self.ui.hit(x, y, self.readings.len());
        self.ui.pressed = control;
        self.ui.focused = control;
        if control == Some(Control::Scrollbar)
            && let Some(thumb) = self.ui.thumb(self.readings.len())
        {
            if thumb.contains(x, y) {
                self.ui.drag_offset = Some(y - thumb.y);
            } else {
                let step = if y < thumb.y {
                    -self.ui.list_height()
                } else {
                    self.ui.list_height()
                };
                self.ui
                    .set_scroll(self.ui.scroll_offset() + step, self.readings.len());
            }
        }
        // SAFETY: capture pairs with release on button-up/cancel.
        unsafe {
            if control.is_some() {
                SetCapture(self.panel);
            }
        }
        self.invalidate();
    }
    fn pointer_up(&mut self, x: i32, y: i32) {
        let pressed = self.ui.pressed.take();
        self.ui.drag_offset = None;
        // SAFETY: release after taking interaction state, safe during message re-entry.
        unsafe {
            ReleaseCapture();
        }
        if let Some(control) = pressed
            && self.ui.hit(x, y, self.readings.len()) == Some(control)
        {
            self.activate_control(control);
        }
        self.invalidate();
    }
    fn key(&mut self, key: usize) {
        match key as u16 {
            VK_ESCAPE => {
                if self.ui.page == Page::Settings {
                    self.show_page(Page::Devices);
                    self.invalidate();
                } else {
                    self.hide_panel();
                }
            }
            VK_TAB => {
                let controls = self.ui.controls();
                let current = controls.iter().position(|c| Some(*c) == self.ui.focused);
                // SAFETY: reads current modifier state, no pointer arguments.
                let backwards = unsafe { GetKeyState(VK_SHIFT as i32) < 0 };
                let next = current.map_or(if backwards { controls.len() - 1 } else { 0 }, |i| {
                    if backwards {
                        (i + controls.len() - 1) % controls.len()
                    } else {
                        (i + 1) % controls.len()
                    }
                });
                let control = controls[next];
                self.ui.focused = Some(control);
                self.ui.ensure_visible(control, self.readings.len());
                self.invalidate();
            }
            VK_RETURN | VK_SPACE => {
                if let Some(control) = self.ui.focused {
                    self.activate_control(control);
                }
            }
            VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT
                if self.ui.page == Page::Settings
                    && matches!(self.ui.focused, Some(Control::Option(_))) =>
            {
                let Some(Control::Option(index)) = self.ui.focused else {
                    return;
                };
                let step = match key as u16 {
                    VK_UP => -3,
                    VK_DOWN => 3,
                    VK_LEFT => -1,
                    _ => 1,
                };
                let control = Control::Option((index as i32 + step).rem_euclid(6) as usize);
                self.ui.focused = Some(control);
                self.ui.ensure_visible(control, self.readings.len());
                self.invalidate();
            }
            VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END => {
                let offset = self.ui.scroll_offset();
                let next = match key as u16 {
                    VK_UP => offset - 48,
                    VK_DOWN => offset + 48,
                    VK_PRIOR => offset - self.ui.list_height(),
                    VK_NEXT => offset + self.ui.list_height(),
                    VK_HOME => 0,
                    _ => self.ui.max_scroll(self.readings.len()),
                };
                self.ui.set_scroll(next, self.readings.len());
                self.invalidate();
            }
            82 => self.refresh(),
            _ => {}
        }
    }
    fn paint(&mut self, hwnd: HWND) {
        let view = ui::View {
            ui: &self.ui,
            readings: &self.readings,
            cfg: &self.cfg,
            busy: self.busy,
            last: self.last,
            scan_error: &self.scan_error,
            layout_status: &self.layout_status,
        };
        let _ = self.renderer.paint(hwnd, self.dpi, |c| view.paint(c));
    }
}

fn scale(n: i32, dpi: u32) -> i32 {
    n * dpi as i32 / 96
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: userdata points to the stable RefCell<App> owned by run(). RefCell
    // prevents aliasing when Windows synchronously reenters during window calls.
    unsafe {
        if msg == WM_NCCALCSIZE && GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_THICKFRAME as isize != 0
        {
            return 0;
        }
        if msg == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        if msg == WM_DESTROY {
            let mut class = [0u16; 64];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), 64);
            if len > 0 && String::from_utf16_lossy(&class[..len as usize]) == CLASS {
                KillTimer(hwnd, 1);
                KillTimer(hwnd, 2);
                KillTimer(hwnd, 3);
                KillTimer(hwnd, 4);
                PostQuitMessage(0);
            }
            return 0;
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if state.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if msg == tray::WM_TRAY && matches!(lp as u32, WM_RBUTTONUP | WM_CONTEXTMENU) {
            // TrackPopupMenu pumps messages. Hold no RefCell borrow while it is open,
            // so battery/layout completions and shutdown can still be processed.
            let command = tray::context_menu(hwnd);
            if IsWindow(hwnd) != 0
                && let Ok(mut app) = (*state).try_borrow_mut()
            {
                match command {
                    1 => app.open_panel(),
                    2 => app.refresh(),
                    3 => app.activate_control(Control::Exit),
                    4 => {
                        app.open_panel();
                        app.show_page(Page::Settings);
                        app.invalidate();
                    }
                    _ => {}
                }
            }
            return 0;
        }
        let Ok(mut app) = (*state).try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_NCHITTEST if hwnd == app.panel => {
                let mut p = POINT {
                    x: (lp as u16 as i16) as i32,
                    y: ((lp >> 16) as u16 as i16) as i32,
                };
                ScreenToClient(hwnd, &mut p);
                let x = p.x * 96 / app.dpi as i32;
                let y = p.y * 96 / app.dpi as i32;
                let left = x < 5;
                let right = x >= app.ui.width - 5;
                let top = y < 5;
                let bottom = y >= app.ui.height - 5;
                let hit = match (left, right, top, bottom) {
                    (true, _, true, _) => HTTOPLEFT,
                    (_, true, true, _) => HTTOPRIGHT,
                    (true, _, _, true) => HTBOTTOMLEFT,
                    (_, true, _, true) => HTBOTTOMRIGHT,
                    (true, _, _, _) => HTLEFT,
                    (_, true, _, _) => HTRIGHT,
                    (_, _, true, _) => HTTOP,
                    (_, _, _, true) => HTBOTTOM,
                    _ if y < 64 && x < app.ui.width - 80 => HTCAPTION,
                    _ => HTCLIENT,
                };
                return hit as isize;
            }
            WM_GETMINMAXINFO if hwnd == app.panel => {
                let info = &mut *(lp as *mut MINMAXINFO);
                let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut area = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                GetMonitorInfoW(monitor, &mut area);
                let width = area.rcWork.right - area.rcWork.left;
                let height = area.rcWork.bottom - area.rcWork.top;
                info.ptMinTrackSize = POINT {
                    x: scale(panel::MIN_WIDTH, app.dpi).min(width),
                    y: scale(panel::MIN_HEIGHT, app.dpi).min(height),
                };
                info.ptMaxTrackSize = POINT {
                    x: width,
                    y: height,
                };
                return 0;
            }
            WM_SIZE if hwnd == app.panel => {
                if wp != SIZE_MINIMIZED as usize {
                    app.update_panel_size();
                    if IsWindowVisible(hwnd) != 0 {
                        SetTimer(hwnd, 6, 1000, None);
                    }
                } else {
                    KillTimer(hwnd, panel::ANIMATION_TIMER);
                    KillTimer(hwnd, 6);
                    app.renderer.release_target();
                }
                return 0;
            }
            WM_EXITSIZEMOVE if hwnd == app.panel => {
                app.update_panel_size();
                if app.ui.width != app.cfg.window_width || app.ui.height != app.cfg.window_height {
                    app.cfg.window_size_manual = true;
                }
                app.save_panel_size();
                return 0;
            }
            WM_MOUSEACTIVATE if hwnd == app.widget => return MA_NOACTIVATE as isize,
            WM_ERASEBKGND => return 1,
            WM_PAINT if hwnd == app.widget => {
                app.paint_widget(hwnd);
                return 0;
            }
            WM_PAINT if hwnd == app.panel => {
                app.paint(hwnd);
                return 0;
            }
            WM_CLOSE if hwnd == app.panel => {
                app.hide_panel();
                return 0;
            }
            WM_LBUTTONUP if hwnd == app.widget => {
                app.refresh();
                return 0;
            }
            WM_RBUTTONUP if hwnd == app.widget => {
                app.open_panel();
                return 0;
            }
            WM_LBUTTONDBLCLK if hwnd == app.widget => {
                app.open_panel();
                return 0;
            }
            WM_MOUSEMOVE if hwnd == app.panel => {
                let x = (lp as u16 as i16) as i32 * 96 / app.dpi as i32;
                let y = ((lp >> 16) as u16 as i16) as i32 * 96 / app.dpi as i32;
                app.pointer_move(x, y);
                return 0;
            }
            WM_MOUSELEAVE if hwnd == app.panel => {
                app.ui.hovered = None;
                app.animate();
                return 0;
            }
            WM_LBUTTONDOWN if hwnd == app.panel => {
                let x = (lp as u16 as i16) as i32 * 96 / app.dpi as i32;
                let y = ((lp >> 16) as u16 as i16) as i32 * 96 / app.dpi as i32;
                app.pointer_down(x, y);
                return 0;
            }
            WM_LBUTTONUP if hwnd == app.panel => {
                let x = (lp as u16 as i16) as i32 * 96 / app.dpi as i32;
                let y = ((lp >> 16) as u16 as i16) as i32 * 96 / app.dpi as i32;
                app.pointer_up(x, y);
                return 0;
            }
            WM_CAPTURECHANGED | WM_CANCELMODE if hwnd == app.panel => {
                app.ui.pressed = None;
                app.ui.drag_offset = None;
                app.invalidate();
                return 0;
            }
            WM_KEYDOWN if hwnd == app.panel => {
                app.key(wp);
                return 0;
            }
            WM_MOUSEWHEEL if hwnd == app.panel => {
                let mut p = POINT {
                    x: (lp as u16 as i16) as i32,
                    y: ((lp >> 16) as u16 as i16) as i32,
                };
                ScreenToClient(hwnd, &mut p);
                let y = p.y * 96 / app.dpi as i32;
                if (app.ui.content_top()..app.ui.footer()).contains(&y) {
                    let count = app.readings.len();
                    app.ui.wheel((wp >> 16) as u16 as i16 as i32, count);
                }
                app.invalidate();
                return 0;
            }
            WM_DPICHANGED if hwnd == app.widget => {
                app.request_layout();
                return 0;
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                app.request_layout();
                app.invalidate_widget();
                app.update_animation_setting();
                return 0;
            }
            WM_DPICHANGED => {
                app.dpi = (wp & 0xffff) as u32;
                let r = &*(lp as *const RECT);
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                app.update_panel_size();
                return 0;
            }
            taskbar::WM_LAYOUT_DIRTY => {
                SetTimer(app.host, 3, 250, None);
                return 0;
            }
            taskbar::WM_LAYOUT => {
                app.apply_layout();
                return 0;
            }
            WM_TIMER if hwnd == app.panel && wp == 6 => {
                if app.ui.page == Page::Devices && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0
                {
                    app.invalidate();
                }
                return 0;
            }
            WM_TIMER if hwnd == app.panel && wp == panel::ANIMATION_TIMER => {
                let startup = app.startup;
                if !app.ui.tick(startup) {
                    KillTimer(hwnd, panel::ANIMATION_TIMER);
                }
                app.invalidate();
                return 0;
            }
            WM_TIMER if hwnd == app.host && wp == 4 => {
                KillTimer(hwnd, 4);
                if app.layout_retry_pending {
                    app.layout_retry_pending = false;
                    if !app.layout_intentionally_hidden && app.layout_failures < 3 {
                        app.layout_failures += 1;
                        if app.layout_failures == 3 {
                            app.open_panel();
                        }
                    }
                    app.request_layout();
                }
                return 0;
            }
            WM_TIMER if hwnd == app.host && wp == 3 => {
                KillTimer(hwnd, wp);
                app.request_layout();
                return 0;
            }
            WM_TIMER if hwnd == app.host => {
                if wp == 2 {
                    KillTimer(hwnd, 2);
                }
                app.refresh();
                return 0;
            }
            WM_DEVICECHANGE => {
                // Debounce hotplug bursts with a one-shot Windows timer.
                SetTimer(app.host, 2, 1000, None);
                app.rescan = true;
                return 0;
            }
            WM_POWERBROADCAST if wp == 0x12 || wp == 7 => {
                app.rescan = true;
                app.refresh();
                return 1;
            }
            WM_RESULTS => {
                app.results();
                return 0;
            }
            WM_OPEN => {
                app.open_panel();
                return 0;
            }
            tray::WM_TRAY if hwnd == app.host => {
                match lp as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => {
                        app.open_panel()
                    }
                    _ => {}
                }
                return 0;
            }
            WM_EXIT => {
                if !app.panel.is_null() {
                    app.save_panel_size();
                }
                app.tray.take();
                app.destroy_widget();
                if !app.panel.is_null() {
                    let panel = app.panel;
                    app.panel = ptr::null_mut();
                    app.renderer.release_target();
                    DestroyWindow(panel);
                }
                DestroyWindow(hwnd);
                return 0;
            }
            WM_ACTIVATE if wp & 0xffff != WA_INACTIVE as usize => {
                app.startup = startup::enabled();
                app.invalidate();
            }
            WM_ACTIVATE if hwnd == app.panel => {
                app.ui.hovered = None;
                app.ui.pressed = None;
                app.animate();
            }
            _ => {
                if msg == app.taskbar {
                    if let Some(icon) = &mut app.tray {
                        icon.explorer_restarted();
                    }
                    app.sync_tray();
                    app.destroy_widget();
                    app.request_layout();
                    return 0;
                }
            }
        }
        drop(app);
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

pub fn run(args: &[String]) -> io::Result<()> {
    // SAFETY: the App Box is kept alive until all windows are destroyed and the
    // reader thread has stopped. Native classes and handles belong to this thread.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance_name = desktop_instance_name()?;
        let mutex = Handle::new(CreateMutexW(ptr::null(), 0, wide(&instance_name).as_ptr()))?;
        let existing = GetLastError() == ERROR_ALREADY_EXISTS;
        let quit = args.iter().any(|a| a == "--quit");
        if existing || quit {
            let mut host = FindWindowW(wide(CLASS).as_ptr(), wide(&instance_name).as_ptr());
            if existing && host.is_null() {
                for _ in 0..40 {
                    thread::sleep(Duration::from_millis(50));
                    host = FindWindowW(wide(CLASS).as_ptr(), wide(&instance_name).as_ptr());
                    if !host.is_null() {
                        break;
                    }
                }
            }
            if !host.is_null() && (quit || !args.iter().any(|a| a == "--background")) {
                if !quit {
                    let mut process_id = 0;
                    GetWindowThreadProcessId(host, &mut process_id);
                    AllowSetForegroundWindow(process_id);
                }
                if PostMessageW(host, if quit { WM_EXIT } else { WM_OPEN }, 0, 0) == 0 {
                    return Err(error("Notify running BatterySentry instance"));
                }
            } else if existing && !quit && !args.iter().any(|a| a == "--background") {
                return Err(io::Error::other(
                    "The running BatterySentry window is not available on this desktop",
                ));
            }
            return Ok(());
        }
        let hinst = GetModuleHandleW(ptr::null());
        for class in [CLASS, PANEL_CLASS, taskbar::WIDGET_CLASS] {
            let class_name = wide(class);
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(procedure),
                hInstance: hinst,
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hIcon: LoadIconW(hinst, ptr::without_provenance::<u16>(1)),
                hIconSm: LoadIconW(hinst, ptr::without_provenance::<u16>(1)),
                lpszClassName: class_name.as_ptr(),
                ..Default::default()
            };
            if RegisterClassExW(&wc) == 0 {
                return Err(error("Register window class"));
            }
        }
        let app = Box::new(RefCell::new(App::new()));
        app.borrow_mut().context = &*app;
        app.borrow_mut().taskbar = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let host = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide(CLASS).as_ptr(),
            wide(&instance_name).as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            (&*app as *const RefCell<App>).cast(),
        );
        if host.is_null() {
            return Err(error("Create tray host"));
        }
        let mut state = app.borrow_mut();
        state.host = host;
        match Worker::new(host) {
            Ok(worker) => state.worker = Some(worker),
            Err(e) => {
                DestroyWindow(host);
                return Err(e);
            }
        }
        state.tracker = Some(taskbar::Tracker::new(host)?);
        state.request_layout();
        if SetTimer(host, 1, state.cfg.interval * 1000, None) == 0 {
            state.destroy_widget();
            DestroyWindow(host);
            return Err(error("Start refresh timer"));
        }
        state.tray = Some(tray::TrayIcon::new(host));
        state.sync_tray();
        state.refresh();
        if !args.iter().any(|a| a == "--background") {
            state.open_panel();
        }
        drop(state);
        let mut msg = MSG::default();
        let result = loop {
            match GetMessageW(&mut msg, ptr::null_mut(), 0, 0) {
                -1 => break Err(error("Windows message loop")),
                0 => break Ok(()),
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        };
        let mut state = app.borrow_mut();
        state.tray.take();
        state.destroy_widget();
        state.renderer.release_target();
        if IsWindow(state.panel) != 0 {
            DestroyWindow(state.panel);
        }
        if IsWindow(host) != 0 {
            DestroyWindow(host);
        }
        if let Some(tracker) = state.tracker.take() {
            tracker.shutdown();
        }
        // Stop worker before releasing App storage or the singleton mutex.
        if let Some(worker) = state.worker.take() {
            worker.shutdown();
        }
        drop(state);
        drop(app);
        drop(mutex);
        result
    }
}

fn light_taskbar() -> bool {
    use windows_sys::Win32::System::Registry::*;
    // SAFETY: read-only registry access into a four-byte DWORD output buffer.
    unsafe {
        let mut value = 0u32;
        let mut len = 4;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut len,
        ) == 0
            && value != 0
    }
}

fn taskbar_key() -> u32 {
    if light_taskbar() {
        rgb(210, 210, 211)
    } else {
        rgb(40, 40, 41)
    }
}
