use super::*;

impl View<'_> {
    pub(super) fn paint_settings(&self, c: &Canvas<'_>) {
        for (control, label, amount) in [
            (Control::Startup, "Launch at sign-in", self.ui.switch),
            (Control::Tray, "Show tray icon", self.ui.tray_switch),
        ] {
            let b = self.ui.bounds(control);
            c.round(b.x, b.y, b.w, b.h, RADIUS_CARD, THEME.border);
            c.round(
                b.x + 1,
                b.y + 1,
                b.w - 2,
                b.h - 2,
                RADIUS_CARD - 1,
                self.button_color(control, THEME.surface, THEME.surface_alt),
            );
            c.text(
                label,
                b.x + 12,
                b.y,
                b.w - 78,
                b.h,
                FONT_BODY,
                WEIGHT_NORMAL,
                THEME.text,
            );
            let x = b.x + b.w - 50;
            c.round(
                x,
                b.y + 12,
                36,
                20,
                20,
                blend(THEME.border, THEME.accent, amount),
            );
            c.round(
                x + 3 + (16.0 * amount).round() as i32,
                b.y + 15,
                14,
                14,
                14,
                THEME.surface,
            );
        }
        let first = self.ui.bounds(Control::Option(0));
        c.text(
            "Refresh interval",
            MARGIN,
            first.y - 30,
            self.ui.width - 2 * MARGIN,
            22,
            FONT_BODY,
            WEIGHT_MEDIUM,
            THEME.text,
        );
        for (index, interval) in INTERVALS.iter().enumerate() {
            let control = Control::Option(index);
            let b = self.ui.bounds(control);
            let selected = self.cfg.interval == *interval;
            let base = if selected {
                THEME.accent_soft
            } else {
                THEME.surface
            };
            c.round(
                b.x,
                b.y,
                b.w,
                b.h,
                RADIUS_BUTTON,
                self.button_color(control, base, THEME.surface_alt),
            );
            c.text(
                &interval_label(*interval),
                b.x + 10,
                b.y,
                b.w - 20,
                b.h,
                FONT_BODY,
                if selected {
                    WEIGHT_MEDIUM
                } else {
                    WEIGHT_NORMAL
                },
                if selected { THEME.accent } else { THEME.text },
            );
            if selected {
                c.fill(b.x + 10, b.y + b.h - 3, b.w - 20, 2, THEME.accent);
            }
        }
    }
}

fn interval_label(n: u32) -> String {
    if n < 60 {
        format!("{n} sec")
    } else {
        format!("{} min", n / 60)
    }
}
