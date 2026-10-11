//! The viewer in a headless window: a recorded session loads, and every view and overlay renders
//! and answers the keyboard.

use std::path::Path;
use std::time::Duration;

use gpui_kit::{Entity, TestAppContext, VisualTestContext};
use ichnos_core::store::{NewEvent, SessionId, Store};
use serde_json::{Value, json};

use super::{Options, Viewer, bind_keys};
use crate::prefs::Prefs;
use crate::summarizers::{Availability, Choice};
use crate::summary_host::SummaryHost;
use crate::view_state::View;

/// Claude Code runs here and Apple Foundation Models doesn't, without starting either.
fn fake_probe(choice: &Choice) -> Availability {
    match choice {
        Choice::AppleFoundationModels => Availability::Missing("no fm here"),
        _ => Availability::Ready,
    }
}

/// Store `records` as session `id` in the sessions folder of `data_dir`.
fn record(data_dir: &Path, id: &str, records: &[Value]) {
    let sessions = data_dir.join(ichnos_core::SESSIONS_DIR);
    std::fs::create_dir_all(&sessions).unwrap();
    let session: SessionId = id.parse().unwrap();
    let store = Store::open(&session.db_path(&sessions)).unwrap();
    for record in records {
        let json = record.to_string();
        store
            .add_event(&NewEvent {
                epoch_ms: record["epoch_ms"].as_i64().unwrap(),
                event: record["event"].as_str().unwrap(),
                tool_use_id: record["tool_use_id"].as_str(),
                json: &json,
            })
            .unwrap();
    }
}

fn session() -> Vec<Value> {
    vec![
        json!({"event": "UserPromptSubmit", "epoch_ms": 1000, "agent_id": "main", "prompt": "fix the bug",
               "extra": {"cwd": "/work/ichnos"}}),
        json!({"event": "PreToolUse", "epoch_ms": 1100, "tool_use_id": "t1", "tool_name": "Bash",
               "input": {"command": "cargo test"}, "agent_id": "main", "status": "started"}),
        json!({"event": "PostToolUseFailure", "epoch_ms": 1500, "tool_use_id": "t1", "status": "error",
               "error": "boom"}),
        json!({"event": "PreToolUse", "epoch_ms": 1600, "tool_use_id": "agent-call", "tool_name": "Agent",
               "input": {"description": "look around", "subagent_type": "Explore"}, "agent_id": "main",
               "status": "started"}),
        json!({"event": "SubagentStart", "epoch_ms": 1700, "agent_id": "a1", "agent_type": "Explore"}),
        json!({"event": "PreToolUse", "epoch_ms": 1800, "tool_use_id": "t2", "tool_name": "Read",
               "input": {"file_path": "/src/main.rs"}, "agent_id": "a1", "status": "started"}),
        json!({"event": "PostToolUse", "epoch_ms": 1900, "tool_use_id": "t2", "status": "ok"}),
        json!({"event": "SubagentStop", "epoch_ms": 2000, "agent_id": "a1", "agent_type": "Explore",
               "extra": {"last_assistant_message": "found it"}}),
        json!({"event": "PostToolUse", "epoch_ms": 2100, "tool_use_id": "agent-call", "status": "ok",
               "response": {"agentId": "a1"}}),
        json!({"event": "PreToolUse", "epoch_ms": 2200, "tool_use_id": "skill-use",
               "tool_name": "Skill", "input": {"skill": "rust-best-practices"},
               "agent_id": "main", "status": "started"}),
        json!({"event": "PostToolUse", "epoch_ms": 2300, "tool_use_id": "skill-use",
               "status": "ok"}),
        json!({"event": "Stop", "epoch_ms": 3000, "agent_id": "main",
               "extra": {"last_assistant_message": "fixed"}}),
    ]
}

/// A viewer on session `s1`, with its data loaded.
fn open(cx: &mut TestAppContext, data_dir: &Path) -> (Entity<Viewer>, VisualTestContext) {
    open_with(cx, data_dir, None)
}

/// A viewer on session `s1` that summarizes with `summaries`.
fn open_with(
    cx: &mut TestAppContext,
    data_dir: &Path,
    summaries: Option<SummaryHost>,
) -> (Entity<Viewer>, VisualTestContext) {
    record(data_dir, "s1", &session());
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let options = Options {
        data_dir: data_dir.to_owned(),
        session: Some("s1".parse().unwrap()),
        view: View::Tree,
        summaries,
        summarizer: crate::summarizers::Choice::ClaudeCode,
        summarizers: crate::summarizers::choices(None),
        probe: fake_probe,
        search: None,
        listener: None,
    };
    let (viewer, visual) = cx.add_window_view(|window, cx| Viewer::new(options, window, cx));
    settle(visual);
    (viewer, visual.clone())
}

