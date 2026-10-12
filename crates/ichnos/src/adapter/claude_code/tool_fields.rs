//! Ichnos's tool fields for Claude Code's tool calls: what a call did, restated from Claude
//! Code's own tool names and input shapes, so core never has to know them.
//!
//! - `label`: the one line a row shows (the command, path, pattern, description, …).
//! - `files`: `[{path, op}]`, the files a `Read` read or an edit tool wrote.
//! - `skill`: the skill a `Skill` call loaded.
//! - `todos`: the list a `TodoWrite` call set.
//! - `spawned_agent`: on the `PostToolUse` of an `Agent` (formerly `Task`) call, the subagent it
//!   started.

use serde_json::{Map, Value, json};

/// Tools that create or change the file at `file_path` / `notebook_path`.
const EDIT_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Tools that start a subagent.
const AGENT_TOOLS: [&str; 2] = ["Agent", "Task"];

/// The longest label kept, in characters; a row shows one line of it.
const LABEL_CHARS: usize = 500;

/// Input fields that tell the most about a call, best first; the first non-empty one is its label.
const LABEL_FIELDS: [&str; 7] = [
    "command",
    "file_path",
    "notebook_path",
    "pattern",
    "description",
    "prompt",
    "skill",
];

/// The tool fields for one Claude Code event named `event`, from the payload's `tool_name`,
/// `tool_input` and `tool_response`; empty when there is nothing to say.
pub(super) fn for_event(
    event: &str,
    tool: Option<&str>,
    input: &Value,
    response: &Value,
) -> Map<String, Value> {
    let mut fields = Map::new();
    let text = |name: &str| {
        input
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    match (event, tool) {
        ("PreToolUse", Some(tool)) => {
            let path = text("file_path").or_else(|| text("notebook_path"));
            let op = if EDIT_TOOLS.contains(&tool) {
                Some("write")
            } else if tool == "Read" {
                Some("read")
            } else {
                None
            };
            if let (Some(path), Some(op)) = (path, op) {
                fields.insert("files".to_owned(), json!([{ "path": path, "op": op }]));
            }
            if tool == "Skill"
                && let Some(skill) = text("skill")
            {
                fields.insert("skill".to_owned(), skill.into());
            }
            if tool == "TodoWrite"
                && let Some(todos) = input.get("todos").filter(|todos| todos.is_array())
            {
                fields.insert("todos".to_owned(), todos.clone());
            }
            if let Some(label) = LABEL_FIELDS.iter().find_map(|&name| text(name)) {
                fields.insert("label".to_owned(), cut(label).into());
            }
        }
        ("PostToolUse", Some(tool)) if AGENT_TOOLS.contains(&tool) => {
            if let Some(agent) = spawned_agent(response) {
                fields.insert("spawned_agent".to_owned(), agent.into());
            }
        }
        ("Notification", _) => {
            if let Some(message) = text("message") {
                fields.insert("label".to_owned(), cut(message).into());
            }
        }
        _ => {}
    }
    fields
}

/// `text` cut to [`LABEL_CHARS`] characters.
fn cut(text: &str) -> String {
    text.chars().take(LABEL_CHARS).collect()
}

/// The subagent an `Agent` call started, from its response: `{"agentId": "a53dcc5a…", …}`.
///
/// The id may sit in nested JSON text, so the response is searched as text.
fn spawned_agent(response: &Value) -> Option<String> {
    if response.is_null() {
        return None;
    }
    let text = response.to_string();
    let key = "agentId";
    let mut rest = text.as_str();
    while let Some(at) = rest.find(key) {
        rest = &rest[at + key.len()..];
        let after = rest.trim_start_matches(['\\', '"']).trim_start();
        let Some(after) = after.strip_prefix(':') else {
            continue;
        };
        let value = after.trim_start().trim_start_matches(['\\', '"']);
        let id: String = value
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(tool: &str, input: &Value) -> Value {
        Value::Object(for_event("PreToolUse", Some(tool), input, &Value::Null))
    }

    #[test]
    fn edit_tools_write_their_file_and_read_reads_it() {
        for tool in EDIT_TOOLS {
            assert_eq!(
                pre(tool, &json!({"file_path": "/a.rs"}))["files"],
                json!([{"path": "/a.rs", "op": "write"}]),
                "{tool}"
            );
        }
        assert_eq!(
            pre("NotebookEdit", &json!({"notebook_path": "/n.ipynb"}))["files"],
            json!([{"path": "/n.ipynb", "op": "write"}])
        );
        assert_eq!(
            pre("Read", &json!({"file_path": "/b.rs"}))["files"],
            json!([{"path": "/b.rs", "op": "read"}])
        );
        assert_eq!(
            pre("Grep", &json!({"file_path": "/c.rs"})).get("files"),
            None
        );
    }

    #[test]
    fn skill_and_todo_calls_name_what_they_set() {
        let skill = pre("Skill", &json!({"skill": "rust-best-practices"}));
        assert_eq!(skill["skill"], json!("rust-best-practices"));
        assert_eq!(pre("Skill", &json!({"skill": ""})).get("skill"), None);
        assert_eq!(
            pre("Bash", &json!({"skill": "not-a-skill"})).get("skill"),
            None
        );

        let todos = json!([{"content": "x", "status": "pending", "activeForm": "X-ing"}]);
        assert_eq!(pre("TodoWrite", &json!({"todos": todos}))["todos"], todos);
        assert_eq!(pre("TodoWrite", &json!({"todos": "no"})).get("todos"), None);
    }

    #[test]
    fn the_label_is_the_most_telling_input_field() {
        let label = |tool, input| pre(tool, &input)["label"].clone();
        assert_eq!(
            label("Bash", json!({"command": "ls -la", "description": "list"})),
            json!("ls -la")
        );
        assert_eq!(
            label(
                "Agent",
                json!({"description": "look around", "prompt": "…"})
            ),
            json!("look around")
        );
        assert_eq!(pre("Bash", &json!({"timeout": 5})).get("label"), None);
        let long = "x".repeat(LABEL_CHARS + 10);
        assert_eq!(
            label("Bash", json!({"command": long}))
                .as_str()
                .map(|l| l.chars().count()),
            Some(LABEL_CHARS)
        );
    }

    #[test]
    fn an_agent_calls_post_names_the_subagent_it_started() {
        let post = |tool, response: Value| {
            for_event("PostToolUse", Some(tool), &Value::Null, &response)
                .get("spawned_agent")
                .cloned()
        };
        assert_eq!(post("Agent", json!({"agentId": "a1"})), Some(json!("a1")));
        assert_eq!(
            post(
                "Task",
                json!([{"text": "{\"agentId\": \"a-2_b\", \"x\": 1}"}])
            ),
            Some(json!("a-2_b"))
        );
        assert_eq!(post("Bash", json!({"agentId": "a1"})), None);
        assert_eq!(post("Agent", json!({"other": 1})), None);
        assert_eq!(post("Agent", Value::Null), None);
    }

    #[test]
    fn a_notification_is_labelled_by_its_message() {
        let fields = for_event(
            "Notification",
            None,
            &json!({"message": "Claude needs your permission to use Bash", "notification_type": "permission_prompt"}),
            &Value::Null,
        );
        assert_eq!(
            fields["label"],
            json!("Claude needs your permission to use Bash")
        );
    }
}
