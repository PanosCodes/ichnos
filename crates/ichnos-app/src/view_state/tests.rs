use ichnos_core::event::Event;
use ichnos_model::model::{Moment, build_session};
use ichnos_model::session_data::SessionData;
use serde_json::{Value, json};

use super::pan::ZOOM_RANGE;
use super::panel::{MAIN_MIN, PANEL_DEFAULT, PANEL_RANGE};
use super::*;

#[test]
fn dragging_the_handle_left_widens_the_panel() {
    let mut panel = PanelWidth::default();
    panel.start(1000.);
    assert!(panel.drag_to(940., 1600.));
    assert_eq!(panel.width(), PANEL_DEFAULT + 60.);
    panel.end();
    assert!(!panel.drag_to(900., 1600.));
    assert!(!panel.is_dragging());
}

#[test]
fn the_panel_width_stays_within_bounds() {
    let mut panel = PanelWidth::default();
    panel.start(1000.);
    panel.drag_to(-5000., 1600.);
    assert_eq!(panel.width(), PANEL_RANGE.1);
    panel.drag_to(5000., 1600.);
    assert_eq!(panel.width(), PANEL_RANGE.0);
    panel.drag_to(-5000., 800.);
    assert_eq!(panel.width(), 800. - MAIN_MIN);
    panel.reset();
    assert_eq!(panel.width(), PANEL_DEFAULT);
}

#[test]
fn a_hidden_panel_gives_the_main_pane_the_whole_window() {
    let mut panel = PanelWidth::default();
    assert_eq!(
        panel.main_width(1000.),
        1000. - PANEL_DEFAULT - HANDLE_WIDTH
    );
    panel.toggle();
    assert!(!panel.is_open());
    assert_eq!(panel.main_width(1000.), 1000.);
    panel.toggle();
    assert_eq!(panel.width(), PANEL_DEFAULT);
}

#[test]
fn switching_sessions_keeps_the_panel() {
    let mut state = ViewState::default();
    state.panel_mut().toggle();
    state.reset_for_session();
    assert!(!state.panel().is_open());
}

#[test]
fn an_agent_reference_names_the_session_and_the_event() {
    let session: SessionId = "afde7f06-8de7-4806-9714-a72e071cedd9".parse().unwrap();
    assert_eq!(
        agent_reference(&session, 68),
        "/ichnos afde7f06-8de7-4806-9714-a72e071cedd9#68"
    );
}

#[test]
fn the_nearest_tick_within_reach_is_hovered() {
    let ticks = [0.1, 0.5, 0.52, 0.9];
    assert_eq!(nearest_tick(&ticks, 0.515, 0.02), Some(2));
    assert_eq!(nearest_tick(&ticks, 0.11, 0.02), Some(0));
    assert_eq!(nearest_tick(&ticks, 0.7, 0.02), None);
    assert_eq!(nearest_tick(&[], 0.5, 1.), None);
}

const DONE: Moment = Moment {
    at: None,
    live: false,
    clock: 10_000,
};

fn model(events: &[Value]) -> SessionModel {
    let mut data = SessionData::default();
    for (row, record) in (1..).zip(events) {
        data.add_event(Event::parse(row, &record.to_string()).unwrap());
    }
    build_session(&data, DONE)
}

fn two_turns() -> SessionModel {
    model(&[
        json!({"event": "UserPromptSubmit", "epoch_ms": 1, "prompt": "first"}),
        json!({"event": "PreToolUse", "epoch_ms": 2, "tool_use_id": "a", "tool_name": "Bash"}),
        json!({"event": "PostToolUseFailure", "epoch_ms": 3, "tool_use_id": "a", "status": "error"}),
        json!({"event": "Stop", "epoch_ms": 4}),
        json!({"event": "UserPromptSubmit", "epoch_ms": 5, "prompt": "second"}),
        json!({"event": "PreToolUse", "epoch_ms": 6, "tool_use_id": "b", "tool_name": "Read"}),
        json!({"event": "PostToolUse", "epoch_ms": 7, "tool_use_id": "b", "status": "ok"}),
        json!({"event": "PreToolUse", "epoch_ms": 8, "tool_use_id": "c", "tool_name": "Bash"}),
        json!({"event": "PostToolUseFailure", "epoch_ms": 9, "tool_use_id": "c", "status": "error"}),
        json!({"event": "Stop", "epoch_ms": 10}),
    ])
}

fn keys(model: &SessionModel, rows: &[TreeRow]) -> Vec<String> {
    rows.iter()
        .map(|row| model.node(row.id).key().to_string())
        .collect()
}

#[test]
fn only_the_last_turn_starts_open() {
    let model = two_turns();
    let state = ViewState::default();
    let rows = state.tree_rows(&model, &Summaries::new());
    assert_eq!(
        keys(&model, &rows),
        [
            "turn:UserPromptSubmit:1:main",
            "turn:UserPromptSubmit:5:main",
            "b",
            "c"
        ]
    );
}

#[test]
fn toggling_a_turn_opens_and_closes_it() {
    let model = two_turns();
    let mut state = ViewState::default();
    let first = model.turns()[0];
    state.toggle(&model, first);
    assert_eq!(state.tree_rows(&model, &Summaries::new()).len(), 5);
    state.toggle(&model, first);
    assert_eq!(state.tree_rows(&model, &Summaries::new()).len(), 4);
}

#[test]
fn filtering_opens_every_turn_and_keeps_matches() {
    let model = two_turns();
    let mut state = ViewState::default();
    state.set_query("is:error");
    let rows = state.tree_rows(&model, &Summaries::new());
    assert_eq!(
        keys(&model, &rows),
        [
            "turn:UserPromptSubmit:1:main",
            "a",
            "turn:UserPromptSubmit:5:main",
            "c"
        ]
    );
}

