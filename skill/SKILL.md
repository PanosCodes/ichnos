---
name: ichnos
description: Query Ichnos's logs of coding-agent sessions (Claude Code and others) with SQL. Use when asked what happened in an agent session (this one or an earlier one), to find which past session worked on something, why a tool call failed, which files a session changed, what a subagent did, how long things took, to compare sessions, or when given an event as `<session-id>#<event-id>`.
---

# Ichnos logs

Ichnos records every agent session it is installed for: prompts, tool calls with their
input, output, errors and durations, subagents, the model's text between calls, and the reply
that ended each turn. Each session
is one SQLite file. Query it with `sqlite3` and answer from the rows; cite event ids so the user
can find them in the viewer.

## Given an event: `/ichnos <session-id>#<event-id> …`

The viewer's "Copy for agent" gives an event as `/ichnos <session-id>#<event-id>`, often
followed by a question. Read that event before anything else:

1. Its database is `sessions/<session-id>.db`; `<event-id>` is its `events.id`.
2. If the event has a `tool_use_id`, it is a call: run the "One call in full" recipe below with
   that id. Otherwise read the one event:
   `SELECT id, event, substr(record, 1, 4000) FROM events WHERE id = <event-id>`.
3. Find its turn, the last `UserPromptSubmit` at or before it, so you know what was being
   asked: `SELECT id, substr(record->>'$.prompt', 1, 500) FROM events
   WHERE event = 'UserPromptSubmit' AND id <= <event-id> ORDER BY id DESC LIMIT 1`.
4. Answer the question that follows the reference. With none, say what the event is, what
   happened, and anything that went wrong.

## Where the logs are

```
~/.ichnos/sessions/<session-id>.db     # or $ICHNOS_DIR/sessions/ when that is set
```

- **This session:** `~/.ichnos/sessions/$CLAUDE_CODE_SESSION_ID.db`
- **Newest sessions:** `ls -t ~/.ichnos/sessions/*.db | head`
- **Sessions of a project:** every database records its working directory. Newest first, with
  each session's size:

```sh
for db in $(ls -t ~/.ichnos/sessions/*.db); do
  echo "$db $(sqlite3 "$db" "SELECT (SELECT record->>'\$.extra.cwd' FROM events WHERE record->>'\$.extra.cwd' IS NOT NULL LIMIT 1) || '  ' || count(*) || ' events' FROM events")"
done
```

## Finding sessions about something: the index

When you don't know which session it was ("when did I fix the login bug?"), search the index
instead of opening every database. `~/.ichnos/index.db` (or `$ICHNOS_DIR/index.db`) holds, for
every session, each prompt, each failed call's error and each one-line summary:

```sql
docs(
  id         INTEGER PRIMARY KEY,
  session_id TEXT,     -- the session; its database is sessions/<session_id>.db
  event_id   INTEGER,  -- the events.id in that session's database the text belongs to
  kind       TEXT,     -- prompt | error | summary
  text       TEXT      -- one line, at most 500 characters
)
```

```sh
sqlite3 -json ~/.ichnos/index.db "
  SELECT session_id, count(*) AS hits, max(substr(text, 1, 150)) AS sample
  FROM docs WHERE text LIKE '%login%' AND text LIKE '%token%'
  GROUP BY session_id ORDER BY hits DESC LIMIT 10"
```

Then open the session's database and drill in from `event_id`. Try a few wordings, since
summaries paraphrase: search for `oauth` as well as `login`.

- The viewer builds the index while it runs. A session ends up in it a few seconds after it
  changes, and only while the viewer is open. Summaries only exist for sessions someone viewed.
  When the index is missing or a session isn't in it, fall back to the session databases.
- Query `docs` only. The `vectors` table needs the `sqlite-vec` extension, so `sqlite3` fails
  on it with "no such module: vec0".
- The same rules as below apply: `SELECT` only, and always `LIMIT`.

## How to query

```sh
sqlite3 -json ~/.ichnos/sessions/<id>.db "SELECT …"
```

- Run `SELECT`s only. The agent may be writing to the database while you read it.
- Don't pass `-readonly`: a database that was closed cleanly then fails with
  "unable to open database file".
- A session can hold megabytes of JSON. Always `LIMIT` rows and `substr` long fields. Start
  with the outline, then drill into the rows you need.
