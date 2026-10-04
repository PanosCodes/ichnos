# Agent instructions

Ichnos records what Claude Code does and shows it in a native viewer. This file is for coding
agents working in the repo; `README.md` is for users.

## Layout

- `crates/ichnos-core`: database, live feed, session model, summarizer. No UI.
- `crates/ichnos`: the hook and CLI. It runs on every Claude Code event, so keep it small and fast.
- `crates/ichnos-app`: the viewer, built with GPUI and GPUI Kit. State lives in `view_state.rs`
  (plain Rust, testable without a window); rendering lives in `viewer.rs` and `viewer/`. The
  summarizer (`summarizer.rs`, `llm.rs`) runs here, on its own tokio runtime.

## Commands

```sh
mise run check     # fmt --check, clippy -D warnings, tests: the same as CI
mise run app       # run the viewer from source
```

Run `mise run check` before saying a change is done. CI runs the same three steps on macOS.

## Rust

- Read `.agents/skill/rust-best-practices/SKILL.md` before writing Rust.
- `unsafe_code` is forbidden. Clippy `pedantic` is on, and `unwrap`/`expect` warn outside tests.
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
