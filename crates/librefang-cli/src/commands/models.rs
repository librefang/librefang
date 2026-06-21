//! `models` CLI command handlers, split out of `main.rs`.
//!
//! Dispatched from `main.rs`; shared helpers and imports come via
//! [`crate::commands::prelude`].

use crate::commands::prelude::*;

// ---------------------------------------------------------------------------
// New command handlers
// ---------------------------------------------------------------------------

pub(crate) fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = match provider_filter {
            Some(p) => format!("{base}/api/models?provider={p}"),
            None => format!("{base}/api/models"),
        };
        let body = daemon_json(client.get(&url).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("models")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            if arr.is_empty() {
                println!("{}", i18n::t("model-none-found"));
                return;
            }
            let header_model = i18n::t("model-header-model");
            let header_provider = i18n::t("label-header-provider");
            let header_tier = i18n::t("model-header-tier");
            let header_context = i18n::t("model-header-context");
            let mut t = crate::table::Table::new(&[
                &header_model,
                &header_provider,
                &header_tier,
                &header_context,
            ]);
            for m in arr {
                t.add_row(&[
                    m["id"].as_str().unwrap_or("?"),
                    m["provider"].as_str().unwrap_or("?"),
                    m["tier"].as_str().unwrap_or("?"),
                    &m["context_window"].as_u64().unwrap_or(0).to_string(),
                ]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        // Standalone: use ModelCatalog directly
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let models = catalog.list_models();
        if json {
            let arr: Vec<serde_json::Value> = models
                .iter()
                .filter(|m| provider_filter.is_none_or(|p| m.provider == p))
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "tier": format!("{:?}", m.tier),
                        "context_window": m.context_window,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        if models.is_empty() {
            println!("{}", i18n::t("model-none-in-catalog"));
            return;
        }
        let header_model = i18n::t("model-header-model");
        let header_provider = i18n::t("label-header-provider");
        let header_tier = i18n::t("model-header-tier");
        let header_context = i18n::t("model-header-context");
        let mut t = crate::table::Table::new(&[
            &header_model,
            &header_provider,
            &header_tier,
            &header_context,
        ]);
        for m in models {
            if let Some(p) = provider_filter {
                if m.provider != p {
                    continue;
                }
            }
            t.add_row(&[
                &m.id,
                &m.provider,
                &format!("{:?}", m.tier),
                &m.context_window.to_string(),
            ]);
        }
        t.print();
    }
}

pub(crate) fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.get("aliases").and_then(|v| v.as_array()) {
            let header_alias = i18n::t("label-header-alias");
            let header_resolves = i18n::t("model-header-resolves-to");
            let mut t = crate::table::Table::new(&[&header_alias, &header_resolves]);
            for entry in arr {
                t.add_row(&[
                    entry["alias"].as_str().unwrap_or("?"),
                    entry["model_id"].as_str().unwrap_or("?"),
                ]);
            }
            t.print();
        } else if let Some(obj) = body.as_object() {
            // Fallback for plain {alias: model_id} format
            let header_alias = i18n::t("label-header-alias");
            let header_resolves = i18n::t("model-header-resolves-to");
            let mut t = crate::table::Table::new(&[&header_alias, &header_resolves]);
            for (alias, target) in obj {
                t.add_row(&[alias.as_str(), target.as_str().unwrap_or("?")]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
            return;
        }
        let header_alias = i18n::t("label-header-alias");
        let header_resolves = i18n::t("model-header-resolves-to");
        let mut t = crate::table::Table::new(&[&header_alias, &header_resolves]);
        for (alias, target) in aliases {
            t.add_row(&[alias, target]);
        }
        t.print();
    }
}

pub(crate) fn cmd_models_providers(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/providers")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("providers")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            let header_provider = i18n::t("label-header-provider");
            let header_auth = i18n::t("model-header-auth");
            let header_models = i18n::t("model-header-models");
            let header_base_url = i18n::t("model-header-base-url");
            let mut t = crate::table::Table::new(&[
                &header_provider,
                &header_auth,
                &header_models,
                &header_base_url,
            ]);
            for p in arr {
                t.add_row(&[
                    p["id"].as_str().unwrap_or("?"),
                    p["auth_status"].as_str().unwrap_or("?"),
                    &p["model_count"].as_u64().unwrap_or(0).to_string(),
                    p["base_url"].as_str().unwrap_or(""),
                ]);
            }
            t.print();
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let providers = catalog.list_providers();
        if json {
            let arr: Vec<serde_json::Value> = providers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "auth_status": format!("{:?}", p.auth_status),
                        "model_count": p.model_count,
                        "base_url": p.base_url,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        let header_provider = i18n::t("label-header-provider");
        let header_auth = i18n::t("model-header-auth");
        let header_models = i18n::t("model-header-models");
        let header_base_url = i18n::t("model-header-base-url");
        let mut t = crate::table::Table::new(&[
            &header_provider,
            &header_auth,
            &header_models,
            &header_base_url,
        ]);
        for p in providers {
            t.add_row(&[
                &p.id,
                &format!("{:?}", p.auth_status),
                &p.model_count.to_string(),
                &p.base_url,
            ]);
        }
        t.print();
    }
}

pub(crate) fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();
    // Use the config set approach through the API
    let body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&serde_json::json!({"path": "default_model.model", "value": model}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "model-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("model-set-success", &[("model", &model)]));
    }
}

/// Interactive model picker — shows numbered list, accepts number or model ID.
pub(crate) fn pick_model() -> String {
    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error(&i18n::t("model-no-catalog"));
        std::process::exit(1);
    }

    // Group by provider for display
    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&librefang_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section(&i18n::t("section-select-model"));
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            let idx_padded = format!("{:>3}", idx);
            let id_padded = format!("{:<36}", m.id);
            let tier_str = format!("{:?}", m.tier);
            let display_str = i18n::t_args(
                "model-picker-item",
                &[
                    ("idx", &idx_padded),
                    ("id", &id_padded),
                    ("tier", &tier_str),
                ],
            );
            println!("{display_str}");
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    let prompt_msg = i18n::t("model-prompt-selection");
    loop {
        let input = prompt_input(&prompt_msg);
        if input.is_empty() {
            continue;
        }
        // Try as number first
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&i18n::t_args(
                "model-out-of-range",
                &[("max", &numbered.len().to_string())],
            ));
            continue;
        }
        // Accept direct model ID if it exists in catalog
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        // Accept as alias
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        // Accept any string (user might know a model not in catalog)
        return input;
    }
}

pub(crate) fn cmd_approvals_list(json: bool) {
    let base = require_daemon("approvals list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/approvals")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("approvals")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("{}", i18n::t("approval-none-pending"));
            return;
        }
        let header_id = i18n::t("label-header-id");
        let header_agent = i18n::t("label-header-agent");
        let header_type = i18n::t("label-header-type");
        let header_request = i18n::t("approval-header-request");
        let mut t =
            crate::table::Table::new(&[&header_id, &header_agent, &header_type, &header_request]);
        for a in arr {
            t.add_row(&[
                a["id"].as_str().unwrap_or("?"),
                a["agent_name"].as_str().unwrap_or("?"),
                a["approval_type"].as_str().unwrap_or("?"),
                a["description"].as_str().unwrap_or(""),
            ]);
        }
        t.print();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_approvals_respond(id: &str, approve: bool) {
    let base = require_daemon("approvals");
    let client = daemon_client();
    let endpoint = if approve { "approve" } else { "reject" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/approvals/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "approval-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "approval-responded",
            &[("id", id), ("action", endpoint)],
        ));
    }
}
