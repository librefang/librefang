//! Contract tests for the bundled `workflow-creator` skill (#6934).
//!
//! The skill itself is registry content: it ships from `librefang/librefang-registry` under `skills/workflow-creator/` and is installed on demand like the other prompt-only skills the dashboard lists.
//! What is testable here is the copy pinned in this repo's registry snapshot, and that copy is what the tests below load — so a change to the shipped skill that stops it loading, trips the load-boundary injection scan, or drifts away from the ceilings `workflow_create` actually enforces fails CI rather than reaching an agent's system prompt.
//!
//! Lives in `librefang-runtime` because that is the crate holding the registry snapshot (`registry_sync::REGISTRY_FIXTURE_DIR`) and it already depends on `librefang-skills`, so both halves of the contract are visible from one place.

use librefang_skills::registry::SkillRegistry;
use librefang_skills::SkillRuntime;
use std::path::{Path, PathBuf};

/// Name the skill registers under. Also its directory name in the snapshot.
const SKILL_NAME: &str = "workflow-creator";

/// The skill's directory inside the pinned registry snapshot.
fn snapshot_skill_dir() -> PathBuf {
    Path::new(librefang_runtime::registry_sync::REGISTRY_FIXTURE_DIR)
        .join("skills")
        .join(SKILL_NAME)
}

/// Copy the snapshot's `skills/` tree into a scratch directory.
///
/// `SkillRegistry::load_all` writes `skill.toml` and `prompt_context.md` beside a `SKILL.md` it auto-converts, so pointing it at the snapshot in-place would mutate a checked-in fixture.
fn staged_skills_dir(tmp: &Path) -> PathBuf {
    let dest = tmp.join("skills").join(SKILL_NAME);
    std::fs::create_dir_all(&dest).expect("create staged skill dir");
    for entry in std::fs::read_dir(snapshot_skill_dir()).expect("read snapshot skill dir") {
        let entry = entry.expect("snapshot dir entry");
        std::fs::copy(entry.path(), dest.join(entry.file_name())).expect("copy snapshot file");
    }
    tmp.join("skills")
}

/// Write a minimal prompt-only skill so the determinism test has neighbours to be ordered against.
fn write_neighbour_skill(skills_dir: &Path, name: &str) {
    let dir = skills_dir.join(name);
    std::fs::create_dir_all(&dir).expect("create neighbour skill dir");
    std::fs::write(
        dir.join("skill.toml"),
        format!(
            "prompt_context = \"Body of {name}.\"\n\n[skill]\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"Neighbour skill for ordering\"\n\n[runtime]\ntype = \"promptonly\"\n"
        ),
    )
    .expect("write neighbour manifest");
}

/// The registry ships this skill as `SKILL.md` and nothing else, and that is load-bearing rather than incidental.
///
/// `SkillRegistry::load_all` only auto-converts a `SKILL.md` when the directory has no `skill.toml`.
/// Adding a `skill.toml` beside it without also writing `prompt_context.md` would load a manifest with no prompt context at all — the entire body of a prompt-only skill silently dropped, with no error anywhere.
#[test]
fn workflow_creator_ships_as_a_skill_md_with_no_competing_manifest() {
    let dir = snapshot_skill_dir();
    assert!(
        dir.join("SKILL.md").is_file(),
        "the snapshot must carry {SKILL_NAME}/SKILL.md at {}",
        dir.display()
    );
    assert!(
        !dir.join("skill.toml").exists(),
        "a skill.toml beside SKILL.md suppresses the auto-convert and drops the prompt body; \
         if structured metadata is genuinely needed, write prompt_context.md as well"
    );
}

/// The shipped skill loads, registers under its own name, and arrives as prompt context rather than as a tool.
///
/// `load_all` returns `Err` from the load-boundary injection scan, so a body that trips a critical pattern fails here rather than at an operator's install.
#[test]
fn workflow_creator_loads_and_registers_as_a_prompt_only_skill() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skills_dir = staged_skills_dir(tmp.path());

    let mut registry = SkillRegistry::new(skills_dir);
    let loaded = registry.load_all().expect("the shipped skill must load");
    assert_eq!(loaded, 1, "exactly the staged skill should load");

    let skill = registry
        .get(SKILL_NAME)
        .unwrap_or_else(|| panic!("{SKILL_NAME} must register under its frontmatter name"));
    assert_eq!(
        skill.manifest.runtime.runtime_type,
        SkillRuntime::PromptOnly
    );
    assert!(
        skill.manifest.tools.provided.is_empty(),
        "a teaching skill must not declare tools of its own — it teaches the builtin workflow_create"
    );

    let body = skill
        .manifest
        .prompt_context
        .as_deref()
        .expect("the Markdown body must survive as prompt context");
    assert!(
        body.contains("workflow_create"),
        "the skill exists to teach workflow_create and must name it"
    );
    assert!(
        !skill.manifest.skill.description.is_empty(),
        "the description lands in the <available_skills> prompt block and must not be empty"
    );
}

