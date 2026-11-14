//! Direct2D/DirectWrite presentation for the opaque panel HWND. All coordinates are DIPs.
use crate::{panel::Bounds, theme, win::wide};
use std::{cell::RefCell, collections::HashMap, io, mem::size_of, ptr};
use windows::{
    Win32::{
        Foundation::{D2DERR_RECREATE_TARGET, HWND as WinHwnd},
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        },
    },
    core::{PCWSTR, Result as WinResult},
};
use windows_numerics::Vector2;
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::{GetClientRect, GetIconInfo, HICON, ICONINFO},
};

struct Paint {
    hwnd: HWND,
    state: PAINTSTRUCT,
    dc: HDC,
}
impl Paint {
    fn begin(hwnd: HWND) -> Self {
        let mut state = PAINTSTRUCT::default();
        // SAFETY: the UI thread serves WM_PAINT for this live HWND.
        let dc = unsafe { BeginPaint(hwnd, &mut state) };
        Self { hwnd, state, dc }
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        // SAFETY: validates the paint region on every exit path.
        unsafe { EndPaint(self.hwnd, &self.state) };
    }
}

struct Buffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    width: i32,
    height: i32,
}
impl Buffer {
    fn new(target: HDC, width: i32, height: i32) -> io::Result<Self> {
        // SAFETY: dimensions are positive and target is the current paint DC.
        unsafe {
            let dc = CreateCompatibleDC(target);
            if dc.is_null() {
                return Err(crate::win::error("Create paint DC"));
            }
            // Use the display DC so the bitmap keeps its color depth.
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err(crate::win::error("Create paint bitmap"));
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return Err(crate::win::error("Select paint bitmap"));
            }
            SetMapMode(dc, MM_TEXT);
            Ok(Self {
                dc,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: this buffer uniquely owns the DC and deselects its bitmap first.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

struct Format {
    text: IDWriteTextFormat,
    ellipsis: IDWriteInlineObject,
}
#[derive(Hash, PartialEq, Eq)]
struct LayoutKey {
    text: String,
    size: i32,
    weight: u32,
    width: i32,
    height: i32,
    right: bool,
}
struct Text {
    factory: IDWriteFactory,
    formats: HashMap<(i32, u32), Format>,
    layouts: HashMap<LayoutKey, IDWriteTextLayout>,
}
impl Text {
    fn layout(
        &mut self,
        text: &str,
        size: i32,
        weight: u32,
        width: i32,
        height: i32,
        right: bool,
    ) -> WinResult<IDWriteTextLayout> {
        let key = LayoutKey {
            text: text.into(),
            size,
            weight,
            width,
            height,
            right,
        };
        if let Some(layout) = self.layouts.get(&key) {
            return Ok(layout.clone());
        }
        // SAFETY: factory/objects stay on this UI thread; UTF-16 buffers outlive each call.
        unsafe {
            if !self.formats.contains_key(&(size, weight)) {
                let format = self.factory.CreateTextFormat(
                    PCWSTR(wide(theme::FONT_FAMILY).as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size as f32,
                    PCWSTR(wide("en-US").as_ptr()),
                )?;
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                let ellipsis = self.factory.CreateEllipsisTrimmingSign(&format)?;
                self.formats.insert(
                    (size, weight),
                    Format {
                        text: format,
                        ellipsis,
                    },
                );
            }
            let format = &self.formats[&(size, weight)];
            let chars: Vec<u16> = text.encode_utf16().collect();
            let layout =
                self.factory
                    .CreateTextLayout(&chars, &format.text, width as f32, height as f32)?;
            layout.SetTextAlignment(if right {
                DWRITE_TEXT_ALIGNMENT_TRAILING
            } else {
                DWRITE_TEXT_ALIGNMENT_LEADING
            })?;
            layout.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    ..Default::default()
                },
                &format.ellipsis,
            )?;
            // Dynamic status strings must not grow the cache for the entire process lifetime.
            if self.layouts.len() >= 64 {
                self.layouts.clear();
            }
            self.layouts.insert(key, layout.clone());
            Ok(layout)
        }
    }
}
struct Factories {
    d2d: ID2D1Factory,
    text: RefCell<Text>,
}
impl Factories {
    fn new() -> WinResult<Self> {
        // SAFETY: both factories are created and used on the owning window thread.
        unsafe {
            Ok(Self {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                text: RefCell::new(Text {
                    factory: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                    formats: HashMap::new(),
                    layouts: HashMap::new(),
                }),
            })
        }
    }
}
struct Device {
    hwnd: HWND,
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    icons: RefCell<HashMap<usize, ID2D1Bitmap>>,
}

#[derive(Default)]
pub struct Renderer {
    factories: Option<Factories>,
    device: Option<Device>,
}
impl Renderer {
    pub fn release_target(&mut self) {
        self.device = None;
    }
    fn target(&mut self, hwnd: HWND, width: u32, height: u32, dpi: u32) -> WinResult<()> {
        if self.factories.is_none() {
            self.factories = Some(Factories::new()?);
        }
        if self.device.as_ref().is_some_and(|d| d.hwnd != hwnd) {
            self.release_target();
        }
        let size = D2D_SIZE_U { width, height };
        // SAFETY: target and brush share one resource domain; HWND and dimensions are current.
        unsafe {
            if let Some(device) = &self.device {
                let current = device.target.GetPixelSize();
                if (current.width != width || current.height != height)
                    && let Err(error) = device.target.Resize(&size)
                {
                    if error.code() != D2DERR_RECREATE_TARGET {
                        return Err(error);
                    }
                    self.release_target();
                }
            }
            if self.device.is_none() {
                let target = self
                    .factories
                    .as_ref()
                    .expect("factories initialized")
                    .d2d
                    .CreateHwndRenderTarget(
                        &D2D1_RENDER_TARGET_PROPERTIES {
                            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_IGNORE,
                            },
                            dpiX: dpi as f32,
                            dpiY: dpi as f32,
                            ..Default::default()
                        },
                        &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                            hwnd: WinHwnd(hwnd),
                            pixelSize: size,
                            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                        },
                    )?;
                let brush = target.CreateSolidColorBrush(&color(0), None)?;
                self.device = Some(Device {
                    hwnd,
                    target,
                    brush,
                    icons: RefCell::new(HashMap::new()),
                });
            }
            let device = self.device.as_ref().expect("target initialized");
            device.target.SetDpi(dpi as f32, dpi as f32);
            device
                .target
                .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            // DEFAULT honors DirectWrite/system rendering parameters on this opaque target.
            device
                .target
                .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_DEFAULT);
        }
        Ok(())
    }
    fn finish(&mut self, result: WinResult<()>) -> WinResult<()> {
        if result
            .as_ref()
            .is_err_and(|e| e.code() == D2DERR_RECREATE_TARGET)
        {
            self.release_target();
        }
        result
    }
    pub fn paint(&mut self, hwnd: HWND, dpi: u32, draw: impl Fn(&Canvas<'_>)) -> io::Result<()> {
        let _paint = Paint::begin(hwnd);
        let mut bounds = RECT::default();
        // SAFETY: writable output for a live panel HWND.
        if unsafe { GetClientRect(hwnd, &mut bounds) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if bounds.right <= 0 || bounds.bottom <= 0 {
            return Ok(());
        }
        for _ in 0..2 {
            self.target(hwnd, bounds.right as u32, bounds.bottom as u32, dpi.max(96))
                .map_err(io::Error::other)?;
            let device = self.device.as_ref().expect("target initialized");
            let canvas = Canvas {
                target: &device.target,
                brush: &device.brush,
                icons: &device.icons,
                text: &self.factories.as_ref().expect("factories initialized").text,
                error: RefCell::new(None),
            };
            // SAFETY: each frame has a balanced BeginDraw/EndDraw on its window thread.
            unsafe { device.target.BeginDraw() };
            draw(&canvas);
            // SAFETY: completes all queued drawing, including deferred device-loss errors.
            let result = unsafe { device.target.EndDraw(None, None) }
                .and_then(|()| canvas.error.into_inner().map_or(Ok(()), Err));
            match self.finish(result) {
                Ok(()) => return Ok(()),
                Err(error) if error.code() == D2DERR_RECREATE_TARGET => continue,
                Err(error) => return Err(io::Error::other(error)),
            }
        }
        Err(io::Error::other(
            "Direct2D render target could not be recreated",
        ))
    }
}

pub struct Canvas<'a> {
    target: &'a ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    icons: &'a RefCell<HashMap<usize, ID2D1Bitmap>>,
    text: &'a RefCell<Text>,
    error: RefCell<Option<windows::core::Error>>,
}
fn color(value: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (value & 255) as f32 / 255.0,
        g: ((value >> 8) & 255) as f32 / 255.0,
        b: ((value >> 16) & 255) as f32 / 255.0,
        a: 1.0,
    }
}
fn rect(b: Bounds) -> D2D_RECT_F {
    D2D_RECT_F {
        left: b.x as f32,
        top: b.y as f32,
        right: (b.x + b.w) as f32,
        bottom: (b.y + b.h) as f32,
    }
}
impl Canvas<'_> {
    pub fn width(&self) -> i32 {
        // SAFETY: target size is queried in DIPs on the owning render thread.
        unsafe { self.target.GetSize().width.floor() as i32 }
    }
    pub fn height(&self) -> i32 {
        // SAFETY: target size is queried in DIPs on the owning render thread.
        unsafe { self.target.GetSize().height.floor() as i32 }
    }
    fn remember<T>(&self, result: WinResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.error.borrow_mut().get_or_insert(error);
                None
            }
        }
    }
    fn brush(&self, value: u32) -> &ID2D1SolidColorBrush {
        // SAFETY: one mutable-color brush is reused serially by this UI-thread target.
        unsafe { self.brush.SetColor(&color(value)) };
        self.brush
    }
    pub fn clear(&self, value: u32) {
        // SAFETY: inside a balanced frame.
        unsafe { self.target.Clear(Some(&color(value))) };
    }
    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, value: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        // SAFETY: DIP rectangle and target-owned brush, used on the UI thread.
        unsafe {
            self.target
                .FillRectangle(&rect(Bounds { x, y, w, h }), self.brush(value))
        };
    }
    pub fn round(&self, x: i32, y: i32, w: i32, h: i32, diameter: i32, value: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        // GDI RoundRect accepted ellipse diameters. Preserve the existing visual curvature.
        let r = (diameter as f32 / 2.0).min(w.min(h) as f32 / 2.0);
        // SAFETY: native antialiased geometry replaces the temporary supersampling bitmaps.
        unsafe {
            self.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect(Bounds { x, y, w, h }),
                    radiusX: r,
                    radiusY: r,
                },
                self.brush(value),
            )
        };
    }
    pub fn line(&self, x1: i32, y1: i32, x2: i32, y2: i32, value: u32) {
        // SAFETY: DIP points are scaled once by the render target.
        unsafe {
            self.target.DrawLine(
                Vector2::new(x1 as f32, y1 as f32),
                Vector2::new(x2 as f32, y2 as f32),
                self.brush(value),
                1.0,
                None,
            )
        };
    }
    pub fn text_width(&self, text: &str, size: i32, weight: i32) -> i32 {
        let layout = self
            .text
            .borrow_mut()
            .layout(text, size, weight as u32, 32768, 4096, false);
        let Some(layout) = self.remember(layout) else {
            return 0;
        };
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: writable metrics for an immutable cached layout with matching format.
        let measured = unsafe { layout.GetMetrics(&mut metrics) };
        if self.remember(measured).is_none() {
            return 0;
        }
        metrics.widthIncludingTrailingWhitespace.ceil() as i32
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Existing bounded text drawing API"
    )]
    pub fn text(
        &self,
        text: &str,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        size: i32,
        weight: u32,
        value: u32,
    ) {
        self.draw_text(text, Bounds { x, y, w, h }, size, weight, value, false);
    }
    pub fn right(&self, text: &str, bounds: Bounds, size: i32, weight: u32, value: u32) {
        self.draw_text(text, bounds, size, weight, value, true);
    }
    fn draw_text(
        &self,
        text: &str,
        bounds: Bounds,
        size: i32,
        weight: u32,
        value: u32,
        right: bool,
    ) {
        if bounds.w <= 0 || bounds.h <= 0 || text.is_empty() {
            return;
        }
        let layout = self
            .text
            .borrow_mut()
            .layout(text, size, weight, bounds.w, bounds.h, right);
        let Some(layout) = self.remember(layout) else {
            return;
        };
        // SAFETY: layout format and bounds match measurement; all text remains in DIPs.
        unsafe {
            self.target.DrawTextLayout(
                Vector2::new(bounds.x as f32, bounds.y as f32),
                &layout,
                self.brush(value),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            )
        };
    }
    pub fn clip(&self, bounds: Bounds, draw: impl FnOnce()) {
        // SAFETY: balanced axis-aligned clipping on the same target, without alpha layers.
        unsafe {
            self.target
                .PushAxisAlignedClip(&rect(bounds), D2D1_ANTIALIAS_MODE_ALIASED)
        };
        draw();
        // SAFETY: balances the clip pushed above.
        unsafe { self.target.PopAxisAlignedClip() };
    }
    pub fn icon(&self, icon: HICON, bounds: Bounds) {
        let mut icons = self.icons.borrow_mut();
        if let std::collections::hash_map::Entry::Vacant(entry) = icons.entry(icon as usize) {
            let Some(bitmap) = self.remember(icon_bitmap(self.target, icon)) else {
                return;
            };
            entry.insert(bitmap);
        }
        // SAFETY: icon bitmap belongs to this target and remains cached for its lifetime.
        unsafe {
            self.target.DrawBitmap(
                &icons[&(icon as usize)],
                Some(&rect(bounds)),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            )
        };
    }
}

