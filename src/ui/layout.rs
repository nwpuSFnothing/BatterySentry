use crate::{panel::Panel, theme::*};
use std::ops::Range;

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

impl Panel {
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
    pub fn max_scroll(&self, count: usize) -> i32 {
        let content = match self.page {
            Page::Devices => count.min(100_000) as i32 * ROW_HEIGHT,
            Page::Settings => SETTINGS_HEIGHT,
        };
        (content - self.list_height()).max(0)
    }
    pub fn visible(&self, count: usize) -> Range<usize> {
        let first = (self.scroll / ROW_HEIGHT) as usize;
        let last = ((self.scroll + self.list_height() + ROW_HEIGHT - 1) / ROW_HEIGHT) as usize;
        first.min(count)..last.min(count)
    }
    pub fn bounds(&self, control: Control) -> Bounds {
        let f = self.footer();
        let y = SETTINGS_TOP - self.settings_scroll;
        let inner = self.width - 2 * MARGIN;
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
                x: MARGIN,
                y,
                w: inner,
                h: 44,
            },
            Control::Tray => Bounds {
                x: MARGIN,
                y: y + 52,
                w: inner,
                h: 44,
            },
            Control::Refresh => Bounds {
                x: MARGIN,
                y: f + 12,
                w: inner - 108,
                h: BUTTON_HEIGHT,
            },
            Control::Settings => Bounds {
                x: self.width - MARGIN - 100,
                y: f + 12,
                w: 100,
                h: BUTTON_HEIGHT,
            },
            Control::Back => Bounds {
                x: MARGIN,
                y: f + 12,
                w: 100,
                h: BUTTON_HEIGHT,
            },
            Control::Exit => Bounds {
                x: self.width - MARGIN - 100,
                y: f + 12,
                w: 100,
                h: BUTTON_HEIGHT,
            },
            Control::Scrollbar => Bounds {
                x: self.width - 12,
                y: self.content_top(),
                w: 8,
                h: self.list_height(),
            },
            Control::Option(i) => {
                let col = (i % 3) as i32;
                let x = MARGIN + col * (inner + GAP) / 3;
                let right = MARGIN + (col + 1) * (inner + GAP) / 3 - GAP;
                Bounds {
                    x,
                    y: y + 144 + (i / 3) as i32 * 40,
                    w: right - x,
                    h: 34,
                }
            }
        }
    }
    pub fn controls(&self) -> &'static [Control] {
        match self.page {
            Page::Devices => &[
                Control::Minimize,
                Control::Close,
                Control::Refresh,
                Control::Settings,
            ],
            Page::Settings => &[
                Control::Minimize,
                Control::Close,
                Control::Startup,
                Control::Tray,
                Control::Option(0),
                Control::Option(1),
                Control::Option(2),
                Control::Option(3),
                Control::Option(4),
                Control::Option(5),
                Control::Exit,
                Control::Back,
            ],
        }
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
            .iter()
            .copied()
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
}

pub fn preferred_height(page: Page, count: usize) -> i32 {
    match page {
        Page::Devices => {
            (LIST_TOP + count.clamp(1, 3) as i32 * ROW_HEIGHT + FOOTER_HEIGHT).max(MIN_HEIGHT)
        }
        Page::Settings => SETTINGS_TOP + SETTINGS_HEIGHT + FOOTER_HEIGHT,
    }
}
