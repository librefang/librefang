//! Idempotent upsert of one `[[sidecar_channels]]` block in config.toml,
//! identified by its `name`. Uses toml_edit to preserve formatting,
//! comments, and key ordering of every other section.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

pub fn upsert_sidecar_block(
    path: &Path,
    name: &str,
    channel_type: &str,
    command: &str,
    args: &[&str],
    env: &BTreeMap<String, String>,
    managed_env_keys: &[&str],
) -> Result<(), String> {
    let original = fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = original
        .parse()
        .map_err(|e| format!("parse {path:?}: {e}"))?;

    // Helper: write the catalog defaults that the form does NOT know about
    // — `command` and `args`. These come from `SIDECAR_CATALOG`, not from
    // the operator's payload. On the **insert** path we always write
    // them. On the **update** path we leave any non-empty existing
    // value alone so operators who hand-edit `config.toml` to point at
    // a venv binary (`command = "/opt/venv/bin/python"`) or pass extra
    // flags (`args = [..., "--debug"]`) don't lose those edits every
    // time someone clicks Save in the dashboard.
    fn write_command_and_args_defaults(block: &mut Table, command: &str, args: &[&str]) {
        block["command"] = value(command);
        let mut args_arr = Array::new();
        for a in args {
            args_arr.push(*a);
        }
        block["args"] = value(args_arr);
    }

    fn command_or_args_present(block: &Table) -> bool {
        let cmd_present = block
            .get("command")
            .and_then(|i| i.as_str())
            .is_some_and(|s| !s.is_empty());
        let args_present = block
            .get("args")
            .and_then(|i| i.as_array())
            .is_some_and(|a| !a.is_empty());
        cmd_present || args_present
    }

    // Helper: apply the keys the dashboard configure form owns. `name`
    // and `channel_type` identify the block. Within the `env` sub-table,
    // only the **schema-managed** keys (those listed in `managed_env_keys`,
    // the non-secret schema fields the form actually renders) are owned
    // by the form; every other env key present in the existing block —
    // operator hand-edits like `PYTHONPATH = "/custom"`, `HTTP_PROXY`,
    // locale variables, or even a hand-edited `TELEGRAM_BOT_TOKEN` inline
    // (legacy) — is preserved as-is across the save. Per managed key:
    // form provides non-empty value ⇒ overwrite; form provides empty /
    // absent ⇒ remove from the env table. Operator-tuned supervision
    // fields (`restart`, `restart_*`, `ready_timeout_secs`,
    // `shutdown_grace_secs`, `message_buffer`, `overflow`, …) live on
    // the same `[[sidecar_channels]]` table but are NOT touched here —
    // they survive a save.
    fn write_form_managed(
        block: &mut Table,
        name: &str,
        channel_type: &str,
        env: &BTreeMap<String, String>,
        managed_env_keys: &[&str],
    ) {
        block["name"] = value(name);
        block["channel_type"] = value(channel_type);
        // Start from the existing env table (clone it) so non-schema
        // keys survive the rewrite. If it's missing or shaped wrong,
        // fall back to a fresh empty table.
        let mut env_table: Table = block
            .get("env")
            .and_then(|i| i.as_table())
            .cloned()
            .unwrap_or_default();
        for key in managed_env_keys {
            match env.get(*key) {
                Some(v) if !v.is_empty() => {
                    env_table[*key] = value(v.clone());
                }
                _ => {
                    env_table.remove(key);
                }
            }
        }
        // Render as `[sidecar_channels.env]` (not dotted inline).
        env_table.set_implicit(false);
        block["env"] = Item::Table(env_table);
    }

    let aot_item = doc
        .entry("sidecar_channels")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let aot = aot_item
        .as_array_of_tables_mut()
        .ok_or_else(|| "config.toml: `sidecar_channels` is not an array-of-tables".to_string())?;

    // Upsert by `name`; if absent, append.
    let mut replaced = false;
    for i in 0..aot.len() {
        let existing_name = aot
            .get(i)
            .and_then(|t| t.get("name"))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if existing_name == name {
            let existing = aot.get_mut(i).expect("indexed");
            // Backfill catalog defaults only if the operator never set
            // `command`/`args` (e.g. block was hand-written as a stub).
            // Otherwise preserve their hand-edits.
            if !command_or_args_present(existing) {
                write_command_and_args_defaults(existing, command, args);
            }
            write_form_managed(existing, name, channel_type, env, managed_env_keys);
            replaced = true;
            break;
        }
    }
    if !replaced {
        let mut block = Table::new();
        write_command_and_args_defaults(&mut block, command, args);
        write_form_managed(&mut block, name, channel_type, env, managed_env_keys);
        aot.push(block);
    }

    // Atomic write to a sibling tempfile then rename.
    let parent = path.parent().ok_or("config path has no parent")?;
    // Disambiguate parallel callers: PID guards against other daemon
    // processes touching the same dir; the per-process atomic counter
    // guards against concurrent threads within this process (e.g. parallel
    // tests, or two HTTP handlers racing on the same config file). Same
    // defect class as secrets_env::upsert_secret (T3.1).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".config.toml.tmp.{}.{seq}", std::process::id()));
    fs::write(&tmp, doc.to_string()).map_err(|e| format!("write {tmp:?}: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

/// In-memory counterpart of [`upsert_sidecar_block`] for the surreal config
/// store (C-005d.3). Upserts one `SidecarChannelConfig` by `name` in a `Vec`,
/// mirroring the toml_edit semantics: on update, catalog `command`/`args`
/// defaults are backfilled only when absent (preserving operator hand-edits);
/// `name`/`channel_type` are set; within `env`, only the schema-managed keys are
/// overwritten (non-empty) or removed (empty/absent) while every other env key
/// and all supervision fields survive. On insert, a fresh entry is built via
/// serde so each `#[serde(default)]` supervision field (restart/backoff/…) gets
/// its canonical default.
///
/// **Security:** the caller passes only the NON-secret schema fields here
/// (`nonsecret_env` + `managed_env_keys`). Secret-typed fields are written to
/// `secrets.env` and never reach this Vec, so they cannot enter the
/// `sidecar_channels` config-store override.
#[cfg(feature = "surreal-backend")]
pub fn upsert_sidecar_in_vec(
    sidecars: &mut Vec<librefang_types::config::SidecarChannelConfig>,
    name: &str,
    channel_type: &str,
    command: &str,
    args: &[&str],
    env: &BTreeMap<String, String>,
    managed_env_keys: &[&str],
) -> Result<(), String> {
    let apply_managed = |env_map: &mut std::collections::HashMap<String, String>| {
        for key in managed_env_keys {
            match env.get(*key) {
                Some(v) if !v.is_empty() => {
                    env_map.insert((*key).to_string(), v.clone());
                }
                _ => {
                    env_map.remove(*key);
                }
            }
        }
    };

    if let Some(existing) = sidecars.iter_mut().find(|s| s.name == name) {
        // Backfill catalog defaults only if the operator never set them.
        if existing.command.is_empty() && existing.args.is_empty() {
            existing.command = command.to_string();
            existing.args = args.iter().map(|a| a.to_string()).collect();
        }
        existing.name = name.to_string();
        existing.channel_type = Some(channel_type.to_string());
        apply_managed(&mut existing.env);
    } else {
        // Build via serde so every `#[serde(default)]` supervision field is set.
        let mut block: librefang_types::config::SidecarChannelConfig =
            serde_json::from_value(serde_json::json!({
                "name": name,
                "channel_type": channel_type,
                "command": command,
                "args": args,
                "env": {},
            }))
            .map_err(|e| format!("construct sidecar entry: {e}"))?;
        apply_managed(&mut block.env);
        sidecars.push(block);
    }
    Ok(())
}

#[cfg(all(test, feature = "surreal-backend"))]
mod tests {
    use super::*;

    #[test]
    fn upsert_in_vec_inserts_then_updates_managed_keys_only() {
        let mut sidecars = Vec::new();
        let mut env = BTreeMap::new();
        env.insert("TELEGRAM_API_BASE".to_string(), "https://api".to_string());
        // A secret-shaped key the caller would NEVER pass (not in managed set):
        env.insert("TELEGRAM_BOT_TOKEN".to_string(), "SECRET".to_string());
        let managed = ["TELEGRAM_API_BASE"];

        upsert_sidecar_in_vec(
            &mut sidecars,
            "telegram",
            "telegram",
            "python3",
            &["-m", "adapter"],
            &env,
            &managed,
        )
        .unwrap();

        assert_eq!(sidecars.len(), 1);
        let s = &sidecars[0];
        assert_eq!(s.name, "telegram");
        assert_eq!(s.channel_type.as_deref(), Some("telegram"));
        assert_eq!(s.command, "python3");
        assert_eq!(s.args, vec!["-m", "adapter"]);
        assert_eq!(
            s.env.get("TELEGRAM_API_BASE").map(String::as_str),
            Some("https://api")
        );
        // SECURITY: a non-managed (secret) key must never land in the struct,
        // even when present in the input map.
        assert!(
            !s.env.contains_key("TELEGRAM_BOT_TOKEN"),
            "non-managed (secret) key must not enter the sidecar override"
        );

        // Update: preserve a hand-edited env key + supervision defaults; clear a
        // managed key when the form sends it empty.
        sidecars[0]
            .env
            .insert("PYTHONPATH".to_string(), "/custom".to_string());
        sidecars[0].restart = true;
        let mut env2 = BTreeMap::new();
        env2.insert("TELEGRAM_API_BASE".to_string(), String::new()); // cleared
        upsert_sidecar_in_vec(
            &mut sidecars,
            "telegram",
            "telegram",
            "python3",
            &["-m", "adapter"],
            &env2,
            &managed,
        )
        .unwrap();
        assert_eq!(sidecars.len(), 1, "update must not append a duplicate");
        let s = &sidecars[0];
        assert!(
            !s.env.contains_key("TELEGRAM_API_BASE"),
            "empty managed key removed"
        );
        assert_eq!(
            s.env.get("PYTHONPATH").map(String::as_str),
            Some("/custom"),
            "hand-edit preserved"
        );
        assert!(s.restart, "supervision field preserved across update");
    }
}

/// Remove the `[[sidecar_channels]]` block identified by `name`; returns whether one was removed.
pub fn remove_sidecar_block(path: &Path, name: &str) -> Result<bool, String> {
    let original = fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = original
        .parse()
        .map_err(|e| format!("parse {path:?}: {e}"))?;

    let now_empty;
    {
        let Some(aot_item) = doc.get_mut("sidecar_channels") else {
            return Ok(false);
        };
        let aot = aot_item.as_array_of_tables_mut().ok_or_else(|| {
            "config.toml: `sidecar_channels` is not an array-of-tables".to_string()
        })?;
        let idx = (0..aot.len()).find(|&i| {
            aot.get(i)
                .and_then(|t| t.get("name"))
                .and_then(|v| v.as_str())
                == Some(name)
        });
        let Some(idx) = idx else {
            return Ok(false);
        };
        aot.remove(idx);
        now_empty = aot.is_empty();
    }
    // Drop a now-empty array entirely rather than leaving a bare `sidecar_channels = []`.
    if now_empty {
        doc.remove("sidecar_channels");
    }

    // Atomic write to a sibling tempfile then rename (same scheme as upsert_sidecar_block).
    let parent = path.parent().ok_or("config path has no parent")?;
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".config.toml.tmp.{}.{seq}", std::process::id()));
    fs::write(&tmp, doc.to_string()).map_err(|e| format!("write {tmp:?}: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(true)
}
