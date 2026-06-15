//! `skill` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// Skill commands
// ---------------------------------------------------------------------------

/// Validate that a skill name from an (untrusted) manifest is safe to use as a
/// single path component under the skills directory.
///
/// `dest = skills_dir.join(name)` with `Path::join` treats an absolute `name`
/// as a full replacement and lets `..` escape the base, so a package with
/// `name = "../../.librefang/config.toml"` would write outside `skills_dir`.
/// Require the name to be exactly one normal path component.
fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("skill name is empty".to_string());
    }
    let mut comps = std::path::Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(c)), None) if c == std::ffi::OsStr::new(name) => Ok(()),
        _ => Err(format!(
            "unsafe skill name '{name}': must be a single path component (no '/', '\\', '..' or absolute path)"
        )),
    }
}

/// Resolve the skills directory: global or per-hand workspace.
pub(crate) fn resolve_skills_dir(hand: Option<&str>) -> PathBuf {
    let home = librefang_home();
    match hand {
        None => home.join("skills"),
        Some(hand_id) => {
            let hand_dir = home.join("workspaces").join("hands").join(hand_id);
            if !hand_dir.exists() {
                eprintln!("Hand '{hand_id}' not found at {}", hand_dir.display());
                std::process::exit(1);
            }
            hand_dir.join("skills")
        }
    }
}

pub(crate) fn cmd_skill_install(source: &str, hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        eprintln!("Error creating skills directory: {e}");
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        // Local directory install
        let manifest_path = source_path.join("skill.toml");
        if !manifest_path.exists() {
            // Check if it's an OpenClaw skill
            if librefang_skills::openclaw_compat::detect_openclaw_skill(&source_path) {
                println!("Detected OpenClaw skill format. Converting...");
                match librefang_skills::openclaw_compat::convert_openclaw_skill(&source_path) {
                    Ok(manifest) => {
                        if let Err(e) = validate_skill_name(&manifest.skill.name) {
                            eprintln!("Refusing to install skill: {e}");
                            std::process::exit(1);
                        }
                        let dest = skills_dir.join(&manifest.skill.name);
                        // Copy skill directory
                        copy_dir_recursive(&source_path, &dest);
                        if let Err(e) = librefang_skills::openclaw_compat::write_librefang_manifest(
                            &dest, &manifest,
                        ) {
                            eprintln!("Failed to write manifest: {e}");
                            std::process::exit(1);
                        }
                        if let Some(h) = hand {
                            println!(
                                "Installed OpenClaw skill '{}' to hand '{h}'",
                                manifest.skill.name
                            );
                        } else {
                            println!("Installed OpenClaw skill: {}", manifest.skill.name);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to convert OpenClaw skill: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            eprintln!("No skill.toml found in {source}");
            std::process::exit(1);
        }

        // Read manifest to get skill name
        let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            eprintln!("Error reading skill.toml: {e}");
            std::process::exit(1);
        });
        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&toml_str).unwrap_or_else(|e| {
                eprintln!("Error parsing skill.toml: {e}");
                std::process::exit(1);
            });

        if let Err(e) = validate_skill_name(&manifest.skill.name) {
            eprintln!("Refusing to install skill: {e}");
            std::process::exit(1);
        }
        let dest = skills_dir.join(&manifest.skill.name);
        copy_dir_recursive(&source_path, &dest);
        if let Some(h) = hand {
            println!(
                "Installed skill '{}' v{} to hand '{h}'",
                manifest.skill.name, manifest.skill.version
            );
        } else {
            println!(
                "Installed skill: {} v{}",
                manifest.skill.name, manifest.skill.version
            );
        }
    } else {
        // Remote install from FangHub
        let mut sp = progress::auto(&format!("Installing {source}"), None);
        sp.tick(1);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = librefang_skills::marketplace::MarketplaceClient::new(
            librefang_skills::marketplace::MarketplaceConfig::default(),
        );
        match rt.block_on(client.install(source, &skills_dir)) {
            Ok(version) => {
                if let Some(h) = hand {
                    sp.finish(&format!("Installed {source} {version} to hand '{h}'"));
                } else {
                    sp.finish(&format!("Installed {source} {version}"));
                }
            }
            Err(e) => {
                sp.finish_with_failure(&format!("Failed to install skill: {e}"));
                std::process::exit(1);
            }
        }
    }
}

