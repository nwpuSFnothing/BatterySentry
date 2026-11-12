//! Logical-pixel geometry and interaction state for the device and settings pages.
use std::{ops::Range, time::Instant};

pub const ROW_HEIGHT: i32 = 92;
pub const LIST_TOP: i32 = 156;
pub const FOOTER_HEIGHT: i32 = 72;
pub const MIN_WIDTH: i32 = 360;
pub const MIN_HEIGHT: i32 = 370;
pub const ANIMATION_TIMER: usize = 5;
const SETTINGS_TOP: i32 = 80;
const SETTINGS_HEIGHT: i32 = 412;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
impl Bounds {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Devices,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Close,
    Minimize,
    Startup,
    Tray,
    Refresh,
    Exit,
    Scrollbar,
    Settings,
    Back,
    Option(usize),
}
impl Control {
    pub fn index(self) -> usize {
        match self {
            Self::Close => 0,
            Self::Minimize => 1,
            Self::Startup => 2,
            Self::Tray => 3,
            Self::Refresh => 4,
            Self::Exit => 5,
            Self::Scrollbar => 6,
            Self::Settings => 7,
            Self::Back => 8,
            Self::Option(i) => 9 + i,
        }
    }
}

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
    pub fn content_top(&self) -> i32 {
        match self.page {
            Page::Devices => LIST_TOP,
            Page::Settings => SETTINGS_TOP,
        }
    }
    pub fn list_height(&self) -> i32 {
        (self.footer() - self.content_top()).max(1)
    }
    pub fn footer(&self) -> i32 {
        self.height - FOOTER_HEIGHT
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
    pub fn max_scroll(&self, count: usize) -> i32 {
        let content = match self.page {
            Page::Devices => count.min(100_000) as i32 * ROW_HEIGHT,
            Page::Settings => SETTINGS_HEIGHT,
        };
        (content - self.list_height()).max(0)
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
    pub fn visible(&self, count: usize) -> Range<usize> {
        let first = (self.scroll / ROW_HEIGHT) as usize;
        let last = ((self.scroll + self.list_height() + ROW_HEIGHT - 1) / ROW_HEIGHT) as usize;
        first.min(count)..last.min(count)
    }
    pub fn bounds(&self, control: Control) -> Bounds {
        let f = self.footer();
        let settings_y = SETTINGS_TOP - self.settings_scroll;
        match control {
            Control::Close => Bounds {
                x: self.width - 40,
                y: 0,
                w: 40,
                h: 40,
            },
            Control::Minimize => Bounds {
                x: self.width - 80,
                y: 0,
                w: 40,
                h: 40,
            },
            Control::Startup => Bounds {
                x: 24,
                y: settings_y,
                w: self.width - 48,
                h: 52,
            },
            Control::Tray => Bounds {
                x: 24,
                y: settings_y + 60,
                w: self.width - 48,
                h: 52,
            },
            Control::Refresh => Bounds {
                x: 24,
                y: f + 14,
                w: self.width - 160,
                h: 40,
            },
            Control::Settings => Bounds {
                x: self.width - 124,
                y: f + 14,
                w: 100,
                h: 40,
            },
            Control::Back => Bounds {
                x: 24,
                y: f + 14,
                w: 100,
                h: 40,
            },
            Control::Exit => Bounds {
                x: self.width - 124,
                y: f + 14,
                w: 100,
                h: 40,
            },
            Control::Scrollbar => Bounds {
                x: self.width - 14,
                y: self.content_top(),
                w: 10,
                h: self.list_height(),
            },
            Control::Option(i) => Bounds {
                x: 24,
                y: settings_y + 156 + i as i32 * 36,
                w: self.width - 48,
                h: 32,
            },
        }
    }
    pub fn controls(&self) -> Vec<Control> {
        let mut controls = vec![Control::Minimize, Control::Close];
        match self.page {
            Page::Devices => controls.extend([Control::Refresh, Control::Settings]),
            Page::Settings => {
                controls.extend([Control::Startup, Control::Tray]);
                controls.extend((0..6).map(Control::Option));
                controls.extend([Control::Back, Control::Exit]);
            }
        }
        controls
    }
    pub fn ensure_visible(&mut self, control: Control, count: usize) {
        if self.page == Page::Settings
            && matches!(
                control,
                Control::Startup | Control::Tray | Control::Option(_)
            )
        {
            let b = self.bounds(control);
            let delta = if b.y < self.content_top() {
                b.y - self.content_top()
            } else {
                (b.y + b.h - self.footer()).max(0)
            };
            self.set_scroll(self.scroll_offset() + delta, count);
        }
    }
    pub fn thumb(&self, count: usize) -> Option<Bounds> {
        let max = self.max_scroll(count);
        if max == 0 {
            return None;
        }
        let track = self.bounds(Control::Scrollbar);
        let h = (track.h * track.h / (max + track.h)).max(24).min(track.h);
        Some(Bounds {
            x: track.x + 2,
            y: track.y
                + (i64::from(self.scroll_offset()) * i64::from(track.h - h) / i64::from(max))
                    as i32,
            w: track.w - 4,
            h,
        })
    }
    pub fn hit(&self, x: i32, y: i32, count: usize) -> Option<Control> {
        self.controls()
            .into_iter()
            .find(|c| {
                let in_content = matches!(c, Control::Startup | Control::Tray | Control::Option(_));
                (!in_content || (y >= self.content_top() && y < self.footer()))
                    && self.bounds(*c).contains(x, y)
            })
            .or_else(|| {
                (self.max_scroll(count) > 0 && self.bounds(Control::Scrollbar).contains(x, y))
                    .then_some(Control::Scrollbar)
            })
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_rows_resize_and_scrollbar_stay_in_bounds() {
        let mut p = Panel::new(460, 504, false);
        assert_eq!(p.visible(100), 0..3);
        p.wheel(-240, 100);
        assert_eq!(p.visible(100), 1..5);
        p.height = 690;
        assert_eq!(p.visible(100), 1..7);
        let thumb = p.thumb(100).expect("long list has a thumb");
        p.drag_offset = Some(thumb.h / 2);
        p.drag_scrollbar(100_000, 100);
        assert_eq!(p.scroll, p.max_scroll(100));
        p.clamp_scroll(1);
        assert_eq!(p.scroll, 0);
        assert!(p.thumb(1).is_none());
        assert_eq!(p.bounds(Control::Close).w, p.bounds(Control::Minimize).w);
        p.height = MIN_HEIGHT;
        p.show_page(Page::Settings);
        let first = p.bounds(Control::Option(0));
        let last = p.bounds(Control::Option(5));
        assert_eq!(first.x, last.x);
        assert!(last.y > first.y);
        p.ensure_visible(Control::Option(5), 1);
        let last = p.bounds(Control::Option(5));
        assert!(last.y >= p.content_top() && last.y + last.h <= p.footer());
        assert_eq!(p.hit(last.x + 10, last.y + 10, 1), Some(Control::Option(5)));
        assert!(p.settings_scroll > 0);
        p.show_page(Page::Devices);
        assert_eq!(p.scroll, 0);
    }
}
