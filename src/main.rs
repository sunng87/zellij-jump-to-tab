//! jump-to-tab — a zellij plugin: popup fuzzy finder over tabs.
//!
//! Open it with a keybinding (see README.md), type part of a tab name,
//! hit Enter to jump. `1`-`9` jump straight to a visible match.

mod fuzzy;

use std::collections::BTreeMap;
use zellij_tile::prelude::*;

// ---------------------------------------------------------------------------
// styling
// ---------------------------------------------------------------------------

const RESET: &str = "\u{1b}[m";
const BOLD: &str = "\u{1b}[1m";
const DIM: &str = "\u{1b}[2m";
const CYAN: &str = "\u{1b}[36m";
const GREEN: &str = "\u{1b}[32m";
const MAGENTA: &str = "\u{1b}[95m";
const WHITE: &str = "\u{1b}[97m";
const RED: &str = "\u{1b}[31m";

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

struct Match {
    position: usize, // TabInfo.position (0-indexed)
    name: String,
    pane_count: usize,
    is_active: bool,
    matched: Vec<usize>, // highlighted char indices into `name`
}

#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    query: String,
    selected: usize,      // index into the filtered match list
    first_visible: usize, // scroll window into the match list
    rows: usize,
    cols: usize,
    current_tab: Option<usize>,  // position of the active tab
    previous_tab: Option<usize>, // position of the previously active tab
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::TabUpdate,
            EventType::Key,
            EventType::Visible,
            EventType::PermissionRequestResult,
        ]);
        self.sync_cursor();
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::TabUpdate(tabs) => {
                self.update_tabs(tabs);
                true
            },
            Event::Key(key) => {
                self.handle_key(key);
                self.sync_cursor();
                true
            },
            Event::Visible(visible) => {
                if visible {
                    self.reset();
                    self.sync_cursor();
                }
                visible
            },
            // re-render once permissions are granted (TabUpdate arrives then)
            Event::PermissionRequestResult(_) => true,
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.rows = rows;
        self.cols = cols;
        print!("{}", self.render_ui());
        // NOTE: never call shim commands (show_cursor, …) from render()!
        // On zellij 0.45.0 plugin commands and rendered content share one
        // stdout stream: a command issued inside render() makes the host
        // consume the rendered text while failing to parse the command,
        // leaving the pane completely blank.
    }
}

// ---------------------------------------------------------------------------
// tab tracking
// ---------------------------------------------------------------------------

impl State {
    fn update_tabs(&mut self, tabs: Vec<TabInfo>) {
        let active = tabs.iter().find(|t| t.active).map(|t| t.position);
        if let Some(pos) = active {
            if Some(pos) != self.current_tab {
                self.previous_tab = self.current_tab;
                self.current_tab = Some(pos);
            }
        }
        self.tabs = tabs;
        let len = self.matches().len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    /// Put the terminal cursor right after the query text ("❯ " + query).
    /// Only ever called from `load`/`update` — see the note in `render`.
    fn sync_cursor(&self) {
        show_cursor(Some((2 + self.query.chars().count(), 0)));
    }

    /// Fresh session every time the popup opens.
    fn reset(&mut self) {
        self.query.clear();
        self.first_visible = 0;
        // preselect the tab after the current one, so Enter with an empty
        // query acts as "next tab"; wraps around to the first tab.
        self.selected = self
            .matches()
            .iter()
            .position(|m| self.current_tab.map_or(true, |c| m.position > c))
            .unwrap_or(0);
    }

    fn matches(&self) -> Vec<Match> {
        let to_match = |t: &TabInfo, matched: Vec<usize>| Match {
            position: t.position,
            name: t.name.clone(),
            pane_count: t.selectable_tiled_panes_count + t.selectable_floating_panes_count,
            is_active: t.active,
            matched,
        };

        if self.query.is_empty() {
            return self.tabs.iter().map(|t| to_match(t, vec![])).collect();
        }

        let mut scored: Vec<(i64, usize, Match)> = self
            .tabs
            .iter()
            .filter_map(|t| {
                fuzzy::fuzzy_match(&self.query, &t.name)
                    .map(|(score, matched)| (score, t.position, to_match(t, matched)))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, m)| m).collect()
    }
}

// ---------------------------------------------------------------------------
// input
// ---------------------------------------------------------------------------

impl State {
    fn handle_key(&mut self, key: KeyWithModifier) {
        // only plain (optionally shifted) keys drive the finder
        let plain = key
            .key_modifiers
            .iter()
            .all(|m| matches!(m, KeyModifier::Shift));

        match key.bare_key {
            BareKey::Enter if plain => self.jump_selected(),
            BareKey::Esc if plain => hide_self(),
            BareKey::Tab if plain => self.toggle_last_tab(),
            BareKey::Up if plain => self.move_selection(-1),
            BareKey::Down if plain => self.move_selection(1),
            BareKey::Backspace if plain => {
                self.query.pop();
                self.after_query_change();
            },
            BareKey::Delete if plain => {
                self.query.clear();
                self.after_query_change();
            },
            BareKey::Char(c) if plain => {
                // with an empty query, 1-9 jump straight to the Nth match
                if self.query.is_empty() && ('1'..='9').contains(&c) {
                    let idx = c.to_digit(10).unwrap() as usize - 1;
                    if let Some(m) = self.matches().get(idx) {
                        let position = m.position;
                        go_to_tab(position as u32 + 1); // go_to_tab is 1-indexed
                        hide_self();
                    }
                } else {
                    self.query.push(c);
                    self.after_query_change();
                }
            },
            _ => {},
        }
    }

