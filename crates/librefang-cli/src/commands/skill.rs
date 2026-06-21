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
        return Err(i18n::t("skill-name-empty"));
    }
    let mut comps = std::path::Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(c)), None) if c == std::ffi::OsStr::new(name) => Ok(()),
        _ => Err(i18n::t_args("skill-name-unsafe", &[("name", name)])),
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
                let path_str = hand_dir.display().to_string();
                eprintln!(
                    "{}",
                    i18n::t_args(
                        "skill-hand-not-found",
                        &[("hand", hand_id), ("path", &path_str)]
                    )
                );
                std::process::exit(1);
            }
            hand_dir.join("skills")
        }
    }
}

pub(crate) fn cmd_skill_install(source: &str, hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        let err_msg = e.to_string();
        eprintln!(
            "{}",
            i18n::t_args("skill-create-skills-dir-failed", &[("error", &err_msg)])
        );
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        // Local directory install
        let manifest_path = source_path.join("skill.toml");
        if !manifest_path.exists() {
            // Check if it's an OpenClaw skill
            if librefang_skills::openclaw_compat::detect_openclaw_skill(&source_path) {
                println!("{}", i18n::t("skill-openclaw-detected"));
                match librefang_skills::openclaw_compat::convert_openclaw_skill(&source_path) {
                    Ok(manifest) => {
                        if let Err(e) = validate_skill_name(&manifest.skill.name) {
                            eprintln!(
                                "{}",
                                i18n::t_args("skill-install-refused", &[("error", &e)])
                            );
                            std::process::exit(1);
                        }
                        let dest = skills_dir.join(&manifest.skill.name);
                        // Copy skill directory
                        copy_dir_recursive(&source_path, &dest);
                        if let Err(e) = librefang_skills::openclaw_compat::write_librefang_manifest(
                            &dest, &manifest,
                        ) {
                            let err_msg = e.to_string();
                            eprintln!(
                                "{}",
                                i18n::t_args("skill-write-manifest-failed", &[("error", &err_msg)])
                            );
                            std::process::exit(1);
                        }
                        if let Some(h) = hand {
                            println!(
                                "{}",
                                i18n::t_args(
                                    "skill-openclaw-installed-to-hand",
                                    &[("name", &manifest.skill.name), ("hand", h)]
                                )
                            );
                        } else {
                            println!(
                                "{}",
                                i18n::t_args(
                                    "skill-openclaw-installed",
                                    &[("name", &manifest.skill.name)]
                                )
                            );
                        }
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        eprintln!(
                            "{}",
                            i18n::t_args("skill-openclaw-convert-failed", &[("error", &err_msg)])
                        );
                        std::process::exit(1);
                    }
                }
                return;
            }
            eprintln!("{}", i18n::t_args("skill-no-toml", &[("path", source)]));
            std::process::exit(1);
        }

        // Read manifest to get skill name
        let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-read-toml-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        });
        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&toml_str).unwrap_or_else(|e| {
                let err_msg = e.to_string();
                eprintln!(
                    "{}",
                    i18n::t_args("skill-parse-toml-failed", &[("error", &err_msg)])
                );
                std::process::exit(1);
            });

        if let Err(e) = validate_skill_name(&manifest.skill.name) {
            eprintln!(
                "{}",
                i18n::t_args("skill-install-refused", &[("error", &e)])
            );
            std::process::exit(1);
        }
        let dest = skills_dir.join(&manifest.skill.name);
        copy_dir_recursive(&source_path, &dest);
        if let Some(h) = hand {
            println!(
                "{}",
                i18n::t_args(
                    "skill-installed-to-hand",
                    &[
                        ("name", &manifest.skill.name),
                        ("version", &manifest.skill.version),
                        ("hand", h)
                    ]
                )
            );
        } else {
            println!(
                "{}",
                i18n::t_args(
                    "skill-installed",
                    &[
                        ("name", &manifest.skill.name),
                        ("version", &manifest.skill.version)
                    ]
                )
            );
        }
    } else {
        // Remote install from FangHub
        let mut sp = progress::auto(
            &i18n::t_args("skill-install-progress", &[("source", source)]),
            None,
        );
        sp.tick(1);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = librefang_skills::marketplace::MarketplaceClient::new(
            librefang_skills::marketplace::MarketplaceConfig::default(),
        );
        match rt.block_on(client.install(source, &skills_dir)) {
            Ok(version) => {
                if let Some(h) = hand {
                    sp.finish(&i18n::t_args(
                        "skill-installed-hub-to-hand",
                        &[("source", source), ("version", &version), ("hand", h)],
                    ));
                } else {
                    sp.finish(&i18n::t_args(
                        "skill-installed-hub",
                        &[("source", source), ("version", &version)],
                    ));
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                sp.finish_with_failure(&i18n::t_args(
                    "skill-install-failed",
                    &[("error", &err_msg)],
                ));
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
                println!("{}", i18n::t_args("skill-list-none-hand", &[("hand", h)]));
            } else {
                println!("{}", i18n::t("skill-list-none"));
            }
        }
        Ok(count) => {
            if let Some(h) = hand {
                let count_str = count.to_string();
                println!(
                    "{}",
                    i18n::t_args(
                        "skill-list-count-hand",
                        &[("count", &count_str), ("hand", h)]
                    )
                );
            } else {
                let count_str = count.to_string();
                println!(
                    "{}",
                    i18n::t_args("skill-list-count", &[("count", &count_str)])
                );
            }
            println!();
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
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-list-load-failed", &[("error", &err_msg)])
            );
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
                println!(
                    "{}",
                    i18n::t_args("skill-removed-from-hand", &[("name", name), ("hand", h)])
                );
            } else {
                println!("{}", i18n::t_args("skill-removed", &[("name", name)]));
            }
        }
        Err(e) => {
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-remove-failed", &[("error", &err_msg)])
            );
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
        Ok(results) if results.is_empty() => {
            println!("{}", i18n::t_args("skill-search-none", &[("query", query)]));
        }
        Ok(results) => {
            println!(
                "{}",
                i18n::t_args("skill-search-results-header", &[("query", query)])
            );
            println!();
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
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-search-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_test(path: Option<PathBuf>, tool: Option<String>, input: Option<String>) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-validation-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        });

    println!(
        "{}",
        i18n::t_args(
            "skill-validated",
            &[
                ("name", prepared.manifest.skill.name.as_str()),
                ("version", prepared.manifest.skill.version.as_str())
            ]
        )
    );
    let runtime_type_str = format!("{:?}", prepared.manifest.runtime.runtime_type);
    println!(
        "{}",
        i18n::t_args("skill-validated-runtime", &[("runtime", &runtime_type_str)])
    );
    let source_dir_str = prepared.source_dir.display().to_string();
    println!(
        "{}",
        i18n::t_args("skill-validated-source", &[("path", &source_dir_str)])
    );
    if !prepared.manifest.skill.description.is_empty() {
        println!(
            "{}",
            i18n::t_args(
                "skill-validated-description",
                &[("description", prepared.manifest.skill.description.as_str())]
            )
        );
    }
    if !prepared.manifest.tools.provided.is_empty() {
        let tools_list = prepared
            .manifest
            .tools
            .provided
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{}",
            i18n::t_args("skill-validated-tools", &[("tools", &tools_list)])
        );
    }
    print_skill_warnings(&prepared.warnings);

    if prepared.has_critical_warnings() {
        eprintln!("{}", i18n::t("skill-refusing-warnings"));
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
        println!("{}", i18n::t("skill-validated-only"));
        return;
    };

    let input_json = match input {
        Some(input) => serde_json::from_str::<serde_json::Value>(&input).unwrap_or_else(|err| {
            let err_msg = err.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-invalid-input-json", &[("error", &err_msg)])
            );
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
            println!();
            println!(
                "{}",
                i18n::t_args("skill-tool-result-header", &[("name", &tool_name)])
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&result.output).unwrap_or_default()
            );
            if result.is_error {
                std::process::exit(1);
            }
        }
        Err(librefang_skills::SkillError::RuntimeNotAvailable(message)) => {
            println!();
            println!("{}", i18n::t("skill-validation-complete"));
            println!(
                "{}",
                i18n::t_args("skill-execution-skipped", &[("message", &message)])
            );
        }
        Err(err) => {
            let err_msg = err.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-execution-failed", &[("error", &err_msg)])
            );
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
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-validation-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        });

    println!(
        "{}",
        i18n::t_args(
            "skill-preparing",
            &[
                ("name", prepared.manifest.skill.name.as_str()),
                ("version", prepared.manifest.skill.version.as_str())
            ]
        )
    );
    print_skill_warnings(&prepared.warnings);
    if prepared.has_critical_warnings() {
        eprintln!("{}", i18n::t("skill-refusing-publish"));
        std::process::exit(1);
    }

    let output_dir = output.unwrap_or_else(|| prepared.source_dir.join("dist"));
    let packaged = librefang_skills::publish::package_prepared_skill(&prepared, &output_dir)
        .unwrap_or_else(|e| {
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-package-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        });

    println!(
        "{}",
        i18n::t_args(
            "skill-bundle-created",
            &[("path", &packaged.archive_path.display().to_string())]
        )
    );
    println!(
        "{}",
        i18n::t_args("skill-bundle-sha", &[("sha", &packaged.sha256)])
    );
    println!(
        "{}",
        i18n::t_args(
            "skill-bundle-size",
            &[("size", &packaged.size_bytes.to_string())]
        )
    );

    let repo = repo.unwrap_or_else(|| format!("librefang-skills/{}", packaged.manifest.skill.name));
    let tag = tag.unwrap_or_else(|| format!("v{}", packaged.manifest.skill.version));

    if dry_run {
        println!("{}", i18n::t("skill-dry-run"));
        println!("{}", i18n::t_args("skill-dry-run-repo", &[("repo", &repo)]));
        println!("{}", i18n::t_args("skill-dry-run-tag", &[("tag", &tag)]));
        return;
    }

    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .unwrap_or_else(|_| {
            eprintln!("{}", i18n::t("skill-github-token-required"));
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

    let sp_title = i18n::t_args(
        "skill-publishing-progress",
        &[
            ("name", packaged.manifest.skill.name.as_str()),
            ("tag", &tag),
        ],
    );
    let mut sp = progress::auto(&sp_title, None);
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
            let err_msg = e.to_string();
            let fail_msg = i18n::t_args("skill-publish-failed", &[("error", &err_msg)]);
            sp.finish_with_failure(&fail_msg);
            std::process::exit(1);
        });

    let success_msg = i18n::t_args(
        "skill-publish-success",
        &[
            ("name", &published.asset_name),
            ("repo", &published.repo),
            ("tag", &published.tag),
        ],
    );
    sp.finish(&success_msg);
    if !published.html_url.is_empty() {
        println!(
            "{}",
            i18n::t_args("skill-publish-release-url", &[("url", &published.html_url)])
        );
    }
}

