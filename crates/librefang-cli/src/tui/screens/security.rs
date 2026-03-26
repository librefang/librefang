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
    fn label(self) -> &'static str {
        match self {
            Self::Core => "Core Security",
            Self::Configurable => "Configurable",
            Self::Monitoring => "Monitoring",
        }
    }
}

// ── Built-in feature definitions ────────────────────────────────────────────

fn builtin_features() -> Vec<SecurityFeature> {
    vec![
        // Core (8)
        SecurityFeature {
            name: "Path Traversal Prevention".into(),
            active: true,
            description: "safe_resolve_path blocks ../../ attacks".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "SSRF Protection".into(),
            active: true,
            description: "Blocks private IPs and metadata endpoints in HTTP fetches".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Subprocess Isolation".into(),
            active: true,
            description: "env_clear() + selective vars on child processes".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "WASM Dual Metering".into(),
            active: true,
            description: "Fuel + epoch interruption with watchdog thread".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Capability Inheritance".into(),
            active: true,
            description: "validate_capability_inheritance prevents privilege escalation".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Secret Zeroization".into(),
            active: true,
            description: "Zeroizing<String> auto-wipes API keys from memory".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Ed25519 Manifest Signing".into(),
            active: true,
            description: "Signed agent manifests with Ed25519 verification".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Taint Tracking".into(),
            active: true,
            description: "Information flow tracking across tool boundaries".into(),
            section: SecuritySection::Core,
        },
        // Configurable (4)
        SecurityFeature {
            name: "OFP Wire Auth".into(),
            active: true,
            description: "HMAC-SHA256 mutual authentication with nonce".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "RBAC Multi-User".into(),
            active: true,
            description: "Role-based access control with user hierarchy".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "Rate Limiting".into(),
            active: true,
            description: "GCRA rate limiter with cost-aware tokens".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "Security Headers".into(),
            active: true,
            description: "CSP, X-Frame-Options, HSTS middleware".into(),
            section: SecuritySection::Configurable,
        },
        // Monitoring (3)
        SecurityFeature {
            name: "Merkle Audit Trail".into(),
            active: true,
            description: "Hash chain audit log with tamper detection".into(),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: "Heartbeat Monitor".into(),
            active: true,
            description: "Background health checks with restart limits".into(),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: "Prompt Injection Scanner".into(),
            active: true,
            description: "Detects override attempts and data exfiltration".into(),
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
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
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
    let inner = widgets::render_screen_block(f, area, "\u{25c6} Security");

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
                    format!("  {active_count}/{total_count} features active"),
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  \u{2502}  Core \u{00b7} Configurable \u{00b7} Monitoring",
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
                format!("  {} {} ", section_icon(feat.section), feat.section.label()),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<30}", "Feature"), theme::table_header()),
                Span::styled(format!(" {:<12}", "Status"), theme::table_header()),
                Span::styled(" Description", theme::table_header()),
            ]));
            current_section = Some(feat.section);
        }

        let (badge, badge_style) = if feat.active {
            (
                "\u{25cf} Active",
                Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "\u{25cb} Inactive",
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
                    widgets::spinner(state.tick, "Verifying audit chain\u{2026}"),
                    chunks[2],
                );
            } else {
                f.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        "  \u{25cb} Press [v] to verify audit chain integrity",
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
                        "  \u{2714} Audit chain verified",
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
                        "  \u{2718} Audit chain verification failed",
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
        widgets::hint_bar("  [\u{2191}\u{2193}] Scroll  [v] Verify Chain  [r] Refresh"),
        chunks[3],
    );
}