/// Let the polls run and draw a frame.
fn settle(visual: &mut VisualTestContext) {
    for _ in 0..4 {
        visual.executor().advance_clock(Duration::from_millis(300));
        visual.run_until_parked();
    }
    visual.update(|window, cx| window.draw(cx).clear(cx));
}

fn press(visual: &mut VisualTestContext, keys: &str) {
    visual.simulate_keystrokes(keys);
    settle(visual);
}

#[gpui_kit::test]
fn a_recorded_session_loads_into_the_tree(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    visual.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert_eq!(viewer.data().events().len(), 12);
        assert_eq!(viewer.model.stats().calls, 4);
        // The only turn is the last one, so it starts open.
        assert!(viewer.rows.len() > 1);
    });
}

#[gpui_kit::test]
fn skill_use_rows_show_the_skill_tag_in_tree_and_table(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (_viewer, mut visual) = open(cx, dir.path());
    let tag_is_visible = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            window.render_frame(cx);
            f32::from(window.find("skill-tag").bounds().size.width) > 0.
        })
    };
    assert!(
        tag_is_visible(&mut visual),
        "Tree view should show the skill tag"
    );

    press(&mut visual, "b");
    assert!(
        tag_is_visible(&mut visual),
        "Table view should show the skill tag"
    );
}

#[gpui_kit::test]
fn the_keyboard_moves_jumps_to_errors_and_switches_views(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());

    press(&mut visual, "j j");
    visual.update(|_, cx| {
        let selected = viewer.read(cx).state.selected().cloned();
        assert_eq!(selected.map(|k| k.to_string()), Some("t1".to_owned()));
    });

    press(&mut visual, "e");
    visual.update(|_, cx| {
        let selected = viewer.read(cx).state.selected().cloned();
        assert_eq!(selected.map(|k| k.to_string()), Some("t1".to_owned()));
    });

    for (key, view) in [("g", View::Graph), ("b", View::Table), ("t", View::Tree)] {
        press(&mut visual, key);
        visual.update(|_, cx| assert_eq!(viewer.read(cx).state.view(), view));
    }
}

#[gpui_kit::test]
fn the_switcher_and_ask_panel_open_and_close(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());

    press(&mut visual, "cmd-k");
    visual.update(|_, cx| assert!(viewer.read(cx).switcher.is_some()));
    press(&mut visual, "escape");
    visual.update(|_, cx| assert!(viewer.read(cx).switcher.is_none()));

    press(&mut visual, "j");
    visual.update(|window, cx| viewer.update(cx, |viewer, cx| viewer.toggle_ask(true, window, cx)));
    settle(&mut visual);
    visual.update(|_, cx| assert!(viewer.read(cx).ask.is_some()));
    press(&mut visual, "escape");
    visual.update(|_, cx| assert!(viewer.read(cx).ask.is_none()));
}

#[gpui_kit::test]
fn replay_rewinds_and_plays_back_to_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());

    press(&mut visual, "space");
    visual.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert!(viewer.state.playing());
        assert!(viewer.state.replay_at().is_some());
    });
    // Ten moments at one step per 250 ms run off the end into live.
    for _ in 0..12 {
        settle(&mut visual);
    }
    visual.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert!(!viewer.state.playing());
        assert_eq!(viewer.state.replay_at(), None);
    });
}

#[gpui_kit::test]
fn every_tree_row_spans_the_list_so_columns_line_up(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    let ids: Vec<(&'static str, usize)> = visual.update(|_, cx| {
        let viewer = viewer.read(cx);
        viewer
            .rows
            .iter()
            .map(|row| {
                let kind = if viewer.model.node(row.id).is_turn() {
                    "turn"
                } else {
                    "row"
                };
                (kind, row.id.index())
            })
            .collect()
    });
    assert!(ids.len() > 2);
    visual.update(|window, cx| {
        window.render_frame(cx);
        let widths: Vec<_> = ids
            .iter()
            .map(|&id| window.find(id).bounds().size.width)
            .collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "row widths differ: {widths:?}"
        );
    });
}

#[gpui_kit::test]
fn dragging_the_graph_moves_it(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{point, px};

    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "g");
    visual.update(|window, cx| {
        window.drag(point(px(200.), px(500.)), point(px(260.), px(480.)), cx);
    });
    settle(&mut visual);
    visual.update(|_, cx| {
        let pan = viewer.read(cx).state.graph_pan();
        assert_eq!(pan.offset(), (60., -20.));
        assert!(!pan.is_dragging());
    });
}

