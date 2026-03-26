//! Color palette matching the LibreFang landing page design system.
//!
//! Core palette from globals.css + code syntax from constants.ts.
//! Tuned for modern dark-mode TUI aesthetics with good contrast.

#![allow(dead_code)] // Full palette — some colors reserved for future screens.

use ratatui::style::{Color, Modifier, Style};

// ── Core Palette (dark mode for terminal) ───────────────────────────────────

pub const ACCENT: Color = Color::Rgb(255, 106, 20); // #FF6A14 — LibreFang orange (warmer)
pub const ACCENT_DIM: Color = Color::Rgb(204, 85, 16); // #CC5510

pub const BG_PRIMARY: Color = Color::Rgb(18, 18, 22); // #121216 — dark background (cooler)
pub const BG_CARD: Color = Color::Rgb(28, 28, 34); // #1C1C22 — dark surface
pub const BG_HOVER: Color = Color::Rgb(40, 40, 50); // #282832 — selection bg (more visible)
pub const BG_CODE: Color = Color::Rgb(22, 22, 28); // #16161C — dark code block

pub const TEXT_PRIMARY: Color = Color::Rgb(230, 230, 235); // #E6E6EB — light text
pub const TEXT_SECONDARY: Color = Color::Rgb(148, 148, 160); // #9494A0 — muted text (cooler)
pub const TEXT_TERTIARY: Color = Color::Rgb(100, 100, 115); // #646473 — dim text

pub const BORDER: Color = Color::Rgb(55, 55, 68); // #373744 — border (subtle blue tint)

// ── Semantic Colors (brighter variants for dark background contrast) ────────

pub const GREEN: Color = Color::Rgb(52, 211, 120); // #34D378 — success (brighter)
pub const BLUE: Color = Color::Rgb(96, 165, 250); // #60A5FA — info (brighter)
pub const YELLOW: Color = Color::Rgb(250, 200, 50); // #FAC832 — warning (more visible)
pub const RED: Color = Color::Rgb(248, 85, 85); // #F85555 — error (brighter)
pub const PURPLE: Color = Color::Rgb(180, 120, 255); // #B478FF — decorators (brighter)

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