#[test]
fn j_and_k_move_through_rows_and_stop_at_the_ends() {
    let order: Vec<Key> = ["x", "y", "z"].map(Key::from).to_vec();
    let mut state = ViewState::default();
    state.move_selection(&order, 1);
    assert_eq!(state.selected(), Some(&Key::from("x")));
    state.move_selection(&order, 1);
    state.move_selection(&order, 1);
    state.move_selection(&order, 1);
    assert_eq!(state.selected(), Some(&Key::from("z")));
    state.move_selection(&order, -5);
    assert_eq!(state.selected(), Some(&Key::from("x")));
    assert!(!state.tail(), "moving by hand stops auto-scroll");
}

#[test]
fn next_error_cycles_through_failed_calls_and_opens_their_turn() {
    let model = two_turns();
    let mut state = ViewState::new(View::Graph);
    assert_eq!(state.next_error(&model), Some(Key::from("a")));
    assert_eq!(state.view(), View::Tree, "errors are shown in the tree");
    let rows = state.tree_rows(&model, &Summaries::new());
    assert!(keys(&model, &rows).contains(&"a".to_owned()));
    assert_eq!(state.next_error(&model), Some(Key::from("c")));
    assert_eq!(state.next_error(&model), Some(Key::from("a")));
    assert_eq!(state.take_pending_reveal(), Some(Key::from("a")));
}

#[test]
fn replay_steps_through_moments_and_runs_off_the_end_into_live() {
    let mut state = ViewState::default();
    let bounds = Some(Bounds { t0: 1, t1: 3 });
    state.toggle_play(bounds);
    assert_eq!((state.replay_at(), state.playing()), (Some(1), true));
    state.step(&[1, 2, 3], true);
    assert_eq!(state.replay_at(), Some(2));
    state.step(&[1, 2, 3], true);
    state.step(&[1, 2, 3], true);
    assert_eq!((state.replay_at(), state.playing()), (None, false));
    state.step(&[1, 2, 3], false);
    assert_eq!(state.replay_at(), Some(2));
}

#[test]
fn seeking_past_the_end_goes_live() {
    let mut state = ViewState::default();
    let bounds = Some(Bounds { t0: 10, t1: 20 });
    state.seek(bounds, 5);
    assert_eq!(state.replay_at(), Some(10));
    state.seek(bounds, 20);
    assert_eq!(state.replay_at(), None);
}

#[test]
fn the_table_lists_subagent_events_without_a_tree_node() {
    let model = model(&[
        json!({"event": "UserPromptSubmit", "epoch_ms": 1, "prompt": "go"}),
        json!({"event": "SubagentStart", "epoch_ms": 2, "agent_id": "a1", "agent_type": "Explore"}),
    ]);
    let rows = ViewState::default().table_rows(&model, &Summaries::new());
    assert_eq!(rows.len(), 2);
    assert!(rows[1].node.is_none());
}

#[test]
fn dragging_moves_the_graph_and_a_short_press_is_still_a_click() {
    let mut pan = Pan::default();
    pan.start((100., 100.));
    assert!(!pan.drag_to((101., 101.)), "a jitter is not a drag");
    pan.end();
    assert!(!pan.was_drag());

    pan.start((100., 100.));
    assert!(pan.drag_to((130., 90.)));
    pan.end();
    assert!(pan.was_drag());
    assert_eq!(pan.offset(), (30., -10.));

    pan.start((0., 0.));
    pan.drag_to((10., 10.));
    assert_eq!(
        pan.offset(),
        (40., 0.),
        "a second drag continues from the first"
    );
    pan.end();
    pan.scroll((-40., 5.));
    assert_eq!(pan.offset(), (0., 5.));
    pan.reset();
    assert_eq!(pan, Pan::default());
}

#[test]
fn zooming_scales_the_offset_and_stays_in_range() {
    let mut pan = Pan::default();
    pan.scroll((10., -20.));
    pan.zoom_by(2.);
    assert_eq!((pan.zoom(), pan.offset()), (2., (20., -40.)));
    pan.zoom_by(10.);
    assert_eq!(pan.zoom(), ZOOM_RANGE.1);
    pan.zoom_by(0.001);
    assert_eq!(pan.zoom(), ZOOM_RANGE.0);
    pan.reset_zoom();
    assert!((pan.zoom() - 1.).abs() < f32::EPSILON);
    assert!((pan.offset().0 - 10.).abs() < 1e-4 && (pan.offset().1 + 20.).abs() < 1e-4);
}

#[test]
fn related_matches_skip_listed_and_deleted_sessions() {
    let id = |text: &str| -> SessionId { text.parse().unwrap() };
    let found = |session: &str| SessionMatch {
        session: id(session),
        event_id: 1,
        kind: ichnos_search::DocKind::Summary,
        text: session.to_owned(),
        distance: 0.1,
    };
    let (a, b, c) = (id("a"), id("b"), id("c"));
    let related = [found("a"), found("gone"), found("c"), found("b")];
    let kept = related_matches(&related, &[&a], &[&a, &b, &c]);
    let sessions: Vec<&str> = kept.iter().map(|found| found.session.as_str()).collect();
    assert_eq!(sessions, ["c", "b"]);
}

#[test]
fn scrubbing_is_held_between_press_and_release() {
    let mut state = ViewState::default();
    assert!(!state.scrubbing());
    state.set_scrubbing(true);
    assert!(state.scrubbing());
    state.set_scrubbing(false);
    assert!(!state.scrubbing());
}
