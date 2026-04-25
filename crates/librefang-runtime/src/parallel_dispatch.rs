//! Batch tool-call planner for the agent loop's parallel dispatcher.
//!
//! Given the sequence of tool calls the LLM produced in one assistant turn,
//! [`plan_batch`] partitions them into ordered groups: calls within a group
//! may run concurrently, groups themselves run sequentially. The original
//! call ordering is preserved so the resulting `tool_result` blocks line up
//! with the assistant's `tool_use` blocks (a hard requirement on the
//! Anthropic Messages API and respected by every other provider).
//!
//! This module is currently **passive** — `plan_batch` is not called by the
//! agent loop yet. PR-4 / PR-5 will wire it into the non-streaming and
//! streaming dispatchers; PR-3 will gate it behind a config flag. See
//! `.plans/parallel-tool-calls.md` for the full series.
//!
//! # Algorithm summary
//! 1. Empty / single-call batch → trivial group(s).
//! 2. Any [`ParallelSafety::Exclusive`] call → every call gets its own
//!    one-element group (whole batch serialises).
//! 3. Greedy bucketing in original order: each call joins the first
//!    compatible existing bucket, or starts a new one. A bucket is
//!    compatible when it does not yet hold a `WriteShared` member and no
//!    `WriteScoped` member's target path overlaps the candidate's.
//!
//! Path overlap is component-aware ("/a/b/c" vs "/a/bc" do not overlap)
//! and lexical (`..` / `.` are folded without touching the filesystem,
//! since target files may not yet exist).

use crate::tool_classifier::{parallel_safety, ParallelSafety};
use librefang_types::tool::{ToolCall, ToolDefinition};
use std::path::{Component, Path, PathBuf};

/// Result of planning a batch of tool calls. `groups[i]` is a set of indexes
/// into the original `&[ToolCall]` slice; calls in the same group may run
/// concurrently, groups themselves run in order.
///
/// Concatenating the groups in declaration order recovers the index sequence
/// `0..N` — a property the dispatcher relies on when stitching `tool_result`
/// blocks back together in original order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelPlan {
    pub groups: Vec<Vec<usize>>,
}

impl ParallelPlan {
    /// Total number of calls covered by all groups. Equals the input length
    /// for any plan produced by [`plan_batch`].
    pub fn call_count(&self) -> usize {
        self.groups.iter().map(|g| g.len()).sum()
    }

    /// `true` iff this plan describes a fully sequential execution
    /// (every group has at most one element). Used by the dispatcher's
    /// fast path to skip `join_all` overhead.
    pub fn is_fully_sequential(&self) -> bool {
        self.groups.iter().all(|g| g.len() <= 1)
    }
}

/// Path or virtual scope key projected from a tool call's input.
///
/// `Real` paths are compared component-wise with prefix semantics.
/// `Virtual` keys are compared as strings — used for tool families whose
/// "scope" is logical rather than filesystem-backed (e.g. every
/// `skill_evolve_*` call on skill `X` contends with every other edit on
/// `X`, regardless of which file inside the skill it touches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedPath {
    Real(PathBuf),
    Virtual(String),
}

/// Lexical normalization: fold `.` and `..` without touching the filesystem.
///
/// Files written by the upcoming call may not yet exist, so
/// [`std::fs::canonicalize`] is unsafe here. We rely on `Path::components`
/// to handle root, prefix (Windows), and component splitting correctly.
///
/// `..` at the top of a relative path is preserved (`./../x` → `../x`)
/// because we cannot resolve it without a cwd; the caller in
/// [`normalize_path`] supplies the cwd when needed.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut popped_root = false;
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => {
                out.push("/");
                popped_root = true;
            }
            Component::CurDir => {
                // skip "."
            }
            Component::ParentDir => {
                // Above root is still root; otherwise pop the last segment.
                // If we're at the top of a relative path, retain ".." so
                // overlap checks remain conservative (different ".." paths
                // can't be proven disjoint without a cwd).
                if !out.pop() || popped_root && out.as_os_str().is_empty() {
                    if popped_root {
                        out.push("/");
                    } else {
                        out.push("..");
                    }
                }
            }
            Component::Normal(n) => out.push(n),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Normalize a raw path string into a [`NormalizedPath::Real`].
///
/// Trailing slashes are stripped. Relative paths are joined onto the
/// current working directory; if the cwd cannot be determined we keep the
/// path relative — overlap with absolute paths then defaults to `false`,
/// which is the conservative answer (different roots can't be proven to
/// overlap, so the planner runs them in separate buckets).
fn normalize_path(raw: &str) -> NormalizedPath {
    let trimmed = raw.trim_end_matches('/');
    let p = Path::new(trimmed);
    let expanded: PathBuf = if p.is_absolute() {
        p.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(p)
    } else {
        p.to_path_buf()
    };
    NormalizedPath::Real(lexical_clean(&expanded))
}

