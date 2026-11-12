use super::*;
use std::ptr;
use windows_sys::Win32::{
    System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::LoadIconW,
};

impl View<'_> {
    pub(super) fn button_color(&self, control: Control, base: u32, highlight: u32) -> u32 {
        let amount = if self.ui.pressed == Some(control) && self.ui.hovered == Some(control) {
            0.65
        } else {
            self.ui.hover[control.index()]
        };
        blend(base, highlight, amount)
    }
    pub(super) fn paint_header(&self, c: &Canvas<'_>) {
        // SAFETY: shared module icon remains valid for the lifetime of the process.
        let icon = unsafe {
            LoadIconW(
                GetModuleHandleW(ptr::null()),
                ptr::without_provenance::<u16>(1),
            )
        };
        c.icon(
            icon,
            Bounds {
                x: MARGIN,
                y: 14,
                w: 24,
                h: 24,
            },
        );
        c.text(
            if self.ui.page == Page::Devices {
                "BatterySentry"
            } else {
                "Settings"
            },
            MARGIN + 34,
            10,
            self.ui.width - MARGIN - 120,
            30,
            FONT_TITLE,
            WEIGHT_MEDIUM,
            THEME.text,
        );
        if self.ui.page == Page::Settings {
            c.text(
                concat!("BatterySentry · ", env!("CARGO_PKG_VERSION")),
                MARGIN,
                44,
                self.ui.width - 2 * MARGIN,
                20,
                FONT_SMALL,
                WEIGHT_NORMAL,
                THEME.muted,
            );
        }
        for control in [Control::Minimize, Control::Close] {
            let b = self.ui.bounds(control);
            let highlight = if control == Control::Close {
                rgb(220, 45, 65)
            } else {
                THEME.border
            };
            c.fill(
                b.x,
                b.y,
                b.w,
                b.h,
                self.button_color(control, THEME.background, highlight),
            );
            let cx = b.x + b.w / 2;
            let cy = b.h / 2;
            if control == Control::Close {
                let ink = blend(THEME.muted, THEME.surface, self.ui.hover[control.index()]);
                c.line(cx - 5, cy - 5, cx + 5, cy + 5, ink);
                c.line(cx + 5, cy - 5, cx - 5, cy + 5, ink);
            } else {
                c.line(cx - 5, cy, cx + 5, cy, THEME.text);
            }
        }
    }
    pub(super) fn paint_actions(&self, c: &Canvas<'_>) {
        let controls: &[(Control, &str)] = match self.ui.page {
            Page::Devices => &[
                (
                    Control::Refresh,
                    if self.busy {
                        "Reading…"
                    } else {
                        "Refresh now"
                    },
                ),
                (Control::Settings, "Settings"),
            ],
            Page::Settings => &[(Control::Back, "Back"), (Control::Exit, "Exit app")],
        };
        for &(control, label) in controls {
            let b = self.ui.bounds(control);
            let primary = control == Control::Refresh;
            let base = if primary { THEME.accent } else { THEME.surface };
            let highlight = if primary {
                THEME.accent_hover
            } else {
                THEME.border
            };
            c.round(
                b.x,
                b.y,
                b.w,
                b.h,
                RADIUS_BUTTON,
                self.button_color(control, base, highlight),
            );
            c.text(
                label,
                b.x + 12,
                b.y,
                b.w - 24,
                b.h,
                FONT_BODY,
                WEIGHT_MEDIUM,
                if primary { THEME.surface } else { THEME.text },
            );
        }
    }
    pub(super) fn paint_focus(&self, c: &Canvas<'_>, control: Control) {
        let b = self.ui.bounds(control);
        c.fill(b.x + 8, b.y + b.h - 3, (b.w - 16).max(1), 2, THEME.accent);
    }
}

pub(super) fn blend(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((from >> shift) & 255) as f32;
        let b = ((to >> shift) & 255) as f32;
        (a + (b - a) * amount.clamp(0.0, 1.0)).round() as u32
    };
    channel(0) | (channel(8) << 8) | (channel(16) << 16)
}
