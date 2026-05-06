//! Integration smoke test for the hand registry (#3696).
//!
//! Exercises the install → activate → list → deactivate → uninstall lifecycle
//! end-to-end on a fresh temp home. The point is to catch regressions where
//! these public-API methods stop composing — every previous bug in this area
//! was a cross-method invariant violation (definitions present but no
//! workspace, instance present after uninstall, …).
//!
//! No LLM, no kernel — pure tool-dispatch / persistence behaviour.

use std::collections::HashMap;

use librefang_hands::registry::HandRegistry;
use librefang_hands::HandStatus;

const SMOKE_HAND_TOML: &str = r#"
id = "smoke-hand"
name = "Smoke Hand"
description = "Fixture used by the registry smoke integration test."
category = "data"

[routing]
aliases = ["smoke"]

[agent]
name = "smoke-hand-agent"
description = "Test agent for the smoke hand"
system_prompt = "You are a smoke-test agent. Echo what you are told."
"#;

const SMOKE_SKILL_MD: &str = "# Smoke Skill\n\nIntegration-test skill body.\n";

#[test]
fn install_activate_deactivate_uninstall_lifecycle() {
    let reg = HandRegistry::new();
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();

    // Sanity: a fresh registry holds nothing.
    assert!(reg.list_definitions().is_empty());
    assert!(reg.list_instances().is_empty());

    // install_from_content_persisted writes both HAND.toml and SKILL.md to
    // `home/workspaces/<id>/` and registers the definition in memory.
    let def = reg
        .install_from_content_persisted(home, SMOKE_HAND_TOML, SMOKE_SKILL_MD)
        .expect("install should succeed");
    assert_eq!(def.id, "smoke-hand");
    assert_eq!(def.name, "Smoke Hand");

    // Files must be on disk so a daemon restart could reload them.
    let workspace = home.join("workspaces").join("smoke-hand");
    assert!(
        workspace.join("HAND.toml").exists(),
        "HAND.toml must be persisted to {workspace:?}"
    );
    assert!(
        workspace.join("SKILL.md").exists(),
        "SKILL.md must be persisted to {workspace:?}"
    );

    // The definition is now visible to the read-side API.
    let listed_ids: Vec<String> = reg.list_definitions().into_iter().map(|d| d.id).collect();
    assert!(
        listed_ids.contains(&"smoke-hand".to_string()),
        "list_definitions must include the freshly installed hand: {listed_ids:?}"
    );
    assert!(reg.get_definition("smoke-hand").is_some());

    // Activate the hand. The empty config map is the explicit-default path
    // for a hand with no required settings — equivalent to the dashboard
    // "activate with defaults" button.
    let instance = reg
        .activate("smoke-hand", HashMap::new())
        .expect("activate should succeed for a hand with no required settings");
    assert_eq!(instance.hand_id, "smoke-hand");
    assert_eq!(instance.status, HandStatus::Active);

    // Active instances are observable via list_instances and find_by_id.
    let instances = reg.list_instances();
    assert_eq!(instances.len(), 1, "exactly one active instance");
    assert_eq!(instances[0].instance_id, instance.instance_id);
    assert!(reg.get_instance(instance.instance_id).is_some());

    // While the hand has a live instance, uninstall must be refused — the
    // contract the DELETE /api/hands/{id} route depends on. We deliberately
    // don't pin the error variant (covered by the unit tests in registry.rs);
    // we only assert the cross-method invariant: refused uninstall does NOT
    // perturb in-memory state.
    let _refused = reg
        .uninstall_hand(home, "smoke-hand")
        .expect_err("uninstall must be refused while a hand has live instances");
    assert!(
        reg.get_definition("smoke-hand").is_some(),
        "definition must survive a refused uninstall"
    );
    assert_eq!(
        reg.list_instances().len(),
        1,
        "instance must survive a refused uninstall"
    );

    // Deactivate brings the instance count back to zero so the workspace can
    // be cleaned up.
    let deactivated = reg
        .deactivate(instance.instance_id)
        .expect("deactivate should succeed");
    assert_eq!(deactivated.instance_id, instance.instance_id);
    assert!(
        reg.list_instances().is_empty(),
        "deactivate must remove the instance from list_instances"
    );

    // Now uninstall must succeed and physically remove both the workspace
    // dir and the in-memory definition.
    reg.uninstall_hand(home, "smoke-hand")
        .expect("uninstall must succeed for a custom hand with no live instances");
    assert!(
        reg.get_definition("smoke-hand").is_none(),
        "definition must be gone after successful uninstall"
    );
    assert!(
        !workspace.exists(),
        "workspace directory must be removed after successful uninstall"
    );
}

#[test]
fn definitions_round_trip_through_a_disk_reload() {
    // After install_from_content_persisted, a second registry that calls
    // reload_from_disk on the same home must see the hand. Locks in the
    // contract that the `home/workspaces/<id>/HAND.toml` layout is the
    // source of truth — a daemon restart re-discovers everything from
    // disk without needing to replay an in-memory log.
    let installer = HandRegistry::new();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    installer
        .install_from_content_persisted(home, SMOKE_HAND_TOML, SMOKE_SKILL_MD)
        .unwrap();

    let fresh = HandRegistry::new();
    assert!(
        fresh.list_definitions().is_empty(),
        "second registry must start empty before reload"
    );
    let (loaded, _failed) = fresh.reload_from_disk(home);
    assert!(
        loaded >= 1,
        "reload_from_disk must report at least one hand loaded, got {loaded}"
    );

    let def = fresh
        .get_definition("smoke-hand")
        .expect("smoke-hand must be discoverable after reload");
    assert_eq!(def.name, "Smoke Hand");
}