/// Component-aware prefix overlap.
///
/// Two `Real` paths overlap when one is a (component) prefix of the other.
/// Two `Virtual` paths overlap iff they are string-equal. Mixed kinds do
/// not overlap (filesystem and virtual scope are independent namespaces).
///
/// Examples:
/// - `/a/b` and `/a/b/c` → overlap (parent / child).
/// - `/a/b` and `/a/bc` → no overlap (component split).
/// - `/a/./b` and `/a/b` → overlap (lexically equal after [`lexical_clean`]).
fn paths_overlap(a: &NormalizedPath, b: &NormalizedPath) -> bool {
    match (a, b) {
        (NormalizedPath::Virtual(x), NormalizedPath::Virtual(y)) => x == y,
        (NormalizedPath::Real(x), NormalizedPath::Real(y)) => x.starts_with(y) || y.starts_with(x),
        _ => false,
    }
}

/// Project a path-shaped scope from a [`ToolCall`]'s input. Returns `None`
/// when the tool isn't path-scoped or when the input doesn't carry the
/// expected `path` / `name` field.
///
/// Only called for [`ParallelSafety::WriteScoped`] tools — read-only tools
/// don't need a scope, write-shared tools own their bucket regardless.
fn extract_scope_path(tool: &str, input: &serde_json::Value) -> Option<NormalizedPath> {
    let raw = match tool {
        "file_write" | "file_edit" | "apply_patch" => input.get("path").and_then(|v| v.as_str())?,
        s if s.starts_with("skill_evolve_") => {
            let name = input.get("name").and_then(|v| v.as_str())?;
            return Some(NormalizedPath::Virtual(format!("skill::{name}")));
        }
        _ => return None,
    };
    if raw.is_empty() {
        return None;
    }
    Some(normalize_path(raw))
}

/// Look up a [`ToolDefinition`] by name within a slice. Linear search is
/// fine — N is small (a single LLM turn rarely exceeds 16 tools, and the
/// agent's tool catalog is in the low hundreds).
fn find_def<'a>(defs: &'a [ToolDefinition], name: &str) -> Option<&'a ToolDefinition> {
    defs.iter().find(|d| d.name == name)
}

