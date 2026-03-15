//! Topic isolation for conversation history.
//!
//! Detects topic shifts in message history and returns only the messages
//! belonging to the current topic. This reduces token usage and improves
//! response quality by avoiding sending irrelevant context to the LLM.

use librefang_types::config::TopicIsolationConfig;
use librefang_types::message::{Message, MessageContent, Role};
use std::collections::HashSet;
use tracing::debug;

/// Find the index of the last topic boundary in the message list.
///
/// Returns `None` if no topic shift is detected (all messages belong to
/// one topic). Otherwise returns the index of the first message in the
/// current topic.
fn find_topic_boundary(messages: &[Message], config: &TopicIsolationConfig) -> Option<usize> {
    // Collect indices of user messages (these are the ones we analyze for topic shifts).
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User)
        .map(|(i, _)| i)
        .collect();

    if user_indices.len() < 2 {
        return None;
    }

    // Walk user messages backwards looking for topic shift.
    // Compare each consecutive pair of user messages.
    let mut latest_boundary: Option<usize> = None;

    for window in user_indices.windows(2).rev() {
        let prev_idx = window[0];
        let curr_idx = window[1];

        let prev_text = extract_text(&messages[prev_idx]);
        let curr_text = extract_text(&messages[curr_idx]);

        if is_topic_shift(&prev_text, &curr_text, config) {
            latest_boundary = Some(curr_idx);
            break;
        }
    }

    latest_boundary
}

/// Detect whether the transition from `prev` to `curr` represents a topic shift.
fn is_topic_shift(prev: &str, curr: &str, config: &TopicIsolationConfig) -> bool {
    let curr_lower = curr.to_lowercase();

    // Check explicit topic-change phrases.
    for phrase in &config.topic_change_phrases {
        if curr_lower.contains(&phrase.to_lowercase()) {
            return true;
        }
    }

    // Check word overlap similarity — low overlap means different topic.
    let similarity = word_overlap_ratio(prev, curr);
    if similarity < config.similarity_threshold
        && !curr.trim().is_empty()
        && !prev.trim().is_empty()
    {
        // Additional guard: very short messages (< 5 words) are likely greetings
        // or follow-ups, not topic shifts.
        let curr_word_count = curr.split_whitespace().count();
        let prev_word_count = prev.split_whitespace().count();
        if curr_word_count >= 5 && prev_word_count >= 5 {
            return true;
        }
    }

    false
}

/// Compute a simple word overlap ratio between two texts.
///
/// Returns a value in [0.0, 1.0] — the Jaccard similarity of the word sets,
/// ignoring common stop words.
fn word_overlap_ratio(a: &str, b: &str) -> f64 {
    let words_a = significant_words(a);
    let words_b = significant_words(b);

    if words_a.is_empty() || words_b.is_empty() {
        return 0.0;
    }

    let intersection = words_a.intersection(&words_b).count();
    let union = words_a.union(&words_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Extract significant (non-stop) words from text, lowercased.
fn significant_words(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|w| w.len() > 2 && !is_stop_word(w))
        .collect()
}

/// Check if a word is a common English stop word.
fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "and"
            | "but"
            | "for"
            | "not"
            | "you"
            | "all"
            | "can"
            | "had"
            | "her"
            | "was"
            | "one"
            | "our"
            | "out"
            | "are"
            | "has"
            | "his"
            | "how"
            | "its"
            | "may"
            | "new"
            | "now"
            | "old"
            | "see"
            | "way"
            | "who"
            | "did"
            | "get"
            | "let"
            | "say"
            | "she"
            | "too"
            | "use"
            | "this"
            | "that"
            | "with"
            | "have"
            | "from"
            | "they"
            | "been"
            | "will"
            | "what"
            | "when"
            | "make"
            | "like"
            | "just"
            | "over"
            | "such"
            | "take"
            | "than"
            | "them"
            | "very"
            | "some"
            | "could"
            | "would"
            | "about"
            | "which"
            | "their"
            | "there"
            | "these"
            | "other"
            | "into"
            | "more"
    )
}

