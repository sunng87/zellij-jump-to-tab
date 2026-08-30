# jump-to-tab

A [zellij](https://zellij.dev) plugin: a popup fuzzy-finder over your tabs.
Type part of a tab name, hit Enter, jump. No mouse, no tab-bar hunting.

```
┌─ jump to tab ─────────────────────────┐
│ ❯ serv▏                               │
│                                       │
│   1 research        ● 3p              │
│ ❯ 2 server            2p              │
│   3 server-logs       1p              │
│                                       │
│ 1-9 quick-jump · ↑↓ select · ⏎ jump…  │
└───────────────────────────────────────┘
```

## Controls

| key | action |
|---|---|
| type | fuzzy-filter the tab list (matched chars highlighted) |
| `1`–`9` | jump straight to the Nth visible match (only when query is empty) |
| `↑` `↓` / `tab`-nav | move selection (wraps) |
| `Enter` | jump to the selected tab; with an empty query, jumps to the next tab |
| `Tab` | toggle to the previously active tab (Alt-Tab style) |
| `Backspace` / `Delete` | delete one char / clear the query |
| `Esc` | close |

## Build

Needs a rust toolchain with the `wasm32-wasip1` (aka `wasm32-wasi`) std.
With nix/flakes this is provided by the included flake:

```sh
nix develop -c cargo build --release
# -> target/wasm32-wasip1/release/jump_to_tab.wasm
```

The project's `.cargo/config.toml` sets `wasm32-wasip1` as the default
build target, so no `--target` flag is needed. Outside the devshell the
build fails loudly (the host toolchain has no wasm std) instead of
silently producing a host binary.

Reloading the plugin in a running session (no zellij restart needed):

```sh
zellij action start-or-reload-plugin file:$PWD/target/wasm32-wasip1/release/jump_to_tab.wasm
```

The plugin must be built against the same zellij version you run
(`zellij-tile = "0.45.0"` here; bump Cargo.toml when zellij upgrades).
Releases are tagged `v*` — pushing a tag triggers
[.github/workflows/release.yml](.github/workflows/release.yml), which runs
the unit tests, builds the wasm, and attaches it to a GitHub release.

## Install (keybinding)

Either build from source (see [Build](#build)), or download
`jump_to_tab.wasm` from the latest
[release](https://github.com/sunng87/zellij-jump-to-tab/releases/latest).

Add to your zellij config (`~/.config/zellij/config.kdl`) — replace the path
with the absolute path to the `.wasm` on your machine:

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

`floating true` makes it open as a centered overlay; `LaunchOrFocusPlugin`
toggles: the same key focuses it if already open. The plugin hides itself
(keeping its state) after every jump, so reopening is instant.

**Permissions:** on first use zellij will ask to grant
`ReadApplicationState` (to receive the tab list) and
`ChangeApplicationState` (to switch tabs) — choose *Always allow* and the
prompt never appears again.

## Development

```sh
nix develop
zellij -l zellij.kdl
```

The dev layout runs the plugin in a bottom pane next to a scratch terminal.
Iterate: edit → `nix develop -c cargo build --release` → reload with
`zellij action start-or-reload-plugin` (see Build). The unit tests for the
fuzzy matcher run on the host target (use your host cargo — the pure nix
devshell binary has a loader mismatch):

```sh
cargo test-host        # alias for: cargo test --target x86_64-unknown-linux-gnu
```

## Design notes

- The tab list comes from `Event::TabUpdate` (full `Vec<TabInfo>` pushed on
  every change — no polling), tracking `position`/`name`/`active`.
- Jumping uses `go_to_tab(position)` with the 0-indexed `TabInfo.position`.
  The plugin-facing `go_to_tab` is effectively 0-indexed on zellij 0.45:
  the host wraps it as `Action::GoToTab { index: n + 1 }`
  (`zellij_exports.rs`) and the screen switches with `switch_active_tab(index - 1)`
  (`screen.rs`), so a `+1` of our own lands one tab too far. Position-based
  jumps stay correct even with duplicate tab names.
- The popup closes with `hide_self()`, not `close_self()`, so the plugin
  stays warm.
- See `RESEARCH.md` for the full API survey this design is based on.