/// Plan how to dispatch a batch of tool calls.
///
/// Guarantees:
/// - **Order preservation**: `plan.groups.iter().flatten()` yields
///   `0, 1, …, calls.len() - 1`. The dispatcher relies on this when
///   stitching `tool_result` blocks back together for the model.
/// - **Sequential semantics across barriers**: groups are contiguous
///   index ranges. A `WriteShared` (e.g. `shell_exec`) acts as a
///   barrier — no `ReadOnly` peer that comes *after* it in the
///   original order can be reordered into a *previous* bucket.
///   Without this rule a later read would observe state from
///   *before* the shell ran, even though the model emitted it
///   *after* the shell call expecting the post-shell view.
/// - **Concurrency within a group**: no two members touch overlapping
///   `WriteScoped` paths, no member is `WriteShared`, and the batch
///   contains no `Exclusive` calls (those force every call into its
///   own one-element group).
/// - **Complexity**: `O(N · P)` where P is the number of paths
///   reserved in the current bucket. Effectively linear for the
///   typical N ≤ 16 case.
pub fn plan_batch(calls: &[ToolCall], defs: &[ToolDefinition]) -> ParallelPlan {
    if calls.is_empty() {
        return ParallelPlan { groups: vec![] };
    }
    if calls.len() == 1 {
        return ParallelPlan {
            groups: vec![vec![0]],
        };
    }

    let safeties: Vec<ParallelSafety> = calls
        .iter()
        .map(|c| parallel_safety(&c.name, find_def(defs, &c.name)))
        .collect();

    // Any Exclusive call forces the whole batch to serialise.
    if safeties
        .iter()
        .any(|s| matches!(s, ParallelSafety::Exclusive))
    {
        return ParallelPlan {
            groups: (0..calls.len()).map(|i| vec![i]).collect(),
        };
    }

    // Contiguous-bucket scheduling: walk in order, accumulating into a
    // "current" bucket. Each call either joins it, forces a flush + new
    // bucket, or sits in its own bucket (and immediately flushes it).
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_paths: Vec<NormalizedPath> = Vec::new();

    for (i, call) in calls.iter().enumerate() {
        let safety = safeties[i];
        match safety {
            // Pre-filter above guarantees we don't see Exclusive here.
            ParallelSafety::Exclusive => {
                unreachable!("Exclusive should have triggered the all-sequential branch")
            }

            ParallelSafety::WriteShared => {
                // Barrier: flush any in-flight bucket, then drop this
                // call into its own bucket. The next call starts fresh,
                // never reusing the pre-barrier bucket.
                if !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                    current_paths.clear();
                }
                groups.push(vec![i]);
            }

            ParallelSafety::ReadOnly => {
                current.push(i);
            }

            ParallelSafety::WriteScoped => {
                let scope = extract_scope_path(&call.name, &call.input);
                let conflict = match &scope {
                    Some(p) => current_paths.iter().any(|q| paths_overlap(p, q)),
                    // No projectable path → cannot prove disjointness with
                    // any peer in the current bucket. Treat as conflict
                    // when the bucket is non-empty.
                    None => !current.is_empty(),
                };
                if conflict {
                    groups.push(std::mem::take(&mut current));
                    current_paths.clear();
                }
                current.push(i);
                match scope {
                    Some(p) => current_paths.push(p),
                    None => {
                        // No scope → cannot accept any future peer either.
                        // Flush immediately so the next call starts a new
                        // bucket.
                        groups.push(std::mem::take(&mut current));
                        current_paths.clear();
                    }
                }
            }
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }

    ParallelPlan { groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            input,
        }
    }

    /// Order preservation: flattening the plan must yield 0..N for every
    /// case the planner produces. This is the dispatcher's hard contract.
    fn assert_plan_covers_all(plan: &ParallelPlan, n: usize) {
        let flat: Vec<usize> = plan.groups.iter().flatten().copied().collect();
        let expected: Vec<usize> = (0..n).collect();
        let mut sorted = flat.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "plan must cover every index exactly once");
        assert_eq!(plan.call_count(), n);
    }

    #[test]
    fn empty_batch_produces_empty_plan() {
        let plan = plan_batch(&[], &[]);
        assert_eq!(plan.groups.len(), 0);
        assert_eq!(plan.call_count(), 0);
        assert!(plan.is_fully_sequential());
    }

    #[test]
    fn single_call_is_one_group() {
        let calls = vec![call("a", "file_read", json!({"path": "/x"}))];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0]]);
        assert!(plan.is_fully_sequential());
        assert_plan_covers_all(&plan, 1);
    }

    /// 3 reads on disjoint paths — fully parallelisable. One group.
    #[test]
    fn three_reads_one_group() {
        let calls = vec![
            call("a", "file_read", json!({"path": "/a"})),
            call("b", "file_read", json!({"path": "/b"})),
            call("c", "file_read", json!({"path": "/c"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0, 1, 2]]);
        assert!(!plan.is_fully_sequential());
        assert_plan_covers_all(&plan, 3);
    }

    /// Read + write on disjoint dirs — the read is `ReadOnly` (no scope),
    /// the write is `WriteScoped` with a different path. Same group OK.
    #[test]
    fn read_plus_write_disjoint_one_group() {
        let calls = vec![
            call("a", "file_read", json!({"path": "/a"})),
            call("b", "file_write", json!({"path": "/b", "content": "x"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0, 1]]);
        assert_plan_covers_all(&plan, 2);
    }

    /// Two writes on different files in the same dir — paths don't share a
    /// component prefix, so they parallelise.
    #[test]
    fn two_writes_sibling_files_one_group() {
        let calls = vec![
            call("a", "file_write", json!({"path": "/a/x", "content": "1"})),
            call("b", "file_write", json!({"path": "/a/y", "content": "2"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0, 1]]);
        assert_plan_covers_all(&plan, 2);
    }

    /// Parent / child overlap — must split into two groups.
    #[test]
    fn parent_child_overlap_splits() {
        let calls = vec![
            call("a", "file_write", json!({"path": "/a/b", "content": "1"})),
            call("b", "file_write", json!({"path": "/a/b/c", "content": "2"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0], vec![1]]);
        assert_plan_covers_all(&plan, 2);
    }

    /// Component vs string prefix: "/a/b" should NOT overlap "/a/bc".
    #[test]
    fn component_aware_prefix_does_not_split() {
        let calls = vec![
            call("a", "file_write", json!({"path": "/a/b", "content": "1"})),
            call("b", "file_write", json!({"path": "/a/bc", "content": "2"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0, 1]]);
        assert_plan_covers_all(&plan, 2);
    }

    /// Trailing slashes and lexical `..` are normalised — paths that
    /// resolve to the same canonical form must overlap.
    #[test]
    fn trailing_slash_and_parent_dir_normalise() {
        let calls = vec![
            call("a", "file_write", json!({"path": "/a/b/", "content": "1"})),
            call(
                "b",
                "file_write",
                json!({"path": "/a/b/c/..", "content": "2"}),
            ),
        ];
        let plan = plan_batch(&calls, &[]);
        // Both resolve to /a/b → overlap → split.
        assert_eq!(plan.groups, vec![vec![0], vec![1]]);
        assert_plan_covers_all(&plan, 2);
    }

    /// `shell_exec` is `WriteShared` — owns its bucket. Adjacent reads
    /// can still parallelise around it.
    #[test]
    fn shell_exec_isolated_in_its_bucket() {
        let calls = vec![
            call("a", "file_read", json!({"path": "/a"})),
            call("b", "shell_exec", json!({"command": "ls"})),
            call("c", "file_read", json!({"path": "/c"})),
        ];
        let plan = plan_batch(&calls, &[]);
        // group 0: read a (alone, then shell joins won't happen because
        //          shell is WriteShared)
        // group 1: shell_exec (owns it)
        // group 2: read c (cannot rejoin group 0 — only forward bucket
        //          creation; greedy doesn't reorder)
        // Order preservation matters more than bucket minimisation.
        assert_eq!(plan.groups.len(), 3);
        assert_eq!(plan.groups[1], vec![1]);
        assert_plan_covers_all(&plan, 3);
    }

    /// An `Exclusive` call (e.g. approval_request) forces every call into
    /// its own group — no concurrency anywhere in the batch.
    #[test]
    fn interactive_forces_full_serial() {
        let calls = vec![
            call("a", "file_read", json!({"path": "/a"})),
            call("b", "approval_request", json!({"reason": "x"})),
            call("c", "file_read", json!({"path": "/c"})),
        ];
        let plan = plan_batch(&calls, &[]);
        assert_eq!(plan.groups, vec![vec![0], vec![1], vec![2]]);
        assert!(plan.is_fully_sequential());
        assert_plan_covers_all(&plan, 3);
    }

    /// Virtual scope: two `skill_evolve_*` calls on the same skill must
    /// split, two on different skills can run together.
    #[test]
    fn skill_evolve_virtual_scope() {
        let same = vec![
            call(
                "a",
                "skill_evolve_update",
                json!({"name": "alpha", "patch": "..."}),
            ),
            call(
                "b",
                "skill_evolve_patch",
                json!({"name": "alpha", "patch": "..."}),
            ),
        ];
        let plan_same = plan_batch(&same, &[]);
        assert_eq!(
            plan_same.groups,
            vec![vec![0], vec![1]],
            "same skill name → split"
        );

        let diff = vec![
            call(
                "a",
                "skill_evolve_update",
                json!({"name": "alpha", "patch": "..."}),
            ),
            call(
                "b",
                "skill_evolve_patch",
                json!({"name": "beta", "patch": "..."}),
            ),
        ];
        let plan_diff = plan_batch(&diff, &[]);
        assert_eq!(
            plan_diff.groups,
            vec![vec![0, 1]],
            "different skills → same group"
        );
    }

    /// `WriteScoped` call without an extractable `path` field falls back
    /// to "single-call bucket" — never proven safe to share.
    #[test]
    fn write_scoped_without_path_is_isolated() {
        let calls = vec![
            call("a", "file_read", json!({"path": "/a"})),
            // file_write missing `path` → WriteScoped without scope.
            call("b", "file_write", json!({"content": "x"})),
            call("c", "file_read", json!({"path": "/c"})),
        ];
        let plan = plan_batch(&calls, &[]);
        // 0 in own bucket, then 1 starts a fresh bucket because it cannot
        // join the read-only one without a scope, then 2 starts another.
        assert_eq!(plan.groups.len(), 3);
        assert_eq!(plan.groups[1], vec![1]);
        assert_plan_covers_all(&plan, 3);
    }

    /// Path overlap between `Real` and `Virtual` always returns false —
    /// distinct namespaces.
    #[test]
    fn real_vs_virtual_paths_do_not_overlap() {
        let real = NormalizedPath::Real(PathBuf::from("/a"));
        let virt = NormalizedPath::Virtual("skill::a".into());
        assert!(!paths_overlap(&real, &virt));
        assert!(!paths_overlap(&virt, &real));
    }

    #[test]
    fn lexical_clean_handles_dot_and_double_dot() {
        assert_eq!(lexical_clean(Path::new("/a/./b")), PathBuf::from("/a/b"));
        assert_eq!(lexical_clean(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        // Trailing slash on the input is already stripped before this fn,
        // but the lexical clean must still produce a stable form.
        assert_eq!(lexical_clean(Path::new("/a/b")), PathBuf::from("/a/b"));
    }
}
