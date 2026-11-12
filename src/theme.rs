//! Colors shared by the light panel and system-themed taskbar widget.
pub const FONT_FAMILY: &str = "Segoe UI";
pub const FONT_TITLE: i32 = 20;
pub const FONT_BODY: i32 = 15;
pub const FONT_SMALL: i32 = 13;
pub const FONT_VALUE: i32 = 24;
pub const FONT_WIDGET: i32 = 18;
pub const WEIGHT_NORMAL: u32 = 400;
pub const WEIGHT_MEDIUM: u32 = 600;
pub const MARGIN: i32 = 16;
pub const GAP: i32 = 8;
pub const RADIUS_CARD: i32 = 12;
pub const RADIUS_BUTTON: i32 = 8;
pub const BUTTON_HEIGHT: i32 = 36;
pub const ROW_HEIGHT: i32 = 88;
pub const LIST_TOP: i32 = 104;
pub const FOOTER_HEIGHT: i32 = 60;
pub const SETTINGS_TOP: i32 = 72;
pub const SETTINGS_HEIGHT: i32 = 264;
pub const MIN_WIDTH: i32 = 360;
pub const MIN_HEIGHT: i32 = 280;
pub const DEFAULT_WIDTH: i32 = 420;
pub const DEFAULT_HEIGHT: i32 = LIST_TOP + 2 * ROW_HEIGHT + FOOTER_HEIGHT;
pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub background: u32,
    pub surface: u32,
    pub surface_alt: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_soft: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
}

impl Theme {
    pub const LIGHT: Self = Self {
        background: rgb(246, 248, 251),
        surface: rgb(255, 255, 255),
        surface_alt: rgb(240, 244, 248),
        text: rgb(27, 36, 48),
        muted: rgb(99, 114, 132),
        border: rgb(227, 234, 242),
        accent: rgb(57, 103, 222),
        accent_hover: rgb(42, 83, 191),
        accent_soft: rgb(231, 239, 255),
        success: rgb(20, 123, 82),
        warning: rgb(157, 97, 12),
        danger: rgb(190, 55, 61),
    };

    pub const DARK: Self = Self {
        background: rgb(25, 30, 39),
        surface: rgb(34, 41, 53),
        surface_alt: rgb(45, 54, 68),
        text: rgb(235, 240, 247),
        muted: rgb(165, 179, 196),
        border: rgb(62, 74, 91),
        accent: rgb(139, 176, 255),
        accent_hover: rgb(163, 193, 255),
        accent_soft: rgb(42, 59, 89),
        success: rgb(94, 223, 165),
        warning: rgb(244, 197, 103),
        danger: rgb(249, 115, 123),
    };
}
