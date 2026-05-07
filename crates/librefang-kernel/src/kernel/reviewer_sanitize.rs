//! Author-supplied content sanitizers for the background skill reviewer
//! prompt. Skill names, descriptions, and trace/response summaries are
//! attacker-influenced — these helpers neutralize the markers and control
//! characters that could otherwise let a compromised payload break out of
//! the reviewer envelope.

/// Sanitize a single-line author-supplied string (skill name, description)
/// for safe interpolation into the reviewer's user message.
///
/// Thin wrapper over `librefang_runtime::prompt_builder::sanitize_for_prompt`
/// — delegating keeps the bracket- and control-char rules consistent with
/// the main prompt builder.
pub(super) fn sanitize_reviewer_line(s: &str, max_chars: usize) -> String {
    librefang_runtime::prompt_builder::sanitize_for_prompt(s, max_chars)
}

/// Sanitize a multi-line block (trace summary, response summary) for
/// embedding inside `<data>…</data>` markers in the reviewer prompt.
///
/// Preserves `\n` (the caller wants readable structure) but strips:
/// - `\r`, null bytes, and other C0 control characters that some LLMs
///   misinterpret as structural separators.
/// - Triple backticks, so the reviewer can't be tricked into treating
///   content as the start of its own code-fenced answer block (which
///   `extract_json_from_llm_response` later greps for).
/// - `<data>` / `</data>` markers, so nothing inside the block can
///   prematurely close our envelope and escape into instructional scope.
///
/// Hard-capped at `max_chars`; truncation is signalled with a trailing
/// `" …[truncated]"`.
pub(super) fn sanitize_reviewer_block(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars));
    for ch in s.chars() {
        // Keep \n, \t. Drop other controls. Everything else passes.
        if ch == '\n' || ch == '\t' {
            out.push(ch);
        } else if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    // Neutralize markers that could break out of the reviewer's data block
    // or forge an answer code fence. Replace rather than strip so the
    // content's shape (indentation, line structure) stays recognizable.
    let out = out
        .replace("```", "``")
        .replace("<data>", "(data)")
        .replace("</data>", "(/data)");
    if out.chars().count() <= max_chars {
        return out;
    }
    // UTF-8-safe truncation: keep chars, not bytes.
    let truncated: String = out.chars().take(max_chars.saturating_sub(14)).collect();
    format!("{truncated} …[truncated]")
}
