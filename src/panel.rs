//! Logical-pixel geometry and interaction state for the device and settings pages.
pub use crate::theme::{LIST_TOP, MIN_HEIGHT, MIN_WIDTH, ROW_HEIGHT};
use crate::theme::{SETTINGS_HEIGHT, SETTINGS_TOP};
pub use crate::ui::layout::{Bounds, Control, Page};
use std::time::Instant;
pub const ANIMATION_TIMER: usize = 5;
#[derive(Debug)]
pub struct Panel {
    pub width: i32,
    pub height: i32,
    pub page: Page,
    pub scroll: i32,
    pub settings_scroll: i32,
    pub hovered: Option<Control>,
    pub pressed: Option<Control>,
    pub focused: Option<Control>,
    pub drag_offset: Option<i32>,
    pub hover: [f32; 15],
    pub switch: f32,
    pub tray_switch: f32,
    pub tray_target: bool,
    pub animations: bool,
    last_tick: Instant,
    pub wheel_remainder: i32,
}
impl Panel {
    pub fn new(width: i32, height: i32, startup: bool) -> Self {
        Self {
            width,
            height,
            page: Page::Devices,
            scroll: 0,
            settings_scroll: 0,
            hovered: None,
            pressed: None,
            focused: None,
            drag_offset: None,
            hover: [0.0; 15],
            switch: if startup { 1.0 } else { 0.0 },
            tray_switch: 1.0,
            tray_target: true,
            animations: true,
            last_tick: Instant::now(),
            wheel_remainder: 0,
        }
    }
    pub fn show_page(&mut self, page: Page) {
        self.page = page;
        self.hovered = None;
        self.pressed = None;
        self.focused = None;
        self.drag_offset = None;
        self.hover.fill(0.0);
        self.wheel_remainder = 0;
    }
    pub fn scroll_offset(&self) -> i32 {
        match self.page {
            Page::Devices => self.scroll,
            Page::Settings => self.settings_scroll,
        }
    }
    pub fn set_scroll(&mut self, offset: i32, count: usize) {
        let offset = offset.clamp(0, self.max_scroll(count));
        match self.page {
            Page::Devices => self.scroll = offset,
            Page::Settings => self.settings_scroll = offset,
        }
    }
    pub fn clamp_scroll(&mut self, count: usize) {
        self.scroll = self.scroll.clamp(
            0,
            (count.min(100_000) as i32 * ROW_HEIGHT - (self.footer() - LIST_TOP)).max(0),
        );
        self.settings_scroll = self
            .settings_scroll
            .clamp(0, (SETTINGS_HEIGHT - (self.footer() - SETTINGS_TOP)).max(0));
    }
    pub fn drag_scrollbar(&mut self, y: i32, count: usize) {
        if let (Some(offset), Some(thumb)) = (self.drag_offset, self.thumb(count)) {
            let track = self.bounds(Control::Scrollbar);
            let travel = track.h - thumb.h;
            if travel > 0 {
                let offset = (i64::from((y - offset - track.y).clamp(0, travel))
                    * i64::from(self.max_scroll(count))
                    / i64::from(travel)) as i32;
                self.set_scroll(offset, count);
            }
        }
    }
    pub fn wheel(&mut self, delta: i32, count: usize) {
        self.wheel_remainder += delta;
        let steps = self.wheel_remainder / 120;
        self.wheel_remainder %= 120;
        self.set_scroll(self.scroll_offset() - steps * 48, count);
    }
    pub fn restart_animation(&mut self) {
        self.last_tick = Instant::now();
    }
    pub fn tick(&mut self, startup: bool) -> bool {
        let now = Instant::now();
        let step = if self.animations {
            now.duration_since(self.last_tick).as_secs_f32() / 0.12
        } else {
            1.0
        };
        self.last_tick = now;
        let mut running = false;
        for (i, value) in self.hover.iter_mut().enumerate() {
            let target = if self.hovered.is_some_and(|c| c.index() == i) {
                1.0
            } else {
                0.0
            };
            *value += (target - *value).clamp(-step, step);
            running |= (*value - target).abs() > 0.001;
        }
        let target = if startup { 1.0 } else { 0.0 };
        self.switch += (target - self.switch).clamp(-step, step);
        running |= (self.switch - target).abs() > 0.001;
        let target = if self.tray_target { 1.0 } else { 0.0 };
        self.tray_switch += (target - self.tray_switch).clamp(-step, step);
        running | ((self.tray_switch - target).abs() > 0.001)
    }
}
