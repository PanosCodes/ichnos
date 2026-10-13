# Ichnos architecture

How Ichnos is put together: the processes that run, the files they share, and which crate owns
what. For working in the repo see `AGENTS.md`.

## The big picture

Ichnos is two binaries and a set of SQLite files. Nothing talks over the network, and the two
binaries never share memory: the session databases are the contract between them.

```text
 coding agent (Claude Code)            anything else that writes Ichnos events
   │ hook payload (JSON, stdin)                 │ Ichnos events (JSON lines, stdin)
   ▼                                            ▼
 ichnos hook claude-code ──► adapter ──►  ichnos log
   │ (also reads the agent's transcript)        │
   └──────────────────────┬─────────────────────┘
                          ▼
                    log::Logger  (ichnos-core)
                          │ append
                          ▼
           ~/.ichnos/sessions/<session-id>.db   ◄── summarizer writes `summaries`
                          │ poll                     (inside ichnos-app)
                          ▼
     ichnos-app: Feed → SessionData → SessionModel → ViewState → GPUI views
                          │
                          └── search indexer ──► ~/.ichnos/index.db

 ichnos open / stop ──── one JSON line over ~/.ichnos/app.sock ────► ichnos-app
```

- **`ichnos`** is short-lived: the agent runs it once per hook event. It translates the payload,
  appends events to the session's database and exits. On `SessionStart` it also starts
  `ichnos open` as a detached process, so the hook never waits for the viewer.
- **`ichnos-app`** is long-lived: one instance at a time. It polls the database of the session it
  shows, rebuilds its model from what it has read, and draws it. It also runs the summarizer and
  the search indexer in the background.

## Crates

```text
ichnos ──────────► ichnos-core ◄──── ichnos-model ◄──── ichnos-app
                        ▲                                   │
                        └────────── ichnos-search ◄─────────┘
                                         │
                                         ▼
                                    ichnos-vec
```

| Crate | Kind | Owns |
| --- | --- | --- |
| `ichnos-core` | library | The event format (`event`) and its one writer (`log`), the SQLite store, the sessions catalog, the app socket protocol and the debug account format. No UI, no async runtime, and no knowledge of any agent. |
| `ichnos-model` | library | The viewer's side of a session: the live feed, calls and entry keys, the session model, labels, filters and the "Ask agent" prompt. Pure apart from the feed's reads; no window, no async runtime. |
| `ichnos` | binary | The CLI: `log`, `hook`, `import`, `install`/`uninstall`, `open`/`stop`. Agent adapters live in `src/adapter/` (only `claude_code` so far). |
| `ichnos-app` | binary | The GPUI viewer: window, views, input, settings, headless screenshots, and its background work (the summarizer and the command it runs). |
| `ichnos-search` | library | Search by meaning: `model2vec-rs` embeddings in a `sqlite-vec` index. Only the viewer uses it. |
| `ichnos-vec` | library | Registers `sqlite-vec` with SQLite. The only crate allowed `unsafe`, for that one call. |

The CLI depends on `ichnos-core` only, so the binary every hook event starts stays small and
quick to start: Cargo, not review, keeps the model and the summarizer off the hook path.
Everything heavy (GPUI, embeddings, the summarizer) is in the app.

### Where agent knowledge lives

Only the adapters know about a particular agent. Core reads and writes Ichnos events, and the
viewer shows Ichnos events; neither checks which agent produced them. An agent that can emit Ichnos
events itself needs no adapter at all: it pipes them into `ichnos log`. An adapter exists for
agents, like Claude Code, whose hooks can only run a command with their own payload.

## Data on disk

Everything lives in the data directory, `~/.ichnos` unless `--dir` or `ICHNOS_DIR` say otherwise.

| Path | Written by | Read by | Holds |
| --- | --- | --- | --- |
| `sessions/<session-id>.db` | `ichnos` (events, adapter state), the app's summarizer (summaries) | the app, the `ichnos` skill's SQL queries | One session. |
| `index.db` | the app's search indexer | the session switcher | Indexed text and vectors for every session. |
| `settings.json` | the app's Settings window | the app at launch | Saved settings: the chosen summarizer. |
| `app.sock` | the app (binds it) | `ichnos open` / `ichnos stop` | The single-instance socket. |
| `hook-errors.log` | anything with nobody to tell | people | Failures in hooks and background workers. |

### A session database

The schema is `crates/ichnos-core/schema.sql`, applied by `store.rs` according to
`PRAGMA user_version`. `store.rs` is the only module that writes SQL against it.

- **`events`**: one row per Ichnos event. `record` is the full event as JSON, exactly what the
  viewer receives; `epoch_ms`, `event` and `tool_use_id` are copies kept for indexed lookups.
  There is no session column: the file is the session. `id` only increases, which makes it the
  viewer's resume cursor.
