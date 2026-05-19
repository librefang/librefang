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

    // Build the replacement table for this single block.
    let mut block = Table::new();
    block["name"] = value(name);
    block["channel_type"] = value(channel_type);
    block["command"] = value(command);
    let mut args_arr = Array::new();
    for a in args {
        args_arr.push(*a);
    }
    block["args"] = value(args_arr);
    let mut env_table = Table::new();
    for (k, v) in env {
        env_table[k] = value(v.clone());
    }
    // Render as `[sidecar_channels.env]` (not dotted inline).
    env_table.set_implicit(false);
    block["env"] = Item::Table(env_table);

    let aot_item = doc
        .entry("sidecar_channels")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let aot = aot_item
        .as_array_of_tables_mut()
        .ok_or_else(|| "config.toml: `sidecar_channels` is not an array-of-tables".to_string())?;

    // Replace by `name`; if absent, append.
    let mut replaced = false;
    for i in 0..aot.len() {
        let existing_name = aot
            .get(i)
            .and_then(|t| t.get("name"))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if existing_name == name {
            *aot.get_mut(i).expect("indexed") = block.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
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