// HICON is a Win32 resource. GDI is used once to read its pixels, never to paint panel controls.
fn icon_bitmap(target: &ID2D1RenderTarget, icon: HICON) -> WinResult<ID2D1Bitmap> {
    let mut info = ICONINFO::default();
    // SAFETY: GetIconInfo returns owned bitmap copies, released after conversion on every path.
    unsafe {
        if GetIconInfo(icon, &mut info) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let result = (|| {
            let mut bitmap = BITMAP::default();
            if info.hbmColor.is_null()
                || GetObjectW(
                    info.hbmColor,
                    size_of::<BITMAP>() as i32,
                    (&mut bitmap as *mut BITMAP).cast(),
                ) == 0
            {
                return Err(windows::core::Error::from_thread());
            }
            let width = bitmap.bmWidth;
            let height = bitmap.bmHeight.abs();
            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            let mut format = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            let rows = GetDIBits(
                dc,
                info.hbmColor,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut format,
                DIB_RGB_COLORS,
            );
            DeleteDC(dc);
            if rows != height {
                return Err(windows::core::Error::from_thread());
            }
            target.CreateBitmap(
                D2D_SIZE_U {
                    width: width as u32,
                    height: height as u32,
                },
                Some(pixels.as_ptr().cast()),
                width as u32 * 4,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                },
            )
        })();
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        result
    }
}