/// The skill's prompt-facing surface must not depend on the order skills were loaded in (#3298).
///
/// Registry order reaches the LLM through `list()` and `all_tool_definitions()`; a reorder across processes invalidates provider prompt caches even when nothing changed.
/// Both registries below hold identical content and differ only in insertion order.
#[test]
fn workflow_creator_prompt_surface_is_deterministic_across_insertion_orders() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skills_dir = staged_skills_dir(tmp.path());
    // Names chosen to straddle `workflow-creator` alphabetically, so a registry that
    // emitted insertion order rather than sorted order would be caught either way.
    write_neighbour_skill(&skills_dir, "alpha-helper");
    write_neighbour_skill(&skills_dir, "zeta-helper");

    // `load_all` converts SKILL.md in place, so this also leaves a skill.toml behind
    // for the by-directory loads below.
    let mut in_order = SkillRegistry::new(skills_dir.clone());
    assert_eq!(in_order.load_all().expect("load_all"), 3);

    let mut reversed = SkillRegistry::new(skills_dir.clone());
    for name in ["zeta-helper", SKILL_NAME, "alpha-helper"] {
        reversed
            .load_skill(&skills_dir.join(name))
            .unwrap_or_else(|e| panic!("load {name}: {e}"));
    }

    let render = |registry: &SkillRegistry| -> String {
        registry
            .list()
            .iter()
            .map(|s| {
                format!(
                    "{}\n{}\n{}",
                    s.manifest.skill.name,
                    s.manifest.skill.description,
                    s.manifest.prompt_context.as_deref().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    };
    assert_eq!(
        render(&in_order),
        render(&reversed),
        "the rendered skill block must be byte-identical across insertion orders (#3298)"
    );
    assert_eq!(
        in_order.list()[1].manifest.skill.name,
        SKILL_NAME,
        "sorted emission must place {SKILL_NAME} between its neighbours"
    );
    assert_eq!(
        in_order.all_tool_definitions().len(),
        reversed.all_tool_definitions().len()
    );
}

/// The numbers in the skill's prose must be the numbers `workflow_create` advertises.
///
/// The skill teaches ceilings and key names, and prose drifts silently: a model told "at most 50 steps" by a skill and rejected at a different ceiling by the tool burns a turn discovering the difference.
/// Reading them out of the live tool schema rather than hardcoding them here means a limit change fails this test instead of quietly making the skill wrong.
#[test]
fn workflow_creator_prose_matches_the_live_workflow_create_schema() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let skills_dir = staged_skills_dir(tmp.path());
    let mut registry = SkillRegistry::new(skills_dir);
    registry.load_all().expect("load_all");
    let body = registry
        .get(SKILL_NAME)
        .expect("skill registered")
        .manifest
        .prompt_context
        .clone()
        .expect("prompt context");

    let defs = librefang_runtime::tool_runner::builtin_tool_definitions();
    let schema = defs
        .iter()
        .find(|d| d.name == "workflow_create")
        .expect("workflow_create must be a builtin tool")
        .input_schema
        .clone();
    let steps = &schema["properties"]["steps"];

    for (label, value) in [
        ("step ceiling", steps["maxItems"].as_u64()),
        (
            "per-step timeout ceiling",
            steps["items"]["properties"]["timeout_secs"]["maximum"].as_u64(),
        ),
        (
            "total timeout ceiling",
            schema["properties"]["total_timeout_secs"]["maximum"].as_u64(),
        ),
    ] {
        let value = value.unwrap_or_else(|| panic!("{label} must be advertised in the schema"));
        assert!(
            body.contains(&value.to_string()),
            "the skill must quote the live {label} ({value}); it teaches a limit the tool enforces"
        );
    }

    // Every step key the skill teaches has to be one the tool actually accepts.
    let step_props = &steps["items"]["properties"];
    for key in [
        "depends_on",
        "output_var",
        "error_mode",
        "required_skills",
        "session_mode",
        "inherit_context",
    ] {
        assert!(
            step_props[key].is_object(),
            "the skill documents step key `{key}`, which workflow_create does not advertise"
        );
        assert!(
            body.contains(key),
            "step key `{key}` is part of the contract and should be covered by the skill"
        );
    }

    assert!(
        body.contains("param_type"),
        "input parameters use `param_type`, not `type` — the skill has to say so"
    );
}
