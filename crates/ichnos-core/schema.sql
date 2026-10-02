-- Ichnos database schema. One file per session:
-- ~/.ichnos/sessions/<session-id>.db.
--
-- The binary compiles this file in and applies it to a database whose `PRAGMA user_version` is
-- older than the version set at the bottom (see `migrate` in src/store.rs). Keep that version and
-- SCHEMA_VERSION in src/store.rs equal; a test checks it.

-- One row per Ichnos event (see src/log.rs). `record` is the full event as JSON,
-- exactly what the viewer receives; the other columns are copies of its fields, kept for
-- indexed lookups.
-- The session id is the file's name (and `record`'s `session_id`), so no column repeats it.
CREATE TABLE IF NOT EXISTS events (
    id          INTEGER PRIMARY KEY,  -- increases with every insert; the viewer's resume cursor
    epoch_ms    INTEGER NOT NULL,     -- when the event happened (or was received)
    event       TEXT NOT NULL,        -- Ichnos event name, e.g. PreToolUse, Stop, AssistantMessage
    tool_use_id TEXT,                 -- pairs a tool call's Pre and Post events; NULL otherwise
    record      TEXT NOT NULL         -- the event as JSON
);
CREATE INDEX IF NOT EXISTS events_tool_use ON events (tool_use_id) WHERE tool_use_id IS NOT NULL;

-- One plain-language sentence per finished entry, written by the summarizer. A cache: entries without a
-- row are summarized again when the session is viewed.
CREATE TABLE IF NOT EXISTS summaries (
    id         INTEGER PRIMARY KEY,                           -- the viewer's summary cursor
    event_id   INTEGER NOT NULL UNIQUE REFERENCES events (id), -- the event that starts the entry
                                                              -- (a tool call's PreToolUse)
    summary    TEXT NOT NULL
);

-- Small values an adapter keeps between its runs, such as how far it has read a transcript.
-- Keys are prefixed with the adapter's name; Ichnos itself never reads them.
CREATE TABLE IF NOT EXISTS adapter_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

PRAGMA user_version = 3;