- Nested JSON keys need the `$` path form: `record->>'$.extra.cwd'`. A bare `'extra.cwd'` looks
  for a key literally named `extra.cwd` and returns NULL.

## Schema

```sql
events(
  id          INTEGER PRIMARY KEY, -- grows with every insert; "event id"
  epoch_ms    INTEGER,             -- when it happened
  event       TEXT,                -- event name, see below
  tool_use_id TEXT,                -- pairs a call's PreToolUse with its PostToolUse(Failure)
  record      TEXT                 -- the whole event as JSON
)
summaries(event_id, summary)  -- one plain-English sentence per finished call,
                              -- keyed by its PreToolUse id; may be missing
```

The file is the session (`sessions/<session-id>.db`); `record->>'session_id'` repeats its id.

`event` is one of `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`,
`PostToolUseFailure`, `SubagentStart`, `SubagentStop`, `Stop`, `Notification`, `AssistantMessage`,
`Debug`.

Fields of `record`:

| Field | Meaning |
| --- | --- |
| `agent_id` | `main`, or the subagent's id |
| `agent_type` | `main`, or the subagent type (set on `SubagentStart`; often null elsewhere) |
| `tool_name`, `input` | the tool and its input (`PreToolUse`) |
| `status` | `started`, `ok` or `error` |
| `duration_ms`, `response`, `error` | on `PostToolUse` / `PostToolUseFailure` |
| `prompt` | the user's prompt (`UserPromptSubmit`) |
| `extra.last_assistant_message` | the reply that ended the turn (`Stop`, `SubagentStop`) |
| `extra.cwd`, `extra.source` | on lifecycle events |
| `label` | the one line a row shows for a tool call or `Notification` (its command, path, …) |
| `files` | `[{"path", "op"}]` on `PreToolUse`: files the call read (`op` `read`) or wrote (`write`) |
| `skill`, `todos` | on `PreToolUse`: the skill the call loaded, the todo list it set |
| `spawned_agent` | on `PostToolUse`: the id of the subagent the call started (its `agent_id`) |
| `message_id`, `texts`, `thinking`, `tool_use_ids`, `usage`, `model` | on `AssistantMessage` |
| `extra.debug`, `extra.turn_id` | on `Debug`: the model's own account of the turn it follows (`skills`, `rules`, `outcome`, …; see `docs/ichnos-debug.schema.json`), or `{"invalid": true, "raw": …, "error": …}` |

A field longer than 8000 characters is stored as `{"truncated": true, "original_length": n,
"head": "<first 8000 chars>"}`. Read such fields with
`coalesce(record->>'$.prompt.head', record->>'$.prompt')`. For a clipped `input`, `head` is the
start of the input's JSON text, so fields like `$.input.file_path` are NULL; `files` and `label`
are never clipped this way, so prefer them.

A tool call is two events with the same `tool_use_id`. Its `PreToolUse` id is the call's id:
summaries are keyed by it and the viewer shows it.

## Recipes

**Outline: one row per prompt** (start here)

```sql
WITH p AS (
  SELECT id, epoch_ms, coalesce(record->>'$.prompt.head', record->>'$.prompt') AS prompt,
         lead(id, 1, 9e18) OVER (ORDER BY id) AS next_id
  FROM events WHERE event = 'UserPromptSubmit'
)
SELECT p.id AS turn_id,
       datetime(p.epoch_ms / 1000, 'unixepoch', 'localtime') AS at,
       substr(p.prompt, 1, 200) AS prompt,
       (SELECT count(*) FROM events e WHERE e.id > p.id AND e.id < p.next_id AND e.event = 'PreToolUse') AS calls,
       (SELECT count(*) FROM events e WHERE e.id > p.id AND e.id < p.next_id AND e.event = 'PostToolUseFailure') AS errors,
       (SELECT substr(e.record->>'$.extra.last_assistant_message', 1, 300) FROM events e
         WHERE e.id > p.id AND e.id < p.next_id AND e.event = 'Stop' ORDER BY e.id DESC LIMIT 1) AS reply
FROM p ORDER BY p.id LIMIT 50;
```

A turn's events are those with `turn_id < id < next turn_id`.

**Failed calls, with what was attempted**

