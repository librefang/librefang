//! Color palette matching the LibreFang landing page design system.
//!
//! Core palette from globals.css + code syntax from constants.ts.
//! Tuned for modern dark-mode TUI aesthetics with good contrast.

#![allow(dead_code)] // Full palette — some colors reserved for future screens.

use ratatui::style::{Color, Modifier, Style};

// ── Core Palette (dark mode — matches docs site zinc + emerald design) ───────

pub const ACCENT: Color = Color::Rgb(52, 211, 153); // #34D399 — emerald-400 (site primary)
pub const ACCENT_DIM: Color = Color::Rgb(16, 185, 129); // #10B981 — emerald-500

pub const BG_PRIMARY: Color = Color::Rgb(24, 24, 27); // #18181B — zinc-900
pub const BG_CARD: Color = Color::Rgb(39, 39, 42); // #27272A — zinc-800
pub const BG_HOVER: Color = Color::Rgb(52, 52, 56); // #343438 — zinc-700/800
pub const BG_CODE: Color = Color::Rgb(30, 30, 33); // #1E1E21 — code block

pub const TEXT_PRIMARY: Color = Color::Rgb(244, 244, 245); // #F4F4F5 — zinc-100
pub const TEXT_SECONDARY: Color = Color::Rgb(161, 161, 170); // #A1A1AA — zinc-400
pub const TEXT_TERTIARY: Color = Color::Rgb(113, 113, 122); // #71717A — zinc-500

pub const BORDER: Color = Color::Rgb(63, 63, 70); // #3F3F46 — zinc-700

// ── Semantic Colors ─────────────────────────────────────────────────────────

pub const GREEN: Color = Color::Rgb(52, 211, 153); // #34D399 — emerald-400
pub const BLUE: Color = Color::Rgb(96, 165, 250); // #60A5FA — blue-400
pub const YELLOW: Color = Color::Rgb(250, 204, 21); // #FACC15 — yellow-400
pub const RED: Color = Color::Rgb(248, 113, 113); // #F87171 — red-400
pub const PURPLE: Color = Color::Rgb(192, 132, 252); // #C084FC — purple-400

// ── Backward-compat aliases ─────────────────────────────────────────────────

pub const CYAN: Color = BLUE;
pub const DIM: Color = TEXT_SECONDARY;
pub const TEXT: Color = TEXT_PRIMARY;

// ── Reusable styles ─────────────────────────────────────────────────────────

pub fn title_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default()
        .fg(TEXT_PRIMARY)
        .bg(BG_HOVER)
        .add_modifier(Modifier::BOLD)
}

pub fn dim_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

pub fn input_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn hint_style() -> Style {
    Style::default().fg(TEXT_TERTIARY)
}

// ── Tab bar styles ──────────────────────────────────────────────────────────