/// Extract text content from a message for comparison.
fn extract_text(msg: &Message) -> String {
    match &msg.content {
        MessageContent::Text(s) => s.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                librefang_types::message::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Apply topic isolation to a message list.
///
/// If topic isolation is enabled and a topic shift is detected, returns only
/// the messages from the current topic (up to `max_topic_messages`).
/// Otherwise returns the original messages unchanged.
pub fn apply_topic_isolation(
    messages: Vec<Message>,
    config: &TopicIsolationConfig,
) -> Vec<Message> {
    if !config.enabled || messages.len() <= config.max_topic_messages {
        return messages;
    }

    if let Some(boundary) = find_topic_boundary(&messages, config) {
        let topic_messages = &messages[boundary..];
        debug!(
            original_count = messages.len(),
            topic_start = boundary,
            topic_count = topic_messages.len(),
            "Topic isolation: trimmed history to current topic"
        );

        // Cap to max_topic_messages from the end if the current topic is still large.
        if topic_messages.len() > config.max_topic_messages {
            let start = topic_messages.len() - config.max_topic_messages;
            topic_messages[start..].to_vec()
        } else {
            topic_messages.to_vec()
        }
    } else {
        // No topic shift detected — apply max_topic_messages cap from end.
        if messages.len() > config.max_topic_messages {
            let start = messages.len() - config.max_topic_messages;
            messages[start..].to_vec()
        } else {
            messages
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::Message;

    fn cfg() -> TopicIsolationConfig {
        TopicIsolationConfig {
            enabled: true,
            max_topic_messages: 10,
            similarity_threshold: 0.15,
            topic_change_phrases: vec!["new topic".to_string(), "different question".to_string()],
        }
    }

    #[test]
    fn test_disabled_returns_original() {
        let mut config = cfg();
        config.enabled = false;
        let msgs = vec![Message::user("hello"), Message::assistant("hi")];
        let result = apply_topic_isolation(msgs.clone(), &config);
        assert_eq!(result.len(), msgs.len());
    }

    #[test]
    fn test_short_history_unchanged() {
        let config = cfg();
        let msgs = vec![Message::user("hello"), Message::assistant("hi")];
        let result = apply_topic_isolation(msgs.clone(), &config);
        assert_eq!(result.len(), msgs.len());
    }

    #[test]
    fn test_explicit_topic_change_phrase() {
        let config = cfg();
        let msgs = vec![
            Message::user("Tell me about Rust programming language features"),
            Message::assistant("Rust is a systems programming language..."),
            Message::user("What are the benefits of ownership in Rust?"),
            Message::assistant("Ownership provides memory safety..."),
            Message::user("New topic: how do I cook pasta perfectly?"),
            Message::assistant("Boil water, add salt..."),
            Message::user("What sauce goes well with spaghetti?"),
            Message::assistant("Marinara, alfredo, pesto..."),
            // Pad to exceed max_topic_messages threshold
            Message::user("How long should I cook the pasta in boiling water?"),
            Message::assistant("Use plenty of water..."),
            Message::user("Should I add olive oil when cooking pasta?"),
            Message::assistant("You're welcome!"),
        ];
        let result = apply_topic_isolation(msgs, &config);
        // Should start from the "New topic" message (index 4)
        let first_user = result.iter().find(|m| m.role == Role::User).unwrap();
        let text = extract_text(first_user);
        assert!(
            text.contains("cook") || text.contains("pasta") || text.contains("New topic"),
            "Expected topic isolation to start from cooking topic, got: {}",
            text
        );
    }

    #[test]
    fn test_semantic_shift_detection() {
        let config = cfg();
        let msgs = vec![
            Message::user("Explain quantum computing principles and qubits"),
            Message::assistant("Quantum computing uses quantum mechanics..."),
            Message::user("How do quantum gates manipulate qubit states?"),
            Message::assistant("Quantum gates are unitary operators..."),
            Message::user("What ingredients do I need for chocolate cake recipe?"),
            Message::assistant("You need flour, sugar, cocoa..."),
            Message::user("Should I use dark chocolate or milk chocolate for frosting?"),
            Message::assistant("Dark chocolate gives richer flavor..."),
            Message::user("Any good chocolate frosting recipe suggestions please?"),
            Message::assistant("Here's a classic recipe..."),
            Message::user("How long should I bake the chocolate cake?"),
            Message::assistant("About 30-35 minutes at 350F..."),
        ];
        let result = apply_topic_isolation(msgs, &config);
        // Should detect shift from quantum computing to baking
        let first_user = result.iter().find(|m| m.role == Role::User).unwrap();
        let text = extract_text(first_user);
        assert!(
            text.contains("chocolate") || text.contains("cake") || text.contains("ingredient"),
            "Expected baking topic, got: {}",
            text
        );
    }

    #[test]
    fn test_word_overlap_ratio() {
        // Same topic
        let ratio = word_overlap_ratio(
            "Rust programming language features",
            "What are Rust language memory features",
        );
        assert!(ratio > 0.2, "Same-topic overlap should be high: {}", ratio);

        // Different topic
        let ratio = word_overlap_ratio(
            "quantum computing qubits entanglement",
            "chocolate cake recipe baking frosting",
        );
        assert!(
            ratio < 0.1,
            "Different-topic overlap should be low: {}",
            ratio
        );
    }

    #[test]
    fn test_no_shift_single_topic() {
        let config = cfg();
        let msgs = vec![
            Message::user("Tell me about Rust ownership model and borrowing"),
            Message::assistant("Rust ownership ensures memory safety..."),
            Message::user("How does borrowing work in Rust programs?"),
            Message::assistant("Borrowing allows references..."),
            Message::user("What about Rust lifetime annotations?"),
            Message::assistant("Lifetimes ensure references are valid..."),
        ];
        let result = apply_topic_isolation(msgs.clone(), &config);
        assert_eq!(result.len(), msgs.len());
    }

    #[test]
    fn test_max_topic_messages_cap() {
        let mut config = cfg();
        config.max_topic_messages = 4;
        // All same topic, but more than max_topic_messages
        let msgs: Vec<Message> = (0..8)
            .flat_map(|i| {
                vec![
                    Message::user(format!("Tell me more about Rust feature number {i}")),
                    Message::assistant(format!("Feature {i} explanation...")),
                ]
            })
            .collect();
        let result = apply_topic_isolation(msgs, &config);
        assert!(result.len() <= 4, "Should cap at max_topic_messages");
    }

    #[test]
    fn test_stop_words_ignored() {
        assert!(is_stop_word("the"));
        assert!(is_stop_word("with"));
        assert!(!is_stop_word("quantum"));
        assert!(!is_stop_word("programming"));
    }
}
