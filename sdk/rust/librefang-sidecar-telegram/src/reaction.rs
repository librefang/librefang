//! Emoji translation table for inbound `Reaction` commands.
//!
//! Mirrors the Python adapter's `_REACTION_MAP`: a small allowlist of incoming reactions translated to Telegram-supported emoji.
//! Falls back to the raw emoji when not in the table — Telegram silently drops unknown reactions, which is acceptable: better to send something the user typed and have Telegram refuse it than to refuse client-side and lose the signal entirely.

pub fn map_reaction(input: &str, clear_done: bool) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    match trimmed {
        "⏳" => vec!["👀".into()],
        "⚙️" | "⚙" => vec!["⚡".into()],
        "✅" => {
            if clear_done {
                Vec::new()
            } else {
                vec!["🎉".into()]
            }
        }
        "❌" => vec!["👎".into()],
        other => vec![other.into()],
    }
}
