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

/// Break every run of three or more backticks without deleting content.
///
/// A single non-overlapping `replace("```", "``")` is insufficient:
/// four backticks become three. Inserting a visible space before every
/// third backtick guarantees no output run can form a Markdown fence.
fn neutralize_code_fences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut backtick_run = 0;

    for ch in s.chars() {
        if ch == '`' {
            if backtick_run == 2 {
                out.push(' ');
                backtick_run = 0;
            }
            out.push(ch);
            backtick_run += 1;
        } else {
            backtick_run = 0;
            out.push(ch);
        }
    }

    out
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
    let out = neutralize_code_fences(&out)
        .replace("<data>", "(data)")
        .replace("</data>", "(/data)");
    if out.chars().count() <= max_chars {
        return out;
    }
    // UTF-8-safe truncation: keep chars, not bytes.
    let truncated: String = out.chars().take(max_chars.saturating_sub(14)).collect();
    format!("{truncated} …[truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fence_neutralization_handles_every_run_length() {
        for run_len in 3..=12 {
            let input = "`".repeat(run_len);
            let output = neutralize_code_fences(&input);

            assert!(
                !output.contains("```"),
                "run of {run_len} backticks rebuilt a fence: {output:?}"
            );
            assert_eq!(output.chars().filter(|ch| *ch == '`').count(), run_len);
        }
    }

    #[test]
    fn reviewer_block_neutralizes_four_backtick_bypass() {
        let output = sanitize_reviewer_block("before ````json after", 200);

        assert!(!output.contains("```"));
        assert_eq!(output, "before `` ``json after");
    }
}
