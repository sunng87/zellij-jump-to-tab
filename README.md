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

The plugin must be built against the same zellij version you run
(`zellij-tile = "0.45.0"` here; bump Cargo.toml when zellij upgrades).

## Install (keybinding)

Add to your zellij config (`~/.config/zellij/config.kdl`) — replace the path
with the absolute path to the built `.wasm` on your machine:

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
Iterate: edit → `cargo build --release` → reopen. The unit tests for the
fuzzy matcher run on the host target (use your host cargo — the pure nix
devshell binary has a loader mismatch):

```sh
cargo test --target x86_64-unknown-linux-gnu
```

## Design notes

- The tab list comes from `Event::TabUpdate` (full `Vec<TabInfo>` pushed on
  every change — no polling), tracking `position`/`name`/`active`.
- Jumping uses `go_to_tab(position + 1)` — the plugin API's `go_to_tab`
  is **1-indexed** (verified against zellij 0.45.0's screen handler), and
  position-based jumps stay correct even with duplicate tab names.
- The popup closes with `hide_self()`, not `close_self()`, so the plugin
  stays warm.
- See `RESEARCH.md` for the full API survey this design is based on.
