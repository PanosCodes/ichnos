# Ichnos

When Claude Code goes off and does forty things, you usually find out what happened by
scrolling the terminal. Ichnos logs every prompt, tool call, subagent and reply instead, and
shows them as a live timeline in a native Mac app. You can see where the time went and why
something failed, and you can replay a session to look at any moment in it.

Everything stays in `~/.ichnos` on your machine. There's no server and no account.

It's early (0.1), so the log format may still change.

## Install

You need macOS 11+, the Xcode command line tools and [mise](https://mise.jdx.dev).

```sh
mise install
mise run install          # puts ichnos and the viewer in ~/.cargo/bin (keep that on your PATH)
ichnos install            # adds the hooks to ~/.claude/settings.json
```

Start a new Claude Code session and the viewer opens on it. Run `ichnos install --project` to
log a single project instead, and `ichnos uninstall` to stop logging (your recorded sessions are
kept). Run `mise run install` again after you pull.

## Usage

The tree view has one row per prompt. Expand a row to see the tool calls and subagents under it,
and select one to see its input and output on the right. Drag along the strip under the toolbar
to replay the session to that point. `⌘K` switches sessions, and it also finds sessions that are
related by meaning, not just by matching words. Press `?` for the rest of the shortcuts.

Each step gets a one-line summary. Claude Haiku writes them by default, using your Claude Code
login. In Settings (`⌘,`) you can use Apple's on-device model instead, or turn the summaries
off.

`ichnos install` also adds a skill, so you can ask your agent things like "why did the last Bash
call fail?" or "which files did yesterday's session change?".

## Debug profiles

To find out where your skills or instructions fall short, pick an "Ichnos debug profile" in
Claude Code's `/config` (`skills`, `task`, `env` or `full`). After each turn, the model explains
in a hidden fork what it did and what it missed, and the answer shows up as a **debug** row. It
costs an extra request per turn, so leave it `off` when you're not tuning anything. It only works
with the all-projects install.

## Other agents

Anything that can print JSON can feed Ichnos:

```sh
echo '{"event":"UserPromptSubmit","session_id":"demo-1","prompt":"hi"}' | ichnos log
```

## Data

Each session is a SQLite file in `~/.ichnos/sessions/`, so you can delete old ones or query them
directly ([schema](crates/ichnos-core/schema.sql)). If something goes wrong, look in
`~/.ichnos/hook-errors.log`. Hook errors never block the agent.

```sh
sqlite3 ~/.ichnos/sessions/<id>.db "
  SELECT record->>'tool_name' AS tool, sum(record->>'duration_ms') AS ms
  FROM events WHERE event LIKE 'PostToolUse%' GROUP BY tool ORDER BY ms DESC"
```

## Development

`mise run app` runs the viewer from source, and `mise run check` runs the same checks as CI.
[`docs/architecture.md`](docs/architecture.md) explains how the pieces fit together, and
[`AGENTS.md`](AGENTS.md) has the repo conventions.
