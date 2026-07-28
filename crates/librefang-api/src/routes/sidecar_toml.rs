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

    atomic_write(path, &doc.to_string())?;
    Ok(())
}

/// Monotonic counter disambiguating concurrent tempfile names within this process.
///
/// Module-level on purpose.
/// Both writers format into the same `.config.toml.tmp.{pid}.{seq}` namespace, so a counter declared inside each function would give each its own sequence starting at zero and the first call to either would mint the identical path — a concurrent configure and remove could then write and rename each other's tempfile, landing one request's document at the other's target or losing it entirely.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write `contents` to `path` by way of a sibling tempfile and a rename.
///
/// Shared by both writers so the sequence that disambiguates their tempfile names is genuinely shared — see [`TMP_SEQ`].
/// PID guards against other daemon processes touching the same directory; the counter guards against concurrent threads within this process (parallel tests, or two HTTP handlers racing on the same config file).
/// Same defect class as `secrets_env::upsert_secret` (T3.1).
fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("config path has no parent")?;
    let tmp = parent.join(next_tmp_name());
    fs::write(&tmp, contents).map_err(|e| format!("write {tmp:?}: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

/// Next tempfile name from the shared sequence.
/// Split out so a test can assert successive names differ without writing to the filesystem.
fn next_tmp_name() -> String {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    format!(".config.toml.tmp.{}.{seq}", std::process::id())
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

    atomic_write(path, &doc.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Both writers mint tempfile names from one namespace, so the sequence backing that namespace has to be shared and atomic.
    /// Before the writers were funnelled through `atomic_write`, each declared its own `static SEQ` starting at zero, so the first `upsert` and the first `remove` in a process both produced `.config.toml.tmp.{pid}.0`.
    ///
    /// Names are drawn concurrently and asserted all-distinct.
    /// This pins atomicity and the single shared counter; it cannot detect a future writer that bypasses `next_tmp_name` and formats the same pattern itself, which is what the doc comment on `TMP_SEQ` is for.
    #[test]
    fn tmp_names_are_unique_across_concurrent_callers() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 32;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..PER_THREAD)
                        .map(|_| next_tmp_name())
                        .collect::<Vec<String>>()
                })
            })
            .collect();

        let names: Vec<String> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("tmp-name thread panicked"))
            .collect();

        let distinct: HashSet<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(
            distinct.len(),
            THREADS * PER_THREAD,
            "tempfile names collided — the sequence is not shared or not atomic; \
             a concurrent configure and remove can rename each other's tempfile away"
        );

        let pid_prefix = format!(".config.toml.tmp.{}.", std::process::id());
        for name in &names {
            assert!(
                name.starts_with(&pid_prefix),
                "unexpected tempfile name shape: {name}"
            );
        }
    }
}