pub(crate) fn cmd_skill_list(hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);

    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
    match registry.load_all() {
        Ok(0) => {
            if let Some(h) = hand {
                println!("No skills installed for hand '{h}'.");
            } else {
                println!("No skills installed.");
            }
        }
        Ok(count) => {
            if let Some(h) = hand {
                println!("{count} skill(s) installed for hand '{h}':\n");
            } else {
                println!("{count} skill(s) installed:\n");
            }
            let mut t = crate::table::Table::new(&["NAME", "VERSION", "TOOLS", "DESCRIPTION"]);
            for skill in registry.list() {
                t.add_row(&[
                    &skill.manifest.skill.name,
                    &skill.manifest.skill.version,
                    &skill.manifest.tools.provided.len().to_string(),
                    &skill.manifest.skill.description,
                ]);
            }
            t.print();
        }
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_remove(name: &str, hand: Option<&str>) {
    // Route through the safe uninstall path (lock + path-traversal
    // guard) instead of `registry.remove()` which calls `remove_dir_all`
    // with no serialisation against concurrent evolve operations.
    let skills_dir = resolve_skills_dir(hand);
    match librefang_skills::evolution::uninstall_skill(&skills_dir, name) {
        Ok(_) => {
            if let Some(h) = hand {
                println!("Removed skill '{name}' from hand '{h}'");
            } else {
                println!("Removed skill: {name}");
            }
        }
        Err(e) => {
            eprintln!("Failed to remove skill: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_search(query: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    match rt.block_on(client.search(query)) {
        Ok(results) if results.is_empty() => println!("No skills found for \"{query}\"."),
        Ok(results) => {
            println!("Skills matching \"{query}\":\n");
            for r in results {
                println!("  {} ({})", r.name, r.stars);
                if !r.description.is_empty() {
                    println!("    {}", r.description);
                }
                println!("    {}", r.url);
                println!();
            }
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_test(path: Option<PathBuf>, tool: Option<String>, input: Option<String>) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Validated skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    println!(
        "  Runtime: {:?}\n  Source: {}",
        prepared.manifest.runtime.runtime_type,
        prepared.source_dir.display()
    );
    if !prepared.manifest.skill.description.is_empty() {
        println!("  Description: {}", prepared.manifest.skill.description);
    }
    if !prepared.manifest.tools.provided.is_empty() {
        println!(
            "  Tools: {}",
            prepared
                .manifest
                .tools
                .provided
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    print_skill_warnings(&prepared.warnings);

    if prepared.has_critical_warnings() {
        eprintln!("Refusing to execute a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let Some(tool_name) = tool.or_else(|| {
        prepared
            .manifest
            .tools
            .provided
            .first()
            .map(|tool| tool.name.clone())
    }) else {
        println!("Validation only: no tool declared to execute.");
        return;
    };

    let input_json = match input {
        Some(input) => serde_json::from_str::<serde_json::Value>(&input).unwrap_or_else(|err| {
            eprintln!("Invalid --input JSON: {err}");
            std::process::exit(1);
        }),
        None => serde_json::json!({}),
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = if prepared.manifest.runtime.runtime_type == librefang_skills::SkillRuntime::Wasm {
        // WASM skills execute in the real sandbox. We pass no kernel handle:
        // pure-compute tools run end to end, while capability-bearing host
        // calls return an error in the result rather than crashing — the right
        // behaviour for a local smoke test outside a running daemon.
        rt.block_on(librefang_runtime::tool_runner::execute_wasm_skill(
            &prepared.manifest,
            &prepared.source_dir,
            &tool_name,
            &input_json,
            None,
            "cli-test",
        ))
    } else {
        let env_policy = load_skill_env_policy_from_config();
        rt.block_on(librefang_skills::loader::execute_skill_tool(
            &prepared.manifest,
            &prepared.source_dir,
            &tool_name,
            &input_json,
            env_policy.as_ref(),
        ))
    };
    match result {
        Ok(result) => {
            println!("\nTool result ({tool_name}):");
            println!(
                "{}",
                serde_json::to_string_pretty(&result.output).unwrap_or_default()
            );
            if result.is_error {
                std::process::exit(1);
            }
        }
        Err(librefang_skills::SkillError::RuntimeNotAvailable(message)) => {
            println!("\nValidation complete.");
            println!("Execution skipped: {message}");
        }
        Err(err) => {
            eprintln!("Skill execution failed: {err}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_publish(
    path: Option<PathBuf>,
    repo: Option<String>,
    tag: Option<String>,
    output: Option<PathBuf>,
    dry_run: bool,
) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Preparing skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    print_skill_warnings(&prepared.warnings);
    if prepared.has_critical_warnings() {
        eprintln!("Refusing to publish a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let output_dir = output.unwrap_or_else(|| prepared.source_dir.join("dist"));
    let packaged = librefang_skills::publish::package_prepared_skill(&prepared, &output_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to package skill: {e}");
            std::process::exit(1);
        });

    println!(
        "Bundle created: {}\n  SHA256: {}\n  Size: {} bytes",
        packaged.archive_path.display(),
        packaged.sha256,
        packaged.size_bytes
    );

    let repo = repo.unwrap_or_else(|| format!("librefang-skills/{}", packaged.manifest.skill.name));
    let tag = tag.unwrap_or_else(|| format!("v{}", packaged.manifest.skill.version));

    if dry_run {
        println!("Dry run only.");
        println!("  Repo: {repo}\n  Tag: {tag}");
        return;
    }

    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .unwrap_or_else(|_| {
            eprintln!("Set GITHUB_TOKEN or GH_TOKEN to publish, or re-run with --dry-run.");
            std::process::exit(1);
        });

    let release_notes = format!(
        "{}\n\nSHA256: `{}`\n\nInstall with:\n`librefang skill install {}`",
        packaged.manifest.skill.description, packaged.sha256, packaged.manifest.skill.name
    );
    let release_name = format!(
        "{} {}",
        packaged.manifest.skill.name, packaged.manifest.skill.version
    );

    let mut sp = progress::auto(
        &format!("Publishing {}@{tag}", packaged.manifest.skill.name),
        None,
    );
    sp.tick(1);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    let published = rt
        .block_on(
            client.publish_bundle(librefang_skills::marketplace::MarketplacePublishRequest {
                repo: &repo,
                tag: &tag,
                bundle_path: &packaged.archive_path,
                release_name: &release_name,
                release_notes: &release_notes,
                token: &token,
            }),
        )
        .unwrap_or_else(|e| {
            sp.finish_with_failure(&format!("Publish failed: {e}"));
            std::process::exit(1);
        });

    sp.finish(&format!(
        "Published {} to {}@{}",
        published.asset_name, published.repo, published.tag
    ));
    if !published.html_url.is_empty() {
        println!("Release: {}", published.html_url);
    }
}

pub(crate) fn resolve_skill_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("Could not determine current directory: {e}");
            std::process::exit(1);
        })
    })
}

pub(crate) fn print_skill_warnings(warnings: &[librefang_skills::verify::SkillWarning]) {
    if warnings.is_empty() {
        println!("  Warnings: none");
        return;
    }

    println!("  Warnings:");
    for warning in warnings {
        println!(
            "    [{}] {}",
            severity_label(warning.severity),
            warning.message
        );
    }
}

pub(crate) fn severity_label(severity: librefang_skills::verify::WarningSeverity) -> &'static str {
    match severity {
        librefang_skills::verify::WarningSeverity::Info => "info",
        librefang_skills::verify::WarningSeverity::Warning => "warn",
        librefang_skills::verify::WarningSeverity::Critical => "critical",
    }
}

pub(crate) fn cmd_skill_create() {
    let name = prompt_input("Skill name: ");
    let description = prompt_input("Description: ");
    let runtime = prompt_input("Runtime (python/node/wasm) [python]: ");
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = librefang_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        eprintln!("Error creating skill directory: {e}");
        std::process::exit(1);
    });

    let tool_name = name.replace('-', "_");

    // A Cargo package name must be `[A-Za-z0-9_-]+` and not start with a digit;
    // a skill name can be anything the user typed. Derive a legal package name
    // for the WASM scaffold's Cargo.toml. The artifact name is fixed to
    // `skill` via `[lib] name`, so this only needs to be valid, not meaningful.
    let pkg_name = {
        let cleaned: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let cleaned = cleaned.trim_matches('-');
        if cleaned.is_empty() {
            "skill".to_string()
        } else if cleaned.starts_with(|c: char| c.is_ascii_digit()) {
            format!("skill-{cleaned}")
        } else {
            cleaned.to_string()
        }
    };

    // Per-runtime scaffold: the manifest `entry` path, the files to write
    // (relative to the skill dir), and any extra build steps the author must
    // run before the entry exists.
    struct Scaffold {
        entry: String,
        files: Vec<(String, String)>,
        build_steps: Vec<String>,
    }

    let scaffold = match runtime.as_str() {
        "python" => Scaffold {
            entry: "src/main.py".to_string(),
            files: vec![(
                "src/main.py".to_string(),
                format!(
                    r#"#!/usr/bin/env python3
"""LibreFang skill: {name}"""
import json
import sys

def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    # TODO: Implement your skill logic here
    result = {{"result": f"Processed: {{input_data.get('input', '')}}"}}

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#
                ),
            )],
            build_steps: vec![],
        },
        "node" => Scaffold {
            entry: "src/index.js".to_string(),
            files: vec![(
                "src/index.js".to_string(),
                format!(
                    r#"// LibreFang skill: {name}
const chunks = [];
process.stdin.on("data", (c) => chunks.push(c));
process.stdin.on("end", () => {{
  const payload = JSON.parse(Buffer.concat(chunks).toString());
  const input = payload.input || {{}};
  // TODO: Implement your skill logic here
  const result = {{ result: `Processed: ${{input.input ?? ""}}` }};
  process.stdout.write(JSON.stringify(result));
}});
"#
                ),
            )],
            build_steps: vec![],
        },
        "wasm" => Scaffold {
            // Entry is the artifact at the skill root, NOT under target/: the
            // packager (`should_include_entry`) excludes `target/`, so a skill
            // referencing the build dir would publish without its binary. The
            // build step copies the compiled module to the root.
            entry: "skill.wasm".to_string(),
            files: vec![
                (
                    "Cargo.toml".to_string(),
                    format!(
                        r#"[package]
name = "{pkg_name}"
version = "0.1.0"
edition = "2021"

[lib]
# Fixed name so the artifact is always `skill.wasm` regardless of package name.
name = "skill"
crate-type = ["cdylib"]

[dependencies]
librefang-skill = "0.1"
serde_json = "1"

[profile.release]
panic = "abort"
"#
                    ),
                ),
                (
                    "src/lib.rs".to_string(),
                    format!(
                        r#"//! LibreFang skill: {name}
use librefang_skill::{{skill, Request}};
use serde_json::{{json, Value}};

pub(crate) fn handle(req: Request) -> Result<Value, String> {{
    match req.tool.as_str() {{
        "{tool_name}" => {{
            // TODO: Implement your skill logic here.
            let input = req.input.get("input").and_then(Value::as_str).unwrap_or("");
            Ok(json!({{ "result": format!("Processed: {{input}}") }}))
        }}
        other => Err(format!("unknown tool: {{other}}")),
    }}
}}

skill!(handle);
"#
                    ),
                ),
            ],
            build_steps: vec![
                "rustup target add wasm32-unknown-unknown".to_string(),
                "cargo build --release --target wasm32-unknown-unknown".to_string(),
                "cp target/wasm32-unknown-unknown/release/skill.wasm skill.wasm".to_string(),
            ],
        },
        other => {
            eprintln!("Unsupported runtime '{other}'. Choose one of: python, node, wasm.");
            std::process::exit(1);
        }
    };

    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "{version}"
description = "{description}"
author = ""
license = "MIT"
tags = []

[runtime]
type = "{runtime}"
entry = "{entry}"

[[tools.provided]]
name = "{tool_name}"
description = "{description}"
input_schema = {{ type = "object", properties = {{ input = {{ type = "string" }} }}, required = ["input"] }}

[requirements]
tools = []
capabilities = []
"#,
        version = librefang_types::VERSION,
        entry = scaffold.entry,
    );

    std::fs::write(skill_dir.join("skill.toml"), &manifest).unwrap();
    for (rel, content) in &scaffold.files {
        let path = skill_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }

    println!("\nSkill created: {}", skill_dir.display());
    println!("\nFiles:");
    println!("  skill.toml");
    for (rel, _) in &scaffold.files {
        println!("  {rel}");
    }
    println!("\nNext steps:");
    let mut step = 1;
    println!("  {step}. Edit the entry point to implement your skill logic");
    for build_step in &scaffold.build_steps {
        step += 1;
        println!("  {step}. {build_step}");
    }
    step += 1;
    println!(
        "  {step}. Test locally: librefang skill test {}",
        skill_dir.display()
    );
    step += 1;
    println!(
        "  {step}. Install: librefang skill install {}",
        skill_dir.display()
    );
}

/// Print an EvolutionResult as a one-line status.
pub(crate) fn print_evolution_result(result: &librefang_skills::evolution::EvolutionResult) {
    let marker = if result.success { "OK" } else { "FAIL" };
    match &result.version {
        Some(v) => println!("[{marker}] {} (v{v})", result.message),
        None => println!("[{marker}] {}", result.message),
    }
}

/// Resolve a skill by name. Respects `--hand` so evolve operations can
/// target a per-hand workspace skills dir just like `install`/`list`.
pub(crate) fn load_installed_skill(
    name: &str,
    hand: Option<&str>,
) -> (PathBuf, librefang_skills::InstalledSkill) {
    let skills_dir = resolve_skills_dir(hand);
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
    if let Err(e) = registry.load_all() {
        eprintln!("Error loading skill registry: {e}");
        std::process::exit(1);
    }
    match registry.get(name) {
        Some(skill) => (skills_dir, skill.clone()),
        None => {
            eprintln!("Skill '{name}' not found in {}", skills_dir.display());
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_evolve(sub: EvolveCommands) {
    match sub {
        EvolveCommands::Create {
            name,
            description,
            context_file,
            tags,
            hand,
        } => {
            let prompt_context = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let tag_list: Vec<String> = tags
                .split(',')
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect();
            let skills_dir = resolve_skills_dir(hand.as_deref());
            if let Err(e) = std::fs::create_dir_all(&skills_dir) {
                eprintln!("Failed to create skills dir: {e}");
                std::process::exit(1);
            }
            match librefang_skills::evolution::create_skill(
                &skills_dir,
                &name,
                &description,
                &prompt_context,
                tag_list,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Create failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Update {
            name,
            context_file,
            changelog,
            hand,
        } => {
            let new_ctx = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::update_skill(
                &skill,
                &new_ctx,
                &changelog,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Patch {
            name,
            old_file,
            new_file,
            changelog,
            replace_all,
            hand,
        } => {
            let old_str = match read_file_or_stdin(&old_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", old_file.display());
                    std::process::exit(1);
                }
            };
            let new_str = match read_file_or_stdin(&new_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", new_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::patch_skill(
                &skill,
                &old_str,
                &new_str,
                &changelog,
                replace_all,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Patch failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Delete { name, hand } => {
            let skills_dir = resolve_skills_dir(hand.as_deref());
            match librefang_skills::evolution::delete_skill(&skills_dir, &name) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Delete failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Rollback { name, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::rollback_skill(&skill, Some("cli")) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Rollback failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::WriteFile {
            name,
            path,
            source,
            hand,
        } => {
            let content = match read_file_or_stdin(&source) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", source.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::write_supporting_file(&skill, &path, &content) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Write-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::RemoveFile { name, path, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::remove_supporting_file(&skill, &path) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Remove-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::History { name, json, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            let meta = librefang_skills::evolution::get_evolution_info(&skill);
            if json {
                match serde_json::to_string_pretty(&meta) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("Failed to serialize history: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            println!("Skill: {}", skill.manifest.skill.name);
            println!("Current version: {}", skill.manifest.skill.version);
            println!("Use count: {}", meta.use_count);
            println!("Evolution count: {}", meta.evolution_count);
            if meta.versions.is_empty() {
                println!("\nNo version history recorded.");
                return;
            }
            println!();
            let mut t = crate::table::Table::new(&["VERSION", "TIMESTAMP", "CHANGELOG"]);
            for v in meta.versions.iter().rev() {
                t.add_row(&[&v.version, &v.timestamp, &v.changelog]);
            }
            t.print();
        }
    }
}

// ---------------------------------------------------------------------------
// Skill workshop pending review (#3328)
// ---------------------------------------------------------------------------

pub(crate) fn cmd_skill_pending(sub: PendingCommands) {
    let skills_root = librefang_home().join("skills");
    match sub {
        PendingCommands::List { agent } => {
            let candidates = match &agent {
                Some(a) => librefang_kernel::skill_workshop::storage::list_pending(&skills_root, a),
                None => librefang_kernel::skill_workshop::storage::list_pending_all(&skills_root),
            };
            let candidates = match candidates {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Failed to read pending directory: {e}");
                    std::process::exit(1);
                }
            };
            if candidates.is_empty() {
                println!(
                    "No pending skill candidates.{}",
                    match &agent {
                        Some(a) => format!(" (filter: agent {a})"),
                        None => String::new(),
                    }
                );
                return;
            }
            println!("{:<38}  {:<18}  {:<22}  NAME", "ID", "SOURCE", "CAPTURED");
            for c in candidates {
                let source_label = match &c.source {
                    librefang_kernel::skill_workshop::CaptureSource::ExplicitInstruction {
                        ..
                    } => "explicit_instr",
                    librefang_kernel::skill_workshop::CaptureSource::UserCorrection { .. } => {
                        "user_correction"
                    }
                    librefang_kernel::skill_workshop::CaptureSource::RepeatedToolPattern {
                        ..
                    } => "tool_pattern",
                };
                println!(
                    "{:<38}  {:<18}  {:<22}  {}",
                    c.id,
                    source_label,
                    c.captured_at.format("%Y-%m-%d %H:%M:%S UTC"),
                    c.name
                );
            }
        }
        PendingCommands::Show { id } => {
            let candidate = match librefang_kernel::skill_workshop::storage::load_candidate(
                &skills_root,
                &id,
            ) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to load candidate: {e}");
                    std::process::exit(1);
                }
            };
            let toml_str = match toml::to_string_pretty(&candidate) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to render candidate as TOML: {e}");
                    std::process::exit(1);
                }
            };
            print!("{toml_str}");
        }
        PendingCommands::Approve { id } => {
            match librefang_kernel::skill_workshop::storage::approve_candidate(
                &skills_root,
                &skills_root,
                &id,
            ) {
                Ok(result) => {
                    println!(
                        "Approved candidate {} → installed skill '{}' (v{}).",
                        id,
                        result.skill_name,
                        result.version.unwrap_or_else(|| "?".to_string())
                    );
                }
                Err(e) => {
                    eprintln!("Approve failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        PendingCommands::Reject { id } => {
            match librefang_kernel::skill_workshop::storage::reject_candidate(&skills_root, &id) {
                Ok(()) => println!("Rejected and removed candidate {id}."),
                Err(e) => {
                    eprintln!("Reject failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_skill_name;

    #[test]
    fn validate_skill_name_accepts_plain_names() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("Skill_1.2").is_ok());
    }

    #[test]
    fn validate_skill_name_rejects_path_traversal_and_absolute() {
        // These would let `skills_dir.join(name)` escape the skills directory.
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("..").is_err());
        assert!(validate_skill_name("../../.librefang/config.toml").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("/etc/passwd").is_err());
        assert!(validate_skill_name("foo/").is_err());
    }
}
