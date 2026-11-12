//! Panel presentation. Application and device ownership remain in app.rs and drivers.rs.
mod components;
pub mod devices;
pub mod layout;
pub mod renderer;
mod settings;
pub mod widget;

use crate::{
    config::{Config, INTERVALS},
    drivers::Reading,
    panel::{Bounds, Control, Page, Panel},
    protocol::BatteryValue,
    theme::*,
};
use components::blend;
use renderer::Canvas;
use std::time::Instant;
const THEME: Theme = Theme::LIGHT;

pub struct View<'a> {
    pub ui: &'a Panel,
    pub readings: &'a [Reading],
    pub cfg: &'a Config,
    pub busy: bool,
    pub last: Option<Instant>,
    pub scan_error: &'a str,
    pub layout_status: &'a str,
}
impl View<'_> {
    pub fn paint(&self, c: &Canvas<'_>) {
        c.clear(THEME.background);
        self.paint_header(c);
        if self.ui.page == Page::Devices {
            self.paint_summary(c);
        }
        c.clip(
            Bounds {
                x: MARGIN,
                y: self.ui.content_top(),
                w: self.ui.width - 2 * MARGIN,
                h: self.ui.list_height(),
            },
            || {
                match self.ui.page {
                    Page::Devices => self.paint_devices(c),
                    Page::Settings => self.paint_settings(c),
                }
                if let Some(control) = self.ui.focused
                    && matches!(
                        control,
                        Control::Startup | Control::Tray | Control::Option(_)
                    )
                {
                    self.paint_focus(c, control);
                }
            },
        );
        if let Some(thumb) = self.ui.thumb(self.readings.len()) {
            c.round(
                thumb.x,
                thumb.y,
                thumb.w,
                thumb.h,
                RADIUS_BUTTON,
                self.button_color(Control::Scrollbar, THEME.border, THEME.muted),
            );
        }
        self.paint_actions(c);
        if let Some(control) = self.ui.focused
            && !matches!(
                control,
                Control::Startup | Control::Tray | Control::Option(_) | Control::Scrollbar
            )
        {
            self.paint_focus(c, control);
        }
    }
}

#[cfg(test)]
mod checks;
