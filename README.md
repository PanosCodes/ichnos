# Ichnos

Ichnos records everything a coding agent does (every prompt, tool call, subagent and model
reply) and shows it as a live timeline in a native app. It works with Claude Code out of the box,
and with anything else that can write Ichnos events.

> **Status:** early (0.1). The event format and the database schema may still change between
> versions.

Your logs stay on your machine, in `~/.ichnos`. Ichnos has no server and no account. Any AI
work, such as the one-line summaries, runs through your own AI tools (by default your Claude Code
login), and you can swap in another command or turn it off.

## Getting started

### Requirements

- macOS 11 or later. The viewer renders with Metal, and CI builds and tests on macOS only.
- The Xcode command line tools.
- [mise](https://mise.jdx.dev), which installs the right Rust version.

### 1. Install

From this folder:

```sh
mise install
mise run install
```

This puts `ichnos` (the hook and CLI) and `ichnos-app` (the viewer) in `~/.cargo/bin`. Claude
Code runs `ichnos` by name, so that folder must be on your `PATH`:

```sh
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
```

Run `mise run install` again whenever you pull changes.

### 2. Turn on logging

```sh
ichnos install            # Claude Code, all projects (~/.claude/settings.json)
ichnos install --project  # Claude Code, this project only (.claude/settings.local.json)
```

This adds Ichnos's hooks to your Claude Code settings, leaving the rest of the file untouched,
and installs the `ichnos` skill next to it. Start a new Claude Code session and it gets logged.

To turn logging off, run `ichnos uninstall` (add `--project` if you installed it that way).
Your recorded sessions are kept.

Sessions recorded before Ichnos logged the model's text can get it from their transcript:
`ichnos import claude-code <session-id>`.

### Other agents

Ichnos takes one input format. Anything that can print JSON (a plugin, a wrapper, your own loop
calling a model API) pipes Ichnos events into `ichnos log`:

```sh
echo '{"event":"UserPromptSubmit","session_id":"demo-1","prompt":"hi"}' | ichnos log
```

## Commands

| Command | What it does |
| --- | --- |
| `ichnos open` | Open the viewer on the newest session |
| `ichnos open <session-id>` | Open the viewer on one session |
| `ichnos open --view graph` | Open the viewer on a view: `tree`, `graph` or `table` |
| `ichnos stop` | Quit the viewer |
| `ichnos log` | Store Ichnos events read from stdin, one JSON object per line |
| `ichnos hook claude-code` | Store one Claude Code hook payload (what `ichnos install` wires up) |
| `ichnos import claude-code <session-id>` | Add the model's text to an older session from its transcript |

Only one viewer runs at a time; `ichnos open` brings the running one forward.

| Setting | Effect |
| --- | --- |
| `ICHNOS_NO_OPEN=1` | New sessions don't open the viewer |
| `ichnos open --no-summaries` | Start the viewer without the one-line summaries (see below) |
| `--settings <file>` | Make `install` / `uninstall` change a different settings file |

## Using the viewer

- **Tree** is the main view: one row per prompt you sent, with its outcome, tokens and time.
  Expand a row to see its tool calls and model steps; Skill calls show the skill name as a tag.
  A subagent's work sits under the call that started it.
- **Inspector** (right side) shows the selected row's input and output, the calls around it, and
  a **Replay from here** button. Its other tabs show what was running at that moment, the todo
  list, files read and changed, and token use.
- **Graph** shows one card per agent with its tools and timings. Click a card to see its prompt
  and result.
- **Table** lists every event; Skill calls show the skill name as a tag.
- **Filter** by typing words, or `is:error`, `is:running`, `tool:Bash`, `agent:Explore`,
  `file:feed.rs`.
- **Replay** with the strip under the toolbar: click anywhere on it, or press `space` to play.
  `▶|` goes back to live.
- **Sessions** (`⌘K` / `Ctrl+K`) lets you search all sessions by first prompt, folder or id.
- **Ask agent** builds a prompt you paste into Claude Code, so it can answer questions about the
  session from the log.
- **Summaries**: each step gets a plain-English one-liner from Claude Haiku, using your Claude
  Code login.

`ichnos open <session-id> --view graph` pins a session and picks the view.

### Keyboard shortcuts

| Key | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `↵` | Expand row |
| `e` | Jump to the next error |
| `g` / `t` / `b` | Graph / tree / table |
| `/` | Focus the filter |
| `space` | Play / pause replay |
| `⌘K` | Switch session |

## Asking agents about sessions

The `ichnos` skill teaches your agent where the logs are and how to query them. Ask things
like:

- "Why did the last Bash call fail?"
- "Which files did yesterday's session in this repo change?"
- "What did the Explore subagent do?"

## Where the data lives

```
~/.ichnos/
  sessions/<session-id>.db   # one SQLite file per session
  hook-errors.log            # hook and summarizer errors (the hook never blocks Claude Code)
  app.log                    # the viewer's log when `ichnos open` started it
  app.sock                   # how `ichnos open` reaches the running viewer
```

Delete session files you don't need. The tables are described in
[`crates/ichnos-core/schema.sql`](crates/ichnos-core/schema.sql), and you can query
them directly. For example, which tools took the most time:

```sh
sqlite3 ~/.ichnos/sessions/<session-id>.db "
  SELECT record->>'tool_name' AS tool, count(*), sum(record->>'duration_ms') AS total_ms
  FROM events WHERE event LIKE 'PostToolUse%' GROUP BY tool ORDER BY total_ms DESC"
```

## Development

- `crates/ichnos-core`: the database, the live feed, the session model and the summarizer
- `crates/ichnos`: the hook and the CLI; it stays small because it runs on every Claude Code event
- `crates/ichnos-app`: the viewer, built with [GPUI](https://gpui.rs) and
  [GPUI Kit](https://gpui-kit.com)

```sh
mise run check     # format check, lint and tests (including the viewer in a headless window)
mise run app       # run the viewer from source
mise run install   # install both programs into ~/.cargo/bin
mise tasks         # list these
```
