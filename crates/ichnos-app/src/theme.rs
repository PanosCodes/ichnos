use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::theme::{Theme, ThemeMode};
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

/// A palette entry: its dark and its light appearance, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy)]
pub struct Tok(u32, u32);

static LIGHT: AtomicBool = AtomicBool::new(false);

/// Switch every palette color to its light (`true`) or dark appearance.
pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

/// Follow the system appearance: the palette and GPUI Kit's own widgets switch together.
pub fn apply_appearance(appearance: gpui_kit::WindowAppearance, cx: &mut gpui_kit::App) {
    let mode = if matches!(
        appearance,
        gpui_kit::WindowAppearance::Light | gpui_kit::WindowAppearance::VibrantLight
    ) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    set_light(mode == ThemeMode::Light);
    Theme::change(mode, None, cx);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// A palette color in the current appearance.
pub fn color(tok: Tok) -> Hsla {
    rgb(if is_light() { tok.1 } else { tok.0 }).into()
}

/// A palette color with alpha, as `0xRRGGBBAA`.
pub fn color_alpha(hex: u32) -> Hsla {
    rgba(hex).into()
}

pub const BG: Tok = Tok(0x001e_1f22, 0x00ff_ffff);
pub const BAR_BG: Tok = Tok(0x002b_2d30, 0x00f4_f4f6);
pub const PANEL: Tok = Tok(0x002b_2d30, 0x00f7_f7f9);
pub const FIELD: Tok = Tok(0x001e_1f22, 0x00ff_ffff);
pub const SEG: Tok = Tok(0x002b_2d30, 0x00e9_e9ec);
pub const SEG_ON: Tok = Tok(0x0043_454a, 0x00ff_ffff);
pub const LINE: Tok = Tok(0x0039_3b40, 0x00d8_d9de);
pub const LINE_2: Tok = Tok(0x004e_5157, 0x00c4_c6cc);
pub const ROW_LINE: Tok = Tok(0x002b_2d30, 0x00f0_f0f3);
pub const SOFT_LINE: Tok = Tok(0x002e_3035, 0x00e6_e7ea);
pub const HOVER: Tok = Tok(0x002e_3033, 0x00f0_f1f4);
pub const CARD: Tok = Tok(0x002b_2d30, 0x00ff_ffff);
pub const HOVER_LINE: Tok = Tok(0x005a_5d63, 0x009a_9da5);

pub const FG: Tok = Tok(0x00df_e1e5, 0x0025_272b);
pub const BRIGHT: Tok = Tok(0x00f0_f1f2, 0x000d_0e10);
pub const TEXT_2: Tok = Tok(0x00ce_d0d6, 0x003a_3d43);
pub const MUTED: Tok = Tok(0x0086_8a91, 0x005e_626a);
pub const DIM: Tok = Tok(0x006f_737a, 0x006b_6f77);
pub const QUIET: Tok = Tok(0x00a1_a3ab, 0x0058_5c64);
pub const BTN: Tok = Tok(0x00b4_b8bf, 0x004a_4d54);
pub const WHITE: Tok = Tok(0x00ff_ffff, 0x000d_0e10);

pub const BLUE: Tok = Tok(0x0054_8af7, 0x002f_6df0);
pub const BLUE_TEXT: Tok = Tok(0x006b_9bfa, 0x001f_55d1);
pub const BLUE_LINE: Tok = Tok(0x002e_436e, 0x00ae_c6f7);
pub const BLUE_BAR: Tok = Tok(0x0037_5fad, 0x005b_8ef2);
pub const TURN_BAR: Tok = Tok(0x002e_436e, 0x00ae_c6f7);
pub const SEL: Tok = Tok(0x002e_436e, 0x00dc_e8fe);
pub const TURN_OPEN: Tok = Tok(0x0026_282e, 0x00f3_f4f7);

pub const GREEN: Tok = Tok(0x005f_ad65, 0x002e_8b3a);
pub const GREEN_BAR: Tok = Tok(0x0057_965c, 0x003a_9a46);
pub const GREEN_TEXT: Tok = Tok(0x007e_c482, 0x001f_7a2c);
pub const LIVE_BG: Tok = Tok(0x0025_3627, 0x00e0_f3e3);
pub const LIVE_TEXT: Tok = Tok(0x006a_ab73, 0x001f_7a2c);

pub const RED: Tok = Tok(0x00f7_5464, 0x00d9_2d3d);
pub const RED_BAR: Tok = Tok(0x00db_5c5c, 0x00d0_4545);
pub const RED_TEXT: Tok = Tok(0x00f2_8b8b, 0x00b3_202f);
pub const RED_SOFT: Tok = Tok(0x00f5_a3a3, 0x00b3_202f);
pub const RED_LINE: Tok = Tok(0x006e_2b28, 0x00f0_b4b4);
pub const RED_BG: Tok = Tok(0x0040_2929, 0x00fd_e8e8);

pub const PURPLE: Tok = Tok(0x00c7_7dbb, 0x009c_3f8e);
pub const PURPLE_TEXT: Tok = Tok(0x00d9_a0cd, 0x0086_307a);
pub const PURPLE_LINE: Tok = Tok(0x005e_3f5a, 0x00dc_b3d4);
pub const PURPLE_BG: Tok = Tok(0x002f_2433, 0x00f6_e9f3);
pub const SUB_BG: Tok = Tok(0x0028_2a2f, 0x00f8_f8fb);
/// The purple of a finished subagent's bar, at 35% opacity.
pub const AGENT_BAR: u32 = 0xc77d_bb59;

pub const AMBER: Tok = Tok(0x00f2_c55c, 0x00b8_7a00);
pub const MODEL_BAR: Tok = Tok(0x005a_5d63, 0x009a_9da5);
pub const EVENT_BAR: Tok = Tok(0x005a_5d63, 0x009a_9da5);
pub const MARK: Tok = Tok(0x004e_5157, 0x00c4_c6cc);
/// Background of a row that just appeared.
pub const FRESH: Tok = Tok(0x003d_3a24, 0x00ff_f4c7);

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
