use serde_json::json;

use super::*;
use crate::session_data::SessionData;

/// Builders shaped like the hook's records.
pub(crate) fn prompt(ms: i64, text: &str) -> Value {
    json!({"event": "UserPromptSubmit", "epoch_ms": ms, "agent_id": "main", "agent_type": "main", "prompt": text})
}
pub(crate) fn stop(ms: i64) -> Value {
    json!({"event": "Stop", "epoch_ms": ms, "agent_id": "main", "agent_type": "main",
           "extra": {"last_assistant_message": "done"}})
}
pub(crate) fn pre(ms: i64, id: &str, tool: &str, input: &Value, agent: &str) -> Value {
    let agent_type = if agent == "main" {
        json!("main")
    } else {
        Value::Null
    };
    json!({"event": "PreToolUse", "epoch_ms": ms, "tool_use_id": id, "tool_name": tool,
           "input": input, "agent_id": agent, "agent_type": agent_type, "status": "started"})
}
/// `record` with `fields` added, such as a producer's tool fields.
pub(crate) fn with(mut record: Value, fields: &Value) -> Value {
    if let (Some(record), Some(fields)) = (record.as_object_mut(), fields.as_object()) {
        record.extend(fields.clone());
    }
    record
}
pub(crate) fn bash(ms: i64, id: &str) -> Value {
    pre(ms, id, "Bash", &json!({"command": "ls"}), "main")
}
pub(crate) fn post(ms: i64, id: &str, extra: &Value) -> Value {
    let mut record = json!({"event": "PostToolUse", "epoch_ms": ms, "tool_use_id": id,
                            "status": "ok", "duration_ms": 5});
    if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
        record.extend(extra.clone());
    }
    record
}
pub(crate) fn step(key: &str, first: i64, last: i64, extra: &Value) -> Value {
    let mut record = json!({"key": key, "agent_id": "main", "first_ms": first, "last_ms": last,
                            "usage": {"output_tokens": 10, "input_tokens": 100}});
    if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
        record.extend(extra.clone());
    }
    record
}

pub(crate) fn data(events: &[Value], steps: &[Value]) -> SessionData {
    let mut data = SessionData::default();
    for (row, record) in (1..).zip(events) {
        data.add_event(Event::parse(row, &record.to_string()).unwrap());
    }
    for record in steps {
        data.add_step(serde_json::from_value(record.clone()).unwrap());
    }
    data
}

pub(crate) const LIVE: Moment = Moment {
    at: None,
    live: true,
    clock: 10_000,
};
pub(crate) const DONE: Moment = Moment {
    at: None,
    live: false,
    clock: 10_000,
};

fn node<'a>(model: &'a SessionModel, key: &str) -> &'a Node {
    model.node(model.find(&Key::from(key)).unwrap())
}

fn keys(model: &SessionModel, ids: &[NodeId]) -> Vec<String> {
    ids.iter()
        .map(|&id| model.node(id).key().to_string())
        .collect()
}

#[test]
fn each_prompt_starts_a_turn_stop_closes_it_and_calls_go_inside() {
    let events = [
        prompt(1, "do it"),
        bash(2, "t1"),
        post(3, "t1", &json!({})),
        stop(4),
        prompt(5, "do it"),
    ];
    let model = build_session(&data(&events, &[]), LIVE);
    let turns = model.turns();
    assert_eq!(turns.len(), 2);
    assert_eq!(keys(&model, model.node(turns[0]).children()), ["t1"]);
    assert_eq!(model.node(turns[0]).end(), Some(4));
    assert_eq!(model.node(turns[0]).status(), Some(Status::Ok));
    assert_eq!(
        model.node(turns[1]).end(),
        None,
        "the last turn is still going while live"
    );
}

#[test]
fn a_node_is_named_by_its_first_event_and_a_model_step_by_none() {
    let events = [prompt(1, "do it"), bash(2, "t1"), post(3, "t1", &json!({}))];
    let steps = [step("m1", 2, 2, &json!({}))];
    let model = build_session(&data(&events, &steps), LIVE);
    assert_eq!(model.node(model.turns()[0]).row_id(), Some(1));
    assert_eq!(node(&model, "t1").row_id(), Some(2));
    assert_eq!(node(&model, "m1").row_id(), None);
}