#[gpui_kit::test]
fn zooming_scales_the_graph_cards(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "g");
    let width = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            window.render_frame(cx);
            f32::from(window.find(("card", 0_usize)).bounds().size.width)
        })
    };
    let actual = width(&mut visual);
    assert!(
        (actual - 340.).abs() < 0.5,
        "main card is {actual}px at 100%"
    );

    press(&mut visual, "cmd-=");
    visual.update(|_, cx| assert!((viewer.read(cx).state.graph_pan().zoom() - 1.2).abs() < 1e-6));
    let zoomed = width(&mut visual);
    assert!(
        (zoomed - 340. * 1.2).abs() < 0.5,
        "main card is {zoomed}px at 120%"
    );

    press(&mut visual, "cmd-0");
    assert!((width(&mut visual) - 340.).abs() < 0.5);
}

#[gpui_kit::test]
fn dragging_the_handle_resizes_the_panel(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{point, px};

    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "g");
    let from = visual.update(|window, cx| {
        window.render_frame(cx);
        let bounds = window.find("panel-handle").bounds();
        point(bounds.center().x, px(500.))
    });
    visual.update(|window, cx| {
        window.drag(from, point(from.x - px(60.), px(500.)), cx);
    });
    settle(&mut visual);
    visual.update(|_, cx| {
        let state = &viewer.read(cx).state;
        assert_eq!(state.panel().width(), 440.);
        assert!(!state.panel().is_dragging());
        assert_eq!(state.graph_pan().offset(), (0., 0.));
    });
}

#[gpui_kit::test]
fn the_help_overlay_and_the_panel_toggle_from_the_keyboard(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());

    press(&mut visual, "?");
    visual.update(|_, cx| assert!(viewer.read(cx).help));
    press(&mut visual, "escape");
    visual.update(|_, cx| assert!(!viewer.read(cx).help));

    press(&mut visual, "cmd-\\");
    visual.update(|_, cx| assert!(!viewer.read(cx).state.panel().is_open()));
    press(&mut visual, "cmd-\\");
    visual.update(|_, cx| assert!(viewer.read(cx).state.panel().is_open()));
}

#[gpui_kit::test]
fn the_arrow_keys_step_the_replay_cursor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());

    press(&mut visual, "left");
    let first = visual.update(|_, cx| viewer.read(cx).state.replay_at());
    assert!(first.is_some());
    press(&mut visual, "left");
    let second = visual.update(|_, cx| viewer.read(cx).state.replay_at());
    assert!(second < first);
    press(&mut visual, "right");
    visual.update(|_, cx| assert_eq!(viewer.read(cx).state.replay_at(), first));
}

#[gpui_kit::test]
fn the_settings_shortcut_opens_one_window(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "cmd-,");
    assert!(cx.read(|cx| viewer.read(cx).settings.is_some()));
    let windows = cx.update(|cx| cx.windows().len());
    press(&mut visual, "cmd-,");
    assert_eq!(cx.update(|cx| cx.windows().len()), windows);
}

#[gpui_kit::test]
fn the_settings_window_opens_again_after_it_is_closed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    let windows = cx.update(|cx| cx.windows().len());
    press(&mut visual, "cmd-,");
    assert_eq!(cx.update(|cx| cx.windows().len()), windows + 1);
    let settings = cx.read(|cx| viewer.read(cx).settings).unwrap();
    cx.update(|cx| {
        settings
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    assert_eq!(cx.update(|cx| cx.windows().len()), windows);
    press(&mut visual, "cmd-,");
    assert_eq!(cx.update(|cx| cx.windows().len()), windows + 1);
}

#[gpui_kit::test]
fn the_settings_switch_changes_what_the_viewer_follows(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, _visual) = open(cx, dir.path());
    let (_settings, settings) =
        cx.add_window_view(|window, cx| crate::settings::Settings::new(viewer.clone(), window, cx));
    let follows = |settings: &mut VisualTestContext| {
        settings.update(|_, cx| viewer.read(cx).follows_newest())
    };
    assert!(!follows(settings));
    for expected in [true, false] {
        settings.update(|window, cx| {
            window.render_frame(cx);
            window.click("follow", cx);
        });
        settle(settings);
        assert_eq!(follows(settings), expected);
    }
}

#[gpui_kit::test]
fn the_settings_tabs_switch_panes_by_click_and_keyboard(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, _visual) = open(cx, dir.path());
    let (_settings, settings) =
        cx.add_window_view(|window, cx| crate::settings::Settings::new(viewer.clone(), window, cx));
    settings.update(|window, cx| {
        window.render_frame(cx);
        window.click("About", cx);
    });
    settle(settings);
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("follow").is_none(),
            "About shows no follow checkbox"
        );
    });
    press(settings, "cmd-1");
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(f32::from(window.find("follow").bounds().size.width) > 0.);
    });
}