pub fn tab_active() -> Style {
    Style::default()
        .fg(BG_PRIMARY)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn tab_separator() -> Style {
    Style::default().fg(BORDER)
}

pub fn tab_inactive() -> Style {
    Style::default().fg(TEXT_TERTIARY)
}

// ── State badge styles ──────────────────────────────────────────────────────

pub fn badge_running() -> Style {
    Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
}

pub fn badge_created() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn badge_suspended() -> Style {
    Style::default().fg(YELLOW).add_modifier(Modifier::BOLD)
}

pub fn badge_terminated() -> Style {
    Style::default().fg(TEXT_TERTIARY)
}

pub fn badge_crashed() -> Style {
    Style::default().fg(RED).add_modifier(Modifier::BOLD)
}

/// Return badge text + style for an agent state string.
pub fn state_badge(state: &str) -> (&'static str, Style) {
    let lower = state.to_lowercase();
    if lower.contains("run") {
        ("\u{25cf} RUN", badge_running())
    } else if lower.contains("creat") || lower.contains("new") || lower.contains("idle") {
        ("\u{25cb} NEW", badge_created())
    } else if lower.contains("sus") || lower.contains("paus") {
        ("\u{25d4} SUS", badge_suspended())
    } else if lower.contains("term") || lower.contains("stop") || lower.contains("end") {
        ("\u{25cb} END", badge_terminated())
    } else if lower.contains("err") || lower.contains("crash") || lower.contains("fail") {
        ("\u{25cf} ERR", badge_crashed())
    } else {
        ("\u{25cb} ---", dim_style())
    }
}

// ── Table / channel styles ──────────────────────────────────────────────────

pub fn table_header() -> Style {
    Style::default()
        .fg(TEXT_SECONDARY)
        .add_modifier(Modifier::BOLD)
}

pub fn channel_ready() -> Style {
    Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
}

pub fn channel_missing() -> Style {
    Style::default().fg(YELLOW)
}

pub fn channel_off() -> Style {
    dim_style()
}

// ── Spinner ─────────────────────────────────────────────────────────────────

pub const SPINNER_FRAMES: &[&str] = &[
    "\u{25dc}", "\u{25dd}", "\u{25de}", "\u{25df}", // ◜ ◝ ◞ ◟ rotating arc
];

// ── Tests ───────────────────────────────────────────────────────────────────
//
// `state_badge` is the single funnel every TUI screen uses to map an agent's
// free-form state string into a (badge_text, style) pair. The classifier is a
// substring scan with **fixed branch order**, so priority matters: "Created"
// must hit NEW before it could ever be misread, and unknown / empty inputs
// must fall through to the dim placeholder rather than panic. Regression net
// for issue #3582.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_state_matches_run_branch() {
        let (text, _) = state_badge("Running");
        assert_eq!(text, "\u{25cf} RUN");
    }

    #[test]
    fn running_lowercase_also_matches() {
        // Classifier lowercases input, so casing must not matter.
        let (text, _) = state_badge("RUNNING");
        assert_eq!(text, "\u{25cf} RUN");
    }

    #[test]
    fn created_idle_and_new_all_map_to_new_branch() {
        for s in ["Created", "Idle", "New", "idle", "CREATED"] {
            let (text, _) = state_badge(s);
            assert_eq!(text, "\u{25cb} NEW", "state {s:?} should map to NEW");
        }
    }

    #[test]
    fn suspended_and_paused_map_to_sus_branch() {
        for s in ["Suspended", "Paused", "pausing", "SUS"] {
            let (text, _) = state_badge(s);
            assert_eq!(text, "\u{25d4} SUS", "state {s:?} should map to SUS");
        }
    }

    #[test]
    fn terminated_stopped_ended_map_to_end_branch() {
        for s in ["Terminated", "Stopped", "Ended", "stop"] {
            let (text, _) = state_badge(s);
            assert_eq!(text, "\u{25cb} END", "state {s:?} should map to END");
        }
    }

    #[test]
    fn crashed_errored_failed_map_to_err_branch() {
        for s in ["Crashed", "Errored", "Failed", "error: boom"] {
            let (text, _) = state_badge(s);
            assert_eq!(text, "\u{25cf} ERR", "state {s:?} should map to ERR");
        }
    }

    #[test]
    fn unknown_state_falls_through_to_placeholder() {
        let (text, _) = state_badge("hibernating");
        assert_eq!(text, "\u{25cb} ---");
    }

    #[test]
    fn empty_state_falls_through_without_panic() {
        let (text, _) = state_badge("");
        assert_eq!(text, "\u{25cb} ---");
    }

    #[test]
    fn run_branch_wins_over_later_branches() {
        // "run" is the first checked substring — a state that *also* contains
        // a later keyword (contrived "running-but-failing") must still classify
        // as RUN, proving the branches are ordered, not OR'd.
        let (text, _) = state_badge("running-but-failing");
        assert_eq!(text, "\u{25cf} RUN");
    }

    #[test]
    fn new_branch_wins_over_term_branch() {
        // Order check: "creat" is matched before "term", so a hypothetical
        // "created-then-terminated" string lands on NEW, not END.
        let (text, _) = state_badge("created-then-terminated");
        assert_eq!(text, "\u{25cb} NEW");
    }

    #[test]
    fn badge_styles_match_dedicated_style_helpers() {
        // Smoke check that each branch returns its dedicated style helper —
        // catches accidental copy-paste regressions that swap, say, RUN's
        // green for ERR's red.
        let (_, run) = state_badge("running");
        let (_, new) = state_badge("created");
        let (_, sus) = state_badge("paused");
        let (_, end) = state_badge("terminated");
        let (_, err) = state_badge("crashed");
        let (_, unk) = state_badge("???");

        assert_eq!(run, badge_running());
        assert_eq!(new, badge_created());
        assert_eq!(sus, badge_suspended());
        assert_eq!(end, badge_terminated());
        assert_eq!(err, badge_crashed());
        assert_eq!(unk, dim_style());
    }
}