#[test]
fn a_call_without_post_is_unfinished_in_a_closed_turn_and_running_in_the_open_one() {
    let events = [
        prompt(1, "do it"),
        bash(2, "lost"),
        prompt(5, "do it"),
        bash(6, "now"),
    ];
    let model = build_session(&data(&events, &[]), LIVE);
    assert_eq!(node(&model, "lost").status(), Some(Status::Unfinished));
    assert_eq!(
        node(&model, "lost").end(),
        Some(5),
        "it ends when the next prompt arrived"
    );
    assert_eq!(node(&model, "now").status(), Some(Status::Started));
}

#[test]
fn when_not_live_the_last_turn_without_stop_ends_at_its_last_child() {
    let events = [prompt(1, "do it"), bash(2, "t1"), post(8, "t1", &json!({}))];
    let model = build_session(&data(&events, &[]), DONE);
    assert_eq!(model.node(model.turns()[0]).end(), Some(8));
}

#[test]
fn a_subagents_calls_hang_under_the_agent_call_that_started_it() {
    let events = [
        prompt(1, "do it"),
        pre(
            2,
            "agent-call",
            "Agent",
            &json!({"description": "look around", "subagent_type": "Explore"}),
            "main",
        ),
        json!({"event": "SubagentStart", "epoch_ms": 3, "agent_id": "a1", "agent_type": "Explore"}),
        pre(4, "sub-read", "Read", &json!({"file_path": "/x"}), "a1"),
        post(5, "sub-read", &json!({})),
        json!({"event": "SubagentStop", "epoch_ms": 6, "agent_id": "a1", "agent_type": "Explore"}),
        post(7, "agent-call", &json!({"spawned_agent": "a1"})),
        stop(8),
    ];
    let model = build_session(&data(&events, &[]), DONE);
    let call = node(&model, "agent-call");
    assert_eq!(call.agent().unwrap().agent_type(), Some("Explore"));
    assert_eq!(keys(&model, call.children()), ["sub-read"]);
    assert_eq!(node(&model, "sub-read").depth(), 2);
    assert_eq!(
        keys(&model, model.node(model.turns()[0]).children()),
        ["agent-call"]
    );
}

#[test]
fn an_agent_call_without_spawned_agent_does_not_adopt_a_subagent() {
    let events = [
        prompt(1, "do it"),
        pre(2, "agent-call", "Agent", &json!({}), "main"),
        json!({"event": "SubagentStart", "epoch_ms": 3, "agent_id": "a1", "agent_type": "Explore"}),
        pre(4, "sub-read", "Read", &json!({}), "a1"),
        post(5, "sub-read", &json!({})),
        json!({"event": "SubagentStop", "epoch_ms": 6, "agent_id": "a1", "agent_type": "Explore"}),
        post(7, "agent-call", &json!({"response": {"agentId": "a1"}})),
        stop(8),
    ];
    let model = build_session(&data(&events, &[]), DONE);
    assert!(
        node(&model, "agent-call").children().is_empty(),
        "core reads `spawned_agent` only, never the tool's response"
    );
    assert_eq!(
        keys(&model, model.node(model.turns()[0]).children()),
        ["agent-call", "agent:a1"]
    );
}

#[test]
fn an_untyped_subagent_with_no_calls_is_a_progress_note() {
    let events = [
        prompt(1, "do it"),
        json!({"event": "SubagentStop", "epoch_ms": 2, "agent_id": "n1", "agent_type": null,
               "extra": {"last_assistant_message": "note"}}),
    ];
    let model = build_session(&data(&events, &[]), DONE);
    assert!(matches!(node(&model, "agent:n1").kind(), NodeKind::Note(_)));
}