```sql
SELECT f.id, f.record->>'$.agent_id' AS agent, f.record->>'$.tool_name' AS tool,
       substr(pre.record->>'$.input', 1, 300) AS input,
       substr(f.record->>'$.error', 1, 300) AS error
FROM events f
LEFT JOIN events pre ON pre.tool_use_id = f.tool_use_id AND pre.event = 'PreToolUse'
WHERE f.event = 'PostToolUseFailure'
ORDER BY f.id LIMIT 30;
```

**Files changed**

```sql
SELECT f.value->>'path' AS file, e.record->>'$.tool_name' AS tool, group_concat(e.id) AS call_ids
FROM events e, json_each(e.record, '$.files') f
WHERE e.event = 'PreToolUse' AND f.value->>'op' = 'write'
GROUP BY file, tool ORDER BY min(e.id);
```

Sessions recorded before Ichnos wrote `files` have none; `ichnos import claude-code
--tool-fields` adds them.

**Time and errors per tool**

```sql
SELECT record->>'$.tool_name' AS tool, count(*) AS calls,
       sum(event = 'PostToolUseFailure') AS errors, sum(record->>'$.duration_ms') AS total_ms
FROM events WHERE event IN ('PostToolUse', 'PostToolUseFailure')
GROUP BY tool ORDER BY calls DESC;
```

**Slowest calls**

```sql
SELECT pre.id AS call_id, post.record->>'$.tool_name' AS tool, post.record->>'$.duration_ms' AS ms,
       substr(pre.record->>'$.input', 1, 150) AS input
FROM events post
JOIN events pre ON pre.tool_use_id = post.tool_use_id AND pre.event = 'PreToolUse'
WHERE post.event IN ('PostToolUse', 'PostToolUseFailure') AND post.record->>'$.duration_ms' IS NOT NULL
ORDER BY post.record->>'$.duration_ms' DESC LIMIT 10;
```

`duration_ms` is a JSON number, so `->>` returns an integer and the sort is numeric.

**Subagents**

```sql
SELECT s.record->>'$.agent_id' AS agent, s.record->>'$.agent_type' AS type,
       datetime(s.epoch_ms / 1000, 'unixepoch', 'localtime') AS started,
       (SELECT count(*) FROM events e WHERE e.record->>'$.agent_id' = s.record->>'$.agent_id' AND e.event = 'PreToolUse') AS calls,
       (SELECT count(*) FROM events e WHERE e.record->>'$.agent_id' = s.record->>'$.agent_id' AND e.event = 'PostToolUseFailure') AS errors
FROM events s WHERE s.event = 'SubagentStart'
ORDER BY s.id LIMIT 30;
```

**Search for text** (in inputs, outputs, errors and prompts)

```sql
SELECT id, event, record->>'$.tool_name' AS tool, substr(record->>'$.input', 1, 200) AS input
FROM events WHERE record LIKE '%hook.rs%' AND event = 'PreToolUse'
ORDER BY id LIMIT 20;
```

**One call in full** (replace `:id` with any of its event ids)

```sql
SELECT e.id, e.event, e.record->>'$.tool_name' AS tool, e.record->>'$.status' AS status,
       e.record->>'$.duration_ms' AS ms,
       substr(e.record->>'$.input', 1, 2000) AS input,
       substr(e.record->>'$.response', 1, 2000) AS response,
       substr(e.record->>'$.error', 1, 2000) AS error,
       s.summary
FROM events e LEFT JOIN summaries s ON s.event_id = e.id
WHERE e.tool_use_id = (SELECT tool_use_id FROM events WHERE id = :id)
ORDER BY e.id;
```

## The model's text

`AssistantMessage` events hold what the model wrote between tool calls. One message can arrive
as several events (one per block) with the same `message_id`; read them in `id` order:

```sql
SELECT id, record->>'$.agent_id' AS agent, record->>'$.message_id' AS message,
       substr(record->>'$.texts', 1, 500) AS texts, record->>'$.tool_use_ids' AS asked_for
FROM events WHERE event = 'AssistantMessage'
ORDER BY id DESC LIMIT 20;
```

Sessions recorded before Ichnos logged model text have none; `ichnos import claude-code <id>`
adds it for Claude Code sessions whose transcript still exists.
