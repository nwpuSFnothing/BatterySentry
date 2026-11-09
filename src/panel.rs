//! Logical-pixel geometry and interaction state for the virtual device list.
use std::{ops::Range, time::Instant};

pub const ROW_HEIGHT: i32 = 80;
pub const LIST_TOP: i32 = 100;
pub const FOOTER_HEIGHT: i32 = 190;
pub const MIN_WIDTH: i32 = 360;
pub const MIN_HEIGHT: i32 = 370;
pub const ANIMATION_TIMER: usize = 5;

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
pub enum Control {
    Close,
    Minimize,
    Startup,
    Tray,
    Interval,
    Refresh,
    Exit,
    Scrollbar,
    Option(usize),
}
impl Control {
    pub fn index(self) -> usize {
        match self {
            Self::Close => 0,
            Self::Minimize => 1,
            Self::Startup => 2,
            Self::Interval => 3,
            Self::Refresh => 4,
            Self::Exit => 5,
            Self::Scrollbar => 6,
            Self::Tray => 7,
            Self::Option(i) => 8 + i,
        }
    }
}

#[derive(Debug)]
pub struct Panel {
    pub width: i32,
    pub height: i32,
    pub scroll: i32,
    pub hovered: Option<Control>,
    pub pressed: Option<Control>,
    pub focused: Option<Control>,
    pub dropdown: bool,
    pub drag_offset: Option<i32>,
    pub hover: [f32; 14],
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
            scroll: 0,
            hovered: None,
            pressed: None,
            focused: None,
            dropdown: false,
            drag_offset: None,
            hover: [0.0; 14],
            switch: if startup { 1.0 } else { 0.0 },
            tray_switch: 1.0,
            tray_target: true,
            animations: true,
            last_tick: Instant::now(),
            wheel_remainder: 0,
        }
    }
    pub fn list_height(&self) -> i32 {
        (self.height - FOOTER_HEIGHT - LIST_TOP).max(1)
    }
    pub fn footer(&self) -> i32 {
        self.height - FOOTER_HEIGHT
    }
    pub fn max_scroll(&self, count: usize) -> i32 {
        ((count.min(100_000) as i32 * ROW_HEIGHT) - self.list_height()).max(0)
    }
    pub fn clamp_scroll(&mut self, count: usize) {
        self.scroll = self.scroll.clamp(0, self.max_scroll(count));
    }
    pub fn visible(&self, count: usize) -> Range<usize> {
        let first = (self.scroll / ROW_HEIGHT) as usize;
        let last = ((self.scroll + self.list_height() + ROW_HEIGHT - 1) / ROW_HEIGHT) as usize;
        first.min(count)..last.min(count)
    }
    pub fn bounds(&self, control: Control) -> Bounds {
        let f = self.footer();
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
                x: 16,
                y: f + 8,
                w: (self.width - 32) / 2,
                h: 48,
            },
            Control::Tray => Bounds {
                x: self.width / 2,
                y: f + 8,
                w: (self.width - 32) / 2,
                h: 48,
            },
            Control::Interval => Bounds {
                x: self.width - 160,
                y: f + 64,
                w: 136,
                h: 36,
            },
            Control::Refresh => Bounds {
                x: 24,
                y: f + 118,
                w: self.width - 148,
                h: 40,
            },
            Control::Exit => Bounds {
                x: self.width - 112,
                y: f + 118,
                w: 88,
                h: 40,
            },
            Control::Scrollbar => Bounds {
                x: self.width - 14,
                y: LIST_TOP,
                w: 10,
                h: self.list_height(),
            },
            Control::Option(i) => {
                let menu = self.menu_bounds();
                let pitch = ((menu.h - 38) / 6).max(1);
                Bounds {
                    x: menu.x + 8,
                    y: menu.y + 30 + i as i32 * pitch,
                    w: menu.w - 16,
                    h: (pitch - 4).max(1),
                }
            }
        }
    }
    pub fn menu_bounds(&self) -> Bounds {
        let bottom = self.footer() + 58;
        let height = 258.min((bottom - 8).max(1));
        Bounds {
            x: self.width - 160,
            y: bottom - height,
            w: 136,
            h: height,
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
            y: track.y + (i64::from(self.scroll) * i64::from(track.h - h) / i64::from(max)) as i32,
            w: track.w - 4,
            h,
        })
    }
    pub fn hit(&self, x: i32, y: i32, count: usize) -> Option<Control> {
        if self.dropdown {
            if let Some(i) = (0..6).find(|i| self.bounds(Control::Option(*i)).contains(x, y)) {
                return Some(Control::Option(i));
            }
            if self.menu_bounds().contains(x, y) {
                return None;
            }
        }
        [
            Control::Close,
            Control::Minimize,
            Control::Startup,
            Control::Tray,
            Control::Interval,
            Control::Refresh,
            Control::Exit,
        ]
        .into_iter()
        .find(|c| self.bounds(*c).contains(x, y))
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
                self.scroll = (i64::from((y - offset - track.y).clamp(0, travel))
                    * i64::from(self.max_scroll(count))
                    / i64::from(travel)) as i32;
            }
        }
    }
    pub fn wheel(&mut self, delta: i32, count: usize) {
        self.wheel_remainder += delta;
        let steps = self.wheel_remainder / 120;
        self.wheel_remainder %= 120;
        self.scroll -= steps * 48;
        self.clamp_scroll(count);
    }
    pub fn restart_animation(&mut self) {
        self.last_tick = Instant::now();
    }
    /// Returns true only while a transition still needs frames.
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
        let mut p = Panel::new(460, 530, false);
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
        let menu = p.menu_bounds();
        let first = p.bounds(Control::Option(0));
        let last = p.bounds(Control::Option(5));
        assert_eq!(first.x, last.x);
        assert!(last.y > first.y);
        assert!(menu.y >= 0 && last.y + last.h <= menu.y + menu.h);
    }
}