- **`summaries`**: one sentence per finished entry, keyed by the `events.id` that starts the
  entry. It is a cache: entries without a row are summarized again when the session is viewed.
- **`adapter_state`**: small values an adapter keeps between runs, such as how far it has read
  a transcript. Keys carry the adapter's name; core never reads them.

One file per session keeps writers apart: concurrent hook processes of different sessions never
touch the same file, and deleting a session is deleting a file. WAL mode and a busy timeout let
the hook processes of one session and the viewer write and read the same file at once.

## Writing: from agent event to row

1. Claude Code runs `ichnos hook claude-code` for each hook in `hooks.json` (`PreToolUse`,
   `PostToolUse`, `PostToolUseFailure`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`,
   `SessionStart`, `Stop`) with the payload on stdin.
2. The adapter (`adapter/claude_code/mod.rs`) maps the payload to an Ichnos event; Claude Code's
   hook names are the Ichnos event names, so this is mostly field by field. Subagent calls carry
   `agent_id` / `agent_type`, so each subagent becomes its own lane.
3. Hooks don't carry the model's text, so the adapter also reads the lines added to the session's
   transcript since its last run (`transcript.rs`) and turns each assistant line into an
   `AssistantMessage` event. Its read offset lives in `adapter_state`, so concurrent hook runs
   never log a line twice.
4. `log::Logger` fills in what the producer may leave out (time, the main agent), clips long
   fields, pairs a `PostToolUse` with its `PreToolUse` to set `status` and `duration_ms`, moves any
   debug account into place, and appends the record.
5. On `SessionStart`, the CLI also spawns `ichnos open <session>` detached (unless
   `ICHNOS_NO_OPEN` is set).

`ichnos log` is steps 4–5 for producers that already write Ichnos events.
`ichnos import claude-code <session>` reruns step 3 over a whole transcript, for sessions logged
before the adapter read transcripts.

## Reading: from row to pixels

The viewer's pipeline runs on every poll that brings something new:

```text
events / summaries rows
   │ Feed::poll                 (model/feed.rs: cursors on events.id and summaries.id)
   ▼
SessionData                     (model/session_data.rs: events in order, summaries by key,
   │                             AssistantMessage pieces merged into Steps)
   │ build_session(data, Moment)
   ▼
SessionModel                    (model/model.rs: turns → calls, steps, subagents; per-agent
   │                             and per-moment numbers; pure, no clock or I/O)
   │ ViewState::tree_rows / table_rows
   ▼
rows                            (app/view_state.rs: view, selection, expansion, filter,
   │                             replay cursor; no GPUI types)
   ▼
GPUI views                      (app/viewer.rs and viewer/: tree, graph, table, inspector,
                                 toolbar, replay strip, switcher, ask panel)
```

- **Polling.** The shown session's feed is polled every 250 ms on GPUI's background executor
  (`viewer.rs`, `live.rs`); the sessions list (`catalog.rs`) every 3 s. The catalog rereads a
  database only when its modification time changes. The database is the only source; there is
  no push channel from the hook to the app.
- **Rebuilding.** The model is rebuilt from `SessionData` rather than patched. `Moment` carries
  the replay cursor, whether the session is live, and the wall clock, which is how one pure
  function serves both the live view and replay: replaying is building the model as of an
  earlier time.
- **Tree shape.** One turn per prompt. A turn holds model steps and calls; a subagent's work
  hangs under the `Agent` call that started it (matched by `agentId`), or under its turn
  otherwise. Nodes live in one arena and refer to each other by `NodeId`.
- **Presentation.** `model/labels.rs` (chips, text, durations, tokens) and `app/present.rs`
  (tone, icon, badge, status) decide what a row says; views only turn that into layout and color.
- **Testing seam.** Everything above the GPUI views is plain Rust and unit-tested without a
  window. `viewer/tests.rs` covers what needs rendering or input, and the `screenshots` feature
  (`mise run shots`) renders every view headlessly to PNGs.

## Background work in the app

### Summarizer

`app/summarizer.rs`, on a thread of its own, so it never blocks GPUI's executors.

- The viewer holds a `Watch` guard for the session it shows; the summarizer works on watched
  sessions only, and stops when the guard drops.
- It pairs tool events into entries like the viewer does, skips running calls and model messages,
  and sends the rest to the model in batches, newest first, so the live end fills in first.
- The model is a shell command (`app/llm.rs`): prompt on stdin, system prompt in
  `$ICHNOS_SYSTEM_PROMPT`, reply on stdout, paired with a batch size (`SummaryModel`), since a
  small on-device model holds fewer entries. The summarizer names no model provider.
- The app's `summarizers.rs` lists the choices (Claude Code's `claude -p --model haiku`, Apple
  Foundation Models' `fm respond`, Off) and checks which this Mac can run. The Settings window
  picks one and `Summarizer::set_model` swaps it between batches; the choice is saved in
  `settings.json`. `--summarizer` or `ICHNOS_SUMMARIZER` override it for one launch, and
  `--no-summaries` turns the summarizer off.
- An app started from the Dock gets only the system `PATH`, so `summarizers.rs` reads the
  interactive login shell's `PATH` once (`$SHELL -ilc`) and passes it to the summarizer commands
  and the availability check; `claude` is often added to `PATH` in `~/.zshrc`.
- Sentences go into the session's `summaries` table, where the viewer's feed picks them up like
  any other new row.

### Search by meaning

`ichnos-search`, started by the app unless `--no-search`.

- A background thread walks every session database, picks the text worth indexing (prompts,
  errors of failed calls, summaries; `docs.rs`), embeds it with a Model2Vec static model
  (256 dimensions, downloaded once) and stores it in `index.db`'s `vec0` table.
- Per-session cursors on `events.id` and `summaries.id` (the `cursors` table) make indexing
  incremental. Changing the embedding model rebuilds the index, since vectors from different
  models can't be compared.
- The `⌘K` switcher queries it and `rank.rs` folds the nearest texts into one result per
  session, listed under the sessions that match by words.

## The CLI and the app: one instance

`core/instance.rs` defines the protocol: one JSON command per line over the Unix socket
`<data dir>/app.sock`, answered with `ok`. The app binds the socket; only one process can, so a
second launch hands its request to the first and exits.

`ichnos open [<session>]` sends "show this session" and starts `ichnos-app` in the background
when nobody answers. `ichnos stop` sends "quit". When the hook opens a new session and an app is
already running, the app is left alone: it lists the new session on its next catalog poll, and
switches to it only if "follow newest" is on.

## Debug accounts (the plugin)

Besides what the agent did, Ichnos can store the model's own account of each turn, in the shape
`docs/ichnos-debug.schema.json` describes.

- `core/debug/compact.rs` defines a compact form (one-letter keys, `"v": 2`) that models are
  asked to write, and expands it into the stored full form (`"v": 1`). The prompt, the schema
  and the expansion all come from one key table, so they can't disagree. A `Profile` picks which
  sections are worth asking for.
- The Claude Code plugin (`plugin/`, installed by `ichnos install`) does the asking. With a debug
  profile set, after each main-agent turn it forks the session, asks for the account, validates
  it against the schema (retrying once), and pipes a `Debug` event to `ichnos log`. It does this
  without holding up the turn.
- The plugin's prompts and schemas (`plugin/hooks/profiles.ts`) are generated from
  `compact.rs` by core's tests; regenerate with `ICHNOS_BLESS=1 cargo test -p ichnos-core debug`.

## Letting agents read the logs

`ichnos install` also installs the `ichnos` skill (`skill/SKILL.md`), which tells an agent where
the session databases are and how to query them with SQL. The viewer's **Ask agent** panel
(`model/ask.rs`) builds a prompt with the same information plus the selected row and the user's
question; Ichnos itself never calls the agent. Both work because the database layout is stable
and `record` keeps every event whole.

## Rules that keep this shape

- Agent-specific code goes in an adapter, never in core or the viewer.
- `ichnos-core`'s `log::Logger` is the only writer of events; `store.rs` is the only module with
  the session SQL, `ichnos-search/src/index.rs` the only one with the index SQL.
- The CLI stays small: no GPUI, no search, nothing slow on the hook path.
- The model and labels are pure; time comes in through `Moment`.
- Viewer state that doesn't need a window lives in `view_state.rs` and is unit-tested there.
- `unsafe` lives only in `ichnos-vec`.

## Where to start for common changes

| Change | Start in |
| --- | --- |
| Support another agent | a new module in `crates/ichnos/src/adapter/` |
| Add an event field or event name | `core/log.rs`, `core/event.rs` |
| Change the database schema | `crates/ichnos-core/schema.sql` and `SCHEMA_VERSION` in `store.rs` |
| Change how a session is grouped into rows | `model/model.rs`, then `app/view_state.rs` |
| Change what a row says | `model/labels.rs`, `app/present.rs` |
| Add or change a view | `app/viewer/` and `app/view_state.rs`; mind the accessibility rules in `AGENTS.md` |
| Change what gets summarized | `app/summarizer.rs` |
| Change what search indexes | `ichnos-search/src/docs.rs` |
| Change the debug account | `core/debug/compact.rs`, then regenerate `plugin/hooks/profiles.ts` |
