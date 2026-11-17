//! jump-to-tab — a zellij plugin: popup fuzzy finder over tabs.
//!
//! Open it with a keybinding (see README.md), type part of a tab name,
//! hit Enter to jump. `1`-`9` jump straight to a visible match.

mod fuzzy;

use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;
use zellij_tile::prelude::*;

/// Display width of a string in terminal columns. Emoji and CJK chars are
/// 2 columns wide — counting chars instead makes padded rows overflow and
/// wrap (e.g. the pane count landing on the next line).
fn display_width(s: &str) -> usize {
    s.width()
}

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
    /// zellij 0.46+ attention state (inbox/"program status" system)
    attention: TabAttention,
    /// pending or flashing bell (counts as NeedsYou, like the tab bar)
    bell: bool,
}

/// What a tab's attention marker looks like — mirrors zellij 0.46's own
/// tab bar (`TabMarker::for_tab` in default-plugins/bars): muted tabs show
/// a dim ⊘ regardless of status, "counted" statuses (needs-you / failed /
/// done / working) show their symbol, a bell degrades to needs-you, and
/// Quiet/NewOutput show nothing (the tab bar skips them too).
enum AttentionKind {
    Muted,
    Status(AttentionStatus),
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
    /// preselect "next tab" as soon as the current tab is known; the first
    /// TabUpdate may arrive before or after Visible(true), and with
    /// close_self() every open is a fresh instance where neither ran yet
    presel_pending: bool,
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
        // fresh instance: preselect the next tab once tab info arrives
        self.presel_pending = true;
        self.sync_cursor();
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::TabUpdate(tabs) => {
                self.update_tabs(tabs);
                true
            }
            Event::Key(key) => {
                self.handle_key(key);
                self.sync_cursor();
                true
            }
            Event::Visible(visible) => {
                if visible {
                    self.reset();
                    self.sync_cursor();
                }
                visible
            }
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
        // a fresh open may still be waiting for tab info to preselect
        self.try_presel();
    }

    /// Put the terminal cursor right after the query text ("❯ " + query).
    /// Only ever called from `load`/`update` — see the note in `render`.
    fn sync_cursor(&self) {
        show_cursor(Some((2 + display_width(&self.query), 0)));
    }

    /// Fresh session every time the popup opens.
    fn reset(&mut self) {
        self.query.clear();
        self.first_visible = 0;
        self.presel_pending = true;
        self.try_presel();
    }

    /// Preselect the tab after the current one, so Enter with an empty query
    /// acts as "next tab"; wraps to the first tab. Deferred until the current
    /// tab is known (TabUpdate may arrive after the popup opens) and
    /// cancelled by any keypress (the user took over).
    fn try_presel(&mut self) {
        if !self.presel_pending || self.query.chars().count() > 0 {
            return;
        }
        if let Some(current) = self.current_tab {
            self.selected = self
                .matches()
                .iter()
                .position(|m| m.position > current)
                .unwrap_or(0);
            self.presel_pending = false;
        }
    }

    fn matches(&self) -> Vec<Match> {
        let to_match = |t: &TabInfo, matched: Vec<usize>| Match {
            position: t.position,
            name: t.name.clone(),
            pane_count: t.selectable_tiled_panes_count + t.selectable_floating_panes_count,
            is_active: t.active,
            matched,
            attention: t.attention.clone(),
            bell: t.has_bell_notification || t.is_flashing_bell,
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
        // any keypress means the user took over — stop auto-preselecting
        self.presel_pending = false;

        // only plain (optionally shifted) keys drive the finder
        let plain = key
            .key_modifiers
            .iter()
            .all(|m| matches!(m, KeyModifier::Shift));
        // exactly Ctrl, no other modifiers
        let ctrl = key.key_modifiers.contains(&KeyModifier::Ctrl)
            && key
                .key_modifiers
                .iter()
                .all(|m| matches!(m, KeyModifier::Ctrl));
        // exactly Alt, optionally with Shift
        let alt = key.key_modifiers.contains(&KeyModifier::Alt)
            && key
                .key_modifiers
                .iter()
                .all(|m| matches!(m, KeyModifier::Alt | KeyModifier::Shift));
        let alt_shift = alt && key.key_modifiers.contains(&KeyModifier::Shift);

        match key.bare_key {
            BareKey::Enter if plain => self.jump_selected(),
            // close_self (not hide_self): on zellij 0.45, hiding leaves focus
            // on the now-invisible pane — the session stops responding to
            // keys. Closing destroys the pane and refocuses properly.
            BareKey::Esc if plain => close_self(),
            BareKey::Tab if plain => self.toggle_last_tab(),
            // alt-tab / alt-shift-tab move down/up
            BareKey::Tab if alt && !alt_shift => self.move_selection(1),
            BareKey::Tab if alt_shift => self.move_selection(-1),
            BareKey::Up if plain => self.move_selection(-1),
            BareKey::Down if plain => self.move_selection(1),
            // readline-style: ctrl-n/ctrl-p move down/up
            BareKey::Char('n') if ctrl => self.move_selection(1),
            BareKey::Char('p') if ctrl => self.move_selection(-1),
            BareKey::Backspace if plain => {
                self.query.pop();
                self.after_query_change();
            }
            BareKey::Delete if plain => {
                self.query.clear();
                self.after_query_change();
            }
            BareKey::Char(c) if plain => {
                // with an empty query, 1-9 jump straight to the Nth match
                if self.query.is_empty() && ('1'..='9').contains(&c) {
                    let idx = c.to_digit(10).unwrap() as usize - 1;
                    if let Some(m) = self.matches().get(idx) {
                        let position = m.position;
                        self.jump_to(position);
                    }
                } else {
                    self.query.push(c);
                    self.after_query_change();
                }
            }
            _ => {}
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
            self.jump_to(m.position);
        } else {
            close_self();
        }
    }

    /// Alt-tab style toggle: switch to the previously active tab.
    fn toggle_last_tab(&mut self) {
        if let Some(prev) = self.previous_tab.filter(|p| Some(*p) != self.current_tab) {
            self.jump_to(prev);
        } else {
            close_self();
        }
    }

    /// Switch tabs and close the finder with a fresh state for next time.
    ///
    /// `go_to_tab` from a plugin takes the 0-indexed `TabInfo.position`:
    /// zellij's host side adds 1 to build `Action::GoToTab` and the screen
    /// subtracts it back (`zellij_exports.rs`: `index: tab_index + 1`,
    /// `screen.rs`: `switch_active_tab(index - 1)`) — so adding our own +1
    /// lands one tab to the right.
    fn jump_to(&mut self, position: usize) {
        go_to_tab(position as u32);
        // keep current/previous accurate before the TabUpdate arrives, so
        // the preselect on next open is relative to the tab we just entered
        self.previous_tab = self.current_tab.replace(position);
        // close, not hide: hiding on zellij 0.45 strands focus on the
        // invisible pane (dead input); closing refocuses. The plugin unloads
        // and reopens fresh (Default state = empty query), so no reset needed.
        close_self();
        self.reset();
    }
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

impl State {
    fn attention_kind(&self, m: &Match) -> Option<AttentionKind> {
        if m.attention.muted {
            return Some(AttentionKind::Muted);
        }
        if m.attention.status.is_counted() {
            Some(AttentionKind::Status(m.attention.status))
        } else if m.bell {
            // a bell without a counted status still needs you
            Some(AttentionKind::Status(AttentionStatus::NeedsYou))
        } else {
            None
        }
    }

    /// Styled marker for the 2-column attention slot; `None` when the tab
    /// has no marker. Colors follow zellij's urgency model: needs-you and
    /// failed are urgent (bold), done/working are calm, muted is dim.
    fn attention_marker(&self, m: &Match) -> Option<String> {
        match self.attention_kind(m)? {
            AttentionKind::Muted => Some(format!("{DIM}⊘{RESET} ")),
            AttentionKind::Status(status) => {
                let count = m.attention.counts.of(status);
                // symbol alone, or symbol + count when several panes share
                // the status — padded to exactly 2 columns
                let plain = if count > 9 {
                    format!("{}+", status.marker(false))
                } else if count > 1 {
                    format!("{}{}", status.marker(false), count)
                } else {
                    format!("{}", status.marker(false))
                };
                let pad = " ".repeat(2usize.saturating_sub(display_width(&plain)));
                let styled = match status {
                    AttentionStatus::NeedsYou | AttentionStatus::Failed => {
                        format!("{BOLD}{RED}{plain}{RESET}")
                    }
                    AttentionStatus::Done => format!("{GREEN}{plain}{RESET}"),
                    AttentionStatus::Working => format!("{CYAN}{plain}{RESET}"),
                    _ => format!("{DIM}{plain}{RESET}"),
                };
                Some(format!("{styled}{pad}"))
            }
        }
    }

    fn render_ui(&mut self) -> String {
        let mut out = String::new();

        // row 0: prompt + query
        out.push_str(&format!(
            "{BOLD}{CYAN}❯{RESET} {}{DIM}▏{RESET}\n",
            self.query
        ));
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

            let show_marker_col = matches.iter().any(|m| self.attention_kind(m).is_some());

            for (display_index, m) in shown.iter().enumerate() {
                let row = self.first_visible + display_index;
                out.push_str(&self.render_match(row, m, show_marker_col));
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
            "{DIM}1-9 quick-jump · ↑↓/⌃n⌃p select · ⏎ jump · tab last · esc close{RESET}"
        ));

        out
    }

    fn render_match(&self, display_index: usize, m: &Match, show_marker_col: bool) -> String {
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

        // 4 cols of prefix ("❯ 1 ") + attention slot + gap + meta — all in
        // display columns. The attention slot only exists when some tab in
        // the list has a marker, so attention-free sessions keep the old,
        // wider name budget.
        let marker_width = if show_marker_col { 2 } else { 0 };
        let name_budget = self
            .cols
            .saturating_sub(4 + marker_width + display_width(&meta) + 1);

        let base = if selected {
            format!("{BOLD}{WHITE}")
        } else {
            String::new()
        };
        let mut name = base.clone();
        let mut name_len = 0usize;
        for (i, ch, ch_width) in self.name_char_widths(&m.name) {
            if name_len + ch_width > name_budget {
                // truncate — but only add the ellipsis if it actually fits,
                // otherwise the row overflows by one column and wraps
                if name_len + 1 <= name_budget {
                    name.push('…');
                    name_len += 1;
                }
                break;
            }
            if m.matched.contains(&i) {
                name.push_str(&format!("{BOLD}{MAGENTA}{ch}{RESET}"));
                name.push_str(&base);
            } else {
                name.push(ch);
            }
            name_len += ch_width;
        }

        let pad = name_budget.saturating_sub(name_len) + 1;
        let marker = if show_marker_col {
            self.attention_marker(m).unwrap_or_else(|| "  ".to_string())
        } else {
            String::new()
        };
        format!(
            "{pointer} {DIM}{quick}{RESET} {name}{}{marker}{DIM}{meta}{RESET}",
            " ".repeat(pad)
        )
    }

    /// Per-char display widths of a tab name, in the same order as
    /// `chars()` (so indices line up with the fuzzy-match highlights).
    ///
    /// Measured as prefix-width differences instead of per-char
    /// `width()` so emoji presentation sequences ("▶️" = U+25B6 +
    /// U+FE0F) report their true 2-column width: the crate only folds
    /// those in when measuring a string. ZWJ emoji families can
    /// over-count here, which only widens the gap — never wraps.
    fn name_char_widths(&self, name: &str) -> Vec<(usize, char, usize)> {
        let chars: Vec<char> = name.chars().collect();
        let mut widths = Vec::with_capacity(chars.len());
        let mut prev = 0usize;
        for (i, &ch) in chars.iter().enumerate() {
            let upto: String = chars[..=i].iter().collect();
            let w = upto.width();
            widths.push((i, ch, w.saturating_sub(prev)));
            prev = w;
        }
        widths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // skip until the final 'm' of an SGR sequence
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn make_match(name: &str, pane_count: usize, is_active: bool) -> Match {
        Match {
            position: 0,
            name: name.into(),
            pane_count,
            is_active,
            matched: vec![],
            attention: TabAttention::default(),
            bell: false,
        }
    }

    fn make_attention_match(status: AttentionStatus, counts: AttentionCounts, muted: bool) -> Match {
        Match {
            attention: TabAttention {
                status,
                counts,
                muted,
            },
            ..make_match("server", 2, false)
        }
    }

    #[test]
    fn display_width_basics() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("\u{1f980}"), 2); // crab is 2 cols
        assert_eq!(display_width("a\u{1f980}b"), 4);
        // emoji presentation sequence: 2 cols as a string
        assert_eq!(display_width("\u{25b6}\u{fe0f}"), 2);
    }

    #[test]
    fn row_with_emoji_fits_within_cols() {
        for cols in [40usize, 60, 80] {
            let mut st = State::default();
            st.cols = cols;
            let m = make_match("\u{1f980} server with an emoji title", 3, true);
            let row = strip_ansi(&st.render_match(0, &m, false));
            assert!(
                display_width(row.trim_end()) <= cols,
                "cols={cols}: row is {} cols wide: {:?}",
                display_width(row.trim_end()),
                row
            );
        }
    }

    #[test]
    fn truncated_emoji_row_fits_within_cols() {
        for cols in [20usize, 30, 50] {
            let mut st = State::default();
            st.cols = cols;
            let m = make_match(
                "\u{1f980}\u{1f980}\u{1f980}\u{1f980}\u{1f980}\u{1f980}\u{1f980}\u{1f980} very long emoji name",
                12,
                false,
            );
            let row = strip_ansi(&st.render_match(0, &m, false));
            assert!(
                display_width(row.trim_end()) <= cols,
                "cols={cols}: row is {} cols wide: {:?}",
                display_width(row.trim_end()),
                row
            );
        }
    }

    #[test]
    fn emoji_row_keeps_meta_on_same_line() {
        // the reported bug: pane count wrapped to the next line
        let mut st = State::default();
        st.cols = 40;
        let m = make_match("\u{1f980} tab", 3, false);
        let row = strip_ansi(&st.render_match(0, &m, false));
        assert!(row.contains("3p"), "meta missing from row: {:?}", row);
        assert!(!row.contains('\n'));
    }

    #[test]
    fn attention_markers_mirror_the_tab_bar() {
        let mut st = State::default();
        st.cols = 60;
        let cases = [
            (AttentionStatus::NeedsYou, "●"),
            (AttentionStatus::Failed, "✗"),
            (AttentionStatus::Done, "✓"),
            (AttentionStatus::Working, "◐"),
        ];
        for (status, symbol) in cases {
            let m = make_attention_match(status, AttentionCounts::default(), false);
            let row = strip_ansi(&st.render_match(0, &m, true));
            assert!(
                row.contains(symbol),
                "{:?} row missing {}: {:?}",
                status,
                symbol,
                row
            );
        }
    }

    #[test]
    fn quiet_new_output_and_idle_have_no_marker() {
        let mut st = State::default();
        st.cols = 60;
        for status in [
            AttentionStatus::Idle,
            AttentionStatus::Quiet,
            AttentionStatus::NewOutput,
        ] {
            let m = make_attention_match(status, AttentionCounts::default(), false);
            let row = strip_ansi(&st.render_match(0, &m, true));
            // a blank 2-col slot, not a symbol
            assert!(
                !row.contains('✓') && !row.contains('✗') && !row.contains('●'),
                "{:?} row: {:?}",
                status,
                row
            );
        }
    }

    #[test]
    fn muted_tab_shows_dim_zero_slash() {
        let mut st = State::default();
        st.cols = 60;
        // muted wins even with an active status — same as the native tab bar
        let m = make_attention_match(
            AttentionStatus::NeedsYou,
            AttentionCounts::default(),
            true,
        );
        let row = strip_ansi(&st.render_match(0, &m, true));
        assert!(row.contains('⊘'), "muted marker missing: {:?}", row);
        assert!(!row.contains('●'), "status leaked through mute: {:?}", row);
    }

    #[test]
    fn bell_without_status_shows_needs_you() {
        let mut st = State::default();
        st.cols = 60;
        let m = Match {
            bell: true,
            ..make_attention_match(AttentionStatus::Idle, AttentionCounts::default(), false)
        };
        let row = strip_ansi(&st.render_match(0, &m, true));
        assert!(row.contains('●'), "bell marker missing: {:?}", row);
    }

    #[test]
    fn multiple_panes_of_one_status_show_a_count() {
        let mut st = State::default();
        st.cols = 60;
        let m = make_attention_match(
            AttentionStatus::Failed,
            AttentionCounts {
                failed: 3,
                ..Default::default()
            },
            false,
        );
        let row = strip_ansi(&st.render_match(0, &m, true));
        assert!(row.contains("✗3"), "count missing: {:?}", row);
    }

    #[test]
    fn marker_row_fits_within_cols() {
        for cols in [30usize, 40, 60] {
            let mut st = State::default();
            st.cols = cols;
            let m = make_attention_match(
                AttentionStatus::NeedsYou,
                AttentionCounts {
                    needs_you: 12,
                    ..Default::default()
                },
                false,
            );
            let row = strip_ansi(&st.render_match(0, &m, true));
            assert!(
                row.contains("●+"),
                "cols={cols}: count marker missing: {:?}",
                row
            );
            assert!(
                display_width(row.trim_end()) <= cols,
                "cols={cols}: row is {} cols wide: {:?}",
                display_width(row.trim_end()),
                row
            );
        }
    }

    #[test]
    fn marker_slot_is_two_columns_so_rows_align() {
        // regression: the marker used to lose its padding space, shifting
        // the meta column left by one on marker rows
        let mut st = State::default();
        st.cols = 60;
        let with_marker = strip_ansi(&st.render_match(
            1,
            &make_attention_match(
                AttentionStatus::NeedsYou,
                AttentionCounts::default(),
                false,
            ),
            true,
        ));
        let without = strip_ansi(&st.render_match(0, &make_match("server", 2, false), true));
        assert!(
            with_marker.contains("● 2p"),
            "marker not 2 cols wide: {:?}",
            with_marker
        );
        // meta column starts at the same offset in both rows
        let meta_at = |row: &str| display_width(&row[..row.find('p').unwrap()]);
        assert_eq!(
            meta_at(&with_marker),
            meta_at(&without),
            "meta misaligned:\n  {:?}\n  {:?}",
            with_marker,
            without
        );
    }
}

