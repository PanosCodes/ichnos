//! Headless screenshots of the viewer (the `screenshots` feature): every view, in light and dark,
//! at a narrow and a wide window, rendered by Metal without showing a window.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::Assets;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, HeadlessAppContext, WindowAppearance, WindowBounds, WindowOptions, px,
    size,
};
use ichnos_core::store::SessionId;

use crate::theme;
use crate::view_state::View;
use crate::viewer::{Options, Viewer, bind_keys};

/// States shot, besides the view as opened (see `Viewer::apply_shot`).
const STATES: [&str; 8] = [
    "", "selected", "turn", "replay", "switcher", "ask", "help", "settings",
];

/// Window widths shot.
const WIDTHS: [f32; 2] = [900., 1400.];

/// Write `<mode>-<width>-<view>.png` for each combination into `out`, showing `session` from the
/// sessions folder in `data_dir`.
pub fn run(out: &Path, data_dir: &Path, session: Option<&SessionId>) {
    std::fs::create_dir_all(out).expect("create the screenshot folder");
    for (mode, appearance) in [
        ("dark", WindowAppearance::Dark),
        ("light", WindowAppearance::Light),
    ] {
        for width in WIDTHS {
            for (view, state) in View::ALL.into_iter().flat_map(|v| STATES.map(|s| (v, s))) {
                // Overlays and selections look the same in every view; shoot them in the tree.
                if !state.is_empty() && view != View::Tree {
                    continue;
                }
                let mut cx = HeadlessAppContext::with_platform(
                    gpui_kit::platform::current_platform(true).text_system(),
                    Arc::new(Assets),
                    gpui_kit::platform::current_headless_renderer,
                );
                cx.update(|cx| {
                    gpui_kit::init(cx);
                    bind_keys(cx);
                    theme::apply_appearance(appearance, cx);
                });
                let options = Options {
                    data_dir: data_dir.to_owned(),
                    session: session.cloned(),
                    view,
                    // Paused, so the settings shot has its dropdown and no model is asked.
                    summaries: crate::summary_host::SummaryHost::start(
                        data_dir.join(ichnos_core::SESSIONS_DIR),
                        data_dir.to_owned(),
                        None,
                    )
                    .ok(),
                    summarizer: crate::summarizers::Choice::ClaudeCode,
                    summarizers: crate::summarizers::choices(None),
                    probe: |_| crate::summarizers::Availability::Ready,
                    search: None,
                    listener: None,
                };
                let (mut handle, viewer) = cx
                    .update(|cx| {
                        gpui_kit::open_window(
                            WindowOptions {
                                window_bounds: Some(WindowBounds::Windowed(Bounds {
                                    origin: gpui_kit::Point::default(),
                                    size: size(px(width), px(700.)),
                                })),
                                focus: false,
                                show: false,
                                ..WindowOptions::default()
                            },
                            cx,
                            |window, cx| cx.new(|cx| Viewer::new(options, window, cx)),
                        )
                    })
                    .expect("open the window");
                let settle = |cx: &mut HeadlessAppContext| {
                    for _ in 0..4 {
                        cx.advance_clock(Duration::from_millis(300));
                        cx.run_until_parked();
                    }
                };
                settle(&mut cx);
                if state == "settings" {
                    // The settings window is its own window: shoot that one.
                    handle = cx
                        .update(|cx| crate::settings::open(&viewer, cx))
                        .expect("open the settings window");
                    settle(&mut cx);
                } else if !state.is_empty() {
                    cx.update_window(handle, |_, window, cx| {
                        viewer.update(cx, |viewer, cx| viewer.apply_shot(state, window, cx));
                    })
                    .expect("set the state");
                    settle(&mut cx);
                }
                cx.update(|cx| theme::apply_appearance(appearance, cx));
                cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                    .expect("render a frame");
                let path = out.join(format!(
                    "{mode}-{width}-{}{}.png",
                    view.label(),
                    if state.is_empty() {
                        String::new()
                    } else {
                        format!("-{state}")
                    }
                ));
                cx.capture_screenshot(handle)
                    .expect("capture the frame")
                    .save(&path)
                    .expect("write the screenshot");
                println!("{}", path.display());
            }
        }
    }
}