#[cfg(test)]
impl Renderer {
    pub(super) fn check_device_loss(&mut self) {
        assert!(self.device.is_some());
        let formats = self.factories.as_ref().unwrap().text.borrow().formats.len();
        let error = windows::core::Error::from_hresult(D2DERR_RECREATE_TARGET);
        assert!(self.finish(Err(error)).is_err());
        assert!(self.device.is_none());
        assert_eq!(
            self.factories.as_ref().unwrap().text.borrow().formats.len(),
            formats
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::Interface;
    #[test]
    fn taskbar_direct2d_draws_into_layered_child() {
        use crate::{drivers::Reading, protocol::BatteryValue, ui::widget};
        use windows_sys::Win32::{
            System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
        };
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                // SAFETY: the fixture owns this HWND and destroys children first.
                unsafe { DestroyWindow(self.0) };
            }
        }
        let readings = [
            Reading {
                device_key: "atk".into(),
                identity: String::new(),
                name: "ATK A9 Plus NK".into(),
                value: Some(BatteryValue::Percent(83)),
                last_known: None,
                detail: String::new(),
            },
            Reading {
                device_key: "aula".into(),
                identity: String::new(),
                name: "AULA F87 Pro".into(),
                value: Some(BatteryValue::Charging),
                last_known: None,
                detail: String::new(),
            },
        ];
        // SAFETY: all native windows, DCs and renderer resources stay on this test thread.
        unsafe {
            let class = wide("BatterySentry.Direct2DTaskbarCheck");
            let instance = GetModuleHandleW(ptr::null());
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                hInstance: instance,
                lpfnWndProc: Some(DefWindowProcW),
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            assert_ne!(RegisterClassExW(&wc), 0);
            let parent = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                440,
                100,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            assert!(!parent.is_null());
            let _parent = Window(parent);
            let child = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_CHILD,
                0,
                0,
                200,
                42,
                parent,
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            assert!(!child.is_null());
            let _child = Window(child);
            let key = theme::rgb(210, 210, 211);
            assert_ne!(SetLayeredWindowAttributes(child, key, 255, LWA_COLORKEY), 0);
            let mut renderer = WidgetRenderer::default();
            for dpi in [96, 120, 144, 192] {
                SetWindowPos(
                    child,
                    ptr::null_mut(),
                    0,
                    0,
                    200 * dpi / 96,
                    42 * dpi / 96,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                InvalidateRect(child, ptr::null(), 0);
                renderer
                    .paint(child, dpi as u32, |c| {
                        widget::paint_widget(c, &readings, theme::Theme::LIGHT, key);
                    })
                    .unwrap();
                let buffer = renderer.buffer.as_ref().unwrap();
                assert_eq!(GetPixel(buffer.dc, 0, 0), key);
                assert_eq!(GetPixel(buffer.dc, 190 * dpi / 96, 21 * dpi / 96), key);
                assert!(
                    !renderer
                        .factories
                        .as_ref()
                        .unwrap()
                        .text
                        .borrow()
                        .formats
                        .is_empty()
                );
            }
            drop(renderer);
        }
    }
    #[test]
    fn text_layout_reuses_formats_and_preserves_single_line_bounds() {
        let factories = Factories::new().unwrap();
        let mut text = factories.text.borrow_mut();
        let a = text
            .layout("BatterySentry", 15, 600, 180, 24, false)
            .unwrap();
        let b = text
            .layout("BatterySentry", 15, 600, 180, 24, false)
            .unwrap();
        assert_eq!(a.as_raw(), b.as_raw());
        let long = text
            .layout(
                "AULA F87 Pro 外设电量 — long device name",
                15,
                600,
                96,
                24,
                false,
            )
            .unwrap();
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: this cached layout is live and the metrics buffer is writable.
        unsafe {
            long.GetMetrics(&mut metrics).unwrap();
            assert_eq!(long.GetWordWrapping(), DWRITE_WORD_WRAPPING_NO_WRAP);
        }
        assert_eq!(metrics.lineCount, 1);
        assert_eq!(metrics.layoutWidth, 96.0);
        assert_eq!(text.formats.len(), 1);
        for i in 0..100 {
            text.layout(&format!("Last scan {i}s ago"), 15, 600, 180, 24, false)
                .unwrap();
        }
        assert!(text.layouts.len() <= 64);
    }
}

// The Explorer child keeps its color-keyed HWND and one-to-one DC presentation.
// All text and geometry use the same DirectWrite/Direct2D Canvas as the panel.
struct DcDevice {
    target: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    icons: RefCell<HashMap<usize, ID2D1Bitmap>>,
}
#[derive(Default)]
pub struct WidgetRenderer {
    device: Option<DcDevice>,
    buffer: Option<Buffer>,
    factories: Option<Factories>,
}
impl WidgetRenderer {
    pub fn paint(&mut self, hwnd: HWND, dpi: u32, draw: impl Fn(&Canvas<'_>)) -> io::Result<()> {
        let paint = Paint::begin(hwnd);
        let mut bounds = RECT::default();
        // SAFETY: the widget HWND and paint DC are owned by this UI thread.
        if unsafe { GetClientRect(hwnd, &mut bounds) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if bounds.right <= 0 || bounds.bottom <= 0 {
            return Ok(());
        }
        if !self
            .buffer
            .as_ref()
            .is_some_and(|b| b.width == bounds.right && b.height == bounds.bottom)
        {
            // Release the target before deleting the DC to which it was bound.
            self.device = None;
            self.buffer = Some(Buffer::new(paint.dc, bounds.right, bounds.bottom)?);
        }
        if self.factories.is_none() {
            self.factories = Some(Factories::new().map_err(io::Error::other)?);
        }
        for _ in 0..2 {
            if self.device.is_none() {
                self.device = Some(self.create_device().map_err(io::Error::other)?);
            }
            let device = self.device.as_ref().expect("widget target initialized");
            let buffer = self.buffer.as_ref().expect("widget buffer initialized");
            let area = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: bounds.right,
                bottom: bounds.bottom,
            };
            // SAFETY: MM_TEXT memory DC is kept alive until after the DC render target.
            let bound = unsafe {
                device
                    .target
                    .BindDC(windows::Win32::Graphics::Gdi::HDC(buffer.dc), &area)
            };
            if let Err(error) = bound {
                if error.code() == D2DERR_RECREATE_TARGET {
                    self.device = None;
                    continue;
                }
                return Err(io::Error::other(error));
            }
            // SAFETY: DPI is applied once by Direct2D; grayscale avoids color-key text fringes.
            unsafe {
                device.target.SetDpi(dpi.max(96) as f32, dpi.max(96) as f32);
                device
                    .target
                    .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                device
                    .target
                    .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                device.target.BeginDraw();
            }
            let canvas = Canvas {
                target: &device.target,
                brush: &device.brush,
                icons: &device.icons,
                text: &self
                    .factories
                    .as_ref()
                    .expect("widget factories initialized")
                    .text,
                error: RefCell::new(None),
            };
            draw(&canvas);
            // SAFETY: balances BeginDraw and reports deferred drawing failures.
            let result = unsafe { device.target.EndDraw(None, None) }
                .and_then(|()| canvas.error.into_inner().map_or(Ok(()), Err));
            if let Err(error) = result {
                if error.code() == D2DERR_RECREATE_TARGET {
                    self.device = None;
                    continue;
                }
                return Err(io::Error::other(error));
            }
            // SAFETY: only presents the finished bitmap, without GDI text, geometry or scaling.
            if unsafe {
                BitBlt(
                    paint.dc,
                    0,
                    0,
                    bounds.right,
                    bounds.bottom,
                    buffer.dc,
                    0,
                    0,
                    SRCCOPY,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        Err(io::Error::other(
            "Direct2D taskbar target could not be recreated",
        ))
    }
    fn create_device(&self) -> WinResult<DcDevice> {
        // SAFETY: the factory and its target/brush remain on the widget's UI thread.
        unsafe {
            let target = self
                .factories
                .as_ref()
                .expect("widget factories initialized")
                .d2d
                .CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    ..Default::default()
                })?;
            let brush = target.CreateSolidColorBrush(&color(0), None)?;
            Ok(DcDevice {
                target,
                brush,
                icons: RefCell::new(HashMap::new()),
            })
        }
    }
}
