//! Emoji translation table for inbound `Reaction` commands.
//!
//! Mirrors the Python adapter's `_REACTION_MAP`: a small allowlist of incoming reactions translated to Telegram-supported emoji.
//! Falls back to the raw emoji when not in the table — Telegram silently drops unknown reactions, which is acceptable: better to send something the user typed and have Telegram refuse it than to refuse client-side and lose the signal entirely.

pub fn map_reaction(input: &str, clear_done: bool) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let normalized = trimmed.trim_end_matches(['\u{FE0F}', '\u{FE0E}']);
    match normalized {
        "⏳" => vec!["👀".into()],
        "⚙" => vec!["⚡".into()],
        "✅" => {
            if clear_done {
                Vec::new()
            } else {
                vec!["🎉".into()]
            }
        }
        "❌" => vec!["👎".into()],
        _ => vec![trimmed.into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variation_selectors_do_not_bypass_reaction_mappings() {
        for selector in ['\u{FE0F}', '\u{FE0E}'] {
            assert_eq!(map_reaction(&format!("⏳{selector}"), false), ["👀"]);
            assert_eq!(map_reaction(&format!("⚙{selector}"), false), ["⚡"]);
            assert_eq!(map_reaction(&format!("✅{selector}"), false), ["🎉"]);
            assert_eq!(
                map_reaction(&format!("✅{selector}"), true),
                Vec::<String>::new()
            );
            assert_eq!(map_reaction(&format!("❌{selector}"), false), ["👎"]);
        }

        assert_eq!(map_reaction(" ♥️ ", false), ["♥️"]);
        assert_eq!(map_reaction("✍️", false), ["✍️"]);
    }
}
