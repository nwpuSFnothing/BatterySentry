//! GDI double buffering in physical pixels. Fonts and the back buffer survive repainting.
use crate::{
    panel::Bounds,
    theme,
    win::{error, wide},
};
use std::{cell::RefCell, collections::HashMap, io, ptr};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

struct Font(HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: text operations restore the previous font before releasing this cache.
        unsafe { DeleteObject(self.0) };
    }
}
type Fonts = RefCell<HashMap<(i32, u32), Font>>;

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
                return Err(error("Create paint DC"));
            }
            // Use the display DC so the bitmap keeps its color depth.
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err(error("Create paint bitmap"));
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() {
                DeleteObject(bitmap);
                DeleteDC(dc);
                return Err(error("Select paint bitmap"));
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

struct Paint {
    hwnd: HWND,
    dc: HDC,
    state: PAINTSTRUCT,
}
impl Paint {
    fn begin(hwnd: HWND) -> Self {
        let mut state = PAINTSTRUCT::default();
        // SAFETY: called only while handling WM_PAINT on the window's UI thread.
        let dc = unsafe { BeginPaint(hwnd, &mut state) };
        Self { hwnd, dc, state }
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        // SAFETY: balances BeginPaint, including allocation-failure paths.
        unsafe { EndPaint(self.hwnd, &self.state) };
    }
}

#[derive(Default)]
pub struct Renderer {
    buffer: Option<Buffer>,
    scratch: RefCell<Option<Buffer>>,
    fonts: Fonts,
    dpi: u32,
}
impl Renderer {
    pub fn paint(
        &mut self,
        hwnd: HWND,
        dpi: u32,
        draw: impl FnOnce(&Canvas<'_>),
    ) -> io::Result<()> {
        let paint = Paint::begin(hwnd);
        let mut bounds = RECT::default();
        // SAFETY: live HWND and writable RECT, called on the owning UI thread.
        if unsafe { GetClientRect(hwnd, &mut bounds) } == 0 {
            return Err(error("Read paint bounds"));
        }
        self.draw(paint.dc, bounds.right, bounds.bottom, dpi, draw)
    }
    pub fn draw(
        &mut self,
        target: HDC,
        width: i32,
        height: i32,
        dpi: u32,
        draw: impl FnOnce(&Canvas<'_>),
    ) -> io::Result<()> {
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        let dpi = dpi.max(96);
        if self.dpi != dpi {
            self.fonts.borrow_mut().clear();
            self.dpi = dpi;
        }
        if !self
            .buffer
            .as_ref()
            .is_some_and(|b| b.width == width && b.height == height)
        {
            self.buffer = Some(Buffer::new(target, width, height)?);
        }
        let buffer = self.buffer.as_ref().expect("paint buffer initialized");
        let canvas = Canvas {
            dc: buffer.dc,
            dpi,
            width,
            height,
            fonts: &self.fonts,
            scratch: &self.scratch,
        };
        draw(&canvas);
        // SAFETY: both DCs use physical pixels; the final copy is always one-to-one.
        if unsafe { BitBlt(target, 0, 0, width, height, buffer.dc, 0, 0, SRCCOPY) } == 0 {
            return Err(error("Present paint buffer"));
        }
        Ok(())
    }
}

pub struct Canvas<'a> {
    dc: HDC,
    dpi: u32,
    width: i32,
    height: i32,
    fonts: &'a Fonts,
    scratch: &'a RefCell<Option<Buffer>>,
}
impl Canvas<'_> {
    pub fn px(&self, value: i32) -> i32 {
        let numerator = i64::from(value) * i64::from(self.dpi);
        ((numerator + numerator.signum() * 48) / 96) as i32
    }
    pub fn width(&self) -> i32 {
        self.width * 96 / self.dpi as i32
    }
    pub fn height(&self) -> i32 {
        self.height * 96 / self.dpi as i32
    }
    fn rect(&self, b: Bounds) -> RECT {
        RECT {
            left: self.px(b.x),
            top: self.px(b.y),
            right: self.px(b.x + b.w),
            bottom: self.px(b.y + b.h),
        }
    }
    pub fn clear(&self, color: u32) {
        self.fill_pixels(
            RECT {
                left: 0,
                top: 0,
                right: self.width,
                bottom: self.height,
            },
            color,
        );
    }
    fn fill_pixels(&self, rect: RECT, color: u32) {
        // SAFETY: stock brush is borrowed, and the DC belongs to this renderer.
        unsafe {
            SetDCBrushColor(self.dc, color);
            FillRect(self.dc, &rect, GetStockObject(DC_BRUSH) as HBRUSH);
        }
    }
    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.fill_pixels(self.rect(Bounds { x, y, w, h }), color);
    }
    pub fn round(&self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = self.rect(Bounds { x, y, w, h });
        let (width, height) = (r.right - r.left, r.bottom - r.top);
        let mut scratch = self.scratch.borrow_mut();
        let (bw, bh) = (width * 3, height * 3);
        if !scratch
            .as_ref()
            .is_some_and(|b| b.width >= bw && b.height >= bh)
        {
            let capacity_w = scratch.as_ref().map_or(bw, |b| b.width.max(bw));
            let capacity_h = scratch.as_ref().map_or(bh, |b| b.height.max(bh));
            if let Ok(buffer) = Buffer::new(self.dc, capacity_w, capacity_h) {
                *scratch = Some(buffer);
            } else {
                Self::round_pixels(self.dc, r, self.px(radius), color);
                return;
            }
        }
        let dc = scratch.as_ref().expect("shape buffer initialized").dc;
        // SAFETY: only this shape's background and geometry use the reusable scratch bitmap.
        // Text is drawn afterwards at native resolution and presented with one-to-one BitBlt.
        unsafe {
            SetStretchBltMode(dc, COLORONCOLOR);
            StretchBlt(
                dc, 0, 0, bw, bh, self.dc, r.left, r.top, width, height, SRCCOPY,
            );
            Self::round_pixels(
                dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: bw,
                    bottom: bh,
                },
                self.px(radius) * 3,
                color,
            );
            let mode = SetStretchBltMode(self.dc, HALFTONE);
            let mut origin = POINT::default();
            SetBrushOrgEx(self.dc, 0, 0, &mut origin);
            StretchBlt(
                self.dc, r.left, r.top, width, height, dc, 0, 0, bw, bh, SRCCOPY,
            );
            SetBrushOrgEx(self.dc, origin.x, origin.y, ptr::null_mut());
            SetStretchBltMode(self.dc, mode);
        }
    }
    fn round_pixels(dc: HDC, r: RECT, radius: i32, color: u32) {
        // SAFETY: borrowed stock objects are restored after drawing into the live memory DC.
        unsafe {
            let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
            let pen = SelectObject(dc, GetStockObject(NULL_PEN));
            SetDCBrushColor(dc, color);
            RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
            SelectObject(dc, pen);
            SelectObject(dc, brush);
        }
    }
    pub fn line(&self, x1: i32, y1: i32, x2: i32, y2: i32, color: u32) {
        // SAFETY: temporary pen is deselected and destroyed on this UI thread.
        unsafe {
            let pen = CreatePen(PS_SOLID, self.px(1).max(1), color);
            let previous = SelectObject(self.dc, pen);
            MoveToEx(self.dc, self.px(x1), self.px(y1), ptr::null_mut());
            LineTo(self.dc, self.px(x2), self.px(y2));
            SelectObject(self.dc, previous);
            DeleteObject(pen);
        }
    }
    fn font(&self, size: i32, weight: u32) -> HFONT {
        let mut fonts = self.fonts.borrow_mut();
        let key = (self.px(size).max(1), weight);
        if let Some(font) = fonts.get(&key) {
            return font.0;
        }
        // SAFETY: terminated face name, font height is explicitly in physical pixels.
        let font = unsafe {
            CreateFontW(
                -key.0,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                ANSI_CHARSET as u32,
                OUT_OUTLINE_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (VARIABLE_PITCH | FF_SWISS) as u32,
                wide(theme::FONT_FAMILY).as_ptr(),
            )
        };
        if font.is_null() {
            // SAFETY: stock font is borrowed and is never inserted into the owned cache.
            return unsafe { GetStockObject(DEFAULT_GUI_FONT) as HFONT };
        }
        fonts.insert(key, Font(font));
        font
    }
    pub fn text_width(&self, text: &str, size: i32, weight: i32) -> i32 {
        // SAFETY: the cached font remains owned by Renderer and is restored after measurement.
        unsafe {
            let previous = SelectObject(self.dc, self.font(size, weight as u32));
            let text = wide(text);
            let mut extent = SIZE::default();
            GetTextExtentPoint32W(self.dc, text.as_ptr(), (text.len() - 1) as i32, &mut extent);
            SelectObject(self.dc, previous);
            (extent.cx * 96 + self.dpi as i32 - 1) / self.dpi as i32
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "GDI text primitive with explicit bounds and style"
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
        color: u32,
    ) {
        self.draw_text(text, Bounds { x, y, w, h }, size, weight, color, DT_LEFT);
    }
    pub fn right(&self, text: &str, bounds: Bounds, size: i32, weight: u32, color: u32) {
        self.draw_text(text, bounds, size, weight, color, DT_RIGHT);
    }
    fn draw_text(
        &self,
        text: &str,
        bounds: Bounds,
        size: i32,
        weight: u32,
        color: u32,
        align: u32,
    ) {
        if bounds.w <= 0 || bounds.h <= 0 {
            return;
        }
        // SAFETY: the selected font and text buffer remain valid through DrawTextW.
        unsafe {
            let previous = SelectObject(self.dc, self.font(size, weight));
            SetBkMode(self.dc, TRANSPARENT as i32);
            SetTextColor(self.dc, color);
            let text = wide(text);
            let mut rect = self.rect(bounds);
            DrawTextW(
                self.dc,
                text.as_ptr(),
                (text.len() - 1) as i32,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | align,
            );
            SelectObject(self.dc, previous);
        }
    }
    pub fn clip(&self, bounds: Bounds, draw: impl FnOnce()) {
        // SAFETY: clip changes are scoped to this DC and restored before presentation.
        unsafe {
            let saved = SaveDC(self.dc);
            let r = self.rect(bounds);
            IntersectClipRect(self.dc, r.left, r.top, r.right, r.bottom);
            draw();
            RestoreDC(self.dc, saved);
        }
    }
    pub fn icon(&self, icon: HICON, bounds: Bounds) {
        // SAFETY: shared icon is borrowed and rendered directly at its physical dimensions.
        unsafe {
            let r = self.rect(bounds);
            DrawIconEx(
                self.dc,
                r.left,
                r.top,
                icon,
                r.right - r.left,
                r.bottom - r.top,
                0,
                ptr::null_mut(),
                DI_NORMAL,
            );
        }
    }
}
