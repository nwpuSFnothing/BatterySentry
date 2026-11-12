use super::*;

pub fn reading_status(reading: &Reading, theme: Theme) -> (&'static str, u32) {
    match reading.value {
        Some(BatteryValue::Percent(n)) if n <= 10 => ("Critical", theme.danger),
        Some(BatteryValue::Percent(n)) if n <= 30 => ("Low battery", theme.warning),
        Some(BatteryValue::Percent(_)) => ("On battery", theme.success),
        Some(BatteryValue::Charging) => ("Charging", theme.accent),
        None if reading.last_known.is_some() => ("Cached", theme.muted),
        None => ("No reading", theme.muted),
    }
}

impl View<'_> {
    pub(super) fn paint_summary(&self, c: &Canvas<'_>) {
        let low = self
            .readings
            .iter()
            .filter(|r| matches!(r.value, Some(BatteryValue::Percent(n)) if n <= 30))
            .count();
        let charging = self
            .readings
            .iter()
            .filter(|r| r.value == Some(BatteryValue::Charging))
            .count();
        let mut summary = format!("{} devices", self.readings.len());
        if low > 0 {
            summary.push_str(&format!(" · {low} low"));
        }
        if charging > 0 {
            summary.push_str(&format!(" · {charging} charging"));
        }
        c.text(
            &summary,
            MARGIN,
            46,
            self.ui.width - 2 * MARGIN,
            22,
            FONT_BODY,
            WEIGHT_MEDIUM,
            THEME.text,
        );
        let status = if !self.scan_error.is_empty() {
            self.scan_error.to_owned()
        } else if !self.layout_status.is_empty() {
            self.layout_status.to_owned()
        } else if self.busy {
            "Reading batteries…".into()
        } else if let Some(last) = self.last {
            let seconds = last.elapsed().as_secs();
            if seconds < 5 {
                "Scan completed just now".into()
            } else if seconds < 60 {
                format!("Last scan {seconds}s ago")
            } else {
                format!("Last scan {}m ago", seconds / 60)
            }
        } else {
            "Waiting for the first scan".into()
        };
        c.text(
            &status,
            MARGIN,
            72,
            self.ui.width - 2 * MARGIN,
            18,
            FONT_SMALL,
            WEIGHT_NORMAL,
            THEME.muted,
        );
    }
    pub(super) fn paint_devices(&self, c: &Canvas<'_>) {
        let w = self.ui.width;
        if self.readings.is_empty() {
            let y = self.ui.content_top() + GAP;
            c.text(
                if self.busy {
                    "Looking for devices…"
                } else if self.scan_error.is_empty() {
                    "No supported devices found"
                } else {
                    "Could not read devices"
                },
                MARGIN,
                y,
                w - 2 * MARGIN,
                26,
                FONT_BODY,
                WEIGHT_MEDIUM,
                THEME.text,
            );
            c.text(
                "Connect a supported receiver and refresh.",
                MARGIN,
                y + 32,
                w - 2 * MARGIN,
                22,
                FONT_SMALL,
                WEIGHT_NORMAL,
                THEME.muted,
            );
        }
        for index in self.ui.visible(self.readings.len()) {
            let reading = &self.readings[index];
            let y = LIST_TOP + index as i32 * ROW_HEIGHT - self.ui.scroll;
            let (label, ink) = reading_status(reading, THEME);
            c.round(
                MARGIN,
                y,
                w - 2 * MARGIN,
                ROW_HEIGHT - GAP,
                RADIUS_CARD,
                THEME.border,
            );
            c.round(
                MARGIN + 1,
                y + 1,
                w - 2 * MARGIN - 2,
                ROW_HEIGHT - GAP - 2,
                RADIUS_CARD - 1,
                THEME.surface,
            );
            c.round(26, y + 20, 28, 32, RADIUS_BUTTON, THEME.surface_alt);
            c.round(31, y + 30, 16, 11, 3, ink);
            c.round(33, y + 32, 12, 7, 2, THEME.surface);
            c.fill(47, y + 33, 2, 5, ink);
            let value = reading.value_text();
            let value_width = c
                .text_width(&value, FONT_VALUE, WEIGHT_MEDIUM as i32)
                .max(58);
            let right = w - MARGIN - 12;
            let value_x = right - value_width;
            c.text(
                &reading.name,
                64,
                y + 8,
                (value_x - 72).max(0),
                26,
                FONT_BODY,
                WEIGHT_MEDIUM,
                THEME.text,
            );
            c.right(
                &value,
                Bounds {
                    x: value_x,
                    y: y + 6,
                    w: value_width,
                    h: 28,
                },
                FONT_VALUE,
                WEIGHT_MEDIUM,
                ink,
            );
            let label_width = c.text_width(label, FONT_SMALL, WEIGHT_NORMAL as i32) + 12;
            let label_x = right - label_width;
            let detail_right = if reading.value == Some(BatteryValue::Charging) {
                right
            } else {
                c.round(
                    label_x,
                    y + 37,
                    label_width,
                    20,
                    RADIUS_BUTTON,
                    blend(THEME.surface, ink, 0.09),
                );
                c.text(
                    label,
                    label_x + 6,
                    y + 37,
                    label_width - 12,
                    20,
                    FONT_SMALL,
                    WEIGHT_NORMAL,
                    ink,
                );
                label_x - GAP
            };
            c.text(
                &reading.detail,
                64,
                y + 36,
                (detail_right - 64).max(0),
                20,
                FONT_SMALL,
                WEIGHT_NORMAL,
                THEME.muted,
            );
            // Charging carries no percentage. Cached percentages retain their muted styling.
            if let Some(BatteryValue::Percent(percent)) = reading.value.or(reading.last_known) {
                let width = right - 64;
                c.round(64, y + 65, width, 4, 4, THEME.surface_alt);
                let filled = width * i32::from(percent.min(100)) / 100;
                if filled > 0 {
                    c.round(64, y + 65, filled, 4, 4, ink);
                }
                c.fill(34, y + 33, 10 * i32::from(percent.min(100)) / 100, 5, ink);
            } else if reading.value == Some(BatteryValue::Charging) {
                c.line(40, y + 24, 37, y + 35, THEME.accent);
                c.line(37, y + 35, 42, y + 35, THEME.accent);
                c.line(42, y + 35, 39, y + 46, THEME.accent);
            }
        }
    }
}

