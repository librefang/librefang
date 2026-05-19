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

    // Helper: write the keys the dashboard configure form owns. `name`
    // and `channel_type` identify the block; `env` is the source of
    // truth for non-secret env values, so it wholly replaces whatever
    // was previously there (a key removed from the form must disappear).
    // Operator-tuned supervision fields (`restart`, `restart_*`,
    // `ready_timeout_secs`, `shutdown_grace_secs`, `message_buffer`,
    // `overflow`, …) live on the same `[[sidecar_channels]]` table but
    // are NOT touched here — they survive a save (codex review fix).
    fn write_form_managed(
        block: &mut Table,
        name: &str,
        channel_type: &str,
        env: &BTreeMap<String, String>,
    ) {
        block["name"] = value(name);
        block["channel_type"] = value(channel_type);
        let mut env_table = Table::new();
        for (k, v) in env {
            env_table[k] = value(v.clone());
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
            write_form_managed(existing, name, channel_type, env);
            replaced = true;
            break;
        }
    }
    if !replaced {
        let mut block = Table::new();
        write_command_and_args_defaults(&mut block, command, args);
        write_form_managed(&mut block, name, channel_type, env);
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
