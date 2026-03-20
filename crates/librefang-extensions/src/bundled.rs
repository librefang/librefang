//! Integration templates loaded from disk at runtime.
//!
//! Integrations are read from `~/.librefang/integrations/` (synced from
//! the registry via `librefang init`). No compile-time embedding.

/// Returns all integration templates found on disk as (id, TOML content) pairs.
pub fn bundled_integrations() -> Vec<(&'static str, &'static str)> {
    disk_integrations()
        .into_iter()
        .map(|(id, content)| {
            let id: &'static str = Box::leak(id.into_boxed_str());
            let content: &'static str = Box::leak(content.into_boxed_str());
            (id, content)
        })
        .collect()
}

fn disk_integrations() -> Vec<(String, String)> {
    let mut results = Vec::new();

    let home = std::env::var("LIBREFANG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join(".librefang")
        });
    let integrations_dir = home.join("integrations");

    if let Ok(entries) = std::fs::read_dir(&integrations_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".toml") => n.trim_end_matches(".toml").to_string(),
                _ => continue,
            };
            let content = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            results.push((name, content));
        }
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_integrations_returns_vec() {
        // Just verify it doesn't panic — actual content depends on disk state
        let _ = bundled_integrations();
    }
}
