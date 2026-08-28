# Feasibility: zellij "jump to tab by name" plugin

**Verdict: fully possible.** Every required primitive exists in the plugin API
of the installed zellij (0.45.0). Verified against the actual `zellij-tile`
0.45.0 / `zellij-utils` 0.45.0 crate sources, the zellij v0.45.0 server test
suite, and the shipped `default.kdl`.

## The three required capabilities

### 1. Enumerate tabs (with names) — ✅ `Event::TabUpdate`

Subscribe in `load()`:

```rust
use zellij_tile::prelude::*;

fn load(&mut self, _: BTreeMap<String, String>) {
    subscribe(&[EventType::TabUpdate]);
}
```

Zellij then delivers `Event::TabUpdate(Vec<TabInfo>)` on plugin load and on
**every tab change** (create/close/rename/reorder/switch). No polling, no
guessing tab ids. `TabInfo` (zellij-utils 0.45.0, `data.rs:2256`) carries:

| field | use for this plugin |
|---|---|
| `position: usize` | 0-indexed tab position (stable ordering) |
| `name: String` | the name as shown in the tab bar (fuzzy-match target) |
| `active: bool` | current tab (to mark/exclude in the UI) |
| `is_fullscreen_active`, `is_sync_panes_active`, `selectable_*_panes_count`, ... | optional decoration |

Note: unnamed tabs have auto-generated names (e.g. "1"); fuzzy-match on both
`name` and rendered `position + 1` for good UX.

### 2. Switch to the chosen tab — ✅ `go_to_tab_name` / `go_to_tab`

- `shim::go_to_tab_name(tab_name: &str)` — switch by exact name
- `shim::go_to_tab(tab_index: u32)` — switch by position (safer for duplicates)
- `shim::focus_or_create_tab(tab_name: &str) -> Option<usize>` — bonus: create if missing
- `shim::switch_tab_to(tab_idx: u32)` — switch within tab mode

Permission (verified in zellij v0.45.0 server plugin tests,
`zellij-server/src/plugins/unit/plugin_tests.rs:4141`):
**`PermissionType::ChangeApplicationState`** for `go_to_tab_name`, plus
**`ReadApplicationState`** for the event subscription. Request at runtime:

```rust
request_permission(&[
    PermissionType::ReadApplicationState,
    PermissionType::ChangeApplicationState,
]);
```

→ user gets zellij's native permission prompt (result arrives as
`Event::PermissionRequestResult`); answer can be remembered per-session or
pre-granted in the layout/plugin config (`permissions { ... }` block).

### 3. The "type to filter" UI — ✅ `Event::Key` + `render`

- While the plugin pane is focused, every keystroke arrives as
  `Event::Key(KeyWithModifier)` (chars, Backspace, Enter, Esc, arrows).
  There is no text-input widget — handle char push/pop and Enter/Esc manually
  (standard for all zellij finder plugins).
- `render(rows, cols)` draws the UI; zellij-tile ships styled components in
  `ui_components` (`Text` with `.selected()`, `.error_color_substring()`,
  `Table`, `NestedList`, `Ribbon`) — enough for a fuzzy-match list with the
  query highlighted. Or just print styled lines.
- `show_cursor(Some((row, col)))` puts the terminal cursor after the query
  string.

## Popup lifecycle (the "quick jump" UX)

Keybinding opens the plugin floating — exact syntax from zellij 0.45.0's
`default.kdl` (this is how the built-in session-manager is bound):

```kdl
keybinds {
    normal {
        bind "Ctrl y" {
            LaunchOrFocusPlugin "file:/path/to/jump_to_tab.wasm" {
                floating true
                move_to_focused_tab true
            };
        }
    }
}
```

`LaunchOrFocusPlugin` (`zellij-utils` `input/actions.rs:386`) reuses the
already-running plugin pane if open, opens+focuses it otherwise — perfect
toggle semantics.

After selection / Esc, the plugin closes itself:

- `close_self()` — dispose the pane (fresh state next open)
- `hide_self()` — suppress the pane but keep state (instant reopen; the
  `LaunchOrFocusPlugin` binding brings it back)

Edge case to handle: with `hide_self()`, focus may stay on the hidden pane in
some versions — the established pattern (used by the built-in session
manager) is `hide_self()` on Esc/Enter; verify behavior on 0.45 during
development.

Optional power feature: `intercept_key_presses()` (needs
`PermissionType::InterceptInput`) + `Event::InterceptedKeyPress` allows a
global hotkey that works even when the plugin is not focused — how plugins
like `zhop`/`room` get harpoon-style global jumps. Start without it.

## Build & packaging

- Rust → **wasm32-wasi** target. On Rust ≥1.84 the target is
  `wasm32-wasip1` (renamed; output is compatible).
- `Cargo.toml`: `zellij-tile = "0.45.0"` (keep in lockstep with the zellij
  binary — a mismatch shows the "plugins aren't compatible" panic).
- Dev loop from the official
  [rust-plugin-example](https://github.com/zellij-org/rust-plugin-example):
  build the `.wasm`, load a dev layout (`zellij -l zellij.kdl`), edits
  hot-reload on `save` when running the dev layout.
- Fuzzy matching: bring a tiny dep (e.g. `fuzzy-matcher` or `nucleo-matcher`)
  — check they compile to wasm32-wasi (pure-rust ones do).

## Prior art (pattern proven many times)

- `Jedsek/tab-finder` — literally this: fuzzy-find + jump to tabs
- `theherk/zellij-tab-switcher`
- `gkstmdgus/zhop` — harpoon-style marks + jumps
- `rvcas/room`, `drop-stones/zellij-loom`, `imsnif/monocle` (fuzzy over panes)

## Risks / caveats

1. `Event::Key` only reaches the plugin while focused → the keybinding must
   open it (that's the intended UX anyway).
2. Duplicate tab names: `go_to_tab_name` switches to the first match;
   disambiguate with `go_to_tab(position)` from the selected `TabInfo`.
3. API lockstep: plugin wasm must be rebuilt per zellij major (0.44 vs 0.45
   differ). Pin `zellij-tile = "0.45.0"`.
4. `zellij-utils`/`zellij-tile` are not on docs.rs with full docs for every
   release — the crate source (as surveyed here) is the reference.

## Reference scratch

This survey was made against the published crate sources:
`zellij-tile 0.45.0` and `zellij-utils 0.45.0` (from crates.io).