#[test]
fn a_model_step_starts_when_its_agent_was_last_waiting_on_a_tool_result() {
    let events = [
        prompt(1000, "do it"),
        bash(1100, "t1"),
        post(1500, "t1", &json!({})),
        stop(3000),
    ];
    let steps = [
        step(
            "model:main:m1",
            1050,
            1090,
            &json!({"tool_use_ids": ["t1"]}),
        ),
        step("model:main:m2", 2000, 2900, &json!({})),
    ];
    let model = build_session(&data(&events, &steps), DONE);
    assert_eq!(
        node(&model, "model:main:m1").start(),
        Some(1000),
        "first step waited from the prompt"
    );
    assert_eq!(
        node(&model, "model:main:m2").start(),
        Some(1500),
        "second step waited from the tool result"
    );
    let totals = model.node(model.turns()[0]).totals();
    assert_eq!(totals.model_ms, 90 + 1400);
    assert_eq!(totals.tool_ms, 400);
    assert_eq!(totals.output_tokens, 20);
}

#[test]
fn replaying_an_earlier_moment_shows_the_session_as_it_was_then() {
    let events = [
        prompt(1000, "do it"),
        bash(1100, "t1"),
        post(1500, "t1", &json!({})),
        stop(3000),
    ];
    let steps = [step("model:main:m1", 1200, 1800, &json!({}))];
    let moment = Moment {
        at: Some(1300),
        live: false,
        clock: 10_000,
    };
    let model = build_session(&data(&events, &steps), moment);
    assert_eq!(
        node(&model, "t1").status(),
        Some(Status::Started),
        "its Post had not happened yet"
    );
    assert_eq!(
        node(&model, "model:main:m1").status(),
        Some(Status::Started),
        "the step was still being written"
    );
    assert_eq!(model.node(model.turns()[0]).end(), None);
}

#[test]
fn open_end_follows_the_clock_while_live_and_recent_else_the_last_event() {
    let bounds = Some(Bounds { t0: 0, t1: 1000 });
    let at = |at, live, clock| open_end(bounds, Moment { at, live, clock });
    assert_eq!(at(Some(500), true, 2000), 500);
    assert_eq!(at(None, true, 2000), 2000);
    assert_eq!(at(None, true, 1000 + STALE_MS + 1), 1000);
    assert_eq!(at(None, false, 2000), 1000);
}

#[test]
fn stats_agent_graph_and_state_at_agree_with_the_tree() {
    let events = [
        prompt(1, "do it"),
        with(
            pre(2, "w", "Write", &json!({}), "main"),
            &json!({"files": [{"path": "/a.rs", "op": "write"}]}),
        ),
        post(3, "w", &json!({})),
        with(
            pre(4, "r", "Read", &json!({}), "main"),
            &json!({"files": [{"path": "/b.rs", "op": "read"}]}),
        ),
        post(5, "r", &json!({"status": "error"})),
        with(
            pre(6, "todo", "TodoWrite", &json!({}), "main"),
            &json!({"todos": [{"content": "x", "status": "pending"}]}),
        ),
        post(7, "todo", &json!({})),
        stop(8),
    ];
    let model = build_session(&data(&events, &[]), DONE);
    let stats = model.stats();
    assert_eq!((stats.calls, stats.errors, stats.turns), (3, 1, 1));

    let graph = model.agent_graph(|_| String::new());
    let tools: Vec<&str> = graph
        .main()
        .tools()
        .iter()
        .map(|(t, _)| t.as_str())
        .collect();
    assert_eq!(tools, ["Write", "Read", "TodoWrite"]);
    assert_eq!(graph.main().errors(), 1);

    let early = model.state_at(4);
    let paths_of = |files: &[FileTouch]| files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths_of(&early.changed), ["/a.rs"]);
    assert_eq!(paths_of(&early.read), ["/b.rs"]);
    assert_eq!(early.todos, None, "the todo list came later");
    assert_eq!(model.state_at(8).todos.map(|todos| todos.len()), Some(1));
}

#[test]
fn moments_and_raw_data_cover_every_event() {
    let events = [prompt(1, "do it"), bash(2, "t1"), post(3, "t1", &json!({}))];
    let steps = [step("model:main:m1", 2, 4, &json!({}))];
    let data = data(&events, &steps);
    assert_eq!(moments(data.events(), data.steps()), [1, 2, 3, 4]);
    let model = build_session(&data, DONE);
    assert_eq!(node(&model, "t1").raw_data().len(), 2);
}
