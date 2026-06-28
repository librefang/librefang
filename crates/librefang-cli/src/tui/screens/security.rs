//! Security screen: security feature dashboard and chain verification.

use crate::tui::theme;
use crate::tui::widgets;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SecurityFeature {
    pub name: String,
    pub active: bool,
    pub description: String,
    pub section: SecuritySection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SecuritySection {
    Core,
    Configurable,
    Monitoring,
}

impl SecuritySection {
    fn label_localized(self) -> String {
        match self {
            Self::Core => crate::i18n::t("tui-security-section-core"),
            Self::Configurable => crate::i18n::t("tui-security-section-configurable"),
            Self::Monitoring => crate::i18n::t("tui-security-section-monitoring"),
        }
    }
}

// ── Built-in feature definitions ────────────────────────────────────────────

fn builtin_features() -> Vec<SecurityFeature> {
    vec![
        // Core (8)
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-path-traversal-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-path-traversal-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-ssrf-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-ssrf-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-subprocess-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-subprocess-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-wasm-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-wasm-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-capability-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-capability-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-secret-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-secret-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-ed25519-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-ed25519-desc"),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-taint-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-taint-desc"),
            section: SecuritySection::Core,
        },
        // Configurable (4)
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-ofp-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-ofp-desc"),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-rbac-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-rbac-desc"),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-rate-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-rate-desc"),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-headers-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-headers-desc"),
            section: SecuritySection::Configurable,
        },
        // Monitoring (3)
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-merkle-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-merkle-desc"),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-heartbeat-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-heartbeat-desc"),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: crate::i18n::t("tui-security-feat-prompt-name"),
            active: true,
            description: crate::i18n::t("tui-security-feat-prompt-desc"),
            section: SecuritySection::Monitoring,
        },
    ]
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct SecurityState {
    pub features: Vec<SecurityFeature>,
    pub chain_verified: Option<bool>,
    pub verify_result: String,
    pub scroll: u16,
    pub loading: bool,
    pub tick: usize,
}

pub enum SecurityAction {
    Continue,
    Refresh,
    VerifyChain,
}

impl SecurityState {
    pub fn new() -> Self {
        Self {
            features: builtin_features(),
            chain_verified: None,
            verify_result: String::new(),
            scroll: 0,
            loading: false,
            tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SecurityAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SecurityAction::Continue;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('l') => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('h') => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
            }
            KeyCode::Char('v') => return SecurityAction::VerifyChain,
            KeyCode::Char('r') => return SecurityAction::Refresh,
            _ => {}
        }
        SecurityAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SecurityState) {
    let inner = widgets::render_screen_block(
        f,
        area,
        &format!("{} {}", "\u{25c6}", crate::i18n::t("tui-security-title")),
    );

    let chunks = Layout::vertical([
        Constraint::Length(2), // summary bar
        Constraint::Min(4),    // features
        Constraint::Length(2), // verify result
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Summary bar ──
    let active_count = state.features.iter().filter(|f| f.active).count();
    let total_count = state.features.len();
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    crate::i18n::t_args(
                        "tui-security-active-features",
                        &[
                            ("active", &active_count.to_string()),
                            ("total", &total_count.to_string()),
                        ],
                    ),
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    crate::i18n::t("tui-security-sections-sub"),
                    theme::dim_style(),
                ),
            ]),
            Line::from(vec![Span::styled(
                "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
                Style::default().fg(theme::BORDER),
            )]),
        ]),
        chunks[0],
    );

    // ── Features list ──
    let mut lines: Vec<Line> = Vec::new();
    let mut current_section: Option<SecuritySection> = None;

    let section_icon = |s: SecuritySection| -> &'static str {
        match s {
            SecuritySection::Core => "\u{25c9}",
            SecuritySection::Configurable => "\u{25ce}",
            SecuritySection::Monitoring => "\u{25c8}",
        }
    };

    for feat in &state.features {
        if current_section != Some(feat.section) {
            if current_section.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  {} {} ",
                    section_icon(feat.section),
                    feat.section.label_localized()
                ),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<30}", crate::i18n::t("tui-security-header-feature")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {:<12}", crate::i18n::t("tui-security-header-status")),
                    theme::table_header(),
                ),
                Span::styled(
                    format!(" {}", crate::i18n::t("tui-security-header-description")),
                    theme::table_header(),
                ),
            ]));
            current_section = Some(feat.section);
        }

        let (badge, badge_style) = if feat.active {
            (
                format!(
                    "{} {}",
                    "\u{25cf}",
                    crate::i18n::t("tui-security-status-active")
                ),
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                format!(
                    "{} {}",
                    "\u{25cb}",
                    crate::i18n::t("tui-security-status-inactive")
                ),
                Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
            )
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<30}", feat.name),
                Style::default().fg(theme::CYAN),
            ),
            Span::styled(format!(" {:<12}", badge), badge_style),
            Span::styled(format!(" {}", feat.description), theme::dim_style()),
        ]));
    }

    let total = lines.len() as u16;
    let visible = chunks[1].height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll.saturating_sub(state.scroll).min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), chunks[1]);

    // ── Verify result ──
    match state.chain_verified {
        None => {
            if state.loading {
                f.render_widget(
                    widgets::spinner(state.tick, &crate::i18n::t("tui-security-verifying")),
                    chunks[2],
                );
            } else {
                f.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(
                            "  {} {}",
                            "\u{25cb}",
                            crate::i18n::t("tui-security-verify-prompt")
                        ),
                        theme::dim_style(),
                    )])),
                    chunks[2],
                );
            }
        }
        Some(true) => {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![Span::styled(
                        format!(
                            "  {} {}",
                            "\u{2714}",
                            crate::i18n::t("tui-security-verify-success")
                        ),
                        Style::default()
                            .fg(theme::GREEN)
                            .add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(vec![Span::styled(
                        format!("  {}", state.verify_result),
                        theme::dim_style(),
                    )]),
                ]),
                chunks[2],
            );
        }
        Some(false) => {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![Span::styled(
                        format!(
                            "  {} {}",
                            "\u{2718}",
                            crate::i18n::t("tui-security-verify-failed")
                        ),
                        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(vec![Span::styled(
                        format!("  {}", state.verify_result),
                        Style::default().fg(theme::RED),
                    )]),
                ]),
                chunks[2],
            );
        }
    }

    // ── Hints ──
    f.render_widget(
        widgets::hint_bar(&crate::i18n::t("tui-security-hints")),
        chunks[3],
    );
}