pub(crate) fn resolve_skill_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|e| {
            let err_msg = e.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-determine-dir-failed", &[("error", &err_msg)])
            );
            std::process::exit(1);
        })
    })
}

pub(crate) fn print_skill_warnings(warnings: &[librefang_skills::verify::SkillWarning]) {
    if warnings.is_empty() {
        println!("{}", i18n::t("skill-warnings-none"));
        return;
    }

    println!("{}", i18n::t("skill-warnings-header"));
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
    let name = prompt_input(&i18n::t("skill-prompt-name"));
    let description = prompt_input(&i18n::t("skill-prompt-description"));
    let runtime = prompt_input(&i18n::t("skill-prompt-runtime"));
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = librefang_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        let err_msg = e.to_string();
        eprintln!(
            "{}",
            i18n::t_args("skill-create-dir-failed", &[("error", &err_msg)])
        );
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
"""BossFang skill: {name}"""
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
                    r#"// BossFang skill: {name}
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
                        r#"//! BossFang skill: {name}
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
            let runtime_str = other.to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-unsupported-runtime", &[("runtime", &runtime_str)])
            );
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

    println!();
    println!(
        "{}",
        i18n::t_args(
            "skill-created",
            &[("path", &skill_dir.display().to_string())]
        )
    );
    println!();
    println!("{}", i18n::t("skill-created-files-header"));
    println!("  skill.toml");
    for (rel, _) in &scaffold.files {
        println!("  {rel}");
    }
    println!();
    println!("{}", i18n::t("skill-created-next-steps-header"));
    let mut step = 1;
    let step_str = step.to_string();
    println!(
        "{}",
        i18n::t_args("skill-created-step-edit", &[("step", &step_str)])
    );
    for build_step in &scaffold.build_steps {
        step += 1;
        println!("  {}. {}", step, build_step);
    }
    step += 1;
    let step_str2 = step.to_string();
    println!(
        "{}",
        i18n::t_args(
            "skill-created-step-test",
            &[
                ("step", &step_str2),
                ("path", &skill_dir.display().to_string())
            ]
        )
    );
    step += 1;
    let step_str3 = step.to_string();
    println!(
        "{}",
        i18n::t_args(
            "skill-created-step-install",
            &[
                ("step", &step_str3),
                ("path", &skill_dir.display().to_string())
            ]
        )
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
        let err_msg = e.to_string();
        eprintln!(
            "{}",
            i18n::t_args("skill-registry-load-failed", &[("error", &err_msg)])
        );
        std::process::exit(1);
    }
    match registry.get(name) {
        Some(skill) => (skills_dir, skill.clone()),
        None => {
            let path_str = skills_dir.display().to_string();
            eprintln!(
                "{}",
                i18n::t_args("skill-not-found", &[("name", name), ("path", &path_str)])
            );
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
                    let path_str = context_file.display().to_string();
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "skill-read-file-failed",
                            &[("path", &path_str), ("error", &err_msg)]
                        )
                    );
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
                let err_msg = e.to_string();
                eprintln!(
                    "{}",
                    i18n::t_args("skill-create-skills-dir-failed", &[("error", &err_msg)])
                );
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
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-create-failed", &[("error", &err_msg)])
                    );
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
                    let path_str = context_file.display().to_string();
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "skill-read-file-failed",
                            &[("path", &path_str), ("error", &err_msg)]
                        )
                    );
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
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-update-failed", &[("error", &err_msg)])
                    );
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
                    let path_str = old_file.display().to_string();
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "skill-read-file-failed",
                            &[("path", &path_str), ("error", &err_msg)]
                        )
                    );
                    std::process::exit(1);
                }
            };
            let new_str = match read_file_or_stdin(&new_file) {
                Ok(s) => s,
                Err(e) => {
                    let path_str = new_file.display().to_string();
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "skill-read-file-failed",
                            &[("path", &path_str), ("error", &err_msg)]
                        )
                    );
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
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-patch-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Delete { name, hand } => {
            let skills_dir = resolve_skills_dir(hand.as_deref());
            match librefang_skills::evolution::delete_skill(&skills_dir, &name) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-delete-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Rollback { name, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::rollback_skill(&skill, Some("cli")) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-rollback-failed", &[("error", &err_msg)])
                    );
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
                    let path_str = source.display().to_string();
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args(
                            "skill-read-file-failed",
                            &[("path", &path_str), ("error", &err_msg)]
                        )
                    );
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::write_supporting_file(&skill, &path, &content) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-write-file-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::RemoveFile { name, path, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::remove_supporting_file(&skill, &path) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-remove-file-failed", &[("error", &err_msg)])
                    );
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
                        let err_msg = e.to_string();
                        eprintln!(
                            "{}",
                            i18n::t_args("skill-serialize-history-failed", &[("error", &err_msg)])
                        );
                        std::process::exit(1);
                    }
                }
                return;
            }
            println!(
                "{}",
                i18n::t_args(
                    "skill-evolution-label",
                    &[("name", &skill.manifest.skill.name)]
                )
            );
            println!(
                "{}",
                i18n::t_args(
                    "skill-version-label",
                    &[("version", &skill.manifest.skill.version)]
                )
            );
            let use_count_str = meta.use_count.to_string();
            println!(
                "{}",
                i18n::t_args("skill-use-count-label", &[("count", &use_count_str)])
            );
            let evolution_count_str = meta.evolution_count.to_string();
            println!(
                "{}",
                i18n::t_args(
                    "skill-evolution-count-label",
                    &[("count", &evolution_count_str)]
                )
            );
            if meta.versions.is_empty() {
                println!();
                println!("{}", i18n::t("skill-no-history"));
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
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-read-pending-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            };
            if candidates.is_empty() {
                let filter_str = match &agent {
                    Some(a) => i18n::t_args("skill-pending-filter", &[("agent", a)]),
                    None => String::new(),
                };
                println!(
                    "{}",
                    i18n::t_args("skill-no-pending", &[("filter", &filter_str)])
                );
                return;
            }
            println!(
                "{:<38}  {:<18}  {:<22}  {}",
                i18n::t("label-id"),
                i18n::t("label-source"),
                i18n::t("label-captured"),
                i18n::t("label-name")
            );
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
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-load-candidate-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            };
            let toml_str = match toml::to_string_pretty(&candidate) {
                Ok(s) => s,
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-render-candidate-failed", &[("error", &err_msg)])
                    );
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
                    let version_str = result.version.unwrap_or_else(|| i18n::t("label-unknown"));
                    println!(
                        "{}",
                        i18n::t_args(
                            "skill-approved-candidate",
                            &[
                                ("id", &id),
                                ("name", &result.skill_name),
                                ("version", &version_str)
                            ]
                        )
                    );
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-approve-candidate-failed", &[("error", &err_msg)])
                    );
                    std::process::exit(1);
                }
            }
        }
        PendingCommands::Reject { id } => {
            match librefang_kernel::skill_workshop::storage::reject_candidate(&skills_root, &id) {
                Ok(()) => {
                    println!(
                        "{}",
                        i18n::t_args("skill-rejected-candidate", &[("id", &id)])
                    );
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    eprintln!(
                        "{}",
                        i18n::t_args("skill-reject-candidate-failed", &[("error", &err_msg)])
                    );
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
