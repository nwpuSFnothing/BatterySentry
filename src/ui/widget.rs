//! Stable GDI rendering for the color-keyed Explorer child window.
pub use super::gdi::Renderer as WidgetRenderer;
use super::{devices::reading_status, gdi::Canvas};
use crate::{drivers::Reading, panel::Bounds, theme::*};

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
