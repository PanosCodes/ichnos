# Agent instructions

Ichnos records what a coding agent does and shows it in a native viewer. This file is for coding
agents working in the repo; `README.md` is for users.

## Layout

- `crates/ichnos-core`: the Ichnos event format (`event.rs`) and writer (`log.rs`), the
  database, the sessions catalog. Shared with the CLI, so no async runtime and no viewer logic.
  No knowledge of any particular agent: it reads and writes only the format that `log.rs`
  describes, including the tool fields adapters fill in.
- `crates/ichnos-model`: the viewer's model of a session (feed, calls, tree, labels, filters).
  Pure and tested without a window; it reads only Ichnos events and their tool fields.
- `crates/ichnos`: the CLI, `ichnos log`, and the adapters in `src/adapter/` that translate an
  agent's own hook payloads (Claude Code only, so far). It runs on every agent event, so keep it
  small and fast. Anything agent-specific belongs in an adapter, never in core or the viewer.
- `crates/ichnos-app`: the viewer, built with GPUI and GPUI Kit. State lives in `view_state.rs`
  (plain Rust, testable without a window); rendering lives in `viewer.rs` and `viewer/`. The
  summarizer (`summarizer.rs`, `llm.rs`) runs here, on a thread of its own.
- `crates/ichnos-search`: search by meaning. Embeds prompts, errors and summaries with
  `model2vec-rs` into a `sqlite-vec` index (`~/.ichnos/index.db`). Only the viewer uses it.
- `crates/ichnos-vec`: registers `sqlite-vec` with SQLite. Nothing else lives here.
- `plugin/`: the Claude Code plugin (TypeScript function hooks) that `ichnos install` writes
  next to the settings file. When a debug profile is set, it forks the session after each turn,
  asks for the model's account in the compact form, checks it and hands it to `ichnos log` as a
  `Debug` event. `hooks/profiles.ts` (prompts and schemas) is written by `ichnos-core`'s tests
  from `debug/compact.rs`; regenerate it with `ICHNOS_BLESS=1 cargo test -p ichnos-core debug`.
  Test the plugin with `claude plugin test plugin`.

## Commands

```sh
mise run check     # fmt --check, clippy -D warnings, tests: the same as CI
mise run app       # run the viewer from source
mise run shots     # headless PNGs of every view, light and dark, narrow and wide, in ./shots
```

After a view change, run `mise run shots [session-id]` and look at the PNGs (light and dark, 900 and
1400 wide) before saying it is done; the shots need no screen access.
Run `mise run check` before saying a change is done. CI runs the same three steps on macOS.

## Rust

- Read `.agents/skill/rust-best-practices/SKILL.md` before writing Rust.
- `unsafe_code` is forbidden. The one exception is the registration call in `crates/ichnos-vec`,
  which has its own `[lints]` table for that reason; don't add `unsafe` anywhere else. Clippy `pedantic` is on, and `unwrap`/`expect` warn outside tests.
- Name a module's file after the module (`adapter.rs` next to `adapter/`), never `mod.rs`; in
  this repo "mod" reads as a Claude Code mod. Clippy's `mod_module_files` enforces it.
- Write doc comments for public items, and put backticks around identifiers and product names
  in them (`doc_markdown` is enforced).
- Keep logic that doesn't need a window in `view_state.rs` and unit-test it there. Use the GPUI
  tests in `viewer/tests.rs` only for behavior that needs rendering or input.

## Accessibility

The viewer draws its own widgets, so anything not declared to the macOS accessibility tree is
invisible to screen readers and to keyboard-click tools such as Homerow. Every view change must
account for this:

- Every clickable element needs a role and a label. A plain `div().id(..).on_click(..)` has
  neither, so add `.role(Role::..)` and `.aria_label(..)` (`accesskit::Role`).
- Pick the role that matches the behavior: `Button` for actions and rows, `Tab` for view and
  inspector switchers (with `.aria_selected(..)`), `Link` for link-like actions.
- Label with what the control does or what the row holds ("Zoom out", "Bash: git status"), never
  with the glyph alone: icon-only controls such as `×`, `◀` and `▶|` must have a written name.
- Labels change with state: a play/pause toggle says "Pause" while playing.
- Reuse the shared helpers (`button` and `link` in `viewer/inspector.rs`, the replay `button`
  closure) instead of building new unlabeled controls; they set the role and label already.
- Keep every action reachable from the keyboard as well as the mouse.
- When adding a view, panel or dialog, check each interactive element in it before you finish.
  The `role` and `aria_*` methods need no extra setup, and the `accesskit` dependency is already
  there.
- Don't rely on color alone to convey state (errors, selection); pair it with text or a label.
