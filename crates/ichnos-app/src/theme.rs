use gpui_kit::{Hsla, rgb, rgba};

use crate::present::Tone;

/// Monospace font for code, numbers and badges.
pub const MONO: &str = "JetBrains Mono";

/// Font size of dense text: badges, captions, column headers, table rows and code. Nothing is
/// smaller; captions set themselves apart by color and weight instead.
pub const FS_SMALL: f32 = 12.;
/// Font size of primary text: a 13px base.
pub const FS_BASE: f32 = 13.;
/// Font size of panel titles.
pub const FS_TITLE: f32 = 14.;
/// Height of buttons, fields and segmented controls.
pub const CONTROL_HEIGHT: f32 = 28.;

/// Corner radius of tags and badges, and of segments nested inside a control.
pub const R_TAG: f32 = 4.;
/// Corner radius of buttons, fields and segmented controls.
pub const R_CONTROL: f32 = 6.;
/// Corner radius of cards, panels and selected list rows.
pub const R_CARD: f32 = 8.;
/// Corner radius of floating popups.
pub const R_POPUP: f32 = 12.;

/// A palette color.
pub fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// A palette color with alpha, as `0xRRGGBBAA`.
pub fn color_alpha(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub const BG: u32 = 0x001e_1f22;
pub const BAR_BG: u32 = 0x002b_2d30;
pub const PANEL: u32 = 0x002b_2d30;
pub const FIELD: u32 = 0x001e_1f22;
pub const SEG: u32 = 0x002b_2d30;
pub const SEG_ON: u32 = 0x0043_454a;
pub const LINE: u32 = 0x0039_3b40;
pub const LINE_2: u32 = 0x004e_5157;
pub const ROW_LINE: u32 = 0x002b_2d30;
pub const SOFT_LINE: u32 = 0x002e_3035;
pub const HOVER: u32 = 0x002e_3033;
pub const CARD: u32 = 0x002b_2d30;
pub const HOVER_LINE: u32 = 0x005a_5d63;

pub const FG: u32 = 0x00df_e1e5;
pub const BRIGHT: u32 = 0x00f0_f1f2;
pub const TEXT_2: u32 = 0x00ce_d0d6;
pub const MUTED: u32 = 0x0086_8a91;
pub const DIM: u32 = 0x006f_737a;
pub const QUIET: u32 = 0x00a1_a3ab;
pub const BTN: u32 = 0x00b4_b8bf;
pub const WHITE: u32 = 0x00ff_ffff;

pub const BLUE: u32 = 0x0054_8af7;
pub const BLUE_TEXT: u32 = 0x006b_9bfa;
pub const BLUE_LINE: u32 = 0x002e_436e;
pub const BLUE_BAR: u32 = 0x0037_5fad;
pub const TURN_BAR: u32 = 0x002e_436e;
pub const SEL: u32 = 0x002e_436e;
pub const TURN_OPEN: u32 = 0x0026_282e;

pub const GREEN: u32 = 0x005f_ad65;
pub const GREEN_BAR: u32 = 0x0057_965c;
pub const GREEN_TEXT: u32 = 0x007e_c482;
pub const LIVE_BG: u32 = 0x0025_3627;
pub const LIVE_TEXT: u32 = 0x006a_ab73;

pub const RED: u32 = 0x00f7_5464;
pub const RED_BAR: u32 = 0x00db_5c5c;
pub const RED_TEXT: u32 = 0x00f2_8b8b;
pub const RED_SOFT: u32 = 0x00f5_a3a3;
pub const RED_LINE: u32 = 0x006e_2b28;
pub const RED_BG: u32 = 0x0040_2929;

pub const PURPLE: u32 = 0x00c7_7dbb;
pub const PURPLE_TEXT: u32 = 0x00d9_a0cd;
pub const PURPLE_LINE: u32 = 0x005e_3f5a;
pub const PURPLE_BG: u32 = 0x002f_2433;
pub const SUB_BG: u32 = 0x0028_2a2f;
/// The purple of a finished subagent's bar, at 35% opacity.
pub const AGENT_BAR: u32 = 0xc77d_bb59;

pub const AMBER: u32 = 0x00f2_c55c;
pub const MODEL_BAR: u32 = 0x005a_5d63;
pub const EVENT_BAR: u32 = 0x005a_5d63;
pub const MARK: u32 = 0x004e_5157;
/// Background of a row that just appeared.
pub const FRESH: u32 = 0x003d_3a24;

/// Text color of a badge or status in `tone`.
pub fn tone_text(tone: Tone) -> Hsla {
    color(match tone {
        Tone::Ok => GREEN_TEXT,
        Tone::Err => RED_TEXT,
        Tone::Run => BLUE_TEXT,
        Tone::Agent => PURPLE_TEXT,
        Tone::Model | Tone::Event => MUTED,
    })
}

/// Color of a status icon in `tone`.
pub fn tone_icon(tone: Tone) -> Hsla {
    color(match tone {
        Tone::Ok => GREEN,
        Tone::Err => RED,
        Tone::Run => BLUE,
        Tone::Agent => PURPLE,
        Tone::Model | Tone::Event => DIM,
    })
}

/// Color of a timeline bar in `tone`.
pub fn tone_bar(tone: Tone) -> Hsla {
    match tone {
        Tone::Ok => color(GREEN_BAR),
        Tone::Err => color(RED_BAR),
        Tone::Run => color(BLUE),
        Tone::Agent => color_alpha(AGENT_BAR),
        Tone::Model => color(MODEL_BAR),
        Tone::Event => color(EVENT_BAR),
    }
}