    fn after_query_change(&mut self) {
        self.selected = 0;
        self.first_visible = 0;
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.matches().len();
        if len == 0 {
            return;
        }
        let mut sel = self.selected as isize + delta;
        if sel < 0 {
            sel = len as isize - 1;
        }
        if sel >= len as isize {
            sel = 0;
        }
        self.selected = sel as usize;
    }

    fn jump_selected(&mut self) {
        let matches = self.matches();
        if let Some(m) = matches.get(self.selected) {
            let position = m.position;
            go_to_tab(position as u32 + 1); // go_to_tab is 1-indexed
        }
        hide_self();
    }

    /// Alt-tab style toggle: switch to the previously active tab.
    fn toggle_last_tab(&mut self) {
        if let Some(prev) = self
            .previous_tab
            .filter(|p| Some(*p) != self.current_tab)
        {
            go_to_tab(prev as u32 + 1); // go_to_tab is 1-indexed
        }
        hide_self();
    }
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

impl State {
    fn render_ui(&mut self) -> String {
        let mut out = String::new();

        // row 0: prompt + query
        out.push_str(&format!("{BOLD}{CYAN}❯{RESET} {}{DIM}▏{RESET}\n", self.query));
        // row 1: blank
        out.push('\n');

        let matches = self.matches();
        let avail = self.rows.saturating_sub(3); // prompt + blank + footer
        if avail > 0 {
            // keep the selection inside the scroll window
            if self.selected < self.first_visible {
                self.first_visible = self.selected;
            }
            if self.selected >= self.first_visible + avail {
                self.first_visible = self.selected + 1 - avail;
            }

            let shown: Vec<&Match> = matches
                .iter()
                .skip(self.first_visible)
                .take(avail)
                .collect();

            if matches.is_empty() {
                if self.tabs.is_empty() {
                    out.push_str(&format!("{DIM}waiting for tab info…{RESET}\n"));
                } else {
                    out.push_str(&format!(
                        "{RED}no matches for{RESET} {BOLD}{}{RESET}\n",
                        self.query
                    ));
                }
            }

            for (display_index, m) in shown.iter().enumerate() {
                let row = self.first_visible + display_index;
                out.push_str(&self.render_match(row, m));
                out.push('\n');
            }

            // pad so the footer sits on the last row
            let blanks = avail.saturating_sub(shown.len() + if matches.is_empty() { 1 } else { 0 });
            for _ in 0..blanks {
                out.push('\n');
            }
        }

        // footer
        out.push_str(&format!(
            "{DIM}1-9 quick-jump · ↑↓ select · ⏎ jump · tab last · esc close{RESET}"
        ));

        out
    }

    fn render_match(&self, display_index: usize, m: &Match) -> String {
        let selected = display_index == self.selected;
        let quick = if display_index < 9 {
            ((b'1' + display_index as u8) as char).to_string()
        } else {
            " ".to_string()
        };
        let pointer = if selected {
            format!("{BOLD}{GREEN}❯{RESET}")
        } else {
            " ".to_string()
        };

        let active_marker = if m.is_active { "● " } else { "" };
        let meta = format!("{}{}p", active_marker, m.pane_count);

        // 4 cols of prefix ("❯ 1 ") + gap + meta
        let name_budget = self.cols.saturating_sub(4 + meta.chars().count() + 1);

        let base = if selected {
            format!("{BOLD}{WHITE}")
        } else {
            String::new()
        };
        let mut name = base.clone();
        let mut name_len = 0usize;
        for (i, ch) in m.name.chars().enumerate() {
            if name_len + 1 > name_budget {
                name.push('…');
                name_len += 1;
                break;
            }
            if m.matched.contains(&i) {
                name.push_str(&format!("{BOLD}{MAGENTA}{ch}{RESET}"));
                name.push_str(&base);
            } else {
                name.push(ch);
            }
            name_len += 1;
        }

        let pad = name_budget.saturating_sub(name_len) + 1;
        format!(
            "{pointer} {DIM}{quick}{RESET} {name}{}{DIM}{meta}{RESET}",
            " ".repeat(pad)
        )
    }
}