pub fn paint_widget(c: &Canvas<'_>, readings: &[Reading], theme: Theme, key: u32) {
    c.clear(key);
    let half = c.height() / 2;
    let font = FONT_WIDGET.min(half - 2).max(8);
    let mut rows: Vec<_> = readings
        .iter()
        .take(2)
        .map(|r| (r.name.clone(), r.value_text(), reading_status(r, theme).1))
        .collect();
    if readings.len() > 2
        && let Some(last) = rows.last_mut()
    {
        last.0 = format!("{} (+{})", last.0, readings.len() - 2);
    }
    if rows.is_empty() {
        rows.push(("BatterySentry".into(), "--".into(), theme.muted));
    }
    let pad = 2;
    let gap = 4;
    let value_width = rows
        .iter()
        .map(|(_, v, _)| c.text_width(v, font, WEIGHT_MEDIUM as i32))
        .max()
        .unwrap_or(0)
        .max(20);
    let name_width = rows
        .iter()
        .map(|(n, _, _)| c.text_width(n, font, WEIGHT_MEDIUM as i32))
        .max()
        .unwrap_or(0)
        .min((c.width() - 2 * pad - gap - value_width).max(1));
    let name_x = (c.width() - pad - value_width - gap - name_width).max(pad);
    let value_x = name_x + name_width + gap;
    c.round(
        name_x - pad,
        0,
        c.width() - name_x + pad,
        c.height(),
        RADIUS_CARD,
        theme.surface,
    );
    for (row, (name, value, ink)) in rows.iter().enumerate() {
        let y = row as i32 * half;
        c.text(
            name,
            name_x,
            y,
            name_width,
            half,
            font,
            WEIGHT_MEDIUM,
            theme.text,
        );
        c.right(
            value,
            Bounds {
                x: value_x,
                y,
                w: value_width,
                h: half,
            },
            font,
            WEIGHT_MEDIUM,
            *ink,
        );
    }
}
