-- The search index: ~/.ichnos/index.db, one file for every session.
--
-- `Index::open` in src/index.rs applies this file and rebuilds the index when the embedding model
-- changes, since vectors from different models can't be compared.

-- Settings of the index itself; `model` names the model that made the vectors.
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- How far each session database has been read. Summaries arrive after their events, so each
-- source table has its own cursor.
CREATE TABLE IF NOT EXISTS cursors (
    session_id TEXT PRIMARY KEY,
    events     INTEGER NOT NULL,  -- last events.id indexed
    summaries  INTEGER NOT NULL   -- last summaries.id indexed
);

-- One row per piece of indexed text; `id` is the rowid of its vector in `vectors`.
CREATE TABLE IF NOT EXISTS docs (
    id         INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    event_id   INTEGER NOT NULL,  -- the session's events.id the text came from
    kind       TEXT NOT NULL,     -- prompt | error | summary
    text       TEXT NOT NULL,
    UNIQUE (session_id, event_id, kind)
);

-- The vectors; the dimension must equal `DIMENSIONS` in src/embedder.rs.
CREATE VIRTUAL TABLE IF NOT EXISTS vectors USING vec0 (
    embedding float[256] distance_metric=cosine
);
