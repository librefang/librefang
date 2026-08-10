//! Emoji translation table for inbound `Reaction` commands.
//!
//! Mirrors the Python adapter's `_REACTION_MAP`: a small allowlist of incoming reactions translated to Telegram-supported emoji.
//! Falls back to the raw emoji when not in the table — Telegram silently drops unknown reactions, which is acceptable: better to send something the user typed and have Telegram refuse it than to refuse client-side and lose the signal entirely.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneReactionPolicy {
    Emit,
    Suppress,
}

pub fn map_reaction(input: &str, done_policy: DoneReactionPolicy) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let normalized = trimmed.trim_end_matches(['\u{FE0F}', '\u{FE0E}']);
    match normalized {
        "⏳" => vec!["👀".into()],
        "⚙" => vec!["⚡".into()],
        "✅" => {
            if done_policy == DoneReactionPolicy::Suppress {
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
            assert_eq!(
                map_reaction(&format!("⏳{selector}"), DoneReactionPolicy::Emit),
                ["👀"]
            );
            assert_eq!(
                map_reaction(&format!("⚙{selector}"), DoneReactionPolicy::Emit),
                ["⚡"]
            );
            assert_eq!(
                map_reaction(&format!("✅{selector}"), DoneReactionPolicy::Emit),
                ["🎉"]
            );
            assert_eq!(
                map_reaction(&format!("✅{selector}"), DoneReactionPolicy::Suppress),
                Vec::<String>::new()
            );
            assert_eq!(
                map_reaction(&format!("❌{selector}"), DoneReactionPolicy::Emit),
                ["👎"]
            );
        }

        assert_eq!(map_reaction(" ♥️ ", DoneReactionPolicy::Emit), ["♥️"]);
        assert_eq!(map_reaction("✍️", DoneReactionPolicy::Emit), ["✍️"]);
    }
}