#[gpui_kit::test]
fn the_settings_window_switches_following(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, _visual) = open(cx, dir.path());
    // The viewer was opened on a session, so it is pinned.
    assert!(!cx.read(|cx| viewer.read(cx).follows_newest()));
    let (_settings, settings) =
        cx.add_window_view(|window, cx| crate::settings::Settings::new(viewer.clone(), window, cx));
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(f32::from(window.find("follow").bounds().size.width) > 0.);
    });
    viewer.update(cx, |viewer, cx| viewer.set_follow_newest(true, cx));
    assert!(cx.read(|cx| viewer.read(cx).follows_newest()));
}

#[gpui_kit::test]
fn the_summarizer_dropdown_switches_and_remembers_the_summarizer(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join(ichnos_core::SESSIONS_DIR);
    // A paused summarizer: the test never asks a real model.
    let host = SummaryHost::start(sessions, dir.path().to_owned(), None).unwrap();
    let (viewer, _visual) = open_with(cx, dir.path(), Some(host));
    let (_settings, settings) =
        cx.add_window_view(|window, cx| crate::settings::Settings::new(viewer.clone(), window, cx));
    settle(settings);
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(f32::from(window.find("summarizer").bounds().size.width) > 0.);
        assert!(
            window.try_find("summarizer-missing").is_none(),
            "Claude Code runs here, so no reason is shown"
        );
    });

    // The native menu's items dispatch this action; index 1 is Apple Foundation Models.
    settings.dispatch_action(crate::settings::PickSummarizer { index: 1 });
    settle(settings);
    assert_eq!(
        settings.update(|_, cx| viewer.read(cx).summarizer().cloned()),
        Some(Choice::AppleFoundationModels)
    );
    assert_eq!(
        Prefs::load(dir.path()).summarizer.as_deref(),
        Some("apple-foundation-models")
    );
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(
            f32::from(window.find("summarizer-missing").bounds().size.width) > 0.,
            "the probe said Apple Foundation Models can't run, so the reason is shown"
        );
    });
}

#[gpui_kit::test]
fn without_summaries_the_settings_show_no_dropdown(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (viewer, _visual) = open(cx, dir.path());
    let (_settings, settings) =
        cx.add_window_view(|window, cx| crate::settings::Settings::new(viewer.clone(), window, cx));
    settings.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("summarizer").is_none());
    });
}

/// What the clipboard holds as text.
fn clipboard(visual: &mut VisualTestContext) -> Option<String> {
    visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

#[gpui_kit::test]
fn the_inspector_copies_the_selected_event_for_an_agent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (_viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "j j");
    visual.update(|window, cx| {
        window.render_frame(cx);
        window.click("copy-for-agent", cx);
    });
    // `t1`'s `PreToolUse` is the session's second event.
    assert_eq!(clipboard(&mut visual).as_deref(), Some("/ichnos s1#2"));
}

/// Right-click tree row `key` and pick item `item` of its menu.
fn pick_from_row_menu(
    viewer: &Entity<Viewer>,
    visual: &mut VisualTestContext,
    key: &str,
    item: usize,
) {
    use gpui_kit::test::TestWindowExt as _;

    let index = visual.update(|_, cx| {
        let viewer = viewer.read(cx);
        let row = viewer
            .rows
            .iter()
            .find(|row| viewer.model.node(row.id).key().as_str() == key);
        row.unwrap().id.index()
    });
    visual.update(|window, cx| {
        window.render_frame(cx);
        window.right_click(("row", index), cx);
    });
    settle(visual);
    visual.update(|window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(item, cx);
    });
    settle(visual);
}

#[gpui_kit::test]
fn a_row_s_right_click_menu_copies_its_events(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    pick_from_row_menu(&viewer, &mut visual, "t1", 0);
    let copied: Value = serde_json::from_str(&clipboard(&mut visual).unwrap()).unwrap();
    let events: Vec<&str> = copied
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["PreToolUse", "PostToolUseFailure"]);
}

#[gpui_kit::test]
fn a_row_s_right_click_menu_copies_its_event_for_an_agent(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (viewer, mut visual) = open(cx, dir.path());
    pick_from_row_menu(&viewer, &mut visual, "t1", 1);
    assert_eq!(clipboard(&mut visual).as_deref(), Some("/ichnos s1#2"));
}

#[gpui_kit::test]
fn a_table_row_s_right_click_menu_copies_its_event_for_an_agent(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;

    let dir = tempfile::tempdir().unwrap();
    let (_viewer, mut visual) = open(cx, dir.path());
    press(&mut visual, "b");
    visual.update(|window, cx| {
        window.render_frame(cx);
        // The table's first row is the prompt, the session's first event.
        window.right_click(("table-row", 0usize), cx);
    });
    settle(&mut visual);
    visual.update(|window, cx| {
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    });
    settle(&mut visual);
    assert_eq!(clipboard(&mut visual).as_deref(), Some("/ichnos s1#1"));
}
